//! Thin UniFFI facade over the Rust-owned Auki peer runtime.
//!
//! Authentication, Domain authorization, identity proof, relay allocation,
//! transport, lifecycle, and cleanup stay in Rust. Swift owns only platform
//! concerns such as Keychain persistence and application lifecycle policy.

use std::sync::Arc;

use auki_sdk_rs::{
    AukiDiscovery as RustAukiDiscovery, AukiDiscoveryCandidate as RustAukiDiscoveryCandidate,
    AukiDiscoveryError, AukiDiscoverySource as RustAukiDiscoverySource, AukiPeer as RustAukiPeer,
    AukiPeerBootstrap, AukiPeerExit, AukiPeerFailure, AukiPeerLifecycle, AukiPeerProtocols,
    AukiPeerRoutes as RustAukiPeerRoutes, AukiPeerStatus as RustAukiPeerStatus, Credentials,
    DdsTrackerMode, DomainDescriptor, DomainSelection, Identity, Multiaddr, PeerId,
    validate_relay_circuit_routes,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::{runtime::Handle, sync::watch};
use uuid::Uuid;

uniffi::setup_scaffolding!();

#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob",
    feature = "message",
    feature = "stream",
    feature = "urdf-fk",
))]
mod protocols;

/// Cloneable cleanup result retained behind a platform adapter's one-shot
/// shutdown barrier.
pub type CleanupResult = Result<(), Arc<str>>;

/// Shared cancellation-safe cleanup primitive for Apple protocol adapters.
///
/// A platform adapter constructs its Rust cleanup future synchronously, then
/// passes it to `get_or_start`. The future runs once on the retained Tokio
/// runtime even if the observing Swift Task is cancelled or the UniFFI object
/// is released on an arbitrary thread.
pub struct DetachedCleanup {
    completion: Mutex<Option<watch::Sender<Option<CleanupResult>>>>,
    runtime: Handle,
}

impl DetachedCleanup {
    pub fn new() -> Self {
        Self {
            completion: Mutex::new(None),
            runtime: Handle::current(),
        }
    }

    pub fn get_or_start<F, E>(
        &self,
        start: impl FnOnce() -> F,
    ) -> watch::Receiver<Option<CleanupResult>>
    where
        F: std::future::Future<Output = Result<(), E>> + Send + 'static,
        E: std::fmt::Display,
    {
        let mut completion = self.completion.lock();
        if let Some(sender) = completion.as_ref() {
            return sender.subscribe();
        }

        let cleanup = start();
        let (sender, receiver) = watch::channel(None);
        *completion = Some(sender.clone());
        self.runtime.spawn(async move {
            let result = cleanup
                .await
                .map_err(|error| Arc::<str>::from(error.to_string()));
            sender.send_replace(Some(result));
        });
        receiver
    }
}

impl Default for DetachedCleanup {
    fn default() -> Self {
        Self::new()
    }
}

/// Wait for the retained result of a detached native cleanup.
pub async fn wait_cleanup(mut receiver: watch::Receiver<Option<CleanupResult>>) -> CleanupResult {
    loop {
        if let Some(result) = receiver.borrow_and_update().clone() {
            return result;
        }
        if receiver.changed().await.is_err() {
            return Err(Arc::from("native cleanup ended without a result"));
        }
    }
}

/// Stable Swift-facing error boundary. Internal Rust errors retain their full
/// source chains while the FFI exposes one non-secret diagnostic string.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AukiSdkError {
    #[error("{message}")]
    Operation { message: String },
}

pub(crate) fn operation_error(
    context: &'static str,
    error: impl std::fmt::Display,
) -> AukiSdkError {
    AukiSdkError::Operation {
        message: format!("{context}: {error}"),
    }
}

fn operation_error_chain(context: &'static str, error: impl std::error::Error) -> AukiSdkError {
    let mut message = format!("{context}: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        let cause = cause.to_string();
        if !message.ends_with(&cause) {
            message.push_str(": ");
            message.push_str(&cause);
        }
        source = source.and_then(std::error::Error::source);
    }
    AukiSdkError::Operation { message }
}

