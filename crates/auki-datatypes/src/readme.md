# `auki-datatypes/src/`

Implementation status of `auki-datatypes`. Spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). It includes the prost-generated modules — every on-disk and libp2p-wire payload type the SDK ships:

- `camera` (Step 1, 2026-05-08) — `PinholeCameraLogEntry` + `DynamicIntrinsics`.
- `point_cloud` (Step 3, 2026-05-08) — `PointCloudLogEntry { bytes data }`. Opaque-bytes-only — layout interpretation comes from `(sensor_id, sensor_hash) → SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id }`. Symmetric with the wire's `PointCloudFrame`.
- `audio` (Step 4, 2026-05-08) — `AudioLogEntry { bytes data }`. Opaque-bytes-only — `sample_format`, `channels`, `sample_rate_hz`, `channel_layout` come from `(sensor_id, sensor_hash) → SensorBody::Microphone`.
- `pose` (Step 5, 2026-05-08) — `SpatialTransform { Vec3 translation; Quat orientation }`, flat. No `PoseLogEntry` wrapper, no per-sample `parent_frame`/`child_frame` — frame identity lives in the manifest's `(from_frame_id, to_frame_id)` pair. Quat is `(x, y, z, w)` Hamilton.
- `time_transform` (Step 6, 2026-05-08) — `TimeTransformEntry { int64 offset_ns; uint32 uncertainty_ns }`. The pre-migration per-entry `source` field moved to the manifest as a tagged-enum `TimeTransformSource` (mirrors `PoseSource`); the per-entry `discontinuous: bool` is gone (computed on read).
- `detection` (Step 8, 2026-05-08) — `DetectionLogEntry { bytes data }`. Opaque-bytes-only — the per-frame detection schema is defined by the Detector, not the SDK (QR portal-uid + corners; ESL class + bbox + confidence; people bboxes). Closes the producer side of the [subscription-as-materialization keystone](../../../parking_lot.md): a Detection Log is `Log<T>` with `T = DetectionLogEntry`, lifecycle inherited from the sensor-log primitive — no "DetectionLog" abstraction. The exact registry shape that pins per-`(detector_id, ...)` interpretation is TBD; it'll be the Detection-Log analog of `SensorRegistryEntry`.
- `joint_encoders` (2026-05-09) — `JointEncodersLogEntry { repeated float angles_rad }`. Per-frame joint-encoder readings in radians, indexed in the producer's emit order; vector length pinned by `(sensor_id, sensor_hash) → SensorBody::JointEncoders { joint_count }`. Joint angles are encoder readings — measurements before any kinematic interpretation; FK against the URDF is a consumer-side derivation (Park).
- `frame_stream` (Step 2, 2026-05-08) — `JpegFrame`. libp2p `/auki/stream/0.1.0` payload.
- `point_cloud_stream` (Step 2, 2026-05-08) — `PointCloudFrame`. libp2p `/auki/stream/0.1.0` payload.
- `joint_encoders_stream` (2026-05-09) — `JointEncodersFrame { repeated float angles_rad }`. libp2p `/auki/stream/0.1.0` payload. Same shape as `joint_encoders::JointEncodersLogEntry` (separate proto package so wire and disk dispatch on distinct Rust types — Step 2/3 precedent). Symmetry locked by an explicit `joint_encoders_disk_wire_byte_identical` test.
- `stream` (Step 2, 2026-05-08) — full envelope: `StreamMessage` (oneof of `Request | Accept | Decline | Frame | EndOfStream`), `StreamRequest`, `AcceptInfo`, `Frame`, `DeclineReason`, `EndReason`. Helper constructors (`StreamMessage::request/accept/decline/frame/end_of_stream`, `DeclineReason::sensor_not_found/sensor_unavailable/producer_shutting_down/other`, same shape on `EndReason`) live in this module — orphan rule satisfied since impls sit in the type's defining crate.

Plus the `impl_log_payload!` macro that wires every on-disk prost type into [`auki_logs::LogPayload`](../../auki-logs/src/lib.rs).

## What's not here yet

