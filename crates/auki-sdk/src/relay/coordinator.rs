//! Reconciles one requester-scoped DMS booking with peer-owned relay
//! reservations and a fenced local route catalog.
//!
//! This module deliberately owns no authentication renewal, discovery,
//! publication, or product-work gating policy.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use auki_p2p::{
    ExpectedRelayLimits, Multiaddr, Node, PeerId, Protocol, RelayBaseTransport,
    RelayConfirmationRejection, RelayProvider, RelayReservationHandle, RelayReservationSnapshot,
    RelayTransportEvent,
};
use parking_lot::Mutex;
use rand::Rng;
use tokio::sync::broadcast;
use tokio::{
    runtime::Handle as RuntimeHandle,
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
    time::Instant,
};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};
use tracing::warn;
use uuid::Uuid;

use crate::runtime_policy::{
    ActiveBookingValidation, RelayBookingExpectation, cap_relay_renewal_delay,
    relay_authorized_until, validate_active_booking,
};

use super::{
    CreateRelayBookingRequest, RelayBookingApi, RelayBookingClientError, RelayBookingMode,
    RelayBookingSnapshot, RelayBookingState, RelayErrorCode, RelayIdempotencyKey, RelayOperation,
    RelaySlotState, ReservationFailedRequest, ReservationFailureReason,
};

/// The complete local fence for one child reservation attempt.
///
/// DMS owns the first three components. The process-local generation makes a
/// late result from a canceled attempt harmless even when DMS legitimately
/// returns to the same assignment and epoch later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LocalRelayFence {
    pub(crate) slot_id: Uuid,
    pub(crate) assignment_id: Uuid,
    pub(crate) reservation_epoch: Uuid,
    pub(crate) local_generation: u64,
}

#[derive(Debug)]
pub(crate) enum ReservationAttemptFailure {
    Provider {
        handle: Option<RelayReservationHandle>,
        reason: ReservationFailureReason,
        retryable: bool,
    },
    BackendStopped {
        handle: Option<RelayReservationHandle>,
    },
}

#[async_trait]
pub(crate) trait RelayReservationBackend: Send + Sync {
    /// Start must be cancellation-safe: dropping this future before a handle
    /// is delivered must roll back any generation minted by the backend.
    async fn start(
        &self,
        provider: RelayProvider,
    ) -> Result<RelayReservationHandle, ReservationAttemptFailure>;

    async fn wait(
        &self,
        handle: RelayReservationHandle,
    ) -> Result<RelayReservationSnapshot, ReservationAttemptFailure>;

    async fn cancel(&self, handle: RelayReservationHandle) -> Result<(), PeerRelayError>;

    fn subscribe(&self) -> Result<broadcast::Receiver<RelayTransportEvent>, PeerRelayError>;
}

#[derive(Clone)]
pub(crate) struct PeerRelayReservations {
    node: Node,
    lifecycle: CancellationToken,
}

impl PeerRelayReservations {
    pub(crate) fn new(node: Node, lifecycle: CancellationToken) -> Self {
        Self { node, lifecycle }
    }
}

