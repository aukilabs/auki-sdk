//! End-to-end smoke for the post-#274 Peer / Session API (no network).
//!
//! Catalog / wire-equivalence assertions live in `auki-domain` now that
//! catalog building moved there (#274 step 6/7).

use std::time::Duration;
use tempfile::tempdir;

use auki_registry::{Camera, ClockBody, ClockMeta, Scope, SensorBody};
use auki_session::{FrameDef, HeadSpec, Peer, SensorLogSpec};

#[test]
fn galbot_session_writes_manifest_then_park_session_independently_constructs_one() {
    let tmp = tempdir().unwrap();

    // ── Galbot side ──────────────────────────────────────────────────────────
    // Peer owns identity + registries; a session is born from it.
    let galbot_peer = Peer::new("galbot", "galbot-ctrl").with_storage_root(tmp.path().join("galbot"));

    let frame = galbot_peer
        .register_frame("head_left_camera_optical", FrameDef::ros_optical())
        .unwrap();

    let sensor = galbot_peer
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
                raster_frame: None,
            }),
        )
        .unwrap();

    let galbot = galbot_peer.start_session().unwrap();

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

    let log = galbot
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

    // 2. The session's live-log view reflects the registered log.
    let logs = galbot.logs();
    assert_eq!(logs.sensor_logs().len(), 1);
    assert_eq!(log.resource_id(), "head_left_rgb");
    assert_eq!(log.log_ref().source_peer_id, "galbot");

    // 3. Park side — an independent peer + session with its own storage root
    // and a different peer_id; doesn't conflict with Galbot's session.
    let park_peer = Peer::new("park", "park-vis").with_storage_root(tmp.path().join("park"));

    let park_frame = park_peer
        .register_frame("base_link", FrameDef::ros_body())
        .unwrap();

    // The RegistryRef carries the peer_id of the peer that registered it.
    assert_eq!(
        park_frame.peer_id, "park",
        "park-registered frame should carry park's peer_id"
    );

    let park = park_peer.start_session().unwrap();
    assert_eq!(park.logs().sensor_logs().len(), 0, "Park has no logs yet");
}

#[tokio::test]
async fn materialize_remote_log_returns_not_implemented() {
    // Full materialization requires a libp2p test harness which is out of scope.
    // Smoke that the surface returns an error as designed.
    let tmp = tempdir().unwrap();
    let peer = Peer::new("park", "vis").with_storage_root(tmp.path().to_path_buf());
    let s = peer.start_session().unwrap();
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
