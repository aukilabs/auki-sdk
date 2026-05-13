//! Runtime that drives a libp2p swarm against a list of allowed peers.
//!
//! [`NetworkRuntime`] owns a [`Swarm`]`<`[`Behaviour`]`>` and a tokio
//! task internally. It maintains the libp2p allow-list (the cluster
//! trust boundary — only peers on the list complete handshakes),
//! auto-dials peers whose multiaddrs we know, reconnects on disconnect
//! with exponential backoff, and accepts inbound substreams on the
//! `/auki/stream/0.1.0` protocol (handed off to per-substream tasks
//! that invoke the consumer's `stream_provider`). Consumers interact
//! through the small set of public methods; they don't drive the swarm
//! event loop themselves.
//!
//! ## Cluster trust boundary
//!
//! `allow_list` is populated from `allowed_peers` on spawn and rewritten
//! by every [`set_allowed_peers`] call. Peers off the list are refused
//! at the libp2p `NetworkBehaviour` layer — inbound and outbound
//! connections from non-listed peer-ids never complete the noise
//! handshake. This is the SDK's primary cluster-trust-boundary
//! enforcement; nothing else in the SDK relaxes it.
//!
//! ## Not the home for
//!
//! - Cluster membership semantics (who's in the cluster, who's the
//!   Manager, when peers join/leave). Those live one layer up
//!   (`auki-domain`'s `ClusterMembership` + Manager state machine).
//!   The runtime is the libp2p plumbing the upper layer steers.
//! - Successor tokens, election rules, gossip. Same — those are
//!   `auki-domain` concerns.

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
    pub fn spawn(
        swarm: Swarm<Behaviour>,
        allowed_peers: Vec<AllowedPeer>,
        stream_provider: StreamProvider,
    ) -> Result<Self, SpawnError> {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| SpawnError::NoTokioRuntime)?;
        let local_peer_id = *swarm.local_peer_id();
        let connected = Arc::new(Mutex::new(HashSet::new()));
        let outbound_control = swarm.behaviour().stream.new_control();
        let inbound_control = outbound_control.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (inbound_shutdown_tx, inbound_shutdown_rx) = watch::channel(false);
        let (command_tx, command_rx) = mpsc::channel::<RuntimeCmd>(16);
        let task = handle.spawn(run_task(
            swarm,
            allowed_peers,
            connected.clone(),
            stream_provider,
            inbound_control,
            inbound_shutdown_rx,
            shutdown_rx,
            command_rx,
        ));
        Ok(Self {
            local_peer_id,
            connected,
            shutdown_tx: Some(shutdown_tx),
            task: Some(task),
            stream_control: outbound_control,
            inbound_shutdown_tx,
            command_tx,
        })
    }

    /// The runtime's local libp2p peer-id (derived from the swarm's
    /// keypair).
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
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
) {
    let mut swarm = swarm;
    let mut known_peers: HashMap<PeerId, Vec<Multiaddr>> = initial_peers
        .iter()
        .map(|p| (p.peer_id, p.multiaddrs.clone()))
        .collect();

    // Populate the allow-list BEFORE any inbound connection can
    // complete its noise handshake. Empty allow-list = swarm refuses
    // every handshake; an unpopulated runtime is invisible by design.
    for pid in known_peers.keys() {
        swarm.behaviour_mut().allow_list.allow_peer(*pid);
    }

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

    // Register inbound `/auki/stream/0.1.0` substream acceptance.
    let stream_proto = StreamProtocol::try_from_owned(STREAM_PROTOCOL.to_string())
        .expect("STREAM_PROTOCOL is a valid libp2p stream protocol id");
    let mut incoming_streams: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match inbound_control.accept(stream_proto) {
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
                handle_event(event, &mut swarm, &known_peers, &mut schedules, &connected);
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
    _swarm: &mut Swarm<Behaviour>,
    known_peers: &HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
    connected: &Arc<Mutex<HashSet<PeerId>>>,
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
            }
        }
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            connected
                .lock()
                .expect("connected set mutex poisoned")
                .remove(&peer_id);
            if known_peers.contains_key(&peer_id) {
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
        swarm.behaviour_mut().allow_list.disallow_peer(*pid);
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
            swarm.behaviour_mut().allow_list.allow_peer(ap.peer_id);
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
        let rt = NetworkRuntime::spawn(swarm, vec![], decline_all_streams())
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
        let rt = NetworkRuntime::spawn(swarm, vec![], decline_all_streams())
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

        let rt = NetworkRuntime::spawn(
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
