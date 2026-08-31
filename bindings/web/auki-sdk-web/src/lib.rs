//! Temporary Web binding bridge to the canonical `auki-sdk` browser facade.
//!
//! Generic peer, relay, authority, and protocol mechanics live in
//! `crates/auki-sdk`; this crate exists only until the application-specific
//! Wasm adapter adopts those canonical names directly.

pub use auki_sdk::{
    AukiPeerConfig as AukiWebPeerConfig, AukiPeerConfigError as AukiWebPeerConfigError,
    DEV_DMS_BASE_URL,
};

#[cfg(target_arch = "wasm32")]
mod bridge {
    use std::cell::RefCell;

    use auki_p2p::{
        ApplicationProtocol, AuthenticatedStream, BrowserIncomingAuthenticatedStreams, Identity,
        PeerId,
    };
    use auki_sdk::{AukiPeer, PreparedPeer};

    pub use auki_sdk::{
        AukiPeerError as AukiWebPeerError, AukiPeerExit as AukiWebPeerExit,
        AukiPeerReachability as AukiWebReachability, AukiPeerRoute as AukiWebRoute,
    };

    use crate::AukiWebPeerConfig;

    /// Short-lived bridge for the current Web example.
    pub struct AukiWebPeer {
        inner: RefCell<Option<AukiPeer>>,
        peer_id: PeerId,
        domain_id: uuid::Uuid,
        reachability: AukiWebReachability,
    }

    impl AukiWebPeer {
        pub async fn start(
            identity: Identity,
            prepared: PreparedPeer,
            config: AukiWebPeerConfig,
        ) -> Result<Self, AukiWebPeerError> {
            let peer = AukiPeer::start(identity, prepared, config).await?;
            Ok(Self {
                peer_id: peer.peer_id(),
                domain_id: peer.domain_id(),
                reachability: peer.reachability().clone(),
                inner: RefCell::new(Some(peer)),
            })
        }

        pub fn peer_id(&self) -> PeerId {
            self.peer_id
        }

        pub fn domain_id(&self) -> uuid::Uuid {
            self.domain_id
        }

        pub fn reachability(&self) -> &AukiWebReachability {
            &self.reachability
        }

        pub fn accept(
            &self,
            protocol: ApplicationProtocol,
        ) -> Result<BrowserIncomingAuthenticatedStreams, AukiWebPeerError> {
            self.inner
                .borrow()
                .as_ref()
                .ok_or(AukiWebPeerError::Stopped)?
                .accept(protocol)
        }

        pub async fn connect(
            &self,
            expected_peer: PeerId,
        ) -> Result<AukiWebRoute, AukiWebPeerError> {
            let peer = self.take_peer()?;
            let result = peer.connect(expected_peer).await;
            self.put_peer(peer);
            result
        }

        pub async fn open(
            &self,
            route: &AukiWebRoute,
            protocol: ApplicationProtocol,
        ) -> Result<AuthenticatedStream, AukiWebPeerError> {
            let peer = self.take_peer()?;
            let result = peer.open(route, protocol).await;
            self.put_peer(peer);
            result
        }

        pub async fn close_route(&self, route: &AukiWebRoute) -> Result<(), AukiWebPeerError> {
            let peer = self.take_peer()?;
            let result = peer.close_route(route).await;
            self.put_peer(peer);
            result
        }

        pub async fn wait_stopped(&self) -> AukiWebPeerExit {
            let Ok(peer) = self.take_peer() else {
                return AukiWebPeerExit::SupervisorStopped;
            };
            let status = peer.wait_stopped().await;
            self.put_peer(peer);
            status
        }

        pub async fn shutdown(&self) -> Result<(), AukiWebPeerError> {
            let peer = self
                .inner
                .borrow_mut()
                .take()
                .ok_or(AukiWebPeerError::Stopped)?;
            peer.shutdown().await
        }

        fn take_peer(&self) -> Result<AukiPeer, AukiWebPeerError> {
            self.inner
                .borrow_mut()
                .take()
                .ok_or(AukiWebPeerError::Stopped)
        }

        fn put_peer(&self, peer: AukiPeer) {
            let replaced = self.inner.borrow_mut().replace(peer);
            debug_assert!(replaced.is_none());
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use bridge::{
    AukiWebPeer, AukiWebPeerError, AukiWebPeerExit, AukiWebReachability, AukiWebRoute,
};
