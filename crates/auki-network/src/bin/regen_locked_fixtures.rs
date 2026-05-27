//! Regenerates the canonical-JSON locked fixtures under
//! `crates/auki-network/tests/locked/`.
//!
//! Run after any intentional schema change to commit updated fixtures:
//!
//! ```sh
//! cargo run -p auki-network --bin regen_locked_fixtures --features swarm
//! ```
//!
//! Hash values for `RegistryRef` fields match the corresponding
//! `auki-registry` locked fixtures. Run
//! `cargo run -p auki-registry --bin regen_locked_fixtures` first to keep
//! the two sets of fixtures in sync.

use auki_manifests::{PoseSource, PoseWriterMode, TimeTransformSource};
use auki_network::resources_protocol::{
    Available, Extent, Head, PoseBlock, PoseManifestPointer, ResourceEntry,
    SensorBlock, SensorKind, SensorManifestPointer, TimeTransformManifestPointer,
    DetectionManifestPointer, VariantContent,
};
use auki_network::stream_protocol::{ReadFrom, StreamRequest};
use auki_registry::{LogRef, RegistryRef};
use std::fs;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from("crates/auki-network/tests/locked");
    fs::create_dir_all(&out).unwrap();

    let cases: Vec<(&str, String)> = vec![
        (
            "catalog_row_sensor_log_camera_live_rolling.json",
            canonical(&make_sensor_camera_live_rolling()),
        ),
        (
            "catalog_row_sensor_log_rangefinder_live_rolling.json",
            canonical(&make_sensor_rangefinder_live_rolling()),
        ),
        (
            "catalog_row_sensor_log_sealed.json",
            canonical(&make_sensor_sealed()),
        ),
        (
            "catalog_row_sensor_log_materialization.json",
            canonical(&make_sensor_materialization()),
        ),
        (
            "catalog_row_pose_log_movable_live_fixed.json",
            canonical(&make_pose_movable_live_fixed()),
        ),
        (
            "catalog_row_pose_log_rigid_sealed.json",
            canonical(&make_pose_rigid_sealed()),
        ),
        (
            "catalog_row_time_transform_log.json",
            canonical(&make_time_transform()),
        ),
        (
            "catalog_row_detection_log.json",
            canonical(&make_detection()),
        ),
        ("stream_request.json", canonical(&make_stream_request())),
    ];

    for (name, json) in cases {
        let path = out.join(name);
        fs::write(&path, json + "\n").unwrap();
        println!("wrote {name}");
    }
}

fn canonical<T: serde::Serialize>(v: &T) -> String {
    let value = serde_json::to_value(v).unwrap();
    let bytes = auki_jcs::canonicalize(&value);
    String::from_utf8(bytes).unwrap()
}

// ─── Shared RegistryRef helpers ──────────────────────────────────────────────

/// Galbot's session/sdk_clock (device-local monotonic, nanoseconds).
/// Hash from auki-registry clock_monotonic.json locked fixture.
fn clock_sdk() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "session/sdk_clock".into(),
        hash: "fb0120a35f4fd1de8a7d46f5e76b7f68".into(),
    }
}

/// Galbot's wall_clock (UTC, nanoseconds).
/// Hash from auki-registry clock_utc.json locked fixture.
fn clock_wall() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "wall_clock".into(),
        hash: "a149244453307763c261dc1759be5373".into(),
    }
}

/// Galbot's head_left_rgb sensor (Camera/rgb, 1920×1200 @ 30 Hz).
/// Hash from auki-registry sensor_camera_rgb.json locked fixture.
fn sensor_head_left_rgb() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "head_left_rgb".into(),
        hash: "8295922307fa2b426453486ba87a59ef".into(),
    }
}

/// Galbot's head_lidar_scan sensor (Rangefinder/3d_lidar).
/// Hash from auki-registry sensor_rangefinder_3d_lidar.json locked fixture.
fn sensor_head_lidar_scan() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "head_lidar_scan".into(),
        hash: "2f26b642d0fc10aabcecf24661f7a83e".into(),
    }
}

/// Galbot's head_left_camera_optical ROS-optical frame.
/// Hash from auki-registry frame_ros_optical.json locked fixture.
fn frame_head_left_camera_optical() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "head_left_camera_optical".into(),
        hash: "3055a8ac27eecd57aa37235b05871c01".into(),
    }
}

/// Park's world frame (ROS-body convention).
/// Hash from auki-manifests pose_log_rigid.json locked fixture (from_frame.hash).
fn frame_park_world() -> RegistryRef {
    RegistryRef {
        peer_id: "park".into(),
        id: "world".into(),
        hash: "2cbe0d5894d3346daa167ecb077cce98".into(),
    }
}

