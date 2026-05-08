# Sprint — auki-datatypes

Current work and the migration sequence to bring real schemas into this crate. Spec: [outer `README.md`](../README.md).

## Now

Scaffolding only — `proto/placeholder.proto` validates the `prost-build` pipeline; no real schemas defined; no downstream consumers wired up. Six log payload types currently live (as drift) in [`auki-registry`](../../auki-registry); each migration step **moves** a type from there to here, not just generates a new one in here.

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

2. **`auki.frame_stream` — `JpegFrame`** + **`auki.point_cloud_stream` — `PointCloudFrame`** (libp2p wire types).
   - Define both `.proto` files. Each is a single `bytes` field at heart.
   - Update [`auki-network`](../../auki-network)'s `stream_protocol` to use the generated types — the wire stream framing (length-prefix + envelope) stays; the `T` payload changes to protobuf bytes.
   - Drop the `#[serde(with = "base64_bytes")]` adapter on `PointCloudFrame` — protobuf handles binary natively, no JSON-array-of-integers tax to dodge. Drops a dep on `base64`.
   - Update the locked cross-language conformance vector for `PointCloudFrame` wire shape.

3. **`auki.point_cloud` — `PointCloudLogEntry`** (on-disk).
   - Define `proto/point_cloud.proto`. Resolve the on-disk-vs-wire drift slop point in [`parking_lot.md`](../parking_lot.md) — typed layout fields (`width`, `height`, `is_dense`) outside the bytes vs raw-bytes-only with layout inside CDR.
   - **Move** `PointCloudLogEntry` out of [`auki-registry`](../../auki-registry).
   - Update [`auki-logs`](../../auki-logs) segment writer/reader.
   - Locked vector.

4. **`auki.audio` — `AudioLogEntry`**.
   - Define `proto/audio.proto`. Resolve the implicit-vs-explicit chunk metadata slop point — add `sample_count: u32` (or `chunk_duration_ns: i64`)?
   - **Move** `AudioLogEntry` out of [`auki-registry`](../../auki-registry).
   - Update [`auki-logs`](../../auki-logs).
   - Locked vector.

5. **`auki.pose` — `SpatialTransform`** (was `TransformSample`; `PoseLogEntry` wrapper goes away).
   - Define `proto/pose.proto`: `SpatialTransform { Vec3 translation = 1; Quat orientation = 2; }` plus `Vec3` and `Quat`. Flat — no `PoseLogEntry` wrapper. From/to live in the manifest, not the entry. Synthesis decided 2026-05-07 — see the corresponding Propagate task in the [root parking lot](../../../parking_lot.md).
   - **Move** `TransformSample` (renamed `SpatialTransform`) out of [`auki-registry`](../../auki-registry); drop `PoseLogEntry`. Rewrite `build_pose_log_manifest` in `auki-registry` (or move it) for the new (from, to)-keyed identity.
   - Update [`auki-session`](../../auki-session) `poselog_path` signature: `(session_root, from_frame_id, to_frame_id) -> PathBuf`, mirroring `timetransform_log_path`.
   - Update [`auki-logs`](../../auki-logs) segment writer/reader.
   - Locked vector.

6. **`auki.time_transform` — `TimeTransformEntry`** (was misnamed `TimeTransformLogEntry` in earlier sprint drafts; correct type name is `TimeTransformEntry`).
   - Define `proto/time_transform.proto`. Resolve the slop points in [`parking_lot.md`](../parking_lot.md): move `source` to manifest; drop `discontinuous` (computed at read time); collapse or relocate `TimeTransformSource` enum.
   - **Move** `TimeTransformEntry` and `TimeTransformSource` out of [`auki-time-transforms`](../../auki-time-transforms)'s data-type role (the sampler logic stays).
   - Update its sampler integration test.
   - Locked vector.

7. **Remove placeholder.** Once at least one real `.proto` exists and is consumed downstream, delete `proto/placeholder.proto`, the `placeholder` module in `lib.rs`, and the smoke test.

8. **Python codegen.** Lands in [`auki-session-py`](../../auki-session-py) when its first implementation starts. `betterproto` generator over the same `.proto` files; locked-vector cross-language test that the Python encoder produces byte-identical bytes to the Rust prost encoder for the same input.

## After the migration

[`auki-registry`](../../auki-registry)'s scope shrinks back to its **canonical** definition per the [Notion Registries doc](https://www.notion.so/34e5c8e96592809d8977feb17c32e5d0): identity + definitions only (Sensor, Frame, Clock entries). All log payload types live here; consumers add an `auki-datatypes` dep alongside their existing `auki-registry` dep.

Manifests stay in `auki-registry` (or possibly migrate to `auki-session` — open) as JCS-canonical JSON via [`auki-jcs`](../../auki-jcs). This crate doesn't touch manifests; it owns segment payloads only.

## Out-of-band

- Manifests, registry entries, signing payloads stay JCS-canonical JSON via [`auki-jcs`](../../auki-jcs). This crate is for segment payloads only.
- libp2p control protocols (`/auki/control/...`) — separate question, in [`auki-session-py/parking_lot.md`](../../auki-session-py/parking_lot.md). When that work starts, those control-message `.proto` files live here too.
