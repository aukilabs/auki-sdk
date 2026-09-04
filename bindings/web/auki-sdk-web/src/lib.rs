//! Generic Rust/Wasm composition for one authenticated Auki browser peer.
//!
//! JavaScript supplies User credentials and an explicit Domain selection, then
//! owns the returned peer object, public routing data, and ordered shutdown.
//! Rust owns authentication and peer startup through
//! [`auki_sdk::AukiPeerBootstrap`]. Rust protocol adapters compiled into the
//! same Wasm module obtain the canonical [`auki_sdk::AukiPeerProtocols`] handle
//! without exposing transport streams to JavaScript.

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod protocol_support;
#[cfg(target_arch = "wasm32")]
mod protocols;

#[cfg(target_arch = "wasm32")]
mod facade {
    use std::cell::RefCell;

    use auki_sdk::{
        AukiDiscovery as SdkDiscovery, AukiDiscoveryCandidate as SdkDiscoveryCandidate,
        AukiDiscoverySource, AukiPeer as SdkPeer, AukiPeerBootstrap, AukiPeerConfig, AukiPeerExit,
        AukiPeerLifecycle, AukiPeerProtocols, AuthClient, AuthEnvironment, Credentials,
        DdsTrackerMode, DomainDescriptor, DomainSelection,
    };
    use js_sys::{Array, Promise};
    use uuid::Uuid;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::future_to_promise;

    use crate::protocol_support::{js_context, js_error};

    /// Explicit DDS tracker behavior for a browser peer.
    #[wasm_bindgen]
    #[derive(Clone, Copy)]
    pub enum AukiDiscoveryMode {
        /// Read fresh advertisements without publishing this peer.
        DiscoverOnly,
        /// Read advertisements and maintain this peer's short-lived lease.
        DiscoverAndAdvertise,
    }

    impl From<AukiDiscoveryMode> for DdsTrackerMode {
        fn from(mode: AukiDiscoveryMode) -> Self {
            match mode {
                AukiDiscoveryMode::DiscoverOnly => Self::DiscoverOnly,
                AukiDiscoveryMode::DiscoverAndAdvertise => Self::DiscoverAndAdvertise,
            }
        }
    }

    /// Whether this browser peer should own a public inbound relay route.
    #[wasm_bindgen]
    #[derive(Clone, Copy)]
    pub enum AukiPeerReachabilityMode {
        /// Do not book a relay. The peer can still dial remote WSS relay routes.
        OutboundOnly,
        /// Book and maintain one relay so other peers can dial this browser peer.
        RelayBacked,
    }

    impl Default for AukiPeerReachabilityMode {
        fn default() -> Self {
            Self::RelayBacked
        }
    }

    /// Authenticated User session used to inspect Domains and start peers.
    #[wasm_bindgen]
    pub struct AukiUserSession {
        bootstrap: AukiPeerBootstrap,
    }

    #[wasm_bindgen]
    impl AukiUserSession {
        /// Authenticate against the shared development environment.
        #[wasm_bindgen(js_name = loginDev)]
        pub async fn login_dev(
            email: String,
            password: String,
        ) -> Result<AukiUserSession, JsValue> {
            let bootstrap = AukiPeerBootstrap::dev(Credentials::user_password(email, password))
                .await
                .map_err(|error| js_context("authenticate User", error))?;
            Ok(Self { bootstrap })
        }

        /// Authenticate against exact API, DDS, and DMS HTTP bases.
        #[wasm_bindgen(js_name = loginWithEnvironment)]
        pub async fn login_with_environment(
            api_base_url: String,
            dds_base_url: String,
            dms_base_url: String,
            email: String,
            password: String,
        ) -> Result<AukiUserSession, JsValue> {
            let environment = AuthEnvironment::new(api_base_url, dds_base_url)
                .map_err(|error| js_context("configure authentication", error))?;
            let peer_config = AukiPeerConfig::new(dms_base_url)
                .map_err(|error| js_context("configure DMS", error))?;
            let client = AuthClient::new(environment)
                .map_err(|error| js_context("configure authentication", error))?;
            let bootstrap = AukiPeerBootstrap::authenticate(
                client,
                Credentials::user_password(email, password),
                peer_config,
            )
            .await
            .map_err(|error| js_context("authenticate User", error))?;
            Ok(Self { bootstrap })
        }

        /// Public descriptors for every Domain this User can select.
        #[wasm_bindgen(js_name = accessibleDomains, unchecked_return_type = "AukiDomain[]")]
        pub async fn accessible_domains(&self) -> Result<Array, JsValue> {
            let choices = self
                .bootstrap
                .accessible_domains()
                .await
                .map_err(|error| js_context("list accessible Domains", error))?;
            let domains = Array::new();
            for choice in choices {
                domains.push(&AukiDomain::from(choice.domain).into());
            }
            Ok(domains)
        }

