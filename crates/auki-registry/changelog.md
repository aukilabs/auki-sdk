# Changelog — auki-registry

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 8, 11:20 HKT, 2026

**`AudioLogEntry` departed at Step 4 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md).** The struct is gone from [`src/lib.rs`](src/lib.rs); replaced by [`auki_datatypes::audio::AudioLogEntry`](../auki-datatypes/src/lib.rs) under the `auki.audio` `.proto` package. Per-step decision: opaque-bytes-only — same stance as Step 3 for point clouds; declines the pre-Step-3 lean toward adding `sample_count`. Readers derive sample count and chunk duration from the bytes plus the `Microphone` registry body.

**`serde_bytes` dep dropped** — `AudioLogEntry` was its last user (`PointCloudLogEntry` already departed at Step 3).

**Docs**: README's "Audio Log payload — schema v1" section replaced with a one-paragraph pointer at [`auki-datatypes`](../auki-datatypes); sample-layout semantics carried over verbatim. `src/readme.md`'s "Log payload types" code block dropped the `AudioLogEntry` struct.

The remaining `PoseLogEntry` + `TransformSample` move out at Step 5.

### broodsugar's claude · May 8, 10:51 HKT, 2026

**`PointCloudLogEntry` departed at Step 3 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md).** The struct is gone from [`src/lib.rs`](src/lib.rs); replaced by [`auki_datatypes::point_cloud::PointCloudLogEntry`](../auki-datatypes/src/lib.rs) under the `auki.point_cloud` `.proto` package. Per-step decision: opaque-bytes-only — the ROS-shaped fields `width` / `height` / `is_dense` are gone from the per-frame entry; readers resolve them via `(sensor_id, sensor_hash) → SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id }`.

**Docs**: README's "Point Cloud Log payload — schema v1" section replaced with a one-paragraph pointer at [`auki-datatypes`](../auki-datatypes); RGB(A) normalization sub-section retained (still happens in [`auki-ros-adapter`](../auki-ros-adapter)). `src/readme.md`'s "Log payload types" code block dropped the `PointCloudLogEntry` struct + the line about its `serde_bytes` tagging. `serde_bytes` dep stays (still used by `AudioLogEntry`).

The remaining `PoseLogEntry` + `TransformSample` move out at Step 5; `AudioLogEntry` at Step 4.

### broodsugar's dobby · May 8, 09:34 HKT, 2026

Path-helper dep renamed: `auki-session = { path = "../auki-session" }` → `auki-layout = { path = "../auki-layout" }` in `Cargo.toml`; six call sites in `src/lib.rs` updated from `auki_session::` to `auki_layout::` (5 path constructions in `write_sensor` / `write_clock` / `write_frame` / `read_sensor` / `read_clock` / `read_frame`, plus the crate-level doc-comment `[auki_session]` link). The on-disk path layout is unchanged; the rename is purely about the upstream crate's name-vs-scope mismatch. README + `src/readme.md` cross-reference targets updated correspondingly. No code or test changes; depends on the companion rename in [`auki-layout/changelog.md`](../auki-layout/changelog.md). Doc-only at the source level (the source change is the import path).

### broodsugar's claude · May 8, 11:30 HKT, 2026

**Step 1 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md) landed.** `SensorLogEntry` (renamed `PinholeCameraLogEntry`) and `DynamicIntrinsics` moved to [`auki-datatypes`](../auki-datatypes) under the new `auki.camera` `.proto` package. Encoding switched from CBOR-via-ciborium to protobuf via prost. Manifest shape is unchanged — same `(sensor_id, sensor_hash)` resolution against the Sensor Registry tells a reader the segments hold `PinholeCameraLogEntry` rather than another payload type.

**Removed from this crate:**

- `pub struct SensorLogEntry { dynamic_intrinsics, frame }` (the `pub` type; renamed to `PinholeCameraLogEntry` at the new home)
- `pub struct DynamicIntrinsics { fx, fy, cx, cy, distortion_coefficients }`

The two types had no inline tests in this crate (test count unchanged at 35).

**Docs**: outer `README.md` Sensor Log payload section replaced with a one-paragraph pointer at [`auki-datatypes`](../auki-datatypes); the same crate's "two kinds of typed data" intro now spells out that the camera payload departed at Step 1 while the others (Point Cloud / Audio / Pose) are still here. `src/readme.md` Log payload types code block dropped the `SensorLogEntry` struct + the `DynamicIntrinsics` struct + the description of `SensorLogEntry.frame`'s `serde_bytes` tagging. Schema-version line updated to flag that `PinholeCameraLogEntry` + `DynamicIntrinsics` version independently in their new home.

