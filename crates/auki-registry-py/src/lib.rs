//! PyO3 bindings for [`auki-registry`](../../auki-registry).
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

use std::path::PathBuf;

use auki_registry_rs as registry;
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyModule};
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

// ─── Frame Registry constructors ───────────────────────────────────

#[pyfunction]
fn frame_ros_body(py: Python<'_>, frame_id: &str) -> PyResult<PyObject> {
    struct_to_pyobject(py, &registry::FrameRegistryEntry::ros_body(frame_id))
}

#[pyfunction]
fn frame_ros_optical(py: Python<'_>, frame_id: &str) -> PyResult<PyObject> {
    struct_to_pyobject(py, &registry::FrameRegistryEntry::ros_optical(frame_id))
}

#[pyfunction]
fn frame_opengl(py: Python<'_>, frame_id: &str) -> PyResult<PyObject> {
    struct_to_pyobject(py, &registry::FrameRegistryEntry::opengl(frame_id))
}

#[pyfunction]
fn frame_unity(py: Python<'_>, frame_id: &str) -> PyResult<PyObject> {
    struct_to_pyobject(py, &registry::FrameRegistryEntry::unity(frame_id))
}

#[pyfunction]
#[pyo3(signature = (*, frame_id, handedness, x, y, z, units))]
fn frame_entry(
    py: Python<'_>,
    frame_id: &str,
    handedness: &str,
    x: &str,
    y: &str,
    z: &str,
    units: &str,
) -> PyResult<PyObject> {
    let entry = registry::FrameRegistryEntry {
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
#[pyo3(signature = (*, sensor_id, width, height, frame_rate_hz, pixel_format, color_space, intrinsics_model, distortion_model, frame_id, frame_hash))]
fn rgb_camera_sensor_entry(
    py: Python<'_>,
    sensor_id: &str,
    width: u32,
    height: u32,
    frame_rate_hz: u32,
    pixel_format: &str,
    color_space: &str,
    intrinsics_model: &str,
    distortion_model: &str,
    frame_id: &str,
    frame_hash: &str,
) -> PyResult<PyObject> {
    let entry = registry::SensorRegistryEntry {
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::RgbCamera(registry::RgbCamera {
            width,
            height,
            frame_rate_hz,
            pixel_format: pixel_format.to_string(),
            color_space: color_space.to_string(),
            intrinsics_model: intrinsics_model.to_string(),
            distortion_model: distortion_model.to_string(),
            frame_id: frame_id.to_string(),
            frame_hash: frame_hash.to_string(),
        }),
    };
    struct_to_pyobject(py, &entry)
}

#[pyfunction]
#[pyo3(signature = (*, sensor_id, fields, point_step, is_bigendian, frame_rate_hz, frame_id, frame_hash))]
fn point_cloud_sensor_entry(
    py: Python<'_>,
    sensor_id: &str,
    fields: &Bound<'_, PyAny>,
    point_step: u32,
    is_bigendian: bool,
    frame_rate_hz: u32,
    frame_id: &str,
    frame_hash: &str,
) -> PyResult<PyObject> {
    let fields: Vec<registry::PointField> = parse_py(py, fields, "fields")?;
    let entry = registry::SensorRegistryEntry {
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::PointCloud(registry::PointCloud {
            fields,
            point_step,
            is_bigendian,
            frame_rate_hz,
            frame_id: frame_id.to_string(),
            frame_hash: frame_hash.to_string(),
        }),
    };
    struct_to_pyobject(py, &entry)
}

#[pyfunction]
#[pyo3(signature = (*, sensor_id, sample_rate_hz, channels, sample_format, channel_layout))]
fn audio_sensor_entry(
    py: Python<'_>,
    sensor_id: &str,
    sample_rate_hz: u32,
    channels: u32,
    sample_format: &str,
    channel_layout: &str,
) -> PyResult<PyObject> {
    let entry = registry::SensorRegistryEntry {
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::Audio(registry::Audio {
            sample_rate_hz,
            channels,
            sample_format: sample_format.to_string(),
            channel_layout: channel_layout.to_string(),
        }),
    };
    struct_to_pyobject(py, &entry)
}

#[pyfunction]
#[pyo3(signature = (*, sensor_id, joint_count, frame_rate_hz))]
fn joint_encoders_sensor_entry(
    py: Python<'_>,
    sensor_id: &str,
    joint_count: u32,
    frame_rate_hz: u32,
) -> PyResult<PyObject> {
    let entry = registry::SensorRegistryEntry {
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::JointEncoders(registry::JointEncoders {
            joint_count,
            frame_rate_hz,
        }),
    };
    struct_to_pyobject(py, &entry)
}

// ─── Clock Registry constructors ───────────────────────────────────

#[pyfunction]
#[pyo3(signature = (*, clock_id, unit, scope, epoch=None))]
fn monotonic_clock_entry(
    py: Python<'_>,
    clock_id: &str,
    unit: &str,
    scope: &str,
    epoch: Option<String>,
) -> PyResult<PyObject> {
    let entry = registry::ClockRegistryEntry {
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
#[pyo3(signature = (*, clock_id, unit, scope, epoch))]
fn utc_clock_entry(
    py: Python<'_>,
    clock_id: &str,
    unit: &str,
    scope: &str,
    epoch: &str,
) -> PyResult<PyObject> {
    let entry = registry::ClockRegistryEntry {
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
fn read_frame(py: Python<'_>, app_root: PathBuf, frame_id: &str, hash: &str) -> PyResult<PyObject> {
    match registry::read_frame(&app_root, frame_id, hash).map_err(map_registry_error)? {
        Some(entry) => struct_to_pyobject(py, &entry),
        None => Ok(py.None()),
    }
}

#[pyfunction]
fn read_sensor(
    py: Python<'_>,
    app_root: PathBuf,
    sensor_id: &str,
    hash: &str,
) -> PyResult<PyObject> {
    match registry::read_sensor(&app_root, sensor_id, hash).map_err(map_registry_error)? {
        Some(entry) => struct_to_pyobject(py, &entry),
        None => Ok(py.None()),
    }
}

#[pyfunction]
fn read_clock(py: Python<'_>, app_root: PathBuf, clock_id: &str, hash: &str) -> PyResult<PyObject> {
    match registry::read_clock(&app_root, clock_id, hash).map_err(map_registry_error)? {
        Some(entry) => struct_to_pyobject(py, &entry),
        None => Ok(py.None()),
    }
}

/// Module entry point.
#[pymodule]
fn auki_registry(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(frame_ros_body, m)?)?;
    m.add_function(wrap_pyfunction!(frame_ros_optical, m)?)?;
    m.add_function(wrap_pyfunction!(frame_opengl, m)?)?;
    m.add_function(wrap_pyfunction!(frame_unity, m)?)?;
    m.add_function(wrap_pyfunction!(frame_entry, m)?)?;
    m.add_function(wrap_pyfunction!(point_field, m)?)?;
    m.add_function(wrap_pyfunction!(rgb_camera_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(point_cloud_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(audio_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(joint_encoders_sensor_entry, m)?)?;
    m.add_function(wrap_pyfunction!(monotonic_clock_entry, m)?)?;
    m.add_function(wrap_pyfunction!(utc_clock_entry, m)?)?;
    m.add_function(wrap_pyfunction!(hash_frame, m)?)?;
    m.add_function(wrap_pyfunction!(hash_sensor, m)?)?;
    m.add_function(wrap_pyfunction!(hash_clock, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_json_frame, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_json_sensor, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_json_clock, m)?)?;
    m.add_function(wrap_pyfunction!(write_frame, m)?)?;
    m.add_function(wrap_pyfunction!(write_sensor, m)?)?;
    m.add_function(wrap_pyfunction!(write_clock, m)?)?;
    m.add_function(wrap_pyfunction!(read_frame, m)?)?;
    m.add_function(wrap_pyfunction!(read_sensor, m)?)?;
    m.add_function(wrap_pyfunction!(read_clock, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::{PyDict, PyList, PyModule};

    const FRAME_ID: &str = "K1-AABBCCDDEEFF/head_left_cam_optical";

    #[test]
    fn module_exposes_registry_surface() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_registry").unwrap();
            auki_registry(py, &module).unwrap();

            assert!(module.getattr("frame_ros_optical").is_ok());
            assert!(module.getattr("point_cloud_sensor_entry").is_ok());
            assert!(module.getattr("write_sensor").is_ok());
        });
    }

    #[test]
    fn frame_write_read_round_trip_returns_hash() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let frame = frame_ros_optical(py, FRAME_ID).unwrap();
            let frame_hash = write_frame(py, dir.path().to_path_buf(), frame.bind(py)).unwrap();

            assert_eq!(frame_hash, "e0d40e7b526e04f15f83f75897f53825");

            let read = read_frame(py, dir.path().to_path_buf(), FRAME_ID, &frame_hash).unwrap();
            let read = read.bind(py);
            assert_eq!(
                read.get_item("frame_id")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                FRAME_ID
            );
            assert_eq!(
                hash_frame(py, read).unwrap(),
                "e0d40e7b526e04f15f83f75897f53825"
            );
        });
    }

    #[test]
    fn point_cloud_sensor_requires_written_frame() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let frame = frame_ros_optical(py, FRAME_ID).unwrap();
            let frame_hash = write_frame(py, dir.path().to_path_buf(), frame.bind(py)).unwrap();
            let field = point_field(py, "x", 0, "float32", 1).unwrap();
            let fields = PyList::new_bound(py, [field.bind(py)]);
            let sensor = point_cloud_sensor_entry(
                py,
                "K1-AABBCCDDEEFF/head_depth_points",
                fields.as_any(),
                4,
                false,
                10,
                FRAME_ID,
                &frame_hash,
            )
            .unwrap();

            let sensor_hash = write_sensor(py, dir.path().to_path_buf(), sensor.bind(py)).unwrap();
            let read = read_sensor(
                py,
                dir.path().to_path_buf(),
                "K1-AABBCCDDEEFF/head_depth_points",
                &sensor_hash,
            )
            .unwrap();

            assert_eq!(
                read.bind(py)
                    .get_item("frame_hash")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                frame_hash
            );
        });
    }

    #[test]
    fn documented_python_call_flow_works_through_module() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_registry").unwrap();
            auki_registry(py, &module).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().to_str().unwrap();

            let frame = module
                .getattr("frame_ros_optical")
                .unwrap()
                .call1((FRAME_ID,))
                .unwrap();
            let frame_hash: String = module
                .getattr("write_frame")
                .unwrap()
                .call1((root, &frame))
                .unwrap()
                .extract()
                .unwrap();
            let field = module
                .getattr("point_field")
                .unwrap()
                .call1(("x", 0_u32, "float32"))
                .unwrap();
            let fields = PyList::new_bound(py, [field]);
            let kwargs = PyDict::new_bound(py);
            kwargs
                .set_item("sensor_id", "K1-AABBCCDDEEFF/head_depth_points")
                .unwrap();
            kwargs.set_item("fields", &fields).unwrap();
            kwargs.set_item("point_step", 4_u32).unwrap();
            kwargs.set_item("is_bigendian", false).unwrap();
            kwargs.set_item("frame_rate_hz", 10_u32).unwrap();
            kwargs.set_item("frame_id", FRAME_ID).unwrap();
            kwargs.set_item("frame_hash", &frame_hash).unwrap();
            let sensor = module
                .getattr("point_cloud_sensor_entry")
                .unwrap()
                .call((), Some(&kwargs))
                .unwrap();

            let sensor_hash: String = module
                .getattr("write_sensor")
                .unwrap()
                .call1((root, &sensor))
                .unwrap()
                .extract()
                .unwrap();

            assert_eq!(
                module
                    .getattr("hash_sensor")
                    .unwrap()
                    .call1((&sensor,))
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                sensor_hash
            );
        });
    }

    #[test]
    fn write_sensor_rejects_missing_frame_hash() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let field = point_field(py, "x", 0, "float32", 1).unwrap();
            let fields = PyList::new_bound(py, [field.bind(py)]);
            let sensor = point_cloud_sensor_entry(
                py,
                "K1-AABBCCDDEEFF/head_depth_points",
                fields.as_any(),
                4,
                false,
                10,
                FRAME_ID,
                "missing",
            )
            .unwrap();

            let err = write_sensor(py, dir.path().to_path_buf(), sensor.bind(py)).unwrap_err();
            assert!(err.is_instance_of::<PyValueError>(py));
        });
    }
}
