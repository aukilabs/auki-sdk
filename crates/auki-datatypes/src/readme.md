# `auki-datatypes/src/`

Implementation status of `auki-datatypes`. Spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). It includes two prost-generated modules — the placeholder (smoke-test) and `auki.camera` (the first real schema, landed Step 1 on 2026-05-08) — plus the `impl_log_payload!` macro that wires every prost type into [`auki_logs::LogPayload`](../../auki-logs/src/lib.rs).

## What's not here yet

Five remaining payload schemas. The migration sequence is in [`sprint.md`](sprint.md); the remaining `.proto` packages and their messages:

- `auki.point_cloud` — `PointCloudLogEntry` (Step 3)
- `auki.audio` — `AudioLogEntry` (Step 4)
- `auki.pose` — `SpatialTransform` (renamed from `TransformSample`; `PoseLogEntry` wrapper goes away — Step 5)
- `auki.time_transform` — `TimeTransformEntry` (was misnamed `TimeTransformLogEntry` in some earlier drafts — Step 6)
- `auki.frame_stream` — `JpegFrame` (libp2p wire — Step 2)
- `auki.point_cloud_stream` — `PointCloudFrame` (libp2p wire — Step 2)

Each one **moves** a hand-written serde-derived type currently in [`auki-registry`](../../auki-registry) (or in [`auki-network`](../../auki-network)'s `stream_protocol` for the libp2p wire types) into here. The `.proto` file becomes the single source of truth; the Rust struct becomes generated; `auki-registry`'s scope shrinks back to identity-only.

## Public surface (current)

```rust
pub mod camera {
    // prost-generated:
    pub struct PinholeCameraLogEntry {
        pub dynamic_intrinsics: Option<DynamicIntrinsics>,
        pub frame: Vec<u8>,
    }
    pub struct DynamicIntrinsics {
        pub fx: f64, pub fy: f64, pub cx: f64, pub cy: f64,
        pub distortion_coefficients: Vec<f64>,
    }
}

pub mod placeholder {
    pub struct PipelineCheck {}
}

// Each prost type satisfies auki_logs::LogPayload via:
macro_rules! impl_log_payload { ($t:ty) => { /* encode_to_vec / decode */ }; }
impl_log_payload!(camera::PinholeCameraLogEntry);
```

## Tests (7 total)

| Test | Asserts |
|------|---------|
| `placeholder_pipeline_check_round_trips` | `PipelineCheck::default().encode_to_vec().decode() == PipelineCheck::default()` — proves prost-build ran. Goes away with the placeholder at Step 7. |
| `pinhole_camera_log_entry_serializes_to_locked_wire_bytes` | Locked prost wire bytes for the M1 example camera log entry. Cross-language readers must reproduce them. |
| `pinhole_camera_log_entry_hash_is_locked` | XXH3-128 (`auki_hash::hash_jcs_bytes`) of those bytes — `0496e1f71a03e00877fc68bf16190026`. Trips if either prost-build or `auki-hash` drifts. |
| `pinhole_camera_log_entry_round_trips` | `encode_to_vec` → `decode` gives back the same struct. |
| `pinhole_camera_log_entry_log_payload_round_trips` | Same, via the `LogPayload` macro impl — pins the macro wiring. |
| `pinhole_camera_log_entry_without_intrinsics_round_trips` | `dynamic_intrinsics: None` (non-autofocusing camera) round-trips. |
| `pinhole_camera_log_entry_segment_round_trip` | End-to-end seam: open `auki_logs::Log<PinholeCameraLogEntry>`, append two entries (one with intrinsics, one without), close, re-read, assert byte-equality. |

## Consumers

- `auki-ros-adapter` — `build_sensor_log_entry` produces `PinholeCameraLogEntry` ready for `auki_logs::Log::append` (Step 1, 2026-05-08).
- `auki-logs` (transitive) — every prost type gets `LogPayload` for free via `impl_log_payload!`.

The remaining downstream crates (`auki-network`'s stream protocol, `auki-time-transforms`'s sampler) come onto generated types one at a time as their migration steps land.
