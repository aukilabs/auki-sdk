//! Regenerates the canonical-JSON locked fixtures under
//! `crates/auki-registry/tests/locked/`.
//!
//! Run after any intentional schema change to commit updated fixtures:
//!
//! ```sh
//! cargo run -p auki-registry --bin regen_locked_fixtures
//! ```

use auki_registry::{
    Audio, Camera, ClockBody, ClockMeta, ClockRegistryEntry, DetectorBody, DetectorRegistryEntry,
    FrameRegistryEntry, JointEncoders, ObjectDetection, PointField, PointFieldDataType,
    Rangefinder, RegistryRef, Rf, Scope, SensorBody, SensorRegistryEntry,
};
use std::fs;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from("crates/auki-registry/tests/locked");
    fs::create_dir_all(&out).unwrap();

    let cases: Vec<(&str, String)> = vec![
        (
            "sensor_camera_rgb.json",
            canonical_sensor(&make_sensor_camera_rgb()),
        ),
        (
            "sensor_camera_depth.json",
            canonical_sensor(&make_sensor_camera_depth()),
        ),
        (
            "sensor_rangefinder_point_cloud.json",
            canonical_sensor(&make_sensor_rangefinder_point_cloud()),
        ),
        (
            "sensor_rangefinder_3d_lidar.json",
            canonical_sensor(&make_sensor_rangefinder_3d_lidar()),
        ),
        (
            "sensor_rf_wifi.json",
            canonical_sensor(&make_sensor_rf_wifi()),
        ),
        (
            "sensor_audio_pcm.json",
            canonical_sensor(&make_sensor_audio_pcm()),
        ),
        (
            "sensor_joint_encoders_absolute.json",
            canonical_sensor(&make_sensor_joint_encoders_absolute()),
        ),
        (
            "clock_monotonic.json",
            canonical_clock(&make_clock_monotonic()),
        ),
        ("clock_utc.json", canonical_clock(&make_clock_utc())),
        (
            "frame_ros_body.json",
            canonical_frame(&make_frame_ros_body()),
        ),
        (
            "frame_ros_optical.json",
            canonical_frame(&make_frame_ros_optical()),
        ),
        ("frame_opengl.json", canonical_frame(&make_frame_opengl())),
        ("frame_unity.json", canonical_frame(&make_frame_unity())),
        (
            "detector_object_detection.json",
            canonical_detector(&make_detector_object_detection()),
        ),
    ];

    for (name, json) in cases {
        let path = out.join(name);
        fs::write(&path, json + "\n").unwrap();
        println!("wrote {name}");
    }
}

// ─── Canonicalization helpers ─────────────────────────────────────────────────

fn canonical_sensor(v: &SensorRegistryEntry) -> String {
    let val = serde_json::to_value(v).expect("serialize");
    String::from_utf8(auki_jcs::canonicalize(&val)).unwrap()
}

fn canonical_clock(v: &ClockRegistryEntry) -> String {
    let val = serde_json::to_value(v).expect("serialize");
    String::from_utf8(auki_jcs::canonicalize(&val)).unwrap()
}

fn canonical_frame(v: &FrameRegistryEntry) -> String {
    let val = serde_json::to_value(v).expect("serialize");
    String::from_utf8(auki_jcs::canonicalize(&val)).unwrap()
}

fn canonical_detector(v: &DetectorRegistryEntry) -> String {
    let val = serde_json::to_value(v).expect("serialize");
    String::from_utf8(auki_jcs::canonicalize(&val)).unwrap()
}

// ─── Frame refs ───────────────────────────────────────────────────────────────
//
// Sensor bodies reference frames with the same peer_id ("galbot"). We compute
// the frame hash by constructing the frame entry and calling its .hash() method
// so the fixture values are always in sync with the frame shapes.

fn galbot_frame_ref(frame_id: &str, entry: &FrameRegistryEntry) -> RegistryRef {
    RegistryRef {
        peer_id: "galbot".into(),
        id: frame_id.into(),
        hash: entry.hash(),
    }
}

// ─── Sensor fixtures ──────────────────────────────────────────────────────────

fn make_sensor_camera_rgb() -> SensorRegistryEntry {
    let optical_frame = FrameRegistryEntry::ros_optical("galbot", "head_left_camera_optical");
    SensorRegistryEntry {
        peer_id: "galbot".into(),
        sensor_id: "head_left_rgb".into(),
        body: SensorBody::Camera(Camera {
            r#type: "rgb".into(),
            width: 1920,
            height: 1200,
            frame_rate_hz: 30,
            image_encoding: "raw".into(),
            pixel_format: "rgb8".into(),
            row_stride_bytes: 1920 * 3,
            color_space: "srgb".into(),
            intrinsics_model: "pinhole".into(),
            distortion_model: "brown_conrady".into(),
            calibration: None,
            frame: galbot_frame_ref("head_left_camera_optical", &optical_frame),
        }),
    }
}

fn make_sensor_camera_depth() -> SensorRegistryEntry {
    let optical_frame = FrameRegistryEntry::ros_optical("galbot", "head_depth_optical");
    SensorRegistryEntry {
        peer_id: "galbot".into(),
        sensor_id: "head_depth".into(),
        body: SensorBody::Camera(Camera {
            r#type: "depth".into(),
            width: 640,
            height: 480,
            frame_rate_hz: 30,
            image_encoding: "raw".into(),
            pixel_format: "16uc1".into(),
            row_stride_bytes: 640 * 2,
            color_space: "depth".into(),
            intrinsics_model: "pinhole".into(),
            distortion_model: "brown_conrady".into(),
            calibration: None,
            frame: galbot_frame_ref("head_depth_optical", &optical_frame),
        }),
    }
}

