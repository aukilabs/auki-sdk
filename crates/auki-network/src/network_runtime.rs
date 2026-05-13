//! Runtime that drives a libp2p swarm against a list of known peers.
//!
//! [`NetworkRuntime`] owns a [`Swarm`]`<`[`Behaviour`]`>` and a tokio
//! task internally. It tracks "known peers" (an in-process membership
//! list), auto-dials peers whose multiaddrs are known, reconnects on
//! disconnect with exponential backoff, accepts inbound substreams on
//! `/auki/stream/0.1.0` (handed off to the consumer's
//! `stream_provider`), and accepts inbound `/auki/join/0.0.1`
//! substreams (handed off to the owner via the `JoinEvent` channel
//! returned by [`Self::spawn`]). Consumers interact through the small
//! set of public methods; they don't drive the swarm event loop
//! themselves.
//!
//! ## Cluster trust boundary
//!
//! Connection-level: open by default — libp2p completes handshakes
//! with anyone. Per-protocol gates enforce cluster membership inside
//! their own handlers (the `/auki/stream/0.1.0` accept path filters
//! by `known_peers`; the `/auki/join/0.0.1` path intentionally does
//! NOT gate, since a non-member peer's first contact with a cluster
//! IS the join handshake). The libp2p `block_list` is reserved for
//! evicting misbehaving peers, not for routine membership
//! enforcement.
//!
//! ## Not the home for
//!
//! - Cluster membership semantics (who's in the cluster, who's the
//!   Manager, when peers join/leave). Those live one layer up
//!   (`auki-domain`'s `ClusterMembership` + Manager state machine).
//!   The runtime is the libp2p plumbing the upper layer steers.
//! - Successor tokens, election rules, gossip. Same — those are
//!   `auki-domain` concerns.

use crate::heartbeat_protocol::{
    HEARTBEAT_INTERVAL, HEARTBEAT_PROTOCOL, HEARTBEAT_TIMEOUT, Heartbeat, read_heartbeat,
    write_heartbeat,
};
use crate::join_protocol::{
    JOIN_PROTOCOL, JoinProtocolError, JoinRequest, JoinResponse, read_join_request,
    read_join_response, write_join_request, write_join_response,
};
use crate::{
    stream_protocol::STREAM_PROTOCOL,
    stream_runtime::{StreamProvider, handle_inbound_substream},
    swarm::{self, Behaviour, BehaviourEvent},
};
#[cfg(test)]
use crate::{PeerIdentity, swarm::{SwarmConfig, build_swarm}};
use futures::StreamExt;
use libp2p::{
    Multiaddr, PeerId, StreamProtocol, Swarm,
    swarm::SwarmEvent,
};
use libp2p_stream::Control;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

/// How long [`NetworkRuntime::shutdown`] gives in-flight inbound
/// substream tasks to flush their final `EndOfStream` before the swarm
/// tears down. 100 ms is comfortably more than the time required to
/// write a single small framed message over a healthy LAN substream.
/// On unclean exit (`Drop`, panic) the grace period is skipped —
/// consumer sees `ConnectionLost` instead of the typed reason.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(100);

/// Initial reconnect backoff. Doubled on each consecutive dial failure
/// or unexpected disconnect, up to [`MAX_BACKOFF`]. Reset on a
/// successful `ConnectionEstablished`.
pub const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Cap on the per-peer reconnect backoff.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Period at which the runtime checks pending reconnects.
pub const RECONNECT_TICK: Duration = Duration::from_millis(500);

/// One entry in the runtime's allow-list / auto-dial schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedPeer {
    /// libp2p peer-id of this peer.
    pub peer_id: PeerId,
    /// Dialable multiaddrs for this peer. Empty list = the runtime
    /// allows inbound connections from this peer but does not
    /// auto-dial them.
    pub multiaddrs: Vec<Multiaddr>,
}

/// Errors from [`NetworkRuntime::spawn`].
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    /// Constructor was called outside a tokio runtime context — the
    /// runtime needs a tokio handle to spawn its driver task.
    #[error("no current tokio runtime — call from within a tokio runtime context")]
    NoTokioRuntime,
}

/// Inbound `/auki/join/0.0.1` event surfaced by the runtime to its
/// owner via the channel returned from [`NetworkRuntime::spawn`].
///
/// The owner (typically `auki-domain`'s `ClusterManager`) reads the
/// request, decides admit-or-reject, and replies via `ack`. The
/// runtime's per-substream task awaits the reply for up to
/// [`JOIN_RESPONSE_TIMEOUT`] before giving up.
#[derive(Debug)]
pub struct JoinEvent {
    /// The peer-id of the requester. Authenticated by libp2p's noise
    /// handshake at connection-establishment time.
    pub peer: PeerId,
    /// The body of the request.
    pub request: JoinRequest,
    /// One-shot channel to reply on. Dropping it without sending is
    /// equivalent to a timeout from the requester's perspective.
    pub ack: oneshot::Sender<JoinResponse>,
}

