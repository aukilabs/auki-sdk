//! Clean libp2p runtime for the RFC-first Auki peer-to-peer protocol.
//!
//! This crate is libp2p-shaped from the boundary inward. Pure policy and
//! validation helpers live here only when they need runtime configuration or
//! libp2p context; protocol truth remains in `auki-protocol`.

pub mod api;
pub mod config;
pub mod get_serving;
pub mod handshake_policy;
pub mod identity;
pub mod lifecycle;
pub mod node;
pub mod offer_catalog_streams;
pub mod offer_loading;
pub mod path_streams;
pub mod paths;
pub mod protocols;
pub mod publication;
pub mod relationship;
pub mod subscribe_serving;
pub mod transport_path;

pub use api::{
    AukiGetProvider, AukiGetProviderError, AukiNode, AukiNodeError, AukiNodeEvent,
    AukiServedSubscription, AukiSubscribeProvider, AukiSubscribeProviderAccept,
    AukiSubscribeProviderError, AukiSubscription, LifecycleDomainAccess, LifecycleInput,
    LocalDomainRegistration, RemoteAllowedOffer, RemoteOfferAppPolicy, RemoteOfferLoadInput,
    ServedGet, ServedOfferCatalog, ServedSubscribe,
};
pub use config::{
    AukiP2pConfig, AuthorityDeadlineConfig, ConfigError, ConfiguredPeer, DialPolicy,
    DialPolicyError, DomainAccessPolicy, OfferPolicy, PeerAdmissionConfig,
    PeerBindingFreshnessConfig, RuntimeLimits, StatusPrivacyConfig,
};
pub use get_serving::{GetServeError, accept_get_streams, read_get_request, write_get_response};
pub use handshake_policy::{
    AppDomainAccess, HandshakeFailureDiagnostic, HandshakeFailureScope, HandshakeLifecycleState,
    HandshakeMetadataField, HandshakePolicyError, HandshakeValidationInput,
    HandshakeValidationResult, PolicyRejectedDomain, build_local_handshake,
    validate_remote_handshake,
};
pub use identity::{LocalPeerIdentity, LocalPeerIdentityError, PEER_DERIVATION_LABEL};
pub use lifecycle::{
    LifecycleHandshakeExchange, LifecycleOpenStreamError, LifecycleProtocolError,
    LifecycleStreamDirection, LifecycleStreamGuard, LifecycleStreamGuardError,
    accept_lifecycle_streams, build_local_peer_handshake, exchange_peer_handshake,
    exchange_peer_handshake_strict, open_lifecycle_stream, open_lifecycle_stream_once,
    read_peer_handshake, read_peer_handshake_strict, write_peer_handshake,
};
pub use node::{
    AukiBrowserBootstrapRecord, AukiP2pEvent, AukiP2pNode, AukiP2pNodeConfig, AukiP2pNodeError,
    BrowserWebRtcDirectConfig, RelayServerConfig, loopback_webrtc_direct_listen_addr,
    loopback_websocket_relay_listen_addr,
};
pub use offer_catalog_streams::{
    Libp2pOfferCatalogClient, OfferCatalogServeError, accept_offer_catalog_streams,
    load_remote_offers_over_libp2p, serve_offer_catalog_response,
};
pub use offer_loading::{
    AppAllowedOffer, AppOfferPolicy, LoadedRemoteOffer, OfferCatalogClient,
    OfferCatalogClientError, OfferLoadContext, OfferLoadError, OfferLoadReport, OfferLookupError,
    load_remote_offers_from_frame, load_remote_offers_with_client,
};
pub use path_streams::{
    Libp2pPathClient, Libp2pSubscription, get_over_libp2p, subscribe_over_libp2p,
};
pub use paths::{
    GetClient, GetInput, GetOutcome, PathClientError, PathContext, PathOrchestrationError,
    SubscribeClient, SubscribeInput, SubscriptionHandle, accept_subscribe_data_frame,
    end_subscription_from_frame, get, subscribe,
};
pub use publication::{
    PublicationMessageError, PublishOfferError, PublishOfferInput, PublishedByteSource,
    PublishedByteSourceFactory, PublishedOfferHandle, ServedPublishedSubscription,
};
pub use relationship::{
    OfferCatalogLoadState, PeerRelationship, PeerRelationshipState, RelationshipFailureRecord,
    RelationshipFailureScope, RelationshipLoadedOffer, RelationshipPathStatus,
    RelationshipRegistryReferenceStatus, RelationshipRejectedDomain, RelationshipStatusBuildError,
    RelationshipStatusOptions, build_relationship_status_snapshot,
};
pub use subscribe_serving::{
    EncodedSubscribeFrame, SubscribeServeError, accept_subscribe_streams, close_subscribe_stream,
    encode_subscribe_data_frame, read_subscribe_request, write_encoded_subscribe_frame,
    write_subscribe_end, write_subscribe_start_result,
};
pub use transport_path::{AukiConnectionDirection, AukiConnectionPath, AukiTransportProtocol};
