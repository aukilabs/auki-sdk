//! End-to-end smoke for the post-#216 Session API.

use std::time::Duration;
use tempfile::tempdir;

use auki_network::resources_protocol::SensorKind;
use auki_registry::{Camera, ClockBody, ClockMeta, Scope, SensorBody};
use auki_session::{FrameDef, HeadSpec, SensorLogSpec, Session};

#[test]
fn galbot_session_writes_manifest_then_park_session_independently_constructs_one() {
    let tmp = tempdir().unwrap();

    // ── Galbot side ──────────────────────────────────────────────────────────
    let galbot = Session::new("galbot", "galbot-ctrl").with_storage_root(tmp.path().join("galbot"));

    let frame = galbot
        .register_frame("head_left_camera_optical", FrameDef::ros_optical())
        .unwrap();

    let sensor = galbot
        .register_sensor(
            "head_left_rgb",
            SensorBody::Camera(Camera {
                r#type: "rgb".into(),
                width: 1920,
                height: 1200,
                frame_rate_hz: 30,
                pixel_format: "rgb8".into(),
                color_space: "srgb".into(),
                intrinsics_model: "pinhole".into(),
                distortion_model: "brown_conrady".into(),
                frame: frame.clone(),
            }),
        )
        .unwrap();

    let clock = galbot
        .register_clock(
            "session/sdk_clock",
            ClockBody::MonotonicClock(ClockMeta {
                unit: "ns".into(),
                monotonic: true,
                epoch: None,
                scope: Scope::DeviceLocal,
            }),
        )
        .unwrap();

    let _log = galbot
        .register_sensor_log(SensorLogSpec {
            sensor: sensor.clone(),
            clock,
            frame: Some(frame),
            head: HeadSpec::Rolling {
                retention_ns: 5_000_000_000,
            },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        })
        .unwrap();

    // 1. Manifest persisted to disk with source==writer==galbot
    let manifest_path = tmp
        .path()
        .join("galbot/logs/galbot/head_left_rgb/manifest.json");
    assert!(
        manifest_path.exists(),
        "manifest at {manifest_path:?} should exist"
    );
    let manifest_str = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(
        manifest_str.contains(r#""source_peer_id":"galbot""#),
        "manifest missing source_peer_id: {manifest_str}"
    );
    assert!(
        manifest_str.contains(r#""writer_peer_id":"galbot""#),
        "manifest missing writer_peer_id: {manifest_str}"
    );
    assert!(
        manifest_str.contains(r#""retention_ns":5000000000"#),
        "manifest missing retention_ns: {manifest_str}"
    );

    // 2. Catalog produces one row for the sensor log
    let catalog = galbot.catalog();
    assert_eq!(catalog.len(), 1);
    let row = &catalog[0];
    assert_eq!(row.source_peer_id, "galbot");
    assert_eq!(row.writer_peer_id, "galbot");
    assert_eq!(row.resource_id, "head_left_rgb");
    assert_eq!(row.state, "live");
    let sensor_block = row.sensor.as_ref().unwrap();
    assert_eq!(sensor_block.kind, SensorKind::Camera);
    assert_eq!(sensor_block.r#type, "rgb");

    // 3. Park side — independently constructs a session with its own storage root
    // and a different peer_id; doesn't conflict with Galbot's session.
    let park = Session::new("park", "park-vis").with_storage_root(tmp.path().join("park"));

    let park_frame = park
        .register_frame("base_link", FrameDef::ros_body())
        .unwrap();

    // The RegistryRef carries the peer_id of the session that registered it.
    assert_eq!(
        park_frame.peer_id, "park",
        "park-registered frame should carry park's peer_id"
    );

    let park_catalog = park.catalog();
    assert_eq!(park_catalog.len(), 0, "Park hasn't registered any logs yet");
}

#[tokio::test]
async fn materialize_remote_log_returns_not_implemented() {
    // Full materialization requires a libp2p test harness which is out of scope.
    // Smoke that the surface returns an error as designed.
    let tmp = tempdir().unwrap();
    let s = Session::new("park", "vis").with_storage_root(tmp.path().to_path_buf());
    let result = s
        .materialize_remote_log(
            auki_registry::LogRef {
                source_peer_id: "galbot".to_string(),
                resource_id: "head_left_rgb".to_string(),
            },
            Duration::from_secs(300),
            Duration::from_secs(10),
        )
        .await;
    assert!(result.is_err(), "materialize_remote_log should return Err");
}
