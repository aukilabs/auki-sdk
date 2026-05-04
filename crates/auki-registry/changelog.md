# Changelog — auki-registry

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 4, 11:11 HKT, 2026

Pose Log capture support added — first concrete step toward `convert_pose`. New types: `PoseSource` (tagged enum, v1 ships `Ros2Tf { publishers: Vec<String> }` and the extension point for SLAM/odometry/manual fixtures), `PoseLogEntry { transforms: Vec<TransformSample> }`, `TransformSample { parent_frame, child_frame, translation: [f64;3], rotation_quat: [f64;4] }`. New `build_pose_log_manifest(app_id, session_id, clock_id, clock_hash, source: &PoseSource, segment_duration, retention) -> serde_json::Value`. Pose Log directories sit at `<session>/poselogs/<recording_uuid>/` — peer to Sensor Log, same parallel-recording machinery (multiple recordings per session, ring buffer + intent captures distinguished only by `retention_ns`). **No Pose Source Registry** — payload is fully self-describing (frame names sit in each `TransformSample`), so source identity rides inline in the manifest under `"source"` as provenance, not a decoder; cf. Sensor Log which earns a registry because its byte payload is uninterpretable without one. `f64` matches ROS `geometry_msgs`; rotation order is xyzw (Hamilton, matches ROS); `/tf_static` merges with `/tf` on capture. Locked canonical bytes + locked hash (`f3d296341347589c72297a0cc7c81cd8`) for the M1 example ROS 2 TF source. New `ciborium` dev-dependency for the CBOR round-trip tests. 6 new tests; auki-registry now at 29 tests.

### broodsugar's claude · May 4, 10:38 HKT, 2026

New `build_sensor_log_manifest(app_id, session_id, sensor_id, sensor_hash, clock_id, clock_hash, segment_duration, retention) -> serde_json::Value` constructs a Sensor Log family manifest with all eight required fields. One function serves Sensor Log, Point Cloud Log, and Audio Log alike — they share the manifest shape; the `(sensor_id, sensor_hash)` pair resolves to the body variant that tells a reader the payload type. Mirrors the existing `auki_time_transforms::build_manifest` pattern; centralizes the spec in code instead of leaving integrators to hand-roll JSON. New `auki-logs` dev-dependency for the round-trip integration test. 2 new tests; auki-registry now at 23 tests. Closes the implementation half of the `app_id` (May 4, 08:52) and `session_id` (May 4, 10:22) spec PRs.

### broodsugar's claude · May 4, 10:22 HKT, 2026

Sensor Log family manifest gains a required `session_id: string` field — UUIDv4 minted by the integrator at app boot, same value as the parent session directory name and `/api/state`'s `session_uuid`. Mirrors the `app_id` shape from earlier today; together they make every manifest self-identifying about which app run produced it. Spec-only; implementation/tests pending. Companion to the lifecycle formalization in `auki-session/README.md`.

### broodsugar's claude · May 4, 08:52 HKT, 2026

Sensor Log family manifest gains a required `app_id: string` field, carrying the same identifier as the daemon's `/api/info` `app` value (e.g. `boosterapp`, `sentinel`). Applies to Sensor Log, Point Cloud Log, and Audio Log — they share the manifest shape. Mandatory addition; breaking against existing on-disk logs (acceptable under v0.x). Implementation/tests still pending.

### broodsugar's claude · May 2, 13:50 HKT, 2026

Added audio sensor support: new `SensorBody::Microphone` variant with fields `sample_rate_hz`, `channels`, `sample_format`, `channel_layout`; new `AudioLogEntry { data: bytes }` payload type with `serde_bytes` so CBOR encodes the sample buffer as a byte string (major type 2). Modelled multi-mic arrays as one sensor with `channels = N` rather than N independent sensors — right for physically-synchronized arrays sharing a clock and origin. v1 spec covers PCM only (`pcm_s16le`/`s24le`/`s32le`/`f32le`/`f64le`); compressed formats (`flac`, `opus`) extend `sample_format` when they earn it without changing the struct shape. Locked canonical bytes + locked hash (`6e0a195364866f18834d2db8e2a0699f`) for an M1 example mic-array entry. 3 new tests; auki-registry now at 21 tests.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
