//! Higher-level cluster orchestration on top of [`crate::cluster_protocol`] —
//! ansuz networking-demo deliverable #4.
//!
//! [`ClusterRuntime`] takes a [`ClusterDoc`] and a participant-info
//! provider and drives a libp2p swarm against them: auto-dials every
//! peer in the doc, exchanges [`ParticipantInfo`] over the
//! `/auki/cluster/1.0.0` protocol, maintains a live peer-state map,
//! and reconnects on disconnect with exponential backoff.
//!
//! ## Shape: opaque runtime, not a `NetworkBehaviour`
//!
//! The runtime owns its own [`Swarm`]`<`[`Behaviour`]`>` and tokio task
//! internally. Consumers interact through
//! [`peers`][ClusterRuntime::peers] /
//! [`shutdown`][ClusterRuntime::shutdown]; they don't drive the swarm
//! event loop themselves.
//!
//! Reasons for the opaque shape rather than a real `NetworkBehaviour`:
//! the actual consumer (Boosterapp's Python sidecar via the planned
//! `auki-py` `cluster.spawn`) cannot drive an async libp2p loop from
//! Python; it wants a thing it can ask "who's connected?" from the
//! HTTP request handler thread. Sentinel and other Rust consumers that
//! want fine control use [`crate::cluster_protocol::Behaviour`]
//! directly and skip this module.
//!
//! ## In scope
//!
//! - Auto-dial every peer in the doc that has at least one address.
//! - On a successful connection to a known peer, send a [`ClusterRequest`].
//! - On an inbound request from a known peer, invoke
//!   `participant_provider` and reply with the result.
//! - Track the live peer-state map; expose it via [`peers`][ClusterRuntime::peers].
//! - Reconnect on disconnect with exponential backoff
//!   ([`INITIAL_BACKOFF`] doubling up to [`MAX_BACKOFF`]).
//!
//! ## Not in scope
//!
//! - **Not a discovery service.** Peers in the doc are pre-pinned;
//!   there's no gossip, no DHT bootstrap.
//! - **Not a session manager.** The session-clock and `session_id`
//!   story belongs to the consumer; the runtime just plumbs whatever
//!   `participant_provider` returns.
//! - **Not the `cluster_joined_at_ns` setter.** That field on the
//!   *local* `ParticipantInfo` is the consumer's responsibility — they
//!   read [`peers`][ClusterRuntime::peers] to know whether any peer
//!   has connected and set the field accordingly.
//! - **Not a trust-extender.** Inbound connections from peers not in
//!   the doc are accepted at the libp2p layer (Noise authenticates)
//!   but the runtime doesn't issue requests to them, doesn't respond
//!   to their requests, and doesn't surface them in
//!   [`peers`][ClusterRuntime::peers]. The doc *is* the trust boundary.

