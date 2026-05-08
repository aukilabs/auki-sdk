# Changelog — auki-datatypes

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

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