        /// Authorize a fresh in-memory identity and start one browser peer.
        ///
        /// Omitting `reachability` preserves the relay-backed default.
        #[wasm_bindgen(js_name = startPeer)]
        pub async fn start_peer(
            &self,
            domain_id: String,
            reachability: Option<AukiPeerReachabilityMode>,
        ) -> Result<AukiPeer, JsValue> {
            let domain_id =
                Uuid::parse_str(&domain_id).map_err(|_| js_error("Domain ID must be a UUID"))?;
            let peer = configured_bootstrap(&self.bootstrap, reachability)
                .start_ephemeral_peer(DomainSelection::new(domain_id))
                .await
                .map_err(|error| js_context("start browser Peer", error))?;
            Ok(AukiPeer::new(peer))
        }

        /// Start a peer with explicit DDS discovery and optional reachability.
        ///
        /// Outbound-only peers may discover but cannot advertise because they
        /// have no public route. Omitting `reachability` remains relay-backed.
        #[wasm_bindgen(js_name = startPeerWithDiscovery)]
        pub async fn start_peer_with_discovery(
            &self,
            domain_id: String,
            mode: AukiDiscoveryMode,
            reachability: Option<AukiPeerReachabilityMode>,
        ) -> Result<AukiPeer, JsValue> {
            let domain_id =
                Uuid::parse_str(&domain_id).map_err(|_| js_error("Domain ID must be a UUID"))?;
            let peer = configured_bootstrap(&self.bootstrap, reachability)
                .with_dds_tracker(mode.into())
                .start_ephemeral_peer(DomainSelection::new(domain_id))
                .await
                .map_err(|error| js_context("start browser Peer with discovery", error))?;
            Ok(AukiPeer::new(peer))
        }
    }

    fn configured_bootstrap(
        bootstrap: &AukiPeerBootstrap,
        reachability: Option<AukiPeerReachabilityMode>,
    ) -> AukiPeerBootstrap {
        match reachability.unwrap_or_default() {
            AukiPeerReachabilityMode::OutboundOnly => bootstrap.clone().without_relay(),
            AukiPeerReachabilityMode::RelayBacked => bootstrap.clone(),
        }
    }

    /// One public Domain choice returned by DDS.
    #[wasm_bindgen]
    pub struct AukiDomain {
        id: String,
        name: Option<String>,
        description: Option<String>,
        organization_id: Option<String>,
    }

    impl From<DomainDescriptor> for AukiDomain {
        fn from(domain: DomainDescriptor) -> Self {
            Self {
                id: domain.id.to_string(),
                name: domain.name,
                description: domain.description,
                organization_id: domain.organization_id.map(|id| id.to_string()),
            }
        }
    }

