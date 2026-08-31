//! Mechanical runtime facade for authenticated Auki peers.
//!
//! [`AukiPeerConfig`] defines the intentionally small host contract and
//! [`AukiPeer`] retains one authenticated transport, its renewable authority, and
//! optional DMS-backed relay reachability for their complete shared lifetime.

mod authorization;
mod config;
mod context;
mod known_peers;
mod peer_runtime;
mod protocols;
mod status;

#[allow(dead_code)]
mod authority;

#[allow(dead_code)]
mod relay;

pub use auki_auth::PreparedPeer;
pub use auki_p2p::{
    AuthenticatedPeer, AuthenticatedRouteStream, DdsVerificationKeys, Identity, Multiaddr,
    P2PAccessClaims, PeerId, RouteCatalogError, RouteCatalogStatus, RouteFence, RouteSnapshot,
    SignedP2pCredential,
};
pub use authority::{ExternalAuthorityRefreshRequest, ExternalAuthorityUpdate};
pub use authorization::{
    AukiPeerAuthorization, AukiPeerAuthorizationError, AukiPeerAuthorizationSnapshot,
};
pub use config::{
    AukiPeerConfig, AukiPeerConfigError, AukiRelayConfig, AukiRelayConfigError, AukiRelayMode,
    DEV_DMS_BASE_URL, InitialPeerRoutes,
};
pub use context::{AukiPeerProtocolContext, AukiPeerRoutes, AukiPeerRoutesError};
pub use known_peers::{
    AukiKnownPeer, AukiKnownPeerEvent, AukiKnownPeerRecvError, AukiKnownPeerSnapshot,
    AukiKnownPeerSubscription, AukiKnownPeers,
};
pub use peer_runtime::{
    AukiPeer, AukiPeerAuthorityError, AukiPeerRelayError, AukiPeerShutdownError,
    AukiPeerStartError, AukiPeerTransportError, ExternalAuthorityControl,
    ExternalAuthorityReplaceOutcome,
};
pub use protocols::{
    AukiPeerProtocols, AukiProtocolError, AukiProtocolRegistration, AukiProtocolRouteAttempt,
    AukiProtocolSpec, AukiProtocolStream,
};
pub use status::{AukiPeerFailure, AukiPeerStatus};