/// One accessible Auki Domain. Selection is always explicit.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiDomain {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub organization_id: Option<String>,
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

/// Explicit DDS tracker behavior selected when starting one peer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiDiscoveryMode {
    DiscoverOnly,
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

/// Provider that produced one process-local discovery observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiDiscoverySource {
    DdsTracker,
}

/// One bounded, untrusted DDS dial candidate.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiDiscoveryCandidate {
    pub peer_id: String,
    pub routes: Vec<String>,
    pub served_protocols: Vec<String>,
    pub expires_at: String,
    pub source: AukiDiscoverySource,
    pub subject_id: Option<String>,
    pub peer_type: Option<String>,
}

impl From<RustAukiDiscoveryCandidate> for AukiDiscoveryCandidate {
    fn from(candidate: RustAukiDiscoveryCandidate) -> Self {
        Self {
            peer_id: candidate.peer_id().to_string(),
            routes: candidate.routes().iter().map(ToString::to_string).collect(),
            served_protocols: candidate.served_protocols().to_vec(),
            expires_at: candidate.expires_at().to_rfc3339(),
            source: match candidate.source() {
                RustAukiDiscoverySource::DdsTracker => AukiDiscoverySource::DdsTracker,
            },
            subject_id: candidate.subject_id().map(|id| id.to_string()),
            peer_type: candidate.peer_type().map(str::to_owned),
        }
    }
}

/// Atomic TCP/WSS route pair confirmed for one relay reservation.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiPeerRoutes {
    pub tcp: String,
    pub wss: String,
}

/// Exact authenticated peer target selected from an advertised peer card.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiPeerTarget {
    pub domain_id: String,
    pub peer_id: String,
    pub route: String,
}

/// Bounded copyable peer description for manual exchange when DDS discovery is
/// disabled. Applications exchange this card, then dial its exact advertised route.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiPeerCard {
    pub version: u32,
    pub domain_id: String,
    pub peer_id: String,
    pub protocols: Vec<String>,
    pub routes: AukiPeerRoutes,
}

const MAX_PEER_CARD_JSON_BYTES: usize = 16 * 1024;
const MAX_PEER_CARD_PROTOCOLS: usize = 32;
const MAX_PROTOCOL_ID_BYTES: usize = 256;
const MAX_ROUTE_BYTES: usize = 2 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PeerCardWire {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime: Option<String>,
    domain_id: String,
    peer_id: String,
    protocols: Vec<String>,
    routes: PeerCardRoutesWire,
}

#[derive(Deserialize, Serialize)]
struct PeerCardRoutesWire {
    tcp: String,
    wss: String,
}

/// Rust-side validation shared by protocol adapters compiled into an umbrella
/// Apple artifact. Swift callers use `peer_card_from_json` or `AukiPeer.card`.
pub fn canonicalize_peer_card(mut card: AukiPeerCard) -> Result<AukiPeerCard, AukiSdkError> {
    if card.version != 1 {
        return Err(AukiSdkError::Operation {
            message: format!("unsupported peer card version {}", card.version),
        });
    }
    let domain_id = Uuid::parse_str(&card.domain_id)
        .map_err(|error| operation_error("parse peer card Domain ID", error))?;
    let peer_id = card
        .peer_id
        .parse::<PeerId>()
        .map_err(|error| operation_error("parse peer card Peer ID", error))?;
    if card.protocols.len() > MAX_PEER_CARD_PROTOCOLS
        || card
            .protocols
            .iter()
            .any(|protocol| protocol.is_empty() || protocol.len() > MAX_PROTOCOL_ID_BYTES)
    {
        return Err(AukiSdkError::Operation {
            message: "peer card protocol list exceeds its bounded envelope".into(),
        });
    }
    if card.routes.tcp.len() > MAX_ROUTE_BYTES || card.routes.wss.len() > MAX_ROUTE_BYTES {
        return Err(AukiSdkError::Operation {
            message: "peer card route exceeds its bounded envelope".into(),
        });
    }
    let tcp = card
        .routes
        .tcp
        .parse::<Multiaddr>()
        .map_err(|error| operation_error("parse peer card TCP route", error))?;
    let wss = card
        .routes
        .wss
        .parse::<Multiaddr>()
        .map_err(|error| operation_error("parse peer card WSS route", error))?;
    let routes = validate_relay_circuit_routes(&tcp, &wss, peer_id)
        .map_err(|error| operation_error("validate peer card relay routes", error))?;

    card.domain_id = domain_id.to_string();
    card.peer_id = peer_id.to_string();
    card.routes = AukiPeerRoutes {
        tcp: routes.tcp().to_string(),
        wss: routes.wss().to_string(),
    };
    Ok(card)
}

