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
//! │   └── frames/<frame_id>/<hash>.json
//! └── <session>/
//!     ├── timetransform_logs/<from_id>__<to_id>/
//!     │   ├── manifest.json
//!     │   └── segments/<padded-ns>.seg      ← one TT log per session
//!     ├── sensorlogs/
//!     │   ├── <recording_uuid_1>/            ← one sensor stream per recording
//!     │   │   ├── manifest.json
//!     │   │   └── segments/<padded-ns>.seg
//!     │   ├── <recording_uuid_2>/
//!     │   │   └── ...
//!     │   └── <recording_uuid_3>/
//!     └── poselogs/
//!         ├── <recording_uuid_1>/            ← one pose source per recording
//!         │   ├── manifest.json
//!         │   └── segments/<padded-ns>.seg
//!         └── <recording_uuid_2>/
//! ```
//!
//! - `<app_root>` is chosen by the integrator (e.g. `/home/booster/auki/boosterapp/`).
//!   The SDK does not enforce structure above the registries.
//! - Registries live at the app root, **shared across all sessions** of this
//!   app. Hash-keyed writes are idempotent — re-writing identical content per
//!   session would be wasted work.
//! - One TimeTransform Log per session (clock offsets are time-localized;
//!   the session is the natural retention boundary).
//! - **A recording is one stream.** Each recording directory is a complete
//!   `auki-logs` log (manifest + segments) for exactly one sensor (under
//!   `sensorlogs/`) or one pose source (under `poselogs/`). Multi-stream
//!   capture means multiple parallel recordings sharing a session, not a
//!   multi-stream recording. The auto-started ring buffer is simply a
//!   recording with `retention_ns: 30s`; intent captures are recordings with
//!   `retention_ns: 0`. Nothing on disk distinguishes them beyond the
//!   manifest's retention value. For sensor logs, the manifest's `sensor_id`
//!   identifies the sensor; for pose logs, the manifest's `source` field
//!   identifies the producer (e.g. ROS TF, SLAM, odometry).
//!
//! `/` in IDs is replaced with `__` so namespaced ids like
//! `K1-AABBCCDDEEFF/head_left_cam` become a single filesystem-safe directory
//! segment in registry paths. The same convention applies to `from_id`/`to_id`
//! in TimeTransform Log paths.

use std::path::{Path, PathBuf};

const REGISTRIES_DIR: &str = "registries";
const SENSORS_DIR: &str = "sensors";
const CLOCKS_DIR: &str = "clocks";
const FRAMES_DIR: &str = "frames";
const TIMETRANSFORM_LOGS_DIR: &str = "timetransform_logs";
const SENSORLOGS_DIR: &str = "sensorlogs";
const POSELOGS_DIR: &str = "poselogs";

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

/// `<app_root>/registries/frames/<frame_id>/<hash>.json`. Frame Registry
/// entries declare the coordinate convention (handedness, axes, units) of
/// a named coordinate system; sensors and pose-log transforms reference
/// the `frame_id` to make their bytes interpretable to consumers.
pub fn frame_entry_path(app_root: &Path, frame_id: &str, hash: &str) -> PathBuf {
    registries_root(app_root)
        .join(FRAMES_DIR)
        .join(id_to_segment(frame_id))
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

/// `<app_root>/<session>/sensorlogs/<recording_uuid>` — one recording = one
/// sensor stream. The auki-logs `manifest.json` and `segments/` directory
/// live directly under this path. The sensor identity is recorded in the
/// log's manifest (`sensor_id` + `sensor_hash`), not encoded in the path.
pub fn sensorlog_path(session_root: &Path, recording_uuid: &str) -> PathBuf {
    session_root.join(SENSORLOGS_DIR).join(recording_uuid)
}

/// `<app_root>/<session>/poselogs/<recording_uuid>` — one recording = one
/// pose source (e.g. ROS TF, SLAM, odometry). The auki-logs `manifest.json`
/// and `segments/` directory live directly under this path. The source
/// identity is recorded inline in the log's manifest under the `source`
/// field, not encoded in the path. Multiple recordings of the same source
/// per session are fine — a typical session has a 30s rolling buffer plus
/// any number of intent captures, distinguished only by `retention_ns`.
pub fn poselog_path(session_root: &Path, recording_uuid: &str) -> PathBuf {
    session_root.join(POSELOGS_DIR).join(recording_uuid)
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
    fn frame_entry_path_uses_frames_dir() {
        assert_eq!(
            frame_entry_path(
                &app(),
                "K1-AABBCCDDEEFF/head_left_cam_optical",
                "cafef00d"
            ),
            PathBuf::from(
                "/home/booster/auki/boosterapp/registries/frames/\
                 K1-AABBCCDDEEFF__head_left_cam_optical/cafef00d.json"
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
    fn sensorlog_path_is_session_join_sensorlogs_join_recording() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(
            sensorlog_path(&session, "rec-456"),
            PathBuf::from("/home/booster/auki/boosterapp/abc-123/sensorlogs/rec-456")
        );
    }

    #[test]
    fn sensorlog_path_does_not_substitute_recording_uuid() {
        // recording_uuid is opaque to the SDK — it doesn't apply the slash
        // substitution we use for namespaced ids. Callers are responsible
        // for passing a filesystem-safe identifier.
        let session = session_root(&app(), "abc-123");
        let path = sensorlog_path(&session, "rec-456");
        assert!(path.ends_with("rec-456"));
        assert!(!path.to_string_lossy().contains("__"));
    }

    #[test]
    fn id_to_segment_is_idempotent_for_ids_without_slashes() {
        assert_eq!(id_to_segment("plain"), "plain");
        assert_eq!(id_to_segment("K1-AABB__already_subbed"), "K1-AABB__already_subbed");
    }

    #[test]
    fn poselog_path_is_session_join_poselogs_join_recording() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(
            poselog_path(&session, "rec-789"),
            PathBuf::from("/home/booster/auki/boosterapp/abc-123/poselogs/rec-789")
        );
    }

    #[test]
    fn poselog_path_does_not_substitute_recording_uuid() {
        // recording_uuid is opaque to the SDK — same convention as sensorlog_path.
        let session = session_root(&app(), "abc-123");
        let path = poselog_path(&session, "rec-789");
        assert!(path.ends_with("rec-789"));
        assert!(!path.to_string_lossy().contains("__"));
    }
}
