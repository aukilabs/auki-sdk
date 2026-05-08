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

/// Placeholder package — pipeline-check only. Removed once the placeholder
/// is no longer the only proof that `prost-build` ran (Step 7 of the
/// migration in [`src/sprint.md`](sprint.md)).
pub mod placeholder {
    include!(concat!(env!("OUT_DIR"), "/auki.placeholder.rs"));
}

/// `auki.camera` — Pinhole camera log payload (Sensor Log family).
/// Migration Step 1.
pub mod camera {
    include!(concat!(env!("OUT_DIR"), "/auki.camera.rs"));
}

impl_log_payload!(camera::PinholeCameraLogEntry);

#[cfg(test)]
mod tests {
    use super::camera::{DynamicIntrinsics, PinholeCameraLogEntry};
    use super::placeholder::PipelineCheck;
    use prost::Message;

    /// Smoke test that `prost-build` actually ran, the generated code
    /// compiled, and the encode/decode round-trip works. When the
    /// placeholder gets removed, this test goes with it — the real
    /// schemas have their own locked conformance vectors.
    #[test]
    fn placeholder_pipeline_check_round_trips() {
        let msg = PipelineCheck::default();
        let bytes = msg.encode_to_vec();
        let decoded = PipelineCheck::decode(&*bytes).expect("decode");
        assert_eq!(msg, decoded);
    }

    // ─── auki.camera locked vectors ──────────────────────────────────────────

    fn m1_pinhole_camera_log_entry() -> PinholeCameraLogEntry {
        PinholeCameraLogEntry {
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
    fn pinhole_camera_log_entry_serializes_to_locked_wire_bytes() {
        let bytes = m1_pinhole_camera_log_entry().encode_to_vec();
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
    fn pinhole_camera_log_entry_hash_is_locked() {
        let bytes = m1_pinhole_camera_log_entry().encode_to_vec();
        assert_eq!(
            auki_hash::hash_jcs_bytes(&bytes),
            "0496e1f71a03e00877fc68bf16190026"
        );
    }

    #[test]
    fn pinhole_camera_log_entry_round_trips() {
        let entry = m1_pinhole_camera_log_entry();
        let bytes = entry.encode_to_vec();
        let decoded = PinholeCameraLogEntry::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// `LogPayload` round-trip — proves the macro-generated impl rides
    /// the same prost path as direct `Message::encode_to_vec` /
    /// `decode` calls. Locks the trait wiring against accidental
    /// regression.
    #[test]
    fn pinhole_camera_log_entry_log_payload_round_trips() {
        use auki_logs::LogPayload;
        let entry = m1_pinhole_camera_log_entry();
        let bytes = LogPayload::encode(&entry);
        let decoded = <PinholeCameraLogEntry as LogPayload>::decode(&bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// Absent-intrinsics case for non-autofocusing cameras — the inline-
    /// optional encoding pays only the message-tag overhead when
    /// `dynamic_intrinsics` is `None`.
    #[test]
    fn pinhole_camera_log_entry_without_intrinsics_round_trips() {
        let entry = PinholeCameraLogEntry {
            dynamic_intrinsics: None,
            frame: vec![0xff; 16],
        };
        let bytes = entry.encode_to_vec();
        let decoded = PinholeCameraLogEntry::decode(&*bytes).expect("decode");
        assert_eq!(decoded, entry);
    }

    /// End-to-end seam test: open a real `auki_logs::Log<PinholeCameraLogEntry>`,
    /// append two entries (one with intrinsics, one without), close, re-read,
    /// assert order + payload byte-equality. Catches any regression in the
    /// `LogPayload` macro wiring or the segment-framing path.
    #[test]
    fn pinhole_camera_log_entry_segment_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 60_000_000_000i64,
            "kind": "test"
        });
        {
            let mut log: auki_logs::Log<PinholeCameraLogEntry> =
                auki_logs::Log::open(dir.path(), manifest).unwrap();
            log.append(100, &m1_pinhole_camera_log_entry()).unwrap();
            log.append(
                200,
                &PinholeCameraLogEntry {
                    dynamic_intrinsics: None,
                    frame: vec![0xab; 8],
                },
            )
            .unwrap();
        }
        let reader: auki_logs::LogReader<PinholeCameraLogEntry> =
            auki_logs::Log::<PinholeCameraLogEntry>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload, m1_pinhole_camera_log_entry());
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.dynamic_intrinsics, None);
        assert_eq!(entries[1].payload.frame, vec![0xab; 8]);
    }
}
