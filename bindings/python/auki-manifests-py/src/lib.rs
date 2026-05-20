//! PyO3 bindings for [`auki-manifests`](../../../../crates/auki-manifests) — pure-
//! function wrappers around the four `build_*_log_manifest` builders.
//!
//! Each builder takes typed args (strings, ints, durations, enums) and
//! returns a Python `dict` that mirrors the JCS-canonical JSON the
//! Rust crate produces. Python consumers can then hand the dict to
//! [`auki_logs.Log.open`](../../auki-logs-py) without re-implementing
//! field names, types, or ordering.
//!
//! Surface — every Rust builder exposed as a `#[pyfunction]`:
//!
//! - `build_sensor_log_manifest(...)` — Sensor Log family.
//! - `build_pose_log_manifest(...)` — Pose Log; `source` arg is a
//!   Python dict matching `PoseSource`'s tagged-enum shape (e.g.
//!   `{"kind": "ros2_tf", "publishers": [...]}`).
//! - `build_time_transform_log_manifest(...)` — TimeTransform Log;
//!   `source` arg is a Python dict matching `TimeTransformSource`'s
//!   shape (e.g. `{"kind": "local_clock_read"}`).
//! - `build_detection_log_manifest(...)` — Detection Log; the one
//!   the [ESL detector](https://github.com/aukilabs/detectors) uses.
//!
//! ## Enum seam
//!
//! `PoseSource`, `PoseWriterMode`, and `TimeTransformSource` are
//! Rust tagged enums. The Python surface takes them as **dicts** /
//! strings rather than introducing PyClass equivalents — keeps the
//! Python footprint small and matches the natural JSON shape Python
//! callers already think in. The wrappers parse via `serde_json` into
//! the Rust enum, surfacing decode errors as `ValueError`.
//!
//! ## Out of scope
//!
//! - **`PoseSource::canonical_bytes` / `hash`** — content-addressing
//!   helpers. Python consumers that need them today re-implement the
//!   canonicalize-via-JCS + XXH3 hash dance themselves; expose later
//!   if a real Python consumer needs the graduation primitives. Filed
//!   in [`parking_lot.md`](../parking_lot.md).

use std::time::Duration;

use auki_manifests_rs as manifests;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};

fn pydict_to_json(py: Python<'_>, dict: &Bound<'_, PyDict>) -> PyResult<serde_json::Value> {
    let json = py.import_bound("json")?;
    let s: String = json.call_method1("dumps", (dict,))?.extract()?;
    serde_json::from_str(&s).map_err(|e| PyValueError::new_err(format!("decode: {e}")))
}

fn json_to_pyobject(py: Python<'_>, v: &serde_json::Value) -> PyResult<PyObject> {
    let json = py.import_bound("json")?;
    let s = serde_json::to_string(v)
        .map_err(|e| PyRuntimeError::new_err(format!("internal manifest serialize: {e}")))?;
    Ok(json.call_method1("loads", (s,))?.unbind())
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

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (*, app_id, session_id, sensor_id, sensor_hash, clock_id, clock_hash, segment_duration_ns, retention_ns, frame_id=None, frame_hash=None))]
fn build_sensor_log_manifest(
    py: Python<'_>,
    app_id: &str,
    session_id: &str,
    sensor_id: &str,
    sensor_hash: &str,
    clock_id: &str,
    clock_hash: &str,
    segment_duration_ns: u64,
    retention_ns: u64,
    frame_id: Option<String>,
    frame_hash: Option<String>,
) -> PyResult<PyObject> {
    let m = manifests::build_sensor_log_manifest(
        app_id,
        session_id,
        sensor_id,
        sensor_hash,
        clock_id,
        clock_hash,
        frame_id.as_deref(),
        frame_hash.as_deref(),
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    );
    json_to_pyobject(py, &m)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn build_pose_log_manifest(
    py: Python<'_>,
    app_id: &str,
    session_id: &str,
    from_frame_id: &str,
    from_frame_hash: &str,
    to_frame_id: &str,
    to_frame_hash: &str,
    clock_id: &str,
    clock_hash: &str,
    source: &Bound<'_, PyDict>,
    writer_mode: &str,
    expected_rate_hz: u32,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> PyResult<PyObject> {
    let pose_source = parse_pose_source(py, source)?;
    let mode = parse_pose_writer_mode(writer_mode)?;
    let m = manifests::build_pose_log_manifest(
        app_id,
        session_id,
        from_frame_id,
        from_frame_hash,
        to_frame_id,
        to_frame_hash,
        clock_id,
        clock_hash,
        &pose_source,
        mode,
        expected_rate_hz,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    );
    json_to_pyobject(py, &m)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn build_time_transform_log_manifest(
    py: Python<'_>,
    app_id: &str,
    session_id: &str,
    from_clock_id: &str,
    from_clock_hash: &str,
    to_clock_id: &str,
    to_clock_hash: &str,
    source: &Bound<'_, PyDict>,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> PyResult<PyObject> {
    let tt_source = parse_time_transform_source(py, source)?;
    let m = manifests::build_time_transform_log_manifest(
        app_id,
        session_id,
        from_clock_id,
        from_clock_hash,
        to_clock_id,
        to_clock_hash,
        &tt_source,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    );
    json_to_pyobject(py, &m)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn build_detection_log_manifest(
    py: Python<'_>,
    app_id: &str,
    session_id: &str,
    detector_id: &str,
    detector_hash: &str,
    input_log_id: &str,
    input_sensor_id: &str,
    input_sensor_hash: &str,
    clock_id: &str,
    clock_hash: &str,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> PyResult<PyObject> {
    let m = manifests::build_detection_log_manifest(
        app_id,
        session_id,
        detector_id,
        detector_hash,
        input_log_id,
        input_sensor_id,
        input_sensor_hash,
        clock_id,
        clock_hash,
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
