# auki-datatypes

Single source of truth for the SDK's shared cross-language segment + wire payload shapes. Owns the `.proto` schemas and the prost-generated Rust code; cross-language consumers (Python via betterproto, future Sentinel ports) generate their own bindings from the same `.proto` files.

Each module exposes one `Data`-shaped message used on both disk (as a Sensor Log segment) and wire (as a `/auki/stream/0.1.0` substream payload). The pre-collapse dual `*_stream` packages were merged in PR #176.

**Status:** Shipped.

## Public surface

- `camera::{CameraFrame, DynamicIntrinsics}`
- `point_cloud::Data`, `audio::Data`, `joint_encoders::Data`
- `detection::DetectionFrame`
- `pose::{SpatialTransform, Vec3, Quat}`
- `time_transform::TimeTransformEntry`
- `stream::{StreamMessage, StreamRequest, StreamManifest, StreamEntry, DeclineReason, EndReason}`
- Locked wire-byte vectors pin every payload across language reimplementations.

## Depends on

- [`auki-logs`](../auki-logs) — only for the blanket `LogPayload` impl for prost types.
