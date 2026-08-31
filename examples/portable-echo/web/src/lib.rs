//! Browser/Wasm adapter for the shared portable echo protocol.

#![cfg(target_arch = "wasm32")]
#![forbid(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    fmt::Display,
    time::Duration,
};

use auki_auth::{
    AuthClient, AuthEnvironment, AuthSession, Credentials, DomainDescriptor, DomainSelection,
};
use auki_p2p::{ApplicationProtocol, BrowserIncomingAuthenticatedStreams, Identity};
use auki_portable_echo_protocol::{ID as ECHO_PROTOCOL_ID, run_server};
use auki_sdk_web::{AukiWebPeer, AukiWebPeerConfig};
use futures::{AsyncWriteExt, FutureExt, pin_mut};
use futures_timer::Delay;
use js_sys::{Array, Error as JsError};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wasm_bindgen::prelude::*;

const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Authenticated dev User session used to list Domains before starting a peer.
#[wasm_bindgen]
pub struct BrowserUserSession {
    auth: AuthSession,
}

#[wasm_bindgen]
impl BrowserUserSession {
    /// Authenticate one User without creating a peer or booking a relay.
    #[wasm_bindgen(js_name = loginDev)]
    pub async fn login_dev(email: String, password: String) -> Result<BrowserUserSession, JsValue> {
        let auth = AuthClient::new(AuthEnvironment::dev())
            .map_err(|error| js_context("configure dev authentication", error))?;
        let session = auth
            .authenticate(Credentials::user_password(email, password))
            .await
            .map_err(|error| js_context("authenticate User", error))?;
        Ok(Self { auth: session })
    }

    /// Public descriptors for every Domain this User can explicitly select.
    #[wasm_bindgen(
        js_name = accessibleDomains,
        unchecked_return_type = "BrowserDomain[]"
    )]
    pub async fn accessible_domains(&self) -> Result<Array, JsValue> {
        let choices = self
            .auth
            .accessible_domains()
            .await
            .map_err(|error| js_context("list accessible Domains", error))?;
        let domains = Array::new();
        for choice in choices {
            domains.push(&BrowserDomain::from(choice.domain).into());
        }
        Ok(domains)
    }

    /// Authorize a fresh ephemeral peer in the selected Domain and acquire its
    /// mandatory relay before opting into the exact echo protocol.
    #[wasm_bindgen(js_name = startPeer)]
    pub async fn start_peer(&self, domain_id: String) -> Result<BrowserEchoServer, JsValue> {
        BrowserEchoServer::start(self.auth.clone(), domain_id).await
    }
}

/// One public Domain choice returned by DDS.
#[wasm_bindgen]
pub struct BrowserDomain {
    id: String,
    name: Option<String>,
    description: Option<String>,
    organization_id: Option<String>,
}

impl From<DomainDescriptor> for BrowserDomain {
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
impl BrowserDomain {
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

/// One explicitly opted-in echo server running on an ephemeral browser peer.
#[wasm_bindgen]
pub struct BrowserEchoServer {
    peer: RefCell<Option<AukiWebPeer>>,
    incoming: RefCell<Option<BrowserIncomingAuthenticatedStreams>>,
    serving: Cell<bool>,
    cancellation: CancellationToken,
    lifecycle: Cell<Lifecycle>,
    peer_id: String,
    domain_id: String,
    wss_route: String,
    tcp_route: Option<String>,
}

#[wasm_bindgen]
impl BrowserEchoServer {
    async fn start(auth: AuthSession, domain_id: String) -> Result<BrowserEchoServer, JsValue> {
        let domain_id =
            Uuid::parse_str(&domain_id).map_err(|_| js_failure("domain ID must be a UUID"))?;
        let identity = Identity::generate();
        let prepared = auth
            .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
            .await
            .map_err(|error| js_context("authorize browser Peer", error))?;
        let peer = AukiWebPeer::start(identity, prepared, AukiWebPeerConfig::dev())
            .await
            .map_err(|error| js_context("start relay-backed browser Peer", error))?;
        let protocol = ApplicationProtocol::new(ECHO_PROTOCOL_ID)
            .map_err(|error| js_context("configure echo protocol", error))?;
        let incoming = match peer.accept(protocol) {
            Ok(incoming) => incoming,
            Err(error) => {
                let _ = peer.shutdown().await;
                return Err(js_context("register echo protocol", error));
            }
        };

        Ok(Self {
            peer_id: peer.peer_id().to_string(),
            domain_id: peer.domain_id().to_string(),
            wss_route: peer.reachability().wss().to_string(),
            tcp_route: peer.reachability().tcp().map(ToString::to_string),
            peer: RefCell::new(Some(peer)),
            incoming: RefCell::new(Some(incoming)),
            serving: Cell::new(false),
            cancellation: CancellationToken::new(),
            lifecycle: Cell::new(Lifecycle::Running),
        })
    }

    /// Session-scoped libp2p Peer ID. A reload intentionally changes it.
    #[wasm_bindgen(getter, js_name = peerId)]
    pub fn peer_id(&self) -> String {
        self.peer_id.clone()
    }

    /// DDS Domain selected during explicit User authorization.
    #[wasm_bindgen(getter, js_name = domainId)]
    pub fn domain_id(&self) -> String {
        self.domain_id.clone()
    }

    /// Confirmed browser-compatible circuit route through the selected relay.
    #[wasm_bindgen(getter, js_name = wssRoute)]
    pub fn wss_route(&self) -> String {
        self.wss_route.clone()
    }

