use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

use async_trait::async_trait;
use auki_auth::{AuthorityRenewal, PreparedPeer};
use auki_p2p::{
    BrowserAuthority, BrowserNode, BrowserNodeExit, Identity, Multiaddr, PeerAuthorityUpdate,
    PeerId, RelayBaseTransport, RelayReservationError,
};
use auki_relay_booking::{
    CreateRelayBookingRequest, RelayAuthorizationError, RelayAuthorizationProvider,
    RelayAuthorizationSnapshot, RelayBookingApi, RelayBookingClient, RelayBookingClientError,
    RelayErrorCode, RelayIdempotencyKey,
};
use chrono::{DateTime, Utc};
use futures::{
    FutureExt,
    channel::oneshot,
    future::{Either, Shared, select},
    lock::Mutex,
    pin_mut,
};
use futures_timer::Delay;
use reqwest::header::HeaderValue;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;

use crate::{
    AukiPeerConfig, AukiRelayConfig,
    browser_booking::{
        ReadyRelay, ReadyRelayError, booking_mode, booking_renewal_delay_at, matches_ready_relay,
        pull_booking_renewal_forward, ready_relay, relay_renewal_start_deadline,
        relay_usable_until,
    },
    browser_protocols::AukiPeerProtocols,
    protocol_contract::AukiProtocolError,
};

const RELAY_RETRY: Duration = Duration::from_secs(2);

/// One browser Peer with renewable DDS authority and mandatory relay reachability.
pub struct AukiPeer {
    node: Rc<BrowserNode>,
    reachability: AukiPeerReachability,
    protocols: AukiPeerProtocols,
    relay: RelaySupervisor,
    closed: bool,
}

/// Confirmed routes for reaching one browser Peer through its relay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AukiPeerReachability {
    wss: Multiaddr,
    tcp: Option<Multiaddr>,
}

/// Clone-only observation of one browser Peer's terminal lifecycle result.
///
/// The observer does not retain the Peer, relay booking, authority, or
/// transport. Every clone receives the same terminal result.
#[derive(Clone)]
pub struct AukiPeerLifecycle {
    stopped: Shared<oneshot::Receiver<SupervisorExit>>,
}

impl AukiPeerLifecycle {
    /// Wait until the browser Peer and its relay supervisor have stopped.
    pub async fn wait_stopped(&self) -> AukiPeerExit {
        peer_exit(wait_for_supervisor(self.stopped.clone()).await)
    }
}

impl AukiPeerReachability {
    /// Exact WSS circuit route suitable for another browser Peer.
    pub fn wss(&self) -> &Multiaddr {
        &self.wss
    }

    /// Exact TCP circuit route suitable for a native Peer, when advertised by DMS.
    pub fn tcp(&self) -> Option<&Multiaddr> {
        self.tcp.as_ref()
    }
}