/// Galbot's base_link frame (ROS-body convention).
/// Hash from auki-manifests pose_log_rigid.json locked fixture (to_frame.hash).
fn frame_galbot_base_link() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "base_link".into(),
        hash: "16645bd8584e5ca5fa1030b439da92dc".into(),
    }
}

/// Galbot's left_gripper frame. Uses same hash pattern as base_link for a
/// secondary frame (movable live scenario). Hash matches frame_galbot_base_link
/// since both are ros_body frames — the movable fixture uses left_gripper->object_pose
/// as the resource_id but the manifest still points to world/base_link for
/// consistency with the established galbot pose tree.
fn frame_galbot_left_gripper() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "left_gripper".into(),
        hash: "16645bd8584e5ca5fa1030b439da92dc".into(),
    }
}

/// Galbot's yolo_v8 detector (ObjectDetection, model yolo_v8n).
/// Hash from auki-registry detector_object_detection.json locked fixture.
fn detector_yolo_v8() -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: "yolo_v8".into(),
        hash: "031308c146a2f6f086713388dc01f60e".into(),
    }
}

/// Standard available block for a live 5-second rolling sensor log.
fn available_sensor_live() -> Available {
    Available {
        bytes: 3_000_000_000,
        entries: 900,
        duration_ns: 5_000_000_000,
    }
}

// ─── ResourceEntry constructors ──────────────────────────────────────────────

