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
    segment_duration: Duration, retention: Duration,
) -> serde_json::Value;

pub fn build_pose_log_manifest(
    app_id: &str, session_id: &str,
    clock_id: &str, clock_hash: &str,
    source: &PoseSource,
    segment_duration: Duration, retention: Duration,
) -> serde_json::Value;

pub fn build_time_transform_log_manifest(
    app_id: &str, session_id: &str,
    from_clock_id: &str, from_clock_hash: &str,
    to_clock_id: &str, to_clock_hash: &str,
    segment_duration: Duration, retention: Duration,
) -> serde_json::Value;

pub enum PoseSource {
    Ros2Tf { publishers: Vec<String> },
}

impl PoseSource {
    pub fn canonical_bytes(&self) -> Vec<u8>;
    pub fn hash(&self) -> String;
}
```

## Tests (6 total)

| Test | Asserts |
|------|---------|
| `build_sensor_log_manifest_contains_all_required_fields` | All eight required fields present with correct values + types. |
| `sensor_log_manifest_opens_a_log_round_trip` | End-to-end: builder produces a manifest `auki-logs::Log<T>::open` accepts; manifest survives a write/read cycle. |
| `ros2_tf_source_serializes_to_canonical_bytes` | M1 example → locked JCS canonical bytes (`{"kind":"ros2_tf",...}`). Catches drift in tagged-enum serde shape OR canonicalization. |
| `ros2_tf_source_hash_is_locked` | M1 example → `f3d296341347589c72297a0cc7c81cd8`. Cross-cutting guard against `auki-jcs` / `auki-hash` / this crate's serde shape drifting. |
| `build_pose_log_manifest_contains_all_required_fields` | All required fields present, including `source.kind` / `source.publishers[0]`. |
| `build_time_transform_log_manifest_contains_required_fields` | All required fields present (six clock-binding + `app_id` / `session_id` + the two from auki-logs). |

`cargo test -p auki-manifests` runs 6 tests.

## Dependencies

- [`auki-jcs`](../../auki-jcs) — canonical JSON for `PoseSource::canonical_bytes`.
- [`auki-hash`](../../auki-hash) — XXH3-128 for `PoseSource::hash`.
- `serde` + `serde_json` — manifests are `serde_json::Value` instances; `PoseSource` is serde-derived.

Dev-deps: `auki-logs` (round-trip test opens an actual `Log<T>`), `ciborium` (placeholder body type encoding), `tempfile`.

## Consumers in this workspace

- [`auki-registry`](../../auki-registry) — uses `build_sensor_log_manifest` and `build_pose_log_manifest` in its end-to-end log integration tests (the registry crate doesn't open logs in production code, but its tests do).
- [`auki-time-transforms`](../../auki-time-transforms) — uses `build_time_transform_log_manifest` in its `Sampler` integration test (the production sampler accepts a pre-built manifest).
- *Downstream apps* (boosterapp, Park, future Sentinel) — call these builders directly when opening logs.
