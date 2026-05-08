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
//! ## What's defined here today
//!
//! Sawslin Phase 1 Lane 0 (PR B) pulled three packages forward in the
//! migration sequence — see [`src/sprint.md`](sprint.md) for the
//! reordering rationale:
//!
//! - [`auki.pose`](self::pose) — `Vec3`, `Quat`, `SpatialTransform`.
//!   The canonical 6-DoF transform shape; same payload on disk (Pose
//!   Log entry) and on the wire (sentinel's per-marker pose substream
//!   from sawslin Phase 3).
//! - [`auki.joint_state`](self::joint_state) — `JointAngles`. The
//!   per-frame angle vector for an articulated-joint sensor; same
//!   payload on disk (Sensor Log for `SensorBody::JointState`) and on
//!   the wire (boosterapp's PoseStream from sawslin Phase 1).
//! - [`auki.pose_stream`](self::pose_stream) — `PoseStreamFrame`. The
//!   `oneof` envelope flowing over the libp2p `AcceptPoseStream`
//!   substream; carries either `JointAngles` or `SpatialTransform`
//!   per frame, per [sawslin locked decision #7].

#![allow(missing_docs, clippy::derive_partial_eq_without_eq)]

/// `auki.joint_state` — articulated-joint angle vector. See the
/// crate-level docs for context.
pub mod joint_state {
    include!(concat!(env!("OUT_DIR"), "/auki.joint_state.rs"));
}

/// `auki.pose` — `Vec3` / `Quat` / `SpatialTransform` 6-DoF transform
/// primitives.
pub mod pose {
    include!(concat!(env!("OUT_DIR"), "/auki.pose.rs"));
}

/// `auki.pose_stream` — `PoseStreamFrame` `oneof` envelope for the
/// libp2p `AcceptPoseStream` substream.
pub mod pose_stream {
    include!(concat!(env!("OUT_DIR"), "/auki.pose_stream.rs"));
}

#[cfg(test)]
mod tests {
    use super::joint_state::JointAngles;
    use super::pose::{Quat, SpatialTransform, Vec3};
    use super::pose_stream::{PoseStreamFrame, pose_stream_frame::Payload};
    use prost::Message;

    // ─── Round-trip smoke tests ──────────────────────────────────────────

