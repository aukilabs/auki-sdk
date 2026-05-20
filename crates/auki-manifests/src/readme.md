# `auki-manifests/src/`

Implementation status of `auki-manifests`. Spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs).

## Public surface

```rust
pub fn build_sensor_log_manifest(
    app_id: &str, session_id: &str,
    sensor_id: &str, sensor_hash: &str,
    clock_id: &str, clock_hash: &str,
    frame_id: Option<&str>, frame_hash: Option<&str>,
    segment_duration: Duration, retention: Duration,
) -> serde_json::Value;

pub fn build_pose_log_manifest(
    app_id: &str, session_id: &str,
    from_frame_id: &str, from_frame_hash: &str,
    to_frame_id: &str, to_frame_hash: &str,
    clock_id: &str, clock_hash: &str,
    source: &PoseSource,
    writer_mode: PoseWriterMode,
    expected_rate_hz: u32,
    segment_duration: Duration, retention: Duration,
) -> serde_json::Value;

pub fn build_time_transform_log_manifest(
    app_id: &str, session_id: &str,
    from_clock_id: &str, from_clock_hash: &str,
    to_clock_id: &str, to_clock_hash: &str,
    source: &TimeTransformSource,
    segment_duration: Duration, retention: Duration,
) -> serde_json::Value;

pub fn build_detection_log_manifest(
    app_id: &str, session_id: &str,
    detector_id: &str, detector_hash: &str,
    input_log_id: &str,
    input_sensor_id: &str, input_sensor_hash: &str,
    clock_id: &str, clock_hash: &str,
    segment_duration: Duration, retention: Duration,
) -> serde_json::Value;

pub enum PoseSource {
    Ros2Tf { publishers: Vec<String> },
}

impl PoseSource {
    pub fn canonical_bytes(&self) -> Vec<u8>;
    pub fn hash(&self) -> String;
}

pub enum PoseWriterMode {
    Rigid,    // serialized as "rigid"
    Movable,  // serialized as "movable"
}

pub enum TimeTransformSource {
    LocalClockRead,    // serialized as {"kind":"local_clock_read"}
}

impl TimeTransformSource {
    pub fn canonical_bytes(&self) -> Vec<u8>;
    pub fn hash(&self) -> String;
}
```

## Tests (12 total)

| Test | Asserts |
|------|---------|
| `build_sensor_log_manifest_contains_all_required_fields` | All eight required fields present with correct values + types. |
| `sensor_log_manifest_opens_a_log_round_trip` | End-to-end: builder produces a manifest `auki-logs::Log<T>::open` accepts; manifest survives a write/read cycle. |
| `ros2_tf_source_serializes_to_canonical_bytes` | M1 example → locked JCS canonical bytes (`{"kind":"ros2_tf",...}`). Catches drift in tagged-enum serde shape OR canonicalization. |
| `ros2_tf_source_hash_is_locked` | M1 example → `f3d296341347589c72297a0cc7c81cd8`. Cross-cutting guard against `auki-jcs` / `auki-hash` / this crate's serde shape drifting. |
| `build_pose_log_manifest_contains_all_required_fields` | All 13 required fields present (frame-pair × 2, clock-pair × 2, app/session, source, writer_mode, expected_rate_hz, segment/retention). |
| `build_pose_log_manifest_serializes_writer_mode_as_snake_case` | `PoseWriterMode::Rigid` → JSON `"rigid"` (and Movable → `"movable"`). Pins the snake_case rename. |
| `build_time_transform_log_manifest_contains_required_fields` | All required fields present (six clock-binding + `app_id` / `session_id` + `source.kind == "local_clock_read"` + the two from auki-logs). |
| `local_clock_read_source_serializes_to_canonical_bytes` | `TimeTransformSource::LocalClockRead` → locked JCS canonical bytes `{"kind":"local_clock_read"}`. Catches drift in tagged-enum serde shape OR canonicalization. Mirrors `ros2_tf_source_serializes_to_canonical_bytes`. |
| `local_clock_read_source_hash_is_locked` | XXH3-128 of those canonical bytes — `8dcea0b9b0b2219d651e0856f112cd65`. |
| `build_detection_log_manifest_contains_all_required_fields` | All 11 required fields present: `app_id`, `session_id`, `detector_id`, `detector_hash`, `input_log_id`, `input_sensor_id`, `input_sensor_hash`, `clock_id`, `clock_hash`, `segment_duration_ns`, `retention_ns`. |
| `build_detection_log_manifest_omits_intent_field` | Pins absence of `intent` — match-the-existing-builders for v1; uniform rollout is a separate PR. Provides a failing test for the future PR to update. |
| `detection_log_manifest_opens_a_log_round_trip` | End-to-end: builder produces a manifest `auki-logs::Log<T>::open` accepts; the manifest survives a write/read cycle and surfaces both `detector_id` + `detector_hash` and `input_sensor_id` + `input_sensor_hash` (self-containedness check). |

`cargo test -p auki-manifests` runs 12 tests.

## Dependencies

- [`auki-jcs`](../../auki-jcs) — canonical JSON for `PoseSource::canonical_bytes`.
- [`auki-hash`](../../auki-hash) — XXH3-128 for `PoseSource::hash`.
- `serde` + `serde_json` — manifests are `serde_json::Value` instances; `PoseSource` is serde-derived.

Dev-deps: `auki-logs` (round-trip test opens an actual `Log<T>`), `ciborium` (placeholder body type encoding), `tempfile`.

## Consumers in this workspace

- [`auki-registry`](../../auki-registry) — uses `build_sensor_log_manifest` and `build_pose_log_manifest` in its end-to-end log integration tests (the registry crate doesn't open logs in production code, but its tests do).
- [`auki-time`](../../auki-time) — uses `build_time_transform_log_manifest` in its `Sampler` integration test (the production sampler accepts a pre-built manifest).
- *Downstream apps* (boosterapp, Park, future Sentinel) — call these builders directly when opening logs.
- [`detectors`](https://github.com/aukilabs/detectors) (downstream) — the integrator (Park / Boosterapp) calls `build_detection_log_manifest` + `auki_layout::detection_log_path` to pre-create the output `Log<DetectionLogEntry>`, then hands the write-handle to the detector loop. The detector itself doesn't construct the manifest — caller-decides per the [keystone's intent-decoupling entry](../../../parking_lot.md).
