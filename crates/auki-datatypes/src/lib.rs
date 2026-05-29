//! Single source of truth for the Auki SDK's shared cross-language data
//! types — the typed payload shapes that flow through logs and streams.
//!
//! The `.proto` files live in [`proto/`](../proto/); `build.rs` invokes
//! `prost-build` to generate Rust code into `OUT_DIR`, included here
//! one module per `.proto` package.
//!
//! Crate name names the **responsibility** (canonical shared data
//! types), not the implementation (protobuf via prost). Encoding could
//! change someday; the responsibility doesn't.
//!
//! See the [outer `README.md`](../README.md) for the spec, the
//! [`parking_lot.md`](../parking_lot.md) for open questions, and
//! [`src/readme.md`](readme.md) for current implementation status.
//!
//! Each generated payload type gets a [`auki_logs::LogPayload`] impl
//! through the `impl_log_payload!` macro below — consumers drop them
//! straight into `auki_logs::Log<T>` without any glue code.

#![allow(missing_docs, clippy::derive_partial_eq_without_eq)]

/// Implement [`auki_logs::LogPayload`] for a prost-generated message
/// type. Encodes via `prost::Message::encode_to_vec`, decodes via
/// `prost::Message::decode`, surfaces decode errors as the `String`
/// shape `LogPayload::decode` returns.
macro_rules! impl_log_payload {
    ($t:ty) => {
        impl ::auki_logs::LogPayload for $t {
            fn encode(&self) -> ::std::vec::Vec<u8> {
                ::prost::Message::encode_to_vec(self)
            }
            fn decode(bytes: &[u8]) -> ::std::result::Result<Self, ::std::string::String> {
                <Self as ::prost::Message>::decode(bytes).map_err(|e| e.to_string())
            }
        }
    };
}

/// `auki.camera` — Pinhole camera log payload (Sensor Log family).
/// Migration Step 1.
pub mod camera {
    include!(concat!(env!("OUT_DIR"), "/auki.camera.rs"));
}

impl_log_payload!(camera::CameraFrame);

/// `auki.point_cloud` — opaque-bytes point-cloud payload shared by disk
/// (Sensor Log segment) and wire (`/auki/stream/0.1.0` substream). One
/// `Data` type, byte-identical encoding on both paths. Layout fields
/// (`fields`, `point_step`, `is_bigendian`, `frame_id`) live on the
/// SensorRegistryEntry's `PointCloud` body — interpretation comes from
/// `(sensor_id, sensor_hash)`, not from the per-frame payload.
pub mod point_cloud {
    include!(concat!(env!("OUT_DIR"), "/auki.point_cloud.rs"));
}

impl_log_payload!(point_cloud::Data);

/// `auki.joint_encoders` — joint-encoder payload shared by disk
/// (Sensor Log segment) and wire (`/auki/stream/0.1.0` substream). One
/// `Data` type, byte-identical encoding on both paths. Per-sample
/// `repeated float angles_rad`; vector length pinned by the
/// `SensorRegistryEntry`'s `JointEncoders { joint_count }` body via
/// `(sensor_id, sensor_hash)`. Joint angles are encoder readings —
/// measurements before any kinematic interpretation; FK against the
/// URDF is a consumer-side derivation.
pub mod joint_encoders {
    include!(concat!(env!("OUT_DIR"), "/auki.joint_encoders.rs"));
}

impl_log_payload!(joint_encoders::Data);

/// `auki.audio` — opaque-bytes audio payload shared by disk (Sensor Log
/// segment) and wire (`/auki/stream/0.1.0` substream). One `Data` type,
/// byte-identical encoding on both paths. `sample_format`, `channels`,
/// `sample_rate_hz`, `channel_layout`, `frame_id` live on the
/// SensorRegistryEntry's `Audio` body — interpretation comes from
/// `(sensor_id, sensor_hash)`, not from the per-chunk payload. Sample
/// count and chunk duration are derivable from the bytes plus the
/// registry; the chunk start timestamp rides in the framing's
/// `timestamp_ns`.
pub mod audio {
    include!(concat!(env!("OUT_DIR"), "/auki.audio.rs"));
}

impl_log_payload!(audio::Data);

/// `auki.detection` — opaque-bytes detection log payload (Detection Log
/// family). Migration Step 8. The detection schema is defined by the
/// Detector — the SDK does not interpret detector-specific fields.
/// Closes the producer side of the subscription-as-materialization
/// keystone: a Detection Log is `Log<T>` with `T = DetectionFrame`,
/// lifecycle inherited from the sensor-log primitive. The frame
/// timestamp rides in the auki-logs framing's `timestamp_ns`.
pub mod detection {
    include!(concat!(env!("OUT_DIR"), "/auki.detection.rs"));
}

