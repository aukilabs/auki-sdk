use std::{error::Error, fmt, future::pending, sync::Arc, time::Duration};

use auki_auth::PreparedPeer;
use auki_p2p::{
    DdsTokenVerifier, DdsVerificationKeys, Multiaddr, Node, NodeObservationEvent,
    NodeObservationStatus, NodeObservations, PeerId, RouteCatalog, RouteCatalogError,
    RouteCatalogStatus,
};
use parking_lot::Mutex;
use tokio::{
    sync::{Mutex as AsyncMutex, broadcast, watch},
    task::{JoinError, JoinHandle},
};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use crate::{
    authority::{
        AuthorityInstallOutcome, AuthorityStatus, AuthoritySupervisor, AuthoritySupervisorConfig,
        AuthoritySupervisorError, ExternalAuthorityHandle, ExternalAuthorityRefreshRequest,
        ExternalAuthorityUpdate, ExternalRefreshRequests, FixedDomainAuthority,
    },
    config::{AukiPeerConfig, AukiRelayConfig},
    context::{AukiPeerProtocolContext, ContextLifecycle},
    discovery::{
        AukiDiscoveryCandidate, AukiDiscoveryError, DdsDiscovery, DdsTrackerMode,
        NativeDdsPublisher,
    },
    known_peers::AukiKnownPeers,
    protocol_contract::AukiProtocolError,
    protocols::AukiPeerProtocols,
    relay::{
        RelayBookingClient, RelayBookingClientError, RelayIdempotencyKey,
        coordinator::{
            PeerRelayReservations, RelayBookingCoordinator, RelayCoordinatorConfig,
            RelayCoordinatorError, RelayCoordinatorHealth, RelayCoordinatorShutdownError,
            RelayCoordinatorShutdownOutcome,
        },
    },
    runtime_policy::{RELAY_STARTUP_STATUS_POLL_INTERVAL, booking_mode},
    status::{AukiPeerExit, AukiPeerFailure, AukiPeerStatus},
};

const RELAY_RESERVATION_RETRY_BUDGET: Duration = Duration::from_secs(30);
const RELAY_RETRY_MIN: Duration = Duration::from_millis(250);
const RELAY_RETRY_MAX: Duration = Duration::from_secs(5);
const RELAY_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
// A retirement can spend one budget joining its worker and up to two budgets
// canceling distinct returned handles, followed by the coordinator's bounded
// DELETE retry budget. Keep the outer deadline above that complete sequence.
const RELAY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(150);
const LISTENER_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const NODE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct AukiPeerAuthorityError(AuthoritySupervisorError);

impl From<AuthoritySupervisorError> for AukiPeerAuthorityError {
    fn from(error: AuthoritySupervisorError) -> Self {
        Self(error)
    }
}

/// Sole host-side control plane for an externally managed authority source.
///
/// This value is intentionally not cloneable: it owns the only refresh-request
/// stream for its [`AukiPeer`]. The replacement handle is weak, so retaining
/// the control does not keep a stopped runtime alive.
pub struct ExternalAuthorityControl {
    handle: ExternalAuthorityHandle,
    requests: AsyncMutex<ExternalRefreshRequests>,
}

/// Result of installing one externally supplied authority update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalAuthorityReplaceOutcome {
    /// A newer signed credential became the current authority revision.
    Replaced {
        /// Newly installed credential revision.
        credential_revision: u64,
    },
    /// The verified claims were equivalent, so only verification keys were refreshed.
    Unchanged {
        /// Existing credential revision, which did not advance.
        credential_revision: u64,
    },
}

impl ExternalAuthorityReplaceOutcome {
    /// Current credential revision after the update was installed.
    pub fn credential_revision(&self) -> u64 {
        match *self {
            Self::Replaced {
                credential_revision,
            }
            | Self::Unchanged {
                credential_revision,
            } => credential_revision,
        }
    }
}

impl From<AuthorityInstallOutcome> for ExternalAuthorityReplaceOutcome {
    fn from(outcome: AuthorityInstallOutcome) -> Self {
        match outcome {
            AuthorityInstallOutcome::Replaced(credential_revision) => Self::Replaced {
                credential_revision,
            },
            AuthorityInstallOutcome::Unchanged(credential_revision) => Self::Unchanged {
                credential_revision,
            },
        }
    }
}

impl ExternalAuthorityControl {
    fn new(handle: ExternalAuthorityHandle, requests: ExternalRefreshRequests) -> Self {
        Self {
            handle,
            requests: AsyncMutex::new(requests),
        }
    }

    /// Atomically validate and install a complete authority replacement.
    ///
    /// Domain and Peer bindings remain pinned to the initial update, and
    /// verification keys are installed before the credential becomes current.
    pub async fn replace(
        &self,
        update: ExternalAuthorityUpdate,
    ) -> Result<ExternalAuthorityReplaceOutcome, AukiPeerAuthorityError> {
        self.handle
            .replace(update)
            .await
            .map(ExternalAuthorityReplaceOutcome::from)
            .map_err(AukiPeerAuthorityError::from)
    }

    /// Wait for the next coalesced relay-authorization refresh request.
    ///
    /// Returns `None` after the associated runtime's authority supervisor has
    /// stopped. The host decides how and when to obtain a replacement update.
    pub async fn next_refresh_request(&self) -> Option<ExternalAuthorityRefreshRequest> {
        self.requests.lock().await.recv().await
    }
}