/// Parse the exact peer and route selected by a Swift caller.
pub fn parse_target(target: AukiPeerTarget) -> Result<(PeerId, Multiaddr), AukiSdkError> {
    let peer_id = target
        .peer_id
        .parse::<PeerId>()
        .map_err(|error| operation_error("parse remote Peer ID", error))?;
    let route = target
        .route
        .parse::<Multiaddr>()
        .map_err(|error| operation_error("parse remote route", error))?;
    Ok((peer_id, route))
}

#[uniffi::export]
pub fn peer_card_to_json(card: AukiPeerCard) -> Result<String, AukiSdkError> {
    let card = canonicalize_peer_card(card)?;
    serde_json::to_string(&PeerCardWire {
        version: card.version,
        runtime: Some("swift".into()),
        domain_id: card.domain_id,
        peer_id: card.peer_id,
        protocols: card.protocols,
        routes: PeerCardRoutesWire {
            tcp: card.routes.tcp,
            wss: card.routes.wss,
        },
    })
    .map_err(|error| operation_error("encode peer card", error))
}

#[uniffi::export]
pub fn peer_card_from_json(json: String) -> Result<AukiPeerCard, AukiSdkError> {
    if json.len() > MAX_PEER_CARD_JSON_BYTES {
        return Err(AukiSdkError::Operation {
            message: "peer card JSON exceeds its bounded envelope".into(),
        });
    }
    let wire = serde_json::from_str::<PeerCardWire>(&json)
        .map_err(|error| operation_error("decode peer card", error))?;
    canonicalize_peer_card(AukiPeerCard {
        version: wire.version,
        domain_id: wire.domain_id,
        peer_id: wire.peer_id,
        protocols: wire.protocols,
        routes: AukiPeerRoutes {
            tcp: wire.routes.tcp,
            wss: wire.routes.wss,
        },
    })
}

/// Select the native TCP member of a peer card's atomic route pair, optionally
/// requiring one advertised protocol first.
#[uniffi::export]
pub fn native_peer_target(
    card: AukiPeerCard,
    required_protocol: Option<String>,
) -> Result<AukiPeerTarget, AukiSdkError> {
    let card = canonicalize_peer_card(card)?;
    if let Some(required_protocol) = required_protocol
        && !card.protocols.iter().any(|item| item == &required_protocol)
    {
        return Err(AukiSdkError::Operation {
            message: format!("peer card does not advertise {required_protocol}"),
        });
    }
    Ok(AukiPeerTarget {
        domain_id: card.domain_id,
        peer_id: card.peer_id,
        route: card.routes.tcp,
    })
}

/// Small cross-language peer lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiPeerStatus {
    Ready,
    AuthorityUnavailable,
    RelayUnavailable,
    FailedTransport,
    FailedAuthority,
    FailedRelay,
    FailedSupervisor,
    FailedCleanup,
    Stopping,
    Stopped,
}