impl AukiPeer {
    /// Start one browser Peer and return only after its WSS relay reservation is publishable.
    pub async fn start(
        identity: Identity,
        prepared: PreparedPeer,
        config: AukiPeerConfig,
    ) -> Result<Self, AukiPeerError> {
        let identity_peer_id = identity.peer_id();
        if prepared.peer_id != identity_peer_id {
            return Err(AukiPeerError::IdentityMismatch {
                identity: identity_peer_id.to_string(),
                authorized: prepared.peer_id.to_string(),
            });
        }

        let PreparedPeer {
            domain,
            peer_id,
            initial_credential,
            verification_keys,
            credential_expires_at,
            renew_at,
            renewal,
        } = prepared;
        let initial_header = initial_credential.to_sensitive_bearer_header()?;
        let initial_update = PeerAuthorityUpdate::new(
            domain.id,
            peer_id,
            verification_keys,
            initial_credential,
            credential_expires_at,
        );
        let node = Rc::new(BrowserNode::start(identity, initial_update).await?);
        let authority = Arc::new(AuthorityManager::new(
            node.authority(),
            renewal,
            CurrentAuthority {
                header: initial_header,
                revision: 1,
                renew_at,
                expires_at: credential_expires_at,
            },
        ));
        let relay_policy = config.relay().ok_or(AukiPeerError::RelayRequired)?;
        let relay_client = RelayBookingClient::new(config.dms_base().clone(), authority.clone())?;
        let acquisition_booking_id = Cell::new(None);
        let ready = {
            let acquisition =
                acquire_ready_relay(&relay_client, &acquisition_booking_id, relay_policy).fuse();
            let stopped = node.wait_stopped().fuse();
            pin_mut!(acquisition, stopped);
            match select(acquisition, stopped).await {
                Either::Left((Ok(ready), _)) => ready,
                Either::Left((Err(error), _)) => {
                    cleanup_startup(
                        &node,
                        &relay_client,
                        &authority,
                        acquisition_booking_id.get(),
                    )
                    .await;
                    return Err(error);
                }
                Either::Right((status, _)) => {
                    cleanup_startup(
                        &node,
                        &relay_client,
                        &authority,
                        acquisition_booking_id.get(),
                    )
                    .await;
                    return Err(AukiPeerError::NodeStoppedDuringStartup { status });
                }
            }
        };
        let (wss, tcp) = match relay_routes(&ready, identity_peer_id) {
            Ok(routes) => routes,
            Err(error) => {
                cleanup_startup(&node, &relay_client, &authority, Some(ready.booking_id)).await;
                return Err(error);
            }
        };

        let reservation = {
            let reservation = node.reserve_relay(ready.provider.clone()).fuse();
            let stopped = node.wait_stopped().fuse();
            pin_mut!(reservation, stopped);
            match select(reservation, stopped).await {
                Either::Left((Ok(reservation), _)) => reservation,
                Either::Left((Err(error), _)) => {
                    cleanup_startup(&node, &relay_client, &authority, Some(ready.booking_id)).await;
                    return Err(error.into());
                }
                Either::Right((status, _)) => {
                    cleanup_startup(&node, &relay_client, &authority, Some(ready.booking_id)).await;
                    return Err(AukiPeerError::NodeStoppedDuringStartup { status });
                }
            }
        };
        if reservation.publishable_route() != Some(&wss) {
            cleanup_startup(&node, &relay_client, &authority, Some(ready.booking_id)).await;
            return Err(AukiPeerError::RelayReservationMismatch);
        }
        let ready = {
            let confirmation = confirm_ready_relay(&relay_client, &ready, relay_policy).fuse();
            let stopped = node.wait_stopped().fuse();
            pin_mut!(confirmation, stopped);
            match select(confirmation, stopped).await {
                Either::Left((Ok(ready), _)) => ready,
                Either::Left((Err(error), _)) => {
                    cleanup_startup(&node, &relay_client, &authority, Some(ready.booking_id)).await;
                    return Err(error);
                }
                Either::Right((status, _)) => {
                    cleanup_startup(&node, &relay_client, &authority, Some(ready.booking_id)).await;
                    return Err(AukiPeerError::NodeStoppedDuringStartup { status });
                }
            }
        };
        let reachability = AukiPeerReachability { wss, tcp };
        let protocols = AukiPeerProtocols::new(Rc::clone(&node), node.domain_id());
        let relay = RelaySupervisor::start(
            relay_client,
            Arc::clone(&authority),
            Rc::clone(&node),
            protocols.clone(),
            ready,
            relay_policy,
        );

        Ok(Self {
            node,
            reachability,
            protocols,
            relay,
            closed: false,
        })
    }

    pub fn peer_id(&self) -> PeerId {
        self.node.peer_id()
    }

    pub fn domain_id(&self) -> Uuid {
        self.node.domain_id()
    }

    pub fn reachability(&self) -> &AukiPeerReachability {
        &self.reachability
    }

    /// Authenticated application protocol registration and opening surface.
    pub fn protocols(&self) -> AukiPeerProtocols {
        self.protocols.clone()
    }

    /// Observe terminal lifecycle without retaining this Peer owner.
    pub fn lifecycle(&self) -> AukiPeerLifecycle {
        self.relay.lifecycle()
    }

    /// Wait until either the browser swarm or its authority/booking supervisor stops.
    pub async fn wait_stopped(&self) -> AukiPeerExit {
        self.lifecycle().wait_stopped().await
    }

    /// Stop the node, delete the booking, and clear authority.
    ///
    /// Protocol handlers stop before the relay booking, authority, and browser
    /// transport are torn down. The owner is consumed so successful return is
    /// the cleanup barrier.
    pub async fn shutdown(mut self) -> Result<(), AukiPeerShutdownError> {
        let protocol_failure = self.protocols.shutdown_all().await.err();
        let relay_failure = match self.relay.stop().await {
            SupervisorExit::Failed { reason } => Some(reason),
            SupervisorExit::Shutdown | SupervisorExit::OwnersDropped | SupervisorExit::Node(_) => {
                None
            }
        };
        self.closed = true;
        match (protocol_failure, relay_failure) {
            (None, None) => Ok(()),
            (protocols, relay) => Err(AukiPeerError::Shutdown {
                details: shutdown_details(protocols, relay),
            }),
        }
    }
}