Every on-disk and libp2p-wire payload type the SDK ships today lives here. The 2026-05-08 migration ran from Step 0 through Step 7; Step 8 followed the same day to close the producer side of the Detector keystone. The `placeholder.proto` smoke-test that proved out the prost-build pipeline before any real schema landed is gone.

The detection-log analog of `SensorRegistryEntry` — the registry entry that pins per-`(detector_id, ...)` interpretation of the opaque `DetectionLogEntry.data` bytes — is **not in this crate's scope**; it's a forthcoming registry shape, sibling to Sensor / Frame / Clock entries. When that registry's identity body lands, it lives in [`auki-registry`](../../auki-registry) (JCS-canonical JSON) the same way `SensorBody` does, not here.

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

pub mod audio {
    // prost-generated; opaque-bytes-only.
    pub struct AudioLogEntry { pub data: Vec<u8> }
}

pub mod pose {
    // prost-generated. proto3 message-typed fields are Option<T>.
    pub struct SpatialTransform {
        pub translation: Option<Vec3>,
        pub orientation: Option<Quat>,
    }
    pub struct Vec3 { pub x: f64, pub y: f64, pub z: f64 }
    pub struct Quat { pub x: f64, pub y: f64, pub z: f64, pub w: f64 }
}

pub mod time_transform {
    // prost-generated.
    pub struct TimeTransformEntry {
        pub offset_ns: i64,        // to_clock - from_clock at sample instant
        pub uncertainty_ns: u32,   // m2 - m1 from the three-read protocol
    }
}