use crate::{
    ParticipantInfo, PeerIdentity,
    cluster_doc::ClusterDoc,
    cluster_protocol::ClusterRequest,
    swarm::{self, Behaviour, BehaviourEvent, SwarmConfig, build_swarm},
};
use futures::StreamExt;
use libp2p::{
    Multiaddr, PeerId, Swarm, request_response,
    swarm::SwarmEvent,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{sync::oneshot, task::JoinHandle};

/// Initial reconnect backoff. Doubled on each consecutive dial failure
/// or unexpected disconnect, up to [`MAX_BACKOFF`]. Reset on a
/// successful `ConnectionEstablished`.
pub const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Cap on the per-peer reconnect backoff.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Period at which the runtime checks pending reconnects. Bounds the
/// dial latency between a peer disconnect and the runtime's next dial
/// attempt to roughly this duration.
pub const RECONNECT_TICK: Duration = Duration::from_millis(500);

/// Callable supplying the consumer's current [`ParticipantInfo`]. The
/// runtime invokes it on each inbound `ClusterRequest`, so
/// `session_now_ns` is fresh on each reply rather than stale at
/// runtime-spawn time. Must be `Send + Sync` because the runtime task
/// holds it in an `Arc` shared with the swarm.
pub type ParticipantInfoProvider = Arc<dyn Fn() -> ParticipantInfo + Send + Sync>;

/// Errors from [`ClusterRuntime::spawn`] / [`ClusterRuntime::from_swarm`].
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    /// Underlying [`build_swarm`] call failed (only `spawn`).
    #[error("swarm build failed: {0}")]
    BuildSwarm(#[from] swarm::BuildError),
    /// Constructor was called outside a tokio runtime context — the
    /// runtime needs a tokio handle to spawn its driver task.
    #[error("no current tokio runtime — call from within a tokio runtime context")]
    NoTokioRuntime,
}

/// Snapshot of one connected peer.
///
/// Returned by [`ClusterRuntime::peers`]. Read-only and `Clone` — the
/// runtime owns the live state internally; this is a copy taken at
/// snapshot time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSnapshot {
    /// libp2p peer id of this peer.
    pub peer_id: PeerId,
    /// Most recent [`ParticipantInfo`] received from this peer.
    /// Refreshed on every response.
    pub info: ParticipantInfo,
    /// Peer's `session_now_ns` value at the moment of the **first**
    /// response received from this peer's current session. Sticky
    /// across reconnects within the same peer-session; reset if the
    /// peer's `session_id` changes (peer restarted with a fresh
    /// session).
    pub first_seen_ns: u64,
}

/// Drives a libp2p swarm against a [`ClusterDoc`], auto-dialing peers,
/// exchanging [`ParticipantInfo`] over `/auki/cluster/1.0.0`, and
/// maintaining the live peer state.
///
/// See the module-level docs for the design rationale.
pub struct ClusterRuntime {
    state: Arc<Mutex<RuntimeState>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

struct RuntimeState {
    /// Peers we've seen at any point, keyed by peer id. `connected:
    /// false` entries are retained so `first_seen_ns` survives
    /// disconnect-reconnect within the same peer-session.
    peers: HashMap<PeerId, PeerEntry>,
}

struct PeerEntry {
    info: ParticipantInfo,
    first_seen_ns: u64,
    connected: bool,
}

impl ClusterRuntime {
    /// Build a swarm from `seed` + `swarm_config`, then drive it
    /// against `doc`. Convenience over [`Self::from_swarm`] —
    /// equivalent to constructing the swarm manually and handing it in.
    pub fn spawn(
        seed: [u8; 32],
        doc: ClusterDoc,
        swarm_config: SwarmConfig,
        participant_provider: ParticipantInfoProvider,
    ) -> Result<Self, SpawnError> {
        let identity = PeerIdentity::from_seed(&seed);
        let swarm = build_swarm(&identity, swarm_config)?;
        Self::from_swarm(swarm, doc, participant_provider)
    }

    /// Drive a pre-built swarm against `doc`. The swarm should already
    /// be listening on its configured addresses. Use this when the
    /// caller needs to learn the swarm's bound addresses *before*
    /// constructing the cluster doc — e.g. tests, or a daemon that
    /// publishes its addresses out-of-band before peers can dial it.
    pub fn from_swarm(
        swarm: Swarm<Behaviour>,
        doc: ClusterDoc,
        participant_provider: ParticipantInfoProvider,
    ) -> Result<Self, SpawnError> {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| SpawnError::NoTokioRuntime)?;
        let state = Arc::new(Mutex::new(RuntimeState {
            peers: HashMap::new(),
        }));
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = handle.spawn(run_task(
            swarm,
            doc,
            state.clone(),
            participant_provider,
            shutdown_rx,
        ));
        Ok(Self {
            state,
            shutdown_tx: Some(shutdown_tx),
            task: Some(task),
        })
    }

