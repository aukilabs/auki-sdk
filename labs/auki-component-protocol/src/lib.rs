//! Portable AukiPeer protocol family for typed Components.
//!
//! This crate is an application-protocol layer over `AukiPeerProtocols`. It
//! does not depend on the manager-era `auki-protocols` crate and it does not
//! change the local contracts owned by [`auki_components`].

#![forbid(unsafe_code)]

mod endpoint;
mod subscription;
mod wire;

pub use subscription::{RemoteObservationEvent, RemoteProductSubscription};

pub use endpoint::{
    ComponentProtocolClient, ComponentProtocolEndpoint, ComponentProtocolError,
    ComponentProtocolOperation, RemoteMirrorStart, RemoteObservations, RemoteProductMirror,
    RemoteProductSync,
};
pub use wire::{
    CATALOG_PROTOCOL_ID, CatalogRequest, CatalogResponse, MAX_CONTROL_FRAME_BYTES,
    MAX_PAYLOAD_FRAME_BYTES, OBSERVATION_STREAM_PROTOCOL_ID, OBSERVATIONS_PROTOCOL_ID,
    OPERATIONS_PROTOCOL_ID, ObservationRequest, ObservationSelection, ObservationStart,
    ObservationSubscriptionRequest, OperationRequest, RemoteOperationError, SourceGap,
};
