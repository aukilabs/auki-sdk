//! Clean libp2p runtime for the RFC-first Auki peer-to-peer protocol.
//!
//! This crate is libp2p-shaped from the boundary inward. Pure policy and
//! validation helpers live here only when they need runtime configuration or
//! libp2p context; protocol truth remains in `auki-protocol`.

pub mod config;
pub mod handshake_policy;
pub mod identity;
pub mod lifecycle;
pub mod node;
pub mod offer_loading;
pub mod protocols;
pub mod relationship;

pub use config::{
    AukiP2pConfig, AuthorityDeadlineConfig, ConfigError, ConfiguredPeer, DialPolicy,
    DialPolicyError, DomainAccessPolicy, OfferPolicy, PeerAdmissionConfig,
    PeerBindingFreshnessConfig, RuntimeLimits, StatusPrivacyConfig,
};
pub use handshake_policy::{
    AppDomainAccess, HandshakeFailureDiagnostic, HandshakeFailureScope, HandshakeLifecycleState,
    HandshakeMetadataField, HandshakePolicyError, HandshakeValidationInput,
    HandshakeValidationResult, PolicyRejectedDomain, build_local_handshake,
    validate_remote_handshake,
};
pub use identity::{LocalPeerIdentity, LocalPeerIdentityError, PEER_DERIVATION_LABEL};
pub use lifecycle::{
    LifecycleHandshakeExchange, LifecycleProtocolError, accept_lifecycle_streams,
    build_local_peer_handshake, exchange_peer_handshake, open_lifecycle_stream,
    read_peer_handshake, write_peer_handshake,
};
pub use node::{AukiP2pEvent, AukiP2pNode, AukiP2pNodeConfig, AukiP2pNodeError};
pub use offer_loading::{
    AppAllowedOffer, AppOfferPolicy, LoadedRemoteOffer, OfferCatalogClient,
    OfferCatalogClientError, OfferLoadContext, OfferLoadError, OfferLoadReport, OfferLookupError,
    load_remote_offers_from_frame, load_remote_offers_with_client,
};
pub use relationship::{
    OfferCatalogLoadState, PeerRelationship, PeerRelationshipState, RelationshipFailureRecord,
    RelationshipFailureScope, RelationshipLoadedOffer, RelationshipPathStatus,
    RelationshipRegistryReferenceStatus, RelationshipRejectedDomain, RelationshipStatusBuildError,
    RelationshipStatusOptions, build_relationship_status_snapshot,
};