    /// Confirmed native-compatible route to this browser reservation, when the
    /// relay advertises its TCP listener.
    #[wasm_bindgen(getter, js_name = tcpRoute)]
    pub fn tcp_route(&self) -> Option<String> {
        self.tcp_route.clone()
    }

    /// Serve one authenticated request with the unmodified shared Rust
    /// protocol and return only its validated application result.
    #[wasm_bindgen(js_name = serveOnce)]
    pub async fn serve_once(&self) -> Result<EchoReceipt, JsValue> {
        if self.lifecycle.get() != Lifecycle::Running {
            return Err(js_failure("echo server is stopped"));
        }
        if self.serving.replace(true) {
            return Err(js_failure("an echo request is already being served"));
        }
        let Some(mut incoming) = self.incoming.borrow_mut().take() else {
            self.serving.set(false);
            return Err(js_failure("echo server is stopped"));
        };

        let (result, incoming_is_open) = serve_one(&self.cancellation, &mut incoming).await;
        self.serving.set(false);
        if incoming_is_open && !self.cancellation.is_cancelled() {
            let replaced = self.incoming.borrow_mut().replace(incoming);
            debug_assert!(replaced.is_none());
        }
        result
    }

    /// Stop accepting echo streams and run the facade's ordered peer cleanup.
    pub async fn shutdown(&self) -> Result<(), JsValue> {
        match self.lifecycle.replace(Lifecycle::Stopping) {
            Lifecycle::Running => {}
            Lifecycle::Stopping => {
                return Err(js_failure("echo server shutdown is already in progress"));
            }
            Lifecycle::Stopped => {
                self.lifecycle.set(Lifecycle::Stopped);
                return Ok(());
            }
        }
        self.cancellation.cancel();
        self.incoming.borrow_mut().take();
        let peer = self.peer.borrow_mut().take();
        let Some(peer) = peer else {
            self.lifecycle.set(Lifecycle::Stopped);
            return Ok(());
        };
        let result = peer
            .shutdown()
            .await
            .map_err(|error| js_context("shut down browser Peer", error));
        self.lifecycle.set(Lifecycle::Stopped);
        result
    }
}

impl Drop for BrowserEchoServer {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

async fn serve_one(
    cancellation: &CancellationToken,
    incoming: &mut BrowserIncomingAuthenticatedStreams,
) -> (Result<EchoReceipt, JsValue>, bool) {
    let accepted = {
        let accept = incoming.accept().fuse();
        let cancelled = cancellation.cancelled().fuse();
        pin_mut!(accept, cancelled);
        futures::select_biased! {
            () = cancelled => return (Err(js_failure("echo server is stopped")), false),
            accepted = accept => accepted,
        }
    };
    let Some(stream) = accepted else {
        return (
            Err(js_failure(
                "browser Peer stopped before an echo stream arrived",
            )),
            false,
        );
    };
    let mut stream = match stream {
        Ok(stream) => stream,
        Err(error) => {
            return (
                Err(js_context("authenticate inbound echo stream", error)),
                true,
            );
        }
    };
    let remote_peer_id = stream.remote_peer().peer_id.to_string();
    let exchange = {
        let request = run_server(&mut stream).fuse();
        let timeout = Delay::new(EXCHANGE_TIMEOUT).fuse();
        let cancelled = cancellation.cancelled().fuse();
        pin_mut!(request, timeout, cancelled);
        futures::select_biased! {
            () = cancelled => EchoExchange::Stopped,
            () = timeout => EchoExchange::TimedOut,
            request = request => EchoExchange::Complete(request),
        }
    };
    let request = match exchange {
        EchoExchange::Stopped => {
            return (Err(js_failure("echo server is stopped")), false);
        }
        EchoExchange::TimedOut => {
            return (Err(js_failure("echo exchange timed out")), true);
        }
        EchoExchange::Complete(request) => {
            match request.map_err(|error| js_context("run shared echo server", error)) {
                Ok(request) => request,
                Err(error) => return (Err(error), true),
            }
        }
    };
    let close = {
        let close = stream.close().fuse();
        let timeout = Delay::new(EXCHANGE_TIMEOUT).fuse();
        let cancelled = cancellation.cancelled().fuse();
        pin_mut!(close, timeout, cancelled);
        futures::select_biased! {
            () = cancelled => return (Err(js_failure("echo server is stopped")), false),
            () = timeout => return (Err(js_failure("closing echo stream timed out")), true),
            close = close => close,
        }
    };
    if let Err(error) = close {
        return (Err(js_context("close echo stream", error)), true);
    }

    (
        Ok(EchoReceipt {
            remote_peer_id,
            payload: request.into_bytes(),
        }),
        true,
    )
}

enum EchoExchange {
    Complete(
        Result<
            auki_portable_echo_protocol::EchoRequest,
            auki_portable_echo_protocol::EchoProtocolError,
        >,
    ),
    TimedOut,
    Stopped,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Lifecycle {
    Running,
    Stopping,
    Stopped,
}

/// Validated result of one shared echo-protocol server conversation.
#[wasm_bindgen]
pub struct EchoReceipt {
    remote_peer_id: String,
    payload: Vec<u8>,
}

#[wasm_bindgen]
impl EchoReceipt {
    #[wasm_bindgen(getter, js_name = remotePeerId)]
    pub fn remote_peer_id(&self) -> String {
        self.remote_peer_id.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn payload(&self) -> Vec<u8> {
        self.payload.clone()
    }
}

fn js_context(context: &'static str, error: impl Display) -> JsValue {
    JsError::new(&format!("{context}: {error}")).into()
}

fn js_failure(message: &'static str) -> JsValue {
    JsError::new(message).into()
}
