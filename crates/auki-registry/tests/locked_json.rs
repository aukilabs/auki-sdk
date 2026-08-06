//! Round-trip harness for locked canonical-JSON fixtures.
//!
//! Each test:
//! 1. Reads `tests/locked/<fixture>.json` from disk.
//! 2. Deserializes into the appropriate registry entry type.
//! 3. Re-canonicalizes via [`auki_jcs`].
//! 4. Asserts byte-equality against the on-disk fixture (trailing newline stripped).
//!
//! This catches unintended schema drift in either the struct shapes or the
//! canonicalization logic. Run after any schema change to surface breakage
//! before it reaches cross-language consumers.
//!
//! To regenerate fixtures after an *intentional* schema change:
//! ```sh
//! cargo run -p auki-registry --bin regen_locked_fixtures
//! ```

use auki_registry::{
    ClockRegistryEntry, DetectorRegistryEntry, FrameRegistryEntry, SensorRegistryEntry,
};
use std::fs;
use std::path::Path;

fn assert_round_trip<T>(fixture: &str)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let path = Path::new("tests/locked").join(fixture);
    let bytes = fs::read(&path).unwrap_or_else(|_| panic!("missing fixture {fixture}"));
    let value: T = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("failed to deserialize {fixture}: {e}"));
    let json_value = serde_json::to_value(&value)
        .unwrap_or_else(|e| panic!("failed to re-serialize {fixture}: {e}"));
    let actual = auki_jcs::canonicalize(&json_value);
    let expected = bytes.trim_ascii_end();
    assert_eq!(
        std::str::from_utf8(&actual).unwrap(),
        std::str::from_utf8(expected).unwrap(),
        "fixture {fixture} drifted"
    );
}

// ─── Sensor fixtures ──────────────────────────────────────────────────────────

#[test]
fn sensor_camera_rgb_locked() {
    assert_round_trip::<SensorRegistryEntry>("sensor_camera_rgb.json");
}

#[test]
fn sensor_camera_depth_locked() {
    assert_round_trip::<SensorRegistryEntry>("sensor_camera_depth.json");
}

#[test]
fn sensor_rangefinder_point_cloud_locked() {
    assert_round_trip::<SensorRegistryEntry>("sensor_rangefinder_point_cloud.json");
}

#[test]
fn sensor_rangefinder_3d_lidar_locked() {
    assert_round_trip::<SensorRegistryEntry>("sensor_rangefinder_3d_lidar.json");
}

#[test]
fn sensor_rf_wifi_locked() {
    assert_round_trip::<SensorRegistryEntry>("sensor_rf_wifi.json");
}

#[test]
fn sensor_audio_pcm_locked() {
    assert_round_trip::<SensorRegistryEntry>("sensor_audio_pcm.json");
}

#[test]
fn sensor_joint_encoders_absolute_locked() {
    assert_round_trip::<SensorRegistryEntry>("sensor_joint_encoders_absolute.json");
}

#[test]
fn sensor_scalar_battery_charge_locked() {
    assert_round_trip::<SensorRegistryEntry>("sensor_scalar_battery_charge.json");
}

// ─── Clock fixtures ───────────────────────────────────────────────────────────

#[test]
fn clock_monotonic_locked() {
    assert_round_trip::<ClockRegistryEntry>("clock_monotonic.json");
}

#[test]
fn clock_utc_locked() {
    assert_round_trip::<ClockRegistryEntry>("clock_utc.json");
}

// ─── Frame fixtures ───────────────────────────────────────────────────────────

#[test]
fn frame_ros_body_locked() {
    assert_round_trip::<FrameRegistryEntry>("frame_ros_body.json");
}

#[test]
fn frame_ros_optical_locked() {
    assert_round_trip::<FrameRegistryEntry>("frame_ros_optical.json");
}

#[test]
fn frame_opengl_locked() {
    assert_round_trip::<FrameRegistryEntry>("frame_opengl.json");
}

#[test]
fn frame_unity_locked() {
    assert_round_trip::<FrameRegistryEntry>("frame_unity.json");
}

// ─── Detector fixtures ────────────────────────────────────────────────────────

#[test]
fn detector_object_detection_locked() {
    assert_round_trip::<DetectorRegistryEntry>("detector_object_detection.json");
}
