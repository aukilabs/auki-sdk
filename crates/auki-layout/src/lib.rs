//! Path helpers for the Auki SDK on-disk session shape.
//!
//! The on-disk layout used by the SDK and consumers (renderers, analysis
//! tools) is:
//!
//! ```text
//! <app_root>/
//! ├── registries/
//! │   ├── sensors/<peer_id>/<sensor_id>/<hash>.json       ← shared across all sessions of this app
//! │   ├── clocks/<peer_id>/<clock_id>/<hash>.json
//! │   ├── frames/<peer_id>/<frame_id>/<hash>.json
//! │   └── detectors/<peer_id>/<detector_id>/<hash>.json   ← Cuba T4
//! └── <session>/
//!     ├── timetransform_logs/<from_id>__<to_id>/
//!     │   ├── log_manifest.json
//!     │   └── segments/<padded-ns>.seg      ← one TT log per session
//!     ├── sensorlogs/
//!     │   ├── <sensor_log_id_1>/             ← one sensor stream per log
//!     │   │   ├── log_manifest.json
//!     │   │   └── segments/<padded-ns>.seg
//!     │   ├── <sensor_log_id_2>/
//!     │   │   └── ...
//!     │   └── <sensor_log_id_3>/
//!     ├── poselogs/
//!     │   ├── <pose_log_id_1>/               ← one pose source per log
//!     │   │   ├── log_manifest.json
//!     │   │   └── segments/<padded-ns>.seg
//!     │   └── <pose_log_id_2>/
//!     └── detection_logs/
//!         ├── <detector_id>__<input_log_id>/ ← one Detector × input sensor log
//!         │   ├── log_manifest.json
//!         │   └── segments/<padded-ns>.seg
//!         └── ...
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
const DETECTORS_DIR: &str = "detectors"; // Cuba T4
const TIMETRANSFORM_LOGS_DIR: &str = "timetransform_logs";
const SENSORLOGS_DIR: &str = "sensorlogs";
const POSELOGS_DIR: &str = "poselogs";
const DETECTION_LOGS_DIR: &str = "detection_logs";

/// `<app_root>/registries`.
pub fn registries_root(app_root: &Path) -> PathBuf {
    app_root.join(REGISTRIES_DIR)
}

/// `<app_root>/registries/sensors/<peer_id>/<sensor_id>/<hash>.json`.
pub fn sensor_entry_path(app_root: &Path, peer_id: &str, sensor_id: &str, hash: &str) -> PathBuf {
    registries_root(app_root)
        .join(SENSORS_DIR)
        .join(id_to_segment(peer_id))
        .join(id_to_segment(sensor_id))
        .join(format!("{hash}.json"))
}

/// `<app_root>/registries/clocks/<peer_id>/<clock_id>/<hash>.json`.
pub fn clock_entry_path(app_root: &Path, peer_id: &str, clock_id: &str, hash: &str) -> PathBuf {
    registries_root(app_root)
        .join(CLOCKS_DIR)
        .join(id_to_segment(peer_id))
        .join(id_to_segment(clock_id))
        .join(format!("{hash}.json"))
}

/// `<app_root>/registries/frames/<peer_id>/<frame_id>/<hash>.json`. Frame Registry
/// entries declare the coordinate convention (handedness, axes, units) of
/// a named coordinate system; sensors and pose-log transforms reference
/// the `frame_id` to make their bytes interpretable to consumers.
pub fn frame_entry_path(app_root: &Path, peer_id: &str, frame_id: &str, hash: &str) -> PathBuf {
    registries_root(app_root)
        .join(FRAMES_DIR)
        .join(id_to_segment(peer_id))
        .join(id_to_segment(frame_id))
        .join(format!("{hash}.json"))
}

/// `<app_root>/registries/detectors/<peer_id>/<detector_id>/<hash>.json`. Detector
/// Registry entries declare what one Detector *is* — its identity
/// (`detector_id`), the body fields the detector needs to interpret
/// itself (`DetectorBody`, e.g. ArUco dictionary), and the detection
/// types it emits (`output_types`).
///
/// Cuba T4 / T16. Same content-addressed multi-version-by-hash shape as
/// [`sensor_entry_path`]: consumers resolve a detector's body and
/// output types via the `(detector_id, detector_hash)` pair, just like
/// sensors. A `DetectionFrame`'s `sensor_hash` (Cuba T5) carries
/// the input-frame provenance; the detector entry covers everything
/// else.
pub fn detector_entry_path(
    app_root: &Path,
    peer_id: &str,
    detector_id: &str,
    hash: &str,
) -> PathBuf {
    registries_root(app_root)
        .join(DETECTORS_DIR)
        .join(id_to_segment(peer_id))
        .join(id_to_segment(detector_id))
        .join(format!("{hash}.json"))
}

/// `<app_root>/<session>` — root of a single session.
pub fn session_root(app_root: &Path, session: &str) -> PathBuf {
    app_root.join(session)
}

/// `<app_root>/<session>/timetransform_logs/<from_id>__<to_id>` — one TT log
/// per ordered clock pair per session. The auki-logs `log_manifest.json` and
/// `segments/` directory live directly under this path.
pub fn timetransform_log_path(session_root: &Path, from_id: &str, to_id: &str) -> PathBuf {
    session_root.join(TIMETRANSFORM_LOGS_DIR).join(format!(
        "{}__{}",
        id_to_segment(from_id),
        id_to_segment(to_id)
    ))
}