The `PoseLogEntry` + `TransformSample` payload types still live here — those move out at Step 5. `PointCloudLogEntry` (Step 3) and `AudioLogEntry` (Step 4) likewise. Will land in v0.0.24.

### broodsugar's claude · May 8, 09:00 HKT, 2026

**Step 0 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md) landed.** Pure refactor extracting manifest concerns into the new [`auki-manifests`](../auki-manifests) crate. No behaviour change, no encoding change.

**Removed from this crate** (moved to [`auki-manifests`](../auki-manifests)):

- `pub fn build_sensor_log_manifest(...)`
- `pub fn build_pose_log_manifest(...)`
- `pub enum PoseSource` (with `canonical_bytes` + `hash` impls)
- The corresponding tests: `build_sensor_log_manifest_contains_all_required_fields`, `sensor_log_manifest_opens_a_log_round_trip`, `m1_ros2_tf_source` helper, `ros2_tf_source_serializes_to_canonical_bytes`, `ros2_tf_source_hash_is_locked`, `build_pose_log_manifest_contains_all_required_fields`, `pose_log_manifest_opens_a_log_round_trip`.
- The private `duration_as_i64_ns` helper (only used by the moved builders).

**Test count: 41 → 35.** All locked-vector semantics preserved at their new home in `auki-manifests` — the `f3d296341347589c72297a0cc7c81cd8` PoseSource hash and the M1 example JCS bytes round-trip identically.

**Docs**: outer `README.md` Pose Log section now points at [`auki-manifests`](../auki-manifests) for `PoseSource` + `build_pose_log_manifest`; `src/readme.md` removed the `PoseSource` definition + builder signatures + the moved test rows. Schema-version line notes that `PoseSource` versions independently in its new home.

The `PoseLogEntry` + `TransformSample` payload types still live here — those move out in Step 5 of the migration (per the [2026-05-07 Pose Log synthesis](../../parking_lot.md)). Will land in v0.0.24.

### broodsugar's dobby · May 7, 22:00 HKT, 2026

**[`auki-registry/README.md`](README.md): per-section "Departing" callouts** + corresponding [`parking_lot.md`](parking_lot.md) item. Each of the four log-payload sections (Sensor Log, Point Cloud Log, Audio Log, Pose Log) now opens with a one-line italic callout pointing at the matching migration step in [`auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md). Pose Log callout additionally cross-links the root parking-lot Propagate task and notes the wrapper-removal + manifest-`(from, to)` reshape. Per-type detail (manifest tables, CBOR shapes, design rationale) stays in this README until each type physically moves to `auki-datatypes` — that doc move is sequenced with the code move per the new parking-lot item. Doc-only.

### broodsugar's claude · May 7, 21:15 HKT, 2026

README + `src/readme.md` opening framing updated to flag the **scope-shrink-in-flight** decided 2026-05-07. The opening "the typed data shapes the SDK persists — registry entries AND log payloads" claim is the AI-drift framing identified in the same session; corrected to *"the SDK's identity catalog"* with an explicit callout that the log payload types currently here (`SensorLogEntry` / `PointCloudLogEntry` / `AudioLogEntry` / `PoseLogEntry` / `TransformSample` / `DynamicIntrinsics` / `build_pose_log_manifest`) are migrating to [`auki-datatypes`](../auki-datatypes) step-by-step. Sequence in [`auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md). Content describing today's code (which still has the old types under their old names) left intact — accurate for the current state. The framing fix is what mattered. Doc-only.

### broodsugar's claude · May 7, 17:30 HKT, 2026

README sweep aligning Sensor/Clock-Log and Pose-Log prose with the [Control API rewrite](../../docs/control-api.md). Path placeholders: `<session>/sensorlogs/<recording_uuid>/` → `<sensor_log_id>` (in the Point Cloud Log section); `<session>/poselogs/<recording_uuid>/` → `<pose_log_id>` (in the Pose Log section). The Sensor Log manifest's `session_id` row now references `/api/info`'s `session_id` instead of `/api/state`'s `session_uuid` (`/api/state` is gone in the v0.0.23 spec). Pose Log narrative drops the "ring buffer + intent captures, distinguished only by `retention_ns`" auto-buffer framing for the unified-log model. Doc-only; no code changes.

### broodsugar's claude · May 7, 11:00 HKT, 2026

