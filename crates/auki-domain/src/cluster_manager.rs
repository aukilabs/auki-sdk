//! [`ClusterManager`] — the SDK-side handle for a cluster a daemon is
//! participating in.
//!
//! Owns the cluster membership document, the libp2p `NetworkRuntime`
//! that drives the swarm, and a heartbeat-to-Discovery task. Daemons
//! (BoosterApp, Park, Sentinel) construct one via [`create_cluster`]
//! (and eventually `join_cluster` once the libp2p join protocol
//! lands) and treat it as a single owned object — its public methods
//! cover everything daemons need to surface to operators
//! (`is_manager`, `manager_peer_id`, `participant_info`,
//! `membership`).
//!
//! ## Manager-role state
//!
//! When `create_cluster` succeeds, the local peer is the cluster's
//! initial Manager. [`Self::is_manager`] is `true`; [`Self::manager_peer_id`]
//! equals the local peer-id. Later, when the join protocol + successor
//! election land, a non-Manager peer holds a `ClusterManager` whose
//! `is_manager` is `false` and whose `manager_peer_id` points at
//! whoever the cluster currently agrees is the Manager.
//!
//! ## Discovery heartbeat
//!
//! While this peer is the Manager, a background task pings Discovery
//! every 3 seconds with the cluster's `peer_count`. Discovery's sweep
//! drops clusters that haven't heartbeated in 10s, so this keeps the
//! directory entry live. The task is cancelled on
//! [`Self::shutdown`].

use crate::cluster_membership::{ClusterMember, ClusterMembership};
use auki_network::ParticipantInfo;
use auki_network::discovery_client::{
    CreateClusterOutcome, DiscoveryClient, DiscoveryError,
};
use auki_network::join_protocol::{JoinRequest, JoinResponse};
use auki_network::network_runtime::{
    AllowedPeer, JoinEvent, MembershipEvent, NetworkRuntime, PeerLivenessEvent,
    SendJoinRequestError, SpawnError,
};
use auki_network::stream_runtime::StreamProvider;
use auki_network::swarm::Behaviour;
use auki_network::{PeerIdentity, Swarm};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Cadence of the Manager → Discovery heartbeat tick. Matches the
/// Hagall v1 contract — Discovery's sweep timer is 10s, so a 3s
/// cadence leaves ~3 consecutive misses' tolerance before the
/// cluster is dropped.
pub const MANAGER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3);

/// Daemon-side identity fields the SDK doesn't own. Passed by the
/// daemon into [`ClusterManager::participant_info`] alongside the
/// cluster-aware fields the SDK fills in.
#[derive(Debug, Clone)]
pub struct DaemonInfo {
    /// Application identifier (`"boosterapp"`, `"sentinel"`, `"park"`).
    pub app: String,
    /// Operator-friendly per-device label.
    pub name: String,
    /// UUIDv4 minted at session boot.
    pub session_id: String,
    /// Identifier of the session's monotonic clock in the clock
    /// registry.
    pub session_clock_id: String,
    /// Content-addressed hash of the clock-registry entry.
    pub session_clock_hash: String,
    /// Session-clock value at the moment of capture.
    pub session_now_ns: u64,
    /// Session-clock value at first cluster connection. `None` if
    /// the daemon hasn't connected to a cluster peer yet.
    pub cluster_joined_at_ns: Option<u64>,
    /// First non-loopback IEEE-administered MAC, lowercased hex
    /// without separators.
    pub app_instance: String,
}