    #[test]
    fn joint_angles_round_trips() {
        let msg = JointAngles {
            angles: vec![0.0, 0.5, -0.5, 1.0, -1.0],
        };
        let bytes = msg.encode_to_vec();
        let decoded = JointAngles::decode(&*bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn spatial_transform_round_trips() {
        let msg = SpatialTransform {
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
        };
        let bytes = msg.encode_to_vec();
        let decoded = SpatialTransform::decode(&*bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn pose_stream_frame_round_trips_joint_angles_arm() {
        let inner = JointAngles {
            angles: vec![0.1, 0.2, 0.3],
        };
        let env = PoseStreamFrame {
            payload: Some(Payload::JointAngles(inner.clone())),
        };
        let bytes = env.encode_to_vec();
        let decoded = PoseStreamFrame::decode(&*bytes).unwrap();
        assert_eq!(env, decoded);
        match decoded.payload {
            Some(Payload::JointAngles(j)) => assert_eq!(j, inner),
            other => panic!("expected JointAngles arm, got {other:?}"),
        }
    }

    #[test]
    fn pose_stream_frame_round_trips_spatial_transform_arm() {
        let inner = SpatialTransform {
            translation: Some(Vec3 {
                x: -0.5,
                y: 0.25,
                z: 1.75,
            }),
            orientation: Some(Quat {
                x: 0.0,
                y: 0.7071068,
                z: 0.0,
                w: 0.7071068,
            }),
        };
        let env = PoseStreamFrame {
            payload: Some(Payload::SpatialTransform(inner.clone())),
        };
        let bytes = env.encode_to_vec();
        let decoded = PoseStreamFrame::decode(&*bytes).unwrap();
        assert_eq!(env, decoded);
        match decoded.payload {
            Some(Payload::SpatialTransform(t)) => assert_eq!(t, inner),
            other => panic!("expected SpatialTransform arm, got {other:?}"),
        }
    }

    #[test]
    fn empty_pose_stream_frame_round_trips_as_neither_arm_set() {
        // Wire-level shape: an envelope with no arm set decodes to a
        // `payload: None`. Treating that as "malformed" is a consumer
        // policy decision; the wire layer round-trips it cleanly.
        let env = PoseStreamFrame { payload: None };
        let bytes = env.encode_to_vec();
        let decoded = PoseStreamFrame::decode(&*bytes).unwrap();
        assert_eq!(env, decoded);
        assert!(decoded.payload.is_none());
    }

    // ─── Locked cross-language conformance vectors ───────────────────────
    //
    // These pin specific message → wire-byte pairings. Any
    // reimplementation in another language (Python via betterproto,
    // future Sentinel ports) must reproduce these exact bytes from the
    // same input. If any of these trips, the `.proto` schema changed —
    // coordinate the bump with every consumer before updating.
    //
    // Hex strings produced by `cargo test print_locked_pose_vectors --
    // --nocapture` (a debug helper kept around for future schema
    // evolution; see the test below).

    /// `JointAngles { angles: [0.0, 0.5, -0.5, 1.0, -1.0] }` —
    /// 5 IEEE-754 little-endian f32s packed as a `repeated float`
    /// field with packed encoding (the proto3 default).
    #[test]
    fn locked_joint_angles_wire_bytes() {
        let msg = JointAngles {
            angles: vec![0.0, 0.5, -0.5, 1.0, -1.0],
        };
        let bytes = msg.encode_to_vec();
        assert_eq!(
            hex::encode(&bytes),
            "0a14000000000000003f000000bf0000803f000080bf",
            "joint_angles wire bytes drifted — see crate docs for the locked recipe"
        );
    }

    /// `SpatialTransform { Vec3{1,2,3}, Quat{0,0,0,1} }`. Pins the
    /// nested-message encoding — any field-number renumber on Vec3 /
    /// Quat / SpatialTransform trips this. Note: proto3 omits
    /// default-zero fields, so the Quat encodes only `w` (the three
    /// zero components are absent from the wire).
    #[test]
    fn locked_spatial_transform_wire_bytes() {
        let msg = SpatialTransform {
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
        };
        let bytes = msg.encode_to_vec();
        assert_eq!(
            hex::encode(&bytes),
            "0a0f0d0000803f15000000401d000040401205250000803f",
            "spatial_transform wire bytes drifted — see crate docs for the locked recipe"
        );
    }

    /// `PoseStreamFrame { JointAngles{[0.1, 0.2, 0.3]} }` — the
    /// joint-arm of the envelope. Pins the oneof field-number 1.
    #[test]
    fn locked_pose_stream_frame_joint_angles_arm_wire_bytes() {
        let env = PoseStreamFrame {
            payload: Some(Payload::JointAngles(JointAngles {
                angles: vec![0.1, 0.2, 0.3],
            })),
        };
        let bytes = env.encode_to_vec();
        assert_eq!(
            hex::encode(&bytes),
            "0a0e0a0ccdcccc3dcdcc4c3e9a99993e",
            "pose_stream_frame joint_angles arm wire bytes drifted"
        );
    }

    /// `PoseStreamFrame { SpatialTransform{Vec3{1,2,3}, Quat{0,0,0,1}} }` —
    /// the spatial-transform arm of the envelope, with a non-degenerate
    /// translation so this exercises Vec3's field encoding. Pins the
    /// oneof field-number 2.
    #[test]
    fn locked_pose_stream_frame_spatial_transform_arm_wire_bytes() {
        let env = PoseStreamFrame {
            payload: Some(Payload::SpatialTransform(SpatialTransform {
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
            })),
        };
        let bytes = env.encode_to_vec();
        assert_eq!(
            hex::encode(&bytes),
            "12180a0f0d0000803f15000000401d000040401205250000803f",
            "pose_stream_frame spatial_transform arm wire bytes drifted"
        );
    }

    /// Debug helper — prints the hex of each locked vector. Useful
    /// when intentionally evolving a schema and the locked tests need
    /// updated values. Always passes; run with
    /// `cargo test print_locked_pose_vectors -- --nocapture`.
    #[test]
    fn print_locked_pose_vectors() {
        let ja = JointAngles {
            angles: vec![0.0, 0.5, -0.5, 1.0, -1.0],
        };
        eprintln!("JointAngles vector hex: {}", hex::encode(ja.encode_to_vec()));

        let st = SpatialTransform {
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
        };
        eprintln!(
            "SpatialTransform vector hex: {}",
            hex::encode(st.encode_to_vec())
        );

        let env_ja = PoseStreamFrame {
            payload: Some(Payload::JointAngles(JointAngles {
                angles: vec![0.1, 0.2, 0.3],
            })),
        };
        eprintln!(
            "PoseStreamFrame{{JointAngles}} vector hex: {}",
            hex::encode(env_ja.encode_to_vec())
        );

        let env_st = PoseStreamFrame {
            payload: Some(Payload::SpatialTransform(SpatialTransform {
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
            })),
        };
        eprintln!(
            "PoseStreamFrame{{SpatialTransform}} vector hex: {}",
            hex::encode(env_st.encode_to_vec())
        );
    }
}