/// `<app_root>/<session>/sensorlogs/<sensor_log_id>` — one sensor log = one
/// sensor stream. The auki-logs `log_manifest.json` and `segments/` directory
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
/// pair). The auki-logs `log_manifest.json` and `segments/` directory live
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

/// `<app_root>/<session>/detection_logs/<detector_id>__<input_log_id>` —
/// one detection log per `(detector, input sensor log)` pair within a
/// session. The auki-logs `log_manifest.json` and `segments/` directory live
/// directly under this path; the producer identity is recorded inline in
/// the log's manifest under `detector_id` + `detector_hash`, mirroring
/// how Sensor Log carries `sensor_id` + `sensor_hash`.
///
/// `detector_id` is namespaced (e.g. `"aukilabs/qr/v1"`) and uses the
/// same `__` substitution as sibling helpers; `input_log_id` is the
/// `sensor_log_id` from [`sensorlog_path`] — typically a UUID minted by
/// the integrator when the sensor log was opened.
///
/// Closes blocker #2 of [`detectors`](https://github.com/aukilabs/detectors)
/// phase 2; the [subscription-as-materialization keystone](../../../parking_lot.md)
/// applies — the same path shape works whether the input sensor log is
/// being written by a local driver, materialized from a peer's stream,
/// or opened from a recording.
pub fn detection_log_path(session_root: &Path, detector_id: &str, input_log_id: &str) -> PathBuf {
    session_root.join(DETECTION_LOGS_DIR).join(format!(
        "{}__{}",
        id_to_segment(detector_id),
        id_to_segment(input_log_id)
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
    fn sensor_entry_path_includes_peer_id_and_id_substitution_and_hash_filename() {
        assert_eq!(
            sensor_entry_path(
                &app(),
                "galbot",
                "K1-AABBCCDDEEFF/head_left_cam",
                "e8cb3879fcfa7f716047aa0892b0c0c0",
            ),
            PathBuf::from(
                "/home/booster/auki/boosterapp/registries/sensors/\
                 galbot/K1-AABBCCDDEEFF__head_left_cam/e8cb3879fcfa7f716047aa0892b0c0c0.json"
            )
        );
    }

    #[test]
    fn clock_entry_path_uses_clocks_dir_and_includes_peer_id() {
        assert_eq!(
            clock_entry_path(&app(), "galbot", "K1-AABBCCDDEEFF/utc", "deadbeef"),
            PathBuf::from(
                "/home/booster/auki/boosterapp/registries/clocks/\
                 galbot/K1-AABBCCDDEEFF__utc/deadbeef.json"
            )
        );
    }

    #[test]
    fn frame_entry_path_uses_frames_dir_and_includes_peer_id() {
        assert_eq!(
            frame_entry_path(
                &app(),
                "galbot",
                "K1-AABBCCDDEEFF/head_left_cam_optical",
                "cafef00d"
            ),
            PathBuf::from(
                "/home/booster/auki/boosterapp/registries/frames/\
                 galbot/K1-AABBCCDDEEFF__head_left_cam_optical/cafef00d.json"
            )
        );
    }

    #[test]
    fn detector_entry_path_uses_detectors_dir_and_id_substitution_and_includes_peer_id() {
        assert_eq!(
            detector_entry_path(&app(), "galbot", "aukilabs/aruco/v1", "f00dcafe"),
            PathBuf::from(
                "/home/booster/auki/boosterapp/registries/detectors/\
                 galbot/aukilabs__aruco__v1/f00dcafe.json"
            )
        );
    }

    #[test]
    fn session_root_is_app_join_session_id() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(
            session,
            PathBuf::from("/home/booster/auki/boosterapp/abc-123")
        );
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
        assert_eq!(
            id_to_segment("K1-AABB__already_subbed"),
            "K1-AABB__already_subbed"
        );
    }

    #[test]
    fn poselog_path_uses_double_underscore_separator() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(
            poselog_path(
                &session,
                "K1-AABB/base_link",
                "K1-AABB/head_left_cam_optical"
            ),
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

    #[test]
    fn detection_log_path_keys_on_detector_id_and_input_log_id() {
        let session = session_root(&app(), "abc-123");
        assert_eq!(
            detection_log_path(&session, "aukilabs/qr/v1", "rec-456"),
            PathBuf::from(
                "/home/booster/auki/boosterapp/abc-123/detection_logs/\
                 aukilabs__qr__v1__rec-456"
            )
        );
    }

    #[test]
    fn detection_log_path_substitutes_slashes_in_detector_id_only() {
        // Detector IDs are namespaced (`aukilabs/qr/v1`) and get `/` →
        // `__`. The input_log_id is opaque (typically a UUID) and is
        // not transformed — same convention as `sensorlog_path`.
        let session = session_root(&app(), "abc-123");
        let path = detection_log_path(&session, "qr/v1", "550e8400-e29b-41d4-a716-446655440000");
        assert!(path.ends_with("qr__v1__550e8400-e29b-41d4-a716-446655440000"));
    }
}
