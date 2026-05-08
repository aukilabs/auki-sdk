# `auki-datatypes/src/`

Implementation status of `auki-datatypes`. Spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). It includes six prost-generated modules:

- `placeholder` — smoke-test only; goes away at Step 7.
- `camera` (Step 1, 2026-05-08) — `PinholeCameraLogEntry` + `DynamicIntrinsics`.
- `point_cloud` (Step 3, 2026-05-08) — `PointCloudLogEntry { bytes data }`. Opaque-bytes-only — layout interpretation comes from `(sensor_id, sensor_hash) → SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id }`. Symmetric with the wire's `PointCloudFrame`.
- `frame_stream` (Step 2, 2026-05-08) — `JpegFrame`. libp2p `/auki/stream/0.1.0` payload.
- `point_cloud_stream` (Step 2, 2026-05-08) — `PointCloudFrame`. libp2p `/auki/stream/0.1.0` payload.
- `stream` (Step 2, 2026-05-08) — full envelope: `StreamMessage` (oneof of `Request | Accept | Decline | Frame | EndOfStream`), `StreamRequest`, `AcceptInfo`, `Frame`, `DeclineReason`, `EndReason`. Helper constructors (`StreamMessage::request/accept/decline/frame/end_of_stream`, `DeclineReason::sensor_not_found/sensor_unavailable/producer_shutting_down/other`, same shape on `EndReason`) live in this module — orphan rule satisfied since impls sit in the type's defining crate.

Plus the `impl_log_payload!` macro that wires every on-disk prost type into [`auki_logs::LogPayload`](../../auki-logs/src/lib.rs).

## What's not here yet

Three remaining on-disk payload schemas. The migration sequence is in [`sprint.md`](sprint.md):

- `auki.audio` — `AudioLogEntry` (Step 4)
- `auki.pose` — `SpatialTransform` (renamed from `TransformSample`; `PoseLogEntry` wrapper goes away — Step 5)
- `auki.time_transform` — `TimeTransformEntry` (was misnamed `TimeTransformLogEntry` in some earlier drafts — Step 6)

Each one **moves** a hand-written serde-derived type currently in [`auki-registry`](../../auki-registry) into here. The `.proto` file becomes the single source of truth; the Rust struct becomes generated; `auki-registry`'s scope shrinks back to identity-only.

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

pub mod point_cloud {
    // prost-generated; opaque-bytes-only.
    pub struct PointCloudLogEntry { pub data: Vec<u8> }
}

pub mod frame_stream {
    pub struct JpegFrame { pub bytes: Vec<u8> }
}

pub mod point_cloud_stream {
    pub struct PointCloudFrame { pub bytes: Vec<u8> }
}

pub mod stream {
    pub struct StreamMessage { pub variant: Option<stream_message::Variant> }
    pub mod stream_message {
        pub enum Variant {
            Request(super::StreamRequest),
            Accept(super::AcceptInfo),
            Decline(super::DeclineReason),
            Frame(super::Frame),
            EndOfStream(super::EndReason),
        }
    }
    pub struct StreamRequest { pub sensor_id: String }
    pub struct AcceptInfo {
        pub sensor_hash: String,
        pub clock_id: String,
        pub clock_hash: String,
    }
    /// `payload` carries prost-encoded `T` bytes; `T` is inferred from
    /// the `AcceptInfo.sensor_hash` handshake (mono-T per substream).
    pub struct Frame { pub timestamp_ns: i64, pub seq: u64, pub payload: Vec<u8> }
    pub struct DeclineReason { pub kind: Option<decline_reason::Kind> }
    pub mod decline_reason {
        pub enum Kind { SensorNotFound(...), SensorUnavailable(...),
                        ProducerShuttingDown(...), Other(Other) }
        pub struct Other { pub detail: String }
    }
    pub struct EndReason { pub kind: Option<end_reason::Kind> }
    pub mod end_reason {
        pub enum Kind { SourceEnded(...), ProducerShuttingDown(...),
                        SessionEnded(...), ProducerError(ProducerError) }
        pub struct ProducerError { pub detail: String }
    }

