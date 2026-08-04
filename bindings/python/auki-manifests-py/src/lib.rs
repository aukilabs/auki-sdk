//! PyO3 bindings for [`auki-manifests`](../../../../crates/auki-manifests) — pure-
//! function wrappers around the four `build_*_log_manifest` builders.
//!
//! Each builder takes typed args (strings, ints, durations, enums) and
//! returns a Python `dict` that mirrors the JCS-canonical JSON the
//! Rust crate produces. Python consumers can then hand the dict to
//! [`auki_logs.Log.open`](../../auki-logs-py) without re-implementing
//! field names, types, or ordering.
//!
//! ## Source / writer split (#216)
//!
//! All four builders now take `source_peer_id` and `writer_peer_id` as
//! the first two positional arguments. The distinction:
//!
//! - `source_peer_id` — the peer that produced the data (sensor,
//!   pose, time-transform, detections).
//! - `writer_peer_id` — the peer that wrote this manifest file. May
//!   differ when a remote peer materializes the log (e.g. Park
//!   re-materializing Galbot's sensor log).
//!
//! ## Cross-registry references
//!
//! All `*_id` + `*_hash` pairs from the old API are replaced by
//! `RegistryRef` / `LogRef` typed objects (from `auki-registry-py`).
//! Callers pass them as Python dicts (``{"peer_id": ..., "id": ..., "hash": ...}``)
//! or as `auki_registry.RegistryRef` pyclass instances — both are
//! accepted via the JSON round-trip path.
//!
//! ## Enum seam
//!
//! `PoseSource`, `PoseWriterMode`, and `TimeTransformSource` are
//! Rust tagged enums. The Python surface takes them as **dicts** /
//! strings rather than introducing PyClass equivalents — keeps the
//! Python footprint small and matches the natural JSON shape Python
//! callers already think in. The wrappers parse via `serde_json` into
//! the Rust enum, surfacing decode errors as `ValueError`.

use std::time::Duration;

use auki_manifests_rs as manifests;
use auki_registry_rs::{LogRef, RegistryRef};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyModule};