impl From<RustAukiPeerStatus> for AukiPeerStatus {
    fn from(status: RustAukiPeerStatus) -> Self {
        match status {
            RustAukiPeerStatus::Ready => Self::Ready,
            RustAukiPeerStatus::AuthorityUnavailable => Self::AuthorityUnavailable,
            RustAukiPeerStatus::RelayUnavailable => Self::RelayUnavailable,
            RustAukiPeerStatus::Failed(AukiPeerFailure::Transport) => Self::FailedTransport,
            RustAukiPeerStatus::Failed(AukiPeerFailure::Authority) => Self::FailedAuthority,
            RustAukiPeerStatus::Failed(AukiPeerFailure::Relay) => Self::FailedRelay,
            RustAukiPeerStatus::Failed(AukiPeerFailure::Supervisor) => Self::FailedSupervisor,
            RustAukiPeerStatus::Failed(AukiPeerFailure::Cleanup) => Self::FailedCleanup,
            RustAukiPeerStatus::Stopping => Self::Stopping,
            RustAukiPeerStatus::Stopped => Self::Stopped,
        }
    }
}

/// Stable libp2p identity whose canonical private-key bytes belong in the
/// platform Keychain. Debug output from the Rust identity redacts the secret.
#[derive(uniffi::Object)]
pub struct AukiPeerIdentity {
    identity: Identity,
}

impl AukiPeerIdentity {
    pub fn rust_identity(&self) -> Identity {
        self.identity.clone()
    }
}

#[uniffi::export]
impl AukiPeerIdentity {
    /// Generate a new stable identity. Persist `encoded()` before use.
    #[uniffi::constructor]
    pub fn generate() -> Arc<Self> {
        Arc::new(Self {
            identity: Identity::generate(),
        })
    }

    /// Restore the exact canonical libp2p private-key encoding from Keychain.
    #[uniffi::constructor]
    pub fn from_encoded(encoded: Vec<u8>) -> Result<Arc<Self>, AukiSdkError> {
        let identity = Identity::from_protobuf_encoding(&encoded)
            .map_err(|error| operation_error("restore Auki peer identity", error))?;
        Ok(Arc::new(Self { identity }))
    }

    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }

    /// Canonical secret bytes suitable for Keychain storage.
    pub fn encoded(&self) -> Result<Vec<u8>, AukiSdkError> {
        self.identity
            .to_protobuf_encoding()
            .map_err(|error| operation_error("encode Auki peer identity", error))
    }
}

/// Authenticated User session used to list Domains and start peers.
#[derive(uniffi::Object)]
pub struct AukiSession {
    bootstrap: AukiPeerBootstrap,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiSession {
    /// Authenticate a User against the shared development environment.
    #[uniffi::constructor]
    pub async fn login_dev(email: String, password: String) -> Result<Arc<Self>, AukiSdkError> {
        let bootstrap = AukiPeerBootstrap::dev(Credentials::user_password(email, password))
            .await
            .map_err(|error| operation_error("authenticate Auki User", error))?;
        Ok(Arc::new(Self { bootstrap }))
    }

    /// List every Domain this User may explicitly select.
    pub async fn accessible_domains(&self) -> Result<Vec<AukiDomain>, AukiSdkError> {
        self.bootstrap
            .accessible_domains()
            .await
            .map(|choices| {
                choices
                    .into_iter()
                    .map(|choice| AukiDomain::from(choice.domain))
                    .collect()
            })
            .map_err(|error| operation_error("list accessible Auki Domains", error))
    }

    /// Authorize the persisted identity and start a relay-backed peer.
    pub async fn start_peer(
        &self,
        domain_id: String,
        identity: Arc<AukiPeerIdentity>,
    ) -> Result<Arc<AukiPeer>, AukiSdkError> {
        let domain_id = Uuid::parse_str(&domain_id)
            .map_err(|error| operation_error("parse Auki Domain ID", error))?;
        let peer = self
            .bootstrap
            .start_peer(DomainSelection::new(domain_id), identity.rust_identity())
            .await
            .map_err(|error| operation_error_chain("start Auki peer", error))?;
        Ok(Arc::new(AukiPeer::new(peer)))
    }

    /// Authorize the persisted identity and start a peer with one explicit DDS
    /// discovery behavior.
    pub async fn start_peer_with_discovery(
        &self,
        domain_id: String,
        identity: Arc<AukiPeerIdentity>,
        mode: AukiDiscoveryMode,
    ) -> Result<Arc<AukiPeer>, AukiSdkError> {
        let domain_id = Uuid::parse_str(&domain_id)
            .map_err(|error| operation_error("parse Auki Domain ID", error))?;
        let peer = self
            .bootstrap
            .clone()
            .with_dds_tracker(mode.into())
            .start_peer(DomainSelection::new(domain_id), identity.rust_identity())
            .await
            .map_err(|error| operation_error_chain("start discoverable Auki peer", error))?;
        Ok(Arc::new(AukiPeer::new(peer)))
    }
}

struct PeerOwner {
    peer: Mutex<Option<RustAukiPeer>>,
    cleanup: DetachedCleanup,
}

impl PeerOwner {
    fn new(peer: RustAukiPeer) -> Self {
        Self {
            peer: Mutex::new(Some(peer)),
            cleanup: DetachedCleanup::new(),
        }
    }

