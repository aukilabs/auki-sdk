//! Regenerates the canonical-JSON locked fixtures under
//! `crates/auki-manifests/tests/locked/`.
//!
//! Run after any intentional schema change to commit updated fixtures:
//!
//! ```sh
//! cargo run -p auki-manifests --bin regen_locked_fixtures
//! ```
//!
//! Hash values for `RegistryRef` fields are computed from the same registry
//! entry shapes that `auki-registry`'s own regen binary produces; running
//! `cargo run -p auki-registry --bin regen_locked_fixtures` first keeps the
//! two sets of fixtures in sync.

use auki_manifests::{
    DetectionLogManifest, PoseLogManifest, PoseSource, PoseWriterMode, SensorLogManifest,
    TimeTransformLogManifest, TimeTransformSource,
};
use auki_registry::{LogRef, RegistryRef};
use std::fs;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from("crates/auki-manifests/tests/locked");
    fs::create_dir_all(&out).unwrap();

    let cases: Vec<(&str, String)> = vec![
        (
            "sensor_log_origin.json",
            canonical(make_sensor_log_origin()),
        ),
        (
            "sensor_log_materialized.json",
            canonical(make_sensor_log_materialized()),
        ),
        ("pose_log_rigid.json", canonical(make_pose_log_rigid())),
        ("pose_log_movable.json", canonical(make_pose_log_movable())),
        (
            "time_transform_log.json",
            canonical(make_time_transform_log()),
        ),
        ("detection_log.json", canonical(make_detection_log())),
    ];

    for (name, json) in cases {
        let path = out.join(name);
        fs::write(&path, json + "\n").unwrap();
        println!("wrote {name}");
    }
}

fn canonical<T: serde::Serialize>(v: T) -> String {
    let value = serde_json::to_value(&v).unwrap();
    let bytes = auki_jcs::canonicalize(&value);
    String::from_utf8(bytes).unwrap()
}

// ─── Shared RegistryRef constants ────────────────────────────────────────────
//
// Hashes are computed from the corresponding auki-registry entry shapes (same
// peer_id / id / canonical JSON). Running `cargo run -p auki-registry
// --bin regen_locked_fixtures` regenerates those entries and the hashes below
// must track them.

/// Galbot's head_left_rgb sensor (Camera/rgb, 1920x1200 @ 30 Hz).
fn sensor_head_left_rgb() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "head_left_rgb".into(),
        // auki-registry sensor_camera_rgb.json hash
        hash: "8295922307fa2b426453486ba87a59ef".into(),
    }
}

/// Galbot's head_left_camera_optical ROS-optical frame.
fn frame_head_left_camera_optical() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "head_left_camera_optical".into(),
        // auki-registry frame_ros_optical.json hash
        hash: "3055a8ac27eecd57aa37235b05871c01".into(),
    }
}

/// Galbot's session/sdk_clock (device-local monotonic, nanoseconds).
fn clock_sdk(owner: &str) -> RegistryRef {
    RegistryRef {
        peer_id: owner.into(),
        id: "session/sdk_clock".into(),
        // auki-registry clock_monotonic.json hash
        hash: "fb0120a35f4fd1de8a7d46f5e76b7f68".into(),
    }
}

/// Galbot's wall_clock (UTC, nanoseconds).
fn clock_wall() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "wall_clock".into(),
        // auki-registry clock_utc.json hash
        hash: "a149244453307763c261dc1759be5373".into(),
    }
}

/// Park's world frame (ROS-body convention).
fn frame_park_world() -> RegistryRef {
    RegistryRef {
        peer_id: "park".into(),
        id: "world".into(),
        // Derived from FrameRegistryEntry::ros_body("park", "world").hash()
        hash: "2cbe0d5894d3346daa167ecb077cce98".into(),
    }
}

/// Galbot's base_link frame (ROS-body convention).
fn frame_galbot_base_link() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "base_link".into(),
        // Derived from FrameRegistryEntry::ros_body("galbot", "base_link").hash()
        hash: "16645bd8584e5ca5fa1030b439da92dc".into(),
    }
}

/// Galbot's yolo_v8 detector (ObjectDetection, model yolo_v8n).
fn detector_yolo_v8() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "yolo_v8".into(),
        // auki-registry detector_object_detection.json hash
        hash: "031308c146a2f6f086713388dc01f60e".into(),
    }
}

