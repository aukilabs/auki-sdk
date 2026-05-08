# Sprint — auki-datatypes

Current work and the migration sequence to bring real schemas into this crate. Spec: [outer `README.md`](../README.md).

## Now

Three real schemas land via [sawslin Phase 1 Lane 0 PR B](https://www.notion.so/3585c8e9659280dd9093c703d88e1530) — `auki.pose` (`SpatialTransform` + `Vec3` + `Quat`), `auki.joint_state` (`JointAngles`), and `auki.pose_stream` (`PoseStreamFrame` `oneof` envelope). Each ships with locked cross-language conformance vectors. Placeholder is gone.

The remaining migration steps below still hold — sawslin only pulled three packages forward. Five existing log payload types still live (as drift) in [`auki-registry`](../../auki-registry); each remaining migration step **moves** a type from there to here, not just generates a new one in here.

## Sawslin queue-jump (landed in PR B)

Sawslin Lane 0 needed the canonical pose payload now (sentinel's per-marker pose stream from Phase 3+ uses the same `SpatialTransform` shape the Pose Log redesign locked). Rather than ship a temporary `TransformFrame` and rename it to `SpatialTransform` later, PR B pulls Step 5's pose schema forward and adds two sibling schemas:

- **`auki.pose`** (originally Step 5) — `SpatialTransform { Vec3 translation, Quat orientation }` + `Vec3` + `Quat`. Defined here as protobuf with locked conformance vectors. **Additive only:** the existing [`auki-registry::TransformSample`](../../auki-registry) and `PoseLogEntry` wrapper are still there, untouched. Step 5's full migration (move out of `auki-registry`, drop the `PoseLogEntry` wrapper, rewrite `build_pose_log_manifest` for the new `(from, to)`-keyed identity, update `auki-session::poselog_path`, update `auki-logs` segment writer/reader) stays as planned but becomes a **delete-the-duplicate** pass since `SpatialTransform` already exists here.
- **`auki.joint_state`** (new — not previously in the migration sequence) — `JointAngles { repeated float angles = 1; }`. Used both on the wire (boosterapp's PoseStream from sawslin Phase 1) and on disk (Sensor Log payload for `SensorBody::JointState` registry entries from PR A). Same shape both places per [sawslin's joint-state-is-a-Sensor-Log decision](https://www.notion.so/3585c8e9659280dd9093c703d88e1530#L12).
- **`auki.pose_stream`** (new) — `PoseStreamFrame { oneof payload { JointAngles joint_angles; SpatialTransform spatial_transform; } }`. The wire envelope flowing over sawslin's `AcceptPoseStream` substream, per [locked decision #7](https://www.notion.so/3585c8e9659280dd9093c703d88e1530#L7) ("same wire variant carries both shapes via the typed payload"). [`auki-network`](../../auki-network) wraps prost-encoded bytes inside the existing length-prefixed JSON framing via a `PoseStreamFrameWire` adapter — same trick `PointCloudFrame` uses today; the adapter goes away when migration step 2 lifts the framing layer to native prost binary across all `T`s.

## Migration sequence

Each step is its own PR with its own locked conformance vector. Each step also resolves the matching per-type slop question in [`parking_lot.md`](../parking_lot.md) (decisions Nils adjudicates per-step rather than upfront).

0. **Prep: extract `auki-manifests` crate.** New crate that holds the SDK's manifest contract — the `build_*_log_manifest` builders, the manifest read-side parsers + validators, and the manifest-shape schemas (currently documented in [`auki-registry/README.md`](../../auki-registry/README.md)). Symmetric with `auki-datatypes`: that crate owns segment payload shapes; this one owns manifest shapes. See the corresponding decision in [`parking_lot.md`](../parking_lot.md).
   - **Move** `build_sensor_log_manifest` and `build_pose_log_manifest` from [`auki-registry`](../../auki-registry).
   - **Move** `build_manifest` from [`auki-time-transforms`](../../auki-time-transforms).
   - **Move** the manifest-table sections of [`auki-registry/README.md`](../../auki-registry/README.md) into the new crate's `README.md`.
   - Pure refactor — no behaviour change, no encoding change. Manifest encoding stays JCS-canonical UTF-8 JSON via [`auki-jcs`](../../auki-jcs); see decision in [`parking_lot.md`](../parking_lot.md).
   - Add the new crate's per-folder `README.md` / `parking_lot.md` / `changelog.md` / `src/readme.md` / `src/sprint.md` per the [folder convention](../../../CONTRIBUTING.md).
   - Lands **before** step 1 so step 1 can stay focused on the segment-encoder swap.

1. **`auki.camera` — `PinholeCameraLogEntry`** (renamed from `SensorLogEntry`).
   - Define `proto/camera.proto`. Message shape: `dynamic_intrinsics` placement is a per-step decision (see slop note in [`parking_lot.md`](../parking_lot.md)).
   - Add locked conformance vector pinning a fixed `PinholeCameraLogEntry` instance to its protobuf wire bytes.
   - **Move** `PinholeCameraLogEntry` (née `SensorLogEntry`) and `DynamicIntrinsics` out of [`auki-registry`](../../auki-registry); update its source + tests + README.
   - Update [`auki-logs`](../../auki-logs) segment writer/reader to use protobuf-encoded bytes for the typed `T` (segments stay length-prefixed; the payload bytes change from CBOR-via-ciborium to prost-encoded).
   - Test the segment round-trip with the new encoding.

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
   - **Schema landed via sawslin PR B** (queue-jumped — see "Sawslin queue-jump" above). `proto/pose.proto` exists with `SpatialTransform { Vec3 translation = 1; Quat orientation = 2; }`, `Vec3`, `Quat`, and locked conformance vectors. Flat — no `PoseLogEntry` wrapper.
   - **Still pending:** **move** `TransformSample` (now duplicate) and `PoseLogEntry` (gone) out of [`auki-registry`](../../auki-registry); rewrite `build_pose_log_manifest` in `auki-registry` (or move it) for the new (from, to)-keyed identity.
   - **Still pending:** update [`auki-session`](../../auki-session) `poselog_path` signature: `(session_root, from_frame_id, to_frame_id) -> PathBuf`, mirroring `timetransform_log_path`.
   - **Still pending:** update [`auki-logs`](../../auki-logs) segment writer/reader.

6. **`auki.time_transform` — `TimeTransformEntry`** (was misnamed `TimeTransformLogEntry` in earlier sprint drafts; correct type name is `TimeTransformEntry`).
   - Define `proto/time_transform.proto`. Resolve the slop points in [`parking_lot.md`](../parking_lot.md): move `source` to manifest; drop `discontinuous` (computed at read time); collapse or relocate `TimeTransformSource` enum.
   - **Move** `TimeTransformEntry` and `TimeTransformSource` out of [`auki-time-transforms`](../../auki-time-transforms)'s data-type role (the sampler logic stays).
   - Update its sampler integration test.
   - Locked vector.

7. ~~**Remove placeholder.**~~ **Done in sawslin PR B** alongside the queue-jumped pose / joint_state / pose_stream packages.

8. **Python codegen.** Lands in [`auki-session-py`](../../auki-session-py) when its first implementation starts. `betterproto` generator over the same `.proto` files; locked-vector cross-language test that the Python encoder produces byte-identical bytes to the Rust prost encoder for the same input.

## After the migration

[`auki-registry`](../../auki-registry)'s scope shrinks back to its **canonical** definition per the [Notion Registries doc](https://www.notion.so/34e5c8e96592809d8977feb17c32e5d0): identity + definitions only (Sensor, Frame, Clock entries). All log payload types live here; consumers add an `auki-datatypes` dep alongside their existing `auki-registry` dep.

Manifests stay in `auki-registry` (or possibly migrate to `auki-session` — open) as JCS-canonical JSON via [`auki-jcs`](../../auki-jcs). This crate doesn't touch manifests; it owns segment payloads only.

## Out-of-band

- Manifests, registry entries, signing payloads stay JCS-canonical JSON via [`auki-jcs`](../../auki-jcs). This crate is for segment payloads only.
- libp2p control protocols (`/auki/control/...`) — separate question, in [`auki-session-py/parking_lot.md`](../../auki-session-py/parking_lot.md). When that work starts, those control-message `.proto` files live here too.
