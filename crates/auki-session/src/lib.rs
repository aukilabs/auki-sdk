//! Path helpers for the Auki SDK on-disk session shape.
//!
//! The on-disk layout used by the SDK and consumers (renderers, analysis
//! tools) is:
//!
//! ```text
//! <app_root>/
//! ├── registries/
//! │   ├── sensors/<sensor_id>/<hash>.json   ← shared across all sessions of this app
//! │   ├── clocks/<clock_id>/<hash>.json
//! │   └── frames/<frame_id>/<hash>.json     ← coming
//! └── <session>/
//!     ├── timetransform_logs/<from_id>__<to_id>/
//!     │   ├── manifest.json
//!     │   └── segments/<padded-ns>.seg      ← one TT log per session
//!     └── sensorlogs/
//!         └── <recording_uuid>/<sensor_id>/
//!             ├── manifest.json              ← one sensor log per recording
//!             └── segments/<padded-ns>.seg
//! ```
//!
//! - `<app_root>` is chosen by the integrator (e.g. `/home/booster/auki/boosterapp/`).
//!   The SDK does not enforce structure above the registries.
//! - Registries live at the app root, **shared across all sessions** of this
//!   app. Hash-keyed writes are idempotent — re-writing identical content per
//!   session would be wasted work.
//! - One TimeTransform Log per session (clock offsets are time-localized;
//!   the session is the natural retention boundary).
//! - Sensor logs are per-recording. The `<recording_uuid>` layer lets one
//!   session hold multiple recordings (e.g. an auto-started rolling buffer
//!   alongside on-demand intent captures); they're uniform on disk and
//!   distinguished only by the `retention_ns` in their manifests.
//!
//! `/` in IDs is replaced with `__` so namespaced ids like
//! `K1-AABBCCDDEEFF/head_left_cam` become a single filesystem-safe directory
//! segment. The same convention applies to `from_id`/`to_id` in TimeTransform
//! Log paths.

use std::path::{Path, PathBuf};

const REGISTRIES_DIR: &str = "registries";
const SENSORS_DIR: &str = "sensors";
const CLOCKS_DIR: &str = "clocks";
const TIMETRANSFORM_LOGS_DIR: &str = "timetransform_logs";
const SENSORLOGS_DIR: &str = "sensorlogs";

/// `<app_root>/registries`.
pub fn registries_root(app_root: &Path) -> PathBuf {
    app_root.join(REGISTRIES_DIR)
}

/// `<app_root>/registries/sensors/<sensor_id>/<hash>.json`.
pub fn sensor_entry_path(app_root: &Path, sensor_id: &str, hash: &str) -> PathBuf {
    registries_root(app_root)
        .join(SENSORS_DIR)
        .join(id_to_segment(sensor_id))
        .join(format!("{hash}.json"))
}

/// `<app_root>/registries/clocks/<clock_id>/<hash>.json`.
pub fn clock_entry_path(app_root: &Path, clock_id: &str, hash: &str) -> PathBuf {
    registries_root(app_root)
        .join(CLOCKS_DIR)
        .join(id_to_segment(clock_id))
        .join(format!("{hash}.json"))
}

/// `<app_root>/<session>` — root of a single session.
pub fn session_root(app_root: &Path, session: &str) -> PathBuf {
    app_root.join(session)
}

/// `<app_root>/<session>/timetransform_logs/<from_id>__<to_id>` — one TT log
/// per ordered clock pair per session. The auki-logs `manifest.json` and
/// `segments/` directory live directly under this path.
pub fn timetransform_log_path(session_root: &Path, from_id: &str, to_id: &str) -> PathBuf {
    session_root.join(TIMETRANSFORM_LOGS_DIR).join(format!(
        "{}__{}",
        id_to_segment(from_id),
        id_to_segment(to_id)
    ))
}

/// `<app_root>/<session>/sensorlogs/<recording_uuid>/<sensor_id>` — one sensor
/// log per recording. The auki-logs `manifest.json` and `segments/` directory
/// live directly under this path.
pub fn sensorlog_path(
    session_root: &Path,
    recording_uuid: &str,
    sensor_id: &str,
) -> PathBuf {
    session_root
        .join(SENSORLOGS_DIR)
        .join(recording_uuid)
        .join(id_to_segment(sensor_id))
}

/// Replace `/` with `__` so namespaced ids become a single filesystem-safe
/// directory segment. Mirrors the convention used inside `auki-registry` for
/// registry-entry paths.
pub fn id_to_segment(id: &str) -> String {
    id.replace('/', "__")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn app() -> PathBuf {
        PathBuf::from("/home/booster/auki/boosterapp")
    }

    #[test]
    fn registries_root_is_under_app() {
        assert_eq!(
            registries_root(&app()),
            PathBuf::from("/home/booster/auki/boosterapp/registries")
        );
    }

    #[test]
    fn sensor_entry_path_includes_id_substitution_and_hash_filename() {
        assert_eq!(
            sensor_entry_path(
                &app(),
                "K1-AABBCCDDEEFF/head_left_cam",
                "e8cb3879fcfa7f716047aa0892b0c0c0",
            ),
            PathBuf::from(
                "/home/booster/auki/boosterapp/registries/sensors/\
                 K1-AABBCCDDEEFF__head_left_cam/e8cb3879fcfa7f716047aa0892b0c0c0.json"
            )
        );
    }

    #[test]
    fn clock_entry_path_uses_clocks_dir() {
        assert_eq!(
            clock_entry_path(&app(), "K1-AABBCCDDEEFF/utc", "deadbeef"),
            PathBuf::from(
                "/home/booster/auki/boosterapp/registries/clocks/\
                 K1-AABBCCDDEEFF__utc/deadbeef.json"
            )
        );
    }

    #[test]
    fn session_root_is_app_join_session_uuid() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(session, PathBuf::from("/home/booster/auki/boosterapp/abc-123"));
    }

    #[test]
    fn timetransform_log_path_uses_double_underscore_separator() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(
            timetransform_log_path(&session, "K1-AABB/utc", "K1-AABB/monotonic"),
            PathBuf::from(
                "/home/booster/auki/boosterapp/abc-123/timetransform_logs/\
                 K1-AABB__utc__K1-AABB__monotonic"
            )
        );
    }

    #[test]
    fn sensorlog_path_includes_recording_layer() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(
            sensorlog_path(&session, "rec-456", "K1-AABB/head_left_cam"),
            PathBuf::from(
                "/home/booster/auki/boosterapp/abc-123/sensorlogs/\
                 rec-456/K1-AABB__head_left_cam"
            )
        );
    }

    #[test]
    fn id_to_segment_is_idempotent_for_ids_without_slashes() {
        assert_eq!(id_to_segment("plain"), "plain");
        assert_eq!(id_to_segment("K1-AABB__already_subbed"), "K1-AABB__already_subbed");
    }
}