impl_log_payload!(detection::DetectionFrame);

/// `auki.pose` — Pose Log segment payload (Migration Step 5). Flat
/// `SpatialTransform` per entry — no `PoseLogEntry { transforms: Vec<…> }`
/// wrapper, no per-sample `parent_frame` / `child_frame`. Frame identity
/// lives in the log's manifest (`from_frame_id` / `from_frame_hash` /
/// `to_frame_id` / `to_frame_hash`); each Pose Log holds one
/// `(from, to)` pair. Quaternion is `(x, y, z, w)` Hamilton.
pub mod pose {
    include!(concat!(env!("OUT_DIR"), "/auki.pose.rs"));
}

impl_log_payload!(pose::SpatialTransform);

/// `auki.time_transform` — TimeTransform Log segment payload
/// (Migration Step 6). `offset_ns` (`to_clock - from_clock`) and
/// `uncertainty_ns` only — the pre-migration `source` field moved to
/// the manifest as a tagged-enum `TimeTransformSource` (mirrors
/// `PoseSource`); the pre-migration `discontinuous: bool` flag is
/// gone (computed on read using the reader's own threshold). The
/// sample's timestamp rides in the auki-logs framing's
/// `timestamp_ns` (from-clock reading at the sample instant).
pub mod time_transform {
    include!(concat!(env!("OUT_DIR"), "/auki.time_transform.rs"));
}

impl_log_payload!(time_transform::TimeTransformEntry);

/// `auki.join` — `/auki/join/0.0.1` request/response messages.
pub mod join {
    include!(concat!(env!("OUT_DIR"), "/auki.join.rs"));

    impl JoinResponse {
        pub fn accept(membership_json: impl Into<String>, successor_token: Vec<u8>) -> Self {
            Self {
                kind: Some(join_response::Kind::Accept(join_response::Accept {
                    membership_json: membership_json.into(),
                    successor_token,
                })),
            }
        }

        pub fn reject(reason: impl Into<String>) -> Self {
            Self {
                kind: Some(join_response::Kind::Reject(join_response::Reject {
                    reason: reason.into(),
                })),
            }
        }
    }
}

/// `auki.info` — `/auki/info/0.0.1` request/response messages.
pub mod info {
    include!(concat!(env!("OUT_DIR"), "/auki.info.rs"));
}

/// `auki.sensors` — `/auki/sensors/0.0.1` request/response messages.
pub mod sensors {
    include!(concat!(env!("OUT_DIR"), "/auki.sensors.rs"));

    impl SensorsRequest {
        pub fn catalog() -> Self {
            Self::default()
        }

        pub fn with_registry_entries() -> Self {
            Self {
                include_registry_entries: true,
                include_frame_entries: false,
            }
        }

        pub fn with_frame_entries() -> Self {
            Self {
                include_registry_entries: true,
                include_frame_entries: true,
            }
        }
    }
}

/// `auki.stream` — `StreamMessage` envelope, `StreamRequest`,
/// `StreamManifest`, `StreamEntry`, `DeclineReason`, `EndReason`. The
/// libp2p substream wire shape; mono-`T` per substream, with
/// `StreamEntry.payload` carrying the prost-encoded `T` bytes.
pub mod stream {
    include!(concat!(env!("OUT_DIR"), "/auki.stream.rs"));

    impl StreamMessage {
        pub fn request(req: StreamRequest) -> Self {
            Self {
                variant: Some(stream_message::Variant::Request(req)),
            }
        }
        pub fn accept(manifest: StreamManifest) -> Self {
            Self {
                variant: Some(stream_message::Variant::Accept(manifest)),
            }
        }
        pub fn decline(reason: DeclineReason) -> Self {
            Self {
                variant: Some(stream_message::Variant::Decline(reason)),
            }
        }
        pub fn entry(entry: StreamEntry) -> Self {
            Self {
                variant: Some(stream_message::Variant::Entry(entry)),
            }
        }
        pub fn end_of_stream(reason: EndReason) -> Self {
            Self {
                variant: Some(stream_message::Variant::EndOfStream(reason)),
            }
        }
    }

