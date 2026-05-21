# Changelog — auki-datatypes

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

**Camera and detection payloads now use the final frame names.** `auki.camera` exports `CameraFrame`, `auki.detection` exports `DetectionFrame`, and the Rust `LogPayload` impls/tests now pin those names directly. The wire bytes stay structurally identical; this is a breaking source/API cleanup with no legacy aliases.

### Nils's codex · May 18, HKT, 2026

**Camera streams now reuse `auki.camera.PinholeCameraLogEntry` directly.** The stream-only `auki.frame_stream.JpegFrame` package is retired: `proto/frame_stream.proto` is deleted, `build.rs` no longer generates it, and `src/lib.rs` no longer exports a `frame_stream` module. `/auki/stream/0.1.0` camera entries now carry the same prost record bytes as camera Sensor Logs, so "robot log == Park stream" holds at the payload-record level.

Docs and parking-lot propagation now say Step 2's camera path reuses `auki.camera`, while `auki.point_cloud_stream.PointCloudFrame` remains stream-specific. This resolves the stale package-naming question for camera payloads without changing the existing point-cloud stream wrapper.

### Arshak's claude · May 16, HKT, 2026

**Cuba T5 + T12 — `DetectionLogEntry` gains `sensor_hash` (field 2) and `type` (field 3).** Each detection record now self-describes its source sensor (T5) and carries an open-string discriminator the consumer uses to pick a per-type decoder for the opaque `data` bytes (T12 — vocabulary like `aruco`, `portal`, `portal_corner`, `esl`, `person`; canonical list in [`detectors/README.md`](https://github.com/aukilabs/detectors)). Field number ledger updated in [`proto/detection.proto`](proto/detection.proto); the SDK still does not decode `data` — application code owns per-type decoders.

**Wire compatibility — preserved.** Proto3 default-elides empty fields, so the existing locked-bytes vector for `step8_detection_log_entry` (`0a0c000102030405060708090a0b`, XXH3-128 `94f8efe6be63d3dc5e045ab08d538a15`) is byte-identical across the addition. Old readers ignore unknown fields; old writers emit no `sensor_hash`/`type` and the new fields stay empty. The test `detection_log_entry_serializes_to_locked_wire_bytes` is the canary against drift.

**Tests**: 51 → 52 (+1 — `detection_log_entry_cuba_fields_round_trip` covering populated-field round-trips). All existing locked-vector tests still pass.

**Touched**: [`proto/detection.proto`](proto/detection.proto) (two-field extension with field-number ledger update); [`src/lib.rs`](src/lib.rs) (fixture, two inline literals, new round-trip test); the Python parity file [`auki-datatypes-py/auki_datatypes/auki/detection.py`](../auki-datatypes-py/auki_datatypes/auki/detection.py) (hand-edited; `protoc-gen-python_betterproto` not installed locally).

**Context**: Commit 1/6 of the Cuba v0.0.45 migration plan in [`exocortices/arshak/cuba/migration-plan-v0.0.45.md`](https://www.notion.so/35d5c8e96592803ab914fdc6f0a8aecd). Sibling commits land `auki_layout::detector_entry_path`, `DetectorRegistryEntry` (+ Cuba T16 `output_types`), `RegistryKind::Detector` on `/auki/registries/0.0.1`, and `StreamDispatch::AcceptDetection` (Cuba T8). The Cuba ArUco daemon consumes all five.

### Nils's codex · May 16, 12:31 HKT, 2026

**`auki.stream` accept metadata is now `StreamDescriptor`.** The stream accept message now carries the live stream descriptor Park needs: `sensor_id`, `sensor_hash`, `clock_id`, `clock_hash`, `frame_id`, and `frame_hash`. This replaces the narrower `AcceptInfo { sensor_hash, clock_id, clock_hash }` shape and keeps `StreamMessage::Accept(...)` as the handshake variant. `frame_id` / `frame_hash` are empty for non-spatial streams; spatial streams publish the exact frame registry reference alongside the sensor and clock references. Breaking wire/source change by design while the SDK is pre-consumer-lock. `cargo test -p auki-datatypes` passes.

### Nils's claude · May 14, 12:54 HKT, 2026

**Dialogue Batch 1 (SDK Rust core, half 1) — new `auki.audio_stream` proto package + `AudioFrame { bytes data = 1; }` wire payload.** Mirror of `auki.joint_encoders` / `auki.joint_encoders_stream` Step 5 precedent: separate proto package so the wire and log code paths dispatch on distinct Rust types, byte-identical wire/disk locked by a dedicated symmetry test. Companion to [`auki-network` changelog 2026-05-14 12:54](../auki-network/changelog.md) which adds the `StreamDispatch::AcceptAudio` arm + dispatch site.

**Opaque-bytes-only**, matching the Step 4 stance for `AudioLogEntry`. `sample_format` / `channels` / `sample_rate_hz` / `channel_layout` resolution comes from `(sensor_id, sensor_hash) → SensorBody::Audio` at handshake; the wire payload carries no per-frame metadata. Sample count is derivable as `data.len() / (sample_byte_width × channels)`; chunk duration is derivable as `sample_count × 1e9 / sample_rate_hz`. Carrying either on the per-frame payload would denormalize derivable metadata into the bytes for marginal reader convenience and risk inconsistency between the field and the bytes — same trade Step 3 / 4 declined.

**Why a separate `auki.audio_stream` proto package** (rather than reusing `auki.audio.AudioLogEntry` on the wire): the wire and log code paths need distinct Rust types so the runtime can dispatch on them (the `StreamDispatch` enum has one arm per `T`); a shared proto package would force every consumer to disambiguate at the call site instead of at the type level. Step 2/3 (point_cloud) and Step 5 (joint_encoders) already established this precedent; this is the fourth paired package and the second one (after joint_encoders) where the symmetry is non-trivially asserted at byte level via a dedicated test.

**Tests**: 46 → 51 (+5 — `audio_frame_serializes_to_locked_wire_bytes`, `audio_frame_hash_is_locked`, `audio_frame_round_trips`, `audio_frame_empty_data_round_trips`, `audio_disk_wire_byte_identical`). Locked wire bytes for the example 16-byte chunk `[0x00, 0x11, …, 0xff]`: `0a1000112233445566778899aabbccddeeff` — **byte-identical to the existing `AudioLogEntry` locked vector** (same field number, same field type, same fixture data). XXH3-128 hash: `a5864ae7018f28a5c094a714af1db62e` — also identical, locking the symmetry property at the hash level too. New test `audio_disk_wire_byte_identical` asserts `entry.encode_to_vec() == frame.encode_to_vec()` directly.

**Touched**: new [`proto/audio_stream.proto`](proto/audio_stream.proto); [`build.rs`](build.rs) gains the file in the compile list; [`src/lib.rs`](src/lib.rs) gets the `pub mod audio_stream` module include next to the other `_stream` modules plus the five new tests.

**Out of scope for this PR** (each lives in a sibling PR or future task — see the [Dialogue quest](https://www.notion.so/3595c8e965928022bb8ecb9a1b0fa46c) on Notion): the Python binding extension on `auki-network-py` (Dialogue T2); Park's producer infrastructure (Dialogue T3); Boosterapp's consumer + K1 player driver (Dialogue T4 + T5).

### Nils's claude · May 14, 11:00 HKT, 2026

**Doc-comment + proto-comment touch-ups for upstream `SensorBody::Microphone` → `SensorBody::Audio` rename.** Companion to [`auki-registry` changelog 2026-05-14 11:00](../auki-registry/changelog.md). No prost wire change (the `auki.audio` package + `AudioLogEntry { bytes data = 1; }` message are unchanged); only the cross-references that named the *registry* body type flip. Touched: [`README.md`](README.md) Step 4 bullet, [`src/readme.md`](src/readme.md) audio entry, [`src/lib.rs`](src/lib.rs) `auki.audio` module doc-comment, [`proto/audio.proto`](proto/audio.proto) header comment, [`proto/detection.proto`](proto/detection.proto) analog reference, [`parking_lot.md`](parking_lot.md) three structured-vs-opaque-bytes references.

### broodsugar's claude · May 9, 13:05 HKT, 2026

**[`parking_lot.md`](parking_lot.md): structured prost fields vs opaque bytes — when does each apply?** Filed after [#77](https://github.com/aukilabs/auki-sdk/pull/77) (JointEncoders) made the split precedent visible. Lists the seven on-disk types in two buckets — opaque-bytes (`PointCloudLogEntry`, `AudioLogEntry`, `DetectionLogEntry`) and structured (`PinholeCameraLogEntry`, `SpatialTransform`, `TimeTransformEntry`, `JointEncodersLogEntry`) — and proposes a working principle: **structured if** the bytes have a single canonical interpretation across all instances of the sensor type; **opaque-bytes if** the bytes have multiple possible layouts the producer must specify or the schema is owned downstream. Forward path: pin in [`src/readme.md`](src/readme.md) when a future payload-type design needs to reference it. Doc-only.

### broodsugar's claude · May 9, 12:46 HKT, 2026

**`auki.joint_encoders` / `JointEncodersLogEntry` (on-disk) and `auki.joint_encoders_stream` / `JointEncodersFrame` (wire) landed.** New paired proto packages, `repeated float angles_rad = 1` on both sides, byte-identical wire/disk shape locked by an explicit `joint_encoders_disk_wire_byte_identical` symmetry test. Producer ships angle vectors; consumer (Park) holds the URDF and does FK. Mirrors the [`Microphone`](../auki-registry/src/lib.rs) / [`PointCloud`](../auki-registry/src/lib.rs) layering — the producer ships raw measurements and just enough deserialization metadata (`joint_count` on the registry body) for the consumer to read the bytes correctly. Schema-for-interpretation lives downstream.

**Why both wire and disk in the same PR.** The [Step 2/3 point-cloud pattern](src/lib.rs) — `auki.<thing>` (disk, `LogPayload`) + `auki.<thing>_stream` (wire, plain `prost::Message`) — already paid the cost of the symmetric structure. Disk-only would have forced boosterapp's existing libp2p stream path to fall back to ad-hoc bytes-on-the-wire and required a follow-up PR for the wire type. Step 2/3 had `bytes`-only payloads so symmetry was trivially true; `JointEncoders` has structured fields, so the symmetry is locked by the new explicit byte-equality test alongside the locked-vector tests.

**Resolved parking-lot questions** (folded into this PR per the [migration-architecture-decisions cadence](parking_lot.md)): `JointEncodersLogEntry` and `JointEncodersFrame` are structured (`repeated float angles_rad`) and ship as a paired wire/disk package; `angles_rad` precision is f32 (matches `SpatialTransform`'s quaternion components); no `velocity_rad_per_s` / `effort_nm` companion fields in v1 (minimal-fields stance from Steps 3/4 — adding new proto fields later is cheap, baking them in now is forever).

**Tests**: 37 → 46 (+9 — `joint_encoders_log_entry_serializes_to_locked_wire_bytes`, `joint_encoders_log_entry_hash_is_locked`, `joint_encoders_log_entry_round_trips`, `joint_encoders_log_entry_log_payload_round_trips`, `joint_encoders_log_entry_empty_angles_round_trips`, `joint_encoders_log_entry_segment_round_trip`, `joint_encoders_frame_serializes_to_locked_wire_bytes`, `joint_encoders_frame_round_trips`, `joint_encoders_disk_wire_byte_identical`). Locked wire bytes for the 6-DOF fixture `[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]`: `0a18000000000000803f0000004000004040000080400000a040`. XXH3-128 hash: `150a56272692540cf5d8e8e93dc74b7a`.

### broodsugar's claude · May 8, 15:34 HKT, 2026

**Step 8 of the [migration](src/sprint.md) landed — `auki.detection` / `DetectionLogEntry` (on-disk).** New `proto/detection.proto` with `DetectionLogEntry { bytes data = 1; }` — opaque-bytes-only, same stance as Steps 3 (point cloud) and 4 (audio). The detection schema is defined per-Detector (QR portal-uid + four corners + content; ESL class + bbox + confidence; people bboxes); the SDK does not interpret detector-specific fields. Carrying detector-specific fields on the prost type would either lock the SDK into knowing every detector's schema or force a degenerate `oneof` of every shipped detector — neither scales.

**Closes the producer side of the [subscription-as-materialization keystone](../../parking_lot.md)** filed by Dobby earlier today. A Detection Log is `Log<T>` with `T = DetectionLogEntry`, lifecycle inherited from the sensor-log primitive — no "DetectionLog" abstraction. Sharpens [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #3 ("`DetectionLogEntry` type") into landed code.

**Not a migration step in the same sense as 1–6.** No source type existed to move; this is a new type added to close the producer side of the keystone. Numbered Step 8 to slot after the placeholder cleanup at Step 7. The previous Step 8 (Python codegen) is renumbered Step 9.

**`auki-logs` needed no changes** — encoder-agnostic since Step 1.

**Tests**: 31 → 37 (+6 — `serializes_to_locked_wire_bytes`, `hash_is_locked`, `round_trips`, `log_payload_round_trips`, `empty_data_round_trips`, `segment_round_trip`). Locked wire bytes for a 12-byte fixture: `0a0c000102030405060708090a0b`. XXH3-128 hash: `94f8efe6be63d3dc5e045ab08d538a15`.

**Out of scope for this PR** (each is its own future single-PR landing): `Log<T>::tail()` (the read side of the keystone, in [`auki-logs`](../auki-logs)); the Detector binding API; the [`auki-sdk-py`](../auki-sdk-py) Python binding; the Detection-Log analog of `SensorRegistryEntry` (lives in [`auki-registry`](../auki-registry) when subscription / discovery for detection logs needs it).

**Resolved parking-lot question**: the opaque-bytes-vs-typed-shape slop point for `DetectionLogEntry` — adjudicated in favour of opaque-bytes-only and propagated in this same PR.

### broodsugar's claude · May 8, 14:37 HKT, 2026

**Step 7 of the [migration](src/sprint.md) landed — placeholder cleanup. The migration is complete.** Deleted `proto/placeholder.proto`, the `placeholder` module in [`src/lib.rs`](src/lib.rs), the `placeholder_pipeline_check_round_trips` smoke test, and the corresponding line in [`build.rs`](build.rs). The seven real `.proto` packages (camera, point_cloud, audio, pose, time_transform, frame_stream, point_cloud_stream, stream) serve as proof that the prost-build pipeline works; the placeholder no longer earned its keep. Test count: 32 → 31.

(Re-application — the original Step 7 PR #70 merged into the Step 6 stacked branch but didn't propagate to develop after #69's squash-merge severed the ancestry. This PR re-applies the change against current develop so the migration's final state matches develop's tree.)

### broodsugar's claude · May 8, 12:43 HKT, 2026

**Step 6 of the [migration](src/sprint.md) landed — `auki.time_transform` / `TimeTransformEntry` (on-disk).** New `proto/time_transform.proto` with `TimeTransformEntry { int64 offset_ns = 1; uint32 uncertainty_ns = 2; }`. **All three slop points resolved**: (a) per-entry `source` moved to manifest as a tagged-enum `TimeTransformSource` (mirrors `PoseSource`); (b) per-entry `discontinuous: bool` dropped — readers compute `|offset_ns - prev_offset_ns| ≥ reader_threshold` against their own tolerance; (c) `TimeTransformSource` kept as tagged enum at the manifest layer (Option 2 — matches `PoseSource`'s extension pattern with one variant today, `LocalClockRead`).

**Moved** `TimeTransformEntry` out of [`auki-time-transforms`](../auki-time-transforms) into this crate; `TimeTransformSource` moved to [`auki-manifests`](../auki-manifests) (manifest metadata, not per-entry). [`auki-time-transforms`](../auki-time-transforms)'s `tick`/`Sampler` simplified — no more `SamplerState`, no more `discontinuity_threshold` arg.

**Tests**: 25 → 32 (+7 — `serializes_to_locked_wire_bytes`, `hash_is_locked`, `round_trips`, `log_payload_round_trips`, `zero_offset_round_trips`, `negative_offset_round_trips`, `segment_round_trip`). Locked wire bytes for `offset_ns: 1_000_000, uncertainty_ns: 250` — `08c0843d10fa01` (7 bytes; both varint fields). XXH3-128 hash: `b7e73628833419a7c299933d07cbe88c`. The negative-offset test pins prost's non-zigzag varint encoding for proto3 `int64` (10 bytes for negatives).

**On-disk migration complete.** All five log payload types live here as prost types; only `placeholder.proto` remains, removed at Step 7.

### broodsugar's claude · May 8, 11:52 HKT, 2026

**Step 5 of the [migration](src/sprint.md) landed — `auki.pose` / `SpatialTransform` (on-disk, flat).** New `proto/pose.proto` with `SpatialTransform { Vec3 translation; Quat orientation }` + `Vec3 { double x, y, z }` + `Quat { double x, y, z, w }`. The pre-migration `auki_registry::PoseLogEntry { transforms: Vec<TransformSample> }` wrapper is gone, and per-sample `parent_frame` / `child_frame` strings are gone — frame identity lives in the manifest's `(from_frame_id, to_frame_id)` pair, mirroring how TimeTransform Log keys per `(from_clock_id, to_clock_id)`.

**Coordinated downstream changes** (all 2026-05-08):
- [`auki-registry`](../auki-registry): `PoseLogEntry` + `TransformSample` removed; `ciborium` dev-dep dropped (was their last user).
- [`auki-manifests`](../auki-manifests): `build_pose_log_manifest` rewritten with 13 args including `from_frame_id` + `from_frame_hash`, `to_frame_id` + `to_frame_hash`, `writer_mode: PoseWriterMode` (`Rigid` / `Movable`), `expected_rate_hz: u32` per the 2026-05-07 synthesis. New `PoseWriterMode` enum.
- [`auki-layout`](../auki-layout): `poselog_path` now `(session_root, from_frame_id, to_frame_id) -> PathBuf`, mirroring `timetransform_log_path`.
- [`auki-logs`](../auki-logs): no changes (encoder-agnostic since Step 1).

**Tests**: 19 → 25 (+6 — `serializes_to_locked_wire_bytes`, `hash_is_locked`, `round_trips`, `log_payload_round_trips`, `default_round_trips`, `segment_round_trip`). Locked wire bytes for the identity-rotation 1-2-3-translation fixture: `0a1b09000000000000f03f110000000000000040190000000000000840120921000000000000f03f`. XXH3-128 hash: `29fa6349ab0b3ff1f06933489db74dfd`. proto3 default-elision means zero-valued doubles inside `Vec3`/`Quat` don't appear on the wire — pinned both in the locked vector test (the Quat's `x=y=z=0` fields are absent from the bytes) and in the dedicated `_default_round_trips` test.

**Resolved parking-lot question**: the Pose Log manifest reshape per the 2026-05-07 synthesis — adjudicated and propagated in this same PR. Two slop fixes remaining (TimeTransformEntry source/discontinuous, TimeTransformSource collapse) — both resolve at Step 6.

### broodsugar's claude · May 8, 11:20 HKT, 2026

**Step 4 of the [migration](src/sprint.md) landed — `auki.audio` / `AudioLogEntry` (on-disk).** New `proto/audio.proto` with `message AudioLogEntry { bytes data = 1; }` — opaque-bytes-only (Option A in the parking-lot slop point, adjudicated in favour). Same stance as Step 3 for point clouds; declines the pre-Step-3 lean toward adding `sample_count: u32`. Sample count and chunk duration are derivable from the bytes plus the SensorRegistryEntry's `Microphone { sample_format, channels, sample_rate_hz }` body — denormalizing either field would risk inconsistency for marginal reader convenience.

**Moved** `AudioLogEntry` out of [`auki-registry`](../auki-registry); no downstream consumers needed updates (no `auki-ros-adapter` builder for audio yet). [`auki-logs`](../auki-logs) needed no changes — encoder-agnostic since Step 1.

**Tests**: 13 → 19 (+6 — `serializes_to_locked_wire_bytes`, `hash_is_locked`, `round_trips`, `log_payload_round_trips`, `empty_data_round_trips`, `segment_round_trip`). Locked wire bytes for a 16-byte `pcm_s16le` stereo fixture: `0a1000112233445566778899aabbccddeeff`. XXH3-128 hash: `a5864ae7018f28a5c094a714af1db62e`.

**Resolved parking-lot question**: the implicit-vs-explicit chunk-metadata slop point — adjudicated and propagated in this same PR.

### broodsugar's claude · May 8, 10:51 HKT, 2026

**Step 3 of the [migration](src/sprint.md) landed — `auki.point_cloud` / `PointCloudLogEntry` (on-disk).** New `proto/point_cloud.proto` with `message PointCloudLogEntry { bytes data = 1; }` — opaque-bytes-only (Option A in the parking-lot slop point, adjudicated in favour). Symmetric with the wire's `PointCloudFrame`; the pre-migration ROS-shaped fields `width` / `height` / `is_dense` are gone — interpretation comes from `(sensor_id, sensor_hash) → SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id }`.

**Moved** `PointCloudLogEntry` out of [`auki-registry`](../auki-registry); [`auki-ros-adapter`](../auki-ros-adapter)'s `build_point_cloud_log_entry` now returns the prost type with just `data` set. [`auki-logs`](../auki-logs) needed no changes — encoder-agnostic since Step 1.

**Tests**: 7 → 13 (+6 — `serializes_to_locked_wire_bytes`, `hash_is_locked`, `round_trips`, `log_payload_round_trips`, `empty_data_round_trips`, `segment_round_trip`). Locked wire bytes for a 24-byte fixture: `0a18000102030405060708090a0b0c0d0e0f1011121314151617`. XXH3-128 hash: `4ea525d849212b2e067e33bec455c7ea`.

**Resolved parking-lot question**: the on-disk-vs-wire drift slop point — adjudicated and propagated in this same PR.

### broodsugar's claude · May 8, 10:11 HKT, 2026

**Doc cleanup after Step 2.** [`README.md`](README.md) layout block enumerates all five `.proto` files (placeholder + camera + frame_stream + point_cloud_stream + stream); [`src/readme.md`](src/readme.md) reflects the five generated modules, the public surface (`StreamMessage`, `StreamRequest`, `AcceptInfo`, `Frame`, `DeclineReason`, `EndReason` plus their helper constructors), and updates the consumers section (`auki-network` and `auki-network-py` both consume the Step 2 types). Status section in the outer README now lists Steps 1 and 2 as landed. Doc-only.

### broodsugar's claude · May 8, 09:42 HKT, 2026

**Step 2 of the [migration](src/sprint.md) landed — libp2p substream wire moves to protobuf.** Three new `.proto` packages: `auki.frame_stream { JpegFrame }`, `auki.point_cloud_stream { PointCloudFrame }`, `auki.stream` (full envelope `StreamMessage` oneof of `Request | Accept | Decline | Frame | EndOfStream`).

**Per-step decision: `Frame.payload = bytes`.** T inferred from `AcceptInfo.sensor_hash` → `SensorRegistryEntry.body` (same chain that already governs on-disk segment payloads). The substream is mono-T per Dagaz D1; oneof-on-every-frame would be redundant.

**Helper constructors on the prost-generated types** — `StreamMessage::request(req) | accept(info) | decline(reason) | frame(f) | end_of_stream(reason)`, `DeclineReason::sensor_not_found() | sensor_unavailable() | producer_shutting_down() | other(detail)`, same shape on `EndReason`. Lives in `pub mod stream { … }` in [`src/lib.rs`](src/lib.rs); orphan rule satisfied since impls sit in the type's defining crate. Verbose match patterns at the call site — `match reason.kind { Some(decline_reason::Kind::SensorNotFound(_)) => … }` — were the alternative; the helpers buy ergonomics.

**Locked cross-language conformance vectors** for `JpegFrame` + `PointCloudFrame` wire bytes pinned in [`auki-network::stream_protocol::tests`](../auki-network/src/stream_protocol.rs). Also pinned: a `StreamMessage::Frame { PointCloudFrame }` envelope round-trip, end-to-end through `read_message` / `write_message`. Cross-language readers (Park's browser-side decoder, future Sentinel ports) MUST reproduce these bytes.

**Test count: 7 → 7** (helpers don't add new tests in this crate; the wire pin lives in `auki-network`).

### broodsugar's claude · May 8, 11:30 HKT, 2026

**Step 1 of the [migration](src/sprint.md) landed — first real schema. `auki.camera` ships `PinholeCameraLogEntry` + `DynamicIntrinsics`** with locked wire-bytes and XXH3-128 hash (`0496e1f71a03e00877fc68bf16190026`).

**Per-step decision: `dynamic_intrinsics` is inline-optional.** `proto/camera.proto`:

```proto
message DynamicIntrinsics { double fx=1; double fy=2; double cx=3; double cy=4; repeated double distortion_coefficients=5; }
message PinholeCameraLogEntry { DynamicIntrinsics dynamic_intrinsics=1; bytes frame=2; }
```

prost generates `dynamic_intrinsics: Option<DynamicIntrinsics>` for proto3 message fields. Non-autofocusing cameras pay only the message-tag overhead when `None`; autofocusing cameras populate per-frame. Promoting to a sibling intrinsics-update sub-stream remains a backward-compatible move (drop the field, mark its number reserved, add a sibling log) — but punted until autofocus shows up as a real workload.

**`impl_log_payload!` macro** in [`src/lib.rs`](src/lib.rs) wires every prost type into [`auki_logs::LogPayload`](../auki-logs/src/lib.rs) with one line of glue:

```rust
macro_rules! impl_log_payload { ($t:ty) => { /* encode_to_vec / decode + map_err */ }; }
impl_log_payload!(camera::PinholeCameraLogEntry);
```

Step 6's `TimeTransformEntry` will pick up the same macro; mid-migration ciborium types implement `LogPayload` directly.

**Locked vectors** (`tests::pinhole_camera_log_entry_serializes_to_locked_wire_bytes` + `_hash_is_locked`) join the workspace's cross-language conformance set. Cross-language readers (Python via betterproto, future Sentinel ports) MUST reproduce the bytes byte-identically.

**End-to-end seam test** opens an `auki_logs::Log<PinholeCameraLogEntry>`, appends two entries (one with intrinsics, one without), closes, re-reads, asserts both timestamp + payload byte-equality. Catches any regression in the macro wiring or the segment-framing path.

**New deps**: `auki-logs` (path-dep — needs the trait); dev-deps `auki-hash` (locked hash) + `serde_json` + `tempfile` (segment round-trip). Production deps add `auki-logs` only.

**Test count: 1 → 7.** Placeholder smoke test stays until Step 7 retires it. Will land in v0.0.24.

### broodsugar's dobby · May 7, 22:30 HKT, 2026

**Migration architecture decisions added to [`parking_lot.md`](parking_lot.md), Step 0 added to [`src/sprint.md`](src/sprint.md).** Two upfront decisions: (1) **Manifest encoding stays JCS-JSON, not protobuf** — JCS gives free cross-language byte-equivalence which protobuf doesn't, manifests are human/browser/ad-hoc-tool-readable, and per-recording metadata doesn't benefit from wire compactness. (2) **`build_*_log_manifest` builders + manifest schemas → new `auki-manifests` crate** — symmetric with this crate (which owns segment payload shapes); `auki-manifests` owns manifest shapes. Sequenced as **Step 0** before migration step 1, pure refactor extracting `build_sensor_log_manifest` + `build_pose_log_manifest` from `auki-registry` and `build_manifest` from `auki-time-transforms`. Naming: `auki-manifests` over `auki-logging` (idiom collision in Rust — reads as observability/tracing). Doc-only.

### broodsugar's claude · May 7, 21:00 HKT, 2026

**Crate renamed `auki-proto` → `auki-datatypes`.** Names the responsibility (canonical shared cross-language data types) instead of the implementation (protobuf via prost). Aligns with the rest of the workspace's concept-naming convention (`auki-registry`, `auki-logs`, `auki-session`, `auki-time-transforms`, `auki-network` — all named for their purpose, not their internals). Future-proofs against any downstream encoding switch.

**Scope clarified, accidental dual-purpose split out.** Per the [Notion Registries doc](https://www.notion.so/34e5c8e96592809d8977feb17c32e5d0)'s definition (*"a shared, versioned catalog of identities + definitions"*), [`auki-registry`](../auki-registry) is supposed to hold registry entries only — Sensor / Frame / Clock identity + definitions, JCS-canonical JSON, content-hashed. The log payload types (`SensorLogEntry`, `PointCloudLogEntry`, `AudioLogEntry`, `PoseLogEntry`, `TransformSample`, `DynamicIntrinsics`) currently dumped in `auki-registry` were AI drift Nils didn't catch. Each migration step now **moves** a type from `auki-registry` into here (rather than the earlier framing of "auki-registry re-exports from auki-datatypes"). Post-migration, `auki-registry` shrinks back to its canonical scope; consumers add an `auki-datatypes` dep alongside.

**Locked renames in [`src/sprint.md`](src/sprint.md):**

- `SensorLogEntry` → `PinholeCameraLogEntry` — names what it actually is (pinhole-projection camera frame entry; `DynamicIntrinsics` is pinhole-shaped). The original generic-sounding name was wrong.
- `TransformSample` → `SpatialTransform` — matches the [Notion Pose Log doc](https://www.notion.so/34b5c8e9659280bd9580c25991f5d491). Also drops the `PoseLogEntry { transforms: Vec<TransformSample> }` wrapper — flat segments per the Pose Log synthesis.
- `TimeTransformLogEntry` → `TimeTransformEntry` — earlier sprint draft typo; correct name in [`auki-time-transforms`](../auki-time-transforms) source is `TimeTransformEntry`.
- `AudioLogEntry` migration step **added** — was missing from the earlier sprint draft.

**Five per-type slop questions added to [`parking_lot.md`](parking_lot.md)** (resolve at the matching migration step, not upfront): PinholeCameraLogEntry intrinsics placement (inline vs sub-stream vs registry-versioned); PointCloudLogEntry on-disk-vs-wire drift (typed-fields-outside-bytes vs raw-bytes-only); AudioLogEntry implicit-vs-explicit chunk metadata; TimeTransformEntry — move `source` to manifest, drop computed `discontinuous`; TimeTransformSource — collapse the single-variant enum.

**No code changes.** Cargo.toml `name` updated, all in-crate doc references updated, workspace `Cargo.toml` member entry retargeted, [`auki-session-py`](../auki-session-py) cross-references updated, root [`parking_lot.md`](../../parking_lot.md) subfolder summary updated. `cargo test -p auki-datatypes` 1 passing (placeholder pipeline-check round-trip, unchanged). The 19:30 entry below describes the original scaffold under the old name; preserved verbatim per append-only.

### broodsugar's claude · May 7, 19:30 HKT, 2026

**Crate scaffolding.** New crate `auki-proto` — single source of truth for the SDK's protobuf schemas. Owns the `.proto` definitions and the prost-generated Rust code; downstream Rust crates (`auki-registry`, `auki-logs`, `auki-network`, `auki-time-transforms`, `auki-ros-adapter`) will import the generated types from here once the migration starts. Cross-language consumers (Python via `betterproto` from [`auki-session-py`](../auki-session-py/), future Sentinel ports) generate their own bindings from the same `.proto` files.

**Why this exists.** Resolves the [`auki-session-py` `payload: bytes` encoding contract](../auki-session-py/parking_lot.md) — segment payloads on disk become protobuf-encoded; manifests + registry entries + signing payloads continue to use JCS-canonical JSON via [`auki-jcs`](../auki-jcs). Two encodings, each doing what they're good at, no overlap on the wire.

**Sub-decisions locked 2026-05-07:** `.proto` files live in a dedicated `auki-proto` crate (vs per-crate `proto/` directories or repo-root `/proto/`) — single source of truth, mirrors how `auki-hash` / `auki-jcs` work as cross-cutting primitives. `prost` for Rust codegen (libp2p-ecosystem default; clean idiomatic structs). `betterproto` for Python (lands in `auki-session-py` when first impl starts; produces dataclass-shaped output that matches the booster-claude sketch).

**Build pipeline self-contained.** `protoc` binary supplied by `protoc-bin-vendored` build-dep — no system `protoc` install needed on dev machines or CI. `build.rs` compiles every `.proto` under `proto/` into Rust under `OUT_DIR`.

**Scaffold contents.** `proto/placeholder.proto` (single empty message — validates the build pipeline end-to-end; will be removed once the first real `.proto` lands), `build.rs` (prost-build invocation), `src/lib.rs` (re-exports the placeholder module), `src/readme.md` (status), `src/sprint.md` (six-step migration plan starting with `SensorLogEntry`), [`README.md`](README.md), [`parking_lot.md`](parking_lot.md). One inline test: `placeholder_pipeline_check_round_trips` verifies encode + decode work.

**Test count: 1.** `cargo test -p auki-proto` passes. `cargo check -p auki-proto` clean.

**Migration sequence in [`src/sprint.md`](src/sprint.md):** (1) `auki.sensor_log` — `SensorLogEntry`; (2) `auki.frame` — `JpegFrame` and `auki.pointcloud` — `PointCloudFrame`; (3) `auki.pose_log` — `PoseLogEntry` + `TransformSample`; (4) `auki.time_transform` — `TimeTransformLogEntry`; (5) remove placeholder; (6) Python codegen for `auki-session-py`. Each step is its own PR with locked conformance vectors. Will land in v0.0.24.

**Four open questions in [`parking_lot.md`](parking_lot.md):** package naming convention; field number allocation strategy; locked conformance vector format (Rust struct literal / JSON / both); schema versioning policy. None gating the scaffold; all need to land before the first real `.proto`.
