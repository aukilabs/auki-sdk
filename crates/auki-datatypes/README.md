# auki-datatypes

Single source of truth for the SDK's shared cross-language segment + wire payload shapes. Owns the `.proto` schemas and the prost-generated Rust code; cross-language consumers (Python via betterproto, future Sentinel ports) generate their own bindings from the same `.proto` files.

Each module exposes a canonical payload shape used on both disk and wire. The pre-collapse dual `*_stream` packages were merged in PR #176.

**Status:** Shipped.

## Public surface

- `camera::{CameraFrame, DynamicIntrinsics}`. A frame's optional dynamic
  intrinsics replace (rather than merge with) the Camera Registry's static
  calibration; metric consumers require one source or fail closed.
- `point_cloud::Data`, `audio::Data`, `joint_encoders::Data`, `scalar::Data`
- `detection::DetectionFrame`
- `pose::{SpatialTransform, Vec3, Quat}`
- `map::{MapUpdate, VoxelChunkUpdate, VoxelDelta, SemanticDelta}` for
  commutative evidence deltas, plus `VoxelMapCheckpoint` and its snapshot
  payloads for ordered full-state replay barriers. A checkpoint is carried in
  a separate protobuf field with no additive chunks, so older readers safely
  ignore it instead of adding absolute evidence.
- `time_transform::TimeTransformEntry`
- `stream::{StreamMessage, StreamRequest, StreamManifest, StreamEntry, DeclineReason, EndReason}`
  - `StreamRequest` fields (field-number ledger — never reuse or renumber): legacy `sensor_id` (1), `resource_id` (2), `source_peer_id` (3), `read_from` oneof (`latest` = 4, `from_start` = 5, `from_timestamp` = 6 with `int64 timestamp_ns`). New `/auki/stream/0.2.0` opens identify logs by `source_peer_id + resource_id`.
  - Map-update manifests pin the Map Registry identity (`map_peer_id`, `map_id`, `map_hash`) and explicit clock owner (`clock_peer_id`).
- Locked wire-byte vectors pin every payload across language reimplementations.

## Depends on

- [`auki-logs`](../auki-logs) — only for the blanket `LogPayload` impl for prost types.