**Frame Registry landed** — third registry alongside Sensor + Clock; closes the parking-lot "Frame Registry shape" question. New `FrameRegistryEntry { frame_id, handedness, axes, units }` with `Handedness { Right, Left }`, `AxisConvention { x, y, z: AxisDirection }`, `AxisDirection { Forward, Backward, Up, Down, Left, Right }`, and `LengthUnit { Meters, Millimeters, Centimeters }`. **Tree structure deliberately not on the entry** — frame parentage lives in the Pose Log via `TransformSample.parent_frame` / `child_frame`; the registry declares what each frame *is in isolation* and the Pose Log declares the edges between them. **Rotation representation deliberately not on the entry** — quaternions are fixed at the `TransformSample` layer (Hamilton convention `[x, y, z, w]`); not per-frame. **No `label` field** — `frame_id` strings are already human-readable; defer until a real ask.

**Four preset constructors** — `FrameRegistryEntry::ros_body(frame_id)` (REP-103 body: right, x=forward y=left z=up, meters), `::ros_optical(frame_id)` (REP-103 optical: right, x=right y=down z=forward, meters), `::opengl(frame_id)` (right, x=right y=up z=backward, meters), `::unity(frame_id)` (left, x=right y=up z=forward, meters). On-disk JSON is fully spelled-out either way — presets are pure ergonomics, no shorthand on the wire. Cross-language readers parse the explicit fields, never the preset name.

**Validation** — `FrameRegistryEntry::validate()` checks the `AxisConvention` triplet is orthogonal (the three axes must be drawn from three distinct axis-pairs: forward/backward, left/right, up/down). `write_frame()` calls `validate()` before hashing — bad axes return `Error::InvalidAxes(detail)` without touching disk. Handedness consistency vs. axes is **not** cross-checked — both fields are integrator declarations.

**`SensorBody::PointCloud`**** and ****`SensorBody::RgbCamera`**** gain a required ****`frame_id: String`**** field** referencing a `FrameRegistryEntry`. This is what fixes the symptom that triggered this work: today the SDK ships `PointCloudFrame` bytes over the Dagaz wire with no declared coordinate convention; with this PR, the producer's `AcceptInfo.sensor_hash` transitively names the frame via `SensorRegistryEntry → frame_id → FrameRegistryEntry`, so a consumer (Park, future Sentinel) can look up the convention to interpret the XYZ axes. **Breaking on-disk for existing Sensor Registry entries** — pre-1.0, integrators regenerate; the `auki-ros-adapter` builders gained matching parameters (`StaticCameraMetadata.frame_id`, `build_point_cloud_registry_entry(.., frame_id)`) and BoosterApp's sidecar will need to thread `frame_id` through.

**Cross-peer registry sync deliberately out of scope.** Park can fetch a `JpegFrame` / `PointCloudFrame` from a Booster peer over libp2p but has no path today to fetch the producer's `FrameRegistryEntry`. That's a Layer 2 networking deliverable (registry advertisement / fetch over libp2p); for v1 the Frame Registry is local-disk only and cross-peer convention agreement is by configuration. Park reads its own local frame entries; once Layer 2 ships the registry can flow over libp2p like any other content-addressed entry.

**New locked cross-language conformance vector** — `FrameRegistryEntry::ros_body("K1-AABBCCDDEEFF/base_link")` pinned to JSON `{"axes":{"x":"forward","y":"left","z":"up"},"frame_id":"K1-AABBCCDDEEFF/base_link","handedness":"right","units":"meters"}` and XXH3-128 `fd0dc3789e898b71b5e16ee122a81a44`. Joins the `auki-hash` / `auki-identity` / `auki-network` cross-language conformance set. Locked sensor + point cloud hashes recomputed: `sensor_entry_hash_is_locked` `e8cb38..` → `d798fa..`; `point_cloud_entry_hash_is_locked` `35b318..` → `79b58e..`. `auki-ros-adapter`'s `build_*_registry_entry_matches_locked_hash` tests track the same updated values.

**Storage** — `<app_root>/registries/frames/<frame_id>/<hash>.json`, sibling to `sensors/` and `clocks/`. `auki-session` got `frame_entry_path()`; `auki-registry` got `write_frame` / `read_frame` with the same idempotent + content-addressed semantics as the sensor/clock pair. New `Error::InvalidAxes(String)` variant. 12 new tests (auki-registry 30 → 41): locked vector + hash + 4 preset round-trips + 2 validate cases + write_frame disk-protection + write/read round-trip + idempotency + read-missing-returns-none.

Will land in v0.0.22.

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
