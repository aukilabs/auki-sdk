//! Round-trip tests for the locked canonical-JSON fixtures in `tests/locked/`.
//!
//! Each test deserializes a fixture into the Rust type, re-serializes to
//! canonical JSON via `auki-jcs`, and asserts byte-equality with the file.
//! A failure means either the fixture is stale (run `regen_locked_fixtures`)
//! or the struct shape changed without a coordinated fixture update.
//!
//! Run with:
//! ```sh
//! cargo test -p auki-network --features swarm --test locked_json
//! ```

use auki_network::resources_protocol::ResourceEntry;
use auki_network::stream_protocol::StreamRequest;
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
    let serialized = serde_json::to_value(&value).unwrap();
    let actual = auki_jcs::canonicalize(&serialized);
    let expected: &[u8] = bytes.trim_ascii_end();
    assert_eq!(
        std::str::from_utf8(&actual).unwrap(),
        std::str::from_utf8(expected).unwrap(),
        "fixture {fixture} drifted"
    );
}

#[test]
fn sensor_log_camera_live_rolling() {
    assert_round_trip::<ResourceEntry>("catalog_row_sensor_log_camera_live_rolling.json");
}

#[test]
fn sensor_log_rangefinder_live_rolling() {
    assert_round_trip::<ResourceEntry>("catalog_row_sensor_log_rangefinder_live_rolling.json");
}

#[test]
fn sensor_log_sealed() {
    assert_round_trip::<ResourceEntry>("catalog_row_sensor_log_sealed.json");
}

#[test]
fn sensor_log_materialization() {
    assert_round_trip::<ResourceEntry>("catalog_row_sensor_log_materialization.json");
}

#[test]
fn pose_log_movable_live_fixed() {
    assert_round_trip::<ResourceEntry>("catalog_row_pose_log_movable_live_fixed.json");
}

#[test]
fn pose_log_rigid_sealed() {
    assert_round_trip::<ResourceEntry>("catalog_row_pose_log_rigid_sealed.json");
}

#[test]
fn time_transform_log() {
    assert_round_trip::<ResourceEntry>("catalog_row_time_transform_log.json");
}

#[test]
fn detection_log() {
    assert_round_trip::<ResourceEntry>("catalog_row_detection_log.json");
}

#[test]
fn stream_request_locked() {
    assert_round_trip::<StreamRequest>("stream_request.json");
}
