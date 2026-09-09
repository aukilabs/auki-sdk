//! Round-trip harness for locked canonical-JSON fixtures.
//!
//! Each test:
//! 1. Reads `tests/locked/<fixture>.json` from disk.
//! 2. Deserializes into the appropriate manifest type.
//! 3. Re-canonicalizes via [`auki_jcs`].
//! 4. Asserts byte-equality against the on-disk fixture (trailing newline stripped).
//!
//! This catches unintended schema drift in either the struct shapes or the
//! canonicalization logic. Run after any schema change to surface breakage
//! before it reaches cross-language consumers.
//!
//! To regenerate fixtures after an *intentional* schema change:
//! ```sh
//! cargo run -p auki-manifests --bin regen_manifest_fixtures
//! ```

use auki_manifests::{
    DetectionLogManifest, PoseLogManifest, SensorLogManifest, TimeTransformLogManifest,
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

// ─── Sensor Log fixtures ──────────────────────────────────────────────────────

#[test]
fn sensor_log_origin_locked() {
    assert_round_trip::<SensorLogManifest>("sensor_log_origin.json");
}

#[test]
fn sensor_log_materialized_locked() {
    assert_round_trip::<SensorLogManifest>("sensor_log_materialized.json");
}

// ─── Pose Log fixtures ────────────────────────────────────────────────────────

#[test]
fn pose_log_rigid_locked() {
    assert_round_trip::<PoseLogManifest>("pose_log_rigid.json");
}

#[test]
fn pose_log_movable_locked() {
    assert_round_trip::<PoseLogManifest>("pose_log_movable.json");
}

// ─── TimeTransform Log fixtures ───────────────────────────────────────────────

#[test]
fn time_transform_log_locked() {
    assert_round_trip::<TimeTransformLogManifest>("time_transform_log.json");
}

// ─── Detection Log fixtures ───────────────────────────────────────────────────

#[test]
fn detection_log_locked() {
    assert_round_trip::<DetectionLogManifest>("detection_log.json");
}
