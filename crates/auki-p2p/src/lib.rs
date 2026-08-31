//! Native, direct libp2p transport gated by mutual DDS authorization.
//!
//! libp2p owns Ed25519 identities, Peer IDs, TCP, Noise, Yamux, dialing, and
//! versioned streams. This crate adds only the DDS P2P JWT checks required
//! before one of those streams becomes an [`AuthenticatedStream`]. It does not
//! fetch credentials or understand tasks, datasets, or machine-auth flows.

mod authentication;
mod authority;
mod error;
mod identity;
#[cfg(not(target_arch = "wasm32"))]
mod identity_store;
mod observation;
mod relay;
mod relay_client;
mod routing;
mod runtime;
mod source_admission;
mod targeted_stream;
mod token;
mod transport;

pub use authentication::{AuthenticatedPeer, SessionRequirements};
pub use authority::{DomainAuthority, P2pCredentialError, P2pCredentialResult};
pub use error::{Error, Result};
pub use identity::{Identity, PeerIdentityProof};
pub use libp2p::{multiaddr::Protocol, swarm::ConnectionId, Multiaddr, PeerId};
pub use observation::{
    AuthenticatedPeerObservation, NodeFailure, NodeObservationEvent, NodeObservationSnapshot,
    NodeObservationStatus, NodeObservations, PeerDisappearanceReason,
    PEER_OBSERVATION_CHANNEL_CAPACITY,
};
pub use relay::{
    ExpectedRelayLimits, RelayConfirmationRejection, RelayProvider, RelayReservationError,
    RelayReservationHandle, RelayReservationSnapshot, RelayReservationState, ReservationGeneration,
};
pub use routing::{
    canonicalize_circuit_route, validate_direct_route, CanonicalCircuitRoute, ConfirmedRoute,
    PublishedRoute, RouteCatalog, RouteCatalogError, RouteCatalogLimits, RouteCatalogResult,
    RouteCatalogStatus, RouteFence, RouteSnapshot,
};
pub use runtime::{AuthenticatedRouteStream, ExactRoute, ProtocolServer, ProtocolSpec};
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
pub use transport::{
    ApplicationProtocol, AuthenticatedStream, IncomingAuthenticatedStreams, Node, RelayRouteHandle,
    RelayTransportEvent,
};