    fn begin_shutdown(&self) -> watch::Receiver<Option<CleanupResult>> {
        // Constructing the shutdown future fences new protocol work before the
        // Swift task can be cancelled. Cleanup then runs detached exactly once.
        self.cleanup.get_or_start(|| {
            let shutdown = self.peer.lock().take().map(RustAukiPeer::shutdown);
            async move {
                match shutdown {
                    Some(shutdown) => shutdown.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }
}

impl Drop for PeerOwner {
    fn drop(&mut self) {
        if self.peer.get_mut().is_some() {
            let _ = self.begin_shutdown();
        }
    }
}

/// One relay-backed native Auki peer. The object is an owner, not a copyable
/// transport handle; UniFFI shares it with Swift through `Arc`.
#[derive(uniffi::Object)]
pub struct AukiPeer {
    owner: PeerOwner,
    peer_id: String,
    domain_id: String,
    listen_addresses: Vec<String>,
    routes: RustAukiPeerRoutes,
    lifecycle: AukiPeerLifecycle,
    status: watch::Receiver<RustAukiPeerStatus>,
    protocols: AukiPeerProtocols,
    discovery: Option<RustAukiDiscovery>,
}

impl AukiPeer {
    fn new(peer: RustAukiPeer) -> Self {
        let context = peer.protocol_context();
        let discovery = peer.discovery_handle().ok();
        Self {
            peer_id: peer.peer_id().to_string(),
            domain_id: peer.domain_id().to_string(),
            listen_addresses: peer
                .listen_addresses()
                .iter()
                .map(ToString::to_string)
                .collect(),
            routes: context.routes(),
            lifecycle: peer.lifecycle(),
            status: peer.subscribe_status(),
            protocols: context.protocols(),
            discovery,
            owner: PeerOwner::new(peer),
        }
    }

    /// Rust-only protocol capability for adapters linked into the same Apple
    /// artifact. It is intentionally absent from the generated Swift API.
    pub fn rust_protocols(&self) -> AukiPeerProtocols {
        self.protocols.clone()
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiPeer {
    pub fn peer_id(&self) -> String {
        self.peer_id.clone()
    }

    pub fn domain_id(&self) -> String {
        self.domain_id.clone()
    }

    pub fn listen_addresses(&self) -> Vec<String> {
        self.listen_addresses.clone()
    }

    /// One atomic snapshot of the required TCP/WSS routes from one relay slot.
    pub fn routes(&self) -> Result<AukiPeerRoutes, AukiSdkError> {
        let route = self
            .routes
            .snapshot()
            .map_err(|error| operation_error("read Auki peer routes", error))?
            .relay_routes
            .into_iter()
            .next()
            .ok_or_else(|| AukiSdkError::Operation {
                message: "read Auki peer routes: peer has no confirmed relay route".into(),
            })?;
        Ok(AukiPeerRoutes {
            tcp: route.routes.tcp().to_string(),
            wss: route.routes.wss().to_string(),
        })
    }

    /// Fetch every fresh same-Domain DDS candidate.
    pub async fn discover(&self) -> Result<Vec<AukiDiscoveryCandidate>, AukiSdkError> {
        self.discover_inner(None).await
    }

    /// Fetch fresh candidates advertising one exact protocol ID.
    pub async fn discover_protocol(
        &self,
        protocol_id: String,
    ) -> Result<Vec<AukiDiscoveryCandidate>, AukiSdkError> {
        self.discover_inner(Some(protocol_id)).await
    }

    /// Build one exact copyable card for the protocols the application has
    /// explicitly mounted on this peer.
    pub fn card(&self, protocols: Vec<String>) -> Result<AukiPeerCard, AukiSdkError> {
        canonicalize_peer_card(AukiPeerCard {
            version: 1,
            domain_id: self.domain_id(),
            peer_id: self.peer_id(),
            protocols,
            routes: self.routes()?,
        })
    }

    pub fn status(&self) -> AukiPeerStatus {
        (*self.status.borrow()).into()
    }

    /// Resolve after requested shutdown or throw on unexpected failure.
    pub async fn wait_stopped(&self) -> Result<(), AukiSdkError> {
        match self.lifecycle.wait_stopped().await {
            AukiPeerExit::Stopped => Ok(()),
            AukiPeerExit::Failed(failure) => Err(AukiSdkError::Operation {
                message: format!("Auki peer stopped unexpectedly: {failure:?}"),
            }),
        }
    }

    /// Fence immediately, then await one detached, replayable ordered cleanup.
    pub async fn shutdown(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.owner.begin_shutdown())
            .await
            .map_err(|error| operation_error("shut down Auki peer", error))
    }
}

impl AukiPeer {
    async fn discover_inner(
        &self,
        protocol_id: Option<String>,
    ) -> Result<Vec<AukiDiscoveryCandidate>, AukiSdkError> {
        let discovery = self
            .discovery
            .as_ref()
            .ok_or_else(|| operation_error("discover Auki peers", AukiDiscoveryError::Disabled))?;
        let candidates = match protocol_id {
            Some(protocol_id) => discovery.discover_protocol(protocol_id).await,
            None => discovery.discover().await,
        }
        .map_err(|error| operation_error("discover Auki peers", error))?;
        Ok(candidates.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::oneshot;

    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("outer failure: {source}")]
    struct NestedTestError {
        #[source]
        source: LeafTestError,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("leaf failure")]
    struct LeafTestError;

    fn assert_send_sync<T: Send + Sync>() {}

    fn peer_card() -> AukiPeerCard {
        let peer_id = Identity::generate().peer_id().to_string();
        let relay_peer_id = Identity::generate().peer_id().to_string();
        AukiPeerCard {
            version: 1,
            domain_id: "00000000-0000-0000-0000-000000000001".into(),
            peer_id: peer_id.clone(),
            protocols: vec!["/example/echo/1.0.0".into()],
            routes: AukiPeerRoutes {
                tcp: format!(
                    "/dns4/relay.example.com/tcp/443/p2p/{relay_peer_id}/p2p-circuit/p2p/{peer_id}"
                ),
                wss: format!(
                    "/dns4/relay.example.com/tcp/4443/wss/p2p/{relay_peer_id}/p2p-circuit/p2p/{peer_id}"
                ),
            },
        }
    }

    #[test]
    fn identity_round_trip_preserves_peer_id() {
        let identity = AukiPeerIdentity::generate();
        let encoded = identity.encoded().unwrap();
        let restored = AukiPeerIdentity::from_encoded(encoded).unwrap();
        assert_eq!(restored.peer_id(), identity.peer_id());
    }

    #[test]
    fn startup_errors_retain_their_native_source_chain_once() {
        let error = operation_error_chain(
            "start peer",
            NestedTestError {
                source: LeafTestError,
            },
        );

        assert_eq!(error.to_string(), "start peer: outer failure: leaf failure");
    }

    #[test]
    fn malformed_identity_fails_closed() {
        assert!(AukiPeerIdentity::from_encoded(b"not-a-private-key".to_vec()).is_err());
    }

    #[test]
    fn generic_peer_card_round_trips_and_selects_the_exact_tcp_route() {
        let card = peer_card();
        let json = peer_card_to_json(card.clone()).unwrap();
        let decoded = peer_card_from_json(json).unwrap();
        let target =
            native_peer_target(decoded.clone(), Some("/example/echo/1.0.0".into())).unwrap();

        assert_eq!(decoded, card);
        assert_eq!(target.peer_id, card.peer_id);
        assert_eq!(target.route, card.routes.tcp);
    }

    #[test]
    fn generic_peer_card_rejects_mismatched_pairs_and_missing_protocols() {
        let card = peer_card();
        assert!(native_peer_target(card.clone(), Some("/missing/1".into())).is_err());

        let mut mismatched = card;
        mismatched.routes.wss = mismatched.routes.wss.replace(
            &mismatched.peer_id,
            &Identity::generate().peer_id().to_string(),
        );
        assert!(canonicalize_peer_card(mismatched).is_err());
    }

    #[test]
    fn facade_handles_fit_uniffis_multithreaded_runtime() {
        assert_send_sync::<AukiPeerBootstrap>();
        assert_send_sync::<AukiPeerIdentity>();
        assert_send_sync::<AukiPeerProtocols>();
        assert_send_sync::<RustAukiPeerRoutes>();
        assert_send_sync::<AukiPeerLifecycle>();
        assert_send_sync::<RustAukiDiscovery>();
    }

    #[test]
    fn every_native_status_has_a_stable_swift_value() {
        let cases = [
            RustAukiPeerStatus::Ready,
            RustAukiPeerStatus::AuthorityUnavailable,
            RustAukiPeerStatus::RelayUnavailable,
            RustAukiPeerStatus::Failed(AukiPeerFailure::Transport),
            RustAukiPeerStatus::Failed(AukiPeerFailure::Authority),
            RustAukiPeerStatus::Failed(AukiPeerFailure::Relay),
            RustAukiPeerStatus::Failed(AukiPeerFailure::Supervisor),
            RustAukiPeerStatus::Failed(AukiPeerFailure::Cleanup),
            RustAukiPeerStatus::Stopping,
            RustAukiPeerStatus::Stopped,
        ];
        for status in cases {
            let _ = AukiPeerStatus::from(status);
        }
    }

    #[test]
    fn cleanup_observers_cannot_cancel_or_restart_native_cleanup() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let cleanup = DetachedCleanup::new();
            let starts = Arc::new(AtomicUsize::new(0));
            let (release, released) = oneshot::channel();
            let first = cleanup.get_or_start({
                let starts = Arc::clone(&starts);
                move || async move {
                    starts.fetch_add(1, Ordering::SeqCst);
                    released.await.map_err(|error| error.to_string())?;
                    Ok::<(), String>(())
                }
            });
            let second = cleanup.get_or_start(|| async {
                panic!("cleanup must not restart");
                #[allow(unreachable_code)]
                Ok::<(), String>(())
            });

            drop(first);
            release.send(()).unwrap();
            assert!(wait_cleanup(second).await.is_ok());
            assert_eq!(starts.load(Ordering::SeqCst), 1);

            let replay = cleanup.get_or_start(|| async {
                panic!("completed cleanup must not restart");
                #[allow(unreachable_code)]
                Ok::<(), String>(())
            });
            assert!(wait_cleanup(replay).await.is_ok());
        });
    }
}
