//! UniFFI Swift bindings for `auki-domain`.
//!
//! ## Scope (v0 — PR C)
//!
//! Full ClusterManager surface for native iOS / Swift consumers, with
//! parity to `bindings/python/auki-domain-py` plus the upstream-only
//! methods (clock sync, diagnostics) explicitly included per the design
//! spec.
//!
//! See `README.md` for the full API surface description and `src/readme.md`
//! for the implementation breakdown.

use libp2p_identity::PeerId;
use multiaddr::Multiaddr;

uniffi::setup_scaffolding!();

// ─── Custom-type registrations ─────────────────────────────────────
//
// PeerId and Multiaddr custom_type! declarations live in auki-network-swift
// too. UniFFI generates per-crate FfiConverter impls anchored on each
// crate's UniFfiTag — since this crate has its own UniFfiTag, we need our
// own custom_type registrations (with `remote` keyword for foreign types).

uniffi::custom_type!(PeerId, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<PeerId>()
            .map_err(|e| anyhow::anyhow!("invalid peer-id {s:?}: {e}"))
    },
    lower: |p: PeerId| p.to_string(),
});

uniffi::custom_type!(Multiaddr, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<Multiaddr>()
            .map_err(|e| anyhow::anyhow!("invalid multiaddr {s:?}: {e}"))
    },
    lower: |m: Multiaddr| m.to_string(),
});

// Subsequent tasks (5+) add upstream type re-exports, orchestrator
// functions (bootstrap_swift / create_cluster_swift / join_cluster_swift),
// and additional Swift-side adapters.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_entry_variants_construct() {
        use auki_network::resources_protocol::{
            ResourceEntry, ResourcePinholeIntrinsics, ResourceQuat, ResourceSpatialTransform,
            ResourceVec3, SensorStreamResource,
        };
        // Construct a SensorStream variant with explicit field values
        // (SensorStreamResource does not impl Default due to required String fields).
        let stream_resource = ResourceEntry::SensorStream(SensorStreamResource {
            id: "test-id".to_string(),
            sensor_id: "test-sensor".to_string(),
            sensor_hash: "abc123".to_string(),
            sensor_kind: "camera".to_string(),
            stream_protocol: "/auki/stream/0.1.0".to_string(),
            payload: "camera_frame".to_string(),
            pinhole_intrinsics: Some(ResourcePinholeIntrinsics {
                fx: 400.0,
                fy: 401.0,
                cx: 272.5,
                cy: 244.5,
            }),
            sensor_entry_json: None,
            frame_entry_json: None,
        });
        assert!(matches!(stream_resource, ResourceEntry::SensorStream(_)));

        // Also verify TransformEdge variant constructs with source as Option<String>.
        use auki_network::resources_protocol::TransformEdgeResource;
        let edge_resource = ResourceEntry::TransformEdge(TransformEdgeResource {
            id: "frame_a->frame_b".to_string(),
            from_frame_id: "frame_a".to_string(),
            from_frame_hash: "fromhash".to_string(),
            to_frame_id: "frame_b".to_string(),
            to_frame_hash: "tohash".to_string(),
            writer_mode: "rigid".to_string(),
            source: Some(r#"{"kind":"ros2_tf"}"#.to_string()),
            transform: ResourceSpatialTransform {
                translation: ResourceVec3 { x: 0.0, y: 0.0, z: 0.0 },
                orientation: ResourceQuat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },
            },
            from_frame_entry_json: None,
            to_frame_entry_json: None,
        });
        assert!(matches!(edge_resource, ResourceEntry::TransformEdge(_)));
    }

    #[test]
    fn peer_id_custom_type_round_trips() {
        let pid = libp2p_identity::Keypair::ed25519_from_bytes([5u8; 32])
            .expect("valid ed25519 seed")
            .public()
            .to_peer_id();
        let s = pid.to_string();
        let back: PeerId = s.parse().expect("canonical PeerId string parses");
        assert_eq!(back, pid);
    }

    #[test]
    fn multiaddr_custom_type_round_trips() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        assert_eq!(addr.to_string().parse::<Multiaddr>().unwrap(), addr);
    }

    #[test]
    fn daemon_info_record_constructs() {
        let info = auki_domain_rs::cluster_manager::DaemonInfo {
            app: "test-app".to_string(),
            name: "test-name".to_string(),
            session_id: "session-1".to_string(),
            session_clock_id: "clock-1".to_string(),
            session_clock_hash: "hash-1".to_string(),
            app_instance: "instance-1".to_string(),
        };
        assert_eq!(info.app, "test-app");
    }

    #[test]
    fn sensor_registry_entry_camera_variant() {
        use auki_registry::{Camera, SensorBody, SensorRegistryEntry};
        // Camera has no Default derive — construct with explicit field values.
        let camera = Camera {
            width: 640,
            height: 480,
            frame_rate_hz: 30,
            pixel_format: "YUV_NV12".to_string(),
            color_space: "BT.709".to_string(),
            intrinsics_model: "pinhole".to_string(),
            distortion_model: "plumb_bob".to_string(),
            frame_id: "robot/cam_optical".to_string(),
            frame_hash: "000000000000000000000000deadbeef".to_string(),
        };
        let entry = SensorRegistryEntry {
            sensor_id: "cam-1".to_string(),
            body: SensorBody::Camera(camera),
        };
        assert!(matches!(entry.body, SensorBody::Camera(_)));
    }
}