    /// Snapshot of currently-connected peers. Lock-light — copies entries
    /// out from under a brief mutex hold, then drops the lock before
    /// allocating the returned `Vec`. Safe to call from any thread,
    /// including non-tokio threads (an HTTP handler reading
    /// `/api/cluster`, for instance).
    pub fn peers(&self) -> Vec<PeerSnapshot> {
        let state = self.state.lock().expect("state mutex poisoned");
        state
            .peers
            .iter()
            .filter(|(_, entry)| entry.connected)
            .map(|(pid, entry)| PeerSnapshot {
                peer_id: *pid,
                info: entry.info.clone(),
                first_seen_ns: entry.first_seen_ns,
            })
            .collect()
    }

    /// Signal the driver task to shut down and abort it. The underlying
    /// swarm is dropped, closing all connections. Idempotent in
    /// practice — calling shutdown consumes `self` and the [`Drop`]
    /// impl on the unconsumed path runs the same cleanup.
    pub fn shutdown(mut self) {
        self.cleanup();
    }

    fn cleanup(&mut self) {
        // Best-effort signal — receiver may have already dropped if
        // the task is exiting.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        // Abort the task. If it has already exited gracefully via the
        // shutdown signal, this is a no-op. Otherwise it's a hard kill
        // which is fine — the swarm drops on task exit and connections
        // close cleanly at the TCP layer regardless.
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for ClusterRuntime {
    fn drop(&mut self) {
        self.cleanup();
    }
}

// ─── run_task ────────────────────────────────────────────────────────────────

/// Per-peer reconnect tracking. `next_dial_at: None` means "no pending
/// dial" — either we're connected or we've never tried.
struct PeerSchedule {
    next_dial_at: Option<Instant>,
    backoff: Duration,
}

async fn run_task(
    mut swarm: Swarm<Behaviour>,
    doc: ClusterDoc,
    state: Arc<Mutex<RuntimeState>>,
    participant_provider: ParticipantInfoProvider,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    // Membership is fixed at spawn time; capture as a HashMap for
    // O(1) lookups during the event loop.
    let known_peers: HashMap<PeerId, Vec<Multiaddr>> = doc
        .peers
        .iter()
        .map(|p| (p.peer_id, p.addresses.clone()))
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

    loop {
        tokio::select! {
            biased;

            _ = &mut shutdown_rx => {
                return;
            }

            event = swarm.next() => {
                let Some(event) = event else {
                    return;
                };
                handle_event(
                    event,
                    &mut swarm,
                    &known_peers,
                    &mut schedules,
                    &state,
                    &participant_provider,
                );
            }

            _ = tick.tick() => {
                drive_pending_dials(&mut swarm, &known_peers, &mut schedules);
            }
        }
    }
}

fn handle_event(
    event: SwarmEvent<BehaviourEvent>,
    swarm: &mut Swarm<Behaviour>,
    known_peers: &HashMap<PeerId, Vec<Multiaddr>>,
    schedules: &mut HashMap<PeerId, PeerSchedule>,
    state: &Arc<Mutex<RuntimeState>>,
    participant_provider: &ParticipantInfoProvider,
) {
    match event {
        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            if known_peers.contains_key(&peer_id) {
                // Reset backoff and clear any pending redial.
                if let Some(sched) = schedules.get_mut(&peer_id) {
                    sched.next_dial_at = None;
                    sched.backoff = INITIAL_BACKOFF;
                }
                // Send our request. Its arrival is the trigger that
                // populates the remote's view of us; their response is
                // what populates our view of them.
                swarm
                    .behaviour_mut()
                    .cluster
                    .send_request(&peer_id, ClusterRequest);
            }
        }
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            // Mark the peer as disconnected, preserving their entry so
            // first_seen_ns survives a same-session reconnect.
            {
                let mut state = state.lock().expect("state mutex poisoned");
                if let Some(entry) = state.peers.get_mut(&peer_id) {
                    entry.connected = false;
                }
            }
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
        SwarmEvent::Behaviour(BehaviourEvent::Cluster(
            request_response::Event::Message { peer, message, .. },
        )) => match message {
            request_response::Message::Request { channel, .. } => {
                if known_peers.contains_key(&peer) {
                    let info = (participant_provider)();
                    let _ = swarm
                        .behaviour_mut()
                        .cluster
                        .send_response(channel, info);
                }
                // Else: drop the channel. The peer-not-in-doc case sees
                // their request time out, which is the correct signal
                // — we don't share our identity with peers outside the
                // cluster.
            }
            request_response::Message::Response { response, .. } => {
                if known_peers.contains_key(&peer) {
                    store_response(state, peer, response);
                }
            }
        },
        _ => {}
    }
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
        // Already connected? Skip — the schedule will get cleared when
        // the existing connection closes, if it ever does.
        if swarm.is_connected(&pid) {
            continue;
        }
        if let Some(addrs) = known_peers.get(&pid) {
            // Failure here is silent — libp2p reports it as
            // OutgoingConnectionError on the next poll, which schedules
            // the next retry with backoff.
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

fn store_response(
    state: &Arc<Mutex<RuntimeState>>,
    peer_id: PeerId,
    response: ParticipantInfo,
) {
    let mut state = state.lock().expect("state mutex poisoned");
    match state.peers.get_mut(&peer_id) {
        // Same peer, same session — refresh info, keep first_seen_ns.
        Some(existing) if existing.info.session_id == response.session_id => {
            existing.info = response;
            existing.connected = true;
        }
        // Either a new peer or the same peer with a fresh session
        // (peer restarted) — replace the entry and reset first_seen_ns.
        _ => {
            let first_seen_ns = response.session_now_ns;
            state.peers.insert(
                peer_id,
                PeerEntry {
                    info: response,
                    first_seen_ns,
                    connected: true,
                },
            );
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster_doc::ClusterPeer;

    /// Build a swarm config that's safe for parallel tests: loopback,
    /// OS-chosen TCP port, mDNS off (no LAN noise / cross-test
    /// interference), relay-server off.
    fn test_swarm_config(agent_version: &str) -> SwarmConfig {
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: agent_version.into(),
            enable_mdns: false,
            enable_relay_server: false,
        }
    }

    /// Wait for a swarm's first `NewListenAddr` event and return the
    /// address.
    async fn wait_for_listen_addr(swarm: &mut Swarm<Behaviour>) -> Multiaddr {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                    return address;
                }
            }
        })
        .await
        .expect("listen addr did not appear within timeout")
    }

