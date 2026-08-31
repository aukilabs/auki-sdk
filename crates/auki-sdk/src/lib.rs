//! Mechanical runtime facade for authenticated Auki peers.
//!
//! [`AukiPeerConfig`] defines the intentionally small host contract and
//! [`AukiPeer`] retains one authenticated transport, its renewable authority, and
//! optional DMS-backed relay reachability for their complete shared lifetime.

mod config;
mod protocol_contract;
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
#[allow(dead_code)]
mod authority;

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
mod relay;

pub use auki_auth::PreparedPeer;
#[cfg(target_arch = "wasm32")]
pub use auki_p2p::BrowserAuthenticatedRouteStream as AuthenticatedRouteStream;
pub use auki_p2p::{
    AuthenticatedPeer, DdsVerificationKeys, Identity, Multiaddr, P2PAccessClaims, PeerId,
    SignedP2pCredential,
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
#[cfg(target_arch = "wasm32")]
pub use browser_peer_runtime::{
    AukiPeer, AukiPeerError, AukiPeerExit, AukiPeerReachability, AukiPeerRoute,
    AukiPeerShutdownError, AukiPeerStartError,
};
#[cfg(target_arch = "wasm32")]
pub use browser_protocols::{AukiPeerProtocols, AukiProtocolRegistration};
pub use config::{
    AukiPeerConfig, AukiPeerConfigError, AukiRelayConfig, AukiRelayConfigError, AukiRelayMode,
    DEV_DMS_BASE_URL, InitialPeerRoutes,
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
    AukiPeer, AukiPeerAuthorityError, AukiPeerRelayError, AukiPeerShutdownError,
    AukiPeerStartError, AukiPeerTransportError, ExternalAuthorityControl,
    ExternalAuthorityReplaceOutcome,
};
pub use protocol_contract::{
    AukiProtocolError, AukiProtocolRouteAttempt, AukiProtocolSpec, AukiProtocolStream,
};
#[cfg(not(target_arch = "wasm32"))]
pub use protocols::{AukiPeerProtocols, AukiProtocolRegistration};
pub use status::{AukiPeerFailure, AukiPeerStatus};