// ─── Fixture constructors ─────────────────────────────────────────────────────

/// Origin log: source == writer == "galbot". Galbot records its own sensor
/// data directly; no remote materialization.
fn make_sensor_log_origin() -> SensorLogManifest {
    SensorLogManifest {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        app_id: "galbot-control-plane".into(),
        session_id: "01HV-galbot-session".into(),
        sensor: sensor_head_left_rgb(),
        clock: clock_sdk("galbot"),
        frame: Some(frame_head_left_camera_optical()),
        segment_duration_ns: 1_000_000_000,
        retention_ns: 5_000_000_000,
    }
}

/// Materialized log: source_peer_id = "galbot", writer_peer_id = "park".
/// Park re-materialized Galbot's sensor log into its own storage with
/// bigger segments and longer retention.
fn make_sensor_log_materialized() -> SensorLogManifest {
    SensorLogManifest {
        source_peer_id: "galbot".into(),
        writer_peer_id: "park".into(),
        app_id: "park-vis".into(),
        session_id: "01HV-park-session".into(),
        // Sensor and clock still reference Galbot's registry entries.
        sensor: sensor_head_left_rgb(),
        clock: clock_sdk("galbot"),
        // Park's materialization omits the per-pixel frame reference
        // (Park re-stores the data but doesn't re-project it into a
        // different optical frame).
        frame: Some(frame_head_left_camera_optical()),
        segment_duration_ns: 10_000_000_000, // Park uses 10-second segments
        retention_ns: 300_000_000_000,       // Park retains 5 minutes
    }
}

/// Rigid pose log: from park's world frame to galbot's base_link.
/// writer_mode = "rigid" — static transform (calibrated mount, doesn't
/// move between samples). One sample is authoritative for the whole log.
fn make_pose_log_rigid() -> PoseLogManifest {
    PoseLogManifest {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        app_id: "galbot-control-plane".into(),
        session_id: "01HV-galbot-session".into(),
        from_frame: frame_park_world(),
        to_frame: frame_galbot_base_link(),
        clock: clock_sdk("galbot"),
        source: PoseSource::Manual,
        writer_mode: PoseWriterMode::Rigid,
        expected_rate_hz: 1,
        segment_duration_ns: 1_000_000_000,
        retention_ns: 0, // Rigid logs never evict — retain forever
    }
}

/// Movable pose log: from park's world frame to galbot's base_link.
/// writer_mode = "movable" — live SLAM/odometry; readers interpolate.
fn make_pose_log_movable() -> PoseLogManifest {
    PoseLogManifest {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        app_id: "galbot-control-plane".into(),
        session_id: "01HV-galbot-session".into(),
        from_frame: frame_park_world(),
        to_frame: frame_galbot_base_link(),
        clock: clock_sdk("galbot"),
        source: PoseSource::Ros2Tf {
            publishers: vec!["robot_state_publisher".into(), "tf_broadcaster".into()],
        },
        writer_mode: PoseWriterMode::Movable,
        expected_rate_hz: 30,
        segment_duration_ns: 1_000_000_000,
        retention_ns: 60_000_000_000, // 1 minute rolling window
    }
}

/// Time-transform log: from session/sdk_clock to wall_clock.
/// source = LocalClockRead (the 1 Hz clock_gettime sampler).
fn make_time_transform_log() -> TimeTransformLogManifest {
    TimeTransformLogManifest {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        app_id: "galbot-control-plane".into(),
        session_id: "01HV-galbot-session".into(),
        from_clock: clock_sdk("galbot"),
        to_clock: clock_wall(),
        source: TimeTransformSource::LocalClockRead,
        segment_duration_ns: 60_000_000_000, // 1-minute segments
        retention_ns: 3_600_000_000_000,     // 1-hour retention
    }
}

/// Detection log: yolo_v8 detector reading galbot's head_left_rgb sensor log.
fn make_detection_log() -> DetectionLogManifest {
    DetectionLogManifest {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        app_id: "galbot-control-plane".into(),
        session_id: "01HV-galbot-session".into(),
        detector: detector_yolo_v8(),
        input_log: LogRef {
            source_peer_id: "galbot".into(),
            resource_id: "head_left_rgb".into(),
        },
        input_sensor: sensor_head_left_rgb(),
        clock: clock_sdk("galbot"),
        segment_duration_ns: 1_000_000_000,
        retention_ns: 60_000_000_000,
    }
}