/// How long the runtime's per-substream join task waits for the
/// owner to reply via the `JoinEvent::ack` channel before closing
/// the substream. Generous because the owner may need to do I/O
/// (e.g. write to disk in a future Manager-state-machine variant).
const JOIN_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the consumer-side [`NetworkRuntime::send_join_request`]
/// waits for the producer's response before returning a timeout
/// error.
pub const JOIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Inbound liveness event surfaced by the runtime to its owner via
/// the channel returned from [`NetworkRuntime::spawn`].
///
/// The runtime maintains a per-peer "last heartbeat" timestamp and
/// fires `Lost { peer_id }` when that timestamp falls more than
/// [`HEARTBEAT_TIMEOUT`] in the past, or when the underlying libp2p
/// connection closes.
///
/// `Connected { peer_id }` fires once on every fresh
/// `ConnectionEstablished` for a known peer, after the runtime has
/// started its heartbeat exchange with them. Owners can use this as
/// the signal to consider a peer "live" and (e.g.) snapshot
/// `cluster_joined_at_ns` for the local `ParticipantInfo`.
#[derive(Debug)]
pub enum PeerLivenessEvent {
    /// A peer connected and the heartbeat exchange started.
    Connected {
        /// The peer-id of the peer.
        peer_id: PeerId,
    },
    /// A peer is no longer reachable — either the libp2p connection
    /// closed or no heartbeat has been received within
    /// [`HEARTBEAT_TIMEOUT`].
    Lost {
        /// The peer-id of the peer.
        peer_id: PeerId,
    },
}

/// Errors from [`NetworkRuntime::send_join_request`].
#[derive(Debug, thiserror::Error)]
pub enum SendJoinRequestError {
    /// `libp2p_stream::Control::open_stream` failed (peer not
    /// reachable, no allow-list entry, etc.).
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    /// I/O or wire-format error reading/writing the framed request
    /// or response.
    #[error("protocol: {0}")]
    Protocol(#[source] JoinProtocolError),
    /// The full request/response round-trip didn't complete within
    /// [`JOIN_REQUEST_TIMEOUT`].
    #[error("join request timed out after {0:?}")]
    Timeout(Duration),
}

/// Diff applied by [`NetworkRuntime::set_allowed_peers`].
///
/// `added` lists peer-ids in the new list but not the old — the runtime
/// has scheduled them for dialing (if they carry addresses). `removed`
/// lists peer-ids in the old list but not the new — the runtime has
/// dropped their connections and removed them from the allow-list.
/// Peers in both keep their existing connection; their addresses are
/// refreshed for future redials.
#[derive(Debug, Clone)]
pub struct UpdateReport {
    /// Peer-ids newly added to the allow-list.
    pub added: Vec<PeerId>,
    /// Peer-ids removed from the allow-list (and disconnected).
    pub removed: Vec<PeerId>,
}

/// Errors from [`NetworkRuntime::set_allowed_peers`].
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// The runtime task isn't accepting commands — typically because
    /// the runtime has shut down or is shutting down.
    #[error("runtime shutting down")]
    RuntimeUnavailable,
}

/// Internal command from public methods to the driver task.
enum RuntimeCmd {
    SetAllowedPeers {
        new_peers: Vec<AllowedPeer>,
        ack: oneshot::Sender<Result<UpdateReport, UpdateError>>,
    },
}

/// Per-peer dial scheduling state.
struct PeerSchedule {
    next_dial_at: Option<Instant>,
    backoff: Duration,
}

/// Drives a libp2p swarm against the allow-list set, auto-dialing
/// peers with known addresses, accepting inbound substreams on
/// `/auki/stream/0.1.0` (handed off to the `stream_provider`),
/// reconnecting on disconnect with exponential backoff. See the
/// module-level docs for the design rationale.
pub struct NetworkRuntime {
    local_peer_id: PeerId,
    connected: Arc<Mutex<HashSet<PeerId>>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    /// Cloneable handle to the swarm's [`libp2p_stream::Behaviour`].
    /// Used by [`crate::stream_runtime`]'s `open_stream` (consumer
    /// side) to open outbound substreams on `/auki/stream/0.1.0`.
    stream_control: Control,
    /// Watch channel signalling per-substream inbound tasks to flush
    /// a final `EndOfStream { reason: ProducerShuttingDown }` and
    /// exit. Sent to by [`Self::shutdown`] before the swarm teardown
    /// signal.
    inbound_shutdown_tx: watch::Sender<bool>,
    /// Command channel from public methods to the driver task.
    command_tx: mpsc::Sender<RuntimeCmd>,
}

/// Cloneable handle to a [`NetworkRuntime`] for command-style
/// operations (`set_allowed_peers`, `connected_peers`). Lets
/// `auki-domain`'s background tasks call back into the runtime
/// without holding the `NetworkRuntime` itself.
#[derive(Clone)]
pub struct NetworkRuntimeHandle {
    command_tx: mpsc::Sender<RuntimeCmd>,
    connected: Arc<Mutex<HashSet<PeerId>>>,
}