    // Helper constructors live alongside the prost types:
    impl StreamMessage { pub fn request(...) / accept(...) / decline(...) /
                                 frame(...) / end_of_stream(...) -> Self; }
    impl DeclineReason { pub fn sensor_not_found() / sensor_unavailable() /
                                 producer_shutting_down() / other(detail) -> Self; }
    impl EndReason     { pub fn source_ended() / producer_shutting_down() /
                                 session_ended() / producer_error(detail) -> Self; }
}

pub mod placeholder {
    pub struct PipelineCheck {}
}

// Every on-disk prost type satisfies auki_logs::LogPayload via:
macro_rules! impl_log_payload { ($t:ty) => { /* encode_to_vec / decode */ }; }
impl_log_payload!(camera::PinholeCameraLogEntry);
impl_log_payload!(point_cloud::PointCloudLogEntry);
// (Stream types don't get LogPayload — they're wire types, not on-disk
// payloads. `Frame.payload` carries the on-disk T's prost bytes.)
```

## Tests (13 total)

| Test | Asserts |
|------|---------|
| `placeholder_pipeline_check_round_trips` | `PipelineCheck::default().encode_to_vec().decode() == PipelineCheck::default()` — proves prost-build ran. Goes away with the placeholder at Step 7. |
| `pinhole_camera_log_entry_serializes_to_locked_wire_bytes` | Locked prost wire bytes for the M1 example camera log entry. Cross-language readers must reproduce them. |
| `pinhole_camera_log_entry_hash_is_locked` | XXH3-128 (`auki_hash::hash_jcs_bytes`) of those bytes — `0496e1f71a03e00877fc68bf16190026`. Trips if either prost-build or `auki-hash` drifts. |
| `pinhole_camera_log_entry_round_trips` | `encode_to_vec` → `decode` gives back the same struct. |
| `pinhole_camera_log_entry_log_payload_round_trips` | Same, via the `LogPayload` macro impl — pins the macro wiring. |
| `pinhole_camera_log_entry_without_intrinsics_round_trips` | `dynamic_intrinsics: None` (non-autofocusing camera) round-trips. |
| `pinhole_camera_log_entry_segment_round_trip` | End-to-end seam: open `auki_logs::Log<PinholeCameraLogEntry>`, append two entries (one with intrinsics, one without), close, re-read, assert byte-equality. |
| `point_cloud_log_entry_serializes_to_locked_wire_bytes` | Locked prost wire bytes for a 24-byte fixture: `0a18000102030405060708090a0b0c0d0e0f1011121314151617`. Cross-language readers must reproduce them. |
| `point_cloud_log_entry_hash_is_locked` | XXH3-128 of those bytes — `4ea525d849212b2e067e33bec455c7ea`. |
| `point_cloud_log_entry_round_trips` | `encode_to_vec` → `decode` gives back the same struct. |
| `point_cloud_log_entry_log_payload_round_trips` | Same, via the `LogPayload` macro impl. |
| `point_cloud_log_entry_empty_data_round_trips` | proto3 default-elision: `PointCloudLogEntry { data: vec![] }` encodes to zero bytes and decodes back to the empty form. |
| `point_cloud_log_entry_segment_round_trip` | End-to-end seam: open `auki_logs::Log<PointCloudLogEntry>`, append two entries (one populated, one empty), close, re-read, assert byte-equality. |

## Consumers

- `auki-ros-adapter` — `build_sensor_log_entry` produces `PinholeCameraLogEntry`; `build_point_cloud_log_entry` produces `PointCloudLogEntry` (opaque-bytes-only since Step 3, 2026-05-08; ROS-side `width × height × is_dense` flattened into the bytes via the registry's `point_step` and `fields`).
- `auki-network`'s `stream_protocol` — re-exports `JpegFrame`, `PointCloudFrame`, and the full `auki.stream` envelope; `stream_runtime`'s `T` bound is `prost::Message + Default + Send + 'static` (Step 2, 2026-05-08).
- `auki-network-py` — PyO3 wrappers track the prost match shape; Python surface unchanged.
- `auki-logs` (transitive) — every on-disk prost type gets `LogPayload` for free via `impl_log_payload!`.

The remaining downstream crate (`auki-time-transforms`'s sampler) comes onto generated types when its migration step lands.
