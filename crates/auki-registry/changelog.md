# Changelog — auki-registry

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

**Camera sensor registry bodies now use `Camera` and the `"camera"` tag.** `SensorBody::Camera(Camera)` replaces the legacy RGB-camera body vocabulary, and the locked canonical JSON/hash fixtures intentionally pin the new registry tag.

### Arshak's claude · May 16, HKT, 2026

**Detector Registry lands — `DetectorRegistryEntry { detector_id, body, output_types }`.** Closes Cuba **T4** + **T16** together. Mirrors `SensorRegistryEntry`: stable `detector_id` + typed `body` (`DetectorBody::{Aruco, Qr, Esl}`) + content-addressed hash via `canonicalize` → `auki_hash::hash_jcs_bytes`. The new `output_types: Vec<String>` field (T16) is the capability-discovery axis — *advertise what you detect, not which implementation you're running* — and its values must match `DetectionLogEntry.type` (Cuba T12) for the entries the detector emits.

**`DetectorBody` is closed**, not open-string: the three concrete bodies cover the Cuba demo set (Aruco's `dictionary`, Qr's empty body, Esl's empty body). No `Other` escape hatch by design — premature variants become permanent. Adding a fourth detector is one tagged-enum variant + a struct + the on-disk migration of any existing entries.

**`write_detector` / `read_detector` mirror `write_sensor` / `read_sensor`** at `<app_root>/registries/detectors/<detector_id>/<hash>.json`. Idempotent on hash; `Error::IdMismatch` on a misplaced or tampered file. No cross-reference validation (no frame_hash equivalent — a detector doesn't pin a frame, the detection log it writes pins its input sensor's hash via the Cuba T5 `sensor_hash` field on `DetectionLogEntry`).

**Locked canonical bytes for the canonical Cuba ArUco entry**:

```
{"detector_id":"aukilabs/aruco/v1","dictionary":"5x5_50","output_types":["aruco"],"type":"aruco"}
```

Key order is RFC 8785 §3.2.3 lexicographic. The new test `detector_entry_canonical_bytes_lock_the_aruco_shape` is the cross-language canary.

**Tests**: 38 → 44 (+6 — `detector_entry_canonical_bytes_lock_the_aruco_shape`, `_round_trips_through_disk`, `_write_is_idempotent_on_hash`, `_two_dictionaries_get_distinct_hashes`, `_slash_in_id_becomes_double_underscore`, `_supports_multiple_output_types`).

**Context**: Commit 3/6 of the Cuba v0.0.45 SDK migration. Detector Registry consumed by `RegistryKind::Detector` on `/auki/registries/0.0.1` (Commit 4) and by detector-aruco's `register_detector` builder.

### Nils's codex · May 16, 17:53 HKT, 2026

**Spatial sensor bodies now pin exact frame-entry versions.** `SensorBody::RgbCamera` and `SensorBody::PointCloud` gain required `frame_hash: String` alongside `frame_id`, so a sensor entry commits to a specific `FrameRegistryEntry` version instead of only naming a mutable frame directory. This is intentionally breaking for any existing on-disk spatial sensor entries without `frame_hash`; there is no compatibility shim, default, or lazy directory scan.

**`write_sensor` validates frame references before touching disk.** For `RgbCamera` / `PointCloud`, `frame_id` and `frame_hash` must both be non-empty and `<app_root>/registries/frames/<frame_id>/<frame_hash>.json` must already exist. Missing or empty references return `Error::FrameReferenceMissing { sensor_id, frame_id, frame_hash }`. `Audio` and `JointEncoders` remain non-spatial and carry no frame fields.

**Locked-vector recompute.** The M1 RGB camera canonical JSON now includes `"frame_hash":"e0d40e7b526e04f15f83f75897f53825"` and its sensor hash is **`69f37478490cf1c0b226dbb86d3454fc`**. The M1 point-cloud entry now includes the same optical-frame hash and its sensor hash is **`2c480838a9be0b14608a8a0d72ee319f`**. `auki-ros-adapter`'s locked builder tests track these same hashes.

**Tests:** `auki-registry` 36 → 38 (+2 — `write_sensor_rejects_missing_frame_reference`, `write_sensor_rejects_empty_frame_hash`), plus existing spatial write tests now write the referenced frame entry first.

### Nils's claude · May 14, 11:00 HKT, 2026

**`SensorBody::Microphone` renamed to `SensorBody::Audio`** — signal-type naming for consistency with `PointCloud` / `JointEncoders` (and the `SensorEntry.kind` open-string contract pinned 2026-05-14 in `auki-network`). Variant tag flips `"microphone"` → `"audio"`; the struct rename is total (`pub struct Microphone` → `pub struct Audio`); body fields unchanged (`sample_rate_hz`, `channels`, `sample_format`, `channel_layout`). All in-crate call sites and tests updated.

**This is the first exercise of the "coordinated `SensorBody` rename = wire break" path** that `auki-network::sensors_protocol`'s `SensorEntry.kind` doc-comment warns about. The wire-tag is `"audio"` everywhere now — on-disk `SensorRegistryEntry`s written before this rename will fail to deserialize against the renamed enum (`"type":"microphone"` no longer matches any variant). v1 demo land is fine (no persistent audio registry on K1s yet); future archaeologists with pre-rename logs can either replay through a one-off renamer or recognize that pre-2026-05-14 logs predate the Hagall principle ("wire formats may break").

**Locked-vector recompute.** `m1_audio_entry()` (was `m1_microphone_entry()`) — locked canonical JSON flips one field:

- Pre-rename: `{"channel_layout":"n_channel","channels":4,"sample_format":"pcm_s16le","sample_rate_hz":48000,"sensor_id":"K1-AABBCCDDEEFF/head_array_4mic","type":"microphone"}`
- Post-rename: `{"channel_layout":"n_channel","channels":4,"sample_format":"pcm_s16le","sample_rate_hz":48000,"sensor_id":"K1-AABBCCDDEEFF/head_array_4mic","type":"audio"}`

XXH3-128: `6e0a195364866f18834d2db8e2a0699f` → **`bc4a0e690f1149c4927ea98c96ead65a`**. Locked in `audio_entry_hash_is_locked` test. Any cross-language reader (Park's browser decoder, future Sentinel) needs the new hash.

**Doc propagation:** [`README.md`](README.md) type listing + Audio section; [`src/readme.md`](src/readme.md) renamed `### SensorBody::Microphone` to `### SensorBody::Audio`; [`parking_lot.md`](parking_lot.md) JointEncoders-minimalism entry's "`Microphone::channels`" analog → "`Audio::channels`"; sibling [`auki-datatypes`](../auki-datatypes) ([README](../auki-datatypes/README.md) Step 4 line, [src/readme.md](../auki-datatypes/src/readme.md) audio bullet, [src/lib.rs](../auki-datatypes/src/lib.rs) `auki.audio` module doc-comment, [proto/audio.proto](../auki-datatypes/proto/audio.proto) header, [proto/detection.proto](../auki-datatypes/proto/detection.proto) analog reference, [parking_lot.md](../auki-datatypes/parking_lot.md) three entries); sibling [`auki-network`](../auki-network) ([sensors_protocol.rs](../auki-network/src/sensors_protocol.rs) `SensorEntry.kind` four-tags list). Notion canonical doc ([Hagall main page](https://www.notion.so/35e5c8e9659280e69b86f5edc32641a0)'s **Sensor catalog shape** subsection + status log; [SDK plan](https://www.notion.so/35f5c8e9659281b3afa7e713bcc89a50) SDK-T12 surface) updated separately.

36 auki-registry tests pass; workspace-wide `cargo test --workspace --lib` clean.

### broodsugar's claude · May 9, 12:46 HKT, 2026

**`SensorBody::JointEncoders { joint_count, frame_rate_hz }` variant added.** Fourth sensor-body kind alongside `RgbCamera` / `PointCloud` / `Microphone`. Per-frame data lives in [`auki_datatypes::joint_encoders::JointEncodersLogEntry`](../auki-datatypes/src/lib.rs) (on-disk) and [`auki_datatypes::joint_encoders_stream::JointEncodersFrame`](../auki-datatypes/src/lib.rs) (libp2p stream wire) — same `repeated float angles_rad = 1` shape on both sides. Producer ships angle vectors; consumer (Park) holds the URDF and does FK. Joint angles are encoder readings — measurements before any kinematic interpretation; pose (cartesian TF) is what you compute via FK, downstream.

**Layering rationale (sensor-data, not pose).** Same shape as `Microphone` (PCM bytes + `channels`/`sample_format` for deserialization), `PointCloud` (CDR bytes + `point_step`/`fields` for deserialization), `RgbCamera` (pixel bytes + intrinsics for projection). Producer ships raw measurements with just enough deserialization metadata (`joint_count`) for the consumer to read them; schema-for-interpretation (URDF) lives downstream. This is the layering call Nils made — overriding an earlier reach for a `PoseSource::JointAngles` pose-log variant. The pose-log path forced a manifest-keying decision (`(from_frame_id, to_frame_id)` is required for `Ros2Tf` but conceptually wrong for joint-space readings) and conflated the measurement layer with the interpretation layer.

**Deliberately minimal.** No `joint_names: Vec<String>` (URDF lives on consumer; producer doesn't read URDF and shouldn't be authoritative for joint names — see [`parking_lot.md`](parking_lot.md#decided-2026-05-09--joint_names-placement-on-the-producer)). No `urdf_id` / `joint_name_hash` (speculative — Park is K1-monoculture today; revisit when ≥2 robot models share a Park instance). No `frame_id` (joint encoders aren't in any cartesian frame; including a `frame_id` would invite consumers to look up a Frame Registry entry that doesn't make sense for this sensor type).

**Tests**: 33 → 36 (+3 — `joint_encoders_entry_serializes_to_canonical_bytes`, `joint_encoders_entry_hash_is_locked` (`cb45b0d89bcb5c738c38ff9c3c9d7768`), `write_then_read_joint_encoders_round_trip`).

**Docs**: [`src/readme.md`](src/readme.md) gains a `SensorBody::JointEncoders` section pointing at the paired log/stream payloads in `auki-datatypes`.

### broodsugar's claude · May 8, 11:52 HKT, 2026

**`PoseLogEntry` + `TransformSample` departed at Step 5 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md).** Both structs are gone from [`src/lib.rs`](src/lib.rs); replaced by the flat [`auki_datatypes::pose::SpatialTransform`](../auki-datatypes/src/lib.rs) with `(from_frame_id, to_frame_id)` identity in the manifest (per the 2026-05-07 synthesis). Per-sample `parent_frame` / `child_frame` strings are gone — each Pose Log holds samples for exactly one ordered frame pair, mirroring TimeTransform Log's per-clock-pair shape.

**`ciborium` dev-dep dropped** — pose tests were its last user (after Steps 3/4 dropped the audio + point-cloud CBOR tests).

**Tests**: 33 → 33 (`pose_log_entry_round_trips_through_cbor` and `pose_log_entry_with_empty_transforms_round_trips` removed; new round-trip tests for `SpatialTransform` live in [`auki-datatypes::tests`](../auki-datatypes/src/lib.rs) instead). The crate's scope-shrink callout marked complete: this crate is back to its canonical role of identity catalogs only.

**Docs**: README's "Pose Log payload — schema v1" section replaced with a one-paragraph pointer at [`auki-datatypes`](../auki-datatypes); Frame Registry sub-section retained (registry is still here). `src/readme.md`'s "Log payload types" code block dropped both pose structs and the surrounding paragraph; scope-shrink callout updated to "complete." Schema-version line moved `PoseLogEntry` / `TransformSample` out of the in-crate list.

### broodsugar's dobby · May 8, 11:27 HKT, 2026

**Filed: sensor_id convention is load-bearing for cross-peer recording provenance.** Annotated the existing "Formalize the sensor_id naming convention?" parking-lot item with the implication of the [root subscription-as-materialization keystone](../../parking_lot.md#subscription-as-materialization-the-unified-detector-ingestion-architecture-filed-by-dobby-2026-05-08): the `<platform-tag>-<machine-id>/<sensor-name>` pattern (e.g. `K1-AABBCCDDEEFF/head_left_cam`) is what makes a recording self-provenant when it moves between peers — the MAC encodes the producing device, and that encoding survives subscription, archival, and replay. An integrator naming their sensor `my-cool-camera` produces a recording with no provenance signal. Lean recorded: keep the SDK out of string-building (no `SensorId` newtype) but raise the README's status from "recommended" to "REQUIRED for cross-peer recording provenance." Pin before the unified subscription primitive lands. Doc-only.

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
