//! Shared P2P identity and DDS mutual-authentication core, plus the native
//! libp2p runtime.
//!
//! The identity, token verification, session requirements, and bounded
//! authentication conversation compile for native and browser Wasm. The
//! current TCP/Noise/Yamux transport remains native-only. This crate does not
//! fetch credentials or understand tasks, datasets, or machine-auth flows.

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod authentication;
#[cfg(not(target_arch = "wasm32"))]
mod authority;
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
mod browser_transport;
mod error;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod identity;
#[cfg(not(target_arch = "wasm32"))]
mod identity_store;
#[cfg(not(target_arch = "wasm32"))]
mod observation;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod relay;
#[cfg(not(target_arch = "wasm32"))]
mod relay_client;
#[cfg(not(target_arch = "wasm32"))]
mod routing;
#[cfg(not(target_arch = "wasm32"))]
mod runtime;
#[cfg(not(target_arch = "wasm32"))]
mod source_admission;
#[cfg(not(target_arch = "wasm32"))]
mod targeted_stream;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod token;
#[cfg(not(target_arch = "wasm32"))]
mod transport;

pub use authentication::{AuthenticatedPeer, SessionRequirements};
#[cfg(not(target_arch = "wasm32"))]
pub use authority::{DomainAuthority, P2pCredentialError, P2pCredentialResult};
pub use error::{Error, Result};
pub use identity::{Identity, PeerIdentityProof};
#[cfg(not(target_arch = "wasm32"))]
pub use libp2p::{multiaddr::Protocol, swarm::ConnectionId, Multiaddr};
pub use libp2p_identity::PeerId;
#[cfg(not(target_arch = "wasm32"))]
pub use observation::{
    AuthenticatedPeerObservation, NodeFailure, NodeObservationEvent, NodeObservationSnapshot,
    NodeObservationStatus, NodeObservations, PeerDisappearanceReason,
    PEER_OBSERVATION_CHANNEL_CAPACITY,
};
pub use relay::{
    ExpectedRelayLimits, RelayBaseTransport, RelayConfirmationRejection, RelayProvider,
    RelayReservationError, RelayReservationHandle, RelayReservationSnapshot, RelayReservationState,
    ReservationGeneration,
};
#[cfg(not(target_arch = "wasm32"))]
pub use routing::{
    canonicalize_circuit_route, validate_direct_route, CanonicalCircuitRoute, ConfirmedRoute,
    PublishedRoute, RouteCatalog, RouteCatalogError, RouteCatalogLimits, RouteCatalogResult,
    RouteCatalogStatus, RouteFence, RouteSnapshot,
};
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::{AuthenticatedRouteStream, ExactRoute, ProtocolServer, ProtocolSpec};
#[cfg(not(target_arch = "wasm32"))]
pub use targeted_stream::TargetedStreamError;
pub use token::{
    DdsTokenVerifier, DdsVerificationKeys, P2PAccessClaims, PeerRole, SignedApplicationMetadata,
    SignedP2pCredential, DDS_PREVIOUS_KEY_MIN_OVERLAP, DDS_VERIFICATION_KEYS_MAX_STALENESS,
    DDS_VERIFICATION_KEY_MAX_BYTES, DOMAIN_SERVER_MAX_DOMAINS, P2P_TOKEN_AUDIENCE,
    P2P_TOKEN_CLOCK_SKEW, P2P_TOKEN_ISSUER, P2P_TOKEN_MAX_APPLICATION_NAME_BYTES,
    P2P_TOKEN_MAX_APPLICATION_VERSION_BYTES, P2P_TOKEN_MAX_BYTES, P2P_TOKEN_MAX_PEER_TYPE_BYTES,
    P2P_TOKEN_MAX_SCOPES, P2P_TOKEN_MAX_SCOPE_BYTES, P2P_TOKEN_SCOPE, P2P_TOKEN_TTL,
    P2P_TOKEN_TYPE,
};
#[cfg(not(target_arch = "wasm32"))]
pub use transport::{
    ApplicationProtocol, AuthenticatedStream, IncomingAuthenticatedStreams, Node, RelayRouteHandle,
    RelayTransportEvent,
};
