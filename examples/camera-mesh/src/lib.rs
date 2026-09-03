//! Shared Camera Mesh application contract and standard-protocol composition.
//!
//! The crate intentionally contains application policy rather than a seventh
//! Auki protocol. Native hosts use it directly; language bindings reproduce
//! the locked wire vectors from [`contract`].

#![forbid(unsafe_code)]

pub mod contract;
mod protocols;

pub use contract::{
    APP, APP_VERSION, CAMERA_CONTROL_RESOURCE_ID, CAMERA_HEIGHT, CAMERA_RATE_HZ,
    CAMERA_REPLY_RESOURCE_ID, CAMERA_RESOURCE_ID, CAMERA_WIDTH, CameraMetadata, CameraRole,
    MAX_BLOB_BYTES, MessageChannelWire, PeerCard, PeerRoutes, SnapshotReadyPayload,
    SnapshotReplyAddress, SnapshotReplyTarget, SnapshotRequestPayload, camera_catalog,
    control_channel, decode_snapshot_ready, decode_snapshot_request, deterministic_jpeg,
    encode_snapshot_ready, encode_snapshot_request, metadata, protocol_ids, protocol_ids_for_role,
    reply_channel, sha256_hex, stream_manifest, synthetic_jpegs,
};
pub use protocols::{
    CameraEvent, CameraProtocols, DiscoveryPeer, RemoteCamera, SnapshotReport, ViewReport,
};