    impl DeclineReason {
        pub fn sensor_not_found() -> Self {
            Self {
                kind: Some(decline_reason::Kind::SensorNotFound(
                    decline_reason::SensorNotFound {},
                )),
            }
        }
        pub fn sensor_unavailable() -> Self {
            Self {
                kind: Some(decline_reason::Kind::SensorUnavailable(
                    decline_reason::SensorUnavailable {},
                )),
            }
        }
        pub fn producer_shutting_down() -> Self {
            Self {
                kind: Some(decline_reason::Kind::ProducerShuttingDown(
                    decline_reason::ProducerShuttingDown {},
                )),
            }
        }
        pub fn other(detail: impl Into<String>) -> Self {
            Self {
                kind: Some(decline_reason::Kind::Other(decline_reason::Other {
                    detail: detail.into(),
                })),
            }
        }
    }

    impl EndReason {
        pub fn source_ended() -> Self {
            Self {
                kind: Some(end_reason::Kind::SourceEnded(end_reason::SourceEnded {})),
            }
        }
        pub fn producer_shutting_down() -> Self {
            Self {
                kind: Some(end_reason::Kind::ProducerShuttingDown(
                    end_reason::ProducerShuttingDown {},
                )),
            }
        }
        pub fn session_ended() -> Self {
            Self {
                kind: Some(end_reason::Kind::SessionEnded(end_reason::SessionEnded {})),
            }
        }
        pub fn producer_error(detail: impl Into<String>) -> Self {
            Self {
                kind: Some(end_reason::Kind::ProducerError(end_reason::ProducerError {
                    detail: detail.into(),
                })),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::audio;
    use super::camera::{CameraFrame, DynamicIntrinsics};
    use super::detection::DetectionFrame;
    use super::joint_encoders;
    use super::point_cloud;
    use super::pose::{Quat, SpatialTransform, Vec3};
    use super::time_transform::TimeTransformEntry;
    use prost::Message;

    // ─── auki.camera locked vectors ──────────────────────────────────────────

    fn m1_camera_frame() -> CameraFrame {
        CameraFrame {
            dynamic_intrinsics: Some(DynamicIntrinsics {
                fx: 1234.5,
                fy: 1234.5,
                cx: 272.0,
                cy: 244.0,
                distortion_coefficients: vec![0.1, -0.2, 0.001, 0.002, 0.0],
            }),
            frame: vec![0x00, 0x01, 0x02, 0x03],
        }
    }

    /// Locks the prost wire bytes for the M1 example pinhole camera log
    /// entry. Cross-language readers (Python via betterproto, future
    /// Sentinel ports) MUST produce these exact bytes for the same
    /// input — joins the workspace's locked conformance set.
    #[test]
    fn camera_frame_serializes_to_locked_wire_bytes() {
        let bytes = m1_camera_frame().encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "0a4e0900000000004a93401100000000004a93401900000000000071402100000000008\
             06e402a289a9999999999b93f9a9999999999c9bffca9f1d24d62503ffca9f1d24d62603\
             f0000000000000000120400010203"
        );
    }

    /// Locks the XXH3-128 of those wire bytes. Trips if either prost-build
    /// or `auki-hash` drifts.
    #[test]
    fn camera_frame_hash_is_locked() {
        let bytes = m1_camera_frame().encode_to_vec();
        assert_eq!(
            auki_hash::hash_jcs_bytes(&bytes),
            "0496e1f71a03e00877fc68bf16190026"
        );
    }

    #[test]
    fn camera_frame_round_trips() {
        let entry = m1_camera_frame();
        let bytes = entry.encode_to_vec();
        let decoded = CameraFrame::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// `LogPayload` round-trip — proves the macro-generated impl rides
    /// the same prost path as direct `Message::encode_to_vec` /
    /// `decode` calls. Locks the trait wiring against accidental
    /// regression.
    #[test]
    fn camera_frame_log_payload_round_trips() {
        use auki_logs::LogPayload;
        let entry = m1_camera_frame();
        let bytes = LogPayload::encode(&entry);
        let decoded = <CameraFrame as LogPayload>::decode(&bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// Absent-intrinsics case for non-autofocusing cameras — the inline-
    /// optional encoding pays only the message-tag overhead when
    /// `dynamic_intrinsics` is `None`.
    #[test]
    fn camera_frame_without_intrinsics_round_trips() {
        let entry = CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![0xff; 16],
        };
        let bytes = entry.encode_to_vec();
        let decoded = CameraFrame::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// End-to-end seam test: open a real `auki_logs::Log<CameraFrame>`,
    /// append two entries (one with intrinsics, one without), close, re-read,
    /// assert order + payload byte-equality. Catches any regression in the
    /// `LogPayload` macro wiring or the segment-framing path.
    #[test]
    fn camera_frame_segment_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 60_000_000_000i64,
            "kind": "test"
        });
        {
            let mut log: auki_logs::Log<CameraFrame> =
                auki_logs::Log::open(dir.path(), manifest).unwrap();
            log.append(100, &m1_camera_frame()).unwrap();
            log.append(
                200,
                &CameraFrame {
                    dynamic_intrinsics: None,
                    frame: vec![0xab; 8],
                },
            )
            .unwrap();
        }
        let reader: auki_logs::LogReader<CameraFrame> =
            auki_logs::Log::<CameraFrame>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload, m1_camera_frame());
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.dynamic_intrinsics, None);
        assert_eq!(entries[1].payload.frame, vec![0xab; 8]);
    }

