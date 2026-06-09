# auki-datatypes

Single source of truth for the SDK's shared cross-language segment + wire payload shapes. Owns the `.proto` schemas and the prost-generated Rust code; cross-language consumers (Python via betterproto, future Sentinel ports) generate their own bindings from the same `.proto` files.

Each module exposes one `Data`-shaped message used on both disk (as a Sensor Log segment) and wire (as a `/auki/stream/0.2.0` substream payload). The pre-collapse dual `*_stream` packages were merged in PR #176.

**Status:** Shipped.

## Public surface

- `camera::{CameraFrame, DynamicIntrinsics}`
- `point_cloud::Data`, `audio::Data`, `joint_encoders::Data`
- `detection::DetectionFrame`
- `pose::{SpatialTransform, Vec3, Quat}`
- `time_transform::TimeTransformEntry`
- `stream::{StreamMessage, StreamRequest, StreamManifest, StreamEntry, DeclineReason, EndReason}`
  - `StreamRequest` fields (field-number ledger — never reuse or renumber): legacy `sensor_id` (1), `resource_id` (2), `source_peer_id` (3), `read_from` oneof (`latest` = 4, `from_start` = 5, `from_timestamp` = 6 with `int64 timestamp_ns`). New `/auki/stream/0.2.0` opens identify logs by `source_peer_id + resource_id`.
- Locked wire-byte vectors pin every payload across language reimplementations.

## Depends on

- [`auki-logs`](../auki-logs) — only for the blanket `LogPayload` impl for prost types.
