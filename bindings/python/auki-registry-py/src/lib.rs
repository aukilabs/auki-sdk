//! PyO3 bindings for [`auki-registry`](../../../../crates/auki-registry).
//!
//! The surface is deliberately dict-oriented, matching
//! [`auki-manifests-py`](../../auki-manifests-py): constructors return
//! plain Python `dict`s with the canonical registry shape, and the
//! `write_*` / `read_*` helpers delegate to the Rust crate for hashing,
//! validation, and on-disk layout.
//!
//! Producer sidecars can now do the complete registry dance in Python:
//! write a `FrameRegistryEntry`, use its returned hash in a spatial
//! `SensorRegistryEntry`, write that sensor entry, then hand the two
//! hashes to `auki_domain.StreamManifestBuilder.from_registry(...)`.
//!
//! ## RegistryRef / LogRef
//!
//! Two helper classes are exposed so callers can construct the nested
//! `(peer_id, id, hash)` / `(source_peer_id, resource_id)` shapes
//! without building plain dicts by hand. Both are serialized as `dict`
//! when passed through the `hash_*` / `write_*` helpers.

use std::path::PathBuf;

use auki_registry_rs as registry;
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyModule};
use serde::Serialize;
use serde::de::DeserializeOwned;

fn py_to_json(py: Python<'_>, value: &Bound<'_, PyAny>, name: &str) -> PyResult<serde_json::Value> {
    let json = py.import_bound("json")?;
    let s: String = json.call_method1("dumps", (value,))?.extract()?;
    serde_json::from_str(&s).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

fn json_to_pyobject(py: Python<'_>, value: &serde_json::Value) -> PyResult<PyObject> {
    let json = py.import_bound("json")?;
    let s = serde_json::to_string(value)
        .map_err(|e| PyRuntimeError::new_err(format!("internal registry serialize: {e}")))?;
    Ok(json.call_method1("loads", (s,))?.unbind())
}

fn struct_to_pyobject<T: Serialize>(py: Python<'_>, value: &T) -> PyResult<PyObject> {
    let json = serde_json::to_value(value)
        .map_err(|e| PyRuntimeError::new_err(format!("internal registry serialize: {e}")))?;
    json_to_pyobject(py, &json)
}

fn parse_py<T: DeserializeOwned>(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<T> {
    let json = py_to_json(py, value, name)?;
    serde_json::from_value(json).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

fn parse_string_enum<T: DeserializeOwned>(name: &str, value: &str) -> PyResult<T> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

fn map_registry_error(err: registry::Error) -> PyErr {
    match err {
        registry::Error::Io(err) => PyOSError::new_err(err.to_string()),
        other => PyValueError::new_err(other.to_string()),
    }
}

fn write_outcome_hash(outcome: registry::WriteOutcome) -> String {
    outcome.hash().to_string()
}

/// Parse a `RegistryRef` out of a Python object that is either:
/// - a `RegistryRef` pyclass instance, or
/// - a plain `dict` with `peer_id`, `id`, `hash` keys.
fn parse_registry_ref(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<registry::RegistryRef> {
    // Try to extract as our RegistryRef pyclass first.
    if let Ok(r) = value.extract::<RegistryRef>() {
        return Ok(registry::RegistryRef {
            peer_id: r.peer_id,
            id: r.id,
            hash: r.hash,
        });
    }
    // Fall back to dict / any JSON-serializable object.
    parse_py(py, value, "registry_ref")
}

// ─── RegistryRef / LogRef helper classes ───────────────────────────────────

/// Python representation of a `(peer_id, id, hash)` registry reference.
/// Passed to sensor/manifest builders wherever a cross-registry pointer is needed.
#[pyclass]
#[derive(Clone)]
pub struct RegistryRef {
    #[pyo3(get, set)]
    pub peer_id: String,
    #[pyo3(get, set)]
    pub id: String,
    #[pyo3(get, set)]
    pub hash: String,
}

#[pymethods]
impl RegistryRef {
    #[new]
    fn new(peer_id: String, id: String, hash: String) -> Self {
        Self { peer_id, id, hash }
    }

    fn __repr__(&self) -> String {
        format!(
            "RegistryRef(peer_id={:?}, id={:?}, hash={:?})",
            self.peer_id, self.id, self.hash
        )
    }

    /// Validate that the `peer_id`, `id`, and `hash` fields are non-empty.
    fn validate(&self) -> PyResult<()> {
        if self.peer_id.is_empty() {
            return Err(PyValueError::new_err(
                "RegistryRef.peer_id must not be empty",
            ));
        }
        if self.id.is_empty() {
            return Err(PyValueError::new_err("RegistryRef.id must not be empty"));
        }
        if self.hash.is_empty() {
            return Err(PyValueError::new_err("RegistryRef.hash must not be empty"));
        }
        Ok(())
    }
}

/// Python representation of a `(source_peer_id, resource_id)` log reference.
/// Used by `DetectionLogManifest.input_log` — logs are addressed by identity
/// tuple, not a single content hash.
#[pyclass]
#[derive(Clone)]
pub struct LogRef {
    #[pyo3(get, set)]
    pub source_peer_id: String,
    #[pyo3(get, set)]
    pub resource_id: String,
}

#[pymethods]
impl LogRef {
    #[new]
    fn new(source_peer_id: String, resource_id: String) -> Self {
        Self {
            source_peer_id,
            resource_id,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "LogRef(source_peer_id={:?}, resource_id={:?})",
            self.source_peer_id, self.resource_id
        )
    }

    fn validate(&self) -> PyResult<()> {
        if self.source_peer_id.is_empty() {
            return Err(PyValueError::new_err(
                "LogRef.source_peer_id must not be empty",
            ));
        }
        if self.resource_id.is_empty() {
            return Err(PyValueError::new_err(
                "LogRef.resource_id must not be empty",
            ));
        }
        Ok(())
    }
}

// ─── Frame Registry constructors ───────────────────────────────────

#[pyfunction]
fn frame_ros_body(py: Python<'_>, peer_id: &str, frame_id: &str) -> PyResult<PyObject> {
    struct_to_pyobject(
        py,
        &registry::FrameRegistryEntry::ros_body(peer_id, frame_id),
    )
}

#[pyfunction]
fn frame_ros_optical(py: Python<'_>, peer_id: &str, frame_id: &str) -> PyResult<PyObject> {
    struct_to_pyobject(
        py,
        &registry::FrameRegistryEntry::ros_optical(peer_id, frame_id),
    )
}

#[pyfunction]
fn frame_opengl(py: Python<'_>, peer_id: &str, frame_id: &str) -> PyResult<PyObject> {
    struct_to_pyobject(py, &registry::FrameRegistryEntry::opengl(peer_id, frame_id))
}

#[pyfunction]
fn frame_unity(py: Python<'_>, peer_id: &str, frame_id: &str) -> PyResult<PyObject> {
    struct_to_pyobject(py, &registry::FrameRegistryEntry::unity(peer_id, frame_id))
}

#[pyfunction]
#[pyo3(signature = (*, peer_id, frame_id, handedness, x, y, z, units))]
fn frame_entry(
    py: Python<'_>,
    peer_id: &str,
    frame_id: &str,
    handedness: &str,
    x: &str,
    y: &str,
    z: &str,
    units: &str,
) -> PyResult<PyObject> {
    let entry = registry::FrameRegistryEntry {
        peer_id: peer_id.to_string(),
        frame_id: frame_id.to_string(),
        handedness: parse_string_enum("handedness", handedness)?,
        axes: registry::AxisConvention {
            x: parse_string_enum("x", x)?,
            y: parse_string_enum("y", y)?,
            z: parse_string_enum("z", z)?,
        },
        units: parse_string_enum("units", units)?,
    };
    struct_to_pyobject(py, &entry)
}

// ─── Sensor Registry constructors ──────────────────────────────────

#[pyfunction]
#[pyo3(signature = (name, offset, datatype, count=1))]
fn point_field(
    py: Python<'_>,
    name: &str,
    offset: u32,
    datatype: &str,
    count: u32,
) -> PyResult<PyObject> {
    let field = registry::PointField {
        name: name.to_string(),
        offset,
        datatype: parse_string_enum("datatype", datatype)?,
        count,
    };
    struct_to_pyobject(py, &field)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (*, peer_id, sensor_id, sensor_type, width, height, frame_rate_hz, image_encoding, pixel_format, row_stride_bytes, color_space, intrinsics_model, distortion_model, frame, calibration=None))]
fn camera_sensor_entry(
    py: Python<'_>,
    peer_id: &str,
    sensor_id: &str,
    sensor_type: &str,
    width: u32,
    height: u32,
    frame_rate_hz: u32,
    image_encoding: &str,
    pixel_format: &str,
    row_stride_bytes: u32,
    color_space: &str,
    intrinsics_model: &str,
    distortion_model: &str,
    frame: &Bound<'_, PyAny>,
    calibration: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyObject> {
    let frame_ref = parse_registry_ref(py, frame)?;
    let calibration = calibration
        .map(|value| parse_py(py, value, "calibration"))
        .transpose()?;
    let entry = registry::SensorRegistryEntry {
        peer_id: peer_id.to_string(),
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::Camera(registry::Camera {
            r#type: sensor_type.to_string(),
            width,
            height,
            frame_rate_hz,
            image_encoding: image_encoding.to_string(),
            pixel_format: pixel_format.to_string(),
            row_stride_bytes,
            color_space: color_space.to_string(),
            intrinsics_model: intrinsics_model.to_string(),
            distortion_model: distortion_model.to_string(),
            calibration,
            frame: frame_ref,
        }),
    };
    struct_to_pyobject(py, &entry)
}

#[pyfunction]
#[pyo3(signature = (*, peer_id, sensor_id, sensor_type, fields, point_step, is_bigendian, frame_rate_hz, frame))]
fn rangefinder_sensor_entry(
    py: Python<'_>,
    peer_id: &str,
    sensor_id: &str,
    sensor_type: &str,
    fields: &Bound<'_, PyAny>,
    point_step: u32,
    is_bigendian: bool,
    frame_rate_hz: u32,
    frame: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let fields: Vec<registry::PointField> = parse_py(py, fields, "fields")?;
    let frame_ref = parse_registry_ref(py, frame)?;
    let entry = registry::SensorRegistryEntry {
        peer_id: peer_id.to_string(),
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::Rangefinder(registry::Rangefinder {
            r#type: sensor_type.to_string(),
            fields,
            point_step,
            is_bigendian,
            frame_rate_hz,
            frame: frame_ref,
        }),
    };
    struct_to_pyobject(py, &entry)
}

#[pyfunction]
#[pyo3(signature = (*, peer_id, sensor_id, sensor_type, sample_rate_hz, channels, sample_format, channel_layout, frame))]
fn audio_sensor_entry(
    py: Python<'_>,
    peer_id: &str,
    sensor_id: &str,
    sensor_type: &str,
    sample_rate_hz: u32,
    channels: u32,
    sample_format: &str,
    channel_layout: &str,
    frame: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let frame_ref = parse_registry_ref(py, frame)?;
    let entry = registry::SensorRegistryEntry {
        peer_id: peer_id.to_string(),
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::Audio(registry::Audio {
            r#type: sensor_type.to_string(),
            sample_rate_hz,
            channels,
            sample_format: sample_format.to_string(),
            channel_layout: channel_layout.to_string(),
            frame: frame_ref,
        }),
    };
    struct_to_pyobject(py, &entry)
}

#[pyfunction]
#[pyo3(signature = (*, peer_id, sensor_id, sensor_type, joint_count, frame_rate_hz, frame))]
fn joint_encoders_sensor_entry(
    py: Python<'_>,
    peer_id: &str,
    sensor_id: &str,
    sensor_type: &str,
    joint_count: u32,
    frame_rate_hz: u32,
    frame: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let frame_ref = parse_registry_ref(py, frame)?;
    let entry = registry::SensorRegistryEntry {
        peer_id: peer_id.to_string(),
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::JointEncoders(registry::JointEncoders {
            r#type: sensor_type.to_string(),
            joint_count,
            frame_rate_hz,
            frame: frame_ref,
        }),
    };
    struct_to_pyobject(py, &entry)
}

#[pyfunction]
#[pyo3(signature = (*, peer_id, sensor_id, sensor_type, frame))]
fn rf_sensor_entry(
    py: Python<'_>,
    peer_id: &str,
    sensor_id: &str,
    sensor_type: &str,
    frame: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let frame_ref = parse_registry_ref(py, frame)?;
    let entry = registry::SensorRegistryEntry {
        peer_id: peer_id.to_string(),
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::Rf(registry::Rf {
            r#type: sensor_type.to_string(),
            frame: frame_ref,
        }),
    };
    struct_to_pyobject(py, &entry)
}

/// Build a non-spatial scalar Sensor Registry entry.
///
/// `sensor_type` and `unit` are open strings so applications can introduce
/// measurements such as `battery_charge` / `percent` without another SDK
/// schema change.
#[pyfunction]
#[pyo3(signature = (*, peer_id, sensor_id, sensor_type, unit, expected_rate_hz))]
fn scalar_sensor_entry(
    py: Python<'_>,
    peer_id: &str,
    sensor_id: &str,
    sensor_type: &str,
    unit: &str,
    expected_rate_hz: u32,
) -> PyResult<PyObject> {
    let scalar = registry::Scalar {
        r#type: sensor_type.to_string(),
        unit: unit.to_string(),
        expected_rate_hz,
    };
    scalar.validate().map_err(map_registry_error)?;
    let entry = registry::SensorRegistryEntry {
        peer_id: peer_id.to_string(),
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::Scalar(scalar),
    };
    struct_to_pyobject(py, &entry)
}

// ─── Validate ID ───────────────────────────────────────────────────

#[pyfunction]
fn validate_sensor_id(id: &str) -> PyResult<()> {
    registry::SensorRegistryEntry::validate_id(id)
        .map_err(|e| PyValueError::new_err(format!("sensor_id: {e}")))
}

#[pyfunction]
fn validate_clock_id(id: &str) -> PyResult<()> {
    registry::ClockRegistryEntry::validate_id(id)
        .map_err(|e| PyValueError::new_err(format!("clock_id: {e}")))
}

#[pyfunction]
fn validate_frame_id(id: &str) -> PyResult<()> {
    registry::FrameRegistryEntry::validate_id(id)
        .map_err(|e| PyValueError::new_err(format!("frame_id: {e}")))
}

#[pyfunction]
fn validate_detector_id(id: &str) -> PyResult<()> {
    registry::DetectorRegistryEntry::validate_id(id)
        .map_err(|e| PyValueError::new_err(format!("detector_id: {e}")))
}

#[pyfunction]
fn validate_map_id(id: &str) -> PyResult<()> {
    registry::MapRegistryEntry::validate_id(id)
        .map_err(|e| PyValueError::new_err(format!("map_id: {e}")))
}

// ─── Clock Registry constructors ───────────────────────────────────

#[pyfunction]
#[pyo3(signature = (*, peer_id, session_id, clock_id, unit, scope, epoch=None))]
fn monotonic_clock_entry(
    py: Python<'_>,
    peer_id: &str,
    session_id: &str,
    clock_id: &str,
    unit: &str,
    scope: &str,
    epoch: Option<String>,
) -> PyResult<PyObject> {
    let entry = registry::ClockRegistryEntry {
        peer_id: peer_id.to_string(),
        session_id: session_id.to_string(),
        clock_id: clock_id.to_string(),
        body: registry::ClockBody::MonotonicClock(registry::ClockMeta {
            unit: unit.to_string(),
            monotonic: true,
            epoch,
            scope: parse_string_enum("scope", scope)?,
        }),
    };
    struct_to_pyobject(py, &entry)
}

#[pyfunction]
#[pyo3(signature = (*, peer_id, session_id, clock_id, unit, scope, epoch))]
fn utc_clock_entry(
    py: Python<'_>,
    peer_id: &str,
    session_id: &str,
    clock_id: &str,
    unit: &str,
    scope: &str,
    epoch: &str,
) -> PyResult<PyObject> {
    let entry = registry::ClockRegistryEntry {
        peer_id: peer_id.to_string(),
        session_id: session_id.to_string(),
        clock_id: clock_id.to_string(),
        body: registry::ClockBody::UtcClock(registry::ClockMeta {
            unit: unit.to_string(),
            monotonic: false,
            epoch: Some(epoch.to_string()),
            scope: parse_string_enum("scope", scope)?,
        }),
    };
    struct_to_pyobject(py, &entry)
}

// ─── Map Registry constructors ─────────────────────────────────────

#[pyfunction]
#[pyo3(signature = (*, peer_id, map_id, frame, voxel_size_m, chunk_dimension, color_model=None, semantic_classes=Vec::new()))]
fn voxel_map_entry(
    py: Python<'_>,
    peer_id: &str,
    map_id: &str,
    frame: &Bound<'_, PyAny>,
    voxel_size_m: f64,
    chunk_dimension: u32,
    color_model: Option<&str>,
    semantic_classes: Vec<String>,
) -> PyResult<PyObject> {
    let entry = registry::MapRegistryEntry {
        peer_id: peer_id.to_string(),
        map_id: map_id.to_string(),
        body: registry::MapBody::Voxel(registry::VoxelMap {
            frame: parse_registry_ref(py, frame)?,
            voxel_size_m: registry::FiniteF64(voxel_size_m),
            chunk_dimension,
            value_model: registry::VoxelValueModel::AdditiveOccupancyEvidence,
            color_model: color_model
                .map(|value| parse_string_enum("color_model", value))
                .transpose()?,
            semantic_classes,
        }),
    };
    entry.validate().map_err(map_registry_error)?;
    struct_to_pyobject(py, &entry)
}

// ─── Hash / canonical JSON helpers ─────────────────────────────────

#[pyfunction]
fn hash_frame(py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::FrameRegistryEntry = parse_py(py, entry, "frame")?;
    Ok(entry.hash())
}

#[pyfunction]
fn hash_sensor(py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::SensorRegistryEntry = parse_py(py, entry, "sensor")?;
    Ok(entry.hash())
}

#[pyfunction]
fn hash_clock(py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::ClockRegistryEntry = parse_py(py, entry, "clock")?;
    Ok(entry.hash())
}

#[pyfunction]
fn hash_map(py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::MapRegistryEntry = parse_py(py, entry, "map")?;
    entry.validate().map_err(map_registry_error)?;
    Ok(entry.hash())
}

#[pyfunction]
fn canonical_json_frame(py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::FrameRegistryEntry = parse_py(py, entry, "frame")?;
    String::from_utf8(entry.canonical_bytes())
        .map_err(|e| PyRuntimeError::new_err(format!("internal registry utf8: {e}")))
}

#[pyfunction]
fn canonical_json_sensor(py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::SensorRegistryEntry = parse_py(py, entry, "sensor")?;
    String::from_utf8(entry.canonical_bytes())
        .map_err(|e| PyRuntimeError::new_err(format!("internal registry utf8: {e}")))
}

#[pyfunction]
fn canonical_json_clock(py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::ClockRegistryEntry = parse_py(py, entry, "clock")?;
    String::from_utf8(entry.canonical_bytes())
        .map_err(|e| PyRuntimeError::new_err(format!("internal registry utf8: {e}")))
}

#[pyfunction]
fn canonical_json_map(py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::MapRegistryEntry = parse_py(py, entry, "map")?;
    entry.validate().map_err(map_registry_error)?;
    String::from_utf8(entry.canonical_bytes())
        .map_err(|e| PyRuntimeError::new_err(format!("internal registry utf8: {e}")))
}

// ─── Storage helpers ───────────────────────────────────────────────

#[pyfunction]
fn write_frame(py: Python<'_>, app_root: PathBuf, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::FrameRegistryEntry = parse_py(py, entry, "frame")?;
    registry::write_frame(&app_root, &entry)
        .map(write_outcome_hash)
        .map_err(map_registry_error)
}

#[pyfunction]
fn write_sensor(py: Python<'_>, app_root: PathBuf, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::SensorRegistryEntry = parse_py(py, entry, "sensor")?;
    registry::write_sensor(&app_root, &entry)
        .map(write_outcome_hash)
        .map_err(map_registry_error)
}

#[pyfunction]
fn write_clock(py: Python<'_>, app_root: PathBuf, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::ClockRegistryEntry = parse_py(py, entry, "clock")?;
    registry::write_clock(&app_root, &entry)
        .map(write_outcome_hash)
        .map_err(map_registry_error)
}

#[pyfunction]
fn write_map(py: Python<'_>, app_root: PathBuf, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::MapRegistryEntry = parse_py(py, entry, "map")?;
    registry::write_map(&app_root, &entry)
        .map(write_outcome_hash)
        .map_err(map_registry_error)
}

#[pyfunction]
fn write_device_model(py: Python<'_>, app_root: PathBuf, entry: &Bound<'_, PyAny>) -> PyResult<String> {
    let entry: registry::DeviceModelRegistryEntry = parse_py(py, entry, "device model")?;
    registry::write_device_model(&app_root, &entry).map(write_outcome_hash).map_err(map_registry_error)
}

#[pyfunction]
fn put_blob(app_root: PathBuf, bytes: &[u8]) -> PyResult<String> {
    registry::put_blob(&app_root, bytes).map_err(map_registry_error)
}

#[pyfunction]
fn get_blob(py: Python<'_>, app_root: PathBuf, sha256: &str) -> PyResult<PyObject> {
    match registry::get_blob(&app_root, sha256).map_err(map_registry_error)? {
        Some(bytes) => Ok(pyo3::types::PyBytes::new_bound(py, &bytes).into_any().unbind()),
        None => Ok(py.None()),
    }
}

#[pyfunction]
fn sha256_hex(bytes: &[u8]) -> String {
    registry::sha256_hex(bytes)
}

#[pyfunction]
#[pyo3(signature = (app_root, urdf_path, root_convention=None))]
fn put_urdf_package(
    py: Python<'_>,
    app_root: PathBuf,
    urdf_path: PathBuf,
    root_convention: Option<String>,
) -> PyResult<PyObject> {
    let package = registry::put_urdf_package(&app_root, &urdf_path, root_convention)
        .map_err(map_registry_error)?;
    let dict = PyDict::new_bound(py);
    dict.set_item("device_model_id", package.device_model_id)?;
    dict.set_item("body", struct_to_pyobject(py, &package.body)?)?;
    Ok(dict.into_any().unbind())
}

#[pyfunction]
fn list_device_models(py: Python<'_>, app_root: PathBuf, peer_id: &str) -> PyResult<PyObject> {
    let entries =
        registry::list_device_models(&app_root, peer_id).map_err(map_registry_error)?;
    struct_to_pyobject(py, &entries)
}

#[pyfunction]
fn read_frame(
    py: Python<'_>,
    app_root: PathBuf,
    peer_id: &str,
    frame_id: &str,
    hash: &str,
) -> PyResult<PyObject> {
    match registry::read_frame(&app_root, peer_id, frame_id, hash).map_err(map_registry_error)? {
        Some(entry) => struct_to_pyobject(py, &entry),
        None => Ok(py.None()),
    }
}

#[pyfunction]
fn read_sensor(
    py: Python<'_>,
    app_root: PathBuf,
    peer_id: &str,
    sensor_id: &str,
    hash: &str,
) -> PyResult<PyObject> {
    match registry::read_sensor(&app_root, peer_id, sensor_id, hash).map_err(map_registry_error)? {
        Some(entry) => struct_to_pyobject(py, &entry),
        None => Ok(py.None()),
    }
}

#[pyfunction]
fn read_clock(
    py: Python<'_>,
    app_root: PathBuf,
    peer_id: &str,
    clock_id: &str,
    hash: &str,
) -> PyResult<PyObject> {
    match registry::read_clock(&app_root, peer_id, clock_id, hash).map_err(map_registry_error)? {
        Some(entry) => struct_to_pyobject(py, &entry),
        None => Ok(py.None()),
    }
}

#[pyfunction]
fn read_map(
    py: Python<'_>,
    app_root: PathBuf,
    peer_id: &str,
    map_id: &str,
    hash: &str,
) -> PyResult<PyObject> {
    match registry::read_map(&app_root, peer_id, map_id, hash).map_err(map_registry_error)? {
        Some(entry) => struct_to_pyobject(py, &entry),
        None => Ok(py.None()),
    }
}

/// Module entry point.
#[pymodule]
fn auki_registry(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RegistryRef>()?;
    m.add_class::<LogRef>()?;
    m.add_function(wrap_pyfunction!(frame_ros_body, m)?)?;
    m.add_function(wrap_pyfunction!(frame_ros_optical, m)?)?;
    m.add_function(wrap_pyfunction!(frame_opengl, m)?)?;
    m.add_function(wrap_pyfunction!(frame_unity, m)?)?;
    m.add_function(wrap_pyfunction!(frame_entry, m)?)?;
    m.add_function(wrap_pyfunction!(point_field, m)?)?;
    m.add_function(wrap_pyfunction!(camera_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(rangefinder_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(audio_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(joint_encoders_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(rf_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(scalar_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(validate_sensor_id, m)?)?;
    m.add_function(wrap_pyfunction!(validate_clock_id, m)?)?;
    m.add_function(wrap_pyfunction!(validate_frame_id, m)?)?;
    m.add_function(wrap_pyfunction!(validate_detector_id, m)?)?;
    m.add_function(wrap_pyfunction!(validate_map_id, m)?)?;
    m.add_function(wrap_pyfunction!(monotonic_clock_entry, m)?)?;
    m.add_function(wrap_pyfunction!(utc_clock_entry, m)?)?;
    m.add_function(wrap_pyfunction!(voxel_map_entry, m)?)?;
    m.add_function(wrap_pyfunction!(hash_frame, m)?)?;
    m.add_function(wrap_pyfunction!(hash_sensor, m)?)?;
    m.add_function(wrap_pyfunction!(hash_clock, m)?)?;
    m.add_function(wrap_pyfunction!(hash_map, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_json_frame, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_json_sensor, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_json_clock, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_json_map, m)?)?;
    m.add_function(wrap_pyfunction!(write_frame, m)?)?;
    m.add_function(wrap_pyfunction!(write_sensor, m)?)?;
    m.add_function(wrap_pyfunction!(write_clock, m)?)?;
    m.add_function(wrap_pyfunction!(write_map, m)?)?;
    m.add_function(wrap_pyfunction!(write_device_model, m)?)?;
    m.add_function(wrap_pyfunction!(put_blob, m)?)?;
    m.add_function(wrap_pyfunction!(get_blob, m)?)?;
    m.add_function(wrap_pyfunction!(sha256_hex, m)?)?;
    m.add_function(wrap_pyfunction!(put_urdf_package, m)?)?;
    m.add_function(wrap_pyfunction!(list_device_models, m)?)?;
    m.add_function(wrap_pyfunction!(read_frame, m)?)?;
    m.add_function(wrap_pyfunction!(read_sensor, m)?)?;
    m.add_function(wrap_pyfunction!(read_clock, m)?)?;
    m.add_function(wrap_pyfunction!(read_map, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::{PyDict, PyList, PyModule};

    const PEER_ID: &str = "test-peer";
    const FRAME_ID: &str = "K1-AABBCCDDEEFF/head_left_cam_optical";

    fn make_frame_ref_dict(py: Python<'_>, hash: &str) -> PyObject {
        let d = PyDict::new_bound(py);
        d.set_item("peer_id", PEER_ID).unwrap();
        d.set_item("id", FRAME_ID).unwrap();
        d.set_item("hash", hash).unwrap();
        d.into_any().unbind()
    }

    #[test]
    fn module_exposes_registry_surface() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_registry").unwrap();
            auki_registry(py, &module).unwrap();

            assert!(module.getattr("RegistryRef").is_ok());
            assert!(module.getattr("LogRef").is_ok());
            assert!(module.getattr("frame_ros_optical").is_ok());
            assert!(module.getattr("rangefinder_sensor_entry").is_ok());
            assert!(module.getattr("write_sensor").is_ok());
            assert!(module.getattr("voxel_map_entry").is_ok());
            assert!(module.getattr("write_map").is_ok());
        });
    }

    #[test]
    fn voxel_map_write_read_round_trip_returns_hash() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let frame = frame_ros_body(py, PEER_ID, "map").unwrap();
            let frame_hash = write_frame(py, dir.path().to_path_buf(), frame.bind(py)).unwrap();
            let frame_ref = make_frame_ref_dict(py, &frame_hash);
            let map = voxel_map_entry(
                py,
                PEER_ID,
                "voxel/world",
                frame_ref.bind(py),
                0.05,
                16,
                None,
                Vec::new(),
            )
            .unwrap();
            let hash = write_map(py, dir.path().to_path_buf(), map.bind(py)).unwrap();
            let read =
                read_map(py, dir.path().to_path_buf(), PEER_ID, "voxel/world", &hash).unwrap();
            assert_eq!(hash_map(py, read.bind(py)).unwrap(), hash);
        });
    }

    #[test]
    fn frame_write_read_round_trip_returns_hash() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let frame = frame_ros_optical(py, PEER_ID, FRAME_ID).unwrap();
            let frame_hash = write_frame(py, dir.path().to_path_buf(), frame.bind(py)).unwrap();

            let read =
                read_frame(py, dir.path().to_path_buf(), PEER_ID, FRAME_ID, &frame_hash).unwrap();
            let read = read.bind(py);
            assert_eq!(
                read.get_item("frame_id")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                FRAME_ID
            );
            assert_eq!(
                read.get_item("peer_id")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                PEER_ID
            );
            assert_eq!(hash_frame(py, read).unwrap(), frame_hash);
        });
    }

    #[test]
    fn rangefinder_sensor_requires_written_frame() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let frame = frame_ros_optical(py, PEER_ID, FRAME_ID).unwrap();
            let frame_hash = write_frame(py, dir.path().to_path_buf(), frame.bind(py)).unwrap();
            let frame_ref = make_frame_ref_dict(py, &frame_hash);
            let field = point_field(py, "x", 0, "float32", 1).unwrap();
            let fields = PyList::new_bound(py, [field.bind(py)]);
            let sensor = rangefinder_sensor_entry(
                py,
                PEER_ID,
                "K1-AABBCCDDEEFF/head_depth_points",
                "point_cloud",
                fields.as_any(),
                4,
                false,
                10,
                frame_ref.bind(py),
            )
            .unwrap();

            let sensor_hash = write_sensor(py, dir.path().to_path_buf(), sensor.bind(py)).unwrap();
            let read = read_sensor(
                py,
                dir.path().to_path_buf(),
                PEER_ID,
                "K1-AABBCCDDEEFF/head_depth_points",
                &sensor_hash,
            )
            .unwrap();

            // frame is now a nested object
            let read_bound = read.bind(py);
            let frame_obj = read_bound.get_item("frame").unwrap();
            let hash_val = frame_obj.get_item("hash").unwrap();
            assert_eq!(hash_val.extract::<String>().unwrap(), frame_hash);
        });
    }

    #[test]
    fn write_sensor_rejects_missing_frame_hash() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let frame_ref = make_frame_ref_dict(py, "missing");
            let field = point_field(py, "x", 0, "float32", 1).unwrap();
            let fields = PyList::new_bound(py, [field.bind(py)]);
            let sensor = rangefinder_sensor_entry(
                py,
                PEER_ID,
                "K1-AABBCCDDEEFF/head_depth_points",
                "point_cloud",
                fields.as_any(),
                4,
                false,
                10,
                frame_ref.bind(py),
            )
            .unwrap();

            let err = write_sensor(py, dir.path().to_path_buf(), sensor.bind(py)).unwrap_err();
            assert!(err.is_instance_of::<PyValueError>(py));
        });
    }

    #[test]
    fn registry_ref_pyclass_round_trips_as_frame_ref() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let frame = frame_ros_optical(py, PEER_ID, FRAME_ID).unwrap();
            let frame_hash = write_frame(py, dir.path().to_path_buf(), frame.bind(py)).unwrap();

            // Build a RegistryRef pyclass instance and pass it to camera_sensor_entry.
            let frame_ref = RegistryRef::new(
                PEER_ID.to_string(),
                FRAME_ID.to_string(),
                frame_hash.clone(),
            );
            let frame_ref_py = Py::new(py, frame_ref).unwrap();
            let sensor = camera_sensor_entry(
                py,
                PEER_ID,
                "K1-AABBCCDDEEFF/head_left_cam",
                "rgb",
                544,
                488,
                20,
                "raw",
                "YUV_NV12",
                544,
                "BT.709",
                "pinhole",
                "plumb_bob",
                frame_ref_py.bind(py),
                None,
            )
            .unwrap();

            let sensor_hash = write_sensor(py, dir.path().to_path_buf(), sensor.bind(py)).unwrap();
            assert!(!sensor_hash.is_empty());
        });
    }

    #[test]
    fn camera_sensor_entry_accepts_static_calibration() {
        Python::with_gil(|py| {
            let frame_ref = make_frame_ref_dict(py, "frame-hash");
            let calibration = PyDict::new_bound(py);
            calibration.set_item("fx", 400.0).unwrap();
            calibration.set_item("fy", 401.0).unwrap();
            calibration.set_item("cx", 272.5).unwrap();
            calibration.set_item("cy", 244.5).unwrap();
            calibration
                .set_item("distortion_coefficients", vec![-0.1, 0.05, 0.0, 0.0, 0.0])
                .unwrap();

            let sensor = camera_sensor_entry(
                py,
                PEER_ID,
                "K1-AABBCCDDEEFF/head_left_cam",
                "rgb",
                544,
                488,
                20,
                "raw",
                "YUV_NV12",
                544,
                "BT.709",
                "pinhole",
                "plumb_bob",
                frame_ref.bind(py),
                Some(calibration.as_any()),
            )
            .unwrap();

            let parsed: registry::SensorRegistryEntry =
                parse_py(py, sensor.bind(py), "sensor").unwrap();
            let registry::SensorBody::Camera(camera) = parsed.body else {
                panic!("expected camera body");
            };
            let calibration = camera.calibration.expect("static calibration");
            assert_eq!(calibration.fx.0, 400.0);
            assert_eq!(calibration.distortion_coefficients.len(), 5);
        });
    }

    #[test]
    fn scalar_sensor_entry_is_non_spatial_and_round_trips() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let sensor = scalar_sensor_entry(
                py,
                PEER_ID,
                "K1-AABBCCDDEEFF/battery_charge",
                "battery_charge",
                "percent",
                1,
            )
            .unwrap();

            let sensor_hash = write_sensor(py, dir.path().to_path_buf(), sensor.bind(py)).unwrap();
            let read = read_sensor(
                py,
                dir.path().to_path_buf(),
                PEER_ID,
                "K1-AABBCCDDEEFF/battery_charge",
                &sensor_hash,
            )
            .unwrap();
            let parsed: registry::SensorRegistryEntry =
                parse_py(py, read.bind(py), "sensor").unwrap();
            let registry::SensorBody::Scalar(scalar) = parsed.body else {
                panic!("expected scalar body");
            };
            assert_eq!(scalar.r#type, "battery_charge");
            assert_eq!(scalar.unit, "percent");
            assert_eq!(scalar.expected_rate_hz, 1);
        });
    }

    #[test]
    fn scalar_sensor_entry_rejects_invalid_metadata() {
        Python::with_gil(|py| {
            let error = scalar_sensor_entry(
                py,
                PEER_ID,
                "K1-AABBCCDDEEFF/battery_charge",
                "battery_charge",
                "percent",
                0,
            )
            .unwrap_err();
            assert!(error.is_instance_of::<PyValueError>(py));
        });
    }
}