    // ─── auki.point_cloud locked vectors ─────────────────────────────────────

    /// Two XYZ float32 points = 24 bytes, deterministic content. Stands
    /// in for a real PointCloud2 CDR payload — the SDK only sees opaque
    /// bytes, so the exact contents don't matter beyond reproducibility.
    fn step3_point_cloud_data() -> point_cloud::Data {
        point_cloud::Data {
            data: (0..24u8).collect(),
        }
    }

    /// Locks the prost wire bytes for the example point-cloud payload.
    /// Cross-language readers MUST produce these exact bytes for the
    /// same input. Field 1 length-delimited: tag 0x0a, varint length
    /// 0x18 (24), then the 24 payload bytes. Same bytes whether the
    /// payload travels on disk (Sensor Log segment) or on the wire
    /// (`/auki/stream/0.1.0` substream).
    #[test]
    fn point_cloud_data_serializes_to_locked_wire_bytes() {
        let bytes = step3_point_cloud_data().encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(hex, "0a18000102030405060708090a0b0c0d0e0f1011121314151617");
    }

    /// XXH3-128 of those wire bytes — joins the workspace's locked
    /// conformance set so future drift in either prost-build or
    /// auki-hash trips the test.
    #[test]
    fn point_cloud_data_hash_is_locked() {
        let bytes = step3_point_cloud_data().encode_to_vec();
        assert_eq!(
            auki_hash::hash_jcs_bytes(&bytes),
            "4ea525d849212b2e067e33bec455c7ea"
        );
    }