impl NetworkRuntimeHandle {
    /// Same semantics as [`NetworkRuntime::set_allowed_peers`].
    pub async fn set_allowed_peers(
        &self,
        new_peers: Vec<AllowedPeer>,
    ) -> Result<UpdateReport, UpdateError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.command_tx
            .send(RuntimeCmd::SetAllowedPeers {
                new_peers,
                ack: ack_tx,
            })
            .await
            .map_err(|_| UpdateError::RuntimeUnavailable)?;
        ack_rx.await.map_err(|_| UpdateError::RuntimeUnavailable)?
    }

    /// Same semantics as [`NetworkRuntime::connected_peers`].
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.connected
            .lock()
            .expect("connected set mutex poisoned")
            .iter()
            .copied()
            .collect()
    }
}

impl NetworkRuntime {
    /// Cloneable [`Control`] handle for outbound stream opens.
    /// Internal — `stream_runtime::open_stream` uses it; external
    /// callers go through `open_stream` itself.
    pub(crate) fn stream_control(&self) -> &Control {
        &self.stream_control
    }
}

impl NetworkRuntime {
    /// Construct a runtime around `swarm`. The swarm's keypair (and
    /// therefore its `PeerId`) becomes the runtime's local identity.
    /// The swarm should already be listening on its configured
    /// addresses.
    ///
    /// `allowed_peers` is the initial cluster trust boundary — only
    /// these peer-ids will complete libp2p handshakes inbound or
    /// outbound. Peers with at least one multiaddr are scheduled for
    /// an immediate dial; address-less entries are accepted as trusted
    /// (the runtime will respond if they dial us) but not auto-dialed.
    ///
    /// Returns the runtime + a receiver for inbound
    /// `/auki/join/0.0.1` events + a receiver for peer-liveness
    /// events. Owners that don't care about either (e.g. tests) can
    /// drop the receivers; the runtime drops events with no
    /// receiver.
    pub fn spawn(
        swarm: Swarm<Behaviour>,
        allowed_peers: Vec<AllowedPeer>,
        stream_provider: StreamProvider,
    ) -> Result<
        (
            Self,
            mpsc::Receiver<JoinEvent>,
            mpsc::Receiver<PeerLivenessEvent>,
        ),
        SpawnError,
    > {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| SpawnError::NoTokioRuntime)?;
        let local_peer_id = *swarm.local_peer_id();
        let connected = Arc::new(Mutex::new(HashSet::new()));
        let outbound_control = swarm.behaviour().stream.new_control();
        let inbound_control = outbound_control.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (inbound_shutdown_tx, inbound_shutdown_rx) = watch::channel(false);
        let (command_tx, command_rx) = mpsc::channel::<RuntimeCmd>(16);
        let (join_events_tx, join_events_rx) = mpsc::channel::<JoinEvent>(16);
        let (liveness_tx, liveness_rx) = mpsc::channel::<PeerLivenessEvent>(64);
        let task = handle.spawn(run_task(
            swarm,
            allowed_peers,
            connected.clone(),
            stream_provider,
            inbound_control,
            inbound_shutdown_rx,
            shutdown_rx,
            command_rx,
            join_events_tx,
            liveness_tx,
        ));
        Ok((
            Self {
                local_peer_id,
                connected,
                shutdown_tx: Some(shutdown_tx),
                task: Some(task),
                stream_control: outbound_control,
                inbound_shutdown_tx,
                command_tx,
            },
            join_events_rx,
            liveness_rx,
        ))
    }

    /// Open an outbound `/auki/join/0.0.1` substream to `peer_id`,
    /// write the request, read the response. Returns once the
    /// full round-trip completes (or fails).
    ///
    /// The peer must be on the local allow-list (`set_allowed_peers`
    /// or the initial `allowed_peers` argument to `spawn`) — libp2p
    /// refuses the noise handshake otherwise. Bootstrap case (first
    /// peer of a cluster joining the Manager): the caller pre-allows
    /// the Manager's peer-id before calling this.
    pub async fn send_join_request(
        &self,
        peer_id: PeerId,
        request: JoinRequest,
    ) -> Result<JoinResponse, SendJoinRequestError> {
        let mut control = self.stream_control.clone();
        let proto = StreamProtocol::try_from_owned(JOIN_PROTOCOL.to_string())
            .expect("JOIN_PROTOCOL is a valid libp2p protocol id");

        let open_fut = control.open_stream(peer_id, proto);
        let mut substream = match tokio::time::timeout(JOIN_REQUEST_TIMEOUT, open_fut).await {
            Err(_) => return Err(SendJoinRequestError::Timeout(JOIN_REQUEST_TIMEOUT)),
            Ok(Err(e)) => return Err(SendJoinRequestError::OpenStream(e)),
            Ok(Ok(s)) => s,
        };

        write_join_request(&mut substream, &request)
            .await
            .map_err(SendJoinRequestError::Protocol)?;

        let response = match tokio::time::timeout(
            JOIN_REQUEST_TIMEOUT,
            read_join_response(&mut substream),
        )
        .await
        {
            Err(_) => return Err(SendJoinRequestError::Timeout(JOIN_REQUEST_TIMEOUT)),
            Ok(Err(e)) => return Err(SendJoinRequestError::Protocol(e)),
            Ok(Ok(r)) => r,
        };
        Ok(response)
    }

    /// The runtime's local libp2p peer-id (derived from the swarm's
    /// keypair).
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// Cloneable handle for command-style operations
    /// ([`set_allowed_peers`](NetworkRuntimeHandle::set_allowed_peers),
    /// [`connected_peers`](NetworkRuntimeHandle::connected_peers)).
    /// Background tasks call back into the runtime through this
    /// handle without holding the [`NetworkRuntime`] itself.
    pub fn handle(&self) -> NetworkRuntimeHandle {
        NetworkRuntimeHandle {
            command_tx: self.command_tx.clone(),
            connected: self.connected.clone(),
        }
    }

    /// Snapshot of currently-connected peers.
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.connected
            .lock()
            .expect("connected set mutex poisoned")
            .iter()
            .copied()
            .collect()
    }

    /// Replace the allow-list with `new_peers`. The runtime diffs:
    ///
    /// - peer-ids in `new_peers` but not the old list are added to
    ///   the libp2p allow-list and (if they carry addresses)
    ///   scheduled for dial
    /// - peer-ids in the old list but not `new_peers` are removed from
    ///   the allow-list and their existing connections dropped
    /// - peer-ids in both keep their existing connection; addresses
    ///   are refreshed in case the new list carries different ones
    ///
    /// Returns an [`UpdateReport`] describing the diff.
    pub async fn set_allowed_peers(
        &self,
        new_peers: Vec<AllowedPeer>,
    ) -> Result<UpdateReport, UpdateError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.command_tx
            .send(RuntimeCmd::SetAllowedPeers {
                new_peers,
                ack: ack_tx,
            })
            .await
            .map_err(|_| UpdateError::RuntimeUnavailable)?;
        ack_rx.await.map_err(|_| UpdateError::RuntimeUnavailable)?
    }

    /// Signal the driver task to shut down and abort it. Inbound
    /// substream tasks have [`SHUTDOWN_GRACE`] to flush their final
    /// typed `EndOfStream` before the swarm tears down. Unclean exit
    /// (`Drop` without `shutdown(self)`, panic) skips the grace —
    /// consumers see `ConnectionLost` instead of the typed reason.
    pub fn shutdown(mut self) {
        let _ = self.inbound_shutdown_tx.send(true);
        self.cleanup();
    }

    fn cleanup(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for NetworkRuntime {
    fn drop(&mut self) {
        self.cleanup();
    }
}

// ─── Driver task ───────────────────────────────────────────────────

async fn run_task(
    swarm: Swarm<Behaviour>,
    initial_peers: Vec<AllowedPeer>,
    connected: Arc<Mutex<HashSet<PeerId>>>,
    stream_provider: StreamProvider,
    mut inbound_control: Control,
    inbound_shutdown_rx: watch::Receiver<bool>,
    mut shutdown_rx: oneshot::Receiver<()>,
    mut command_rx: mpsc::Receiver<RuntimeCmd>,
    join_events_tx: mpsc::Sender<JoinEvent>,
    liveness_tx: mpsc::Sender<PeerLivenessEvent>,
) {
    let mut swarm = swarm;
    let local_peer_id = *swarm.local_peer_id();
    let mut known_peers: HashMap<PeerId, Vec<Multiaddr>> = initial_peers
        .iter()
        .map(|p| (p.peer_id, p.multiaddrs.clone()))
        .collect();

    // Initial dial schedule. Peers with at least one address are
    // dialed immediately on first tick; address-less entries are
    // honoured as trusted (we'll respond to them if they dial us) but
    // not auto-dialed.
    let mut schedules: HashMap<PeerId, PeerSchedule> = known_peers
        .iter()
        .filter(|(_, addrs)| !addrs.is_empty())
        .map(|(pid, _)| {
            (
                *pid,
                PeerSchedule {
                    next_dial_at: Some(Instant::now()),
                    backoff: INITIAL_BACKOFF,
                },
            )
        })
        .collect();

    let mut tick = tokio::time::interval(RECONNECT_TICK);

    // Per-peer last-heartbeat-received timestamp. Updated by both
    // inbound-side reader tasks (heartbeats received over their
    // substreams) and outbound-side reader tasks (same substream,
    // other direction). Read by the monitor task that fires
    // `PeerLivenessEvent::Lost` when a peer's timestamp falls more
    // than `HEARTBEAT_TIMEOUT` in the past.
    let last_heartbeat_at: Arc<Mutex<HashMap<PeerId, Instant>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // Active per-peer heartbeat tasks, keyed by peer-id. Aborted on
    // disconnect (and re-spawned on the next ConnectionEstablished).
    let mut heartbeat_tasks: HashMap<PeerId, JoinHandle<()>> = HashMap::new();

    // Spawn the heartbeat-monitor task. Scans `last_heartbeat_at`
    // periodically and fires `Lost` events for peers whose
    // timestamps have expired. Aborted on runtime shutdown via the
    // owning JoinHandle (we never explicitly cancel it — when the
    // runtime task returns, its JoinHandle drops and the monitor
    // task drops with it).
    let _monitor_task = tokio::spawn(monitor_peer_liveness(
        last_heartbeat_at.clone(),
        liveness_tx.clone(),
    ));

    // Register inbound `/auki/stream/0.1.0` substream acceptance.
    let stream_proto = StreamProtocol::try_from_owned(STREAM_PROTOCOL.to_string())
        .expect("STREAM_PROTOCOL is a valid libp2p stream protocol id");
    let mut incoming_streams: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(stream_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    // Register inbound `/auki/join/0.0.1` substream acceptance.
    let join_proto = StreamProtocol::try_from_owned(JOIN_PROTOCOL.to_string())
        .expect("JOIN_PROTOCOL is a valid libp2p protocol id");
    let mut incoming_joins: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(join_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    // Register inbound `/auki/heartbeat/0.0.1` substream acceptance.
    let heartbeat_proto = StreamProtocol::try_from_owned(HEARTBEAT_PROTOCOL.to_string())
        .expect("HEARTBEAT_PROTOCOL is a valid libp2p protocol id");
    let mut incoming_heartbeats: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(heartbeat_proto) {
        Ok(s) => s.boxed(),
        Err(_already_registered) => futures::stream::pending().boxed(),
    };

    loop {
        tokio::select! {
            biased;

            _ = &mut shutdown_rx => {
                tokio::time::sleep(SHUTDOWN_GRACE).await;
                return;
            }

            event = swarm.next() => {
                let Some(event) = event else { return; };
                handle_event(
                    event,
                    local_peer_id,
                    &mut swarm,
                    &known_peers,
                    &mut schedules,
                    &connected,
                    &inbound_control,
                    &last_heartbeat_at,
                    &mut heartbeat_tasks,
                    &liveness_tx,
                );
            }

            inbound = incoming_streams.next() => {
                let Some((peer, substream)) = inbound else { return; };
                // Inbound stream-protocol substreams from peers not on
                // the allow-list are impossible (the libp2p allow-list
                // refuses them at handshake time), but defensive
                // double-check on the substream side belt-and-braces
                // the trust boundary.
                if !known_peers.contains_key(&peer) {
                    drop(substream);
                    continue;
                }
                let provider = stream_provider.clone();
                let task_shutdown = inbound_shutdown_rx.clone();
                tokio::spawn(handle_inbound_substream(
                    peer,
                    substream,
                    provider,
                    task_shutdown,
                ));
            }

            join = incoming_joins.next() => {
                let Some((peer, substream)) = join else { return; };
                // Inbound join substreams come from peers on the
                // allow-list (libp2p enforces). The Manager-side
                // owner decides whether to admit; the runtime just
                // plumbs the request through and ferries the response
                // back.
                let tx = join_events_tx.clone();
                tokio::spawn(handle_inbound_join_substream(peer, substream, tx));
            }

            heartbeat = incoming_heartbeats.next() => {
                let Some((peer, substream)) = heartbeat else { return; };
                // Inbound side of the heartbeat substream — only
                // accept if we have the HIGHER peer-id (the opener
                // is the lower one, by the convention below).
                // Defensive: reject otherwise so we don't end up
                // with two heartbeat substreams per pair.
                if local_peer_id < peer {
                    drop(substream);
                    continue;
                }
                // Cancel any previous task and start a fresh one.
                if let Some(prev) = heartbeat_tasks.remove(&peer) {
                    prev.abort();
                }
                let task = tokio::spawn(run_heartbeat_pair(
                    peer,
                    substream,
                    last_heartbeat_at.clone(),
                    liveness_tx.clone(),
                ));
                heartbeat_tasks.insert(peer, task);
            }

            _ = tick.tick() => {
                drive_pending_dials(&mut swarm, &known_peers, &mut schedules);
            }

            cmd = command_rx.recv() => {
                let Some(cmd) = cmd else { continue; };
                handle_command(cmd, &mut swarm, &mut known_peers, &mut schedules, &connected);
            }
        }
    }
}

fn handle_event(
    event: SwarmEvent<BehaviourEvent>,
    local_peer_id: PeerId,
    _swarm: &mut Swarm<Behaviour>,
    known_peers: &HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
    stream_control: &Control,
    last_heartbeat_at: &Arc<Mutex<HashMap<PeerId, Instant>>>,
    heartbeat_tasks: &mut HashMap<PeerId, JoinHandle<()>>,
    liveness_tx: &mpsc::Sender<PeerLivenessEvent>,
) {
    match event {
        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            if known_peers.contains_key(&peer_id) {
                if let Some(sched) = schedules.get_mut(&peer_id) {
                    sched.next_dial_at = None;
                    sched.backoff = INITIAL_BACKOFF;
                }
                connected
                    .lock()
                    .expect("connected set mutex poisoned")
                    .insert(peer_id);

                // Initialise last-heartbeat-at so the monitor task
                // gives this peer the full timeout window before
                // declaring them lost.
                last_heartbeat_at
                    .lock()
                    .expect("last_heartbeat_at lock")
                    .insert(peer_id, Instant::now());

                // Surface the Connected event to the owner.
                let _ = liveness_tx.try_send(PeerLivenessEvent::Connected { peer_id });

                // Lower peer-id initiates the heartbeat substream.
                // The other side accepts via `incoming_heartbeats`.
                if local_peer_id < peer_id {
                    if let Some(prev) = heartbeat_tasks.remove(&peer_id) {
                        prev.abort();
                    }
                    let control = stream_control.clone();
                    let last = last_heartbeat_at.clone();
                    let liveness = liveness_tx.clone();
                    let task = tokio::spawn(async move {
                        open_and_run_heartbeat_pair(peer_id, control, last, liveness).await;
                    });
                    heartbeat_tasks.insert(peer_id, task);
                }
            }
        }
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            connected
                .lock()
                .expect("connected set mutex poisoned")
                .remove(&peer_id);
            if let Some(task) = heartbeat_tasks.remove(&peer_id) {
                task.abort();
            }
            last_heartbeat_at
                .lock()
                .expect("last_heartbeat_at lock")
                .remove(&peer_id);
            if known_peers.contains_key(&peer_id) {
                let _ = liveness_tx.try_send(PeerLivenessEvent::Lost { peer_id });
                schedule_retry(schedules, peer_id);
            }
        }
        SwarmEvent::OutgoingConnectionError {
            peer_id: Some(peer_id),
            ..
        } => {
            if known_peers.contains_key(&peer_id) {
                schedule_retry(schedules, peer_id);
            }
        }
        _ => {}
    }
}

fn handle_command(
    cmd: RuntimeCmd,
    swarm: &mut Swarm<Behaviour>,
    known_peers: &mut HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
) {
    match cmd {
        RuntimeCmd::SetAllowedPeers { new_peers, ack } => {
            let report = apply_peer_update(swarm, known_peers, schedules, connected, new_peers);
            let _ = ack.send(Ok(report));
        }
    }
}

fn apply_peer_update(
    swarm: &mut Swarm<Behaviour>,
    known_peers: &mut HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
    new_peers: Vec<AllowedPeer>,
) -> UpdateReport {
    let new_set: HashSet<PeerId> = new_peers.iter().map(|p| p.peer_id).collect();

    // Removed peers: drop connection + disallow + clear schedule.
    let removed: Vec<PeerId> = known_peers
        .keys()
        .copied()
        .filter(|pid| !new_set.contains(pid))
        .collect();
    for pid in &removed {
        let _ = swarm.disconnect_peer_id(*pid);
        schedules.remove(pid);
        known_peers.remove(pid);
        connected
            .lock()
            .expect("connected set mutex poisoned")
            .remove(pid);
    }

    // Added peers: allow + (if addresses are present) schedule dial.
    let now = Instant::now();
    let added: Vec<PeerId> = new_peers
        .iter()
        .map(|p| p.peer_id)
        .filter(|pid| !known_peers.contains_key(pid))
        .collect();
    for ap in &new_peers {
        if added.contains(&ap.peer_id) {
            let has_addrs = !ap.multiaddrs.is_empty();
            known_peers.insert(ap.peer_id, ap.multiaddrs.clone());
            if has_addrs {
                schedules.insert(
                    ap.peer_id,
                    PeerSchedule {
                        next_dial_at: Some(now),
                        backoff: INITIAL_BACKOFF,
                    },
                );
            }
        } else {
            // Refresh addresses for existing peers.
            known_peers.insert(ap.peer_id, ap.multiaddrs.clone());
        }
    }
    let _ = &swarm;

    UpdateReport { added, removed }
}

fn drive_pending_dials(
    swarm: &mut Swarm<Behaviour>,
    known_peers: &HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
) {
    let now = Instant::now();
    let due: Vec<PeerId> = schedules
        .iter()
        .filter_map(|(pid, sched)| sched.next_dial_at.filter(|t| *t <= now).map(|_| *pid))
        .collect();
    for pid in due {
        if let Some(sched) = schedules.get_mut(&pid) {
            sched.next_dial_at = None;
        }
        if swarm.is_connected(&pid) {
            continue;
        }
        if let Some(addrs) = known_peers.get(&pid) {
            let _ = swarm::dial_peer(swarm, pid, addrs.clone());
        }
    }
}

/// Per-substream task for an inbound `/auki/join/0.0.1` request.
///
/// Reads the framed [`JoinRequest`], forwards it to the runtime's
/// owner via a [`JoinEvent`] on the channel, awaits the owner's
/// reply (up to [`JOIN_RESPONSE_TIMEOUT`]), writes the framed
/// [`JoinResponse`] back, closes the substream.
///
/// Errors at any stage are logged to stderr and drop the substream
/// silently — peers retry by opening a fresh substream. (The
/// alternative — surfacing per-substream errors back through the
/// channel — would require the owner to track every in-flight
/// request and provide its own timeout; not worth it.)
async fn handle_inbound_join_substream(
    peer: PeerId,
    mut substream: libp2p::Stream,
    join_events_tx: mpsc::Sender<JoinEvent>,
) {
    let request = match read_join_request(&mut substream).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("auki-network: join substream from {peer}: read request failed: {e}");
            return;
        }
    };

    let (ack_tx, ack_rx) = oneshot::channel();
    if join_events_tx
        .send(JoinEvent {
            peer,
            request,
            ack: ack_tx,
        })
        .await
        .is_err()
    {
        // Owner has dropped the receiver — no one's listening. Drop
        // the substream silently; the requester sees a closed
        // connection.
        return;
    }

    let response = match tokio::time::timeout(JOIN_RESPONSE_TIMEOUT, ack_rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => {
            // Sender dropped without sending — treat as a reject
            // with a generic reason so the requester gets some
            // signal rather than a silent connection drop.
            JoinResponse::Reject {
                reason: "join handler dropped without replying".into(),
            }
        }
        Err(_) => JoinResponse::Reject {
            reason: format!("join handler timed out after {JOIN_RESPONSE_TIMEOUT:?}"),
        },
    };

    if let Err(e) = write_join_response(&mut substream, &response).await {
        eprintln!("auki-network: join substream to {peer}: write response failed: {e}");
    }
}

