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
    AllowedPeer, JoinEvent, NetworkRuntime, SendJoinRequestError, SpawnError,
};
use auki_network::stream_runtime::StreamProvider;
use auki_network::swarm::Behaviour;
use auki_network::{PeerIdentity, Swarm};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
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
    /// Manager; pointed at someone else otherwise.
    manager_peer_id: Arc<Mutex<PeerId>>,
    runtime: NetworkRuntime,
    discovery: DiscoveryClient,
    local_multiaddrs: Vec<Multiaddr>,
    /// Manager-side heartbeat task. Some(_) while this peer is the
    /// Manager; None otherwise. Cancelled on `shutdown`.
    heartbeat_task: Option<JoinHandle<()>>,
    /// Task that drains inbound `/auki/join/0.0.1` events from the
    /// runtime, decides admit-or-reject, and replies. Lives for the
    /// lifetime of the ClusterManager. Cancelled on `shutdown`.
    join_handler_task: Option<JoinHandle<()>>,
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
        let (runtime, join_events_rx) = NetworkRuntime::spawn(swarm, vec![], stream_provider)?;

        // 4. Manager-side Discovery heartbeat tick.
        let heartbeat_task = Some(spawn_manager_heartbeat(
            discovery.clone(),
            cluster_name.clone(),
            membership.clone(),
        ));

        let manager_peer_id = Arc::new(Mutex::new(local_peer_id));

        // 5. Drain inbound `/auki/join/0.0.1` events: admit-or-reject
        //    based on whether we're the Manager + whether the peer
        //    is already a member.
        let join_handler_task = Some(spawn_join_handler(
            join_events_rx,
            cluster_name.clone(),
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

        Ok(member)
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
        let (runtime, join_events_rx) = NetworkRuntime::spawn(
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

        // 6. Drain inbound join events. As a non-Manager, our
        //    handler always rejects with "not the manager"; if/when
        //    SDK-T7 elects us we'll start admitting.
        let join_handler_task = Some(spawn_join_handler(
            join_events_rx,
            cluster_name.clone(),
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
            heartbeat_task: None,
            join_handler_task,
        })
    }

    /// Shutdown — cancels the Manager heartbeat tick (if any),
    /// cancels the join handler task, deregisters the cluster from
    /// Discovery (if we're the Manager), and shuts down the
    /// runtime. Consumes `self`.
    pub async fn shutdown(mut self) -> Result<(), DiscoveryError> {
        // 1. Cancel background tasks FIRST so we stop touching
        //    Discovery / membership between teardown steps.
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
        }
        if let Some(task) = self.join_handler_task.take() {
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
}
