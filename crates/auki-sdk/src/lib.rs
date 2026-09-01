//! Mechanical runtime facade for authenticated Auki peers.
//!
//! [`AukiPeerConfig`] defines the intentionally small host contract and
//! [`AukiPeer`] retains one authenticated transport, its renewable authority, and
//! optional DMS-backed relay reachability for their complete shared lifetime.

mod bootstrap;
mod config;
mod protocol_contract;
mod runtime_policy;
mod status;

#[cfg(not(target_arch = "wasm32"))]
mod authorization;
#[cfg(any(test, target_arch = "wasm32"))]
mod browser_booking;
#[cfg(target_arch = "wasm32")]
mod browser_peer_runtime;
#[cfg(target_arch = "wasm32")]
mod browser_protocols;
#[cfg(not(target_arch = "wasm32"))]
mod context;
#[cfg(not(target_arch = "wasm32"))]
mod known_peers;
#[cfg(not(target_arch = "wasm32"))]
mod peer_runtime;
#[cfg(not(target_arch = "wasm32"))]
mod protocols;

#[cfg(not(target_arch = "wasm32"))]
mod authority;

#[cfg(not(target_arch = "wasm32"))]
mod relay;

#[cfg(not(target_arch = "wasm32"))]
pub use auki_auth::AppCredentials;
pub use auki_auth::{
    AuthClient, AuthEnvironment, AuthLimits, Credentials, DomainChoice, DomainDescriptor,
    DomainSelection, PreparedPeer, PrincipalKind,
};
#[cfg(target_arch = "wasm32")]
pub use auki_p2p::BrowserAuthenticatedRouteStream as AuthenticatedRouteStream;
pub use auki_p2p::{
    AuthenticatedPeer, DdsVerificationKeys, Identity, Multiaddr, P2PAccessClaims, PeerId,
    RelayCircuitRoutes, SignedP2pCredential,
};
#[cfg(not(target_arch = "wasm32"))]
pub use auki_p2p::{
    AuthenticatedRouteStream, RouteCatalogError, RouteCatalogStatus, RouteFence, RouteSnapshot,
};
#[cfg(not(target_arch = "wasm32"))]
pub use authority::{ExternalAuthorityRefreshRequest, ExternalAuthorityUpdate};
#[cfg(not(target_arch = "wasm32"))]
pub use authorization::{
    AukiPeerAuthorization, AukiPeerAuthorizationError, AukiPeerAuthorizationSnapshot,
};
pub use bootstrap::{AukiPeerBootstrap, AukiPeerBootstrapError};
#[cfg(target_arch = "wasm32")]
pub use browser_peer_runtime::{
    AukiPeer, AukiPeerError, AukiPeerLifecycle, AukiPeerReachability, AukiPeerShutdownError,
    AukiPeerStartError,
};
#[cfg(target_arch = "wasm32")]
pub use browser_protocols::{AukiPeerProtocols, AukiProtocolRegistration};
#[cfg(not(target_arch = "wasm32"))]
pub use config::InitialPeerRoutes;
pub use config::{
    AukiPeerConfig, AukiPeerConfigError, AukiRelayConfig, AukiRelayConfigError, AukiRelayMode,
    DEV_DMS_BASE_URL,
};
#[cfg(not(target_arch = "wasm32"))]
pub use context::{AukiPeerProtocolContext, AukiPeerRoutes, AukiPeerRoutesError};
#[cfg(not(target_arch = "wasm32"))]
pub use known_peers::{
    AukiKnownPeer, AukiKnownPeerEvent, AukiKnownPeerRecvError, AukiKnownPeerSnapshot,
    AukiKnownPeerSubscription, AukiKnownPeers,
};
#[cfg(not(target_arch = "wasm32"))]
pub use peer_runtime::{
    AukiPeer, AukiPeerAuthorityError, AukiPeerLifecycle, AukiPeerRelayError, AukiPeerShutdownError,
    AukiPeerStartError, AukiPeerTransportError, ExternalAuthorityControl,
    ExternalAuthorityReplaceOutcome,
};
pub use protocol_contract::{
    AukiProtocolError, AukiProtocolRouteAttempt, AukiProtocolSpec, AukiProtocolStream,
};
#[cfg(not(target_arch = "wasm32"))]
pub use protocols::{AukiPeerProtocols, AukiProtocolRegistration};
#[cfg(not(target_arch = "wasm32"))]
pub use status::AukiPeerStatus;
pub use status::{AukiPeerExit, AukiPeerFailure};