#[async_trait]
impl RelayReservationBackend for PeerRelayReservations {
    async fn start(
        &self,
        provider: RelayProvider,
    ) -> Result<RelayReservationHandle, ReservationAttemptFailure> {
        tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => Err(ReservationAttemptFailure::BackendStopped { handle: None }),
            result = self.node.start_relay_reservation(provider) => {
                result.map_err(|error| reservation_attempt_failure(PeerRelayError::P2p(error), None))
            }
        }
    }

    async fn wait(
        &self,
        handle: RelayReservationHandle,
    ) -> Result<RelayReservationSnapshot, ReservationAttemptFailure> {
        tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => Err(ReservationAttemptFailure::BackendStopped { handle: Some(handle) }),
            result = self.node.wait_relay_reservation(handle) => {
                result.map_err(|error| reservation_attempt_failure(PeerRelayError::P2p(error), Some(handle)))
            }
        }
    }

    async fn cancel(&self, handle: RelayReservationHandle) -> Result<(), PeerRelayError> {
        tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => Err(PeerRelayError::Stopped),
            result = self.node.cancel_relay_reservation(handle) => result.map_err(PeerRelayError::P2p),
        }
    }

    fn subscribe(&self) -> Result<broadcast::Receiver<RelayTransportEvent>, PeerRelayError> {
        if self.lifecycle.is_cancelled() {
            return Err(PeerRelayError::Stopped);
        }
        Ok(self.node.subscribe_relay_events())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PeerRelayError {
    #[error("the peer relay-reservation capability is stopped")]
    Stopped,
    #[error(transparent)]
    P2p(#[from] auki_p2p::Error),
}

fn reservation_attempt_failure(
    error: PeerRelayError,
    handle: Option<RelayReservationHandle>,
) -> ReservationAttemptFailure {
    match error {
        PeerRelayError::Stopped => ReservationAttemptFailure::BackendStopped { handle },
        error @ PeerRelayError::P2p(_) => ReservationAttemptFailure::Provider {
            handle,
            reason: reservation_failure_reason(&error, false),
            retryable: reservation_failure_is_retryable(&error),
        },
    }
}

fn reservation_failure_reason(
    error: &PeerRelayError,
    was_publishable: bool,
) -> ReservationFailureReason {
    use auki_p2p::Error;

    let PeerRelayError::P2p(error) = error else {
        return if was_publishable {
            ReservationFailureReason::ReservationLost
        } else {
            ReservationFailureReason::DialFailed
        };
    };
    match error {
        Error::RelayDirectConnectionMismatch { .. }
        | Error::InvalidRemoteAddress { .. }
        | Error::InvalidRelayRoute { .. }
        | Error::RelayReservation(
            auki_p2p::RelayReservationError::InvalidBase { .. }
            | auki_p2p::RelayReservationError::BasePeerMismatch { .. }
            | auki_p2p::RelayReservationError::DuplicateBase(_)
            | auki_p2p::RelayReservationError::MissingTransportBase(_)
            | auki_p2p::RelayReservationError::EmptyBases,
        ) => ReservationFailureReason::AddressMismatch,
        Error::RelayConfirmationRejected(
            RelayConfirmationRejection::MissingLimits
            | RelayConfirmationRejection::IncompleteLimits { .. }
            | RelayConfirmationRejection::LimitMismatch { .. },
        ) => ReservationFailureReason::LimitMismatch,
        Error::RelayReservationClosed(_) if was_publishable => {
            ReservationFailureReason::ReservationLost
        }
        Error::RelayReservationClosed(_) | Error::RelayConfirmationRejected(_) => {
            ReservationFailureReason::ReservationDenied
        }
        Error::Dns(_) | Error::Dial(_) => ReservationFailureReason::DialFailed,
        _ if was_publishable => ReservationFailureReason::ReservationLost,
        _ => ReservationFailureReason::DialFailed,
    }
}

fn reservation_failure_is_retryable(error: &PeerRelayError) -> bool {
    let PeerRelayError::P2p(error) = error else {
        return false;
    };
    matches!(
        error,
        auki_p2p::Error::Dns(_)
            | auki_p2p::Error::Dial(_)
            | auki_p2p::Error::RelayReservationClosed(_)
            | auki_p2p::Error::SwarmStopped
            | auki_p2p::Error::Io(_)
    )
}

pub(crate) fn relay_provider(
    peer_id: &str,
    bases: &[String],
    duration_seconds: u32,
    data_bytes_per_direction: u64,
) -> Result<RelayProvider, auki_p2p::RelayReservationError> {
    let peer_id =
        peer_id
            .parse()
            .map_err(|error| auki_p2p::RelayReservationError::InvalidBase {
                address: peer_id.to_string(),
                reason: format!("invalid provider Peer ID: {error}"),
            })?;
    let limits = ExpectedRelayLimits::new(
        Duration::from_secs(u64::from(duration_seconds)),
        data_bytes_per_direction,
    )?;
    RelayProvider::new(peer_id, bases, limits)
}

pub(crate) fn confirmed_route(snapshot: &RelayReservationSnapshot) -> Option<Multiaddr> {
    snapshot.publishable_route().cloned()
}

pub(crate) type SharedReservationBackend = Arc<dyn RelayReservationBackend>;

#[derive(Clone, Debug)]
pub(crate) struct PublishedRelayRoute {
    pub(crate) fence: LocalRelayFence,
    pub(crate) route: Multiaddr,
    pub(crate) wss_route: Option<Multiaddr>,
    pub(crate) limits: ExpectedRelayLimits,
    pub(crate) authorized_until: chrono::DateTime<chrono::Utc>,
    pub(crate) relay_peer_id: PeerId,
}

#[async_trait]
pub(crate) trait RelayRouteRegistry: Send + Sync {
    async fn publish(&self, route: PublishedRelayRoute) -> Result<(), String>;
    async fn refresh_authority(
        &self,
        fence: LocalRelayFence,
        authorized_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, String>;
    async fn tombstone(&self, fence: LocalRelayFence) -> Result<bool, String>;
    fn fence_all(&self) -> Result<(), String>;
}

pub(crate) type SharedRouteRegistry = Arc<dyn RelayRouteRegistry>;

static NEXT_LOCAL_RELAY_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_local_relay_generation() -> Result<u64, RelayCoordinatorError> {
    NEXT_LOCAL_RELAY_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |generation| {
            generation.checked_add(1)
        })
        .map_err(|_| RelayCoordinatorError::LocalGenerationExhausted)
}

fn route_fence(fence: LocalRelayFence) -> auki_p2p::RouteFence {
    auki_p2p::RouteFence {
        route_id: fence.slot_id,
        authority_id: fence.assignment_id,
        authority_epoch: fence.reservation_epoch,
        local_generation: fence.local_generation,
    }
}

#[derive(Default)]
struct FencedRouteState {
    closed: bool,
    owned: HashSet<LocalRelayFence>,
}

/// A permanent, coordinator-local publication fence around the shared route
/// catalog.
///
/// The outer mutex is intentionally held across each synchronous catalog
/// mutation. That makes publication and emergency fencing linearizable: a
/// publication either commits before `fence_all` and is removed by it, or sees
/// the closed bit and cannot commit. Exact owned fences ensure an old
/// coordinator cannot remove direct routes or relay routes published by its
/// replacement.
struct FencedRouteCatalog {
    catalog: auki_p2p::RouteCatalog,
    state: Mutex<FencedRouteState>,
}

impl FencedRouteCatalog {
    fn new(catalog: auki_p2p::RouteCatalog) -> Self {
        Self {
            catalog,
            state: Mutex::new(FencedRouteState::default()),
        }
    }
}

#[async_trait]
impl RelayRouteRegistry for FencedRouteCatalog {
    async fn publish(&self, route: PublishedRelayRoute) -> Result<(), String> {
        let mut state = self.state.lock();
        if state.closed {
            return Err("relay route registry is permanently fenced".to_string());
        }
        let fence = route.fence;
        self.catalog
            .publish_confirmed(auki_p2p::ConfirmedRoute {
                fence: route_fence(fence),
                relay_peer_id: route.relay_peer_id,
                route: route.route,
                wss_route: route.wss_route,
                limits: route.limits,
                authorized_until: route.authorized_until,
            })
            .map_err(|error| error.to_string())?;
        state.owned.insert(fence);
        Ok(())
    }

    async fn refresh_authority(
        &self,
        fence: LocalRelayFence,
        authorized_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, String> {
        let mut state = self.state.lock();
        if state.closed || !state.owned.contains(&fence) {
            return Ok(false);
        }
        match self
            .catalog
            .refresh_authorization(route_fence(fence), authorized_until)
        {
            Ok(_) => Ok(true),
            Err(
                auki_p2p::RouteCatalogError::RouteNotFound
                | auki_p2p::RouteCatalogError::StaleRouteFence,
            ) => {
                state.owned.remove(&fence);
                Ok(false)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    async fn tombstone(&self, fence: LocalRelayFence) -> Result<bool, String> {
        let mut state = self.state.lock();
        if !state.owned.contains(&fence) {
            return Ok(false);
        }
        match self.catalog.tombstone(route_fence(fence)) {
            Ok(_) => {
                state.owned.remove(&fence);
                Ok(true)
            }
            Err(
                auki_p2p::RouteCatalogError::RouteNotFound
                | auki_p2p::RouteCatalogError::StaleRouteFence,
            ) => {
                state.owned.remove(&fence);
                Ok(false)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn fence_all(&self) -> Result<(), String> {
        let mut state = self.state.lock();
        state.closed = true;
        let fences: Vec<_> = state.owned.iter().copied().collect();
        let mut first_error = None;
        for fence in fences {
            match self.catalog.tombstone(route_fence(fence)) {
                Ok(_)
                | Err(
                    auki_p2p::RouteCatalogError::RouteNotFound
                    | auki_p2p::RouteCatalogError::StaleRouteFence,
                ) => {
                    state.owned.remove(&fence);
                }
                Err(error) => {
                    first_error.get_or_insert_with(|| error.to_string());
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl RelayRouteRegistry for auki_p2p::RouteCatalog {
    async fn publish(&self, route: PublishedRelayRoute) -> Result<(), String> {
        self.publish_confirmed(auki_p2p::ConfirmedRoute {
            fence: route_fence(route.fence),
            relay_peer_id: route.relay_peer_id,
            route: route.route,
            wss_route: route.wss_route,
            limits: route.limits,
            authorized_until: route.authorized_until,
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    async fn refresh_authority(
        &self,
        fence: LocalRelayFence,
        authorized_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, String> {
        match self.refresh_authorization(route_fence(fence), authorized_until) {
            Ok(_) => Ok(true),
            Err(
                auki_p2p::RouteCatalogError::RouteNotFound
                | auki_p2p::RouteCatalogError::StaleRouteFence,
            ) => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn tombstone(&self, fence: LocalRelayFence) -> Result<bool, String> {
        match self.tombstone(route_fence(fence)) {
            Ok(_) => Ok(true),
            Err(
                auki_p2p::RouteCatalogError::RouteNotFound
                | auki_p2p::RouteCatalogError::StaleRouteFence,
            ) => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }

    fn fence_all(&self) -> Result<(), String> {
        self.tombstone_all()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RelayCoordinatorConfig {
    pub(crate) idempotency_key: RelayIdempotencyKey,
    pub(crate) mode: RelayBookingMode,
    pub(crate) requested_duration_seconds: u64,
    pub(crate) relay_count: u8,
    pub(crate) status_poll_interval: Duration,
    pub(crate) reservation_retry_budget: Duration,
    pub(crate) retry_min: Duration,
    pub(crate) retry_max: Duration,
    pub(crate) http_timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RelayCoordinatorError {
    #[error("relay booking request failed")]
    Dms(#[from] RelayBookingClientError),
    #[error("the active relay booking does not match the requested configuration")]
    ActiveBookingMismatch,
    #[error("relay booking authority ended")]
    AuthorityEnded,
    #[error("relay coordinator task stopped")]
    Stopped,
    #[error("relay route registry rejected a fenced update: {0}")]
    RouteRegistry(String),
    #[error("relay reservation cleanup did not complete: {0}")]
    ReservationCleanup(String),
    #[error("the process-local relay generation space is exhausted")]
    LocalGenerationExhausted,
    #[error("relay reservation event stream lagged by {0} events")]
    RelayEventLagged(u64),
    #[error("the peer relay-reservation capability failed")]
    RelayTransport(#[from] PeerRelayError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RelayCoordinatorShutdownOutcome {
    Graceful,
    ForcedAfterTimeout,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RelayCoordinatorShutdownError {
    #[error("relay coordinator graceful shutdown failed")]
    Graceful(#[source] RelayCoordinatorError),
    #[error("relay coordinator forced cleanup failed")]
    ForcedCleanup(#[source] RelayCoordinatorError),
}

impl RelayCoordinatorError {
    pub(crate) fn startup_retry_after(&self, fallback: Duration) -> Option<Duration> {
        let Self::Dms(error) = self else {
            return None;
        };
        if error.is_retryable()
            || error.http_code().is_some_and(|code| {
                code.is_stale_requester_principal()
                    || matches!(
                        code,
                        RelayErrorCode::ActiveBookingConflict | RelayErrorCode::TargetPeerConflict
                    )
            })
        {
            return Some(error.retry_after().unwrap_or(fallback));
        }
        None
    }
}

pub(crate) struct RelayBookingCoordinator {
    commands: mpsc::Sender<CoordinatorCommand>,
    health: watch::Receiver<bool>,
    task: Option<AbortOnDropHandle<Result<(), RelayCoordinatorError>>>,
    force_shutdown: CancellationToken,
    routes: SharedRouteRegistry,
    runtime: RuntimeHandle,
}

impl RelayBookingCoordinator {
    pub(crate) async fn start(
        api: Arc<dyn RelayBookingApi>,
        reservations: PeerRelayReservations,
        routes: auki_p2p::RouteCatalog,
        config: RelayCoordinatorConfig,
    ) -> Result<Self, RelayCoordinatorError> {
        Self::start_with_backends(
            api,
            Arc::new(reservations),
            Arc::new(FencedRouteCatalog::new(routes)),
            config,
        )
        .await
    }

    async fn start_with_backends(
        api: Arc<dyn RelayBookingApi>,
        backend: SharedReservationBackend,
        routes: SharedRouteRegistry,
        config: RelayCoordinatorConfig,
    ) -> Result<Self, RelayCoordinatorError> {
        // Preflight the peer-owned capability before creating control-plane
        // authority. A stopped peer must not leave a fresh DMS booking behind.
        let relay_events = backend.subscribe()?;
        let active =
            match bounded_control_call(config.http_timeout, RelayOperation::Active, api.active())
                .await
            {
                Ok(active) => active,
                Err(error)
                    if error
                        .http_code()
                        .is_some_and(RelayErrorCode::is_stale_requester_principal) =>
                {
                    // Create is the expiry-on-access path for a new requester
                    // Peer ID and returns the authoritative conflict Retry-After
                    // while the previous requester authority is still live.
                    None
                }
                Err(error) => return Err(error.into()),
            };
        let snapshot = match active {
            Some(snapshot) => {
                validate_booking_matches(&snapshot, &config)?;
                snapshot
            }
            None => {
                let request = CreateRelayBookingRequest::new(
                    config.mode,
                    config.requested_duration_seconds,
                    config.relay_count,
                )?;
                let response = bounded_control_call(
                    config.http_timeout,
                    RelayOperation::Create,
                    api.create(&config.idempotency_key, &request),
                )
                .await?;
                if response.snapshot.state == RelayBookingState::Active {
                    response.snapshot
                } else {
                    let replacement_key =
                        RelayIdempotencyKey::new(format!("auki-sdk-relay-{}", Uuid::new_v4()))?;
                    bounded_control_call(
                        config.http_timeout,
                        RelayOperation::Create,
                        api.create(&replacement_key, &request),
                    )
                    .await?
                    .snapshot
                }
            }
        };

        validate_booking_matches(&snapshot, &config)?;
        let (commands, command_rx) = mpsc::channel(32);
        let (events, event_rx) = mpsc::channel(64);
        let (health_tx, health) = watch::channel(true);
        let mut actor = CoordinatorActor {
            api,
            backend,
            routes: Arc::clone(&routes),
            config: config.clone(),
            snapshot,
            slots: HashMap::new(),
            retiring: HashMap::new(),
            pending_relay_failures: HashMap::new(),
            detached_cleanups: JoinSet::new(),
            command_rx,
            events: events.clone(),
            event_rx,
            relay_events,
            next_poll: Instant::now(),
            next_renew: Instant::now(),
            next_expiry: Instant::now() + Duration::from_secs(86_400),
            control_fenced: false,
        };
        actor.schedule_renewal();
        actor.apply_current_snapshot().await?;
        actor.schedule_status_poll(false);
        let force_shutdown = CancellationToken::new();
        let actor_force_shutdown = force_shutdown.clone();
        let runtime = RuntimeHandle::current();
        let task = AbortOnDropHandle::new(tokio::spawn(async move {
            run_coordinator_actor(actor, health_tx, actor_force_shutdown).await
        }));
        Ok(Self {
            commands,
            health,
            task: Some(task),
            force_shutdown,
            routes,
            runtime,
        })
    }

    pub(crate) fn health(&self) -> RelayCoordinatorHealth {
        RelayCoordinatorHealth {
            health: self.health.clone(),
        }
    }

    pub(crate) async fn shutdown(
        mut self,
        delete_booking: bool,
        graceful_timeout: Duration,
    ) -> Result<RelayCoordinatorShutdownOutcome, RelayCoordinatorShutdownError> {
        match tokio::time::timeout(graceful_timeout, self.request_stop_and_join(delete_booking))
            .await
        {
            Ok(result) => {
                result.map_err(RelayCoordinatorShutdownError::Graceful)?;
                Ok(RelayCoordinatorShutdownOutcome::Graceful)
            }
            Err(_) => {
                let _ = self.routes.fence_all();
                self.force_shutdown.cancel();
                self.join_actor()
                    .await
                    .map_err(RelayCoordinatorShutdownError::ForcedCleanup)?;
                Ok(RelayCoordinatorShutdownOutcome::ForcedAfterTimeout)
            }
        }
    }

    async fn request_stop_and_join(
        &mut self,
        delete_booking: bool,
    ) -> Result<(), RelayCoordinatorError> {
        let (response, receiver) = oneshot::channel();
        let command_result = match self
            .commands
            .send(CoordinatorCommand::Stop {
                delete_booking,
                response,
            })
            .await
        {
            Ok(()) => receiver
                .await
                .unwrap_or(Err(RelayCoordinatorError::Stopped)),
            Err(_) => Err(RelayCoordinatorError::Stopped),
        };
        let task_result = self.join_actor().await;
        task_result.and(command_result)
    }

    async fn join_actor(&mut self) -> Result<(), RelayCoordinatorError> {
        let result = match self.task.as_mut() {
            Some(task) => task.await.map_err(|_| RelayCoordinatorError::Stopped)?,
            None => return Ok(()),
        };
        self.task.take();
        result
    }
}

impl Drop for RelayBookingCoordinator {
    fn drop(&mut self) {
        let _ = self.routes.fence_all();
        self.force_shutdown.cancel();
        if let Some(task) = self.task.take() {
            // Async Drop cannot join the actor here. Keep one explicit owner
            // that observes the bounded force-fence result instead of
            // aborting a worker that may be the sole owner of a reservation
            // handle or silently dropping the JoinHandle.
            self.runtime.spawn(async move {
                match task.await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        warn!(error = %error, "relay coordinator drop cleanup failed");
                    }
                    Err(error) => {
                        warn!(error = %error, "relay coordinator drop cleanup task failed");
                    }
                }
            });
        }
    }
}

async fn run_coordinator_actor(
    mut actor: CoordinatorActor,
    health: watch::Sender<bool>,
    force_shutdown: CancellationToken,
) -> Result<(), RelayCoordinatorError> {
    let _health_guard = CoordinatorHealthGuard(health);
    let run_result = {
        let running = actor.run();
        tokio::pin!(running);
        tokio::select! {
            biased;
            _ = force_shutdown.cancelled() => None,
            result = &mut running => Some(result),
        }
    };
    actor.force_fence().await?;
    match run_result {
        None | Some(Ok(())) => Ok(()),
        Some(Err(error)) => Err(error),
    }
}

struct CoordinatorHealthGuard(watch::Sender<bool>);

impl Drop for CoordinatorHealthGuard {
    fn drop(&mut self) {
        self.0.send_replace(false);
    }
}

#[derive(Clone)]
pub(crate) struct RelayCoordinatorHealth {
    health: watch::Receiver<bool>,
}

impl RelayCoordinatorHealth {
    pub(crate) fn is_failed(&self) -> bool {
        !*self.health.borrow()
    }

    pub(crate) async fn failed(&mut self) {
        if !*self.health.borrow() {
            return;
        }
        loop {
            if self.health.changed().await.is_err() || !*self.health.borrow() {
                return;
            }
        }
    }
}

enum CoordinatorCommand {
    Stop {
        delete_booking: bool,
        response: oneshot::Sender<Result<(), RelayCoordinatorError>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalSlotState {
    Reserving(Option<RelayReservationHandle>),
    Confirmed(RelayReservationHandle),
    ReportingFailure(ReservationFailureReason),
}

struct LocalSlot {
    fence: LocalRelayFence,
    relay_peer_id: String,
    provider_base_addresses: Vec<String>,
    limits: ExpectedRelayLimits,
    authorized_until: chrono::DateTime<chrono::Utc>,
    state: LocalSlotState,
    cancel_retry: CancellationToken,
    worker: Option<AbortOnDropHandle<Result<Option<RelayReservationHandle>, String>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetirementAction {
    Reconcile,
    ReportFailure(ReservationFailureReason),
    Remove,
}

struct RetiringSlot {
    local: LocalSlot,
    action: RetirementAction,
    cleanup: AbortOnDropHandle<Result<(), String>>,
}

enum ChildEvent {
    Started {
        fence: LocalRelayFence,
        handle: RelayReservationHandle,
    },
    Confirmed {
        fence: LocalRelayFence,
        snapshot: RelayReservationSnapshot,
    },
    Retrying {
        fence: LocalRelayFence,
        handle: RelayReservationHandle,
    },
    Failed {
        fence: LocalRelayFence,
        handle: Option<RelayReservationHandle>,
        reason: ReservationFailureReason,
    },
    RetryFailure {
        fence: LocalRelayFence,
        reason: ReservationFailureReason,
    },
    CleanupComplete {
        fence: LocalRelayFence,
    },
    BackendStopped,
}

struct CoordinatorActor {
    api: Arc<dyn RelayBookingApi>,
    backend: SharedReservationBackend,
    routes: SharedRouteRegistry,
    config: RelayCoordinatorConfig,
    snapshot: RelayBookingSnapshot,
    slots: HashMap<Uuid, LocalSlot>,
    retiring: HashMap<Uuid, RetiringSlot>,
    pending_relay_failures: HashMap<RelayReservationHandle, ReservationFailureReason>,
    detached_cleanups: JoinSet<Result<(), String>>,
    command_rx: mpsc::Receiver<CoordinatorCommand>,
    events: mpsc::Sender<ChildEvent>,
    event_rx: mpsc::Receiver<ChildEvent>,
    relay_events: broadcast::Receiver<RelayTransportEvent>,
    next_poll: Instant,
    next_renew: Instant,
    next_expiry: Instant,
    control_fenced: bool,
}

impl CoordinatorActor {
    async fn run(&mut self) -> Result<(), RelayCoordinatorError> {
        loop {
            if self.control_fenced {
                return Err(RelayCoordinatorError::AuthorityEnded);
            }
            tokio::select! {
                biased;
                command = self.command_rx.recv() => {
                    let Some(command) = command else {
                        self.fence_control_plane().await?;
                        break;
                    };
                    match command {
                        CoordinatorCommand::Stop { delete_booking, response } => {
                            let result = self.stop(delete_booking).await;
                            let _ = response.send(result);
                            break;
                        }
                    }
                }
                event = self.relay_events.recv() => {
                    self.handle_relay_receive(event).await?;
                }
                _ = tokio::time::sleep_until(self.next_renew), if !self.control_fenced => {
                    self.renew().await?;
                }
                _ = tokio::time::sleep_until(self.next_expiry), if !self.control_fenced => {
                    self.expire_local_authority().await?;
                }
                event = self.event_rx.recv() => {
                    if let Some(event) = event {
                        self.handle_child_event(event).await?;
                    }
                }
                cleanup = self.detached_cleanups.join_next(), if !self.detached_cleanups.is_empty() => {
                    if let Some(result) = cleanup {
                        detached_cleanup_result(result)
                            .map_err(RelayCoordinatorError::ReservationCleanup)?;
                    }
                }
                _ = tokio::time::sleep_until(self.next_poll), if !self.control_fenced => {
                    let protected = self.next_renew.min(self.next_expiry);
                    if protected.saturating_duration_since(Instant::now())
                        <= self.config.http_timeout
                    {
                        self.next_poll = protected + Duration::from_millis(1);
                    } else {
                        self.poll().await?;
                    }
                }
            }
        }
        Ok(())
    }

    fn schedule_renewal(&mut self) {
        let now = chrono::Utc::now();
        let remaining = self
            .snapshot
            .authority_expires_at
            .signed_duration_since(now)
            .to_std()
            .unwrap_or_default();
        let preferred = remaining.mul_f64(rand::random::<f64>() * 0.10 + 0.25);
        let renew_after = cap_relay_renewal_delay(remaining, preferred, self.config.http_timeout)
            .max(Duration::from_millis(1));
        self.next_renew = Instant::now() + renew_after;
    }

    async fn poll(&mut self) -> Result<(), RelayCoordinatorError> {
        let succeeded = match bounded_control_call(
            self.config.http_timeout,
            RelayOperation::Active,
            self.api.active(),
        )
        .await
        {
            Ok(Some(snapshot)) => {
                self.apply_snapshot(snapshot).await?;
                true
            }
            Ok(None) => {
                warn!("active relay booking disappeared");
                self.fence_control_plane().await?;
                return Err(RelayCoordinatorError::AuthorityEnded);
            }
            Err(error) => {
                warn!(error = %error, "relay booking status poll failed");
                if control_error_ends_authority(&error) {
                    self.fence_control_plane().await?;
                    return Err(RelayCoordinatorError::AuthorityEnded);
                }
                false
            }
        };
        let delay = if succeeded {
            self.status_poll_delay()
        } else {
            retry_jitter(self.config.retry_min, self.config.retry_max)
        };
        self.next_poll = Instant::now() + delay;
        Ok(())
    }

    async fn renew(&mut self) -> Result<(), RelayCoordinatorError> {
        match bounded_control_call(
            self.config.http_timeout,
            RelayOperation::Renew,
            self.api.renew(self.snapshot.booking_id),
        )
        .await
        {
            Ok(snapshot) => {
                self.apply_snapshot(snapshot).await?;
                self.schedule_renewal();
            }
            Err(error) => {
                warn!(error = %error, "relay booking authority renewal failed");
                if control_error_ends_authority(&error) {
                    self.fence_control_plane().await?;
                    return Err(RelayCoordinatorError::AuthorityEnded);
                }
                let now = chrono::Utc::now();
                let remaining = self
                    .snapshot
                    .authority_expires_at
                    .signed_duration_since(now)
                    .to_std()
                    .unwrap_or_default();
                let retry_window =
                    cap_relay_renewal_delay(remaining, remaining, self.config.http_timeout);
                if retry_window.is_zero() {
                    self.fence_control_plane().await?;
                    return Err(RelayCoordinatorError::AuthorityEnded);
                }
                let retry_max = self.config.retry_max.min(retry_window);
                let retry_min = self.config.retry_min.min(retry_max);
                self.next_renew = Instant::now() + retry_jitter(retry_min, retry_max);
            }
        }
        Ok(())
    }

    async fn apply_snapshot(
        &mut self,
        snapshot: RelayBookingSnapshot,
    ) -> Result<(), RelayCoordinatorError> {
        if snapshot.booking_id != self.snapshot.booking_id {
            return Err(RelayCoordinatorError::ActiveBookingMismatch);
        }
        if snapshot.state != RelayBookingState::Active {
            self.snapshot = snapshot;
            self.fence_control_plane().await?;
            return Err(RelayCoordinatorError::AuthorityEnded);
        }
        validate_booking_matches(&snapshot, &self.config)?;
        self.snapshot = snapshot;
        self.apply_current_snapshot().await?;
        self.next_poll = Instant::now();
        Ok(())
    }

    async fn apply_current_snapshot(&mut self) -> Result<(), RelayCoordinatorError> {
        if self.snapshot.state != RelayBookingState::Active {
            self.remove_all_slots().await?;
            return Err(RelayCoordinatorError::AuthorityEnded);
        }

        let desired: HashMap<Uuid, _> = self
            .snapshot
            .slots
            .iter()
            .filter(|slot| slot.state == RelaySlotState::Ready)
            .filter_map(|slot| {
                Some((
                    slot.slot_id,
                    (
                        slot.assignment_id?,
                        slot.reservation_epoch?,
                        slot.provider_peer_id.clone()?,
                        slot.provider_base_addresses.clone()?,
                        slot.limits?,
                        slot.provider_lease_expires_at?,
                    ),
                ))
            })
            .collect();

        let stale: Vec<_> = self
            .slots
            .values()
            .filter(|local| {
                desired.get(&local.fence.slot_id).is_none_or(
                    |(assignment, epoch, peer, bases, limits, _)| {
                        *assignment != local.fence.assignment_id
                            || *epoch != local.fence.reservation_epoch
                            || peer != &local.relay_peer_id
                            || bases != &local.provider_base_addresses
                            || local.limits.duration().as_secs()
                                != u64::from(limits.duration_seconds)
                            || local.limits.data_bytes_per_direction()
                                != limits.data_bytes_per_direction
                    },
                )
            })
            .map(|local| local.fence)
            .collect();
        for fence in stale {
            self.begin_retirement(fence, RetirementAction::Reconcile)
                .await?;
        }

        for (slot_id, (assignment_id, reservation_epoch, peer, bases, limits, lease_deadline)) in
            desired
        {
            let authorized_until = relay_authorized_until(
                self.snapshot.requested_until,
                self.snapshot.authority_expires_at,
                lease_deadline,
            );
            if self.retiring.contains_key(&slot_id) {
                continue;
            }
            if authorized_until <= chrono::Utc::now() {
                if let Some(existing) = self.slots.get(&slot_id) {
                    self.begin_retirement(existing.fence, RetirementAction::Remove)
                        .await?;
                }
                continue;
            }
            if let Some(existing) = self.slots.get(&slot_id) {
                let fence = existing.fence;
                match existing.state {
                    LocalSlotState::Confirmed(_) => {
                        if self
                            .routes
                            .refresh_authority(fence, authorized_until)
                            .await
                            .map_err(RelayCoordinatorError::RouteRegistry)?
                        {
                            self.slots
                                .get_mut(&slot_id)
                                .expect("the coordinator serializes slot updates")
                                .authorized_until = authorized_until;
                            continue;
                        }
                        self.begin_retirement(fence, RetirementAction::Reconcile)
                            .await?;
                        continue;
                    }
                    LocalSlotState::Reserving(_) => {
                        self.slots
                            .get_mut(&slot_id)
                            .expect("the coordinator serializes slot updates")
                            .authorized_until = authorized_until;
                        continue;
                    }
                    LocalSlotState::ReportingFailure(_) => {
                        self.slots
                            .get_mut(&slot_id)
                            .expect("the coordinator serializes slot updates")
                            .authorized_until = authorized_until;
                        continue;
                    }
                }
            }

            let provider = match relay_provider(
                &peer,
                &bases,
                limits.duration_seconds,
                limits.data_bytes_per_direction,
            ) {
                Ok(provider) => provider,
                Err(error) => {
                    warn!(slot_id = %slot_id, error = %error, "DMS relay provider metadata is invalid");
                    let fence = self.next_fence(slot_id, assignment_id, reservation_epoch)?;
                    self.slots.insert(
                        slot_id,
                        LocalSlot {
                            fence,
                            relay_peer_id: peer,
                            provider_base_addresses: bases,
                            limits: ExpectedRelayLimits::new(
                                Duration::from_secs(u64::from(limits.duration_seconds)),
                                limits.data_bytes_per_direction,
                            )
                            .map_err(|_| RelayCoordinatorError::ActiveBookingMismatch)?,
                            authorized_until,
                            state: LocalSlotState::ReportingFailure(
                                ReservationFailureReason::AddressMismatch,
                            ),
                            cancel_retry: CancellationToken::new(),
                            worker: None,
                        },
                    );
                    let _ = self
                        .events
                        .send(ChildEvent::RetryFailure {
                            fence,
                            reason: ReservationFailureReason::AddressMismatch,
                        })
                        .await;
                    continue;
                }
            };
            let fence = self.next_fence(slot_id, assignment_id, reservation_epoch)?;
            let cancel_retry = CancellationToken::new();
            self.slots.insert(
                slot_id,
                LocalSlot {
                    fence,
                    relay_peer_id: provider.relay_peer_id().to_string(),
                    provider_base_addresses: bases,
                    limits: provider.expected_limits(),
                    authorized_until,
                    state: LocalSlotState::Reserving(None),
                    cancel_retry: cancel_retry.clone(),
                    worker: None,
                },
            );
            let worker = spawn_reservation_worker(
                Arc::clone(&self.backend),
                self.events.clone(),
                fence,
                provider,
                cancel_retry,
                self.config.clone(),
            );
            self.slots
                .get_mut(&slot_id)
                .expect("the coordinator serializes slot creation")
                .worker = Some(worker);
        }
        self.reset_expiry_timer();
        Ok(())
    }

    fn next_fence(
        &mut self,
        slot_id: Uuid,
        assignment_id: Uuid,
        reservation_epoch: Uuid,
    ) -> Result<LocalRelayFence, RelayCoordinatorError> {
        Ok(LocalRelayFence {
            slot_id,
            assignment_id,
            reservation_epoch,
            local_generation: next_local_relay_generation()?,
        })
    }

    async fn handle_child_event(&mut self, event: ChildEvent) -> Result<(), RelayCoordinatorError> {
        match event {
            ChildEvent::Started { fence, handle } => {
                let accepted = self.slots.get_mut(&fence.slot_id).is_some_and(|local| {
                    if local.fence == fence
                        && matches!(local.state, LocalSlotState::Reserving(None))
                    {
                        local.state = LocalSlotState::Reserving(Some(handle));
                        true
                    } else {
                        false
                    }
                });
                if accepted {
                    self.pending_relay_failures.retain(|candidate, _| {
                        candidate.relay_peer_id() != handle.relay_peer_id()
                            || candidate.generation() >= handle.generation()
                    });
                } else {
                    self.pending_relay_failures.remove(&handle);
                    self.spawn_detached_cleanup(handle);
                }
            }
            ChildEvent::Confirmed { fence, snapshot } => {
                let expected_handle = snapshot.handle();
                let current = self.slots.get(&fence.slot_id).is_some_and(|local| {
                    local.fence == fence
                        && matches!(local.state, LocalSlotState::Reserving(Some(handle)) if handle == expected_handle)
                        && self.snapshot_has_fence(fence)
                });
                if current {
                    if let Some(reason) = self.pending_relay_failures.remove(&expected_handle) {
                        let _ = self.join_slot_worker(fence).await?;
                        self.handle_local_failure(fence, reason).await?;
                        return Ok(());
                    }
                } else {
                    self.pending_relay_failures.remove(&snapshot.handle());
                    self.spawn_detached_cleanup(snapshot.handle());
                    return Ok(());
                }
                let _ = self.join_slot_worker(fence).await?;
                let Some(route) = confirmed_route(&snapshot) else {
                    self.handle_local_failure(fence, ReservationFailureReason::ReservationDenied)
                        .await?;
                    return Ok(());
                };
                self.publish_confirmed_route(fence, route, snapshot.handle().relay_peer_id())
                    .await?;
                self.slots
                    .get_mut(&fence.slot_id)
                    .expect("current slot")
                    .state = LocalSlotState::Confirmed(snapshot.handle());
            }
            ChildEvent::Retrying { fence, handle } => {
                if let Some(local) = self
                    .slots
                    .get_mut(&fence.slot_id)
                    .filter(|local| local.fence == fence)
                    && matches!(local.state, LocalSlotState::Reserving(Some(current)) if current == handle)
                {
                    local.state = LocalSlotState::Reserving(None);
                }
                self.pending_relay_failures.remove(&handle);
            }
            ChildEvent::Failed {
                fence,
                handle,
                reason,
            } => {
                if self.is_reserving(fence) {
                    let worker_handle = self.join_slot_worker(fence).await?;
                    let owned_handle = handle.or(worker_handle);
                    let reason = owned_handle
                        .and_then(|handle| self.pending_relay_failures.remove(&handle))
                        .map_or(reason, |pending| preferred_failure_reason(reason, pending));
                    if let Some(handle) = owned_handle
                        && let Some(local) = self
                            .slots
                            .get_mut(&fence.slot_id)
                            .filter(|local| local.fence == fence)
                        && matches!(local.state, LocalSlotState::Reserving(None))
                    {
                        local.state = LocalSlotState::Reserving(Some(handle));
                    }
                    self.handle_local_failure(fence, reason).await?;
                } else if let Some(handle) = handle {
                    self.spawn_detached_cleanup(handle);
                }
            }
            ChildEvent::RetryFailure { fence, reason } => {
                if self.is_current(fence) {
                    self.report_failure(fence, reason).await?;
                }
            }
            ChildEvent::CleanupComplete { fence } => {
                self.finish_retirement(fence).await?;
            }
            ChildEvent::BackendStopped => {
                return Err(RelayCoordinatorError::RelayTransport(
                    PeerRelayError::Stopped,
                ));
            }
        }
        self.next_poll = Instant::now();
        Ok(())
    }

    fn spawn_detached_cleanup(&mut self, handle: RelayReservationHandle) {
        let backend = Arc::clone(&self.backend);
        let timeout = self.config.reservation_retry_budget;
        self.detached_cleanups
            .spawn(async move { cancel_reservation_with_timeout(&backend, handle, timeout).await });
    }

    async fn publish_confirmed_route(
        &mut self,
        fence: LocalRelayFence,
        route: Multiaddr,
        relay_peer_id: PeerId,
    ) -> Result<(), RelayCoordinatorError> {
        let local = self
            .slots
            .get(&fence.slot_id)
            .filter(|local| local.fence == fence)
            .ok_or_else(|| {
                RelayCoordinatorError::RouteRegistry(
                    "confirmed route no longer has a current local fence".to_string(),
                )
            })?;
        let target_peer_id = match route.iter().last() {
            Some(Protocol::P2p(peer_id)) => peer_id,
            _ => {
                return Err(RelayCoordinatorError::RouteRegistry(
                    "confirmed relay route is missing its target Peer ID".to_string(),
                ));
            }
        };
        let provider = relay_provider(
            &relay_peer_id.to_string(),
            &local.provider_base_addresses,
            local.limits.duration_seconds(),
            local.limits.data_bytes_per_direction(),
        )
        .map_err(|error| RelayCoordinatorError::RouteRegistry(error.to_string()))?;
        let expected_route = provider
            .circuit_route_for_transport(RelayBaseTransport::Tcp, target_peer_id)
            .map_err(|error| RelayCoordinatorError::RouteRegistry(error.to_string()))?;
        if expected_route != route {
            return Err(RelayCoordinatorError::RouteRegistry(
                "confirmed TCP route differs from the current DMS provider metadata".to_string(),
            ));
        }
        let wss_route = provider
            .base_for_transport(RelayBaseTransport::Wss)
            .map(|_| provider.circuit_route_for_transport(RelayBaseTransport::Wss, target_peer_id))
            .transpose()
            .map_err(|error| RelayCoordinatorError::RouteRegistry(error.to_string()))?;
        let published = PublishedRelayRoute {
            fence,
            route,
            wss_route,
            limits: local.limits,
            authorized_until: local.authorized_until,
            relay_peer_id,
        };
        if let Err(error) = self.routes.publish(published).await {
            warn!(slot_id = %fence.slot_id, %error, "confirmed relay route could not be published");
            self.begin_retirement(fence, RetirementAction::Remove)
                .await?;
            self.finish_retirement(fence).await?;
            return Err(RelayCoordinatorError::RouteRegistry(error));
        }
        Ok(())
    }

    async fn handle_relay_event(
        &mut self,
        event: RelayTransportEvent,
    ) -> Result<(), RelayCoordinatorError> {
        match event {
            RelayTransportEvent::Renewed(_) | RelayTransportEvent::Publishable(_) => {}
            RelayTransportEvent::Unpublished { handle } => {
                self.observe_relay_failure(handle, ReservationFailureReason::ReservationLost)
                    .await?;
            }
            RelayTransportEvent::ConfirmationRejected { handle, reason } => {
                let failure = match reason {
                    RelayConfirmationRejection::MissingLimits
                    | RelayConfirmationRejection::IncompleteLimits { .. }
                    | RelayConfirmationRejection::LimitMismatch { .. } => {
                        ReservationFailureReason::LimitMismatch
                    }
                    _ => ReservationFailureReason::ReservationLost,
                };
                self.observe_relay_failure(handle, failure).await?;
            }
            RelayTransportEvent::Canceled { handle } => {
                self.observe_relay_failure(handle, ReservationFailureReason::ReservationLost)
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_relay_receive(
        &mut self,
        event: Result<RelayTransportEvent, broadcast::error::RecvError>,
    ) -> Result<(), RelayCoordinatorError> {
        match event {
            Ok(event) => self.handle_relay_event(event).await,
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                Err(RelayCoordinatorError::RelayEventLagged(skipped))
            }
            Err(broadcast::error::RecvError::Closed) => {
                self.relay_events = self.backend.subscribe()?;
                Ok(())
            }
        }
    }

    async fn observe_relay_failure(
        &mut self,
        handle: RelayReservationHandle,
        reason: ReservationFailureReason,
    ) -> Result<(), RelayCoordinatorError> {
        if let Some(fence) = self.confirmed_fence_for_handle(handle) {
            self.pending_relay_failures.remove(&handle);
            self.handle_local_failure(fence, reason).await?;
        } else if self.slots.values().any(|slot| {
            matches!(slot.state, LocalSlotState::Reserving(_))
                && slot.relay_peer_id == handle.relay_peer_id().to_string()
        }) {
            // Relay transport broadcasts can overtake Started/Retrying child
            // events. Hold exact-handle evidence until that generation is
            // consumed; never map a late old handle to a newer attempt by Peer ID.
            self.pending_relay_failures
                .entry(handle)
                .and_modify(|current| {
                    if reason == ReservationFailureReason::LimitMismatch {
                        *current = reason;
                    }
                })
                .or_insert(reason);
        }
        Ok(())
    }

    async fn handle_local_failure(
        &mut self,
        fence: LocalRelayFence,
        reason: ReservationFailureReason,
    ) -> Result<(), RelayCoordinatorError> {
        self.begin_retirement(fence, RetirementAction::ReportFailure(reason))
            .await?;
        Ok(())
    }

    async fn report_failure(
        &mut self,
        fence: LocalRelayFence,
        reason: ReservationFailureReason,
    ) -> Result<(), RelayCoordinatorError> {
        if !self.snapshot_has_fence(fence) {
            return Ok(());
        }
        let protected = self.next_renew.min(self.next_expiry);
        let until_protected = protected.saturating_duration_since(Instant::now());
        if until_protected <= self.config.http_timeout {
            let sender = self.events.clone();
            tokio::spawn(async move {
                tokio::time::sleep(until_protected + Duration::from_millis(1)).await;
                let _ = sender
                    .send(ChildEvent::RetryFailure { fence, reason })
                    .await;
            });
            return Ok(());
        }
        let request = ReservationFailedRequest {
            slot_id: fence.slot_id,
            assignment_id: fence.assignment_id,
            reservation_epoch: fence.reservation_epoch,
            reason,
        };
        match bounded_control_call(
            self.config.http_timeout,
            RelayOperation::ReservationFailed,
            self.api
                .report_reservation_failed(self.snapshot.booking_id, &request),
        )
        .await
        {
            Ok(snapshot) => {
                if self.is_current(fence) {
                    self.begin_retirement(fence, RetirementAction::Reconcile)
                        .await?;
                }
                self.apply_snapshot(snapshot).await?;
                self.next_poll = Instant::now() + self.config.status_poll_interval;
            }
            Err(error) => {
                warn!(error = %error, slot_id = %fence.slot_id, "reservation-failure report failed");
                if control_error_ends_authority(&error) {
                    self.fence_control_plane().await?;
                    return Err(RelayCoordinatorError::AuthorityEnded);
                }
                self.next_poll = Instant::now();
                if error.is_retryable() {
                    let sender = self.events.clone();
                    let delay = retry_jitter(self.config.retry_min, self.config.retry_max);
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        let _ = sender
                            .send(ChildEvent::RetryFailure { fence, reason })
                            .await;
                    });
                }
            }
        }
        Ok(())
    }

    async fn begin_retirement(
        &mut self,
        fence: LocalRelayFence,
        action: RetirementAction,
    ) -> Result<(), RelayCoordinatorError> {
        if self.retirement_already_started(fence, action) {
            return Ok(());
        }
        let Some(current) = self.slots.get(&fence.slot_id) else {
            return Ok(());
        };
        if current.fence != fence {
            return Ok(());
        }
        self.routes
            .tombstone(fence)
            .await
            .map_err(RelayCoordinatorError::RouteRegistry)?;
        self.start_retirement_after_route_fence(fence, action);
        Ok(())
    }

    fn retirement_already_started(
        &mut self,
        fence: LocalRelayFence,
        action: RetirementAction,
    ) -> bool {
        if let Some(retiring) = self.retiring.get_mut(&fence.slot_id) {
            if retiring.local.fence == fence && action == RetirementAction::Remove {
                retiring.action = RetirementAction::Remove;
            }
            return true;
        }
        false
    }

    fn start_retirement_after_route_fence(
        &mut self,
        fence: LocalRelayFence,
        action: RetirementAction,
    ) {
        let mut local = self
            .slots
            .remove(&fence.slot_id)
            .expect("the coordinator serializes slot removal");
        local.cancel_retry.cancel();
        let handle = match local.state {
            LocalSlotState::Confirmed(handle) | LocalSlotState::Reserving(Some(handle)) => {
                Some(handle)
            }
            _ => None,
        };
        let worker = local.worker.take();
        let cleanup = spawn_retirement_cleanup(
            Arc::clone(&self.backend),
            self.events.clone(),
            fence,
            worker,
            handle,
            self.config.reservation_retry_budget,
        );
        self.retiring.insert(
            fence.slot_id,
            RetiringSlot {
                local,
                action,
                cleanup,
            },
        );
        self.reset_expiry_timer();
    }

    async fn finish_retirement(
        &mut self,
        fence: LocalRelayFence,
    ) -> Result<(), RelayCoordinatorError> {
        let Some(retiring) = self.retiring.get_mut(&fence.slot_id) else {
            return Ok(());
        };
        if retiring.local.fence != fence {
            return Ok(());
        }
        (&mut retiring.cleanup)
            .await
            .map_err(|error| RelayCoordinatorError::ReservationCleanup(error.to_string()))?
            .map_err(RelayCoordinatorError::ReservationCleanup)?;
        let retiring = self
            .retiring
            .remove(&fence.slot_id)
            .expect("the completed retirement remains actor-owned");

        match retiring.action {
            RetirementAction::Reconcile => self.apply_current_snapshot().await?,
            RetirementAction::ReportFailure(reason) if self.snapshot_has_fence(fence) => {
                let mut local = retiring.local;
                local.state = LocalSlotState::ReportingFailure(reason);
                local.worker = None;
                self.slots.insert(fence.slot_id, local);
                self.reset_expiry_timer();
                self.report_failure(fence, reason).await?;
            }
            RetirementAction::ReportFailure(_) => self.apply_current_snapshot().await?,
            RetirementAction::Remove => {}
        }
        self.reset_expiry_timer();
        Ok(())
    }

    async fn remove_all_slots(&mut self) -> Result<(), RelayCoordinatorError> {
        let fences: Vec<_> = self.slots.values().map(|slot| slot.fence).collect();
        for fence in fences {
            self.begin_retirement(fence, RetirementAction::Remove)
                .await?;
        }
        for retiring in self.retiring.values_mut() {
            retiring.action = RetirementAction::Remove;
        }
        self.drain_cleanup_tasks().await
    }

    async fn force_fence(&mut self) -> Result<(), RelayCoordinatorError> {
        self.control_fenced = true;
        self.event_rx.close();
        let route_error = self.routes.fence_all().err();
        let fences: Vec<_> = self.slots.values().map(|slot| slot.fence).collect();
        for fence in fences {
            if !self.retirement_already_started(fence, RetirementAction::Remove)
                && self
                    .slots
                    .get(&fence.slot_id)
                    .is_some_and(|slot| slot.fence == fence)
            {
                self.start_retirement_after_route_fence(fence, RetirementAction::Remove);
            }
        }
        for retiring in self.retiring.values_mut() {
            retiring.action = RetirementAction::Remove;
        }
        let cleanup = self.drain_cleanup_tasks().await;
        if let Some(error) = route_error {
            return Err(RelayCoordinatorError::RouteRegistry(error));
        }
        cleanup
    }

    async fn drain_cleanup_tasks(&mut self) -> Result<(), RelayCoordinatorError> {
        let mut first_error = None;
        let retirement_ids: Vec<_> = self.retiring.keys().copied().collect();
        for slot_id in retirement_ids {
            let result = match self.retiring.get_mut(&slot_id) {
                Some(retiring) => (&mut retiring.cleanup).await,
                None => continue,
            };
            self.retiring.remove(&slot_id);
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    first_error.get_or_insert(error);
                }
                Err(error) => {
                    first_error.get_or_insert(error.to_string());
                }
            }
        }
        while let Some(result) = self.detached_cleanups.join_next().await {
            if let Err(error) = detached_cleanup_result(result) {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            return Err(RelayCoordinatorError::ReservationCleanup(error));
        }
        self.reset_expiry_timer();
        Ok(())
    }

    async fn stop(&mut self, delete_booking: bool) -> Result<(), RelayCoordinatorError> {
        self.event_rx.close();
        self.remove_all_slots().await?;
        if delete_booking && !self.control_fenced {
            let started = Instant::now();
            let mut retry_ceiling = self.config.retry_min;
            loop {
                match bounded_control_call(
                    self.config.http_timeout,
                    RelayOperation::Delete,
                    self.api.delete(self.snapshot.booking_id),
                )
                .await
                {
                    Ok(()) => break,
                    Err(error) if error.http_code() == Some(RelayErrorCode::AuthorityEnded) => {
                        break;
                    }
                    Err(error) if error.is_retryable() => {
                        let delay = retry_jitter(self.config.retry_min, retry_ceiling);
                        if started.elapsed().saturating_add(delay)
                            >= self.config.reservation_retry_budget
                        {
                            return Err(error.into());
                        }
                        tokio::time::sleep(delay).await;
                        retry_ceiling = (retry_ceiling * 2).min(self.config.retry_max);
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    async fn fence_control_plane(&mut self) -> Result<(), RelayCoordinatorError> {
        self.control_fenced = true;
        self.event_rx.close();
        self.remove_all_slots().await
    }

    fn is_current(&self, fence: LocalRelayFence) -> bool {
        self.slots
            .get(&fence.slot_id)
            .is_some_and(|slot| slot.fence == fence)
            && self.snapshot_has_fence(fence)
    }

    fn is_reserving(&self, fence: LocalRelayFence) -> bool {
        self.is_current(fence)
            && self
                .slots
                .get(&fence.slot_id)
                .is_some_and(|slot| matches!(slot.state, LocalSlotState::Reserving(_)))
    }

    async fn join_slot_worker(
        &mut self,
        fence: LocalRelayFence,
    ) -> Result<Option<RelayReservationHandle>, RelayCoordinatorError> {
        let worker = self
            .slots
            .get_mut(&fence.slot_id)
            .filter(|slot| slot.fence == fence)
            .and_then(|slot| slot.worker.as_mut());
        let result = match worker {
            Some(worker) => worker
                .await
                .map_err(|error| RelayCoordinatorError::ReservationCleanup(error.to_string()))?
                .map_err(RelayCoordinatorError::ReservationCleanup),
            None => Ok(None),
        };
        if let Some(slot) = self
            .slots
            .get_mut(&fence.slot_id)
            .filter(|slot| slot.fence == fence)
        {
            slot.worker.take();
        }
        result
    }

    fn confirmed_fence_for_handle(
        &self,
        handle: RelayReservationHandle,
    ) -> Option<LocalRelayFence> {
        self.slots.values().find_map(|slot| match slot.state {
            LocalSlotState::Confirmed(current) if current == handle => Some(slot.fence),
            _ => None,
        })
    }

    fn snapshot_has_fence(&self, fence: LocalRelayFence) -> bool {
        self.snapshot.slots.iter().any(|slot| {
            slot.slot_id == fence.slot_id
                && slot.assignment_id == Some(fence.assignment_id)
                && slot.reservation_epoch == Some(fence.reservation_epoch)
        })
    }

    fn schedule_status_poll(&mut self, immediate: bool) {
        self.next_poll = if immediate {
            Instant::now()
        } else {
            Instant::now() + self.status_poll_delay()
        };
    }

    fn status_poll_delay(&self) -> Duration {
        let configured = status_poll_jitter(self.config.status_poll_interval);
        let now = chrono::Utc::now();
        let deadline_delay = self
            .slots
            .values()
            .filter(|slot| !matches!(slot.state, LocalSlotState::ReportingFailure(_)))
            .map(|slot| slot.authorized_until)
            .min()
            .and_then(|deadline| deadline.signed_duration_since(now).to_std().ok())
            .map(|remaining| remaining.saturating_sub(self.config.http_timeout));
        deadline_delay
            .map(|delay| configured.min(delay).max(Duration::from_millis(1)))
            .unwrap_or(configured)
    }

    fn reset_expiry_timer(&mut self) {
        let now = chrono::Utc::now();
        let next = self
            .slots
            .values()
            .filter(|slot| !matches!(slot.state, LocalSlotState::ReportingFailure(_)))
            .map(|slot| slot.authorized_until)
            .min()
            .unwrap_or(now + chrono::Duration::hours(24));
        let delay = next.signed_duration_since(now).to_std().unwrap_or_default();
        self.next_expiry = Instant::now() + delay;
    }

    async fn expire_local_authority(&mut self) -> Result<(), RelayCoordinatorError> {
        let now = chrono::Utc::now();
        let expired: Vec<_> = self
            .slots
            .values()
            .filter(|slot| !matches!(slot.state, LocalSlotState::ReportingFailure(_)))
            .filter(|slot| slot.authorized_until <= now)
            .map(|slot| slot.fence)
            .collect();
        for fence in expired {
            self.begin_retirement(fence, RetirementAction::Remove)
                .await?;
        }
        self.reset_expiry_timer();
        Ok(())
    }
}

impl Drop for CoordinatorActor {
    fn drop(&mut self) {
        self.control_fenced = true;
        let _ = self.routes.fence_all();
        for slot in self.slots.values_mut() {
            slot.cancel_retry.cancel();
            if let Some(worker) = &slot.worker {
                worker.abort();
            }
        }
        for retiring in self.retiring.values() {
            retiring.cleanup.abort();
        }
        self.detached_cleanups.abort_all();
    }
}

fn control_error_ends_authority(error: &RelayBookingClientError) -> bool {
    error.http_code().is_some_and(|code| {
        code.is_stale_requester_principal()
            || code.is_invalid_requester_principal()
            || matches!(
                code,
                RelayErrorCode::AuthorityEnded | RelayErrorCode::NotFound
            )
    })
}

async fn bounded_control_call<T, F>(
    timeout: Duration,
    operation: RelayOperation,
    future: F,
) -> Result<T, RelayBookingClientError>
where
    F: Future<Output = Result<T, RelayBookingClientError>>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| RelayBookingClientError::Transport {
            operation,
            timeout: true,
        })?
}

fn spawn_retirement_cleanup(
    backend: SharedReservationBackend,
    sender: mpsc::Sender<ChildEvent>,
    fence: LocalRelayFence,
    worker: Option<AbortOnDropHandle<Result<Option<RelayReservationHandle>, String>>>,
    handle: Option<RelayReservationHandle>,
    timeout: Duration,
) -> AbortOnDropHandle<Result<(), String>> {
    AbortOnDropHandle::new(tokio::spawn(async move {
        let result = async {
            let mut cleanup_error = None;
            let mut worker_handle = None;
            if let Some(mut worker) = worker {
                match tokio::time::timeout(timeout, &mut worker).await {
                    Ok(Ok(Ok(returned_handle))) => worker_handle = returned_handle,
                    Ok(Ok(Err(error))) => cleanup_error = Some(error),
                    Ok(Err(error)) => {
                        cleanup_error = Some(format!("reservation worker failed: {error}"));
                    }
                    Err(_) => {
                        worker.abort();
                        let _ = worker.await;
                        cleanup_error = Some("reservation worker cleanup timed out".to_string());
                    }
                }
            }
            let mut candidates = Vec::with_capacity(2);
            if let Some(handle) = handle {
                candidates.push(handle);
            }
            if let Some(worker_handle) = worker_handle
                && !candidates.contains(&worker_handle)
            {
                candidates.push(worker_handle);
            }
            for candidate in candidates {
                if let Err(error) =
                    cancel_reservation_with_timeout(&backend, candidate, timeout).await
                {
                    cleanup_error.get_or_insert(error);
                }
            }
            cleanup_error.map_or(Ok(()), Err)
        }
        .await;
        let _ = sender.send(ChildEvent::CleanupComplete { fence }).await;
        result
    }))
}

fn detached_cleanup_result(
    result: Result<Result<(), String>, tokio::task::JoinError>,
) -> Result<(), String> {
    match result {
        Ok(result) => result,
        Err(error) => Err(format!("detached reservation cleanup task failed: {error}")),
    }
}

async fn cancel_reservation_with_timeout(
    backend: &SharedReservationBackend,
    handle: RelayReservationHandle,
    timeout: Duration,
) -> Result<(), String> {
    match tokio::time::timeout(timeout, backend.cancel(handle)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) if cancellation_already_complete(&error) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("reservation cancellation timed out".to_string()),
    }
}

fn cancellation_already_complete(error: &PeerRelayError) -> bool {
    matches!(error, PeerRelayError::Stopped)
        || matches!(
            error,
            PeerRelayError::P2p(auki_p2p::Error::RelayReservation(
                auki_p2p::RelayReservationError::StaleHandle
                    | auki_p2p::RelayReservationError::UnknownHandle
            ))
        )
}

fn preferred_failure_reason(
    observed: ReservationFailureReason,
    pending: ReservationFailureReason,
) -> ReservationFailureReason {
    if observed == ReservationFailureReason::LimitMismatch
        || pending == ReservationFailureReason::LimitMismatch
    {
        ReservationFailureReason::LimitMismatch
    } else {
        pending
    }
}

fn spawn_reservation_worker(
    backend: SharedReservationBackend,
    sender: mpsc::Sender<ChildEvent>,
    fence: LocalRelayFence,
    provider: RelayProvider,
    cancel_retry: CancellationToken,
    config: RelayCoordinatorConfig,
) -> AbortOnDropHandle<Result<Option<RelayReservationHandle>, String>> {
    AbortOnDropHandle::new(tokio::spawn(async move {
        let started = Instant::now();
        let mut retry_ceiling = config.retry_min;
        loop {
            if cancel_retry.is_cancelled() {
                return Ok(None);
            }
            let remaining_budget = config
                .reservation_retry_budget
                .saturating_sub(started.elapsed());
            if remaining_budget.is_zero() {
                return finish_reservation_worker(
                    &backend,
                    &sender,
                    fence,
                    None,
                    ReservationFailureReason::DialFailed,
                    config.reservation_retry_budget,
                )
                .await;
            }

            let attempt_backend = Arc::clone(&backend);
            let attempt_provider = provider.clone();
            let mut attempt = AbortOnDropHandle::new(tokio::spawn(async move {
                attempt_backend.start(attempt_provider).await
            }));
            let joined = tokio::select! {
                biased;
                _ = cancel_retry.cancelled() => {
                    return match abort_reservation_start(attempt).await? {
                        AbortedReservationStart::Handle(handle) => Ok(handle),
                        AbortedReservationStart::BackendStopped(handle) => {
                            finish_stopped_reservation_worker(&sender, handle).await
                        }
                    };
                }
                _ = tokio::time::sleep(remaining_budget) => {
                    let handle = match abort_reservation_start(attempt).await? {
                        AbortedReservationStart::Handle(handle) => handle,
                        AbortedReservationStart::BackendStopped(handle) => {
                            return finish_stopped_reservation_worker(&sender, handle).await;
                        }
                    };
                    return finish_reservation_worker(
                        &backend,
                        &sender,
                        fence,
                        handle,
                        ReservationFailureReason::DialFailed,
                        config.reservation_retry_budget,
                    ).await;
                }
                joined = &mut attempt => joined,
            };
            let start_result = match joined {
                Ok(result) => result,
                Err(_) => Err(ReservationAttemptFailure::Provider {
                    handle: None,
                    reason: ReservationFailureReason::DialFailed,
                    retryable: false,
                }),
            };
            let failure = if let Ok(handle) = start_result {
                if sender
                    .send(ChildEvent::Started { fence, handle })
                    .await
                    .is_err()
                {
                    cancel_reservation_with_timeout(
                        &backend,
                        handle,
                        config.reservation_retry_budget,
                    )
                    .await?;
                    return Ok(None);
                }
                let remaining_budget = config
                    .reservation_retry_budget
                    .saturating_sub(started.elapsed());
                if remaining_budget.is_zero() {
                    return finish_reservation_worker(
                        &backend,
                        &sender,
                        fence,
                        Some(handle),
                        ReservationFailureReason::ReservationDenied,
                        config.reservation_retry_budget,
                    )
                    .await;
                }
                let wait_backend = Arc::clone(&backend);
                let wait = async move { wait_backend.wait(handle).await };
                tokio::pin!(wait);
                let result = tokio::select! {
                    biased;
                    _ = cancel_retry.cancelled() => {
                        return Ok(Some(handle));
                    }
                    _ = tokio::time::sleep(remaining_budget) => {
                        return finish_reservation_worker(
                            &backend,
                            &sender,
                            fence,
                            Some(handle),
                            ReservationFailureReason::ReservationDenied,
                            config.reservation_retry_budget,
                        ).await;
                    }
                    result = &mut wait => result,
                };
                match result {
                    Ok(snapshot) => {
                        if sender
                            .send(ChildEvent::Confirmed {
                                fence,
                                snapshot: snapshot.clone(),
                            })
                            .await
                            .is_err()
                        {
                            cancel_reservation_with_timeout(
                                &backend,
                                snapshot.handle(),
                                config.reservation_retry_budget,
                            )
                            .await?;
                            return Ok(None);
                        }
                        return Ok(Some(snapshot.handle()));
                    }
                    Err(failure) => failure,
                }
            } else {
                start_result.expect_err("the reservation start result was checked")
            };
            let (failure_handle, failure_reason, retryable) = match failure {
                ReservationAttemptFailure::Provider {
                    handle,
                    reason,
                    retryable,
                } => (handle, reason, retryable),
                ReservationAttemptFailure::BackendStopped { handle } => {
                    return finish_stopped_reservation_worker(&sender, handle).await;
                }
            };
            if !retryable || started.elapsed() >= config.reservation_retry_budget {
                return finish_reservation_worker(
                    &backend,
                    &sender,
                    fence,
                    failure_handle,
                    failure_reason,
                    config.reservation_retry_budget,
                )
                .await;
            }
            if let Some(handle) = failure_handle {
                if cancel_reservation_with_timeout(
                    &backend,
                    handle,
                    config.reservation_retry_budget,
                )
                .await
                .is_err()
                {
                    return finish_reservation_worker(
                        &backend,
                        &sender,
                        fence,
                        Some(handle),
                        failure_reason,
                        config.reservation_retry_budget,
                    )
                    .await;
                }
                if sender
                    .send(ChildEvent::Retrying { fence, handle })
                    .await
                    .is_err()
                {
                    return Ok(None);
                }
            }
            let delay = retry_jitter(config.retry_min, retry_ceiling);
            if started.elapsed().saturating_add(delay) >= config.reservation_retry_budget {
                return finish_reservation_worker(
                    &backend,
                    &sender,
                    fence,
                    None,
                    failure_reason,
                    config.reservation_retry_budget,
                )
                .await;
            }
            tokio::select! {
                _ = cancel_retry.cancelled() => return Ok(None),
                _ = tokio::time::sleep(delay) => {}
            }
            retry_ceiling = (retry_ceiling * 2).min(config.retry_max);
        }
    }))
}

enum AbortedReservationStart {
    Handle(Option<RelayReservationHandle>),
    BackendStopped(Option<RelayReservationHandle>),
}

async fn abort_reservation_start(
    attempt: AbortOnDropHandle<Result<RelayReservationHandle, ReservationAttemptFailure>>,
) -> Result<AbortedReservationStart, String> {
    attempt.abort();
    match attempt.await {
        Ok(Ok(handle)) => Ok(AbortedReservationStart::Handle(Some(handle))),
        Ok(Err(ReservationAttemptFailure::Provider { handle, .. })) => {
            Ok(AbortedReservationStart::Handle(handle))
        }
        Ok(Err(ReservationAttemptFailure::BackendStopped { handle })) => {
            Ok(AbortedReservationStart::BackendStopped(handle))
        }
        Err(error) if error.is_cancelled() => Ok(AbortedReservationStart::Handle(None)),
        Err(error) => Err(format!("reservation start task failed: {error}")),
    }
}

async fn finish_stopped_reservation_worker(
    sender: &mpsc::Sender<ChildEvent>,
    handle: Option<RelayReservationHandle>,
) -> Result<Option<RelayReservationHandle>, String> {
    let _ = sender.send(ChildEvent::BackendStopped).await;
    Ok(handle)
}

async fn finish_reservation_worker(
    backend: &SharedReservationBackend,
    sender: &mpsc::Sender<ChildEvent>,
    fence: LocalRelayFence,
    handle: Option<RelayReservationHandle>,
    reason: ReservationFailureReason,
    timeout: Duration,
) -> Result<Option<RelayReservationHandle>, String> {
    if sender
        .send(ChildEvent::Failed {
            fence,
            handle,
            reason,
        })
        .await
        .is_ok()
    {
        return Ok(handle);
    }
    if let Some(handle) = handle {
        cancel_reservation_with_timeout(backend, handle, timeout).await?;
    }
    Ok(None)
}

fn retry_jitter(minimum: Duration, maximum: Duration) -> Duration {
    let minimum_ms = u64::try_from(minimum.as_millis()).unwrap_or(u64::MAX);
    let maximum_ms = u64::try_from(maximum.max(minimum).as_millis()).unwrap_or(u64::MAX);
    Duration::from_millis(rand::thread_rng().gen_range(minimum_ms..=maximum_ms))
}

fn status_poll_jitter(interval: Duration) -> Duration {
    let interval_ms = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX);
    let minimum_ms = interval_ms.saturating_mul(80) / 100;
    let maximum_ms = interval_ms.saturating_mul(120) / 100;
    Duration::from_millis(rand::thread_rng().gen_range(minimum_ms..=maximum_ms))
}

fn validate_booking_matches(
    snapshot: &RelayBookingSnapshot,
    config: &RelayCoordinatorConfig,
) -> Result<(), RelayCoordinatorError> {
    let expected = RelayBookingExpectation {
        mode: config.mode,
        requested_duration_seconds: config.requested_duration_seconds,
        relay_count: config.relay_count,
    };
    match validate_active_booking(snapshot, expected) {
        ActiveBookingValidation::Match => Ok(()),
        ActiveBookingValidation::PolicyMismatch => {
            Err(RelayCoordinatorError::ActiveBookingMismatch)
        }
        ActiveBookingValidation::Ended => Err(RelayCoordinatorError::AuthorityEnded),
    }
}

#[cfg(test)]
mod tests;