/// Camera/rgb live rolling sensor log — the canonical galbot head_left_rgb
/// row. Rolling 5-second window at 30 Hz.
fn make_sensor_camera_live_rolling() -> ResourceEntry {
    ResourceEntry {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        resource_id: "head_left_rgb".into(),
        state: "live".into(),
        head: Some(Head::Rolling {
            retention_ns: 5_000_000_000,
        }),
        extent: None,
        available: available_sensor_live(),
        sensor: Some(SensorBlock {
            kind: SensorKind::Camera,
            r#type: "rgb".into(),
            sensor_id: "head_left_rgb".into(),
            sensor_hash: "8295922307fa2b426453486ba87a59ef".into(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: clock_sdk(),
                frame: Some(frame_head_left_camera_optical()),
            },
        },
    }
}

/// Rangefinder/3d_lidar live rolling sensor log — galbot head_lidar_scan
/// at 10 Hz, 5-second rolling window.
fn make_sensor_rangefinder_live_rolling() -> ResourceEntry {
    ResourceEntry {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        resource_id: "head_lidar".into(),
        state: "live".into(),
        head: Some(Head::Rolling {
            retention_ns: 5_000_000_000,
        }),
        extent: None,
        available: Available {
            bytes: 35_000_000,
            entries: 50,
            duration_ns: 5_000_000_000,
        },
        sensor: Some(SensorBlock {
            kind: SensorKind::Rangefinder,
            r#type: "3d_lidar".into(),
            sensor_id: "head_lidar_scan".into(),
            sensor_hash: "2f26b642d0fc10aabcecf24661f7a83e".into(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: clock_sdk(),
                frame: Some(sensor_head_lidar_scan()),
            },
        },
    }
}

/// Sealed sensor log — yesterday's capture of head_left_rgb, fully
/// archived. No head; extent carries the inclusive time bounds.
fn make_sensor_sealed() -> ResourceEntry {
    ResourceEntry {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        resource_id: "yesterday_capture".into(),
        state: "sealed".into(),
        head: None,
        extent: Some(Extent {
            start_at_ns: 1_733_750_400_000_000_000, // 2024-12-09T08:00:00Z
            finish_at_ns: 1_733_836_800_000_000_000, // 2024-12-10T08:00:00Z
        }),
        available: Available {
            bytes: 864_000_000_000,
            entries: 2_592_000,
            duration_ns: 86_400_000_000_000,
        },
        sensor: Some(SensorBlock {
            kind: SensorKind::Camera,
            r#type: "rgb".into(),
            sensor_id: "head_left_rgb".into(),
            sensor_hash: "8295922307fa2b426453486ba87a59ef".into(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: clock_sdk(),
                frame: Some(frame_head_left_camera_optical()),
            },
        },
    }
}

/// Materialized sensor log — Park re-materialized Galbot's head_left_rgb
/// with extended retention. source_peer_id = "galbot", writer_peer_id = "park".
fn make_sensor_materialization() -> ResourceEntry {
    ResourceEntry {
        source_peer_id: "galbot".into(),
        writer_peer_id: "park".into(),
        resource_id: "head_left_rgb".into(),
        state: "live".into(),
        head: Some(Head::Rolling {
            retention_ns: 300_000_000_000, // 5 minutes — Park buffers longer
        }),
        extent: None,
        available: Available {
            bytes: 12_000_000_000,
            entries: 9_000,
            duration_ns: 300_000_000_000,
        },
        sensor: Some(SensorBlock {
            kind: SensorKind::Camera,
            r#type: "rgb".into(),
            sensor_id: "head_left_rgb".into(),
            sensor_hash: "8295922307fa2b426453486ba87a59ef".into(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: clock_sdk(),
                frame: Some(frame_head_left_camera_optical()),
            },
        },
    }
}

/// Movable live pose log with a fixed-start head — left_gripper→object_pose
/// (live SLAM-style; started_at_ns anchors the window start).
fn make_pose_movable_live_fixed() -> ResourceEntry {
    ResourceEntry {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        resource_id: "left_gripper->object_pose".into(),
        state: "live".into(),
        head: Some(Head::Fixed {
            started_at_ns: 1_733_836_800_000_000_000, // 2024-12-10T08:00:00Z
        }),
        extent: None,
        available: Available {
            bytes: 512_000,
            entries: 6_400,
            duration_ns: 213_000_000_000,
        },
        sensor: None,
        pose: Some(PoseBlock {
            writer_mode: PoseWriterMode::Movable,
        }),
        variant_content: VariantContent::PoseLog {
            manifest: PoseManifestPointer {
                from_frame: frame_park_world(),
                to_frame: frame_galbot_left_gripper(),
                clock: clock_sdk(),
                source: PoseSource::Ros2Tf {
                    publishers: vec![
                        "robot_state_publisher".into(),
                        "tf_broadcaster".into(),
                    ],
                },
                expected_rate_hz: 30,
            },
        },
    }
}

/// Rigid sealed pose log — world→base_link static calibrated transform,
/// archived after session end. extent carries a single-sample time range.
fn make_pose_rigid_sealed() -> ResourceEntry {
    ResourceEntry {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        resource_id: "world->base_link".into(),
        state: "sealed".into(),
        head: None,
        extent: Some(Extent {
            start_at_ns: 1_733_836_800_000_000_000,
            finish_at_ns: 1_733_836_800_000_000_000, // single sample
        }),
        available: Available {
            bytes: 80,
            entries: 1,
            duration_ns: 0,
        },
        sensor: None,
        pose: Some(PoseBlock {
            writer_mode: PoseWriterMode::Rigid,
        }),
        variant_content: VariantContent::PoseLog {
            manifest: PoseManifestPointer {
                from_frame: frame_park_world(),
                to_frame: frame_galbot_base_link(),
                clock: clock_sdk(),
                source: PoseSource::Manual,
                expected_rate_hz: 0,
            },
        },
    }
}

/// Time-transform log — session/sdk_clock→wall_clock, live rolling 60s.
fn make_time_transform() -> ResourceEntry {
    ResourceEntry {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        resource_id: "session/sdk_clock->wall_clock".into(),
        state: "live".into(),
        head: Some(Head::Rolling {
            retention_ns: 60_000_000_000, // 1 minute
        }),
        extent: None,
        available: Available {
            bytes: 4_096,
            entries: 60,
            duration_ns: 60_000_000_000,
        },
        sensor: None,
        pose: None,
        variant_content: VariantContent::TimeTransformLog {
            manifest: TimeTransformManifestPointer {
                from_clock: clock_sdk(),
                to_clock: clock_wall(),
                source: TimeTransformSource::LocalClockRead,
            },
        },
    }
}

/// Detection log — yolo_v8@head_left_rgb, live rolling 5s.
fn make_detection() -> ResourceEntry {
    ResourceEntry {
        source_peer_id: "galbot".into(),
        writer_peer_id: "galbot".into(),
        resource_id: "yolo_v8@head_left_rgb".into(),
        state: "live".into(),
        head: Some(Head::Rolling {
            retention_ns: 5_000_000_000,
        }),
        extent: None,
        available: Available {
            bytes: 250_000,
            entries: 150,
            duration_ns: 5_000_000_000,
        },
        sensor: None,
        pose: None,
        variant_content: VariantContent::DetectionLog {
            manifest: DetectionManifestPointer {
                detector: detector_yolo_v8(),
                input_log: LogRef {
                    source_peer_id: "galbot".into(),
                    resource_id: "head_left_rgb".into(),
                },
                input_sensor: sensor_head_left_rgb(),
                clock: clock_sdk(),
            },
        },
    }
}

/// Stream request — subscribe to galbot's head_left_rgb from a specific
/// timestamp (2024-12-10T08:00:00Z expressed as nanoseconds).
fn make_stream_request() -> StreamRequest {
    StreamRequest {
        source_peer_id: "galbot".into(),
        resource_id: "head_left_rgb".into(),
        from: ReadFrom::FromTimestamp(1_733_836_800_000_000_000),
    }
}