/// Open an outbound `/auki/heartbeat/0.0.1` substream to `peer` and
/// run the bidirectional heartbeat loop on it. Caller is the
/// lower-peer-id side (the opener); the accepter calls
/// [`run_heartbeat_pair`] directly with an inbound substream.
///
/// Exits on substream close / fatal I/O error. On exit, emits a
/// `Lost { peer_id }` event so the owner knows the connection is no
/// longer carrying a liveness signal. The runtime's own
/// `ConnectionClosed` handling also emits `Lost`; the duplicate is
/// fine — owners can dedupe by tracking already-lost peer-ids.
async fn open_and_run_heartbeat_pair(
    peer: PeerId,
    mut control: Control,
    last_heartbeat_at: Arc<Mutex<HashMap<PeerId, Instant>>>,
    liveness_tx: mpsc::Sender<PeerLivenessEvent>,
) {
    let proto = StreamProtocol::try_from_owned(HEARTBEAT_PROTOCOL.to_string())
        .expect("HEARTBEAT_PROTOCOL is a valid libp2p protocol id");
    // Retry the open a few times — the peer may not have completed
    // their inbound `accept` registration in the moment between
    // ConnectionEstablished firing and this task starting.
    let mut substream = None;
    for _ in 0..5 {
        match control.open_stream(peer, proto.clone()).await {
            Ok(s) => {
                substream = Some(s);
                break;
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    let Some(substream) = substream else {
        // Opening failed; surface as Lost so the owner doesn't wait
        // for a heartbeat that will never come.
        let _ = liveness_tx.try_send(PeerLivenessEvent::Lost { peer_id: peer });
        return;
    };
    run_heartbeat_pair(peer, substream, last_heartbeat_at, liveness_tx).await;
}

/// Run the bidirectional heartbeat loop on `substream`. Writes a
/// `Heartbeat` every [`HEARTBEAT_INTERVAL`]; reads continuously and
/// updates the per-peer last-heartbeat timestamp on each frame
/// received. Returns when either side of the substream errors.
async fn run_heartbeat_pair(
    peer: PeerId,
    substream: libp2p::Stream,
    last_heartbeat_at: Arc<Mutex<HashMap<PeerId, Instant>>>,
    liveness_tx: mpsc::Sender<PeerLivenessEvent>,
) {
    use futures::AsyncReadExt as _;
    // The substream is bidirectional; split it so we can read on
    // one half and write on the other concurrently.
    let (mut reader, mut writer) = substream.split();

    let mut write_tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    // First tick fires immediately — send a heartbeat right away so
    // the peer sees the liveness signal without waiting an interval.
    loop {
        tokio::select! {
            biased;

            _ = write_tick.tick() => {
                let now_ns = unix_now_ns();
                if write_heartbeat(&mut writer, &Heartbeat { sent_at_unix_ns: now_ns }).await.is_err() {
                    break;
                }
            }

            r = read_heartbeat(&mut reader) => {
                match r {
                    Ok(_hb) => {
                        last_heartbeat_at
                            .lock()
                            .expect("last_heartbeat_at lock")
                            .insert(peer, Instant::now());
                    }
                    Err(_) => break,
                }
            }
        }
    }
    let _ = liveness_tx.try_send(PeerLivenessEvent::Lost { peer_id: peer });
}

/// Background task that scans the per-peer last-heartbeat timestamps
/// every half-`HEARTBEAT_TIMEOUT` and fires `Lost` events for peers
/// whose timestamps have expired. Runs for the lifetime of the
/// `NetworkRuntime`; aborted via the JoinHandle when the runtime
/// task returns.
async fn monitor_peer_liveness(
    last_heartbeat_at: Arc<Mutex<HashMap<PeerId, Instant>>>,
    liveness_tx: mpsc::Sender<PeerLivenessEvent>,
) {
    let mut tick = tokio::time::interval(HEARTBEAT_TIMEOUT / 2);
    // Dedupe: only emit Lost once per peer per disconnection. A
    // re-Connected reinserts in `last_heartbeat_at`, which removes
    // the peer from `lost_already`.
    let mut lost_already: HashSet<PeerId> = HashSet::new();
    loop {
        tick.tick().await;
        let now = Instant::now();
        let mut to_emit: Vec<PeerId> = Vec::new();
        let mut still_known: HashSet<PeerId> = HashSet::new();
        {
            let map = last_heartbeat_at.lock().expect("last_heartbeat_at lock");
            for (pid, ts) in map.iter() {
                still_known.insert(*pid);
                if now.duration_since(*ts) > HEARTBEAT_TIMEOUT && !lost_already.contains(pid) {
                    to_emit.push(*pid);
                }
            }
        }
        // Reset dedupe for peers no longer in the map (reconnected
        // and removed by ConnectionClosed handler, or just absent).
        lost_already.retain(|pid| still_known.contains(pid));
        for pid in to_emit {
            lost_already.insert(pid);
            if liveness_tx
                .try_send(PeerLivenessEvent::Lost { peer_id: pid })
                .is_err()
            {
                // Channel full or receiver dropped; nothing useful
                // to do. Owner will eventually catch up or shut down.
            }
        }
    }
}

fn unix_now_ns() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn schedule_retry(schedules: &mut HashMap<PeerId, PeerSchedule>, peer_id: PeerId) {
    let sched = schedules.entry(peer_id).or_insert(PeerSchedule {
        next_dial_at: None,
        backoff: INITIAL_BACKOFF,
    });
    sched.next_dial_at = Some(Instant::now() + sched.backoff);
    sched.backoff = std::cmp::min(sched.backoff.mul_f32(2.0), MAX_BACKOFF);
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_runtime::decline_all_streams;

    async fn build_test_swarm() -> Swarm<Behaviour> {
        let identity = PeerIdentity::from_seed(&[7u8; 32]);
        let cfg = SwarmConfig {
            listen_addresses: vec![
                "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
            ],
            ..SwarmConfig::default()
        };
        build_swarm(&identity, cfg).expect("build_swarm succeeds")
    }

    #[tokio::test]
    async fn spawn_with_empty_allow_list_starts_invisible() {
        let swarm = build_test_swarm().await;
        let (rt, _join_events, _liveness) = NetworkRuntime::spawn(swarm, vec![], decline_all_streams())
            .expect("spawn succeeds");
        assert!(rt.connected_peers().is_empty());
        rt.shutdown();
    }

    #[tokio::test]
    async fn local_peer_id_matches_swarm_identity() {
        let identity = PeerIdentity::from_seed(&[42u8; 32]);
        let cfg = SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            ..SwarmConfig::default()
        };
        let swarm = build_swarm(&identity, cfg).expect("build_swarm succeeds");
        let expected = identity.peer_id();
        let (rt, _join_events, _liveness) = NetworkRuntime::spawn(swarm, vec![], decline_all_streams())
            .expect("spawn succeeds");
        assert_eq!(rt.local_peer_id(), expected);
        rt.shutdown();
    }

    #[tokio::test]
    async fn set_allowed_peers_diff_reports_added_and_removed() {
        let swarm = build_test_swarm().await;
        let pid_a = PeerIdentity::from_seed(&[1u8; 32]).peer_id();
        let pid_b = PeerIdentity::from_seed(&[2u8; 32]).peer_id();
        let pid_c = PeerIdentity::from_seed(&[3u8; 32]).peer_id();

        let (rt, _join_events, _liveness) = NetworkRuntime::spawn(
            swarm,
            vec![
                AllowedPeer { peer_id: pid_a, multiaddrs: vec![] },
                AllowedPeer { peer_id: pid_b, multiaddrs: vec![] },
            ],
            decline_all_streams(),
        )
        .expect("spawn succeeds");

        // Swap b → c, keep a.
        let report = rt
            .set_allowed_peers(vec![
                AllowedPeer { peer_id: pid_a, multiaddrs: vec![] },
                AllowedPeer { peer_id: pid_c, multiaddrs: vec![] },
            ])
            .await
            .expect("set_allowed_peers succeeds");

        assert_eq!(report.added, vec![pid_c]);
        assert_eq!(report.removed, vec![pid_b]);
        rt.shutdown();
    }
}