    #[test]
    fn point_cloud_data_round_trips() {
        let entry = step3_point_cloud_data();
        let bytes = entry.encode_to_vec();
        let decoded = point_cloud::Data::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// `LogPayload` round-trip — proves the macro-generated impl rides
    /// the same prost path as direct `Message::encode_to_vec` /
    /// `decode` calls.
    #[test]
    fn point_cloud_data_log_payload_round_trips() {
        use auki_logs::LogPayload;
        let entry = step3_point_cloud_data();
        let bytes = LogPayload::encode(&entry);
        let decoded = <point_cloud::Data as LogPayload>::decode(&bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// Empty payload — opaque-bytes-only is honest about empty: a frame
    /// with zero points encodes to a single tag byte (no length, no body).
    #[test]
    fn point_cloud_data_empty_round_trips() {
        let entry = point_cloud::Data { data: vec![] };
        let bytes = entry.encode_to_vec();
        // proto3 default-elision: an empty `bytes` field encodes as
        // zero output bytes (the field is its default value).
        assert_eq!(bytes.len(), 0);
        let decoded = point_cloud::Data::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// End-to-end seam test: open a real `auki_logs::Log<point_cloud::Data>`,
    /// append two entries (one populated, one empty), close, re-read,
    /// assert order + payload byte-equality. Catches any regression in the
    /// `LogPayload` macro wiring or the segment-framing path.
    #[test]
    fn point_cloud_data_segment_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 60_000_000_000i64,
            "kind": "test"
        });
        {
            let mut log: auki_logs::Log<point_cloud::Data> =
                auki_logs::Log::open(dir.path(), manifest).unwrap();
            log.append(100, &step3_point_cloud_data()).unwrap();
            log.append(200, &point_cloud::Data { data: vec![] })
                .unwrap();
        }
        let reader: auki_logs::LogReader<point_cloud::Data> =
            auki_logs::Log::<point_cloud::Data>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload, step3_point_cloud_data());
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.data, Vec::<u8>::new());
    }

    // ─── auki.audio locked vectors ───────────────────────────────────────────

    /// 16 bytes of stereo `pcm_s16le` — 4 frames × 2 channels × 2 bytes.
    /// Stands in for a real audio chunk; the SDK only sees opaque bytes,
    /// so the exact contents don't matter beyond reproducibility.
    fn step4_audio_data() -> audio::Data {
        audio::Data {
            data: (0..16u8).map(|i| i.wrapping_mul(17)).collect(),
        }
    }

    /// Locks the prost wire bytes for the example audio payload. Field 1
    /// length-delimited: tag 0x0a, varint length 0x10 (16), then the 16
    /// payload bytes (`0x00, 0x11, 0x22, ..., 0xff`). Cross-language
    /// readers MUST reproduce them. Same bytes whether the payload
    /// travels on disk (Sensor Log segment) or on the wire
    /// (`/auki/stream/0.1.0` substream).
    #[test]
    fn audio_data_serializes_to_locked_wire_bytes() {
        let bytes = step4_audio_data().encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(hex, "0a1000112233445566778899aabbccddeeff");
    }

    /// XXH3-128 of those wire bytes.
    #[test]
    fn audio_data_hash_is_locked() {
        let bytes = step4_audio_data().encode_to_vec();
        assert_eq!(
            auki_hash::hash_jcs_bytes(&bytes),
            "a5864ae7018f28a5c094a714af1db62e"
        );
    }

    #[test]
    fn audio_data_round_trips() {
        let entry = step4_audio_data();
        let bytes = entry.encode_to_vec();
        let decoded = audio::Data::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// `LogPayload` round-trip — proves the macro-generated impl rides
    /// the same prost path as direct `Message::encode_to_vec` / `decode`.
    #[test]
    fn audio_data_log_payload_round_trips() {
        use auki_logs::LogPayload;
        let entry = step4_audio_data();
        let bytes = LogPayload::encode(&entry);
        let decoded = <audio::Data as LogPayload>::decode(&bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// Empty chunk — proto3 default-elision: zero-byte chunk encodes to
    /// zero output bytes and decodes back to the empty form.
    #[test]
    fn audio_data_empty_round_trips() {
        let entry = audio::Data { data: vec![] };
        let bytes = entry.encode_to_vec();
        assert_eq!(bytes.len(), 0);
        let decoded = audio::Data::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// End-to-end seam: open a real `auki_logs::Log<audio::Data>`,
    /// append two entries (one populated, one empty), close, re-read,
    /// assert order + payload byte-equality.
    #[test]
    fn audio_data_segment_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 60_000_000_000i64,
            "kind": "test"
        });
        {
            let mut log: auki_logs::Log<audio::Data> =
                auki_logs::Log::open(dir.path(), manifest).unwrap();
            log.append(100, &step4_audio_data()).unwrap();
            log.append(200, &audio::Data { data: vec![] }).unwrap();
        }
        let reader: auki_logs::LogReader<audio::Data> =
            auki_logs::Log::<audio::Data>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload, step4_audio_data());
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.data, Vec::<u8>::new());
    }

    // ─── auki.pose locked vectors ────────────────────────────────────────────

    /// Identity-ish pose: 1m forward translation along +x, no rotation
    /// (unit quaternion `[0, 0, 0, 1]`). Plain integer-valued doubles
    /// keep the wire bytes stable and human-checkable.
    fn step5_spatial_transform() -> SpatialTransform {
        SpatialTransform {
            translation: Some(Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            }),
            orientation: Some(Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            }),
        }
    }

    /// Locks the prost wire bytes for the Step 5 example
    /// `SpatialTransform`. Cross-language readers MUST reproduce these
    /// exact bytes. Note: proto3 default-elision means zero-valued
    /// `double` fields don't appear on the wire — `Quat { x:0, y:0,
    /// z:0, w:1 }` encodes only its `w` field (9 bytes inside its
    /// length-delimited envelope).
    #[test]
    fn spatial_transform_serializes_to_locked_wire_bytes() {
        let bytes = step5_spatial_transform().encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "0a1b09000000000000f03f110000000000000040190000000000000840\
             120921000000000000f03f"
        );
    }

    /// XXH3-128 of those wire bytes — joins the workspace's locked
    /// conformance set.
    #[test]
    fn spatial_transform_hash_is_locked() {
        let bytes = step5_spatial_transform().encode_to_vec();
        assert_eq!(
            auki_hash::hash_jcs_bytes(&bytes),
            "29fa6349ab0b3ff1f06933489db74dfd"
        );
    }

    #[test]
    fn spatial_transform_round_trips() {
        let entry = step5_spatial_transform();
        let bytes = entry.encode_to_vec();
        let decoded = SpatialTransform::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// `LogPayload` round-trip — proves the macro-generated impl rides
    /// the same prost path as direct `Message::encode_to_vec` / `decode`.
    #[test]
    fn spatial_transform_log_payload_round_trips() {
        use auki_logs::LogPayload;
        let entry = step5_spatial_transform();
        let bytes = LogPayload::encode(&entry);
        let decoded = <SpatialTransform as LogPayload>::decode(&bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// Default-elided round-trip — a `SpatialTransform` with both
    /// nested fields `None` (proto3 message-typed fields are
    /// `Option<T>` in prost) encodes to zero bytes.
    #[test]
    fn spatial_transform_default_round_trips() {
        let entry = SpatialTransform {
            translation: None,
            orientation: None,
        };
        let bytes = entry.encode_to_vec();
        assert_eq!(bytes.len(), 0);
        let decoded = SpatialTransform::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// End-to-end seam: open a real `auki_logs::Log<SpatialTransform>`,
    /// append two entries (one populated, one default), close, re-read,
    /// assert order + payload byte-equality. Pins the flat-not-wrapped
    /// shape end-to-end through the segment writer/reader.
    #[test]
    fn spatial_transform_segment_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 60_000_000_000i64,
            "kind": "test"
        });
        {
            let mut log: auki_logs::Log<SpatialTransform> =
                auki_logs::Log::open(dir.path(), manifest).unwrap();
            log.append(100, &step5_spatial_transform()).unwrap();
            log.append(
                200,
                &SpatialTransform {
                    translation: None,
                    orientation: None,
                },
            )
            .unwrap();
        }
        let reader: auki_logs::LogReader<SpatialTransform> =
            auki_logs::Log::<SpatialTransform>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload, step5_spatial_transform());
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.translation, None);
        assert_eq!(entries[1].payload.orientation, None);
    }

    // ─── auki.time_transform locked vectors ──────────────────────────────────

    /// 1 ms positive offset, 250 ns uncertainty — round numbers picked
    /// for stable wire bytes; the values are within the realistic
    /// monotonic↔UTC offset range a daemon would see at startup.
    fn step6_time_transform_entry() -> TimeTransformEntry {
        TimeTransformEntry {
            offset_ns: 1_000_000,
            uncertainty_ns: 250,
        }
    }

    /// Locks the prost wire bytes for the Step 6 example
    /// `TimeTransformEntry`. Cross-language readers MUST reproduce
    /// these exact bytes.
    #[test]
    fn time_transform_entry_serializes_to_locked_wire_bytes() {
        let bytes = step6_time_transform_entry().encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        // Field 1 (offset_ns, varint): tag 0x08, value 1_000_000 → 3 bytes (`c0 84 3d`).
        // Field 2 (uncertainty_ns, varint): tag 0x10, value 250 → 2 bytes (`fa 01`).
        assert_eq!(hex, "08c0843d10fa01");
    }

    /// XXH3-128 of those wire bytes.
    #[test]
    fn time_transform_entry_hash_is_locked() {
        let bytes = step6_time_transform_entry().encode_to_vec();
        assert_eq!(
            auki_hash::hash_jcs_bytes(&bytes),
            "b7e73628833419a7c299933d07cbe88c"
        );
    }

    #[test]
    fn time_transform_entry_round_trips() {
        let entry = step6_time_transform_entry();
        let bytes = entry.encode_to_vec();
        let decoded = TimeTransformEntry::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    #[test]
    fn time_transform_entry_log_payload_round_trips() {
        use auki_logs::LogPayload;
        let entry = step6_time_transform_entry();
        let bytes = LogPayload::encode(&entry);
        let decoded = <TimeTransformEntry as LogPayload>::decode(&bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// proto3 default-elision: `offset_ns = 0` (perfectly synchronized
    /// clocks at the sample instant) and `uncertainty_ns = 0` encode
    /// to zero bytes.
    #[test]
    fn time_transform_entry_zero_offset_round_trips() {
        let entry = TimeTransformEntry {
            offset_ns: 0,
            uncertainty_ns: 0,
        };
        let bytes = entry.encode_to_vec();
        assert_eq!(bytes.len(), 0);
        let decoded = TimeTransformEntry::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// Negative-offset round-trip — if the to-clock leads the
    /// from-clock at the sample instant, `offset_ns` is negative.
    /// Pins zigzag/varint signed encoding behavior (proto3 `int64`
    /// is *not* zigzag-encoded; stays a regular varint with the
    /// 64-bit two's-complement representation, so negatives are
    /// 10-byte varints).
    #[test]
    fn time_transform_entry_negative_offset_round_trips() {
        let entry = TimeTransformEntry {
            offset_ns: -42_000_000,
            uncertainty_ns: 100,
        };
        let bytes = entry.encode_to_vec();
        let decoded = TimeTransformEntry::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// End-to-end seam: open a real `auki_logs::Log<TimeTransformEntry>`,
    /// append two entries (one populated, one default), close, re-read,
    /// assert order + payload byte-equality. Pins the new on-disk
    /// shape end-to-end through the segment writer/reader.
    #[test]
    fn time_transform_entry_segment_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 60_000_000_000i64,
            "kind": "test"
        });
        {
            let mut log: auki_logs::Log<TimeTransformEntry> =
                auki_logs::Log::open(dir.path(), manifest).unwrap();
            log.append(100, &step6_time_transform_entry()).unwrap();
            log.append(
                200,
                &TimeTransformEntry {
                    offset_ns: 0,
                    uncertainty_ns: 0,
                },
            )
            .unwrap();
        }
        let reader: auki_logs::LogReader<TimeTransformEntry> =
            auki_logs::Log::<TimeTransformEntry>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload, step6_time_transform_entry());
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.offset_ns, 0);
        assert_eq!(entries[1].payload.uncertainty_ns, 0);
    }

    // ─── auki.detection locked vectors ───────────────────────────────────────

    /// 12 bytes of deterministic content. Stands in for any Detector's
    /// per-frame detection payload — the SDK only sees opaque bytes, so
    /// the exact contents don't matter beyond reproducibility. The
    /// per-detector schema (QR corners, ESL bbox, person bbox, …) sits
    /// inside these bytes; the SDK doesn't decode it.
    fn step8_detection_frame() -> DetectionFrame {
        // Pre-Cuba shape: only `data` set; `sensor_hash` / `type` defaulted.
        // Proto3 elides default-valued fields on the wire, so the locked
        // hex below is unchanged across the Cuba field additions.
        DetectionFrame {
            data: (0..12u8).collect(),
            sensor_hash: String::new(),
            r#type: String::new(),
        }
    }

    /// Locks the prost wire bytes for the Step 8 example detection log
    /// entry. Cross-language readers MUST produce these exact bytes for
    /// the same input. Field 1 length-delimited: tag 0x0a, varint
    /// length 0x0c (12), then the 12 payload bytes.
    #[test]
    fn detection_frame_serializes_to_locked_wire_bytes() {
        let bytes = step8_detection_frame().encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(hex, "0a0c000102030405060708090a0b");
    }

    /// XXH3-128 of those wire bytes — joins the workspace's locked
    /// conformance set so future drift in either prost-build or
    /// auki-hash trips the test.
    #[test]
    fn detection_frame_hash_is_locked() {
        let bytes = step8_detection_frame().encode_to_vec();
        assert_eq!(
            auki_hash::hash_jcs_bytes(&bytes),
            "94f8efe6be63d3dc5e045ab08d538a15"
        );
    }

    #[test]
    fn detection_frame_round_trips() {
        let entry = step8_detection_frame();
        let bytes = entry.encode_to_vec();
        let decoded = DetectionFrame::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// `LogPayload` round-trip — proves the macro-generated impl rides
    /// the same prost path as direct `Message::encode_to_vec` /
    /// `decode` calls.
    #[test]
    fn detection_frame_log_payload_round_trips() {
        use auki_logs::LogPayload;
        let entry = step8_detection_frame();
        let bytes = LogPayload::encode(&entry);
        let decoded = <DetectionFrame as LogPayload>::decode(&bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// Empty payload — opaque-bytes-only is honest about empty: a frame
    /// with zero detections encodes to zero output bytes (proto3
    /// default-elision) and decodes back to the empty form. A Detector
    /// that runs and finds nothing on a frame still emits an entry —
    /// the framing layer's `timestamp_ns` is the "I looked at this
    /// frame and saw nothing" record; the entry's empty `data` is the
    /// per-detector schema's choice of how to express that.
    #[test]
    fn detection_frame_empty_data_round_trips() {
        let entry = DetectionFrame {
            data: vec![],
            sensor_hash: String::new(),
            r#type: String::new(),
        };
        let bytes = entry.encode_to_vec();
        assert_eq!(bytes.len(), 0);
        let decoded = DetectionFrame::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// Cuba T5 + T12 — sensor_hash and type round-trip when populated.
    /// Locked-bytes test for `step8_detection_frame` proves the
    /// pre-Cuba empty-string defaults are wire-elided; this test proves
    /// populated values survive a full encode/decode cycle.
    #[test]
    fn detection_frame_cuba_fields_round_trip() {
        let entry = DetectionFrame {
            data: vec![0xAA, 0xBB],
            sensor_hash: "abcdef0123456789".to_string(),
            r#type: "aruco".to_string(),
        };
        let bytes = entry.encode_to_vec();
        let decoded = DetectionFrame::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
        assert_eq!(decoded.sensor_hash, "abcdef0123456789");
        assert_eq!(decoded.r#type, "aruco");
    }

    /// End-to-end seam test: open a real `auki_logs::Log<DetectionFrame>`,
    /// append two entries (one populated, one empty), close, re-read,
    /// assert order + payload byte-equality. Catches any regression in the
    /// `LogPayload` macro wiring or the segment-framing path.
    #[test]
    fn detection_frame_segment_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 60_000_000_000i64,
            "kind": "test"
        });
        {
            let mut log: auki_logs::Log<DetectionFrame> =
                auki_logs::Log::open(dir.path(), manifest).unwrap();
            log.append(100, &step8_detection_frame()).unwrap();
            log.append(
                200,
                &DetectionFrame {
                    data: vec![],
                    sensor_hash: String::new(),
                    r#type: String::new(),
                },
            )
            .unwrap();
        }
        let reader: auki_logs::LogReader<DetectionFrame> =
            auki_logs::Log::<DetectionFrame>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload, step8_detection_frame());
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.data, Vec::<u8>::new());
    }

    // ─── auki.joint_encoders locked vectors ──────────────────────────────────

    /// 6-DOF arm fixture, integer-valued radians for stable wire bytes.
    /// Joint ordering is producer-defined; this fixture pins one valid
    /// ordering so cross-language readers reproduce the byte-equal
    /// encoding.
    fn step_joint_encoders_data() -> joint_encoders::Data {
        joint_encoders::Data {
            angles_rad: vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
        }
    }

    /// Locks the prost wire bytes for the example joint-encoders
    /// payload. Cross-language readers (Python via betterproto, future
    /// boosterapp Python) MUST produce these exact bytes for the same
    /// input. Field 1 packed-repeated float: tag 0x0a, varint length
    /// 0x18 (24 bytes = 6 × 4), then 6 little-endian f32s. Same bytes
    /// whether the payload travels on disk (Sensor Log segment) or on
    /// the wire (`/auki/stream/0.1.0` substream).
    #[test]
    fn joint_encoders_data_serializes_to_locked_wire_bytes() {
        let bytes = step_joint_encoders_data().encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(hex, "0a18000000000000803f0000004000004040000080400000a040");
    }

    /// XXH3-128 of those wire bytes — joins the workspace's locked
    /// conformance set so future drift in either prost-build or
    /// auki-hash trips the test.
    #[test]
    fn joint_encoders_data_hash_is_locked() {
        let bytes = step_joint_encoders_data().encode_to_vec();
        assert_eq!(
            auki_hash::hash_jcs_bytes(&bytes),
            "150a56272692540cf5d8e8e93dc74b7a"
        );
    }

    #[test]
    fn joint_encoders_data_round_trips() {
        let entry = step_joint_encoders_data();
        let bytes = entry.encode_to_vec();
        let decoded = joint_encoders::Data::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// `LogPayload` round-trip — proves the macro-generated impl rides
    /// the same prost path as direct `Message::encode_to_vec` /
    /// `decode` calls.
    #[test]
    fn joint_encoders_data_log_payload_round_trips() {
        use auki_logs::LogPayload;
        let entry = step_joint_encoders_data();
        let bytes = LogPayload::encode(&entry);
        let decoded = <joint_encoders::Data as LogPayload>::decode(&bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// Empty angle vector — proto3 default-elision: a packed-repeated
    /// float field with no elements encodes to zero bytes.
    #[test]
    fn joint_encoders_data_empty_round_trips() {
        let entry = joint_encoders::Data { angles_rad: vec![] };
        let bytes = entry.encode_to_vec();
        assert_eq!(bytes.len(), 0);
        let decoded = joint_encoders::Data::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// End-to-end seam test: open a real
    /// `auki_logs::Log<joint_encoders::Data>`, append two entries
    /// (one populated, one empty), close, re-read, assert order +
    /// payload byte-equality. Catches any regression in the
    /// `LogPayload` macro wiring or the segment-framing path.
    #[test]
    fn joint_encoders_data_segment_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 60_000_000_000i64,
            "kind": "test"
        });
        {
            let mut log: auki_logs::Log<joint_encoders::Data> =
                auki_logs::Log::open(dir.path(), manifest).unwrap();
            log.append(100, &step_joint_encoders_data()).unwrap();
            log.append(200, &joint_encoders::Data { angles_rad: vec![] })
                .unwrap();
        }
        let reader: auki_logs::LogReader<joint_encoders::Data> =
            auki_logs::Log::<joint_encoders::Data>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload, step_joint_encoders_data());
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.angles_rad, Vec::<f32>::new());
    }
}
