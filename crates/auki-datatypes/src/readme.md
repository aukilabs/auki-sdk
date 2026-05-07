# `auki-datatypes/src/`

Implementation status of `auki-datatypes`. Spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). It includes one prost-generated module — the placeholder package — and a smoke test that validates the build pipeline end-to-end (encode + decode round-trip on `PipelineCheck`).

## What's not here yet

Every real schema. The migration sequence is in [`sprint.md`](sprint.md); briefly, the planned `.proto` packages and their messages (post-rename + post-slop-fix decisions; final names land per-step):

- `auki.camera` — `PinholeCameraLogEntry` (renamed from `SensorLogEntry`)
- `auki.point_cloud` — `PointCloudLogEntry`
- `auki.audio` — `AudioLogEntry`
- `auki.pose` — `SpatialTransform` (renamed from `TransformSample`; `PoseLogEntry` wrapper goes away)
- `auki.time_transform` — `TimeTransformEntry` (was misnamed `TimeTransformLogEntry` in some earlier drafts)
- `auki.frame_stream` — `JpegFrame` (libp2p wire)
- `auki.point_cloud_stream` — `PointCloudFrame` (libp2p wire)

Each one **moves** a hand-written serde-derived type currently in [`auki-registry`](../../auki-registry) (or in [`auki-network`](../../auki-network)'s `stream_protocol` for the libp2p wire types) into here. The `.proto` file becomes the single source of truth; the Rust struct becomes generated; `auki-registry`'s scope shrinks back to identity-only.

Locked conformance vectors land in `tests/locked_vectors.rs` alongside each real `.proto` file.

## Public surface (current)

```rust
pub mod placeholder {
    // prost-generated:
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PipelineCheck {}
}
```

That's the entirety of the workspace-visible surface today.

## Tests

One inline test:

| Test | Asserts |
|------|---------|
| `placeholder_pipeline_check_round_trips` | `PipelineCheck::default().encode_to_vec().decode() == PipelineCheck::default()` — proves prost-build ran, generated code compiles, encode/decode work. |

`cargo test -p auki-datatypes` runs one test; it will go away when the placeholder is removed.

## Consumers

None yet. The migration in [`sprint.md`](sprint.md) brings each downstream crate (`auki-logs`, `auki-network`, `auki-ros-adapter`, `auki-time-transforms`) onto the generated types one at a time.