pub mod detection {
    // prost-generated; opaque-bytes-only.
    pub struct DetectionLogEntry { pub data: Vec<u8> }
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

// Every on-disk prost type satisfies auki_logs::LogPayload via:
macro_rules! impl_log_payload { ($t:ty) => { /* encode_to_vec / decode */ }; }
impl_log_payload!(camera::PinholeCameraLogEntry);
impl_log_payload!(point_cloud::PointCloudLogEntry);
impl_log_payload!(audio::AudioLogEntry);
impl_log_payload!(pose::SpatialTransform);
impl_log_payload!(time_transform::TimeTransformEntry);
impl_log_payload!(detection::DetectionLogEntry);
// (Stream types don't get LogPayload — they're wire types, not on-disk
// payloads. `Frame.payload` carries the on-disk T's prost bytes.)
```

## Tests (37 total)

| Test | Asserts |
|------|---------|
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
| `audio_log_entry_serializes_to_locked_wire_bytes` | Locked prost wire bytes for a 16-byte `pcm_s16le` stereo fixture: `0a1000112233445566778899aabbccddeeff`. |
| `audio_log_entry_hash_is_locked` | XXH3-128 of those bytes — `a5864ae7018f28a5c094a714af1db62e`. |
| `audio_log_entry_round_trips` | `encode_to_vec` → `decode` gives back the same struct. |
| `audio_log_entry_log_payload_round_trips` | Same, via the `LogPayload` macro impl. |
| `audio_log_entry_empty_data_round_trips` | proto3 default-elision: empty chunk encodes to zero bytes. |
| `audio_log_entry_segment_round_trip` | End-to-end seam: open `auki_logs::Log<AudioLogEntry>`, append two entries (one populated, one empty), close, re-read. |
| `spatial_transform_serializes_to_locked_wire_bytes` | Locked prost wire bytes for an identity-rotation 1-2-3-translation fixture: `0a1b09…120921000000000000f03f`. proto3 default-elision: zero-valued `double` fields don't appear on the wire. |
| `spatial_transform_hash_is_locked` | XXH3-128 of those bytes — `29fa6349ab0b3ff1f06933489db74dfd`. |
| `spatial_transform_round_trips` | `encode_to_vec` → `decode` gives back the same struct. |
| `spatial_transform_log_payload_round_trips` | Same, via the `LogPayload` macro impl. |
| `spatial_transform_default_round_trips` | proto3 default-elision: `SpatialTransform { translation: None, orientation: None }` encodes to zero bytes. |
| `spatial_transform_segment_round_trip` | End-to-end seam: open `auki_logs::Log<SpatialTransform>`, append two entries (one populated, one default), close, re-read. Pins the flat-not-wrapped shape end-to-end. |
| `time_transform_entry_serializes_to_locked_wire_bytes` | Locked prost wire bytes for `offset_ns: 1_000_000, uncertainty_ns: 250` — `08c0843d10fa01` (7 bytes; both varint fields). |
| `time_transform_entry_hash_is_locked` | XXH3-128 of those bytes — `b7e73628833419a7c299933d07cbe88c`. |
| `time_transform_entry_round_trips` | `encode_to_vec` → `decode` gives back the same struct. |
| `time_transform_entry_log_payload_round_trips` | Same, via the `LogPayload` macro impl. |
| `time_transform_entry_zero_offset_round_trips` | proto3 default-elision: zeroed entry encodes to zero bytes. |
| `time_transform_entry_negative_offset_round_trips` | Negative `offset_ns` round-trips (proto3 `int64` is non-zigzag, 10-byte varint for negatives). |
| `time_transform_entry_segment_round_trip` | End-to-end seam: open `auki_logs::Log<TimeTransformEntry>`, append two entries, close, re-read. |
| `detection_log_entry_serializes_to_locked_wire_bytes` | Locked prost wire bytes for a 12-byte fixture: `0a0c000102030405060708090a0b`. Cross-language readers must reproduce them. |
| `detection_log_entry_hash_is_locked` | XXH3-128 of those bytes — `94f8efe6be63d3dc5e045ab08d538a15`. |
| `detection_log_entry_round_trips` | `encode_to_vec` → `decode` gives back the same struct. |
| `detection_log_entry_log_payload_round_trips` | Same, via the `LogPayload` macro impl. |
| `detection_log_entry_empty_data_round_trips` | proto3 default-elision: empty payload (Detector ran, saw nothing) encodes to zero bytes. |
| `detection_log_entry_segment_round_trip` | End-to-end seam: open `auki_logs::Log<DetectionLogEntry>`, append two entries (one populated, one empty), close, re-read. |

## Consumers

- `auki-ros-adapter` — `build_sensor_log_entry` produces `PinholeCameraLogEntry`; `build_point_cloud_log_entry` produces `PointCloudLogEntry` (opaque-bytes-only since Step 3, 2026-05-08; ROS-side `width × height × is_dense` flattened into the bytes via the registry's `point_step` and `fields`). No `AudioLogEntry` builder yet — the type is here for future audio capture pipelines. No `SpatialTransform` builder yet either — a future TF adapter would fan a `TFMessage` into N parallel pose logs.
- `auki-time-transforms` — `tick()` and `Sampler::start` produce `TimeTransformEntry` (re-exported from this crate since Step 6, 2026-05-08); `TimeTransformSource` is now manifest metadata in `auki-manifests`, also re-exported.
- `auki-manifests` — `build_pose_log_manifest` references the new pose-log shape: `(from_frame_id, from_frame_hash, to_frame_id, to_frame_hash, …, writer_mode: PoseWriterMode, expected_rate_hz)` (Step 5, 2026-05-08). `build_time_transform_log_manifest` takes `&TimeTransformSource` since Step 6.
- `auki-layout` — `poselog_path(session_root, from_frame_id, to_frame_id) -> PathBuf` mirrors `timetransform_log_path`'s `(from, to)`-keyed shape (Step 5).
- `auki-network`'s `stream_protocol` — re-exports `JpegFrame`, `PointCloudFrame`, and the full `auki.stream` envelope; `stream_runtime`'s `T` bound is `prost::Message + Default + Send + 'static` (Step 2, 2026-05-08).
- `auki-network-py` — PyO3 wrappers track the prost match shape; Python surface unchanged.
- `auki-logs` (transitive) — every on-disk prost type gets `LogPayload` for free via `impl_log_payload!`.
- [`detectors`](https://github.com/aukilabs/detectors) (downstream) — phase-2 Detection-Log writers append `DetectionLogEntry { data: <detector-specific schema> }` to a `Log<DetectionLogEntry>`; the SDK doesn't decode `data`, so each detector controls its own schema. Step 8 unblocks phase-2 blocker #3 (the `DetectionLogEntry` type); the remaining phase-2 blockers — `Log<T>::tail()` for the read side, the Detector binding API, the `auki-sdk-py` Python binding — sit elsewhere.
