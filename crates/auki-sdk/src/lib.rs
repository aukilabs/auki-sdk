//! Mechanical runtime facade for authenticated Auki peers.
//!
//! [`AukiPeerConfig`] defines the intentionally small host contract. The live
//! [`AukiPeer`](https://docs.rs/auki-sdk/latest/auki_sdk/struct.AukiPeer.html)
//! owner is added separately once its lifecycle can be composed atomically.

mod authorization;
mod config;
mod context;
mod status;

#[allow(dead_code)]
mod authority;

#[allow(dead_code)]
mod relay;

pub use auki_auth::PreparedPeer;
pub use auki_domain::{
    DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
    DomainRouteAttempt, ServedProtocols,
};
pub use auki_p2p::{
    AuthenticatedPeer, AuthenticatedRouteStream, Identity, Multiaddr, P2PAccessClaims, PeerId,
    RouteCatalogError, RouteCatalogStatus, RouteFence, RouteSnapshot,
};
pub use auki_session::{Peer, Session};
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
pub use status::{AukiPeerFailure, AukiPeerStatus};
