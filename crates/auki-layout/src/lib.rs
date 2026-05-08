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
//!     │   ├── <sensor_log_id_1>/             ← one sensor stream per log
//!     │   │   ├── manifest.json
//!     │   │   └── segments/<padded-ns>.seg
//!     │   ├── <sensor_log_id_2>/
//!     │   │   └── ...
//!     │   └── <sensor_log_id_3>/
//!     └── poselogs/
//!         ├── <pose_log_id_1>/               ← one pose source per log
//!         │   ├── manifest.json
//!         │   └── segments/<padded-ns>.seg
//!         └── <pose_log_id_2>/
//! ```
//!
//! - `<app_root>` is chosen by the integrator (e.g. `/home/booster/auki/boosterapp/`).
//!   The SDK does not enforce structure above the registries.
//! - Registries live at the app root, **shared across all sessions** of this
//!   app. Hash-keyed writes are idempotent — re-writing identical content per
//!   session would be wasted work.
//! - One TimeTransform Log per session (clock offsets are time-localized;
//!   the session is the natural retention boundary).
//! - **A log is one stream.** Each `<sensor_log_id>/` or `<pose_log_id>/`
//!   directory is a complete `auki-logs` log (manifest + segments) for
//!   exactly one sensor (under `sensorlogs/`) or one pose source (under
//!   `poselogs/`). Multi-stream capture means multiple parallel logs sharing
//!   a session, not a multi-stream log. Buffers, intent recordings, and
//!   time-bounded captures are all the same kind of log on disk — they
//!   differ only in their manifest's `retention_ns` (backward window kept on
//!   disk; `0` = no eviction). Whether a daemon auto-creates any log at
//!   session boot is daemon-application policy, not SDK contract — see the
//!   [Control API spec](../../../docs/control-api.md). For sensor logs, the
//!   manifest's `sensor_id` identifies the sensor; for pose logs, the
//!   manifest's `source` field identifies the producer (e.g. ROS TF, SLAM,
//!   odometry).
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

/// `<app_root>/<session>/sensorlogs/<sensor_log_id>` — one sensor log = one
/// sensor stream. The auki-logs `manifest.json` and `segments/` directory
/// live directly under this path. The sensor identity is recorded in the
/// log's manifest (`sensor_id` + `sensor_hash`), not encoded in the path.
/// `sensor_log_id` is opaque to the SDK; the integrator (or daemon) mints
/// a filesystem-safe identifier (typically a UUID) when opening the log.
pub fn sensorlog_path(session_root: &Path, sensor_log_id: &str) -> PathBuf {
    session_root.join(SENSORLOGS_DIR).join(sensor_log_id)
}

/// `<app_root>/<session>/poselogs/<from_id>__<to_id>` — one pose log per
/// `(from_frame_id, to_frame_id)` pair per session. Mirrors the
/// [`timetransform_log_path`] shape (one TT log per ordered clock
/// pair). The auki-logs `manifest.json` and `segments/` directory live
/// directly under this path; the producer identity is recorded inline
/// in the log's manifest under the `source` field, not encoded in the
/// path.
///
/// Step 5 of the [`auki-datatypes` migration] (2026-05-08) reshaped
/// the Pose Log to per-`(from, to)` identity — pre-migration logs
/// keyed on an opaque `pose_log_id`. A producer that observes a
/// multi-pair ROS `TFMessage` is responsible for fanning the message
/// into N parallel pose logs.
pub fn poselog_path(session_root: &Path, from_frame_id: &str, to_frame_id: &str) -> PathBuf {
    session_root.join(POSELOGS_DIR).join(format!(
        "{}__{}",
        id_to_segment(from_frame_id),
        id_to_segment(to_frame_id)
    ))
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
    fn session_root_is_app_join_session_id() {
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
    fn sensorlog_path_is_session_join_sensorlogs_join_sensor_log_id() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(
            sensorlog_path(&session, "rec-456"),
            PathBuf::from("/home/booster/auki/boosterapp/abc-123/sensorlogs/rec-456")
        );
    }

    #[test]
    fn sensorlog_path_does_not_substitute_sensor_log_id() {
        // sensor_log_id is opaque to the SDK — it doesn't apply the slash
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
    fn poselog_path_uses_double_underscore_separator() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(
            poselog_path(&session, "K1-AABB/base_link", "K1-AABB/head_left_cam_optical"),
            PathBuf::from(
                "/home/booster/auki/boosterapp/abc-123/poselogs/\
                 K1-AABB__base_link__K1-AABB__head_left_cam_optical"
            )
        );
    }

    #[test]
    fn poselog_path_substitutes_slashes_inside_each_frame_id() {
        // mirrors timetransform_log_path: each side's `/` becomes `__`,
        // then the two sides join with another `__`.
        let session = session_root(&app(), "abc-123");
        let path = poselog_path(&session, "from/x", "to/y");
        assert!(path.ends_with("from__x__to__y"));
    }
}