fn make_sensor_rangefinder_point_cloud() -> SensorRegistryEntry {
    let lidar_frame = FrameRegistryEntry::ros_body("galbot", "head_lidar");
    SensorRegistryEntry {
        peer_id: "galbot".into(),
        sensor_id: "head_lidar".into(),
        body: SensorBody::Rangefinder(Rangefinder {
            r#type: "point_cloud".into(),
            fields: vec![
                PointField {
                    name: "x".into(),
                    offset: 0,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
                PointField {
                    name: "y".into(),
                    offset: 4,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
                PointField {
                    name: "z".into(),
                    offset: 8,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
                PointField {
                    name: "intensity".into(),
                    offset: 12,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
            ],
            point_step: 16,
            is_bigendian: false,
            frame_rate_hz: 10,
            frame: galbot_frame_ref("head_lidar", &lidar_frame),
        }),
    }
}

fn make_sensor_rangefinder_3d_lidar() -> SensorRegistryEntry {
    let lidar_frame = FrameRegistryEntry::ros_body("galbot", "head_lidar_scan");
    SensorRegistryEntry {
        peer_id: "galbot".into(),
        sensor_id: "head_lidar_scan".into(),
        body: SensorBody::Rangefinder(Rangefinder {
            r#type: "3d_lidar".into(),
            // Required fields per the struct shape; 3d_lidar types encode the
            // point layout as part of the body the same as point_cloud.
            fields: vec![
                PointField {
                    name: "x".into(),
                    offset: 0,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
                PointField {
                    name: "y".into(),
                    offset: 4,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
                PointField {
                    name: "z".into(),
                    offset: 8,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
                PointField {
                    name: "ring".into(),
                    offset: 12,
                    datatype: PointFieldDataType::Uint16,
                    count: 1,
                },
            ],
            point_step: 16,
            is_bigendian: false,
            frame_rate_hz: 10,
            frame: galbot_frame_ref("head_lidar_scan", &lidar_frame),
        }),
    }
}

fn make_sensor_rf_wifi() -> SensorRegistryEntry {
    let base_frame = FrameRegistryEntry::ros_body("galbot", "base_link");
    SensorRegistryEntry {
        peer_id: "galbot".into(),
        sensor_id: "wifi_scanner".into(),
        body: SensorBody::Rf(Rf {
            r#type: "wifi".into(),
            frame: galbot_frame_ref("base_link", &base_frame),
        }),
    }
}

fn make_sensor_audio_pcm() -> SensorRegistryEntry {
    let base_frame = FrameRegistryEntry::ros_body("galbot", "base_link");
    SensorRegistryEntry {
        peer_id: "galbot".into(),
        sensor_id: "mic_array".into(),
        body: SensorBody::Audio(Audio {
            r#type: "pcm".into(),
            sample_rate_hz: 48_000,
            channels: 4,
            sample_format: "pcm_s16le".into(),
            channel_layout: "n_channel".into(),
            frame: galbot_frame_ref("base_link", &base_frame),
        }),
    }
}

fn make_sensor_joint_encoders_absolute() -> SensorRegistryEntry {
    let base_frame = FrameRegistryEntry::ros_body("galbot", "base_link");
    SensorRegistryEntry {
        peer_id: "galbot".into(),
        sensor_id: "left_arm_joints".into(),
        body: SensorBody::JointEncoders(JointEncoders {
            r#type: "absolute".into(),
            joint_count: 7,
            frame_rate_hz: 100,
            frame: galbot_frame_ref("base_link", &base_frame),
        }),
    }
}

// ─── Clock fixtures ───────────────────────────────────────────────────────────

fn make_clock_monotonic() -> ClockRegistryEntry {
    ClockRegistryEntry {
        peer_id: "galbot".into(),
        session_id: "sess-7f3a".into(),
        clock_id: "galbot/sess-7f3a/monotonic".into(),
        body: ClockBody::MonotonicClock(ClockMeta {
            unit: "ns".into(),
            monotonic: true,
            epoch: None,
            scope: Scope::DeviceLocal,
        }),
    }
}

fn make_clock_utc() -> ClockRegistryEntry {
    ClockRegistryEntry {
        peer_id: "galbot".into(),
        session_id: "sess-7f3a".into(),
        clock_id: "galbot/sess-7f3a/utc".into(),
        body: ClockBody::UtcClock(ClockMeta {
            unit: "ns".into(),
            monotonic: false,
            epoch: Some("1970-01-01T00:00:00Z".into()),
            scope: Scope::DeviceLocal,
        }),
    }
}

// ─── Frame fixtures ───────────────────────────────────────────────────────────

fn make_frame_ros_body() -> FrameRegistryEntry {
    FrameRegistryEntry::ros_body("galbot", "base_link")
}

fn make_frame_ros_optical() -> FrameRegistryEntry {
    FrameRegistryEntry::ros_optical("galbot", "head_left_camera_optical")
}

fn make_frame_opengl() -> FrameRegistryEntry {
    FrameRegistryEntry::opengl("galbot", "world_gl")
}

fn make_frame_unity() -> FrameRegistryEntry {
    FrameRegistryEntry::unity("galbot", "world_unity")
}

// ─── Detector fixtures ────────────────────────────────────────────────────────

fn make_detector_object_detection() -> DetectorRegistryEntry {
    DetectorRegistryEntry {
        peer_id: "galbot".into(),
        detector_id: "yolo_v8".into(),
        body: DetectorBody::ObjectDetection(ObjectDetection {
            model: "yolo_v8n".into(),
        }),
        input_types: vec![],
        output_types: vec!["object_detection".into()],
    }
}
