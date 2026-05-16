# Sprint — auki-datatypes

Current work and the migration sequence to bring real schemas into this crate. Spec: [outer `README.md`](../README.md).

## Now

**2026-05-08 migration complete (Step 7); Step 8 added the same day.** Every on-disk log payload type lives here. Steps 1–7 ran the migration that moved pre-existing types from [`auki-registry`](../../auki-registry) and [`auki-time-transforms`](../../auki-time-transforms) into this crate; Step 8 added a new type (`DetectionLogEntry`) that closes the producer side of the [subscription-as-materialization keystone](../../../parking_lot.md) and unblocks [`detectors`](https://github.com/aukilabs/detectors) phase 2's blocker #3. The crate is the single source of truth for cross-language segment payload shapes; consumer crates (auki-registry, auki-time-transforms, auki-network, auki-ros-adapter, auki-network-py) all reference the prost-generated types from here.

Pre-migration history (kept for context): six log payload types lived as drift in [`auki-registry`](../../auki-registry) and [`auki-time-transforms`](../../auki-time-transforms); each migration step **moved** a type from there to here, not just generating a new one. The sequence below is preserved as a record of what was moved when.

## Migration sequence

Each step is its own PR with its own locked conformance vector. Each step also resolves the matching per-type slop question in [`parking_lot.md`](../parking_lot.md) (decisions Nils adjudicates per-step rather than upfront).

0. **✓ Prep: extract `auki-manifests` crate** (landed 2026-05-08). Pure refactor; no behaviour change, no encoding change.
   - **Moved** `build_sensor_log_manifest` and `build_pose_log_manifest` from [`auki-registry`](../../auki-registry) → [`auki-manifests`](../../auki-manifests).
   - **Moved** `build_manifest` from [`auki-time-transforms`](../../auki-time-transforms) → [`auki-manifests`](../../auki-manifests) (renamed `build_time_transform_log_manifest` for unambiguity vs siblings).
   - **Moved** `PoseSource` (inline pose-log producer identity) from [`auki-registry`](../../auki-registry) → [`auki-manifests`](../../auki-manifests) — it's manifest metadata, not a registry entry.
   - **Moved** locked vectors `ros2_tf_source_serializes_to_canonical_bytes` + `ros2_tf_source_hash_is_locked` (M1 example → JCS bytes + `f3d296341347589c72297a0cc7c81cd8`).
   - Manifest encoding stays JCS-canonical UTF-8 JSON via [`auki-jcs`](../../auki-jcs).
   - Per-folder docs seeded; workspace `Cargo.toml` updated; `auki-time-transforms` gains an `auki-manifests` dev-dep so the `Sampler` integration test still constructs a manifest.
   - `cargo test -p auki-manifests` 6/6 passing; downstream tests pass workspace-wide (`auki-registry` 41 → 35 since 6 tests moved, `auki-time-transforms` 10 → 9 since 1 test moved).

1. **✓ `auki.camera` — `PinholeCameraLogEntry`** (renamed from `SensorLogEntry`; landed 2026-05-08).
   - `proto/camera.proto` defines `PinholeCameraLogEntry { DynamicIntrinsics dynamic_intrinsics = 1; bytes frame = 2; }` + `DynamicIntrinsics { double fx, fy, cx, cy = 1..4; repeated double distortion_coefficients = 5; }`. Per-step decision: `dynamic_intrinsics` is **inline-optional** (proto3 message-typed fields are `Option<T>` in prost) — non-autofocusing cameras pay only the message-tag overhead; autofocusing cameras populate per-frame. Promoting to a sibling intrinsics-update sub-stream remains possible without breaking on-disk readers.
   - Locked conformance vectors pin both wire bytes and XXH3-128 hash (`0496e1f71a03e00877fc68bf16190026`) for the M1 example.
   - **Moved** `PinholeCameraLogEntry` (née `SensorLogEntry`) and `DynamicIntrinsics` out of [`auki-registry`](../../auki-registry). [`auki-ros-adapter`](../../auki-ros-adapter)'s `build_sensor_log_entry` now produces the prost type; `dynamic_intrinsics` callers handle the `Option<...>` (`.as_ref().unwrap()`).
   - [`auki-logs`](../../auki-logs) became encoding-agnostic via a new `LogPayload` trait — consumers pick prost / ciborium / their own. Per-step decision adopting the parking-lot lean. The `impl_log_payload!` macro in this crate gives every prost type a one-line impl. Mid-migration ciborium types (TimeTransformEntry) write their `LogPayload` impl directly. `auki-logs` drops ciborium from production deps; `Error::Cbor` → `Error::Payload`.
   - End-to-end seam test: `auki_logs::Log<PinholeCameraLogEntry>` round-trip with both intrinsics-present and intrinsics-absent entries.

2. **✓ `auki.frame_stream` — `JpegFrame`** + **`auki.point_cloud_stream` — `PointCloudFrame`** + **`auki.stream` envelope** (libp2p wire types; landed 2026-05-08).
   - Three `.proto` packages: `auki.frame_stream { JpegFrame }`, `auki.point_cloud_stream { PointCloudFrame }`, and `auki.stream` (the full envelope — `StreamMessage` oneof of `StreamRequest | StreamDescriptor | DeclineReason | Frame | EndReason`).
   - Per-step decision: `Frame.payload = bytes` (T inferred from `StreamDescriptor.sensor_hash` → `SensorRegistryEntry.body`). The substream is mono-T per Dagaz D1; the variant tag would be redundant on every frame.
   - Wire format: 4-byte BE u32 length prefix + prost-encoded `StreamMessage`. The envelope itself moves to protobuf (not just inner T) so we get binary natively — drops the `#[serde(with = "base64_bytes")]` adapter and the `base64` dep on the swarm path. Old JSON-on-wire `/auki/stream/1.0.0` is retired in this PR; new protocol is `/auki/stream/0.1.0`.
   - Workspace-wide protocol-id rename: `/auki/cluster/1.0.0` → `/auki/cluster/0.0.1`, `/auki/identify/1.0.0` → `/auki/identify/0.0.1`. Resolves the "save 1.0.0 for the first official release" stance.
   - [`auki-network`](../../auki-network)'s `stream_protocol` re-exports the prost types from this crate; `stream_runtime`'s `T` bound switches from `Serialize + DeserializeOwned` to `prost::Message + Default`. [`auki-network-py`](../../auki-network-py)'s PyO3 wrappers updated for the prost match patterns.
   - Locked cross-language conformance vectors for `JpegFrame` + `PointCloudFrame` wire bytes pinned in `auki-network::stream_protocol::tests`.

3. **✓ `auki.point_cloud` — `PointCloudLogEntry`** (on-disk; landed 2026-05-08).
   - `proto/point_cloud.proto` defines `PointCloudLogEntry { bytes data = 1; }`. Per-step decision: **opaque-bytes-only** (Option A in the parking-lot slop point) — symmetric with the wire's `PointCloudFrame { bytes }`, doesn't bake ROS `PointCloud2`'s `width × height × is_dense` shape into the SDK type. Layout interpretation comes from `(sensor_id, sensor_hash) → SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id }`. Resolves the on-disk-vs-wire drift in [`parking_lot.md`](../parking_lot.md).
   - **Moved** `PointCloudLogEntry` out of [`auki-registry`](../../auki-registry). [`auki-ros-adapter`](../../auki-ros-adapter)'s `build_point_cloud_log_entry` now returns the prost type with just `data` set; `width` / `height` / `is_dense` are no longer carried per-frame (`apply_normalization` still uses ROS-side `msg.width × msg.height` to compute `num_points` for the layout repacking, then flattens into the bytes).
   - [`auki-logs`](../../auki-logs) needed no changes — encoder-agnostic since Step 1.
   - Locked conformance vectors pin both wire bytes (`0a18000102030405060708090a0b0c0d0e0f1011121314151617` for a 24-byte fixture) and XXH3-128 hash (`4ea525d849212b2e067e33bec455c7ea`).

4. **✓ `auki.audio` — `AudioLogEntry`** (on-disk; landed 2026-05-08).
   - `proto/audio.proto` defines `AudioLogEntry { bytes data = 1; }`. Per-step decision: **opaque-bytes-only** (Option A in the parking-lot slop point) — same stance as Step 3 for point clouds, declining the pre-Step-3 lean toward adding `sample_count`. Sample count derivable as `data.len() / (sample_byte_width × channels)`; chunk duration derivable as `sample_count × 1e9 / sample_rate_hz`. Reader needs the registry to interpret bytes anyway, so denormalizing either field would risk inconsistency for marginal convenience.
   - **Moved** `AudioLogEntry` out of [`auki-registry`](../../auki-registry); no downstream consumers (no `auki-ros-adapter` builder for audio yet).
   - [`auki-logs`](../../auki-logs) needed no changes — encoder-agnostic since Step 1.
   - **Dropped** the `serde_bytes` dep from [`auki-registry`](../../auki-registry) — `AudioLogEntry` was its last user.
   - Locked conformance vectors pin both wire bytes (`0a1000112233445566778899aabbccddeeff` for a 16-byte `pcm_s16le` stereo fixture) and XXH3-128 hash (`a5864ae7018f28a5c094a714af1db62e`).

5. **✓ `auki.pose` — `SpatialTransform`** (on-disk; landed 2026-05-08).
   - `proto/pose.proto` defines `SpatialTransform { Vec3 translation = 1; Quat orientation = 2; }` + `Vec3 { double x, y, z }` + `Quat { double x, y, z, w }`. Flat — the pre-migration `PoseLogEntry { transforms: Vec<TransformSample> }` wrapper is gone, and per-sample `parent_frame` / `child_frame` strings (which existed on `TransformSample`) are gone too. Per the synthesis decided 2026-05-07.
   - **Moved** `TransformSample` (renamed `SpatialTransform`) and dropped `PoseLogEntry` from [`auki-registry`](../../auki-registry); the crate's `ciborium` dev-dep dropped at the same time (pose types were its last user).
   - **Rewrote `build_pose_log_manifest`** in [`auki-manifests`](../../auki-manifests) for the new identity: 13 args, including `from_frame_id` + `from_frame_hash`, `to_frame_id` + `to_frame_hash` (mirrors `build_time_transform_log_manifest`'s clock-pair pattern), `writer_mode: PoseWriterMode` (`Rigid` or `Movable`), `expected_rate_hz: u32`. Resolves the manifest-reshape parking-lot item.
   - **Updated `poselog_path`** in [`auki-layout`](../../auki-layout) to `(session_root, from_frame_id, to_frame_id) -> PathBuf`, mirroring `timetransform_log_path`. The on-disk segment is `<session>/poselogs/<from_id>__<to_id>` (each frame_id's `/` substituted to `__`).
   - [`auki-logs`](../../auki-logs) needed no changes — encoder-agnostic since Step 1.
   - Producer guidance: a multi-pair ROS `TFMessage` fans into N parallel pose logs (one per `(from, to)` pair). Each log sees one timestamped sample per source message for its pair.
   - Locked conformance vectors pin both wire bytes (`0a1b09000000000000f03f110000000000000040190000000000000840120921000000000000f03f` for an identity-rotation 1-2-3 translation fixture) and XXH3-128 hash (`29fa6349ab0b3ff1f06933489db74dfd`).

6. **✓ `auki.time_transform` — `TimeTransformEntry`** (on-disk; landed 2026-05-08).
   - `proto/time_transform.proto` defines `TimeTransformEntry { int64 offset_ns = 1; uint32 uncertainty_ns = 2; }`. Per-step decisions resolved all three slop points: (a) `source` moved to manifest as a tagged-enum `TimeTransformSource` (mirrors `PoseSource`); (b) `discontinuous: bool` dropped — readers compute `|offset_ns - prev_offset_ns| ≥ reader_threshold` against their own tolerance; (c) `TimeTransformSource` kept as tagged enum at the manifest layer (Option 2 — matches `PoseSource`'s extension pattern with one variant today, `LocalClockRead`).
   - **Moved** `TimeTransformEntry` out of [`auki-time-transforms`](../../auki-time-transforms) into this crate. **Moved** `TimeTransformSource` out of [`auki-time-transforms`](../../auki-time-transforms) into [`auki-manifests`](../../auki-manifests) (it's manifest metadata, not a per-entry field — same role as `PoseSource`).
   - **Rewrote `build_time_transform_log_manifest`** in [`auki-manifests`](../../auki-manifests) to take `&TimeTransformSource`; the manifest gains a `"source"` field mirroring Pose Log's shape.
   - **Simplified `tick()` and `Sampler::start`** in [`auki-time-transforms`](../../auki-time-transforms): no more `SamplerState`, no more `discontinuity_threshold` arg. The sampler is now a pure `clock → entry` pipeline; discontinuity detection is the reader's responsibility.
   - [`auki-logs`](../../auki-logs) needed no changes — encoder-agnostic since Step 1.
   - **Dropped** `ciborium` + `serde` + `serde_json` deps from [`auki-time-transforms`](../../auki-time-transforms) — encoding is now prost in the new home, and the sampler is a thin wrapper that doesn't need them. Picked up `auki-datatypes` (for the prost type re-export) and `auki-manifests` (for `TimeTransformSource`).
   - Locked conformance vectors pin both wire bytes (`08c0843d10fa01` for `offset_ns: 1_000_000, uncertainty_ns: 250`) and XXH3-128 hash (`b7e73628833419a7c299933d07cbe88c`); plus a JCS-canonical-bytes + hash vector for `TimeTransformSource::LocalClockRead` (`8dcea0b9b0b2219d651e0856f112cd65`).

7. **✓ Remove placeholder** (landed 2026-05-08). Deleted `proto/placeholder.proto`, the `placeholder` module in `lib.rs`, the `placeholder_pipeline_check_round_trips` smoke test, and the corresponding line in `build.rs`. The seven real `.proto` packages serve as proof that the prost-build pipeline works; the placeholder no longer earned its keep. Test count: 32 → 31.

8. **✓ `auki.detection` — `DetectionLogEntry`** (on-disk; landed 2026-05-08).
   - `proto/detection.proto` defines `DetectionLogEntry { bytes data = 1; }`. Per-step decision: **opaque-bytes-only** — same stance as Steps 3 (point cloud) and 4 (audio). The detection schema is per-Detector (QR portal-uid + four corners + content; ESL class + bbox + confidence; people bboxes); the SDK does not interpret detector-specific fields. Carrying detector-specific fields on this prost type would either lock the SDK into knowing every detector's schema or force a degenerate `oneof` of every shipped detector — neither scales.
   - **New type, not a migration.** Unlike Steps 1–6, no source type existed to move; Step 8 adds `DetectionLogEntry` as the producer-side closure of the [Detector keystone](../../../parking_lot.md) filed by Dobby earlier today. A Detection Log is `Log<T>` with `T = DetectionLogEntry`, lifecycle inherited from the sensor-log primitive. No "DetectionLog" abstraction.
   - [`auki-logs`](../../auki-logs) needed no changes — encoder-agnostic since Step 1.
   - **Deferred for separate PRs (the rest of [`detectors`](https://github.com/aukilabs/detectors) phase 2's blockers):** `Log<T>::tail()` (the read side of the keystone, in `auki-logs`); the Detector binding API (`Detector::new(sensor_log) -> Log<DetectionLogEntry>` write-handle, location TBD between `auki-logs` and a new home); the [`auki-sdk-py`](../../auki-sdk-py) Python binding for the ESL detector. Each is its own single-PR landing; this Step 8 is the smallest piece that sits cleanly in this crate.
   - **Detection-Log registry shape — out of this crate's scope.** The Detection-Log analog of `SensorRegistryEntry` (the registry entry that pins per-`(detector_id, ...)` interpretation of the opaque bytes) is a forthcoming sibling shape that lives in [`auki-registry`](../../auki-registry), not here. File when subscription/discovery for detection logs needs it.
   - Locked conformance vectors pin both wire bytes (`0a0c000102030405060708090a0b` for a 12-byte fixture) and XXH3-128 hash (`94f8efe6be63d3dc5e045ab08d538a15`).

9. **✓ Python codegen** (landed 2026-05-09). New crate [`auki-datatypes-py`](../../auki-datatypes-py) — pure-Python (no Rust, no maturin) `betterproto`-generated dataclass-shaped bindings, one submodule per `.proto` package. **Cross-language byte equality verified**: every locked vector from `*_serializes_to_locked_wire_bytes` in [`src/lib.rs`](lib.rs) has a matching Python test that pins the same hex bytes — 10/10 tests passing. `betterproto` pinned to `1.2.5`; codegen runs via [`regen.sh`](../../auki-datatypes-py/regen.sh) when `.proto` files change. **Earlier framing — that this would land in `auki-session-py`** — superseded by the per-component naming decision (root parking-lot, 2026-05-06); `auki-datatypes-py` is its own crate.

## After the migration

[`auki-registry`](../../auki-registry)'s scope shrinks back to its **canonical** definition per the [Notion Registries doc](https://www.notion.so/34e5c8e96592809d8977feb17c32e5d0): identity + definitions only (Sensor, Frame, Clock entries). All log payload types live here; consumers add an `auki-datatypes` dep alongside their existing `auki-registry` dep.

Manifests stay in `auki-registry` (or possibly migrate to `auki-layout` — open) as JCS-canonical JSON via [`auki-jcs`](../../auki-jcs). This crate doesn't touch manifests; it owns segment payloads only.

## Out-of-band

- Manifests, registry entries, signing payloads stay JCS-canonical JSON via [`auki-jcs`](../../auki-jcs). This crate is for segment payloads only.
- libp2p control protocols (`/auki/control/...`) — separate question, in [`auki-session-py/parking_lot.md`](../../auki-session-py/parking_lot.md). When that work starts, those control-message `.proto` files live here too.