    /// Build a participant-info provider that returns a fixed
    /// `ParticipantInfo` (with a fresh `session_now_ns` per call so
    /// tests can verify the callable is invoked per request).
    fn fixture_provider(peer_id: PeerId, app: &str, name: &str) -> ParticipantInfoProvider {
        let app = app.to_string();
        let name = name.to_string();
        let session_id = format!("session-{}", name);
        let session_clock_id = format!("{}/clock", name);
        Arc::new(move || {
            let session_now_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            ParticipantInfo {
                app: app.clone(),
                name: name.clone(),
                session_id: session_id.clone(),
                session_clock_id: session_clock_id.clone(),
                session_clock_hash: "deadbeef".into(),
                session_now_ns,
                cluster_joined_at_ns: None,
                peer_id,
                app_instance: "00163eabcdef".into(),
            }
        })
    }

    /// Poll a closure until it returns `true` or the deadline expires.
    async fn poll_until<F: FnMut() -> bool>(mut cond: F, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if cond() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Helper: build a swarm, wait for it to listen, return swarm + addr.
    async fn build_listening_swarm(
        identity: &PeerIdentity,
        agent_version: &str,
    ) -> (Swarm<Behaviour>, Multiaddr) {
        let mut swarm = build_swarm(identity, test_swarm_config(agent_version)).unwrap();
        let addr = wait_for_listen_addr(&mut swarm).await;
        (swarm, addr)
    }

    fn cluster_peer(peer_id: PeerId, addr: Multiaddr) -> ClusterPeer {
        ClusterPeer {
            peer_id,
            addresses: vec![addr],
            expected_app_id: None,
            note: None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_runtimes_discover_each_other_via_cluster_doc() {
        let id_a = PeerIdentity::from_seed(&[31u8; 32]);
        let id_b = PeerIdentity::from_seed(&[32u8; 32]);

        let (swarm_a, addr_a) = build_listening_swarm(&id_a, "test-a/0").await;
        let (swarm_b, addr_b) = build_listening_swarm(&id_b, "test-b/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-2-peer".into(),
            peers: vec![
                cluster_peer(id_a.peer_id(), addr_a),
                cluster_peer(id_b.peer_id(), addr_b),
            ],
        };

        let rt_a = ClusterRuntime::from_swarm(
            swarm_a,
            doc.clone(),
            fixture_provider(id_a.peer_id(), "boosterapp", "robot-a"),
        )
        .expect("spawn rt_a");
        let rt_b = ClusterRuntime::from_swarm(
            swarm_b,
            doc.clone(),
            fixture_provider(id_b.peer_id(), "sentinel", "sentinel-b"),
        )
        .expect("spawn rt_b");

        let converged = poll_until(
            || rt_a.peers().len() == 1 && rt_b.peers().len() == 1,
            Duration::from_secs(10),
        )
        .await;
        assert!(
            converged,
            "two runtimes did not converge in time: rt_a sees {}, rt_b sees {}",
            rt_a.peers().len(),
            rt_b.peers().len()
        );

        // Each side should see the other, not itself.
        assert_eq!(rt_a.peers()[0].peer_id, id_b.peer_id());
        assert_eq!(rt_a.peers()[0].info.app, "sentinel");
        assert_eq!(rt_a.peers()[0].info.name, "sentinel-b");
        assert_eq!(rt_b.peers()[0].peer_id, id_a.peer_id());
        assert_eq!(rt_b.peers()[0].info.app, "boosterapp");
        assert_eq!(rt_b.peers()[0].info.name, "robot-a");

        // first_seen_ns is the peer's session_now_ns at first response.
        assert!(rt_a.peers()[0].first_seen_ns > 0);
        assert!(rt_b.peers()[0].first_seen_ns > 0);

        rt_a.shutdown();
        rt_b.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 6)]
    async fn three_runtimes_form_full_mesh() {
        let id_a = PeerIdentity::from_seed(&[41u8; 32]);
        let id_b = PeerIdentity::from_seed(&[42u8; 32]);
        let id_c = PeerIdentity::from_seed(&[43u8; 32]);

        let (swarm_a, addr_a) = build_listening_swarm(&id_a, "a/0").await;
        let (swarm_b, addr_b) = build_listening_swarm(&id_b, "b/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "c/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-3-peer".into(),
            peers: vec![
                cluster_peer(id_a.peer_id(), addr_a),
                cluster_peer(id_b.peer_id(), addr_b),
                cluster_peer(id_c.peer_id(), addr_c),
            ],
        };

        let rt_a = ClusterRuntime::from_swarm(
            swarm_a,
            doc.clone(),
            fixture_provider(id_a.peer_id(), "boosterapp", "a"),
        )
        .unwrap();
        let rt_b = ClusterRuntime::from_swarm(
            swarm_b,
            doc.clone(),
            fixture_provider(id_b.peer_id(), "sentinel", "b"),
        )
        .unwrap();
        let rt_c = ClusterRuntime::from_swarm(
            swarm_c,
            doc.clone(),
            fixture_provider(id_c.peer_id(), "park", "c"),
        )
        .unwrap();

        let converged = poll_until(
            || {
                rt_a.peers().len() == 2
                    && rt_b.peers().len() == 2
                    && rt_c.peers().len() == 2
            },
            Duration::from_secs(15),
        )
        .await;
        assert!(
            converged,
            "three runtimes did not converge: a={}, b={}, c={}",
            rt_a.peers().len(),
            rt_b.peers().len(),
            rt_c.peers().len()
        );

        rt_a.shutdown();
        rt_b.shutdown();
        rt_c.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 6)]
    async fn peer_leaving_drops_off_other_peers() {
        let id_a = PeerIdentity::from_seed(&[51u8; 32]);
        let id_b = PeerIdentity::from_seed(&[52u8; 32]);
        let id_c = PeerIdentity::from_seed(&[53u8; 32]);

        let (swarm_a, addr_a) = build_listening_swarm(&id_a, "a/0").await;
        let (swarm_b, addr_b) = build_listening_swarm(&id_b, "b/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "c/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-leave".into(),
            peers: vec![
                cluster_peer(id_a.peer_id(), addr_a),
                cluster_peer(id_b.peer_id(), addr_b),
                cluster_peer(id_c.peer_id(), addr_c),
            ],
        };

        let rt_a = ClusterRuntime::from_swarm(
            swarm_a,
            doc.clone(),
            fixture_provider(id_a.peer_id(), "boosterapp", "a"),
        )
        .unwrap();
        let rt_b = ClusterRuntime::from_swarm(
            swarm_b,
            doc.clone(),
            fixture_provider(id_b.peer_id(), "sentinel", "b"),
        )
        .unwrap();
        let rt_c = ClusterRuntime::from_swarm(
            swarm_c,
            doc.clone(),
            fixture_provider(id_c.peer_id(), "park", "c"),
        )
        .unwrap();

        // Wait for the full mesh.
        let converged = poll_until(
            || {
                rt_a.peers().len() == 2
                    && rt_b.peers().len() == 2
                    && rt_c.peers().len() == 2
            },
            Duration::from_secs(15),
        )
        .await;
        assert!(converged, "did not converge before leave test");

        // Kill rt_c. rt_a and rt_b should drop it within a few seconds.
        rt_c.shutdown();

        let dropped = poll_until(
            || {
                let a_sees_c = rt_a.peers().iter().any(|p| p.peer_id == id_c.peer_id());
                let b_sees_c = rt_b.peers().iter().any(|p| p.peer_id == id_c.peer_id());
                !a_sees_c && !b_sees_c
            },
            Duration::from_secs(10),
        )
        .await;
        assert!(
            dropped,
            "surviving runtimes still see departed peer: a={:?}, b={:?}",
            rt_a.peers()
                .iter()
                .map(|p| p.peer_id)
                .collect::<Vec<_>>(),
            rt_b.peers()
                .iter()
                .map(|p| p.peer_id)
                .collect::<Vec<_>>(),
        );

        // a and b still see each other.
        assert_eq!(rt_a.peers().len(), 1);
        assert_eq!(rt_b.peers().len(), 1);
        assert_eq!(rt_a.peers()[0].peer_id, id_b.peer_id());
        assert_eq!(rt_b.peers()[0].peer_id, id_a.peer_id());

        rt_a.shutdown();
        rt_b.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn unknown_peer_is_not_surfaced() {
        // rt has only itself in its doc. An outsider connects and sends
        // a cluster request; we expect rt to drop the request silently
        // and not surface the outsider in peers().
        let id_rt = PeerIdentity::from_seed(&[61u8; 32]);
        let id_outsider = PeerIdentity::from_seed(&[62u8; 32]);
        let pid_rt = id_rt.peer_id();

        let (swarm_rt, addr_rt) = build_listening_swarm(&id_rt, "rt/0").await;
        let (mut swarm_outsider, _addr_outsider) =
            build_listening_swarm(&id_outsider, "outsider/0").await;

        // Doc lists ONLY rt — outsider is not in it.
        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-trust-boundary".into(),
            peers: vec![cluster_peer(pid_rt, addr_rt.clone())],
        };

        // Outsider dials rt directly, bypassing the doc.
        swarm_outsider
            .dial(addr_rt.clone())
            .expect("outsider dials rt");

        // Drive the outsider's swarm in a background task so its
        // libp2p state machine progresses (identify, the cluster
        // request, etc.). The runtime owns swarm_rt.
        let outsider_handle = tokio::spawn(async move {
            // Wait for the connection to rt to come up, then send a
            // cluster request.
            let mut request_sent = false;
            loop {
                tokio::select! {
                    event = swarm_outsider.next() => {
                        let Some(event) = event else { return; };
                        if let SwarmEvent::ConnectionEstablished { peer_id, .. } = event {
                            if !request_sent && peer_id == pid_rt {
                                swarm_outsider
                                    .behaviour_mut()
                                    .cluster
                                    .send_request(&pid_rt, ClusterRequest);
                                request_sent = true;
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        // Yield to let other tasks make progress.
                    }
                }
            }
        });

        // Hand swarm_rt to the runtime so it starts driving its event
        // loop — at this point the outsider's TCP SYN may already have
        // been sent and is buffered for delivery as soon as we poll.
        let rt = ClusterRuntime::from_swarm(
            swarm_rt,
            doc,
            fixture_provider(pid_rt, "boosterapp", "rt"),
        )
        .unwrap();

        // Give the system a couple of seconds to settle. The runtime
        // should NOT surface the outsider — its request should be
        // dropped silently, and no entry should appear in peers().
        tokio::time::sleep(Duration::from_secs(2)).await;

        assert_eq!(
            rt.peers().len(),
            0,
            "runtime surfaced an out-of-doc peer: {:?}",
            rt.peers()
                .iter()
                .map(|p| p.peer_id)
                .collect::<Vec<_>>()
        );

        outsider_handle.abort();
        rt.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_is_idempotent_and_drops_state() {
        let id = PeerIdentity::from_seed(&[71u8; 32]);
        let (swarm, _addr) = build_listening_swarm(&id, "alone/0").await;
        let doc = ClusterDoc {
            version: 1,
            cluster_name: "alone".into(),
            peers: vec![],
        };
        let rt = ClusterRuntime::from_swarm(
            swarm,
            doc,
            fixture_provider(id.peer_id(), "test", "alone"),
        )
        .unwrap();

        assert_eq!(rt.peers().len(), 0);
        rt.shutdown();
        // shutdown consumes self; the test passes if it returns
        // promptly (no deadlock / hang) and the assertion above held.
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drop_without_explicit_shutdown_cleans_up() {
        // Same as above, but without calling shutdown — the Drop impl
        // should signal + abort the task. The test passes if dropping
        // the runtime returns promptly.
        let id = PeerIdentity::from_seed(&[72u8; 32]);
        let (swarm, _addr) = build_listening_swarm(&id, "drop/0").await;
        let doc = ClusterDoc {
            version: 1,
            cluster_name: "drop".into(),
            peers: vec![],
        };
        let _rt = ClusterRuntime::from_swarm(
            swarm,
            doc,
            fixture_provider(id.peer_id(), "test", "drop"),
        )
        .unwrap();
        // _rt drops here.
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_outside_tokio_runtime_returns_error() {
        // tokio::test puts us inside a runtime — to test the
        // no-runtime path we spawn a std thread, which has no current
        // tokio runtime, and try to call from there.
        let id = PeerIdentity::from_seed(&[73u8; 32]);
        let (swarm, _addr) = build_listening_swarm(&id, "no-rt/0").await;
        let doc = ClusterDoc {
            version: 1,
            cluster_name: "no-rt".into(),
            peers: vec![],
        };
        let provider = fixture_provider(id.peer_id(), "test", "no-rt");

        let result = std::thread::spawn(move || {
            ClusterRuntime::from_swarm(swarm, doc, provider).map(|_| ())
        })
        .join()
        .expect("std thread");

        assert!(matches!(result, Err(SpawnError::NoTokioRuntime)));
    }
}
