//! Mechanical runtime facade for authenticated Auki peers.
//!
//! [`AukiPeerConfig`] defines the intentionally small host contract and
//! [`AukiPeer`] retains one authenticated Domain, its renewable authority, and
//! optional DMS-backed relay reachability for their complete shared lifetime.

mod authorization;
mod config;
mod context;
mod peer_runtime;
mod status;

#[allow(dead_code)]
mod authority;

#[allow(dead_code)]
mod relay;

pub use auki_auth::PreparedPeer;
pub use auki_domain::{
    DomainPeers, DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec,
    DomainProtocolStream, DomainRouteAttempt, ServedProtocols,
};
pub use auki_p2p::{
    AuthenticatedPeer, AuthenticatedRouteStream, DdsVerificationKeys, Identity, Multiaddr,
    P2PAccessClaims, PeerId, RouteCatalogError, RouteCatalogStatus, RouteFence, RouteSnapshot,
    SignedP2pCredential,
};
pub use auki_session::{Peer, Session};
pub use authority::{ExternalAuthorityRefreshRequest, ExternalAuthorityUpdate};
pub use authorization::{
    AukiPeerAuthorization, AukiPeerAuthorizationError, AukiPeerAuthorizationSnapshot,
};
pub use config::{
    AukiPeerConfig, AukiPeerConfigError, AukiRelayConfig, AukiRelayConfigError, AukiRelayMode,
    DEV_DMS_BASE_URL, InitialPeerRoutes,
};
pub use context::{
    AukiPeerProtocolContext, AukiPeerProtocols, AukiPeerProtocolsError, AukiPeerRoutes,
    AukiPeerRoutesError,
};
pub use peer_runtime::{
    AukiPeer, AukiPeerAuthorityError, AukiPeerRelayError, AukiPeerShutdownError,
    AukiPeerStartError, ExternalAuthorityControl, ExternalAuthorityReplaceOutcome,
};
pub use status::{AukiPeerFailure, AukiPeerStatus};
