//! PyO3 bindings for [`auki-layout`](../../../../crates/auki-layout) — pure-function
//! wrappers around the on-disk path helpers.
//!
//! Lets a Python consumer (e.g. the ESL detector loop in
//! [`detectors`](https://github.com/aukilabs/detectors)) compute
//! SDK-canonical paths without re-implementing the `__`-substitution
//! and directory-name conventions in Python. Drift risk: if the Rust
//! `id_to_segment` substitution rule changes, Python users who
//! hand-roll the path concat would silently break — these wrappers
//! keep both sides reading from one source of truth.
//!
//! Surface — every Rust path helper exposed as a `#[pyfunction]`,
//! returning `str` (PathBuf converts via `to_string_lossy`):
//!
//! - `registries_root(app_root)`
//! - `sensor_entry_path(app_root, peer_id, sensor_id, hash)`
//! - `clock_entry_path(app_root, peer_id, clock_id, hash)`
//! - `frame_entry_path(app_root, peer_id, frame_id, hash)`
//! - `session_root(app_root, session)`
//! - `timetransform_log_path(session_root, from_id, to_id)`
//! - `sensorlog_path(session_root, sensor_log_id)`
//! - `poselog_path(session_root, from_frame_id, to_frame_id)`
//! - `detection_log_path(session_root, detector_id, input_log_id)`
//! - `id_to_segment(id)`
//!
//! No state, no PyClasses. Each call is a thin Rust wrapper.

// PyO3 0.22 proc-macro expansions trigger these Rust 2024/Clippy lints. They
// cannot be corrected in handwritten wrapper code without changing the shared
// binding ABI, so scope the compatibility allowance to this crate.
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::useless_conversion)]

use std::path::PathBuf;

use auki_layout_rs as layout;
use pyo3::prelude::*;
use pyo3::types::PyModule;

fn pathbuf_to_string(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

#[pyfunction]
fn registries_root(app_root: PathBuf) -> String {
    pathbuf_to_string(layout::registries_root(&app_root))
}

#[pyfunction]
fn sensor_entry_path(app_root: PathBuf, peer_id: &str, sensor_id: &str, hash: &str) -> String {
    pathbuf_to_string(layout::sensor_entry_path(
        &app_root, peer_id, sensor_id, hash,
    ))
}

#[pyfunction]
fn clock_entry_path(app_root: PathBuf, peer_id: &str, clock_id: &str, hash: &str) -> String {
    pathbuf_to_string(layout::clock_entry_path(&app_root, peer_id, clock_id, hash))
}

#[pyfunction]
fn frame_entry_path(app_root: PathBuf, peer_id: &str, frame_id: &str, hash: &str) -> String {
    pathbuf_to_string(layout::frame_entry_path(&app_root, peer_id, frame_id, hash))
}

#[pyfunction]
fn session_root(app_root: PathBuf, session: &str) -> String {
    pathbuf_to_string(layout::session_root(&app_root, session))
}

#[pyfunction]
fn timetransform_log_path(session_root: PathBuf, from_id: &str, to_id: &str) -> String {
    pathbuf_to_string(layout::timetransform_log_path(
        &session_root,
        from_id,
        to_id,
    ))
}

#[pyfunction]
fn sensorlog_path(session_root: PathBuf, sensor_log_id: &str) -> String {
    pathbuf_to_string(layout::sensorlog_path(&session_root, sensor_log_id))
}

#[pyfunction]
fn poselog_path(session_root: PathBuf, from_frame_id: &str, to_frame_id: &str) -> String {
    pathbuf_to_string(layout::poselog_path(
        &session_root,
        from_frame_id,
        to_frame_id,
    ))
}

#[pyfunction]
fn detection_log_path(session_root: PathBuf, detector_id: &str, input_log_id: &str) -> String {
    pathbuf_to_string(layout::detection_log_path(
        &session_root,
        detector_id,
        input_log_id,
    ))
}

#[pyfunction]
fn id_to_segment(id: &str) -> String {
    layout::id_to_segment(id)
}

/// Module entry point. The `#[pymodule]` macro generates the
/// `PyInit_auki_layout` symbol the host interpreter resolves at
/// `import auki_layout`.
#[pymodule]
fn auki_layout(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(registries_root, m)?)?;
    m.add_function(wrap_pyfunction!(sensor_entry_path, m)?)?;
    m.add_function(wrap_pyfunction!(clock_entry_path, m)?)?;
    m.add_function(wrap_pyfunction!(frame_entry_path, m)?)?;
    m.add_function(wrap_pyfunction!(session_root, m)?)?;
    m.add_function(wrap_pyfunction!(timetransform_log_path, m)?)?;
    m.add_function(wrap_pyfunction!(sensorlog_path, m)?)?;
    m.add_function(wrap_pyfunction!(poselog_path, m)?)?;
    m.add_function(wrap_pyfunction!(detection_log_path, m)?)?;
    m.add_function(wrap_pyfunction!(id_to_segment, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Rust-side smoke tests against the rlib. Verify the helper
    //! wrappers reach the upstream Rust functions and do the
    //! `__`-substitution correctly. The Python surface is exercised
    //! in `python_tests/`.

    use super::*;

    #[test]
    fn detection_log_path_substitutes_slashes_in_detector_id() {
        let s = detection_log_path(PathBuf::from("/session"), "aukilabs/qr/v1", "rec-456");
        assert_eq!(s, "/session/detection_logs/aukilabs__qr__v1__rec-456");
    }

    #[test]
    fn sensor_entry_path_includes_id_subst_and_hash_filename() {
        let s = sensor_entry_path(
            PathBuf::from("/app"),
            "galbot",
            "K1-AABBCCDDEEFF/head_left_cam",
            "deadbeef",
        );
        assert_eq!(
            s,
            "/app/registries/sensors/galbot/K1-AABBCCDDEEFF__head_left_cam/deadbeef.json"
        );
    }

    #[test]
    fn id_to_segment_substitutes_slashes() {
        assert_eq!(id_to_segment("foo/bar/baz"), "foo__bar__baz");
        assert_eq!(id_to_segment("plain"), "plain");
    }
}
