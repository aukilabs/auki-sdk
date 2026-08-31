//! Generic Rust/Wasm composition for one authenticated Auki browser peer.
//!
//! JavaScript owns authentication, explicit Domain selection, peer startup,
//! public routing data, and ordered shutdown. Rust protocol adapters compiled
//! into the same Wasm module obtain the canonical [`auki_sdk::AukiPeerProtocols`]
//! handle without exposing transport streams to JavaScript.

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod facade {
    use std::{cell::RefCell, fmt::Display};

    use auki_auth::{
        AuthClient, AuthEnvironment, AuthSession, Credentials, DomainDescriptor, DomainSelection,
    };
    use auki_sdk::{
        AukiPeer as SdkPeer, AukiPeerConfig, AukiPeerExit, AukiPeerLifecycle, AukiPeerProtocols,
        Identity,
    };
    use js_sys::{Array, Error as JsError, Promise};
    use uuid::Uuid;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::future_to_promise;

    /// Authenticated User session used to inspect Domains and start peers.
    #[wasm_bindgen]
    pub struct AukiUserSession {
        auth: AuthSession,
        peer_config: AukiPeerConfig,
    }

    #[wasm_bindgen]
    impl AukiUserSession {
        /// Authenticate against the shared development environment.
        #[wasm_bindgen(js_name = loginDev)]
        pub async fn login_dev(
            email: String,
            password: String,
        ) -> Result<AukiUserSession, JsValue> {
            Self::login(
                AuthEnvironment::dev(),
                AukiPeerConfig::dev(),
                email,
                password,
            )
            .await
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
            Self::login(environment, peer_config, email, password).await
        }

        /// Public descriptors for every Domain this User can select.
        #[wasm_bindgen(js_name = accessibleDomains, unchecked_return_type = "AukiDomain[]")]
        pub async fn accessible_domains(&self) -> Result<Array, JsValue> {
            let choices = self
                .auth
                .accessible_domains()
                .await
                .map_err(|error| js_context("list accessible Domains", error))?;
            let domains = Array::new();
            for choice in choices {
                domains.push(&AukiDomain::from(choice.domain).into());
            }
            Ok(domains)
        }

        /// Authorize a fresh in-memory identity and start its mandatory relay.
        #[wasm_bindgen(js_name = startPeer)]
        pub async fn start_peer(&self, domain_id: String) -> Result<AukiPeer, JsValue> {
            let domain_id =
                Uuid::parse_str(&domain_id).map_err(|_| js_failure("Domain ID must be a UUID"))?;
            let identity = Identity::generate();
            let prepared = self
                .auth
                .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
                .await
                .map_err(|error| js_context("authorize browser Peer", error))?;
            let peer = SdkPeer::start(identity, prepared, self.peer_config.clone())
                .await
                .map_err(|error| js_context("start relay-backed browser Peer", error))?;
            Ok(AukiPeer::new(peer))
        }
    }

    impl AukiUserSession {
        async fn login(
            environment: AuthEnvironment,
            peer_config: AukiPeerConfig,
            email: String,
            password: String,
        ) -> Result<Self, JsValue> {
            let client = AuthClient::new(environment)
                .map_err(|error| js_context("configure authentication", error))?;
            let auth = client
                .authenticate(Credentials::user_password(email, password))
                .await
                .map_err(|error| js_context("authenticate User", error))?;
            Ok(Self { auth, peer_config })
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

    /// One ephemeral relay-backed browser peer.
    #[wasm_bindgen]
    pub struct AukiPeer {
        inner: RefCell<Option<SdkPeer>>,
        lifecycle: AukiPeerLifecycle,
        peer_id: String,
        domain_id: String,
        wss_route: String,
        tcp_route: Option<String>,
    }

    impl AukiPeer {
        fn new(peer: SdkPeer) -> Self {
            Self {
                lifecycle: peer.lifecycle(),
                peer_id: peer.peer_id().to_string(),
                domain_id: peer.domain_id().to_string(),
                wss_route: peer.reachability().wss().to_string(),
                tcp_route: peer.reachability().tcp().map(ToString::to_string),
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

        /// Confirmed browser-compatible circuit route.
        #[wasm_bindgen(getter, js_name = wssRoute)]
        pub fn wss_route(&self) -> String {
            self.wss_route.clone()
        }

        /// Confirmed native-compatible circuit route, when advertised.
        #[wasm_bindgen(getter, js_name = tcpRoute)]
        pub fn tcp_route(&self) -> Option<String> {
            self.tcp_route.clone()
        }

        /// Resolve after explicit shutdown or reject after unexpected terminal failure.
        #[wasm_bindgen(js_name = waitStopped, unchecked_return_type = "Promise<void>")]
        pub fn wait_stopped(&self) -> Promise {
            let lifecycle = self.lifecycle.clone();
            future_to_promise(async move {
                match lifecycle.wait_stopped().await {
                    AukiPeerExit::SupervisorStopped => Ok(JsValue::UNDEFINED),
                    AukiPeerExit::Node(status) => Err(js_context(
                        "browser Peer transport stopped unexpectedly",
                        format!("{status:?}"),
                    )),
                    AukiPeerExit::SupervisorFailed { reason } => {
                        Err(js_context("browser Peer stopped unexpectedly", reason))
                    }
                }
            })
        }

        /// Stop protocols, release the relay booking, and stop the transport.
        #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
        pub fn shutdown(&self) -> Result<Promise, JsValue> {
            let peer = self
                .inner
                .borrow_mut()
                .take()
                .ok_or_else(|| js_failure("Auki peer is stopped"))?;
            let shutdown = peer.shutdown();
            Ok(future_to_promise(async move {
                shutdown
                    .await
                    .map_err(|error| js_context("shut down browser Peer", error))?;
                Ok(JsValue::UNDEFINED)
            }))
        }
    }

    fn js_context(context: &'static str, error: impl Display) -> JsValue {
        JsError::new(&format!("{context}: {error}")).into()
    }

    fn js_failure(message: &'static str) -> JsValue {
        JsError::new(message).into()
    }
}

#[cfg(target_arch = "wasm32")]
pub use facade::{AukiDomain, AukiPeer, AukiUserSession};