impl Drop for AukiPeer {
    fn drop(&mut self) {
        if !self.closed {
            self.protocols.abort_all();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AukiPeerExit {
    Node(BrowserNodeExit),
    SupervisorFailed { reason: String },
    SupervisorStopped,
}

#[derive(Debug, thiserror::Error)]
pub enum AukiPeerError {
    #[error("authorized Peer ID {authorized} does not match identity Peer ID {identity}")]
    IdentityMismatch {
        identity: String,
        authorized: String,
    },
    #[error("P2P runtime failed: {0}")]
    P2p(#[from] auki_p2p::Error),
    #[error("DMS relay booking failed: {0}")]
    Relay(#[from] RelayBookingClientError),
    #[error("DMS relay selection failed: {reason}")]
    RelaySelection { reason: String },
    #[error("relay route construction failed: {0}")]
    RelayRoute(#[from] RelayReservationError),
    #[error("browser peers require relay-backed reachability")]
    RelayRequired,
    #[error("confirmed relay reservation route differs from its selected provider")]
    RelayReservationMismatch,
    #[error("browser P2P node stopped during startup: {status:?}")]
    NodeStoppedDuringStartup { status: BrowserNodeExit },
    #[error("DMS changed or withdrew the relay assignment during startup")]
    RelayChangedDuringStartup,
    #[error("relay authority or provider lease reached its safety deadline during startup")]
    RelayAuthorityEndedDuringStartup,
    #[error("Web Peer shutdown was incomplete: {details}")]
    Shutdown { details: String },
}

pub type AukiPeerStartError = AukiPeerError;
pub type AukiPeerShutdownError = AukiPeerError;

fn shutdown_details(protocols: Option<AukiProtocolError>, relay: Option<String>) -> String {
    match (protocols, relay) {
        (Some(protocols), Some(relay)) => {
            format!("protocol cleanup failed: {protocols}; relay cleanup failed: {relay}")
        }
        (Some(protocols), None) => format!("protocol cleanup failed: {protocols}"),
        (None, Some(relay)) => format!("relay cleanup failed: {relay}"),
        (None, None) => unreachable!("shutdown details require at least one failure"),
    }
}

impl From<ReadyRelayError> for AukiPeerError {
    fn from(error: ReadyRelayError) -> Self {
        Self::RelaySelection {
            reason: error.to_string(),
        }
    }
}

async fn acquire_ready_relay(
    client: &RelayBookingClient,
    adopted_booking_id: &Cell<Option<Uuid>>,
    policy: AukiRelayConfig,
) -> Result<ReadyRelay, AukiPeerError> {
    let request = CreateRelayBookingRequest::new(
        booking_mode(policy),
        policy.requested_duration.as_secs(),
        policy.relay_count,
    )?;
    let idempotency_key =
        RelayIdempotencyKey::new(format!("auki-sdk-browser-relay-{}", Uuid::new_v4()))?;

    loop {
        let (snapshot, created_by_this_attempt) = match client.active().await {
            Ok(Some(snapshot)) => (snapshot, false),
            Ok(None) => match client.create(&idempotency_key, &request).await {
                Ok(created) => (created.snapshot, true),
                Err(error)
                    if error.http_code() == Some(RelayErrorCode::ActiveBookingConflict)
                        || error.is_retryable() =>
                {
                    Delay::new(error.retry_after().unwrap_or(RELAY_RETRY)).await;
                    continue;
                }
                Err(error) => return Err(error.into()),
            },
            Err(error) if error.is_retryable() => {
                Delay::new(error.retry_after().unwrap_or(RELAY_RETRY)).await;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if created_by_this_attempt {
            adopted_booking_id.set(Some(snapshot.booking_id));
        }
        let ready = ready_relay(&snapshot, policy)?;
        adopted_booking_id.set(Some(snapshot.booking_id));
        if let Some(ready) = ready {
            return Ok(ready);
        }
        Delay::new(policy.status_poll_interval).await;
    }
}

fn relay_routes(
    ready: &ReadyRelay,
    peer_id: PeerId,
) -> Result<(Multiaddr, Option<Multiaddr>), AukiPeerError> {
    let wss = ready
        .provider
        .circuit_route_for_transport(RelayBaseTransport::Wss, peer_id)?;
    let tcp = ready
        .provider
        .base_for_transport(RelayBaseTransport::Tcp)
        .map(|_| {
            ready
                .provider
                .circuit_route_for_transport(RelayBaseTransport::Tcp, peer_id)
        })
        .transpose()?;
    Ok((wss, tcp))
}

async fn confirm_ready_relay(
    client: &RelayBookingClient,
    pinned: &ReadyRelay,
    policy: AukiRelayConfig,
) -> Result<ReadyRelay, AukiPeerError> {
    let snapshot = client
        .active()
        .await?
        .ok_or(AukiPeerError::RelayChangedDuringStartup)?;
    if !matches_ready_relay(pinned, &snapshot, policy)? {
        return Err(AukiPeerError::RelayChangedDuringStartup);
    }
    let ready = ready_relay(&snapshot, policy)?.ok_or(AukiPeerError::RelayChangedDuringStartup)?;
    if Utc::now() >= relay_usable_until(&ready) {
        return Err(AukiPeerError::RelayAuthorityEndedDuringStartup);
    }
    Ok(ready)
}

async fn cleanup_startup(
    node: &BrowserNode,
    client: &RelayBookingClient,
    authority: &AuthorityManager,
    known_booking_id: Option<Uuid>,
) {
    let _ = node.shutdown().await;
    if let Some(booking_id) = known_booking_id {
        let _ = client.delete(booking_id).await;
    }
    authority.stop().await;
}

struct AuthorityManager {
    authority: BrowserAuthority,
    cancellation: CancellationToken,
    state: Mutex<AuthorityState>,
}

struct AuthorityState {
    renewal: AuthorityRenewal,
    current: CurrentAuthority,
    pending: Option<PendingAuthority>,
    stopped: bool,
}

struct CurrentAuthority {
    header: HeaderValue,
    revision: u64,
    renew_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

struct PendingAuthority {
    update: PeerAuthorityUpdate,
    header: HeaderValue,
    renew_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl AuthorityManager {
    fn new(
        authority: BrowserAuthority,
        renewal: AuthorityRenewal,
        current: CurrentAuthority,
    ) -> Self {
        Self {
            authority,
            cancellation: CancellationToken::new(),
            state: Mutex::new(AuthorityState {
                renewal,
                current,
                pending: None,
                stopped: false,
            }),
        }
    }

    async fn maintain(&self) -> Result<(), AuthorityManagerError> {
        let mut state = self.state.lock().await;
        if state.stopped {
            return Err(AuthorityManagerError::Stopped);
        }
        let now = Utc::now();
        if state.pending.is_none() && now < state.current.renew_at {
            return Ok(());
        }
        match self.renew_locked(&mut state).await {
            Ok(()) => Ok(()),
            Err(error) if state.current.expires_at > Utc::now() => {
                warn!(error = %error, "browser authority renewal will retry before expiry");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    async fn renew_after_unauthorized(
        &self,
        rejected_revision: u64,
    ) -> Result<(), AuthorityManagerError> {
        let mut state = self.state.lock().await;
        if state.stopped {
            return Err(AuthorityManagerError::Stopped);
        }
        if state.current.revision > rejected_revision {
            return Ok(());
        }
        if state.current.revision != rejected_revision {
            return Err(AuthorityManagerError::StaleRevision);
        }
        self.renew_locked(&mut state).await
    }

    async fn renew_locked(&self, state: &mut AuthorityState) -> Result<(), AuthorityManagerError> {
        if state.pending.is_none() {
            let renewed = state
                .renewal
                .renew_with_cancellation(&self.cancellation)
                .await?;
            let header = renewed.credential.to_sensitive_bearer_header()?;
            state.pending = Some(PendingAuthority {
                update: PeerAuthorityUpdate::new(
                    renewed.domain.id,
                    renewed.peer_id,
                    renewed.verification_keys,
                    renewed.credential,
                    renewed.credential_expires_at,
                ),
                header,
                renew_at: renewed.renew_at,
                expires_at: renewed.credential_expires_at,
            });
        }
        let pending = state
            .pending
            .as_ref()
            .expect("pending authority was just installed");
        let next_revision = state
            .current
            .revision
            .checked_add(1)
            .ok_or(AuthorityManagerError::RevisionExhausted)?;
        self.authority.replace(&pending.update).await?;
        let pending = state
            .pending
            .take()
            .expect("accepted pending authority exists");
        state.current = CurrentAuthority {
            header: pending.header,
            revision: next_revision,
            renew_at: pending.renew_at,
            expires_at: pending.expires_at,
        };
        Ok(())
    }

    async fn fence_peer(&self) {
        self.authority.clear().await;
    }

    fn cancel_renewal(&self) {
        self.cancellation.cancel();
    }

    async fn stop(&self) {
        self.cancel_renewal();
        let mut state = self.state.lock().await;
        state.stopped = true;
        state.pending = None;
        drop(state);
        self.fence_peer().await;
    }
}

#[async_trait(?Send)]
impl RelayAuthorizationProvider for AuthorityManager {
    async fn authorization(&self) -> Result<RelayAuthorizationSnapshot, RelayAuthorizationError> {
        self.maintain().await.map_err(|error| {
            warn!(error = %error, "browser authority maintenance before DMS request failed");
            RelayAuthorizationError
        })?;
        let state = self.state.lock().await;
        if state.stopped || state.current.expires_at <= Utc::now() {
            return Err(RelayAuthorizationError);
        }
        Ok(RelayAuthorizationSnapshot::new(
            state.current.header.clone(),
            state.current.revision,
        ))
    }

    async fn refresh_after_unauthorized(
        &self,
        rejected_revision: u64,
    ) -> Result<(), RelayAuthorizationError> {
        self.renew_after_unauthorized(rejected_revision)
            .await
            .map_err(|error| {
                warn!(error = %error, "browser authority refresh after DMS 401 failed");
                RelayAuthorizationError
            })
    }
}

#[derive(Debug, thiserror::Error)]
enum AuthorityManagerError {
    #[error("authority renewal failed: {0}")]
    Renewal(#[from] auki_auth::Error),
    #[error("authority installation failed: {0}")]
    Installation(#[from] auki_p2p::Error),
    #[error("authority is stopped")]
    Stopped,
    #[error("unauthorized refresh referenced a stale credential revision")]
    StaleRevision,
    #[error("authority credential revision exhausted")]
    RevisionExhausted,
}

struct RelaySupervisor {
    stop: async_channel::Sender<()>,
    stopped: Shared<oneshot::Receiver<SupervisorExit>>,
    authority: Arc<AuthorityManager>,
}

impl RelaySupervisor {
    fn start(
        client: RelayBookingClient,
        authority: Arc<AuthorityManager>,
        node: Rc<BrowserNode>,
        protocols: AukiPeerProtocols,
        ready: ReadyRelay,
        policy: AukiRelayConfig,
    ) -> Self {
        let (stop, stop_receiver) = async_channel::bounded(1);
        let (stopped_sender, stopped_receiver) = oneshot::channel();
        let booking_id = ready.booking_id;
        let task_authority = Arc::clone(&authority);
        spawn_local(async move {
            let end = supervise_relay(
                &client,
                Arc::clone(&task_authority),
                Rc::clone(&node),
                ready,
                policy,
                stop_receiver,
            )
            .await;
            if matches!(&end, SupervisionEnd::Node(_) | SupervisionEnd::Failed(_)) {
                protocols.abort_all();
            }
            task_authority.cancel_renewal();
            let status = finish_supervision(end, &client, &task_authority, &node, booking_id).await;
            let _ = stopped_sender.send(status);
        });
        Self {
            stop,
            stopped: stopped_receiver.shared(),
            authority,
        }
    }

    fn lifecycle(&self) -> AukiPeerLifecycle {
        AukiPeerLifecycle {
            stopped: self.stopped.clone(),
        }
    }

    async fn wait_stopped(&self) -> SupervisorExit {
        wait_for_supervisor(self.stopped.clone()).await
    }

    async fn stop(&self) -> SupervisorExit {
        self.authority.cancel_renewal();
        let _ = self.stop.try_send(());
        self.wait_stopped().await
    }
}

impl Drop for RelaySupervisor {
    fn drop(&mut self) {
        self.authority.cancel_renewal();
        let _ = self.stop.try_send(());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SupervisorExit {
    Shutdown,
    OwnersDropped,
    Node(BrowserNodeExit),
    Failed { reason: String },
}

fn peer_exit(exit: SupervisorExit) -> AukiPeerExit {
    match exit {
        SupervisorExit::Node(status) => AukiPeerExit::Node(status),
        SupervisorExit::Failed { reason } => AukiPeerExit::SupervisorFailed { reason },
        SupervisorExit::Shutdown | SupervisorExit::OwnersDropped => AukiPeerExit::SupervisorStopped,
    }
}

async fn wait_for_supervisor(stopped: Shared<oneshot::Receiver<SupervisorExit>>) -> SupervisorExit {
    stopped.await.unwrap_or_else(|_| SupervisorExit::Failed {
        reason: "browser relay supervisor status channel dropped".into(),
    })
}

enum SupervisionEnd {
    Shutdown,
    OwnersDropped,
    Node(BrowserNodeExit),
    Failed(SupervisorError),
}

async fn supervise_relay(
    client: &RelayBookingClient,
    authority: Arc<AuthorityManager>,
    node: Rc<BrowserNode>,
    mut pinned: ReadyRelay,
    policy: AukiRelayConfig,
    stop: async_channel::Receiver<()>,
) -> SupervisionEnd {
    let now = Utc::now();
    let mut next_renew = now + chrono_duration(booking_renewal_delay_at(&pinned, now));
    let mut next_poll = now + chrono_duration(policy.status_poll_interval);
    loop {
        let iteration = relay_iteration(
            client,
            &authority,
            &mut pinned,
            &mut next_renew,
            &mut next_poll,
            policy,
        )
        .fuse();
        let shutdown = stop.recv().fuse();
        let node_stopped = node.wait_stopped().fuse();
        pin_mut!(iteration, shutdown, node_stopped);
        futures::select_biased! {
            signal = shutdown => {
                return if signal.is_ok() {
                    SupervisionEnd::Shutdown
                } else {
                    SupervisionEnd::OwnersDropped
                };
            }
            status = node_stopped => return SupervisionEnd::Node(status),
            result = iteration => {
                if let Err(error) = result {
                    return SupervisionEnd::Failed(error);
                }
            }
        }
    }
}

async fn relay_iteration(
    client: &RelayBookingClient,
    authority: &AuthorityManager,
    pinned: &mut ReadyRelay,
    next_renew: &mut DateTime<Utc>,
    next_poll: &mut DateTime<Utc>,
    policy: AukiRelayConfig,
) -> Result<(), SupervisorError> {
    let now = Utc::now();
    ensure_relay_usable(pinned, now)?;
    let wake_at = (*next_renew)
        .min(*next_poll)
        .min(relay_usable_until(pinned));
    let delay = wake_at
        .signed_duration_since(now)
        .to_std()
        .unwrap_or_default();
    Delay::new(delay).await;

    let now = Utc::now();
    ensure_relay_usable(pinned, now)?;
    authority.maintain().await?;
    let now = Utc::now();
    ensure_relay_usable(pinned, now)?;
    let should_renew = now >= *next_renew;

    if should_renew {
        match client.renew(pinned.booking_id).await {
            Ok(snapshot) => {
                apply_relay_snapshot(pinned, &snapshot, policy)?;
                let now = Utc::now();
                ensure_relay_usable(pinned, now)?;
                *next_renew = now + chrono_duration(booking_renewal_delay_at(pinned, now));
                *next_poll = now + chrono_duration(policy.status_poll_interval);
            }
            Err(error) if error.is_retryable() => {
                let retry = error.retry_after().unwrap_or(RELAY_RETRY);
                let now = Utc::now();
                let latest_retry = relay_renewal_start_deadline(pinned);
                if now >= latest_retry {
                    return Err(SupervisorError::RelayAuthorityEnded);
                }
                *next_renew = (now + chrono_duration(retry)).min(latest_retry);
                warn!(error = %error, ?retry, "browser relay renewal will retry");
            }
            Err(error) => return Err(error.into()),
        }
        return Ok(());
    }

    if now >= *next_poll {
        match client.active().await {
            Ok(Some(snapshot)) => {
                apply_relay_snapshot(pinned, &snapshot, policy)?;
                let now = Utc::now();
                ensure_relay_usable(pinned, now)?;
                *next_renew = pull_booking_renewal_forward(*next_renew, pinned, now);
                *next_poll = now + chrono_duration(policy.status_poll_interval);
            }
            Ok(None) => return Err(SupervisorError::BookingDisappeared),
            Err(error) if error.is_retryable() => {
                let retry = error.retry_after().unwrap_or(RELAY_RETRY);
                *next_poll = Utc::now() + chrono_duration(retry);
                warn!(error = %error, ?retry, "browser relay status poll will retry");
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn apply_relay_snapshot(
    pinned: &mut ReadyRelay,
    snapshot: &auki_relay_booking::RelayBookingSnapshot,
    policy: AukiRelayConfig,
) -> Result<(), SupervisorError> {
    if !matches_ready_relay(pinned, snapshot, policy)? {
        return Err(SupervisorError::RelayChanged);
    }
    *pinned = ready_relay(snapshot, policy)?.ok_or(SupervisorError::RelayChanged)?;
    Ok(())
}

fn ensure_relay_usable(ready: &ReadyRelay, now: DateTime<Utc>) -> Result<(), SupervisorError> {
    if now >= relay_usable_until(ready) {
        return Err(SupervisorError::RelayAuthorityEnded);
    }
    Ok(())
}

async fn finish_supervision(
    end: SupervisionEnd,
    client: &RelayBookingClient,
    authority: &AuthorityManager,
    node: &BrowserNode,
    booking_id: Uuid,
) -> SupervisorExit {
    let (success, root_failure, context) = match end {
        SupervisionEnd::Shutdown => (SupervisorExit::Shutdown, None, "shutdown".to_owned()),
        SupervisionEnd::OwnersDropped => (
            SupervisorExit::OwnersDropped,
            None,
            "all Peer owners dropped".to_owned(),
        ),
        SupervisionEnd::Node(status) => (
            SupervisorExit::Node(status.clone()),
            None,
            format!("browser node stopped: {status:?}"),
        ),
        SupervisionEnd::Failed(error) => {
            let reason = error.to_string();
            (
                SupervisorExit::Failed {
                    reason: reason.clone(),
                },
                Some(reason.clone()),
                reason,
            )
        }
    };

    let mut cleanup_failures = Vec::new();
    if let Err(error) = node.shutdown().await
        && !matches!(error, auki_p2p::Error::SwarmStopped)
    {
        cleanup_failures.push(format!("node cleanup failed: {error}"));
    }
    if let Err(error) = client.delete(booking_id).await
        && error.http_code() != Some(RelayErrorCode::NotFound)
    {
        cleanup_failures.push(format!("booking cleanup failed: {error}"));
    }
    authority.stop().await;

    if let Some(reason) = root_failure {
        if cleanup_failures.is_empty() {
            SupervisorExit::Failed { reason }
        } else {
            SupervisorExit::Failed {
                reason: format!("{reason}; {}", cleanup_failures.join("; ")),
            }
        }
    } else if cleanup_failures.is_empty() {
        success
    } else {
        SupervisorExit::Failed {
            reason: format!("{context}; {}", cleanup_failures.join("; ")),
        }
    }
}

fn chrono_duration(duration: Duration) -> chrono::Duration {
    chrono::Duration::from_std(duration).unwrap_or_else(|_| chrono::Duration::seconds(1))
}

#[derive(Debug, thiserror::Error)]
enum SupervisorError {
    #[error("browser authority ended: {0}")]
    Authority(#[from] AuthorityManagerError),
    #[error("DMS relay operation failed: {0}")]
    Relay(#[from] RelayBookingClientError),
    #[error("DMS relay snapshot failed validation: {0}")]
    Selection(#[from] ReadyRelayError),
    #[error("active DMS relay booking disappeared")]
    BookingDisappeared,
    #[error("browser relay authority or provider lease reached its safety deadline")]
    RelayAuthorityEnded,
    #[error("DMS changed or withdrew the browser relay assignment; restart the Peer")]
    RelayChanged,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test(async)]
    async fn lifecycle_clones_share_one_terminal_result() {
        let (stopped_sender, stopped_receiver) = oneshot::channel();
        let lifecycle = AukiPeerLifecycle {
            stopped: stopped_receiver.shared(),
        };
        let clone = lifecycle.clone();
        stopped_sender
            .send(SupervisorExit::Failed {
                reason: "relay ended".to_owned(),
            })
            .expect("terminal status receiver remains alive");

        let expected = AukiPeerExit::SupervisorFailed {
            reason: "relay ended".to_owned(),
        };
        assert_eq!(lifecycle.wait_stopped().await, expected);
        assert_eq!(clone.wait_stopped().await, expected);
    }
}