/// Errors from [`ClusterManager::create_cluster`].
#[derive(Debug, Error)]
pub enum CreateClusterError {
    /// Discovery rejected the create call — typically because the
    /// cluster name was already taken (in which case the caller
    /// should `list_clusters` and join the existing cluster instead),
    /// or because of an HTTP transport / status error.
    #[error("Discovery: {0}")]
    Discovery(#[from] DiscoveryError),
    /// Discovery's atomic create returned 409 — the cluster already
    /// exists. The caller should list and join instead.
    #[error("cluster {0:?} already exists; list and join instead")]
    AlreadyExists(String),
    /// `NetworkRuntime::spawn` failed — usually because the caller
    /// isn't inside a tokio runtime.
    #[error("runtime spawn failed: {0}")]
    Runtime(#[from] SpawnError),
}

/// Errors from [`ClusterManager::admit_peer`].
#[derive(Debug, Error)]
pub enum AdmitError {
    /// The local peer is not the Manager. Only the Manager admits
    /// new peers; other peers route join requests to the Manager.
    #[error("not the Manager of cluster {cluster:?}; manager_peer_id={manager}")]
    NotManager {
        /// Cluster name this Manager is responsible for.
        cluster: String,
        /// The peer-id of the actual Manager.
        manager: PeerId,
    },
    /// The peer is already a member of this cluster.
    #[error("peer {0} is already a cluster member")]
    AlreadyMember(PeerId),
    /// The runtime's `set_allowed_peers` call failed (typically
    /// because the runtime is shutting down).
    #[error("runtime: {0}")]
    Runtime(#[from] auki_network::network_runtime::UpdateError),
}

/// Errors from [`ClusterManager::join_cluster`].
#[derive(Debug, Error)]
pub enum JoinClusterError {
    /// Discovery rejected the list / lookup call.
    #[error("Discovery: {0}")]
    Discovery(#[from] DiscoveryError),
    /// Discovery's directory doesn't contain a cluster with this
    /// name. The caller should `create_cluster` if they want to be
    /// the first.
    #[error("cluster {0:?} not found in Discovery directory")]
    NotFound(String),
    /// The Manager's `/auki/join/0.0.1` substream open / read / write
    /// failed, or the Manager hung up before responding.
    #[error("join request: {0}")]
    SendJoin(#[from] SendJoinRequestError),
    /// The Manager refused the join.
    #[error("Manager rejected join: {0}")]
    Rejected(String),
    /// The Manager's response carried a malformed
    /// `membership_json` payload.
    #[error("invalid membership JSON from Manager: {0}")]
    InvalidMembership(#[source] serde_json::Error),
    /// `NetworkRuntime::spawn` failed.
    #[error("runtime spawn failed: {0}")]
    Runtime(#[from] SpawnError),
}

/// SDK-side handle for a cluster a daemon is participating in. See
/// the module-level docs.
pub struct ClusterManager {
    cluster_name: String,
    local_peer_id: PeerId,
    membership: Arc<Mutex<ClusterMembership>>,
    /// Canonical peer-id of whoever the cluster currently agrees is
    /// the Manager. Equals `local_peer_id` when this peer is the
    /// Manager; pointed at someone else otherwise. Mutated by the
    /// liveness handler when an election promotes the local peer.
    manager_peer_id: Arc<Mutex<PeerId>>,
    runtime: NetworkRuntime,
    discovery: DiscoveryClient,
    local_multiaddrs: Vec<Multiaddr>,
    /// Manager-side Discovery heartbeat task. Wrapped in
    /// `Arc<Mutex<Option<_>>>` so the liveness handler can spawn it
    /// on Manager-promotion (SDK-T7 handoff). `Some` while this peer
    /// is the Manager; `None` otherwise. Cancelled on `shutdown`.
    heartbeat_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Task that drains inbound `/auki/join/0.0.1` events from the
    /// runtime, decides admit-or-reject, and replies. Lives for the
    /// lifetime of the ClusterManager. Cancelled on `shutdown`.
    join_handler_task: Option<JoinHandle<()>>,
    /// Task that drains `PeerLivenessEvent`s from the runtime,
    /// runs the cluster-internal election on Manager death, and
    /// orchestrates Manager-handoff when the local peer wins.
    /// Cancelled on `shutdown`.
    liveness_handler_task: Option<JoinHandle<()>>,
    /// Task that drains inbound `/auki/membership/0.0.1` gossip
    /// events from the runtime, parses the membership JSON, swaps
    /// the local membership document, and pushes the updated
    /// allow-list to the runtime. Cancelled on `shutdown`.
    membership_handler_task: Option<JoinHandle<()>>,
}

impl ClusterManager {
    /// Create a new cluster and become its initial Manager. Atomic
    /// against concurrent `create_cluster` calls — only one peer
    /// wins; the loser gets [`CreateClusterError::AlreadyExists`]
    /// and should `list` + `join` instead.
    ///
    /// Sequence:
    /// 1. Call `discovery.create_cluster(...)` with the local peer
    ///    as the initial Manager.
    /// 2. Initialize the membership document with the local peer as
    ///    its only entry (`join_ts_ns` = `now_ns()`, opaque empty
    ///    successor token for v1).
    /// 3. Spawn the `NetworkRuntime` with an empty allow-list (no
    ///    cluster members yet besides ourselves; we don't dial
    ///    ourselves) and the daemon's `stream_provider`.
    /// 4. Spawn the Manager-side Discovery heartbeat tick.
    /// 5. Return the `ClusterManager`.
    pub async fn create_cluster(
        cluster_name: impl Into<String>,
        local_identity: PeerIdentity,
        local_multiaddrs: Vec<Multiaddr>,
        discovery: DiscoveryClient,
        swarm: Swarm<Behaviour>,
        stream_provider: StreamProvider,
    ) -> Result<Self, CreateClusterError> {
        let cluster_name = cluster_name.into();
        let local_peer_id = local_identity.peer_id();

        // 1. Atomic create on Discovery.
        match discovery
            .create_cluster(&cluster_name, &local_peer_id, &local_multiaddrs)
            .await?
        {
            CreateClusterOutcome::Created(_entry) => { /* proceed */ }
            CreateClusterOutcome::AlreadyExists => {
                return Err(CreateClusterError::AlreadyExists(cluster_name));
            }
        }

        // 2. Initialize the membership with the local peer.
        let mut membership = ClusterMembership::new(cluster_name.clone());
        let now_ns = now_unix_nanos();
        membership.admit(ClusterMember {
            peer_id: local_peer_id,
            multiaddrs: local_multiaddrs.clone(),
            join_ts_ns: now_ns,
            // v1: empty successor token (signature verification
            // disabled per Discovery v1 contract).
            successor_token: Some(Vec::new()),
        });
        let membership = Arc::new(Mutex::new(membership));

        // 3. Spawn the runtime. Initial allow-list is empty — we
        //    don't dial ourselves; the runtime expands its
        //    allow-list as peers are admitted.
        let (runtime, join_events_rx, liveness_rx, membership_events_rx) =
            NetworkRuntime::spawn(swarm, vec![], stream_provider)?;

        // 4. Manager-side Discovery heartbeat tick.
        let heartbeat_task: Arc<Mutex<Option<JoinHandle<()>>>> =
            Arc::new(Mutex::new(Some(spawn_manager_heartbeat(
                discovery.clone(),
                cluster_name.clone(),
                membership.clone(),
            ))));

        let manager_peer_id = Arc::new(Mutex::new(local_peer_id));

        // 5. Drain inbound `/auki/join/0.0.1` events.
        let join_handler_task = Some(spawn_join_handler(
            join_events_rx,
            cluster_name.clone(),
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
        ));

        // 6. Drain peer-liveness events: on Manager death, run the
        //    cluster-internal election; if we win, become the new
        //    Manager (update state, rotate Discovery, start the
        //    heartbeat tick).
        let liveness_handler_task = Some(spawn_liveness_handler(
            liveness_rx,
            cluster_name.clone(),
            local_peer_id,
            local_multiaddrs.clone(),
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
            discovery.clone(),
            heartbeat_task.clone(),
        ));

        // 7. Drain inbound /auki/membership/0.0.1 gossip events. As
        //    the freshly-minted Manager we don't expect to receive
        //    any (nobody else is gossiping yet), but if a stale peer
        //    sends one we apply it last-write-wins — the next
        //    Manager broadcast supersedes.
        let membership_handler_task = Some(spawn_membership_handler(
            membership_events_rx,
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
        ));

        Ok(Self {
            cluster_name,
            local_peer_id,
            membership,
            manager_peer_id,
            runtime,
            discovery,
            local_multiaddrs,
            heartbeat_task,
            join_handler_task,
            liveness_handler_task,
            membership_handler_task,
        })
    }

    /// The cluster's name.
    pub fn cluster_name(&self) -> &str {
        &self.cluster_name
    }

    /// The local peer's libp2p peer-id.
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// The local peer's dialable multiaddrs (the ones Discovery is
    /// pointed at).
    pub fn local_multiaddrs(&self) -> &[Multiaddr] {
        &self.local_multiaddrs
    }

    /// `true` if the local peer is currently the cluster's Manager.
    pub fn is_manager(&self) -> bool {
        *self.manager_peer_id.lock().expect("manager_peer_id lock") == self.local_peer_id
    }

    /// Canonical peer-id of whoever the cluster currently agrees is
    /// the Manager. May be the local peer.
    pub fn manager_peer_id(&self) -> PeerId {
        *self.manager_peer_id.lock().expect("manager_peer_id lock")
    }

    /// Snapshot of the cluster's membership document. Clones; safe to
    /// call from any thread.
    pub fn membership(&self) -> ClusterMembership {
        self.membership.lock().expect("membership lock").clone()
    }

    /// Number of cluster members. Aggregate (matches the
    /// `peer_count` Discovery records in its heartbeat snapshot).
    pub fn peer_count(&self) -> usize {
        self.membership.lock().expect("membership lock").peers.len()
    }

    /// Admit a new peer to the cluster. Manager-only — other peers
    /// route join requests to the Manager.
    ///
    /// On success the membership document gains a new
    /// [`ClusterMember`] entry, the runtime's allow-list is
    /// extended, and the new entry is returned. In v1 the successor
    /// token is an empty byte vec — signature verification is
    /// disabled per the Discovery v1 contract, so the bytes don't
    /// need to mean anything yet. SDK-T4 (when it lands) replaces
    /// this with a signed token.
    pub async fn admit_peer(
        &self,
        peer_id: PeerId,
        multiaddrs: Vec<Multiaddr>,
    ) -> Result<ClusterMember, AdmitError> {
        // Manager check.
        let manager = self.manager_peer_id();
        if manager != self.local_peer_id {
            return Err(AdmitError::NotManager {
                cluster: self.cluster_name.clone(),
                manager,
            });
        }

        // Build the new member entry, then take the lock briefly to
        // append. Releasing the lock before the (async) runtime
        // call avoids holding it across .await.
        let member = ClusterMember {
            peer_id,
            multiaddrs: multiaddrs.clone(),
            join_ts_ns: now_unix_nanos(),
            successor_token: Some(Vec::new()),
        };

        let new_allow_list: Vec<AllowedPeer> = {
            let mut membership = self.membership.lock().expect("membership lock");
            if membership.peers.iter().any(|p| p.peer_id == peer_id) {
                return Err(AdmitError::AlreadyMember(peer_id));
            }
            membership.admit(member.clone());
            // Build the allow-list from membership, excluding our
            // own peer-id (the runtime doesn't dial itself).
            membership
                .peers
                .iter()
                .filter(|p| p.peer_id != self.local_peer_id)
                .map(|p| AllowedPeer {
                    peer_id: p.peer_id,
                    multiaddrs: p.multiaddrs.clone(),
                })
                .collect()
        };

        // Push the updated allow-list to the runtime.
        self.runtime.set_allowed_peers(new_allow_list).await?;

        // Gossip the updated membership to every connected peer so
        // existing members learn about the new joiner (the joiner
        // itself already has the same JSON in the `JoinResponse::Accept`
        // it just received). Fire-and-forget; per-peer errors are
        // logged inside the broadcast tasks.
        broadcast_current_membership(&self.runtime.handle(), &self.membership);

        Ok(member)
    }

    /// Open an outbound stream subscription on `peer_id` for the
    /// named sensor. Thin delegator over
    /// [`NetworkRuntime::open_stream`] — the cluster handle is the
    /// daemon's natural entry point and shouldn't force consumers to
    /// reach into the runtime directly.
    ///
    /// Returns once the producer has either Accepted (typed
    /// [`StreamSubscription<T>`]) or Declined
    /// ([`OpenStreamError::Declined { reason }`]) the request. The
    /// peer must be a member of the cluster (checked by the
    /// runtime's allow-list on the producer side per the
    /// `/auki/stream/0.1.0` trust-boundary resolution 2026-05-13 —
    /// non-cluster substreams are silently dropped).
    ///
    /// `T` is the typed payload the substream carries (`JpegFrame`,
    /// `PointCloudFrame`, `JointEncodersFrame`); the consumer
    /// statically knows which `T` to expect per call.
    pub async fn open_stream<T>(
        &self,
        peer_id: PeerId,
        request: auki_network::stream_protocol::StreamRequest,
    ) -> Result<auki_network::stream_runtime::StreamSubscription<T>, auki_network::stream_runtime::OpenStreamError>
    where
        T: prost::Message + Default + Send + 'static,
    {
        self.runtime.open_stream::<T>(peer_id, request).await
    }

    /// Construct a [`ParticipantInfo`] with the cluster-aware fields
    /// (`is_manager`, `manager_peer_id`, `peer_id`) populated by the
    /// SDK. The daemon supplies the rest of the fields via the
    /// [`DaemonInfo`] arg. Daemons serve this verbatim on their
    /// Control API's `GET /api/info`.
    pub fn participant_info(&self, daemon: DaemonInfo) -> ParticipantInfo {
        let manager_peer_id = self.manager_peer_id();
        ParticipantInfo {
            app: daemon.app,
            name: daemon.name,
            session_id: daemon.session_id,
            session_clock_id: daemon.session_clock_id,
            session_clock_hash: daemon.session_clock_hash,
            session_now_ns: daemon.session_now_ns,
            cluster_joined_at_ns: daemon.cluster_joined_at_ns,
            peer_id: self.local_peer_id,
            app_instance: daemon.app_instance,
            is_manager: manager_peer_id == self.local_peer_id,
            manager_peer_id: manager_peer_id.to_string(),
        }
    }

    /// Join an existing cluster by talking to its Manager. Lists
    /// Discovery, finds the entry for `cluster_name`, opens a
    /// libp2p `/auki/join/0.0.1` substream to the Manager, sends a
    /// `JoinRequest`, parses the Manager's `JoinResponse`, and
    /// returns a `ClusterManager` populated with the full membership
    /// the Manager gossiped.
    ///
    /// The local peer is NOT the Manager — `is_manager()` returns
    /// `false`, `manager_peer_id()` points at whichever peer is
    /// recorded in Discovery's directory at the time of the call.
    /// No Discovery-heartbeat task is spawned (only Managers
    /// heartbeat); future Manager-handoff machinery (SDK-T7) will
    /// elect a successor and spawn the heartbeat then.
    pub async fn join_cluster(
        cluster_name: impl Into<String>,
        local_identity: PeerIdentity,
        local_multiaddrs: Vec<Multiaddr>,
        discovery: DiscoveryClient,
        swarm: Swarm<Behaviour>,
        stream_provider: StreamProvider,
    ) -> Result<Self, JoinClusterError> {
        let cluster_name = cluster_name.into();
        let local_peer_id = local_identity.peer_id();

        // 1. Look up the cluster in Discovery's directory.
        let clusters = discovery.list_clusters().await?;
        let entry = clusters
            .into_iter()
            .find(|c| c.name == cluster_name)
            .ok_or_else(|| JoinClusterError::NotFound(cluster_name.clone()))?;
        let manager_peer = entry.manager_peer_id;
        let manager_multiaddrs = entry.manager_multiaddrs.clone();

        // 2. Spawn the runtime with the Manager pre-allowed (so we
        //    can dial it for the join handshake). The allow-list
        //    expands once the Manager gossips back the full
        //    membership.
        let (runtime, join_events_rx, liveness_rx, membership_events_rx) = NetworkRuntime::spawn(
            swarm,
            vec![AllowedPeer {
                peer_id: manager_peer,
                multiaddrs: manager_multiaddrs.clone(),
            }],
            stream_provider,
        )?;

        // 3. Wait until the runtime has dialed the Manager and the
        //    libp2p connection is established. The runtime's
        //    auto-dial tick runs every 500ms; the first tick fires
        //    immediately. Cap the wait so a misconfigured Manager
        //    address surfaces as a clear error within seconds rather
        //    than hanging.
        let connect_deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if runtime.connected_peers().contains(&manager_peer) {
                break;
            }
            if std::time::Instant::now() >= connect_deadline {
                return Err(JoinClusterError::SendJoin(
                    SendJoinRequestError::Timeout(Duration::from_secs(10)),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // 4. Open the join substream + send the request.
        let response = runtime
            .send_join_request(
                manager_peer,
                JoinRequest {
                    multiaddrs: local_multiaddrs.clone(),
                },
            )
            .await?;

        // 4. Parse the Manager's response.
        let (membership, _token) = match response {
            JoinResponse::Accept {
                membership_json,
                successor_token,
            } => {
                let parsed: ClusterMembership = serde_json::from_str(&membership_json)
                    .map_err(JoinClusterError::InvalidMembership)?;
                (parsed, successor_token)
            }
            JoinResponse::Reject { reason } => return Err(JoinClusterError::Rejected(reason)),
        };

        // 5. Expand the runtime allow-list to cover every peer in
        //    the Manager-gossiped membership (other than ourselves).
        let allow_list: Vec<AllowedPeer> = membership
            .peers
            .iter()
            .filter(|m| m.peer_id != local_peer_id)
            .map(|m| AllowedPeer {
                peer_id: m.peer_id,
                multiaddrs: m.multiaddrs.clone(),
            })
            .collect();
        runtime
            .set_allowed_peers(allow_list)
            .await
            .map_err(|e| JoinClusterError::Runtime(SpawnError::NoTokioRuntime).into_with(e))?;

        let membership = Arc::new(Mutex::new(membership));
        let manager_peer_id = Arc::new(Mutex::new(manager_peer));

        // 6. Drain inbound join events. As a non-Manager our handler
        //    always rejects with "not the manager"; once an election
        //    promotes us the same handler starts admitting (it reads
        //    `manager_peer_id` per call).
        let join_handler_task = Some(spawn_join_handler(
            join_events_rx,
            cluster_name.clone(),
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
        ));

        // 7. Drain peer-liveness events: on Manager death, run the
        //    cluster-internal election; if we win, become the new
        //    Manager (update state, rotate Discovery, start the
        //    heartbeat tick).
        let heartbeat_task: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::new(Mutex::new(None));
        let liveness_handler_task = Some(spawn_liveness_handler(
            liveness_rx,
            cluster_name.clone(),
            local_peer_id,
            local_multiaddrs.clone(),
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
            discovery.clone(),
            heartbeat_task.clone(),
        ));

        // 8. Drain inbound /auki/membership/0.0.1 gossip events.
        //    The Manager pushes updates here when peers join / leave
        //    after our own join; we apply them last-write-wins.
        let membership_handler_task = Some(spawn_membership_handler(
            membership_events_rx,
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
        ));

        Ok(Self {
            cluster_name,
            local_peer_id,
            membership,
            manager_peer_id,
            runtime,
            discovery,
            local_multiaddrs,
            heartbeat_task,
            join_handler_task,
            liveness_handler_task,
            membership_handler_task,
        })
    }

    /// Shutdown — cancels all background tasks, deregisters the
    /// cluster from Discovery (if we're the Manager), and shuts
    /// down the runtime. Consumes `self`.
    pub async fn shutdown(mut self) -> Result<(), DiscoveryError> {
        // 1. Cancel background tasks FIRST so we stop touching
        //    Discovery / membership between teardown steps.
        if let Some(task) = self.heartbeat_task.lock().expect("heartbeat lock").take() {
            task.abort();
        }
        if let Some(task) = self.join_handler_task.take() {
            task.abort();
        }
        if let Some(task) = self.liveness_handler_task.take() {
            task.abort();
        }
        if let Some(task) = self.membership_handler_task.take() {
            task.abort();
        }

        // 2. If we're the Manager, deregister the cluster.
        let was_manager =
            *self.manager_peer_id.lock().expect("manager_peer_id lock") == self.local_peer_id;
        let result = if was_manager {
            self.discovery.deregister(&self.cluster_name).await
        } else {
            Ok(())
        };

        // 3. Shut down the runtime regardless of Discovery's result.
        self.runtime.shutdown();
        result
    }
}

// Lightweight wrapper for chaining a runtime error into JoinClusterError.
// The compiler doesn't auto-coerce UpdateError -> SpawnError -> JoinClusterError;
// we adapt explicitly.
impl JoinClusterError {
    fn into_with(self, _e: auki_network::network_runtime::UpdateError) -> JoinClusterError {
        // Keeping the original SpawnError context for now — UpdateError
        // happens at runtime-shutdown boundaries, so the join-side caller
        // sees Runtime(_) generically. v2 may split this into a typed
        // sub-variant if needed.
        self
    }
}

/// Drain peer-liveness events from the runtime; act on Manager
/// death (run the cluster-internal election, and if local wins,
/// orchestrate the Manager-handoff).
///
/// **Election rule** (per Hagall T4 / SDK-T6): sort cluster members
/// by `(join_ts_ns, peer_id)` ascending. The earliest-joined
/// reachable peer wins. If the local peer is that earliest-joined
/// reachable peer, it becomes the new Manager.
///
/// **Handoff** (per Hagall T7 / SDK-T7): the winner (a) updates
/// `manager_peer_id` locally so subsequent calls see itself as the
/// Manager, (b) calls Discovery's `rotate_manager` to update the
/// directory hint, (c) spawns the Manager-side Discovery
/// heartbeat tick.
///
/// **Reachability** is approximated as "in the runtime's
/// `connected_peers()` set OR equal to local_peer_id." When the
/// Manager dies, the local peer is always "reachable to itself"; the
/// other reachable peers are those still libp2p-connected via the
/// runtime. The earliest peer with a join_ts_ns less than the local
/// peer's own that's also reachable wins; if none such exists, the
/// local peer wins.
#[allow(clippy::too_many_arguments)]
fn spawn_liveness_handler(
    mut rx: mpsc::Receiver<PeerLivenessEvent>,
    cluster_name: String,
    local_peer_id: PeerId,
    local_multiaddrs: Vec<Multiaddr>,
    manager_peer_id: Arc<Mutex<PeerId>>,
    membership: Arc<Mutex<ClusterMembership>>,
    runtime: auki_network::NetworkRuntimeHandle,
    discovery: DiscoveryClient,
    heartbeat_task: Arc<Mutex<Option<JoinHandle<()>>>>,
) -> JoinHandle<()> {
    // Dedupe: don't run election twice for the same Lost event
    // (the runtime emits Lost from both `ConnectionClosed` and the
    // heartbeat-timeout monitor; the dedupe is per-peer-id).
    let acted_on_lost: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    tokio::spawn(async move {
        while let Some(evt) = rx.recv().await {
            match evt {
                PeerLivenessEvent::Connected { .. } => { /* informational */ }
                PeerLivenessEvent::Lost { peer_id: lost_pid } => {
                    let current_manager =
                        *manager_peer_id.lock().expect("manager_peer_id lock");
                    let am_manager = current_manager == local_peer_id;

                    if !am_manager && lost_pid == current_manager {
                        // Manager died. Run the election.
                        if acted_on_lost.swap(true, Ordering::SeqCst) {
                            // Already running / ran.
                            continue;
                        }
                        // For "reachable peers", give the connection
                        // teardown a brief moment so connected_peers()
                        // reflects the disconnection.
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        let connected = runtime.connected_peers();
                        let membership_snapshot =
                            membership.lock().expect("membership lock").clone();
                        let winner = elect_successor(
                            &membership_snapshot,
                            local_peer_id,
                            &connected,
                        );
                        if winner == Some(local_peer_id) {
                            // Become Manager.
                            *manager_peer_id.lock().expect("manager_peer_id lock") =
                                local_peer_id;

                            // Tell Discovery about the rotation.
                            if let Err(e) = discovery
                                .rotate_manager(
                                    &cluster_name,
                                    &local_peer_id,
                                    &local_multiaddrs,
                                )
                                .await
                            {
                                eprintln!(
                                    "auki-domain: rotate_manager failed for cluster \
                                    {cluster_name:?}: {e}"
                                );
                            }

                            // Start the Manager-side heartbeat tick.
                            let new_tick = spawn_manager_heartbeat(
                                discovery.clone(),
                                cluster_name.clone(),
                                membership.clone(),
                            );
                            let prev = heartbeat_task
                                .lock()
                                .expect("heartbeat_task lock")
                                .replace(new_tick);
                            if let Some(p) = prev {
                                p.abort();
                            }

                            // Evict the dead Manager from membership +
                            // push the updated allow-list. (We won
                            // the election, so we own membership now.)
                            let new_allow_list = {
                                let mut m =
                                    membership.lock().expect("membership lock");
                                m.peers.retain(|p| p.peer_id != lost_pid);
                                m.peers
                                    .iter()
                                    .filter(|p| p.peer_id != local_peer_id)
                                    .map(|p| AllowedPeer {
                                        peer_id: p.peer_id,
                                        multiaddrs: p.multiaddrs.clone(),
                                    })
                                    .collect::<Vec<_>>()
                            };
                            if let Err(e) = runtime.set_allowed_peers(new_allow_list).await
                            {
                                eprintln!(
                                    "auki-domain: post-election set_allowed_peers \
                                    failed for {cluster_name:?}: {e}"
                                );
                            }
                            // Gossip the post-handoff view so survivors
                            // converge on the new Manager identity + the
                            // post-eviction membership.
                            broadcast_current_membership(&runtime, &membership);
                            eprintln!(
                                "auki-domain: cluster {cluster_name:?}: local peer \
                                {local_peer_id} promoted to Manager after \
                                detecting Lost {lost_pid}"
                            );
                        } else {
                            // Someone else (earlier-joined, still
                            // reachable) wins. Update the local view
                            // and wait for them to register with
                            // Discovery.
                            if let Some(new_manager) = winner {
                                *manager_peer_id.lock().expect("manager_peer_id lock") =
                                    new_manager;
                            }
                        }
                    } else if am_manager {
                        // We're the Manager and a peer died. Evict
                        // from membership + push the updated
                        // allow-list.
                        let new_allow_list = {
                            let mut m = membership.lock().expect("membership lock");
                            let before = m.peers.len();
                            m.peers.retain(|p| p.peer_id != lost_pid);
                            if m.peers.len() == before {
                                // Wasn't actually a member (or
                                // already evicted) — nothing to do.
                                continue;
                            }
                            m.peers
                                .iter()
                                .filter(|p| p.peer_id != local_peer_id)
                                .map(|p| AllowedPeer {
                                    peer_id: p.peer_id,
                                    multiaddrs: p.multiaddrs.clone(),
                                })
                                .collect::<Vec<_>>()
                        };
                        if let Err(e) = runtime.set_allowed_peers(new_allow_list).await {
                            eprintln!(
                                "auki-domain: Manager evict post-Lost set_allowed_peers \
                                failed: {e}"
                            );
                        }
                        // Gossip the shrunken membership so remaining
                        // peers also evict the dead one.
                        broadcast_current_membership(&runtime, &membership);
                    }
                }
            }
        }
    })
}

/// Spawn a task that drains inbound `/auki/membership/0.0.1` gossip
/// events from `rx`, applies each update last-write-wins to the
/// local `membership`, and pushes the recomputed allow-list to the
/// `runtime`. Lives for the lifetime of the `ClusterManager`;
/// cancelled on `shutdown`.
///
/// **Does NOT mutate `manager_peer_id`.** The election in
/// `spawn_liveness_handler` is the single source of truth for who
/// the Manager is — each peer runs the same deterministic algorithm
/// over the same membership and converges independently. A gossip
/// from a non-Manager (e.g. during a split-brain window) would lie
/// about the Manager identity; ignoring the sender peer-id avoids
/// that footgun.
fn spawn_membership_handler(
    mut rx: mpsc::Receiver<MembershipEvent>,
    local_peer_id: PeerId,
    _manager_peer_id: Arc<Mutex<PeerId>>,
    membership: Arc<Mutex<ClusterMembership>>,
    runtime: auki_network::NetworkRuntimeHandle,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(MembershipEvent { peer, update }) = rx.recv().await {
            let parsed: ClusterMembership = match serde_json::from_str(&update.membership_json) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!(
                        "auki-domain: membership gossip from {peer}: invalid JSON: {e}"
                    );
                    continue;
                }
            };
            // Last-write-wins: replace local membership and rebuild
            // the allow-list. The cluster-trust gate on the runtime
            // side already refused non-cluster senders, but any
            // cluster member can in principle send — `manager_peer_id`
            // intentionally stays unchanged so a non-Manager
            // gossiper can't claim the role.
            let new_allow_list: Vec<AllowedPeer> = {
                let mut m = membership.lock().expect("membership lock");
                *m = parsed;
                m.peers
                    .iter()
                    .filter(|p| p.peer_id != local_peer_id)
                    .map(|p| AllowedPeer {
                        peer_id: p.peer_id,
                        multiaddrs: p.multiaddrs.clone(),
                    })
                    .collect()
            };
            if let Err(e) = runtime.set_allowed_peers(new_allow_list).await {
                eprintln!(
                    "auki-domain: membership gossip apply: set_allowed_peers \
                    failed: {e}"
                );
            }
        }
    })
}

/// Serialize the current `membership` and `broadcast_membership` it
/// over `/auki/membership/0.0.1`. Logged-and-swallow on encode
/// failure; per-peer write failures are logged inside the runtime's
/// per-task spawns. The Manager calls this after admit, after
/// eviction, and on Manager-promotion.
fn broadcast_current_membership(
    runtime: &auki_network::NetworkRuntimeHandle,
    membership: &Arc<Mutex<ClusterMembership>>,
) {
    let json = {
        let m = membership.lock().expect("membership lock");
        match serde_json::to_string(&*m) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "auki-domain: serializing membership for gossip failed: {e}"
                );
                return;
            }
        }
    };
    if let Err(e) = runtime.broadcast_membership(json) {
        eprintln!("auki-domain: broadcast_membership failed: {e}");
    }
}

/// Cluster-internal election (SDK-T6). Deterministic: sort
/// membership by `(join_ts_ns, peer_id)` ascending; return the
/// earliest-joined peer that's "reachable" (in `connected` or equal
/// to `local_peer_id`). Returns `None` only if the membership is
/// empty (degenerate / shouldn't happen in practice).
///
/// This is a pure function for ease of testing.
pub fn elect_successor(
    membership: &ClusterMembership,
    local_peer_id: PeerId,
    connected: &[PeerId],
) -> Option<PeerId> {
    let mut sorted: Vec<&ClusterMember> = membership.peers.iter().collect();
    sorted.sort_by(|a, b| {
        a.join_ts_ns
            .cmp(&b.join_ts_ns)
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });
    for m in sorted {
        if m.peer_id == local_peer_id || connected.contains(&m.peer_id) {
            return Some(m.peer_id);
        }
    }
    None
}

fn spawn_manager_heartbeat(
    discovery: DiscoveryClient,
    cluster_name: String,
    membership: Arc<Mutex<ClusterMembership>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(MANAGER_HEARTBEAT_INTERVAL);
        // First tick fires immediately; skip it because Discovery
        // already has our state from `create_cluster`'s synchronous
        // `create` call. The next tick happens at +3s.
        tick.tick().await;
        loop {
            tick.tick().await;
            let peer_count = {
                let m = membership.lock().expect("membership lock");
                m.peers.len() as u32
            };
            // Errors are logged to stderr (via the default
            // tracing-or-print path) and tolerated — a transient
            // Discovery hiccup shouldn't kill the cluster. Discovery
            // sweeps after 10s of no heartbeat anyway, so persistent
            // failures self-resolve.
            if let Err(e) = discovery.heartbeat(&cluster_name, peer_count).await {
                eprintln!(
                    "auki-domain: Discovery heartbeat for cluster {cluster_name:?} failed: {e}"
                );
            }
        }
    })
}

/// Spawn a task that drains inbound join events from `rx` and
/// replies on each `ack`. Manager peers admit + push the updated
/// allow-list via the runtime handle; non-Manager peers reject
/// with `"not the manager"`. The task lives for the lifetime of
/// the `ClusterManager`; cancelled on `shutdown`.
fn spawn_join_handler(
    mut rx: mpsc::Receiver<JoinEvent>,
    cluster_name: String,
    local_peer_id: PeerId,
    manager_peer_id: Arc<Mutex<PeerId>>,
    membership: Arc<Mutex<ClusterMembership>>,
    runtime: auki_network::NetworkRuntimeHandle,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let JoinEvent {
                peer,
                request,
                ack,
            } = event;

            // Manager check.
            let am_manager =
                *manager_peer_id.lock().expect("manager_peer_id lock") == local_peer_id;
            if !am_manager {
                let _ = ack.send(JoinResponse::Reject {
                    reason: "not the manager".into(),
                });
                continue;
            }

            // Build the new member entry; check for duplicate
            // membership; append + build the new allow-list inside
            // a short lock window. The runtime call happens
            // afterwards (locks released, no holding across await).
            let (new_allow_list, member, full_membership_json) = {
                let mut m = membership.lock().expect("membership lock");
                if m.peers.iter().any(|p| p.peer_id == peer) {
                    drop(m);
                    let _ = ack.send(JoinResponse::Reject {
                        reason: "already a member".into(),
                    });
                    continue;
                }
                let member = ClusterMember {
                    peer_id: peer,
                    multiaddrs: request.multiaddrs.clone(),
                    join_ts_ns: now_unix_nanos(),
                    successor_token: Some(Vec::new()),
                };
                m.admit(member.clone());
                let allow_list: Vec<AllowedPeer> = m
                    .peers
                    .iter()
                    .filter(|p| p.peer_id != local_peer_id)
                    .map(|p| AllowedPeer {
                        peer_id: p.peer_id,
                        multiaddrs: p.multiaddrs.clone(),
                    })
                    .collect();
                let json = serde_json::to_string(&*m)
                    .expect("ClusterMembership serializes by construction");
                (allow_list, member, json)
            };

            if let Err(e) = runtime.set_allowed_peers(new_allow_list).await {
                eprintln!(
                    "auki-domain: join handler for cluster {cluster_name:?}: \
                    set_allowed_peers failed: {e}; sending Reject"
                );
                let _ = ack.send(JoinResponse::Reject {
                    reason: format!("runtime: {e}"),
                });
                continue;
            }

            let _ = ack.send(JoinResponse::Accept {
                membership_json: full_membership_json,
                successor_token: member.successor_token.unwrap_or_default(),
            });

            // Gossip the updated membership to every other connected
            // peer so existing members learn about the new joiner.
            // The new joiner itself just received the same JSON in
            // the JoinResponse::Accept above; the broadcast targets
            // everyone else.
            broadcast_current_membership(&runtime, &membership);
        }
    })
}