fn pyany_to_json(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<serde_json::Value> {
    let json = py.import_bound("json")?;
    let s: String = json.call_method1("dumps", (value,))?.extract()?;
    serde_json::from_str(&s).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

fn pydict_to_json(py: Python<'_>, dict: &Bound<'_, PyDict>) -> PyResult<serde_json::Value> {
    pyany_to_json(py, dict.as_any(), "dict")
}

fn json_to_pyobject(py: Python<'_>, v: &serde_json::Value) -> PyResult<PyObject> {
    let json = py.import_bound("json")?;
    let s = serde_json::to_string(v)
        .map_err(|e| PyRuntimeError::new_err(format!("internal manifest serialize: {e}")))?;
    Ok(json.call_method1("loads", (s,))?.unbind())
}

fn parse_registry_ref(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<RegistryRef> {
    let v = pyany_to_json(py, value, name)?;
    serde_json::from_value(v).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

fn parse_log_ref(py: Python<'_>, value: &Bound<'_, PyAny>, name: &str) -> PyResult<LogRef> {
    let v = pyany_to_json(py, value, name)?;
    serde_json::from_value(v).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

fn parse_pose_source(py: Python<'_>, dict: &Bound<'_, PyDict>) -> PyResult<manifests::PoseSource> {
    let v = pydict_to_json(py, dict)?;
    serde_json::from_value(v).map_err(|e| PyValueError::new_err(format!("source: {e}")))
}

fn parse_time_transform_source(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
) -> PyResult<manifests::TimeTransformSource> {
    let v = pydict_to_json(py, dict)?;
    serde_json::from_value(v).map_err(|e| PyValueError::new_err(format!("source: {e}")))
}

fn parse_pose_writer_mode(s: &str) -> PyResult<manifests::PoseWriterMode> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|e| PyValueError::new_err(format!("writer_mode: {e}")))
}

/// Build a Sensor Log manifest.
///
/// Parameters
/// ----------
/// source_peer_id:
///     The peer that produced the sensor data.
/// writer_peer_id:
///     The peer writing this manifest file (may differ for materialized copies).
/// app_id:
///     Application identifier, e.g. ``"boosterapp"``.
/// session_id:
///     UUIDv4 run identifier.
/// sensor:
///     A ``RegistryRef`` dict ``{"peer_id": ..., "id": ..., "hash": ...}``
///     pointing at the sensor registry entry. Also accepted as an
///     ``auki_registry.RegistryRef`` instance.
/// clock:
///     A ``RegistryRef`` dict pointing at the clock registry entry.
/// frame:
///     Optional ``RegistryRef`` dict for the spatial frame (omit for non-spatial
///     sensors like audio / joint encoders).
/// segment_duration_ns:
///     Roll-over interval for log segments in nanoseconds.
/// retention_ns:
///     Eviction age for segments in nanoseconds (0 = keep forever).
#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (*, source_peer_id, writer_peer_id, app_id, session_id, sensor, clock, segment_duration_ns, retention_ns, frame=None))]
fn build_sensor_log_manifest(
    py: Python<'_>,
    source_peer_id: &str,
    writer_peer_id: &str,
    app_id: &str,
    session_id: &str,
    sensor: &Bound<'_, PyAny>,
    clock: &Bound<'_, PyAny>,
    segment_duration_ns: u64,
    retention_ns: u64,
    frame: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyObject> {
    let sensor_ref = parse_registry_ref(py, sensor, "sensor")?;
    let clock_ref = parse_registry_ref(py, clock, "clock")?;
    let frame_ref = match frame {
        Some(f) => Some(parse_registry_ref(py, f, "frame")?),
        None => None,
    };
    let m = manifests::build_sensor_log_manifest(
        source_peer_id,
        writer_peer_id,
        app_id,
        session_id,
        sensor_ref,
        clock_ref,
        frame_ref,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    );
    json_to_pyobject(py, &m)
}

/// Build a Pose Log manifest.
///
/// Parameters
/// ----------
/// source_peer_id / writer_peer_id:
///     Source/writer split (see module docstring).
/// from_frame / to_frame:
///     ``RegistryRef`` dicts ``{"peer_id": ..., "id": ..., "hash": ...}``
///     for the frame pair.
/// clock:
///     ``RegistryRef`` dict for the clock.
/// source:
///     Dict matching ``PoseSource``'s tagged-enum shape, e.g.
///     ``{"kind": "ros2_tf", "publishers": [...]}``.
/// writer_mode:
///     ``"rigid"`` or ``"movable"``.
/// expected_rate_hz:
///     Producer's nominal sample rate (hint only).
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn build_pose_log_manifest(
    py: Python<'_>,
    source_peer_id: &str,
    writer_peer_id: &str,
    app_id: &str,
    session_id: &str,
    from_frame: &Bound<'_, PyAny>,
    to_frame: &Bound<'_, PyAny>,
    clock: &Bound<'_, PyAny>,
    source: &Bound<'_, PyDict>,
    writer_mode: &str,
    expected_rate_hz: u32,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> PyResult<PyObject> {
    let from_ref = parse_registry_ref(py, from_frame, "from_frame")?;
    let to_ref = parse_registry_ref(py, to_frame, "to_frame")?;
    let clock_ref = parse_registry_ref(py, clock, "clock")?;
    let pose_source = parse_pose_source(py, source)?;
    let mode = parse_pose_writer_mode(writer_mode)?;
    let m = manifests::build_pose_log_manifest(
        source_peer_id,
        writer_peer_id,
        app_id,
        session_id,
        from_ref,
        to_ref,
        clock_ref,
        &pose_source,
        mode,
        expected_rate_hz,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    );
    json_to_pyobject(py, &m)
}

/// Build a TimeTransform Log manifest.
///
/// Parameters
/// ----------
/// source_peer_id / writer_peer_id:
///     Source/writer split (see module docstring).
/// from_clock / to_clock:
///     ``RegistryRef`` dicts for the two clock entries.
/// source:
///     Dict matching ``TimeTransformSource``'s tagged-enum shape, e.g.
///     ``{"kind": "local_clock_read"}``.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn build_time_transform_log_manifest(
    py: Python<'_>,
    source_peer_id: &str,
    writer_peer_id: &str,
    app_id: &str,
    session_id: &str,
    from_clock: &Bound<'_, PyAny>,
    to_clock: &Bound<'_, PyAny>,
    source: &Bound<'_, PyDict>,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> PyResult<PyObject> {
    let from_ref = parse_registry_ref(py, from_clock, "from_clock")?;
    let to_ref = parse_registry_ref(py, to_clock, "to_clock")?;
    let tt_source = parse_time_transform_source(py, source)?;
    let m = manifests::build_time_transform_log_manifest(
        source_peer_id,
        writer_peer_id,
        app_id,
        session_id,
        from_ref,
        to_ref,
        &tt_source,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    );
    json_to_pyobject(py, &m)
}

/// Build a Detection Log manifest.
///
/// Parameters
/// ----------
/// source_peer_id / writer_peer_id:
///     Source/writer split (see module docstring).
/// detector:
///     ``RegistryRef`` dict for the detector registry entry.
/// input_log:
///     ``LogRef`` dict ``{"source_peer_id": ..., "resource_id": ...}``
///     identifying the input sensor log being tailed.
/// input_sensor:
///     ``RegistryRef`` dict copied from the input log's manifest
///     (makes the detection log self-contained).
/// clock:
///     ``RegistryRef`` dict for the clock.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn build_detection_log_manifest(
    py: Python<'_>,
    source_peer_id: &str,
    writer_peer_id: &str,
    app_id: &str,
    session_id: &str,
    instance_id: &str,
    detector: &Bound<'_, PyAny>,
    input_log: &Bound<'_, PyAny>,
    input_sensor: &Bound<'_, PyAny>,
    clock: &Bound<'_, PyAny>,
    cadence: &Bound<'_, PyAny>,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> PyResult<PyObject> {
    let detector_ref = parse_registry_ref(py, detector, "detector")?;
    let log_ref = parse_log_ref(py, input_log, "input_log")?;
    let input_sensor_ref = parse_registry_ref(py, input_sensor, "input_sensor")?;
    let clock_ref = parse_registry_ref(py, clock, "clock")?;
    let cadence_value = pyany_to_json(py, cadence, "cadence")?;
    let cadence: manifests::DetectionCadence = serde_json::from_value(cadence_value)
        .map_err(|e| PyValueError::new_err(format!("cadence: {e}")))?;
    let m = manifests::build_detection_log_manifest(
        source_peer_id,
        writer_peer_id,
        app_id,
        session_id,
        instance_id,
        detector_ref,
        log_ref,
        input_sensor_ref,
        clock_ref,
        cadence,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    );
    json_to_pyobject(py, &m)
}

/// Module entry point.
#[pymodule]
fn auki_manifests(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(build_sensor_log_manifest, m)?)?;
    m.add_function(wrap_pyfunction!(build_pose_log_manifest, m)?)?;
    m.add_function(wrap_pyfunction!(build_time_transform_log_manifest, m)?)?;
    m.add_function(wrap_pyfunction!(build_detection_log_manifest, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Rust-side smoke tests for the enum-parsing helpers. The
    //! Python surface (round-trips through `json.dumps` / `json.loads`)
    //! is exercised in `python_tests/`.

    use super::*;

    #[test]
    fn parse_pose_writer_mode_accepts_canonical_strings() {
        let m = parse_pose_writer_mode("rigid").unwrap();
        assert_eq!(m, manifests::PoseWriterMode::Rigid);
        let m = parse_pose_writer_mode("movable").unwrap();
        assert_eq!(m, manifests::PoseWriterMode::Movable);
    }

    #[test]
    fn parse_pose_writer_mode_rejects_garbage() {
        let r = parse_pose_writer_mode("nonsense");
        assert!(r.is_err());
    }
}