impl fmt::Debug for ExternalAuthorityControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalAuthorityControl")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
enum RelayRuntimeError {
    #[error("DMS relay client construction failed")]
    Client(#[source] RelayBookingClientError),
    #[error("relay allocation or reservation reconciliation failed")]
    Coordinator(#[source] RelayCoordinatorError),
    #[error("relay startup ended before one route was confirmed")]
    Unavailable,
    #[error("relay shutdown failed")]
    Shutdown(#[source] RelayCoordinatorShutdownError),
    #[error("relay shutdown exhausted its graceful deadline before DMS deletion was confirmed")]
    ForcedShutdown,
}

/// Detailed relay-lifecycle failure retained without exposing DMS control capabilities.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct AukiPeerRelayError(RelayRuntimeError);

impl AukiPeerRelayError {
    fn client(error: RelayBookingClientError) -> Self {
        Self(RelayRuntimeError::Client(error))
    }

    fn coordinator(error: RelayCoordinatorError) -> Self {
        Self(RelayRuntimeError::Coordinator(error))
    }

    fn unavailable() -> Self {
        Self(RelayRuntimeError::Unavailable)
    }

    fn shutdown(error: RelayCoordinatorShutdownError) -> Self {
        Self(RelayRuntimeError::Shutdown(error))
    }

    fn forced_shutdown() -> Self {
        Self(RelayRuntimeError::ForcedShutdown)
    }
}

/// Failure to atomically compose one peer runtime.
#[derive(Debug, thiserror::Error)]
pub enum AukiPeerStartError {
    /// The supplied identity is not the identity authorized by DDS.
    #[error("authorized Peer ID {authorized} does not match identity Peer ID {identity}")]
    IdentityMismatch {
        /// Peer ID bound into the supplied authority.
        authorized: PeerId,
        /// Peer ID derived from the supplied stable identity.
        identity: PeerId,
    },
    /// Local advertised-route composition failed.
    #[error("failed to initialize the local route catalog")]
    Routes(#[source] RouteCatalogError),
    /// The authenticated P2P transport could not start.
    #[error("failed to start the authenticated P2P transport")]
    Transport(#[source] auki_p2p::Error),
    /// The configured listeners did not all bind within the startup deadline.
    #[error("the authenticated P2P listeners did not bind within the startup deadline")]
    ListenerStartupTimeout,
    /// The authenticated transport stopped before startup became ready.
    #[error("the authenticated P2P transport stopped before AukiPeer startup became ready")]
    TransportUnavailable,
    /// Initial authority supervision failed or ended before readiness.
    #[error("failed to start local authority supervision")]
    Authority(#[source] AukiPeerAuthorityError),
    /// Signed local authority became unavailable before relay readiness.
    #[error("local authority became unavailable before AukiPeer startup became ready")]
    AuthorityUnavailable,
    /// Required relay-backed reachability could not become ready.
    #[error("failed to establish required relay-backed reachability")]
    Relay(#[source] AukiPeerRelayError),
    /// Required initial DDS self-advertisement could not be established.
    #[error("failed to start DDS peer discovery")]
    Discovery(#[source] AukiDiscoveryError),
    /// A retained startup status channel ended unexpectedly.
    #[error("an AukiPeer startup component stopped unexpectedly")]
    Supervisor,
}

#[derive(Debug, Default)]
struct ShutdownFailures {
    discovery: Option<AukiDiscoveryError>,
    relay: Option<AukiPeerRelayError>,
    routes: Vec<RouteCatalogError>,
    protocols: Option<AukiProtocolError>,
    transport: Option<AukiPeerTransportError>,
    supervisor: Option<JoinError>,
}

impl ShutdownFailures {
    fn is_empty(&self) -> bool {
        self.relay.is_none()
            && self.discovery.is_none()
            && self.routes.is_empty()
            && self.protocols.is_none()
            && self.transport.is_none()
            && self.supervisor.is_none()
    }
}

#[derive(Debug, thiserror::Error)]
enum PeerTransportShutdownError {
    #[error("the P2P transport rejected graceful shutdown")]
    P2p(#[source] auki_p2p::Error),
    #[error("the P2P transport exceeded its graceful shutdown deadline")]
    Timeout,
}

/// Detailed transport-shutdown failure retained behind the facade boundary.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct AukiPeerTransportError(PeerTransportShutdownError);

/// Failures retained after every ordered cleanup stage has been attempted.
#[derive(Debug)]
pub struct AukiPeerShutdownError {
    failures: ShutdownFailures,
}

impl AukiPeerShutdownError {
    /// DDS advertisement withdrawal or publisher cleanup failure.
    pub fn discovery(&self) -> Option<&AukiDiscoveryError> {
        self.failures.discovery.as_ref()
    }

    /// Relay drain, booking deletion, or forced relay cleanup failure.
    pub fn relay(&self) -> Option<&AukiPeerRelayError> {
        self.failures.relay.as_ref()
    }

    /// Local route-catalog cleanup failures.
    pub fn routes(&self) -> &[RouteCatalogError] {
        &self.failures.routes
    }

    /// Managed protocol-server shutdown failure.
    pub fn protocols(&self) -> Option<&AukiProtocolError> {
        self.failures.protocols.as_ref()
    }

    /// Authenticated transport shutdown failure.
    pub fn transport(&self) -> Option<&AukiPeerTransportError> {
        self.failures.transport.as_ref()
    }

    /// Facade status-monitor join failure.
    pub fn supervisor(&self) -> Option<&JoinError> {
        self.failures.supervisor.as_ref()
    }
}

impl fmt::Display for AukiPeerShutdownError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = usize::from(self.failures.relay.is_some())
            + usize::from(self.failures.discovery.is_some())
            + self.failures.routes.len()
            + usize::from(self.failures.protocols.is_some())
            + usize::from(self.failures.transport.is_some())
            + usize::from(self.failures.supervisor.is_some());
        write!(
            formatter,
            "AukiPeer shutdown completed with {count} cleanup failure{}",
            if count == 1 { "" } else { "s" }
        )
    }
}

impl Error for AukiPeerShutdownError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let Some(error) = self.failures.discovery.as_ref() {
            return Some(error);
        }
        if let Some(error) = self.failures.relay.as_ref() {
            return Some(error);
        }
        if let Some(error) = self.failures.routes.first() {
            return Some(error);
        }
        if let Some(error) = self.failures.protocols.as_ref() {
            return Some(error);
        }
        if let Some(error) = self.failures.transport.as_ref() {
            return Some(error);
        }
        self.failures
            .supervisor
            .as_ref()
            .map(|error| error as &(dyn Error + 'static))
    }
}

#[derive(Clone)]
struct PeerStatusController {
    inner: Arc<Mutex<PeerStatusState>>,
    sender: watch::Sender<AukiPeerStatus>,
}

struct PeerStatusState {
    status: AukiPeerStatus,
    shutting_down: bool,
}

/// Clone-only observation of one native Peer's terminal lifecycle result.
///
/// The observer does not retain the Peer, relay booking, authority, or
/// transport. Every clone receives the same terminal result, including after
/// [`AukiPeer::shutdown`] consumes the owner.
#[derive(Clone)]
pub struct AukiPeerLifecycle {
    status: watch::Receiver<AukiPeerStatus>,
}

impl AukiPeerLifecycle {
    /// Wait until the native Peer has stopped or failed.
    pub async fn wait_stopped(&self) -> AukiPeerExit {
        wait_for_stopped_status(self.status.clone()).await
    }
}

impl PeerStatusController {
    fn new(status: AukiPeerStatus) -> Self {
        let (sender, _) = watch::channel(status);
        Self {
            inner: Arc::new(Mutex::new(PeerStatusState {
                status,
                shutting_down: false,
            })),
            sender,
        }
    }

    fn status(&self) -> AukiPeerStatus {
        self.inner.lock().status
    }

    fn subscribe(&self) -> watch::Receiver<AukiPeerStatus> {
        self.sender.subscribe()
    }

    fn update(&self, status: AukiPeerStatus) {
        let mut state = self.inner.lock();
        if state.shutting_down || state.status == status {
            return;
        }
        state.status = status;
        self.sender.send_replace(status);
    }

    fn begin_shutdown(&self) {
        let mut state = self.inner.lock();
        if state.shutting_down {
            return;
        }
        state.shutting_down = true;
        state.status = AukiPeerStatus::Stopping;
        self.sender.send_replace(AukiPeerStatus::Stopping);
    }

    fn finish_shutdown(&self, clean: bool) {
        let status = if clean {
            AukiPeerStatus::Stopped
        } else {
            AukiPeerStatus::Failed(AukiPeerFailure::Cleanup)
        };
        let mut state = self.inner.lock();
        state.shutting_down = true;
        state.status = status;
        self.sender.send_replace(status);
    }
}

enum StartupAuthority {
    Pull(PreparedPeer),
    External(ExternalAuthorityUpdate),
}

impl StartupAuthority {
    fn peer_id(&self) -> PeerId {
        match self {
            Self::Pull(prepared) => prepared.peer_id,
            Self::External(update) => update.peer_id(),
        }
    }

    fn domain_id(&self) -> Uuid {
        match self {
            Self::Pull(prepared) => prepared.domain.id,
            Self::External(update) => update.domain_id(),
        }
    }

    fn initial_verification_keys(&self) -> DdsVerificationKeys {
        match self {
            Self::Pull(prepared) => prepared.verification_keys.clone(),
            Self::External(update) => update.verification_keys().clone(),
        }
    }

    async fn start_supervisor(
        self,
        authority: FixedDomainAuthority,
    ) -> Result<(AuthoritySupervisor, StartupAuthorityControl), AuthoritySupervisorError> {
        let shutdown = CancellationToken::new();
        match self {
            Self::Pull(prepared) => AuthoritySupervisor::start_pull(
                authority,
                prepared,
                AuthoritySupervisorConfig::default(),
                &shutdown,
            )
            .await
            .map(|supervisor| (supervisor, StartupAuthorityControl::Pull)),
            Self::External(initial) => AuthoritySupervisor::start_external(
                authority,
                initial,
                AuthoritySupervisorConfig::default(),
                &shutdown,
            )
            .await
            .map(|(supervisor, handle, requests)| {
                (
                    supervisor,
                    StartupAuthorityControl::External(ExternalAuthorityControl::new(
                        handle, requests,
                    )),
                )
            }),
        }
    }
}

enum StartupAuthorityControl {
    Pull,
    External(ExternalAuthorityControl),
}

/// One owner of an authenticated transport, authority renewal, and optional relay booking.
///
/// At most one live `AukiPeer` in a process may own a given Peer ID. Reusing
/// that identity after a completed shutdown is supported; simultaneous
/// processes or pods using the same identity are outside this version's
/// supported deployment model.
pub struct AukiPeer {
    peer_id: PeerId,
    domain_id: Uuid,
    node: Option<Node>,
    listen_addresses: Vec<Multiaddr>,
    known_peers: AukiKnownPeers,
    protocols: AukiPeerProtocols,
    discovery: Option<DdsDiscovery>,
    discovery_publisher: Option<NativeDdsPublisher>,
    authority: Option<AuthoritySupervisor>,
    relay: Option<RelayBookingCoordinator>,
    relay_lifecycle: CancellationToken,
    route_catalog: RouteCatalog,
    protocol_context: AukiPeerProtocolContext,
    status: PeerStatusController,
    monitor_shutdown: CancellationToken,
    monitor: Option<JoinHandle<()>>,
    closed: bool,
}

impl AukiPeer {
    /// Join and retain one authenticated peer until all configured reachability is ready.
    ///
    /// Dropping this future cancels only process-local resources through RAII.
    /// It deliberately does not issue a cancellation-owned DMS `DELETE`; a
    /// booking created immediately before cancellation is allowed to expire at
    /// its requester-authority TTL.
    pub async fn start(
        identity: auki_p2p::Identity,
        prepared: PreparedPeer,
        config: AukiPeerConfig,
    ) -> Result<Self, AukiPeerStartError> {
        let (runtime, control) =
            Self::start_with_authority(identity, StartupAuthority::Pull(prepared), config).await?;
        match control {
            StartupAuthorityControl::Pull => Ok(runtime),
            StartupAuthorityControl::External(_) => {
                unreachable!("pull startup never creates external authority control")
            }
        }
    }

    /// Join a peer whose complete authority updates are supplied by its host.
    ///
    /// The returned [`ExternalAuthorityControl`] is the sole replacement and
    /// refresh-request control plane. It is withheld until the same Domain,
    /// authority, and optional relay readiness gates as [`Self::start`] pass.
    /// A relay authorization failure before then fails startup; the caller can
    /// obtain fresh authority and retry with a new update.
    pub async fn start_external(
        identity: auki_p2p::Identity,
        initial: ExternalAuthorityUpdate,
        config: AukiPeerConfig,
    ) -> Result<(Self, ExternalAuthorityControl), AukiPeerStartError> {
        let (runtime, control) =
            Self::start_with_authority(identity, StartupAuthority::External(initial), config)
                .await?;
        let StartupAuthorityControl::External(control) = control else {
            unreachable!("external startup always creates external authority control")
        };
        Ok((runtime, control))
    }

    async fn start_with_authority(
        identity: auki_p2p::Identity,
        initial_authority: StartupAuthority,
        config: AukiPeerConfig,
    ) -> Result<(Self, StartupAuthorityControl), AukiPeerStartError> {
        let identity_peer_id = identity.peer_id();
        let authorized_peer_id = initial_authority.peer_id();
        if authorized_peer_id != identity_peer_id {
            return Err(AukiPeerStartError::IdentityMismatch {
                authorized: authorized_peer_id,
                identity: identity_peer_id,
            });
        }
        let domain_id = initial_authority.domain_id();
        let verification_keys = initial_authority.initial_verification_keys();
        let verifier = DdsTokenVerifier::from_keys(verification_keys)
            .map_err(AukiPeerStartError::Transport)?;
        let node = Node::start(
            identity,
            verifier,
            config.listen_addresses().iter().cloned(),
        )
        .map_err(AukiPeerStartError::Transport)?;
        let mut listen_addresses =
            match tokio::time::timeout(LISTENER_STARTUP_TIMEOUT, node.wait_for_listeners()).await {
                Ok(Ok(addresses)) => addresses,
                Ok(Err(error)) => {
                    node.shutdown_now().await;
                    return Err(AukiPeerStartError::Transport(error));
                }
                Err(_) => {
                    node.shutdown_now().await;
                    return Err(AukiPeerStartError::ListenerStartupTimeout);
                }
            };
        listen_addresses.sort_unstable_by_key(ToString::to_string);
        let route_catalog = match RouteCatalog::new(
            identity_peer_id,
            config.advertised_direct_routes().to_vec(),
            config.route_catalog_limits(),
        ) {
            Ok(catalog) => catalog,
            Err(error) => {
                node.shutdown_now().await;
                return Err(AukiPeerStartError::Routes(error));
            }
        };
        let lifecycle = ContextLifecycle::new();
        let protocols = AukiPeerProtocols::new(
            node.clone(),
            domain_id,
            config
                .initial_peer_routes()
                .iter()
                .map(|initial| (initial.peer_id(), initial.routes().to_vec())),
            lifecycle.clone(),
        );
        let known_peers = AukiKnownPeers::new(domain_id, node.observations(), lifecycle.clone());
        let relay_lifecycle = CancellationToken::new();

        let (authority, authority_control) = match initial_authority
            .start_supervisor(FixedDomainAuthority::new(node.authority(), domain_id))
            .await
        {
            Ok(started) => started,
            Err(error) => {
                node.shutdown_now().await;
                return Err(AukiPeerStartError::Authority(error.into()));
            }
        };

        // Create each receiver exactly once. The same receivers fence the
        // final startup decision and are then moved into the live monitor, so
        // a transition at that handoff remains observable through watch state.
        let observations = node.observations();
        let mut signals = RuntimeSignals {
            node_events: observations.subscribe(),
            observations,
            authority: authority.subscribe_status(),
            routes: route_catalog.subscribe(),
            relay: None,
            relay_required: config.relay_required(),
        };

        let mut relay = match config.relay() {
            Some(relay_config) => {
                let client = match RelayBookingClient::new(
                    config.dms_base().clone(),
                    authority.relay_authorization(),
                ) {
                    Ok(client) => Arc::new(client),
                    Err(error) => {
                        authority.shutdown().await;
                        node.shutdown_now().await;
                        return Err(AukiPeerStartError::Relay(AukiPeerRelayError::client(error)));
                    }
                };
                let coordinator_config = match relay_coordinator_config(relay_config) {
                    Ok(config) => config,
                    Err(error) => {
                        authority.shutdown().await;
                        node.shutdown_now().await;
                        return Err(AukiPeerStartError::Relay(AukiPeerRelayError::client(error)));
                    }
                };
                let coordinator = loop {
                    match RelayBookingCoordinator::start(
                        client.clone(),
                        PeerRelayReservations::new(node.clone(), relay_lifecycle.clone()),
                        route_catalog.clone(),
                        coordinator_config.clone(),
                    )
                    .await
                    {
                        Ok(coordinator) => break coordinator,
                        Err(error) => {
                            let Some(retry_after) = error.startup_retry_after(
                                relay_config
                                    .status_poll_interval
                                    .min(RELAY_STARTUP_STATUS_POLL_INTERVAL),
                            ) else {
                                authority.shutdown().await;
                                node.shutdown_now().await;
                                return Err(AukiPeerStartError::Relay(
                                    AukiPeerRelayError::coordinator(error),
                                ));
                            };
                            warn!(
                                error = %error,
                                ?retry_after,
                                "retrying relay startup with the same idempotency key"
                            );
                            tokio::time::sleep(retry_after).await;
                        }
                    }
                };
                signals.relay = Some(coordinator.health());
                Some(coordinator)
            }
            None => None,
        };

        let initial_status = match signals.wait_until_ready().await {
            Ok(status) => status,
            Err(error) => {
                if let Some(coordinator) = relay {
                    match coordinator.shutdown(true, RELAY_SHUTDOWN_TIMEOUT).await {
                        Ok(RelayCoordinatorShutdownOutcome::Graceful) => {}
                        Ok(RelayCoordinatorShutdownOutcome::ForcedAfterTimeout) => {
                            warn!(
                                "relay startup cleanup timed out before DMS deletion was confirmed"
                            );
                        }
                        Err(cleanup_error) => {
                            warn!(error = %cleanup_error, "failed to clean up relay after startup failure");
                        }
                    }
                }
                authority.shutdown().await;
                relay_lifecycle.cancel();
                node.shutdown_now().await;
                return Err(error.into_start_error());
            }
        };

        let (discovery, discovery_publisher) = match config.dds_tracker() {
            Some(tracker) => {
                let discovery = match DdsDiscovery::new(
                    tracker,
                    domain_id,
                    identity_peer_id,
                    authority.dds_authorization(),
                ) {
                    Ok(discovery) => discovery,
                    Err(error) => {
                        cleanup_relay_after_startup_failure(relay.take()).await;
                        authority.shutdown().await;
                        relay_lifecycle.cancel();
                        node.shutdown_now().await;
                        return Err(AukiPeerStartError::Discovery(error));
                    }
                };
                let publisher = if tracker.mode() == DdsTrackerMode::DiscoverAndAdvertise {
                    match discovery
                        .start_native_publisher(
                            route_catalog.clone(),
                            protocols.clone(),
                            authority.subscribe_status(),
                        )
                        .await
                    {
                        Ok(publisher) => Some(publisher),
                        Err(error) => {
                            cleanup_relay_after_startup_failure(relay.take()).await;
                            authority.shutdown().await;
                            relay_lifecycle.cancel();
                            node.shutdown_now().await;
                            return Err(AukiPeerStartError::Discovery(error));
                        }
                    }
                } else {
                    None
                };
                (Some(discovery), publisher)
            }
            None => (None, None),
        };

        let protocol_context = AukiPeerProtocolContext::new(
            domain_id,
            identity_peer_id,
            authority.public_authorization(),
            protocols.clone(),
            route_catalog.clone(),
            lifecycle,
        );
        let status = PeerStatusController::new(initial_status);
        let monitor_shutdown = CancellationToken::new();
        let monitor = tokio::spawn(monitor_runtime(
            status.clone(),
            monitor_shutdown.clone(),
            protocols.clone(),
            discovery_publisher
                .as_ref()
                .map(NativeDdsPublisher::cancellation),
            signals,
        ));

        Ok((
            Self {
                peer_id: identity_peer_id,
                domain_id,
                node: Some(node),
                listen_addresses,
                known_peers,
                protocols,
                discovery,
                discovery_publisher,
                authority: Some(authority),
                relay,
                relay_lifecycle,
                route_catalog,
                protocol_context,
                status,
                monitor_shutdown,
                monitor: Some(monitor),
                closed: false,
            },
            authority_control,
        ))
    }

    /// Stable local libp2p Peer ID.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Exact authenticated DDS Domain UUID.
    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    /// Addresses that successfully bound before startup returned.
    pub fn listen_addresses(&self) -> &[Multiaddr] {
        &self.listen_addresses
    }

    /// Narrow authenticated custom-protocol and publication context.
    pub fn protocol_context(&self) -> AukiPeerProtocolContext {
        self.protocol_context.clone()
    }

    /// Authenticated application protocol registration and opening surface.
    pub fn protocols(&self) -> AukiPeerProtocols {
        self.protocols.clone()
    }

    /// Observational view of currently connected, mutually authenticated peers.
    pub fn known_peers(&self) -> AukiKnownPeers {
        self.known_peers.clone()
    }

    /// Fetch a fresh bounded list of untrusted same-Domain dial candidates.
    pub async fn discover(&self) -> Result<Vec<AukiDiscoveryCandidate>, AukiDiscoveryError> {
        self.discovery
            .as_ref()
            .ok_or(AukiDiscoveryError::Disabled)?
            .discover()
            .await
    }

    /// Clone the configured Rust-owned discovery capability for an async
    /// platform adapter without retaining this peer owner.
    pub fn discovery_handle(&self) -> Result<crate::AukiDiscovery, AukiDiscoveryError> {
        self.discovery
            .clone()
            .map(crate::AukiDiscovery::new)
            .ok_or(AukiDiscoveryError::Disabled)
    }

    /// Fetch fresh candidates advertising one exact inbound protocol ID.
    pub async fn discover_protocol(
        &self,
        protocol_id: impl AsRef<str>,
    ) -> Result<Vec<AukiDiscoveryCandidate>, AukiDiscoveryError> {
        self.discovery
            .as_ref()
            .ok_or(AukiDiscoveryError::Disabled)?
            .discover_protocol(protocol_id.as_ref())
            .await
    }

    /// Replace dial candidates for one expected peer after start.
    ///
    /// Empty `routes` clears the peer. Protocol `open` / client `fetch` use
    /// these candidates; phonebooks should call this after Discovery poll.
    pub fn replace_peer_routes(
        &self,
        expected_peer: PeerId,
        routes: impl IntoIterator<Item = Multiaddr>,
    ) -> Result<(), AukiProtocolError> {
        self.protocols
            .replace_peer_routes(expected_peer, routes)
    }

    /// Current facade lifecycle and readiness snapshot.
    pub fn status(&self) -> AukiPeerStatus {
        self.status.status()
    }

    /// Subscribe to facade lifecycle and readiness changes.
    pub fn subscribe_status(&self) -> watch::Receiver<AukiPeerStatus> {
        self.status.subscribe()
    }

    /// Retain a clone-only observer that outlives the consumed peer owner.
    pub fn lifecycle(&self) -> AukiPeerLifecycle {
        AukiPeerLifecycle {
            status: self.status.subscribe(),
        }
    }

    /// Wait for one stable terminal status without consuming the peer owner.
    ///
    /// Every waiter observes the retained final status. `Stopping` is skipped
    /// because ordered cleanup may still finish successfully or fail.
    pub async fn wait_stopped(&self) -> AukiPeerExit {
        self.lifecycle().wait_stopped().await
    }

    /// Drain relay reservations, stop protocols and authority, then stop the transport.
    ///
    /// Every cleanup stage is attempted even if an earlier stage fails. Await
    /// this method to completion when DMS booking deletion is required.
    #[allow(clippy::manual_async_fn)]
    pub fn shutdown(
        mut self,
    ) -> impl std::future::Future<Output = Result<(), AukiPeerShutdownError>> {
        // Fence while constructing the future. Retained protocol contexts must
        // reject new work even if the caller delays or cancels polling cleanup.
        self.protocol_context.fence();
        self.status.begin_shutdown();
        self.monitor_shutdown.cancel();
        let discovery_publisher = self.discovery_publisher.take();
        if let Some(publisher) = discovery_publisher.as_ref() {
            publisher.abort();
        }
        async move {
            let mut failures = ShutdownFailures::default();
            if let Some(monitor) = self.monitor.take()
                && let Err(error) = monitor.await
            {
                failures.supervisor = Some(error);
            }

            if let Some(publisher) = discovery_publisher
                && let Err(error) = publisher.shutdown().await
            {
                failures.discovery = Some(error);
            }

            if let Some(relay) = self.relay.take() {
                match relay.shutdown(true, RELAY_SHUTDOWN_TIMEOUT).await {
                    Ok(RelayCoordinatorShutdownOutcome::Graceful) => {}
                    Ok(RelayCoordinatorShutdownOutcome::ForcedAfterTimeout) => {
                        failures.relay = Some(AukiPeerRelayError::forced_shutdown());
                    }
                    Err(error) => failures.relay = Some(AukiPeerRelayError::shutdown(error)),
                }
            }
            self.relay_lifecycle.cancel();
            if let Err(error) = self.route_catalog.tombstone_all() {
                failures.routes.push(error);
            }
            if let Err(error) = self.route_catalog.replace_direct_routes(Vec::new()) {
                failures.routes.push(error);
            }
            if let Err(error) = self.protocols.shutdown_all().await {
                failures.protocols = Some(error);
            }
            if let Some(authority) = self.authority.take() {
                authority.shutdown().await;
            }
            // Keep the owner slot populated across the asynchronous barrier. If
            // this shutdown future is canceled, `Drop` can still force the node
            // down even when public protocol handles retain their own clones.
            if let Some(node) = self.node.as_ref() {
                match tokio::time::timeout(NODE_SHUTDOWN_TIMEOUT, node.shutdown()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        failures.transport = Some(AukiPeerTransportError(
                            PeerTransportShutdownError::P2p(error),
                        ));
                    }
                    Err(_) => {
                        node.shutdown_now().await;
                        failures.transport =
                            Some(AukiPeerTransportError(PeerTransportShutdownError::Timeout));
                    }
                }
            }
            drop(self.node.take());

            let clean = failures.is_empty();
            self.status.finish_shutdown(clean);
            self.closed = true;
            if clean {
                Ok(())
            } else {
                Err(AukiPeerShutdownError { failures })
            }
        }
    }
}

impl Drop for AukiPeer {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        self.protocol_context.fence();
        self.status.begin_shutdown();
        self.monitor_shutdown.cancel();
        if let Some(monitor) = self.monitor.take() {
            monitor.abort();
        }
        if let Some(publisher) = self.discovery_publisher.take() {
            publisher.abort();
            drop(publisher);
        }
        drop(self.relay.take());
        self.relay_lifecycle.cancel();
        let route_cleanup_failed = self.route_catalog.tombstone_all().is_err()
            | self
                .route_catalog
                .replace_direct_routes(Vec::new())
                .is_err();
        drop(self.authority.take());
        self.protocols.abort_all();
        if let Some(node) = self.node.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move { node.shutdown_now().await });
        }
        if route_cleanup_failed {
            warn!("AukiPeer drop could not completely clear its local route catalog");
        }
        // Only the awaited shutdown path may publish Stopped: Drop performs
        // local fencing but deliberately cannot confirm DMS DELETE or await
        // the transport's asynchronous cleanup barrier.
        self.closed = true;
    }
}

async fn cleanup_relay_after_startup_failure(relay: Option<RelayBookingCoordinator>) {
    let Some(relay) = relay else {
        return;
    };
    match relay.shutdown(true, RELAY_SHUTDOWN_TIMEOUT).await {
        Ok(RelayCoordinatorShutdownOutcome::Graceful) => {}
        Ok(RelayCoordinatorShutdownOutcome::ForcedAfterTimeout) => {
            warn!("relay discovery-startup cleanup timed out before DMS deletion was confirmed");
        }
        Err(error) => {
            warn!(error = %error, "failed to clean up relay after discovery startup failure");
        }
    }
}

fn relay_coordinator_config(
    relay: AukiRelayConfig,
) -> Result<RelayCoordinatorConfig, RelayBookingClientError> {
    Ok(RelayCoordinatorConfig {
        idempotency_key: RelayIdempotencyKey::new(format!("auki-sdk-relay-{}", Uuid::new_v4()))?,
        mode: booking_mode(relay),
        requested_duration_seconds: relay.requested_duration.as_secs(),
        relay_count: relay.relay_count,
        status_poll_interval: relay.status_poll_interval,
        reservation_retry_budget: RELAY_RESERVATION_RETRY_BUDGET,
        retry_min: RELAY_RETRY_MIN,
        retry_max: RELAY_RETRY_MAX,
        http_timeout: RELAY_HTTP_TIMEOUT,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupReadinessError {
    Transport,
    Authority,
    Relay,
    Supervisor,
}

impl StartupReadinessError {
    fn into_start_error(self) -> AukiPeerStartError {
        match self {
            Self::Transport => AukiPeerStartError::TransportUnavailable,
            Self::Authority => AukiPeerStartError::AuthorityUnavailable,
            Self::Relay => AukiPeerStartError::Relay(AukiPeerRelayError::unavailable()),
            Self::Supervisor => AukiPeerStartError::Supervisor,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupReadiness {
    Waiting,
    Ready,
}

fn startup_readiness(
    transport: NodeObservationStatus,
    authority: &AuthorityStatus,
    confirmed_relay_count: usize,
    relay_failed: bool,
    relay_required: bool,
) -> Result<StartupReadiness, StartupReadinessError> {
    if relay_required && relay_failed {
        return Err(StartupReadinessError::Relay);
    }
    let transport_ready = match transport {
        NodeObservationStatus::Running => true,
        NodeObservationStatus::Failed(_) | NodeObservationStatus::Stopped => {
            return Err(StartupReadinessError::Transport);
        }
    };
    let authority_ready = match authority {
        AuthorityStatus::Ready { .. } => true,
        AuthorityStatus::Starting | AuthorityStatus::Expired { .. } => false,
        AuthorityStatus::Stopped => return Err(StartupReadinessError::Authority),
    };
    let reachability_ready = !relay_required || confirmed_relay_count > 0;
    Ok(
        if transport_ready && authority_ready && reachability_ready {
            StartupReadiness::Ready
        } else {
            StartupReadiness::Waiting
        },
    )
}

struct RuntimeSignals {
    observations: NodeObservations,
    node_events: broadcast::Receiver<NodeObservationEvent>,
    authority: watch::Receiver<AuthorityStatus>,
    routes: watch::Receiver<RouteCatalogStatus>,
    relay: Option<RelayCoordinatorHealth>,
    relay_required: bool,
}

impl RuntimeSignals {
    fn snapshot(&self) -> RuntimeSignalSnapshot {
        RuntimeSignalSnapshot {
            transport: self.observations.snapshot().status(),
            authority: self.authority.borrow().clone(),
            routes: self.routes.borrow().clone(),
            relay_failed: self
                .relay
                .as_ref()
                .is_some_and(RelayCoordinatorHealth::is_failed),
        }
    }

    async fn wait_until_ready(&mut self) -> Result<AukiPeerStatus, StartupReadinessError> {
        loop {
            let snapshot = self.snapshot();
            match startup_readiness(
                snapshot.transport,
                &snapshot.authority,
                snapshot.routes.confirmed_relay_count,
                snapshot.relay_failed,
                self.relay_required,
            )? {
                StartupReadiness::Ready => {
                    return Ok(snapshot.facade_status(self.relay_required));
                }
                StartupReadiness::Waiting => self.changed().await?,
            }
        }
    }

    async fn changed(&mut self) -> Result<(), StartupReadinessError> {
        tokio::select! {
            changed = self.routes.changed() => {
                changed.map_err(|_| StartupReadinessError::Supervisor)
            }
            _ = async {
                match self.relay.as_mut() {
                    Some(relay) => relay.failed().await,
                    None => pending::<()>().await,
                }
            } => Ok(()),
            event = self.node_events.recv() => {
                match event {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => Ok(()),
                    Err(broadcast::error::RecvError::Closed) => Err(StartupReadinessError::Supervisor),
                }
            }
            changed = self.authority.changed() => {
                changed.map_err(|_| StartupReadinessError::Supervisor)
            }
        }
    }
}

struct RuntimeSignalSnapshot {
    transport: NodeObservationStatus,
    authority: AuthorityStatus,
    routes: RouteCatalogStatus,
    relay_failed: bool,
}

impl RuntimeSignalSnapshot {
    fn facade_status(&self, relay_required: bool) -> AukiPeerStatus {
        observed_status(
            self.transport,
            &self.authority,
            &self.routes,
            self.relay_failed,
            relay_required,
        )
    }
}

async fn monitor_runtime(
    status: PeerStatusController,
    shutdown: CancellationToken,
    protocols: AukiPeerProtocols,
    discovery_shutdown: Option<CancellationToken>,
    mut signals: RuntimeSignals,
) {
    loop {
        let observed = signals.snapshot().facade_status(signals.relay_required);
        if observed.is_terminal() {
            // Fence before publishing the terminal result so a waiter can never
            // observe failure while a retained protocol handle still accepts
            // new work.
            protocols.abort_all();
            if let Some(discovery_shutdown) = &discovery_shutdown {
                discovery_shutdown.cancel();
            }
            status.update(observed);
            return;
        }
        status.update(observed);
        let changed = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            changed = signals.changed() => changed,
        };
        if changed.is_err() {
            protocols.abort_all();
            if let Some(discovery_shutdown) = &discovery_shutdown {
                discovery_shutdown.cancel();
            }
            status.update(AukiPeerStatus::Failed(AukiPeerFailure::Supervisor));
            return;
        }
    }
}

async fn wait_for_stopped_status(mut status: watch::Receiver<AukiPeerStatus>) -> AukiPeerExit {
    loop {
        let observed = *status.borrow_and_update();
        match observed {
            AukiPeerStatus::Stopped => return AukiPeerExit::Stopped,
            AukiPeerStatus::Failed(failure) => return AukiPeerExit::Failed(failure),
            AukiPeerStatus::Ready
            | AukiPeerStatus::AuthorityUnavailable
            | AukiPeerStatus::RelayUnavailable
            | AukiPeerStatus::Stopping => {}
        }
        if status.changed().await.is_err() {
            return AukiPeerExit::Failed(AukiPeerFailure::Supervisor);
        }
    }
}

fn observed_status(
    transport: NodeObservationStatus,
    authority: &AuthorityStatus,
    routes: &RouteCatalogStatus,
    relay_failed: bool,
    relay_required: bool,
) -> AukiPeerStatus {
    if matches!(
        transport,
        NodeObservationStatus::Failed(_) | NodeObservationStatus::Stopped
    ) {
        return AukiPeerStatus::Failed(AukiPeerFailure::Transport);
    }
    if matches!(authority, AuthorityStatus::Stopped) {
        return AukiPeerStatus::Failed(AukiPeerFailure::Authority);
    }
    if relay_failed {
        return AukiPeerStatus::Failed(AukiPeerFailure::Relay);
    }
    if matches!(
        authority,
        AuthorityStatus::Starting | AuthorityStatus::Expired { .. }
    ) {
        return AukiPeerStatus::AuthorityUnavailable;
    }
    if relay_required && routes.confirmed_relay_count == 0 {
        return AukiPeerStatus::RelayUnavailable;
    }
    AukiPeerStatus::Ready
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, str::FromStr, time::SystemTime};

    use async_trait::async_trait;
    use auki_auth::{
        AuthorityRenewal, AuthorityRenewalProvider, DomainDescriptor, RenewedAuthority,
    };
    use auki_p2p::{
        DdsVerificationKeys, P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_SCOPE, P2P_TOKEN_TTL,
        P2P_TOKEN_TYPE, P2PAccessClaims, RouteCatalogLimits, SignedP2pCredential,
    };
    use chrono::{TimeZone, Utc};
    use httpmock::{Method::DELETE, Method::GET, Method::POST, Method::PUT, MockServer};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde_json::json;

    use super::*;
    use crate::{AukiPeerAuthorizationError, AukiPeerRoutesError, AukiRelayMode, DdsTrackerConfig};

    const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

    const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

    struct NeverRenew;

    #[async_trait]
    impl AuthorityRenewalProvider for NeverRenew {
        async fn renew_authority(
            &self,
            _cancellation: &CancellationToken,
        ) -> auki_auth::Result<RenewedAuthority> {
            pending().await
        }
    }

    fn unix_time() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn verification_keys() -> DdsVerificationKeys {
        DdsVerificationKeys::new(0, TEST_DDS_PUBLIC_KEY.to_vec(), None)
    }

    fn signed_credential(
        identity: &auki_p2p::Identity,
        domain_id: Uuid,
        issued_at: u64,
        peer_type: &str,
    ) -> (SignedP2pCredential, chrono::DateTime<Utc>, String) {
        let expiration = issued_at + P2P_TOKEN_TTL.as_secs();
        let claims = P2PAccessClaims {
            token_type: P2P_TOKEN_TYPE.into(),
            iss: P2P_TOKEN_ISSUER.into(),
            aud: vec![P2P_TOKEN_AUDIENCE.into()],
            sub: Uuid::new_v4().to_string(),
            organization_id: None,
            peer_type: Some(peer_type.into()),
            peer_id: identity.peer_id().to_string(),
            domain_ids: vec![domain_id.to_string()],
            scopes: vec![P2P_TOKEN_SCOPE.into()],
            application: None,
            iat: issued_at,
            nbf: None,
            exp: expiration,
        };
        let compact = encode(
            &Header::new(Algorithm::ES256),
            &claims,
            &EncodingKey::from_ec_pem(TEST_DDS_PRIVATE_KEY).unwrap(),
        )
        .unwrap();
        (
            SignedP2pCredential::new(compact.clone()).unwrap(),
            Utc.timestamp_opt(expiration as i64, 0).unwrap(),
            compact,
        )
    }

    fn fixture(identity: &auki_p2p::Identity, domain_id: Uuid) -> PreparedPeer {
        let (credential, expires_at, _) =
            signed_credential(identity, domain_id, unix_time(), "robot");
        PreparedPeer {
            domain: DomainDescriptor::assigned(domain_id),
            peer_id: identity.peer_id(),
            initial_credential: credential,
            verification_keys: verification_keys(),
            credential_expires_at: expires_at,
            renew_at: expires_at - chrono::Duration::minutes(1),
            renewal: AuthorityRenewal::new(NeverRenew),
        }
    }

    fn external_fixture(
        identity: &auki_p2p::Identity,
        domain_id: Uuid,
        issued_at: u64,
    ) -> (ExternalAuthorityUpdate, String) {
        let (credential, expires_at, compact) =
            signed_credential(identity, domain_id, issued_at, "compute");
        (
            ExternalAuthorityUpdate::new(
                domain_id,
                identity.peer_id(),
                verification_keys(),
                credential,
                expires_at,
            ),
            compact,
        )
    }

    fn external_update_from_compact(
        identity: &auki_p2p::Identity,
        domain_id: Uuid,
        compact: String,
        expires_at: chrono::DateTime<Utc>,
    ) -> ExternalAuthorityUpdate {
        ExternalAuthorityUpdate::new(
            domain_id,
            identity.peer_id(),
            verification_keys(),
            SignedP2pCredential::new(compact).unwrap(),
            expires_at,
        )
    }

    fn direct_config() -> AukiPeerConfig {
        AukiPeerConfig::new("http://127.0.0.1:9")
            .unwrap()
            .direct_only()
            .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn direct_only_runtime_is_ready_and_ordered_shutdown_fences_every_context_view() {
        let identity = auki_p2p::Identity::generate();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let advertised =
            Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/4001/p2p/{peer_id}")).unwrap();
        let config = direct_config()
            .with_advertised_direct_routes([advertised])
            .unwrap();
        let prepared = fixture(&identity, domain_id);
        let runtime = AukiPeer::start(identity, prepared, config).await.unwrap();

        assert_eq!(runtime.peer_id(), peer_id);
        assert_eq!(runtime.domain_id(), domain_id);
        assert_eq!(runtime.status(), AukiPeerStatus::Ready);
        assert_eq!(runtime.listen_addresses().len(), 1);
        assert_ne!(
            runtime.listen_addresses()[0].to_string(),
            "/ip4/127.0.0.1/tcp/0"
        );
        assert_eq!(runtime.known_peers().peer_count(), 0);
        let protocols = runtime.protocols();
        assert_eq!(protocols.peer_id(), peer_id);
        assert_eq!(protocols.domain_id(), domain_id);
        let registration = protocols
            .register(
                crate::AukiProtocolSpec::new("/example/runtime/1.0.0", 1, 32).unwrap(),
                |_stream| async {},
            )
            .unwrap();
        assert!(matches!(
            protocols.register(
                crate::AukiProtocolSpec::new("/example/runtime/1.0.0", 1, 32).unwrap(),
                |_stream| async {},
            ),
            Err(AukiProtocolError::DuplicateProtocol(protocol))
                if protocol == "/example/runtime/1.0.0"
        ));

        let context = runtime.protocol_context();
        let current = context.authorization().current().unwrap();
        assert_eq!(current.credential_revision(), 1);
        assert_eq!(current.peer_type(), Some("robot"));
        let route_snapshot = context.routes().snapshot().unwrap();
        assert_eq!(
            route_snapshot.direct_routes,
            [Multiaddr::from_str("/ip4/127.0.0.1/tcp/4001").unwrap()]
        );
        assert!(route_snapshot.relay_routes.is_empty());
        let mut statuses = runtime.subscribe_status();

        runtime.shutdown().await.unwrap();
        registration.close().await.unwrap();
        assert!(matches!(
            protocols.register(
                crate::AukiProtocolSpec::new("/example/after-stop/1.0.0", 1, 32).unwrap(),
                |_stream| async {},
            ),
            Err(AukiProtocolError::Stopped)
        ));
        assert_eq!(*statuses.borrow_and_update(), AukiPeerStatus::Stopped);
        assert_eq!(
            context.authorization().current().unwrap_err(),
            AukiPeerAuthorizationError::Stopped
        );
        assert!(matches!(
            context.routes().snapshot(),
            Err(AukiPeerRoutesError::Stopped)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lifecycle_clone_observes_clean_stop_after_shutdown_consumes_peer() {
        let identity = auki_p2p::Identity::generate();
        let prepared = fixture(&identity, Uuid::new_v4());
        let runtime = AukiPeer::start(identity, prepared, direct_config())
            .await
            .unwrap();
        let lifecycle = runtime.lifecycle();
        let waiter = tokio::spawn({
            let lifecycle = lifecycle.clone();
            async move { lifecycle.wait_stopped().await }
        });

        runtime.shutdown().await.unwrap();

        assert_eq!(waiter.await.unwrap(), AukiPeerExit::Stopped);
        assert_eq!(lifecycle.wait_stopped().await, AukiPeerExit::Stopped);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn discovery_is_explicit_and_advertising_requires_a_publishable_route() {
        let identity = auki_p2p::Identity::generate();
        let domain_id = Uuid::new_v4();
        let runtime = AukiPeer::start(
            identity.clone(),
            fixture(&identity, domain_id),
            direct_config(),
        )
        .await
        .unwrap();
        assert_eq!(
            runtime.discover().await.unwrap_err(),
            AukiDiscoveryError::Disabled
        );
        runtime.shutdown().await.unwrap();

        let dds = MockServer::start();
        let discovering_identity = auki_p2p::Identity::generate();
        let discover_only = direct_config().with_dds_tracker(
            DdsTrackerConfig::new(dds.base_url(), DdsTrackerMode::DiscoverOnly).unwrap(),
        );
        let discover_only_runtime = AukiPeer::start(
            discovering_identity.clone(),
            fixture(&discovering_identity, domain_id),
            discover_only,
        )
        .await
        .expect("discover-only startup must not contact an unavailable tracker");
        discover_only_runtime.shutdown().await.unwrap();

        let advertising_identity = auki_p2p::Identity::generate();
        let config = direct_config().with_dds_tracker(
            DdsTrackerConfig::new(dds.base_url(), DdsTrackerMode::DiscoverAndAdvertise).unwrap(),
        );
        assert!(matches!(
            AukiPeer::start(
                advertising_identity.clone(),
                fixture(&advertising_identity, domain_id),
                config,
            )
            .await,
            Err(AukiPeerStartError::Discovery(
                AukiDiscoveryError::InvalidResponse { .. }
            ))
        ));

        let route = Multiaddr::from_str("/ip4/127.0.0.1/tcp/4001").unwrap();
        let rejected = dds.mock(|when, then| {
            when.method(PUT);
            then.status(400).header("cache-control", "no-store");
        });
        let rejected_cleanup = dds.mock(|when, then| {
            when.method(DELETE).path(format!(
                "/api/v1/domains/{domain_id}/p2p/advertisements/self"
            ));
            then.status(204).header("cache-control", "no-store");
        });
        let rejected_identity = auki_p2p::Identity::generate();
        let config = direct_config()
            .with_advertised_direct_routes([route])
            .unwrap()
            .with_dds_tracker(
                DdsTrackerConfig::new(dds.base_url(), DdsTrackerMode::DiscoverAndAdvertise)
                    .unwrap(),
            );
        assert!(matches!(
            AukiPeer::start(
                rejected_identity.clone(),
                fixture(&rejected_identity, domain_id),
                config,
            )
            .await,
            Err(AukiPeerStartError::Discovery(
                AukiDiscoveryError::HttpStatus { status: 400, .. }
            ))
        ));
        rejected.assert_calls(1);
        rejected_cleanup.assert_calls(1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unexpected_transport_stop_fences_protocols_before_waiters_observe_failure() {
        let identity = auki_p2p::Identity::generate();
        let prepared = fixture(&identity, Uuid::new_v4());
        let runtime = AukiPeer::start(identity, prepared, direct_config())
            .await
            .unwrap();
        let protocols = runtime.protocols();
        let node = runtime.node.as_ref().unwrap().clone();

        node.shutdown_now().await;
        let terminal = tokio::time::timeout(Duration::from_secs(2), runtime.wait_stopped())
            .await
            .expect("transport failure must become observable");
        assert_eq!(terminal, AukiPeerExit::Failed(AukiPeerFailure::Transport));
        assert_eq!(runtime.wait_stopped().await, terminal);
        assert!(matches!(
            protocols.register(
                crate::AukiProtocolSpec::new("/example/fenced/1.0.0", 1, 32).unwrap(),
                |_stream| async {},
            ),
            Err(AukiProtocolError::Stopped)
        ));

        let _ = runtime.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn constructing_shutdown_future_synchronously_fences_every_context_view() {
        let identity = auki_p2p::Identity::generate();
        let prepared = fixture(&identity, Uuid::new_v4());
        let runtime = AukiPeer::start(identity, prepared, direct_config())
            .await
            .unwrap();
        let context = runtime.protocol_context();
        let protocols = runtime.protocols();
        let status = runtime.subscribe_status();

        let shutdown = runtime.shutdown();

        assert_eq!(*status.borrow(), AukiPeerStatus::Stopping);
        assert!(matches!(
            protocols.register(
                crate::AukiProtocolSpec::new("/example/shutdown-fence/1.0.0", 1, 32).unwrap(),
                |_stream| async {},
            ),
            Err(AukiProtocolError::Stopped)
        ));
        assert!(matches!(
            context.authorization().current(),
            Err(AukiPeerAuthorizationError::Stopped)
        ));
        assert!(matches!(
            context.routes().snapshot(),
            Err(AukiPeerRoutesError::Stopped)
        ));

        drop(shutdown);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn known_peer_snapshot_is_cleared_by_the_runtime_fence() {
        let server_identity = auki_p2p::Identity::generate();
        let server_peer_id = server_identity.peer_id();
        let client_identity = auki_p2p::Identity::generate();
        let client_peer_id = client_identity.peer_id();
        let domain_id = Uuid::new_v4();

        let server = AukiPeer::start(
            server_identity.clone(),
            fixture(&server_identity, domain_id),
            direct_config(),
        )
        .await
        .unwrap();
        let (accepted_sender, accepted_receiver) = tokio::sync::oneshot::channel();
        let accepted = Arc::new(Mutex::new(Some(accepted_sender)));
        let handler_accepted = Arc::clone(&accepted);
        let registration = server
            .protocols()
            .register(
                crate::AukiProtocolSpec::new("/example/known-peers/1.0.0", 1, 1).unwrap(),
                move |_stream| {
                    let accepted = handler_accepted.lock().take();
                    async move {
                        if let Some(accepted) = accepted {
                            let _ = accepted.send(());
                        }
                        pending::<()>().await;
                    }
                },
            )
            .unwrap();

        let client_config = direct_config()
            .with_peer_routes(server_peer_id, server.listen_addresses().iter().cloned())
            .unwrap();
        let client = AukiPeer::start(
            client_identity.clone(),
            fixture(&client_identity, domain_id),
            client_config,
        )
        .await
        .unwrap();
        let stream = client
            .protocols()
            .open(server_peer_id, "/example/known-peers/1.0.0")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), accepted_receiver)
            .await
            .unwrap()
            .unwrap();

        let known_peers = server.known_peers();
        assert_eq!(known_peers.snapshot().peers()[0].peer_id(), client_peer_id);

        server.shutdown().await.unwrap();
        assert!(known_peers.snapshot().peers().is_empty());

        drop(stream);
        registration.close().await.unwrap();
        client.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn discovery_publishes_mounts_and_exact_dials_without_a_pasted_peer_card() {
        const PROTOCOL: &str = "/example/discovery-e2e/1.0.0";

        let dds = MockServer::start();
        let domain_id = Uuid::new_v4();
        let advertising_identity = auki_p2p::Identity::generate();
        let advertising_peer_id = advertising_identity.peer_id();
        let discovering_identity = auki_p2p::Identity::generate();

        let socket = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let route = Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{port}")).unwrap();
        let collection_path = format!("/api/v1/domains/{domain_id}/p2p/advertisements");
        let self_path = format!("{collection_path}/self");
        let expires_at = Utc::now() + chrono::Duration::minutes(2);

        let initial_publish = dds.mock(|when, then| {
            when.method(PUT).path(self_path.clone()).json_body(json!({
                "routes": [route.to_string()],
                "protocols": [],
            }));
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "peer_id": advertising_peer_id.to_string(),
                    "routes": [route.to_string()],
                    "protocols": [],
                    "expires_at": expires_at,
                }));
        });
        let mounted_publish = dds.mock(|when, then| {
            when.method(PUT).path(self_path.clone()).json_body(json!({
                "routes": [route.to_string()],
                "protocols": [PROTOCOL],
            }));
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "peer_id": advertising_peer_id.to_string(),
                    "routes": [route.to_string()],
                    "protocols": [PROTOCOL],
                    "expires_at": expires_at,
                }));
        });
        let list = dds.mock(|when, then| {
            when.method(GET)
                .path(collection_path)
                .query_param("limit", "100")
                .query_param("protocol", PROTOCOL);
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "advertisements": [{
                        "peer_id": advertising_peer_id.to_string(),
                        "routes": [route.to_string()],
                        "protocols": [PROTOCOL],
                        "expires_at": expires_at,
                    }]
                }));
        });
        let withdraw = dds.mock(|when, then| {
            when.method(DELETE).path(self_path);
            then.status(204).header("cache-control", "no-store");
        });

        let advertising_config = AukiPeerConfig::new("http://127.0.0.1:9")
            .unwrap()
            .direct_only()
            .with_listen_addresses([route.clone()])
            .unwrap()
            .with_advertised_direct_routes([route.clone()])
            .unwrap()
            .with_dds_tracker(
                DdsTrackerConfig::new(dds.base_url(), DdsTrackerMode::DiscoverAndAdvertise)
                    .unwrap(),
            );
        let advertising = AukiPeer::start(
            advertising_identity.clone(),
            fixture(&advertising_identity, domain_id),
            advertising_config,
        )
        .await
        .unwrap();
        initial_publish.assert_calls(1);

        let (accepted_sender, accepted_receiver) = tokio::sync::oneshot::channel();
        let accepted = Arc::new(Mutex::new(Some(accepted_sender)));
        let handler_accepted = Arc::clone(&accepted);
        let registration = advertising
            .protocols()
            .register(
                crate::AukiProtocolSpec::new(PROTOCOL, 1, 32).unwrap(),
                move |_stream| {
                    let accepted = handler_accepted.lock().take();
                    async move {
                        if let Some(accepted) = accepted {
                            let _ = accepted.send(());
                        }
                        pending::<()>().await;
                    }
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while mounted_publish.calls() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("protocol mount must asynchronously replace the advertisement");

        let discovering_config = direct_config().with_dds_tracker(
            DdsTrackerConfig::new(dds.base_url(), DdsTrackerMode::DiscoverOnly).unwrap(),
        );
        let discovering = AukiPeer::start(
            discovering_identity.clone(),
            fixture(&discovering_identity, domain_id),
            discovering_config,
        )
        .await
        .unwrap();
        let candidates = discovering.discover_protocol(PROTOCOL).await.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].peer_id(), advertising_peer_id);
        assert_eq!(candidates[0].routes(), std::slice::from_ref(&route));
        assert!(discovering.known_peers().snapshot().peers().is_empty());

        let stream = discovering
            .protocols()
            .open_exact(
                candidates[0].peer_id(),
                candidates[0].routes()[0].clone(),
                PROTOCOL,
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), accepted_receiver)
            .await
            .expect("discovered exact route must reach the mounted protocol")
            .unwrap();
        assert_eq!(
            discovering.known_peers().snapshot().peers()[0].peer_id(),
            advertising_peer_id
        );
        drop(stream);
        list.assert_calls(1);

        let retained_discovery = discovering.discovery_handle().unwrap();
        discovering.shutdown().await.unwrap();
        assert_eq!(
            retained_discovery.discover().await.unwrap_err(),
            AukiDiscoveryError::Authentication
        );
        advertising.shutdown().await.unwrap();
        registration.close().await.unwrap();
        withdraw.assert_calls(1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn external_direct_only_runtime_replaces_authority_and_shutdown_ends_control() {
        let identity = auki_p2p::Identity::generate();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time().saturating_sub(2);
        let (initial, initial_compact) = external_fixture(&identity, domain_id, issued_at);
        let initial_expiration = initial.credential_expires_at();

        assert_eq!(initial.domain_id(), domain_id);
        assert_eq!(initial.peer_id(), peer_id);
        assert_eq!(initial.verification_key_generation(), 0);
        let rendered = format!("{initial:?}");
        assert!(rendered.contains("[redacted]"));
        assert!(!rendered.contains(&initial_compact));
        assert!(!rendered.contains("BEGIN PUBLIC KEY"));

        let (runtime, control) =
            AukiPeer::start_external(identity.clone(), initial, direct_config())
                .await
                .unwrap();
        assert_eq!(runtime.status(), AukiPeerStatus::Ready);
        assert_eq!(runtime.domain_id(), domain_id);
        assert_eq!(
            runtime
                .protocol_context()
                .authorization()
                .current()
                .unwrap()
                .peer_type(),
            Some("compute")
        );

        let (replacement, replacement_compact) =
            external_fixture(&identity, domain_id, issued_at + 1);
        let replacement_expiration = replacement.credential_expires_at();
        let outcome = control.replace(replacement).await.unwrap();
        assert_eq!(
            outcome,
            ExternalAuthorityReplaceOutcome::Replaced {
                credential_revision: 2
            }
        );
        assert_eq!(outcome.credential_revision(), 2);
        assert_eq!(
            runtime
                .protocol_context()
                .authorization()
                .current()
                .unwrap()
                .credential_revision(),
            2
        );

        let duplicate = external_update_from_compact(
            &identity,
            domain_id,
            replacement_compact,
            replacement_expiration,
        );
        assert_eq!(
            control.replace(duplicate).await.unwrap(),
            ExternalAuthorityReplaceOutcome::Unchanged {
                credential_revision: 2
            }
        );

        let control = Arc::new(control);
        let waiting_control = Arc::clone(&control);
        let waiting = tokio::spawn(async move { waiting_control.next_refresh_request().await });
        tokio::task::yield_now().await;

        runtime.shutdown().await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap(),
            None
        );
        let stopped_update =
            external_update_from_compact(&identity, domain_id, initial_compact, initial_expiration);
        let stopped = control.replace(stopped_update).await.unwrap_err();
        assert!(stopped.to_string().contains("stopped"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn externally_managed_authority_republishes_the_same_self_advertisement() {
        let dds = MockServer::start();
        let identity = auki_p2p::Identity::generate();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let route = Multiaddr::from_str("/ip4/127.0.0.1/tcp/4001").unwrap();
        let path = format!("/api/v1/domains/{domain_id}/p2p/advertisements/self");
        let publication = dds.mock(|when, then| {
            when.method(PUT).path(path.clone()).json_body(json!({
                "routes": [route.to_string()],
                "protocols": [],
            }));
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "peer_id": peer_id.to_string(),
                    "routes": [route.to_string()],
                    "protocols": [],
                    "expires_at": Utc::now() + chrono::Duration::minutes(2),
                }));
        });
        let withdrawal = dds.mock(|when, then| {
            when.method(DELETE).path(path);
            then.status(204).header("cache-control", "no-store");
        });
        let config = direct_config()
            .with_advertised_direct_routes([route])
            .unwrap()
            .with_dds_tracker(
                DdsTrackerConfig::new(dds.base_url(), DdsTrackerMode::DiscoverAndAdvertise)
                    .unwrap(),
            );
        let issued_at = unix_time().saturating_sub(2);
        let (initial, _) = external_fixture(&identity, domain_id, issued_at);
        let (runtime, control) = AukiPeer::start_external(identity.clone(), initial, config)
            .await
            .unwrap();
        publication.assert_calls(1);

        let (replacement, _) = external_fixture(&identity, domain_id, issued_at + 1);
        assert!(matches!(
            control.replace(replacement).await.unwrap(),
            ExternalAuthorityReplaceOutcome::Replaced { .. }
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            while publication.calls() < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("authority revision must asynchronously refresh the advertisement");

        runtime.shutdown().await.unwrap();
        withdrawal.assert_calls(1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn constructing_shutdown_future_stops_discovery_before_it_is_polled() {
        let dds = MockServer::start();
        let identity = auki_p2p::Identity::generate();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let route = Multiaddr::from_str("/ip4/127.0.0.1/tcp/4001").unwrap();
        let path = format!("/api/v1/domains/{domain_id}/p2p/advertisements/self");
        let publication = dds.mock(|when, then| {
            when.method(PUT).path(path.clone());
            then.status(200)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "peer_id": peer_id.to_string(),
                    "routes": [route.to_string()],
                    "protocols": [],
                    "expires_at": Utc::now() + chrono::Duration::minutes(2),
                }));
        });
        let withdrawal = dds.mock(|when, then| {
            when.method(DELETE).path(path);
            then.status(204).header("cache-control", "no-store");
        });
        let config = direct_config()
            .with_advertised_direct_routes([route])
            .unwrap()
            .with_dds_tracker(
                DdsTrackerConfig::new(dds.base_url(), DdsTrackerMode::DiscoverAndAdvertise)
                    .unwrap(),
            );
        let issued_at = unix_time().saturating_sub(2);
        let (initial, _) = external_fixture(&identity, domain_id, issued_at);
        let (runtime, control) = AukiPeer::start_external(identity.clone(), initial, config)
            .await
            .unwrap();
        publication.assert_calls(1);

        let unpolled_shutdown = runtime.shutdown();
        let (replacement, _) = external_fixture(&identity, domain_id, issued_at + 1);
        control.replace(replacement).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while withdrawal.calls() < 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("constructing shutdown must cancel and withdraw discovery");
        assert_eq!(publication.calls(), 1);
        drop(unpolled_shutdown);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn external_refresh_request_requires_an_advancing_credential_revision() {
        let identity = auki_p2p::Identity::generate();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time().saturating_sub(2);
        let (initial, initial_compact) = external_fixture(&identity, domain_id, issued_at);
        let initial_expiration = initial.credential_expires_at();
        let (runtime, control) =
            AukiPeer::start_external(identity.clone(), initial, direct_config())
                .await
                .unwrap();
        let relay_authorization = runtime.authority.as_ref().unwrap().relay_authorization();
        let refresh_authorization = relay_authorization.clone();
        let refresh =
            tokio::spawn(async move { refresh_authorization.refresh_after_unauthorized(1).await });

        let request = tokio::time::timeout(Duration::from_secs(1), control.next_refresh_request())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.request_id(), 1);
        assert_eq!(request.rejected_credential_revision(), 1);

        let duplicate =
            external_update_from_compact(&identity, domain_id, initial_compact, initial_expiration);
        assert_eq!(
            control.replace(duplicate).await.unwrap(),
            ExternalAuthorityReplaceOutcome::Unchanged {
                credential_revision: 1
            }
        );
        tokio::task::yield_now().await;
        assert!(!refresh.is_finished());

        let (replacement, _) = external_fixture(&identity, domain_id, issued_at + 1);
        assert_eq!(
            control.replace(replacement).await.unwrap(),
            ExternalAuthorityReplaceOutcome::Replaced {
                credential_revision: 2
            }
        );
        tokio::time::timeout(Duration::from_secs(1), refresh)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        drop(control);
        assert!(
            tokio::time::timeout(
                Duration::from_secs(1),
                relay_authorization.refresh_after_unauthorized(2),
            )
            .await
            .unwrap()
            .is_err()
        );
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_external_runtime_wakes_the_refresh_request_stream() {
        let identity = auki_p2p::Identity::generate();
        let (initial, _) =
            external_fixture(&identity, Uuid::new_v4(), unix_time().saturating_sub(1));
        let (runtime, control) = AukiPeer::start_external(identity, initial, direct_config())
            .await
            .unwrap();

        drop(runtime);

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), control.next_refresh_request())
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_mismatch_is_rejected_before_local_runtime_creation() {
        let authorized_identity = auki_p2p::Identity::generate();
        let prepared = fixture(&authorized_identity, Uuid::new_v4());
        let actual_identity = auki_p2p::Identity::generate();
        let actual_peer = actual_identity.peer_id();
        let error = match AukiPeer::start(actual_identity, prepared, direct_config()).await {
            Ok(_) => panic!("a mismatched identity must not start"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            AukiPeerStartError::IdentityMismatch { identity, .. } if identity == actual_peer
        ));

        let authorized_identity = auki_p2p::Identity::generate();
        let (initial, _) = external_fixture(
            &authorized_identity,
            Uuid::new_v4(),
            unix_time().saturating_sub(1),
        );
        let actual_identity = auki_p2p::Identity::generate();
        let actual_peer = actual_identity.peer_id();
        let error = match AukiPeer::start_external(actual_identity, initial, direct_config()).await
        {
            Ok(_) => panic!("a mismatched external authority must not start"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            AukiPeerStartError::IdentityMismatch { identity, .. } if identity == actual_peer
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_runtime_fences_context_without_a_network_cleanup_api_call() {
        let identity = auki_p2p::Identity::generate();
        let prepared = fixture(&identity, Uuid::new_v4());
        let runtime = AukiPeer::start(identity, prepared, direct_config())
            .await
            .unwrap();
        let context = runtime.protocol_context();
        let status = runtime.subscribe_status();
        let registration = runtime
            .protocols()
            .register(
                crate::AukiProtocolSpec::new("/example/drop-fence/1.0.0", 1, 32).unwrap(),
                |_stream| async {},
            )
            .unwrap();
        assert!(registration.owns_server_for_test());

        drop(runtime);

        assert!(!registration.owns_server_for_test());
        tokio::time::timeout(Duration::from_secs(1), registration.close())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*status.borrow(), AukiPeerStatus::Stopping);
        assert!(matches!(
            context.authorization().current(),
            Err(AukiPeerAuthorizationError::Stopped)
        ));
        assert!(matches!(
            context.routes().snapshot(),
            Err(AukiPeerRoutesError::Stopped)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canceled_relay_start_does_not_delete_the_created_booking() {
        let server = MockServer::start();
        let booking_id = Uuid::new_v4();
        let created_at = Utc::now();
        let snapshot = json!({
            "booking_id": booking_id,
            "mode": "public",
            "state": "active",
            "relay_count": 1,
            "requested_duration_seconds": 86_400,
            "requested_until": created_at + chrono::Duration::hours(24),
            "authority_expires_at": created_at + chrono::Duration::minutes(20),
            "assigned_count": 0,
            "provider_ready_count": 0,
            "unfilled_count": 1,
            "created_at": created_at,
            "slots": [{
                "slot_id": Uuid::new_v4(),
                "slot_index": 0,
                "state": "queued"
            }]
        });
        let active = server.mock(|when, then| {
            when.method(GET).path("/relay-bookings/active");
            then.status(204).header("cache-control", "no-store");
        });
        let create = server.mock(|when, then| {
            when.method(POST).path("/relay-bookings");
            then.status(201)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .header("location", format!("/relay-bookings/{booking_id}"))
                .json_body(snapshot);
        });
        let delete = server.mock(|when, then| {
            when.method(DELETE)
                .path(format!("/relay-bookings/{booking_id}"));
            then.status(204).header("cache-control", "no-store");
        });

        let identity = auki_p2p::Identity::generate();
        let prepared = fixture(&identity, Uuid::new_v4());
        let config = AukiPeerConfig::new(server.base_url())
            .unwrap()
            .with_relay(
                AukiRelayConfig::new(
                    AukiRelayMode::Public,
                    1,
                    Duration::from_secs(86_400),
                    Duration::from_secs(60),
                )
                .unwrap(),
            )
            .unwrap();
        let startup = tokio::spawn(AukiPeer::start(identity, prepared, config));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if create.calls_async().await == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("startup must reach DMS create");
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !startup.is_finished(),
            "startup must still be waiting for a confirmed route"
        );
        startup.abort();
        assert!(matches!(startup.await, Err(error) if error.is_cancelled()));
        tokio::time::sleep(Duration::from_millis(50)).await;
        active.assert_calls_async(1).await;
        create.assert_calls_async(1).await;
        delete.assert_calls_async(0).await;
    }

    #[tokio::test]
    async fn external_relay_startup_401_fails_at_the_bounded_refresh_deadline() {
        let server = MockServer::start();
        let unauthorized = server.mock(|when, then| {
            when.method(GET).path("/relay-bookings/active");
            then.status(401)
                .header("cache-control", "no-store")
                .header("content-type", "application/json")
                .json_body(json!({
                    "code": "unauthorized",
                    "error": "startup authority was rejected"
                }));
        });

        let identity = auki_p2p::Identity::generate();
        let (initial, _) =
            external_fixture(&identity, Uuid::new_v4(), unix_time().saturating_sub(1));
        let config = AukiPeerConfig::new(server.base_url())
            .unwrap()
            .with_relay(
                AukiRelayConfig::new(
                    AukiRelayMode::Public,
                    1,
                    Duration::from_secs(86_400),
                    Duration::from_secs(60),
                )
                .unwrap(),
            )
            .unwrap();
        let startup = tokio::spawn(AukiPeer::start_external(identity, initial, config));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if unauthorized.calls_async().await == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("startup must reach DMS before its external control is returned");
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !startup.is_finished(),
            "the external refresh deadline must be pending after the 401"
        );

        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(11)).await;
        let result = tokio::time::timeout(Duration::from_secs(2), startup)
            .await
            .expect("external relay startup must fail within its refresh deadline")
            .unwrap();
        assert!(matches!(result, Err(AukiPeerStartError::Relay(_))));
        unauthorized.assert_calls_async(1).await;
    }

    #[test]
    fn relay_start_configs_use_fresh_keys_and_clones_retain_one_key() {
        let relay = AukiRelayConfig::default();
        let first = relay_coordinator_config(relay).unwrap();
        let retry = first.clone();
        let next_start = relay_coordinator_config(relay).unwrap();
        assert_eq!(first.idempotency_key, retry.idempotency_key);
        assert_ne!(first.idempotency_key, next_start.idempotency_key);
    }

    #[test]
    fn status_projection_never_calls_a_confirmed_route_ready_after_component_failure() {
        let route_status = RouteCatalogStatus {
            revision: 1,
            direct_route_count: 0,
            confirmed_relay_count: 1,
        };
        let authority = AuthorityStatus::Ready {
            credential_revision: 1,
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };
        assert_eq!(
            observed_status(
                NodeObservationStatus::Stopped,
                &authority,
                &route_status,
                false,
                true
            ),
            AukiPeerStatus::Failed(AukiPeerFailure::Transport)
        );
        assert_eq!(
            observed_status(
                NodeObservationStatus::Running,
                &AuthorityStatus::Stopped,
                &route_status,
                false,
                true
            ),
            AukiPeerStatus::Failed(AukiPeerFailure::Authority)
        );
        assert_eq!(
            observed_status(
                NodeObservationStatus::Running,
                &authority,
                &route_status,
                true,
                true,
            ),
            AukiPeerStatus::Failed(AukiPeerFailure::Relay)
        );
    }

    #[test]
    fn startup_waits_through_recoverable_authority_gaps_even_with_a_confirmed_route() {
        let ready_authority = AuthorityStatus::Ready {
            credential_revision: 1,
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };
        assert_eq!(
            startup_readiness(
                NodeObservationStatus::Running,
                &AuthorityStatus::Expired {
                    credential_revision: 1,
                    expired_at: Utc::now(),
                },
                1,
                false,
                true,
            ),
            Ok(StartupReadiness::Waiting)
        );
        assert_eq!(
            startup_readiness(
                NodeObservationStatus::Running,
                &ready_authority,
                1,
                false,
                true,
            ),
            Ok(StartupReadiness::Ready)
        );
        assert_eq!(
            startup_readiness(
                NodeObservationStatus::Stopped,
                &ready_authority,
                1,
                false,
                true,
            ),
            Err(StartupReadinessError::Transport)
        );
    }

    #[test]
    fn direct_only_startup_waits_for_authority_and_transport_without_requiring_a_route() {
        let ready_authority = AuthorityStatus::Ready {
            credential_revision: 1,
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };
        assert_eq!(
            startup_readiness(
                NodeObservationStatus::Running,
                &AuthorityStatus::Expired {
                    credential_revision: 1,
                    expired_at: Utc::now(),
                },
                0,
                false,
                false,
            ),
            Ok(StartupReadiness::Waiting)
        );
        assert_eq!(
            startup_readiness(
                NodeObservationStatus::Running,
                &ready_authority,
                0,
                false,
                false,
            ),
            Ok(StartupReadiness::Ready)
        );
        assert_eq!(
            startup_readiness(
                NodeObservationStatus::Stopped,
                &ready_authority,
                0,
                false,
                false,
            ),
            Err(StartupReadinessError::Transport)
        );
        assert_eq!(
            startup_readiness(
                NodeObservationStatus::Running,
                &AuthorityStatus::Stopped,
                0,
                false,
                false,
            ),
            Err(StartupReadinessError::Authority)
        );
    }

    #[test]
    fn route_catalog_limits_match_selected_relay_capacity() {
        let catalog = RouteCatalog::new(
            auki_p2p::Identity::generate().peer_id(),
            Vec::new(),
            RouteCatalogLimits::new(16, 3),
        )
        .unwrap();
        assert_eq!(catalog.status().unwrap().confirmed_relay_count, 0);
    }
}