fn now_unix_nanos() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_info_is_cheap_to_clone() {
        let d = DaemonInfo {
            app: "x".into(),
            name: "y".into(),
            session_id: "z".into(),
            session_clock_id: "c".into(),
            session_clock_hash: "h".into(),
            session_now_ns: 0,
            cluster_joined_at_ns: None,
            app_instance: "abc".into(),
        };
        let _ = d.clone();
    }

    #[test]
    fn manager_heartbeat_interval_matches_v1_contract() {
        // 3s heartbeat / 10s sweep — matches aukilabs/discovery#5.
        assert_eq!(MANAGER_HEARTBEAT_INTERVAL, Duration::from_secs(3));
    }

    fn make_peer(seed: u8, join_ts: i64) -> ClusterMember {
        let id = auki_network::PeerIdentity::from_seed(&[seed; 32]);
        ClusterMember {
            peer_id: id.peer_id(),
            multiaddrs: vec![],
            join_ts_ns: join_ts,
            successor_token: None,
        }
    }

    #[test]
    fn election_earliest_joined_reachable_peer_wins() {
        // A joined first, B second, C third. All reachable. A wins.
        let m_a = make_peer(1, 100);
        let m_b = make_peer(2, 200);
        let m_c = make_peer(3, 300);
        let mut membership = ClusterMembership::new("foo");
        for m in [m_a.clone(), m_b.clone(), m_c.clone()] {
            membership.admit(m);
        }
        // local = B; all peers reachable.
        let winner = elect_successor(
            &membership,
            m_b.peer_id,
            &[m_a.peer_id, m_b.peer_id, m_c.peer_id],
        );
        assert_eq!(winner, Some(m_a.peer_id));
    }

    #[test]
    fn election_skips_unreachable_earlier_peers() {
        // A joined first but is unreachable; B is reachable and
        // joined second. B wins.
        let m_a = make_peer(1, 100);
        let m_b = make_peer(2, 200);
        let m_c = make_peer(3, 300);
        let mut membership = ClusterMembership::new("foo");
        for m in [m_a.clone(), m_b.clone(), m_c.clone()] {
            membership.admit(m);
        }
        // local = B; only B + C reachable. A is missing.
        let winner = elect_successor(&membership, m_b.peer_id, &[m_c.peer_id]);
        // B is "reachable to itself" and joined before C → B wins.
        assert_eq!(winner, Some(m_b.peer_id));
    }

    #[test]
    fn election_tie_breaks_on_lower_peer_id() {
        // Two peers with the same join_ts_ns; lower peer_id wins.
        let m_x = make_peer(1, 100);
        let m_y = make_peer(2, 100);
        let mut membership = ClusterMembership::new("foo");
        membership.admit(m_x.clone());
        membership.admit(m_y.clone());
        let lower = std::cmp::min(m_x.peer_id, m_y.peer_id);
        let local = m_x.peer_id;
        let winner = elect_successor(&membership, local, &[m_x.peer_id, m_y.peer_id]);
        assert_eq!(winner, Some(lower));
    }

    #[test]
    fn election_returns_local_when_alone() {
        // Only the local peer is reachable. Local wins (because
        // local is "reachable to itself").
        let m_a = make_peer(1, 100);
        let m_b = make_peer(2, 200);
        let mut membership = ClusterMembership::new("foo");
        membership.admit(m_a.clone());
        membership.admit(m_b.clone());
        // local = B; A unreachable.
        let winner = elect_successor(&membership, m_b.peer_id, &[]);
        assert_eq!(winner, Some(m_b.peer_id));
    }

    #[test]
    fn election_empty_membership_returns_none() {
        let membership = ClusterMembership::new("foo");
        let local = auki_network::PeerIdentity::from_seed(&[9u8; 32]).peer_id();
        assert_eq!(elect_successor(&membership, local, &[]), None);
    }
}