    #[wasm_bindgen]
    impl AukiDomain {
        #[wasm_bindgen(getter)]
        pub fn id(&self) -> String {
            self.id.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn name(&self) -> Option<String> {
            self.name.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn description(&self) -> Option<String> {
            self.description.clone()
        }

        #[wasm_bindgen(getter, js_name = organizationId)]
        pub fn organization_id(&self) -> Option<String> {
            self.organization_id.clone()
        }
    }

    /// One bounded, untrusted DDS dial candidate.
    #[wasm_bindgen]
    pub struct AukiDiscoveryCandidate {
        peer_id: String,
        routes: Vec<String>,
        served_protocols: Vec<String>,
        expires_at: String,
        source: String,
    }

    impl From<SdkDiscoveryCandidate> for AukiDiscoveryCandidate {
        fn from(candidate: SdkDiscoveryCandidate) -> Self {
            Self {
                peer_id: candidate.peer_id().to_string(),
                routes: candidate.routes().iter().map(ToString::to_string).collect(),
                served_protocols: candidate.served_protocols().to_vec(),
                expires_at: candidate.expires_at().to_rfc3339(),
                source: match candidate.source() {
                    AukiDiscoverySource::DdsTracker => "dds_tracker".into(),
                },
            }
        }
    }

    #[wasm_bindgen]
    impl AukiDiscoveryCandidate {
        #[wasm_bindgen(getter, js_name = peerId)]
        pub fn peer_id(&self) -> String {
            self.peer_id.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn routes(&self) -> Vec<String> {
            self.routes.clone()
        }

        #[wasm_bindgen(getter, js_name = servedProtocols)]
        pub fn served_protocols(&self) -> Vec<String> {
            self.served_protocols.clone()
        }

        #[wasm_bindgen(getter, js_name = expiresAt)]
        pub fn expires_at(&self) -> String {
            self.expires_at.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn source(&self) -> String {
            self.source.clone()
        }
    }

    /// One ephemeral browser peer with optional relay-backed reachability.
    #[wasm_bindgen]
    pub struct AukiPeer {
        inner: RefCell<Option<SdkPeer>>,
        lifecycle: AukiPeerLifecycle,
        discovery: Option<SdkDiscovery>,
        peer_id: String,
        domain_id: String,
        wss_route: Option<String>,
        tcp_route: Option<String>,
        relay_backed: bool,
    }

    impl AukiPeer {
        fn new(peer: SdkPeer) -> Self {
            let discovery = peer.discovery_handle().ok();
            let relay_routes = peer.reachability().relay_routes();
            let wss_route = relay_routes.map(|routes| routes.wss().to_string());
            let tcp_route = relay_routes.map(|routes| routes.tcp().to_string());
            Self {
                lifecycle: peer.lifecycle(),
                peer_id: peer.peer_id().to_string(),
                domain_id: peer.domain_id().to_string(),
                relay_backed: peer.reachability().is_relay_backed(),
                wss_route,
                tcp_route,
                discovery,
                inner: RefCell::new(Some(peer)),
            }
        }

        /// Canonical protocol surface for Rust adapters in this Wasm module.
        pub fn protocols(&self) -> Option<AukiPeerProtocols> {
            self.inner.borrow().as_ref().map(SdkPeer::protocols)
        }
    }

    #[wasm_bindgen]
    impl AukiPeer {
        /// Session-scoped libp2p Peer ID. Reloading intentionally changes it.
        #[wasm_bindgen(getter, js_name = peerId)]
        pub fn peer_id(&self) -> String {
            self.peer_id.clone()
        }

        /// DDS Domain selected during explicit User authorization.
        #[wasm_bindgen(getter, js_name = domainId)]
        pub fn domain_id(&self) -> String {
            self.domain_id.clone()
        }

        /// Confirmed browser-compatible circuit route, if relay-backed.
        #[wasm_bindgen(getter, js_name = wssRoute)]
        pub fn wss_route(&self) -> Option<String> {
            self.wss_route.clone()
        }

        /// Confirmed native-compatible circuit route, if relay-backed.
        #[wasm_bindgen(getter, js_name = tcpRoute)]
        pub fn tcp_route(&self) -> Option<String> {
            self.tcp_route.clone()
        }

        /// Whether this peer owns and maintains an inbound relay booking.
        #[wasm_bindgen(getter, js_name = relayBacked)]
        pub fn relay_backed(&self) -> bool {
            self.relay_backed
        }

        /// Fetch every fresh same-Domain DDS candidate.
        #[wasm_bindgen(unchecked_return_type = "AukiDiscoveryCandidate[]")]
        pub async fn discover(&self) -> Result<Array, JsValue> {
            self.discover_with_protocol(None).await
        }

        /// Fetch fresh candidates advertising one exact protocol ID.
        #[wasm_bindgen(js_name = discoverProtocol, unchecked_return_type = "AukiDiscoveryCandidate[]")]
        pub async fn discover_protocol(&self, protocol_id: String) -> Result<Array, JsValue> {
            self.discover_with_protocol(Some(protocol_id)).await
        }

        /// Resolve after explicit shutdown or reject after unexpected terminal failure.
        #[wasm_bindgen(js_name = waitStopped, unchecked_return_type = "Promise<void>")]
        pub fn wait_stopped(&self) -> Promise {
            let lifecycle = self.lifecycle.clone();
            future_to_promise(async move {
                match lifecycle.wait_stopped().await {
                    AukiPeerExit::Stopped => Ok(JsValue::UNDEFINED),
                    AukiPeerExit::Failed(failure) => Err(js_context(
                        "browser Peer stopped unexpectedly",
                        format!("{failure:?}"),
                    )),
                }
            })
        }

        /// Stop protocols, release any relay booking, and stop the transport.
        #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
        pub fn shutdown(&self) -> Result<Promise, JsValue> {
            let peer = self
                .inner
                .borrow_mut()
                .take()
                .ok_or_else(|| js_error("Auki peer is stopped"))?;
            let shutdown = peer.shutdown();
            Ok(future_to_promise(async move {
                shutdown
                    .await
                    .map_err(|error| js_context("shut down browser Peer", error))?;
                Ok(JsValue::UNDEFINED)
            }))
        }
    }

    impl AukiPeer {
        async fn discover_with_protocol(
            &self,
            protocol_id: Option<String>,
        ) -> Result<Array, JsValue> {
            let discovery = self
                .discovery
                .as_ref()
                .ok_or_else(|| js_error("DDS discovery is not configured for this Auki peer"))?;
            let candidates = match protocol_id {
                Some(protocol_id) => discovery.discover_protocol(protocol_id).await,
                None => discovery.discover().await,
            }
            .map_err(|error| js_context("discover Auki peers", error))?;
            let values = Array::new();
            for candidate in candidates {
                values.push(&AukiDiscoveryCandidate::from(candidate).into());
            }
            Ok(values)
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use facade::{
    AukiDiscoveryCandidate, AukiDiscoveryMode, AukiDomain, AukiPeer, AukiPeerReachabilityMode,
    AukiUserSession,
};
#[cfg(target_arch = "wasm32")]
pub use protocols::*;
