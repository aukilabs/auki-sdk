//! [`ClusterManager`] — the SDK-side handle for a cluster a daemon is
//! participating in. **The single SDK entry point for all
//! Discovery / cluster-lifecycle interaction.** Per Hagall constraint
//! #5 ("the SDK should handle as much as possible of the daemon-side
//! networking, so that Booster and Park work the same way"), daemons
//! never construct a [`DiscoveryClient`] themselves; they declare
//! intent via [`ClusterTarget`] and call [`ClusterManager::bootstrap`]
//! (or the operator-intent-shaped [`ClusterManager::create_cluster`] /
//! [`ClusterManager::join_cluster`] primitives). All Discovery HTTP
//! talking, decision logic, and lifecycle management live SDK-side.
//!
//! Owns the cluster membership document, the libp2p `NetworkRuntime`
//! that drives the swarm, and a liveness-check-to-Discovery task.
//! Daemons (BoosterApp, Park, Sentinel) treat the returned
//! `ClusterManager` as a single owned object — its public methods
//! cover everything daemons need to surface to operators
//! (`is_manager`, `manager_peer_id`, `participant_info`,
//! `membership`).
//!
//! ## App-facing entry points
//!
//! - [`ClusterManager::list_clusters`] — snapshot Discovery's directory.
//! - [`ClusterManager::bootstrap`] — policy-driven: declare intent via
//!   [`ClusterTarget`] (Create / Join / JoinOrCreate / MostRecentOrCreate)
//!   and the SDK does list + decide + create-or-join internally.
//! - [`ClusterManager::create_cluster`] — operator-intent primitive when
//!   "create exactly this name" is unambiguous (e.g. Park UI Create
//!   button).
//! - [`ClusterManager::join_cluster`] — operator-intent primitive when
//!   "join exactly this name" is unambiguous (e.g. Park UI Join button).
//!
//! ## Manager-role state
//!
//! When `create_cluster` (or `bootstrap` that resolves to create)
//! succeeds, the local peer is the cluster's initial Manager.
//! [`Self::is_manager`] is `true`; [`Self::manager_peer_id`] equals the
//! local peer-id. For a `join`, `is_manager` is `false` and
//! `manager_peer_id` points at whoever the cluster currently agrees is
//! the Manager.
//!
//! ## Discovery liveness check
//!
//! While this peer is the Manager, a background task pushes a
//! `liveness_check` to Discovery every [`LIVENESS_CHECK_INTERVAL`] (1s)
//! with the cluster's `peer_count`. Discovery's `liveness requirement`
//! sweep drops clusters that haven't received a check in 3s (3 missed),
//! so this keeps the directory entry live. The task is cancelled on
//! [`Self::shutdown`].

use crate::cluster_membership::{ClusterMember, ClusterMembership};
use auki_network::ParticipantInfo;
use auki_network::SessionHandle;
use auki_network::discovery_client::{
    ClusterEntry, CreateClusterOutcome, DiscoveryClient, DiscoveryError,
};
use auki_network::heartbeat_protocol::HeartbeatDomainClock;
use auki_network::registries_protocol::{
    RegistryEntryEnvelope, RegistryKind, RegistryRequest, RegistryResponse,
};
use auki_network::resources_protocol::{ResourceEntry, ResourcesRequest, ResourcesResponse};
use auki_registry::{
    ClockRegistryEntry, DetectorRegistryEntry, FrameRegistryEntry, SensorRegistryEntry,
};
use auki_time::SessionClock;

// Re-exports so app code can stay scoped to `auki_domain` imports
// (per the "ClusterManager is the single SDK entry point" contract
// at the top of this module).
pub use auki_network::diagnostic_protocol::DiagnosticMessage;
pub use auki_network::discovery_client::ClusterEntry as DiscoveryClusterEntry;
pub use auki_network::discovery_client::DiscoveryError as DiscoveryClientError;
use auki_network::heartbeat_protocol::HEARTBEAT_TIMEOUT;
use auki_network::info_protocol::InfoResponse;
use auki_network::join_protocol::{JoinRequest, JoinResponse};
use auki_network::network_runtime::{
    AllowedPeer, BroadcastDiagnosticError, DiagnosticEvent, HeartbeatNtpSampleObservation,
    HeartbeatTimestampSource, InfoRequestEvent, JoinEvent, MembershipEvent, NetworkRuntime,
    PeerLivenessEvent, RegistryRequestEvent, RequestInfoError, RequestRegistryError,
    RequestResourcesError, ResourcesRequestEvent, SendJoinRequestError, SpawnError,
};
use auki_network::stream_runtime::StreamProvider;
use auki_network::swarm::Behaviour;
use auki_network::{PeerIdentity, Swarm};
use auki_time::{
    ClockSyncHandle, ClockSyncObservation, ClockTransformEstimate, DomainClockDescriptor,
    DomainClockEstimate,
};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Cadence of the Manager → Discovery liveness-check tick. Matches the
/// Hagall v1 contract (2026-05-14 rename) — Discovery's `liveness
/// requirement` sweep window is 3s, so a 1s cadence leaves 3
/// consecutive misses' tolerance before the cluster is swept.
pub const LIVENESS_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Relay reservation request for a Manager-capable peer.
///
/// `relay_dial_multiaddr` is the address this native peer can dial to
/// reserve the circuit. `relay_advertise_multiaddr` is the relay base
/// address Discovery should expose for browser peers. Both addresses
/// must identify the same relay peer id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerRelayReservation {
    /// Native-dialable relay address, including `/p2p/<relay-peer-id>`.
    pub relay_dial_multiaddr: Multiaddr,
    /// Browser-dialable relay base address, including
    /// `/p2p/<relay-peer-id>`.
    pub relay_advertise_multiaddr: Multiaddr,
    /// Maximum time to wait for the relay reservation to complete.
    pub timeout: Duration,
}

/// Daemon-side identity fields the SDK doesn't own. The daemon
/// hands one of these to [`ClusterManager::create_cluster`] /
/// [`ClusterManager::join_cluster`] **at construction time**; the
/// ClusterManager stores it and rebuilds a fresh
/// [`ParticipantInfo`] on each call to `participant_info()` /
/// inbound `/auki/info/0.0.1` request.
///
/// Dynamic fields (`session_now_ns` from the SDK-owned [`SessionClock`],
/// `cluster_joined_at_ns` set lazily on first non-self peer
/// observation) live on the ClusterManager — not on `DaemonInfo` —
/// so daemons aren't responsible for keeping them fresh.
#[derive(Debug, Clone)]
pub struct DaemonInfo {
    /// Application identifier (`"boosterapp"`, `"sentinel"`, `"park"`).
    pub app: String,
    /// Operator-friendly per-device label.
    pub name: String,
    /// UUIDv4 minted at session boot.
    pub session_id: String,
    /// Compatibility input for older callers that still construct a
    /// session-clock registry id. New `ParticipantInfo` values use the
    /// SDK-owned peer-id anchored `SessionClock`.
    pub session_clock_id: String,
    /// Compatibility input for older callers that still construct a
    /// session-clock registry hash.
    pub session_clock_hash: String,
    /// First non-loopback IEEE-administered MAC, lowercased hex
    /// without separators.
    pub app_instance: String,
}

/// Why this cluster handle cannot currently produce a local
/// session-clock to cluster-domain-clock estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainClockEstimateUnavailable {
    /// No heartbeat has advertised a domain-clock source for this
    /// cluster, and this peer did not create the cluster as initial
    /// Manager.
    SourceUnavailable {
        /// Cluster whose domain clock was requested.
        cluster_name: String,
    },
    /// A domain-clock source is known, but the local peer does not
    /// yet have a heartbeat-derived transform into that source's
    /// concrete backing clock.
    BackingEstimateUnavailable {
        /// Local session clock id.
        local_clock_id: String,
        /// Concrete backing clock required by the domain source.
        backing_clock_id: String,
    },
    /// The known source metadata and the peer-clock estimate are
    /// inconsistent, or their composed offset overflowed.
    InvalidSource(auki_time::DomainClockEstimateError),
}

impl fmt::Display for DomainClockEstimateUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceUnavailable { cluster_name } => {
                write!(
                    f,
                    "domain clock source unavailable for cluster {cluster_name:?}"
                )
            }
            Self::BackingEstimateUnavailable {
                local_clock_id,
                backing_clock_id,
            } => write!(
                f,
                "peer-clock estimate unavailable from {local_clock_id:?} to backing clock {backing_clock_id:?}"
            ),
            Self::InvalidSource(err) => write!(f, "invalid domain clock source: {err}"),
        }
    }
}

impl std::error::Error for DomainClockEstimateUnavailable {}

/// Why this cluster handle cannot convert the current local session
/// clock reading into cluster-domain time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainTimeNowError {
    /// The cluster-domain clock estimate is not available yet.
    Unavailable(DomainClockEstimateUnavailable),
    /// Applying the estimated offset to the current local session
    /// clock reading would overflow the SDK's signed nanosecond
    /// timestamp representation.
    ConversionOutOfRange {
        /// Current reading of the local session monotonic clock.
        session_now_ns: i64,
        /// Offset from the local session clock into the cluster
        /// domain clock.
        offset_ns: i64,
    },
}

impl From<DomainClockEstimateUnavailable> for DomainTimeNowError {
    fn from(err: DomainClockEstimateUnavailable) -> Self {
        Self::Unavailable(err)
    }
}

impl fmt::Display for DomainTimeNowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(err) => write!(f, "{err}"),
            Self::ConversionOutOfRange {
                session_now_ns,
                offset_ns,
            } => write!(
                f,
                "domain time conversion out of range: session_now_ns {session_now_ns} + offset_ns {offset_ns}"
            ),
        }
    }
}

impl std::error::Error for DomainTimeNowError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DomainClockSourceKey {
    cluster_name: String,
    backing_peer_id: String,
    backing_clock_id: String,
}

impl DomainClockSourceKey {
    fn from_source(source: &HeartbeatDomainClock) -> Self {
        Self {
            cluster_name: source.cluster_name.clone(),
            backing_peer_id: source.backing_peer_id.clone(),
            backing_clock_id: source.backing_clock_id.clone(),
        }
    }
}

type DomainClockSources = Arc<Mutex<HashMap<DomainClockSourceKey, HeartbeatDomainClock>>>;

/// Application-supplied source of truth for resources the daemon
/// can currently provide. Install this provider for all resource
/// types: sensor streams, pose logs, time-transform logs, etc.
pub trait ResourceCatalogProvider: Send + Sync + 'static {
    /// Snapshot currently-advertised resources. Called once per
    /// inbound `/auki/resources/0.2.0` request. Keep it cheap — the
    /// runtime's per-substream task gives the SDK 2 s to respond
    /// before closing the substream.
    fn snapshot(&self) -> Vec<ResourceEntry>;

    /// Snapshot resources for a concrete request. The default
    /// implementation filters by requested variant and returns the
    /// matching rows.
    fn snapshot_for_request(
        &self,
        request: &ResourcesRequest,
        _registry_app_root: Option<&Path>,
    ) -> Vec<ResourceEntry> {
        let resources = self.snapshot();
        if request.variants.is_empty() {
            return resources;
        }
        use auki_network::resources_protocol::{Variant, VariantContent};
        resources
            .into_iter()
            .filter(|r| {
                let row_variant = match &r.variant_content {
                    VariantContent::SensorLog { .. } => Variant::SensorLog,
                    VariantContent::PoseLog { .. } => Variant::PoseLog,
                    VariantContent::TimeTransformLog { .. } => Variant::TimeTransformLog,
                    VariantContent::DetectionLog { .. } => Variant::DetectionLog,
                };
                request.variants.contains(&row_variant)
            })
            .collect()
    }
}

/// What kind of cluster lifecycle action [`ClusterManager::bootstrap`]
/// should perform. Captures the four decision shapes that every Auki
/// daemon picker has had to express in its own code; lifting them into
/// the SDK is the Hagall-constraint-#5 "uniform Discovery talking"
/// fix.
///
/// Use the static constructors ([`ClusterTarget::create`],
/// [`ClusterTarget::join`], [`ClusterTarget::join_or_create`],
/// [`ClusterTarget::most_recent_or_create`]) for ergonomics; the bare
/// enum variants are exposed for pattern-matching only.
#[derive(Debug, Clone)]
pub enum ClusterTarget {
    /// Create a new cluster with this exact name. Errors with
    /// [`BootstrapError::AlreadyExists`] if Discovery already has it.
    /// Use this when operator intent is "create" (e.g. Park's Create
    /// button).
    Create {
        /// Cluster name to create.
        name: String,
    },
    /// Join an existing cluster with this exact name. Errors with
    /// [`BootstrapError::NotFound`] if Discovery doesn't have it. Use
    /// this when operator intent is "join" (e.g. Park's Join button
    /// clicked on a list row).
    Join {
        /// Cluster name to join.
        name: String,
    },
    /// Join the named cluster if it exists; otherwise create it. Use
    /// this when a daemon is configured with a specific cluster name
    /// but doesn't care whether it's the first peer or a joiner
    /// (e.g. Boosterapp with `--cluster-name` set).
    JoinOrCreate {
        /// Cluster name to join-or-create.
        name: String,
    },
    /// Join the most-recently-created cluster in Discovery's directory.
    /// If the directory is empty, create a cluster named
    /// `fallback_name`. Use this for headless daemons with no operator
    /// intent (e.g. Boosterapp without `--cluster-name`).
    MostRecentOrCreate {
        /// Cluster name to fall back to if Discovery's directory is
        /// empty at bootstrap time.
        fallback_name: String,
    },
}

impl ClusterTarget {
    /// Create a new cluster with `name`. Sugar for
    /// [`ClusterTarget::Create`].
    pub fn create(name: impl Into<String>) -> Self {
        Self::Create { name: name.into() }
    }

    /// Join an existing cluster with `name`. Sugar for
    /// [`ClusterTarget::Join`].
    pub fn join(name: impl Into<String>) -> Self {
        Self::Join { name: name.into() }
    }

    /// Join the cluster `name` if it exists; otherwise create it.
    /// Sugar for [`ClusterTarget::JoinOrCreate`].
    pub fn join_or_create(name: impl Into<String>) -> Self {
        Self::JoinOrCreate { name: name.into() }
    }

    /// Join the most-recent cluster; fall back to creating
    /// `fallback_name` on empty directory. Sugar for
    /// [`ClusterTarget::MostRecentOrCreate`].
    pub fn most_recent_or_create(fallback_name: impl Into<String>) -> Self {
        Self::MostRecentOrCreate {
            fallback_name: fallback_name.into(),
        }
    }
}

/// Errors from [`ClusterManager::bootstrap`].
///
/// Aggregates the failure modes of [`CreateClusterError`] and
/// [`JoinClusterError`] since bootstrap may resolve to either path.
#[derive(Debug, Error)]
pub enum BootstrapError {
    /// Discovery rejected the list / create / lookup call, or HTTP
    /// transport failed.
    #[error("Discovery: {0}")]
    Discovery(#[from] DiscoveryError),
    /// `ClusterTarget::Create { name }` was passed but the cluster
    /// already exists in Discovery's directory. The caller can choose
    /// to retry with `ClusterTarget::JoinOrCreate` or
    /// `ClusterTarget::Join` if they want fallthrough.
    #[error("cluster {0:?} already exists in Discovery directory")]
    AlreadyExists(String),
    /// `ClusterTarget::Join { name }` was passed but the cluster is
    /// not in Discovery's directory.
    #[error("cluster {0:?} not in Discovery directory")]
    NotFound(String),
    /// Joining the Manager failed (substream open / read / write
    /// error, or the Manager hung up before responding).
    #[error("join request: {0}")]
    SendJoin(#[from] SendJoinRequestError),
    /// The Manager refused the join with a typed reason.
    #[error("Manager rejected join: {0}")]
    Rejected(String),
    /// The Manager's response carried a malformed `membership_json`.
    #[error("invalid membership JSON from Manager: {0}")]
    InvalidMembership(#[source] serde_json::Error),
    /// `NetworkRuntime::spawn` failed.
    #[error("runtime spawn failed: {0}")]
    Runtime(#[from] SpawnError),
    /// The Manager could not reserve its relay-mediated address before
    /// publishing Discovery metadata.
    #[error("relay reservation failed: {0}")]
    RelayReservation(#[from] auki_network::swarm::RelayReservationError),
}

impl From<CreateClusterError> for BootstrapError {
    fn from(e: CreateClusterError) -> Self {
        match e {
            CreateClusterError::Discovery(d) => BootstrapError::Discovery(d),
            CreateClusterError::AlreadyExists(n) => BootstrapError::AlreadyExists(n),
            CreateClusterError::Runtime(s) => BootstrapError::Runtime(s),
            CreateClusterError::RelayReservation(r) => BootstrapError::RelayReservation(r),
        }
    }
}

impl From<JoinClusterError> for BootstrapError {
    fn from(e: JoinClusterError) -> Self {
        match e {
            JoinClusterError::Discovery(d) => BootstrapError::Discovery(d),
            JoinClusterError::NotFound(n) => BootstrapError::NotFound(n),
            JoinClusterError::SendJoin(s) => BootstrapError::SendJoin(s),
            JoinClusterError::Rejected(r) => BootstrapError::Rejected(r),
            JoinClusterError::InvalidMembership(e) => BootstrapError::InvalidMembership(e),
            JoinClusterError::Runtime(s) => BootstrapError::Runtime(s),
        }
    }
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
    /// Relay reservation failed before Discovery registration. The
    /// cluster is not created in Discovery when this is returned.
    #[error("relay reservation failed: {0}")]
    RelayReservation(#[from] auki_network::swarm::RelayReservationError),
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
    /// [`ClusterManager::shutdown`] has been called. Callers
    /// holding a stale `Arc<ClusterManager>` clone see this
    /// rather than a cascading channel-closed error.
    #[error("ClusterManager has been shut down")]
    Stopped,
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
    discovery: Arc<DiscoveryClient>,
    local_multiaddrs: Vec<Multiaddr>,
    relay_multiaddrs: Vec<Multiaddr>,
    /// Static daemon-side identity fields. Stored at construction;
    /// combined with dynamic SDK-tracked fields (`session_now_ns`,
    /// `cluster_joined_at_ns`, `is_manager`, `manager_peer_id`,
    /// `peer_id`) when building a [`ParticipantInfo`].
    daemon_info: DaemonInfo,
    /// SDK-owned session-monotonic clock. Compatibility callers still pass
    /// `DaemonInfo.session_clock_id/hash`, but ParticipantInfo is minted from
    /// this peer-id anchored clock.
    session_clock: SessionClock,
    /// Session-clock value at first observation of a peer other
    /// than ourselves. `None` while this daemon is alone in its
    /// cluster; set once and sticky thereafter. Mutated by
    /// `spawn_info_handler` lazily on each `participant_info()`
    /// build.
    cluster_joined_at_ns: Arc<Mutex<Option<u64>>>,
    /// Peer-local clock sync estimates produced from heartbeat NTP
    /// sample events. `auki-time` owns the retention/selection
    /// policy; `ClusterManager` only forwards events and exposes
    /// read-only snapshots.
    clock_sync: ClockSyncHandle,
    /// Domain-clock source declarations received on heartbeat frames,
    /// plus the initial Manager's own declaration when this peer
    /// creates the cluster.
    domain_clock_sources: DomainClockSources,
    /// Manager-side Discovery liveness-check task. Wrapped in
    /// `Arc<Mutex<Option<_>>>` so the liveness handler can spawn it
    /// on Manager-promotion (SDK-T7 handoff). `Some` while this peer
    /// is the Manager; `None` otherwise. Cancelled on `shutdown`.
    liveness_check_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Task that drains inbound `/auki/join/0.0.1` events from the
    /// runtime, decides admit-or-reject, and replies. Lives for the
    /// lifetime of the ClusterManager. Cancelled on `shutdown` via
    /// the `Mutex<Option<_>>::take()` pattern (idempotent against
    /// concurrent / repeated `shutdown` calls).
    join_handler_task: Mutex<Option<JoinHandle<()>>>,
    /// Task that drains heartbeat-carrier events from the runtime,
    /// runs the domain-side heartbeat timer, runs the cluster-
    /// internal election on Manager death, and orchestrates
    /// Manager-handoff when the local peer wins.
    /// Cancelled on `shutdown`.
    liveness_handler_task: Mutex<Option<JoinHandle<()>>>,
    /// Task that drains inbound `/auki/membership/0.0.1` gossip
    /// events from the runtime, parses the membership JSON, swaps
    /// the local membership document, and pushes the updated
    /// allow-list to the runtime. Cancelled on `shutdown`.
    membership_handler_task: Mutex<Option<JoinHandle<()>>>,
    /// Task that drains inbound `/auki/info/0.0.1` requests from
    /// the runtime, builds a [`ParticipantInfo`] from current
    /// state, and replies. Cancelled on `shutdown`.
    info_handler_task: Mutex<Option<JoinHandle<()>>>,
    /// Task that drains inbound `/auki/resources/0.2.0` requests
    /// from the runtime, snapshots resource providers, and replies.
    /// Cancelled on `shutdown`.
    resources_handler_task: Mutex<Option<JoinHandle<()>>>,
    /// Task that drains inbound `/auki/registries/0.0.1` requests
    /// from the runtime, reads the requested entry from
    /// producer-local registry storage, and replies. Cancelled on
    /// `shutdown`.
    registry_handler_task: Mutex<Option<JoinHandle<()>>>,
    /// Task that drains inbound best-effort app diagnostic messages.
    diagnostic_handler_task: Mutex<Option<JoinHandle<()>>>,
    diagnostic_messages: Arc<Mutex<Vec<InboundDiagnosticMessage>>>,
    /// Application-supplied resource catalog provider. Resource catalog
    /// is primarily served via [`SessionHandle`]; this provider is an
    /// alternative for consumers that don't use the session abstraction.
    resource_catalog_provider: Arc<Mutex<Option<Arc<dyn ResourceCatalogProvider>>>>,
    /// Source of resource catalog rows for the `/auki/resources/0.2.0`
    /// handler. When set, its [`SessionHandle::catalog`] method is
    /// called on each inbound request instead of building rows from
    /// the old sensor/resource provider pair.
    session_handle: Arc<Mutex<Option<Arc<dyn SessionHandle>>>>,
    /// Producer-local app root containing `registries/{sensors,clocks,frames}`.
    /// `None` until the daemon calls [`Self::set_registry_app_root`];
    /// inbound registry fetches return `entry: None` while unset.
    registry_app_root: Arc<Mutex<Option<PathBuf>>>,
    /// Set to `true` by [`Self::shutdown`] before any teardown
    /// begins. Pub I/O methods (`admit_peer`,
    /// `fetch_participant_info`) check this and fast-fail with a
    /// typed `Stopped` error so callers holding stale `Arc<Self>`
    /// clones after shutdown see a clean signal rather than the
    /// cascading runtime-channel-closed / libp2p-substream-failed
    /// errors. Snapshot accessors (`membership`, `peer_count`,
    /// `is_manager`, …) are not gated — returning the
    /// last-observed state is harmless and lets consumers drain
    /// their final view.
    stopped: AtomicBool,
}

/// Best-effort diagnostic message received from a cluster peer.
#[derive(Debug, Clone)]
pub struct InboundDiagnosticMessage {
    /// Authenticated sender peer id.
    pub peer_id: PeerId,
    /// Opaque app-level diagnostic topic and payload.
    pub message: DiagnosticMessage,
}

impl ClusterManager {
    /// Snapshot Discovery's cluster directory. The SDK-fronted entry
    /// point for "list clusters" — apps don't construct
    /// [`DiscoveryClient`] themselves.
    ///
    /// Sorted by `created_ns` desc (newest-first) per the Hagall v1
    /// contract. Returns the typed [`DiscoveryClusterEntry`] values
    /// re-exported from this module.
    pub async fn list_clusters(
        discovery_url: impl Into<String>,
    ) -> Result<Vec<ClusterEntry>, DiscoveryError> {
        let client = DiscoveryClient::new(discovery_url.into());
        client.list_clusters().await
    }

    /// Policy-driven cluster bootstrap — **the single entry point**
    /// for app daemons that don't have unambiguous operator intent.
    /// Park's Create / Join buttons still call
    /// [`Self::create_cluster`] / [`Self::join_cluster`] directly
    /// (operator-intent path); headless daemons (Boosterapp,
    /// Sentinel) call this with the matching [`ClusterTarget`].
    ///
    /// Dispatch:
    /// - [`ClusterTarget::Create { name }`][ClusterTarget::Create] →
    ///   [`Self::create_cluster`]. Errors with
    ///   [`BootstrapError::AlreadyExists`] if the name is taken.
    /// - [`ClusterTarget::Join { name }`][ClusterTarget::Join] →
    ///   [`Self::join_cluster`]. Errors with
    ///   [`BootstrapError::NotFound`] if Discovery doesn't have it.
    /// - [`ClusterTarget::JoinOrCreate { name }`][ClusterTarget::JoinOrCreate]
    ///   → list Discovery; if the name exists, join; else create.
    ///   Race-tolerant: if a concurrent `create` won the name between
    ///   our list and our own `create`, the
    ///   `CreateClusterError::AlreadyExists` is caught and we fall
    ///   through to `join`.
    /// - [`ClusterTarget::MostRecentOrCreate { fallback_name }`][ClusterTarget::MostRecentOrCreate]
    ///   → list Discovery; if non-empty, join the first entry
    ///   (created_ns desc → newest-first); else create
    ///   `fallback_name`.
    ///
    /// All Discovery talking is internal — apps don't construct
    /// [`DiscoveryClient`].
    pub async fn bootstrap(
        target: ClusterTarget,
        local_identity: PeerIdentity,
        local_multiaddrs: Vec<Multiaddr>,
        discovery_url: impl Into<String>,
        swarm: Swarm<Behaviour>,
        stream_provider: StreamProvider,
        daemon_info: DaemonInfo,
    ) -> Result<Self, BootstrapError> {
        let discovery_url = discovery_url.into();
        match target {
            ClusterTarget::Create { name } => Self::create_cluster(
                name,
                local_identity,
                local_multiaddrs,
                discovery_url,
                swarm,
                stream_provider,
                daemon_info,
            )
            .await
            .map_err(BootstrapError::from),
            ClusterTarget::Join { name } => Self::join_cluster(
                name,
                local_identity,
                local_multiaddrs,
                discovery_url,
                swarm,
                stream_provider,
                daemon_info,
            )
            .await
            .map_err(BootstrapError::from),
            ClusterTarget::JoinOrCreate { name } => {
                let entries = Self::list_clusters(discovery_url.clone()).await?;
                if entries.iter().any(|e| e.name == name) {
                    Self::join_cluster(
                        name,
                        local_identity,
                        local_multiaddrs,
                        discovery_url,
                        swarm,
                        stream_provider,
                        daemon_info,
                    )
                    .await
                    .map_err(BootstrapError::from)
                } else {
                    // Race window: someone else may have created
                    // `name` between our list and our create. If our
                    // create comes back AlreadyExists, fall through to
                    // join (the race-winner's cluster).
                    match Self::create_cluster(
                        name.clone(),
                        local_identity.clone(),
                        local_multiaddrs.clone(),
                        discovery_url.clone(),
                        swarm,
                        stream_provider,
                        daemon_info,
                    )
                    .await
                    {
                        Ok(m) => Ok(m),
                        Err(CreateClusterError::AlreadyExists(_)) => {
                            // The race-winner's create burned our
                            // swarm + stream_provider; we can't retry
                            // join from inside this function (they're
                            // moved). Surface as AlreadyExists; the
                            // app re-calls bootstrap if it wants to
                            // retry. Documented in this method's
                            // doc-comment.
                            Err(BootstrapError::AlreadyExists(name))
                        }
                        Err(e) => Err(BootstrapError::from(e)),
                    }
                }
            }
            ClusterTarget::MostRecentOrCreate { fallback_name } => {
                let entries = Self::list_clusters(discovery_url.clone()).await?;
                if let Some(first) = entries.first() {
                    let name = first.name.clone();
                    Self::join_cluster(
                        name,
                        local_identity,
                        local_multiaddrs,
                        discovery_url,
                        swarm,
                        stream_provider,
                        daemon_info,
                    )
                    .await
                    .map_err(BootstrapError::from)
                } else {
                    Self::create_cluster(
                        fallback_name,
                        local_identity,
                        local_multiaddrs,
                        discovery_url,
                        swarm,
                        stream_provider,
                        daemon_info,
                    )
                    .await
                    .map_err(BootstrapError::from)
                }
            }
        }
    }

    /// Create a new cluster and become its initial Manager. Atomic
    /// against concurrent `create_cluster` calls — only one peer
    /// wins; the loser gets [`CreateClusterError::AlreadyExists`]
    /// and should `list` + `join` instead.
    ///
    /// Operator-intent primitive — call this when the operator clicked
    /// "Create" in a UI. For headless daemons, use
    /// [`Self::bootstrap`] with a [`ClusterTarget`] policy instead.
    ///
    /// Sequence:
    /// 1. Build the [`DiscoveryClient`] from `discovery_url` and call
    ///    `create_cluster(...)` with the local peer as the initial
    ///    Manager.
    /// 2. Initialize the membership document with the local peer as
    ///    its only entry (`join_ts_ns` = `now_ns()`, opaque empty
    ///    successor token for v1).
    /// 3. Spawn the `NetworkRuntime` with an empty allow-list (no
    ///    cluster members yet besides ourselves; we don't dial
    ///    ourselves) and the daemon's `stream_provider`.
    /// 4. Spawn the Manager-side Discovery liveness-check tick.
    /// 5. Return the `ClusterManager`.
    pub async fn create_cluster(
        cluster_name: impl Into<String>,
        local_identity: PeerIdentity,
        local_multiaddrs: Vec<Multiaddr>,
        discovery_url: impl Into<String>,
        swarm: Swarm<Behaviour>,
        stream_provider: StreamProvider,
        daemon_info: DaemonInfo,
    ) -> Result<Self, CreateClusterError> {
        Self::create_cluster_with_relay_hints(
            cluster_name,
            local_identity,
            local_multiaddrs,
            Vec::new(),
            discovery_url,
            swarm,
            stream_provider,
            daemon_info,
        )
        .await
    }

    /// Create a new cluster and publish relay hints alongside the
    /// Manager's direct addresses in Discovery.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_cluster_with_relay_multiaddrs(
        cluster_name: impl Into<String>,
        local_identity: PeerIdentity,
        local_multiaddrs: Vec<Multiaddr>,
        relay_multiaddrs: Vec<Multiaddr>,
        discovery_url: impl Into<String>,
        swarm: Swarm<Behaviour>,
        stream_provider: StreamProvider,
        daemon_info: DaemonInfo,
    ) -> Result<Self, CreateClusterError> {
        Self::create_cluster_with_relay_hints(
            cluster_name,
            local_identity,
            local_multiaddrs,
            relay_multiaddrs,
            discovery_url,
            swarm,
            stream_provider,
            daemon_info,
        )
        .await
    }

    /// Reserve a relay-mediated Manager address before creating the
    /// cluster, then publish both the native Manager addresses and the
    /// relay circuit Manager address through Discovery.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_cluster_with_relay_reservation(
        cluster_name: impl Into<String>,
        local_identity: PeerIdentity,
        mut local_multiaddrs: Vec<Multiaddr>,
        relay_reservation: ManagerRelayReservation,
        discovery_url: impl Into<String>,
        mut swarm: Swarm<Behaviour>,
        stream_provider: StreamProvider,
        daemon_info: DaemonInfo,
    ) -> Result<Self, CreateClusterError> {
        let mut relay_multiaddrs = Vec::new();
        reserve_manager_relay_multiaddr(
            &mut swarm,
            &mut local_multiaddrs,
            &mut relay_multiaddrs,
            &relay_reservation,
        )
        .await?;
        Self::create_cluster_with_relay_hints(
            cluster_name,
            local_identity,
            local_multiaddrs,
            relay_multiaddrs,
            discovery_url,
            swarm,
            stream_provider,
            daemon_info,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_cluster_with_relay_hints(
        cluster_name: impl Into<String>,
        local_identity: PeerIdentity,
        local_multiaddrs: Vec<Multiaddr>,
        relay_multiaddrs: Vec<Multiaddr>,
        discovery_url: impl Into<String>,
        swarm: Swarm<Behaviour>,
        stream_provider: StreamProvider,
        daemon_info: DaemonInfo,
    ) -> Result<Self, CreateClusterError> {
        let discovery = DiscoveryClient::new(discovery_url.into());
        let cluster_name = cluster_name.into();
        let local_peer_id = local_identity.peer_id();
        let session_clock = SessionClock::new(
            local_peer_id.to_string(),
            daemon_info.session_id.clone(),
            "monotonic",
        );
        let cluster_joined_at_ns: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));
        let manager_peer_id = Arc::new(Mutex::new(local_peer_id));
        let clock_sync = ClockSyncHandle::default();
        let domain_clock_sources = domain_clock_source_store();
        let initial_domain_clock =
            initial_domain_clock_source(&cluster_name, local_peer_id, &session_clock);
        observe_heartbeat_domain_clock_source(
            &domain_clock_sources,
            &cluster_name,
            local_peer_id,
            Some(initial_domain_clock.clone()),
        );
        let advertised_domain_clock_source = Arc::new(Mutex::new(Some(initial_domain_clock)));

        // 1. Atomic create on Discovery.
        let create_outcome = if relay_multiaddrs.is_empty() {
            discovery
                .create_cluster(
                    cluster_name.clone(),
                    local_peer_id,
                    local_multiaddrs.clone(),
                )
                .await?
        } else {
            discovery
                .create_cluster_with_relay_multiaddrs(
                    cluster_name.clone(),
                    local_peer_id,
                    local_multiaddrs.clone(),
                    relay_multiaddrs.clone(),
                )
                .await?
        };
        match create_outcome {
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
        let (
            runtime,
            join_events_rx,
            liveness_rx,
            membership_events_rx,
            info_events_rx,
            resources_events_rx,
            registry_events_rx,
            diagnostic_events_rx,
        ) = NetworkRuntime::spawn(
            swarm,
            vec![],
            stream_provider,
            heartbeat_timestamp_source(
                session_clock.clone(),
                advertised_domain_clock_source.clone(),
            ),
        )?;
        runtime
            .set_heartbeat_targets(vec![])
            .await
            .map_err(|_| CreateClusterError::Runtime(SpawnError::NoTokioRuntime))?;

        // 4. Manager-side Discovery liveness-check tick.
        let liveness_check_task: Arc<Mutex<Option<JoinHandle<()>>>> =
            Arc::new(Mutex::new(Some(spawn_manager_liveness_check(
                discovery.clone(),
                cluster_name.clone(),
                membership.clone(),
            ))));

        // 5. Drain inbound `/auki/join/0.0.1` events.
        let join_handler_task = Mutex::new(Some(spawn_join_handler(
            join_events_rx,
            cluster_name.clone(),
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
        )));

        // 6. Drain heartbeat carrier events and run the domain-side
        //    liveness timer. On Manager death, run the cluster-
        //    internal election; if we win, become the new Manager
        //    (update state, rotate Discovery, start the liveness
        //    check tick).
        let liveness_handler_task = Mutex::new(Some(spawn_liveness_handler(
            liveness_rx,
            cluster_name.clone(),
            local_peer_id,
            local_multiaddrs.clone(),
            relay_multiaddrs.clone(),
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
            discovery.clone(),
            liveness_check_task.clone(),
            clock_sync.clone(),
            domain_clock_sources.clone(),
            advertised_domain_clock_source.clone(),
            session_clock.clone(),
        )));

        // 7. Drain inbound /auki/membership/0.0.1 gossip events. As
        //    the freshly-minted Manager we don't expect to receive
        //    any (nobody else is gossiping yet), but if a stale peer
        //    sends one we apply it last-write-wins — the next
        //    Manager broadcast supersedes.
        let membership_handler_task = Mutex::new(Some(spawn_membership_handler(
            membership_events_rx,
            cluster_name.clone(),
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
        )));

        // 8. Drain inbound /auki/info/0.0.1 requests. Build a fresh
        //    `ParticipantInfo` from stored daemon_info + dynamic SDK
        //    state on each request and reply.
        let info_handler_task = Mutex::new(Some(spawn_info_handler(
            info_events_rx,
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            daemon_info.clone(),
            session_clock.clone(),
            cluster_joined_at_ns.clone(),
        )));

        let registry_app_root: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

        // 9. Drain inbound /auki/resources/0.2.0 requests. The
        //    resources handler uses an app-supplied provider when
        //    present, otherwise falls back to a SessionHandle.
        let resource_catalog_provider: Arc<Mutex<Option<Arc<dyn ResourceCatalogProvider>>>> =
            Arc::new(Mutex::new(None));
        let session_handle: Arc<Mutex<Option<Arc<dyn SessionHandle>>>> = Arc::new(Mutex::new(None));
        let resources_handler_task = Mutex::new(Some(spawn_resources_handler(
            resources_events_rx,
            resource_catalog_provider.clone(),
            session_handle.clone(),
        )));

        // 10. Drain inbound /auki/registries/0.0.1 requests. Read
        //     the exact registry entry from app-root storage if the
        //     daemon has registered an app root.
        let registry_handler_task = Mutex::new(Some(spawn_registry_handler(
            registry_events_rx,
            registry_app_root.clone(),
            local_peer_id.to_string(),
        )));
        let diagnostic_messages = Arc::new(Mutex::new(Vec::new()));
        let diagnostic_handler_task = Mutex::new(Some(spawn_diagnostic_handler(
            diagnostic_events_rx,
            diagnostic_messages.clone(),
        )));

        Ok(Self {
            cluster_name,
            local_peer_id,
            membership,
            manager_peer_id,
            runtime,
            discovery,
            local_multiaddrs,
            relay_multiaddrs,
            daemon_info,
            session_clock,
            cluster_joined_at_ns,
            clock_sync,
            domain_clock_sources,
            liveness_check_task,
            join_handler_task,
            liveness_handler_task,
            membership_handler_task,
            info_handler_task,
            resources_handler_task,
            registry_handler_task,
            diagnostic_handler_task,
            diagnostic_messages,
            resource_catalog_provider,
            session_handle,
            registry_app_root,
            stopped: AtomicBool::new(false),
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

    /// Relay multiaddrs this cluster entry should preserve when the
    /// local peer becomes Manager.
    pub fn relay_multiaddrs(&self) -> &[Multiaddr] {
        &self.relay_multiaddrs
    }

    /// Broadcast one best-effort diagnostic message to connected cluster peers.
    pub fn broadcast_diagnostic_message(
        &self,
        message: DiagnosticMessage,
    ) -> Result<(), BroadcastDiagnosticError> {
        self.runtime.broadcast_diagnostic_message(message)
    }

    /// Drain diagnostic messages received since the previous call.
    pub fn drain_diagnostic_messages(&self) -> Vec<InboundDiagnosticMessage> {
        std::mem::take(
            &mut *self
                .diagnostic_messages
                .lock()
                .expect("diagnostic_messages lock"),
        )
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
        if self.stopped.load(Ordering::SeqCst) {
            return Err(AdmitError::Stopped);
        }
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
        sync_heartbeat_targets(
            &self.runtime.handle(),
            self.local_peer_id,
            &self.manager_peer_id,
            &self.membership,
            &self.cluster_name,
        )
        .await;

        // Gossip the updated membership to every connected peer so
        // existing members learn about the new joiner (the joiner
        // itself already has the same JSON in the `JoinResponse::Accept`
        // it just received). Fire-and-forget; per-peer errors are
        // logged inside the broadcast tasks.
        broadcast_current_membership(
            &self.runtime.handle(),
            &self.manager_peer_id,
            &self.membership,
        );

        Ok(member)
    }

    /// Open an outbound stream subscription on `peer_id` for the
    /// source-qualified log named by `request`. Thin delegator over
    /// [`NetworkRuntime::open_stream`] — the cluster handle is the
    /// daemon's natural entry point and shouldn't force consumers to
    /// reach into the runtime directly.
    ///
    /// Returns once the producer has either Accepted (typed
    /// [`StreamSubscription<T>`]) or Declined
    /// ([`OpenStreamError::Declined { reason }`]) the request. The
    /// peer must be a member of the cluster (checked by the
    /// runtime's allow-list on the producer side per the
    /// `/auki/stream/0.2.0` trust-boundary resolution 2026-05-13 —
    /// non-cluster substreams are silently dropped).
    ///
    /// `T` is the typed payload the substream carries (`CameraFrame`,
    /// `PointCloudFrame`, `JointEncodersFrame`, audio, pose, or detection);
    /// the consumer
    /// statically knows which `T` to expect per call.
    pub async fn open_stream<T>(
        &self,
        peer_id: PeerId,
        request: auki_network::stream_protocol::StreamRequest,
    ) -> Result<
        auki_network::stream_runtime::StreamSubscription<T>,
        auki_network::stream_runtime::OpenStreamError,
    >
    where
        T: prost::Message + Default + Send + 'static,
    {
        self.runtime.open_stream::<T>(peer_id, request).await
    }

    /// Build a fresh [`ParticipantInfo`] snapshot. Combines the
    /// stored daemon-side identity fields (passed at construction
    /// via [`DaemonInfo`]) with SDK-tracked dynamic fields
    /// (`session_now_ns` read from the SDK-owned [`SessionClock`],
    /// `cluster_joined_at_ns` set
    /// lazily on first non-self peer observation), `is_manager` /
    /// `manager_peer_id` from cluster state, and the local
    /// `peer_id`.
    ///
    /// Cluster peers fetch each other's copies over libp2p
    /// `/auki/info/0.0.1` — the only peer-facing identity surface
    /// (#293). Apps may also render it on their own local operator
    /// UI.
    pub fn participant_info(&self) -> ParticipantInfo {
        let manager_peer_id = self.manager_peer_id();
        let session_now_ns = self.session_clock.now_ns();

        // Lazy `cluster_joined_at_ns`: set on first observation of
        // any peer other than ourselves (per ansuz D3 the local peer
        // sets its own value, not the SDK on join_cluster's behalf —
        // a one-peer cluster shouldn't tick `cluster_joined_at_ns`).
        let cluster_joined_at_ns = {
            let mut guard = self
                .cluster_joined_at_ns
                .lock()
                .expect("cluster_joined_at_ns mutex poisoned");
            if guard.is_none() {
                let has_other = self
                    .membership
                    .lock()
                    .expect("membership lock")
                    .peers
                    .iter()
                    .any(|p| p.peer_id != self.local_peer_id);
                if has_other {
                    *guard = Some(session_now_ns);
                }
            }
            *guard
        };

        ParticipantInfo {
            app: self.daemon_info.app.clone(),
            name: self.daemon_info.name.clone(),
            session_id: self.daemon_info.session_id.clone(),
            session_clock_id: self.session_clock.clock_id().to_string(),
            session_clock_hash: self.session_clock.clock_hash(),
            session_now_ns,
            cluster_joined_at_ns,
            peer_id: self.local_peer_id,
            app_instance: self.daemon_info.app_instance.clone(),
            is_manager: manager_peer_id == self.local_peer_id,
            manager_peer_id: manager_peer_id.to_string(),
        }
    }

    /// Best current peer-clock transform estimate for an ordered
    /// local/remote clock pair, if heartbeat NTP samples have
    /// produced one. This is a read-only view over `auki-time`'s
    /// sync state; `ClusterManager` does not own sample policy.
    pub fn clock_sync_estimate(
        &self,
        local_clock_id: &str,
        remote_clock_id: &str,
    ) -> Option<ClockTransformEstimate> {
        self.clock_sync.estimate(local_clock_id, remote_clock_id)
    }

    /// Snapshot all current heartbeat-derived peer-clock transform
    /// estimates known to this manager.
    pub fn clock_sync_estimates(&self) -> Vec<ClockTransformEstimate> {
        self.clock_sync.estimates()
    }

    fn session_clock_now_ns(&self) -> i64 {
        self.session_clock.now_i64_ns()
    }

    fn domain_clock_estimate_at(
        &self,
        session_now_ns: i64,
    ) -> Result<DomainClockEstimate, DomainClockEstimateUnavailable> {
        let preferred_backing_peer_id = self.manager_peer_id().to_string();
        let session_clock_hash = self.session_clock.clock_hash();
        estimate_cluster_domain_clock(
            &self.clock_sync,
            &self.domain_clock_sources,
            &self.cluster_name,
            Some(&preferred_backing_peer_id),
            self.session_clock.clock_id(),
            &session_clock_hash,
            session_now_ns,
        )
    }

    /// Best current transform estimate from this peer's session
    /// clock into the cluster's stable domain clock.
    ///
    /// Returns an explicit unavailable reason when the domain-clock
    /// source has not been advertised yet, or when this peer has not
    /// yet measured its session clock against that source's backing
    /// clock. No wall-clock fallback is used.
    pub fn domain_clock_estimate(
        &self,
    ) -> Result<DomainClockEstimate, DomainClockEstimateUnavailable> {
        self.domain_clock_estimate_at(self.session_clock_now_ns())
    }

    /// Current local reading converted into the cluster's stable
    /// domain clock.
    ///
    /// This is the convenience form of [`Self::domain_clock_estimate`]
    /// for callers that need "domain time now" rather than the full
    /// transform estimate. It returns typed unavailable errors until
    /// the domain source and any required peer-clock transform are
    /// known. No wall-clock fallback is used.
    pub fn domain_time_now(&self) -> Result<i64, DomainTimeNowError> {
        let session_now_ns = self.session_clock_now_ns();
        let estimate = self.domain_clock_estimate_at(session_now_ns)?;
        convert_session_now_to_domain_time(&estimate, session_now_ns)
    }

    /// Fetch a cluster peer's [`ParticipantInfo`] over the
    /// `/auki/info/0.0.1` libp2p protocol. The target peer's
    /// `ClusterManager` builds its own `ParticipantInfo` from its
    /// stored state and serializes it over the wire.
    ///
    /// `peer_id` must be a current cluster member — the runtime
    /// allow-list gates the outbound substream. Returns the
    /// parsed `ParticipantInfo` ready for HTTP serialization.
    pub async fn fetch_participant_info(
        &self,
        peer_id: PeerId,
    ) -> Result<ParticipantInfo, FetchParticipantInfoError> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(FetchParticipantInfoError::Stopped);
        }
        let response = self.runtime.request_participant_info(peer_id).await?;
        let info: ParticipantInfo = serde_json::from_str(&response.participant_info_json)?;
        Ok(info)
    }

    /// Register (or replace) the application-supplied
    /// [`ResourceCatalogProvider`]. Use this for all resource
    /// types: sensor logs, pose logs, time-transform logs, etc.
    ///
    /// Inbound `/auki/resources/0.2.0` requests received before this
    /// call return an empty catalog. After this call, each request
    /// invokes the provider on the request path. The SDK stores the
    /// provider object, not a one-time catalog snapshot.
    pub fn set_resource_catalog_provider(&self, provider: Arc<dyn ResourceCatalogProvider>) {
        *self
            .resource_catalog_provider
            .lock()
            .expect("resource_catalog_provider lock") = Some(provider);
    }

    /// Register (or replace) the [`SessionHandle`] that the
    /// `/auki/resources/0.2.0` handler delegates to when a remote peer
    /// asks for the local peer's resource catalog. Call this once the
    /// `auki-session` layer has bootstrapped its session state.
    ///
    /// Inbound resource requests received before this call return an
    /// empty catalog, which is the correct "no session yet" answer.
    pub fn set_session_handle(&self, handle: Arc<dyn SessionHandle>) {
        *self.session_handle.lock().expect("session_handle lock") = Some(handle);
    }

    /// Register (or replace) the producer-local app root used to
    /// serve `/auki/registries/0.0.1` requests. The SDK reads existing
    /// registry files from this app root via `auki-registry` and
    /// returns canonical JSON entries to cluster peers.
    ///
    /// Inbound registry requests received before this call answer with
    /// `entry: None`, which means "this peer does not have that exact
    /// registry entry" from the consumer's perspective.
    pub fn set_registry_app_root(&self, app_root: impl Into<PathBuf>) {
        *self
            .registry_app_root
            .lock()
            .expect("registry_app_root lock") = Some(app_root.into());
    }

    /// Fetch a cluster peer's current generalized resource catalog
    /// over `/auki/resources/0.2.0`. This is the canonical discovery
    /// path for currently-requestable sensor, pose, time-transform, and
    /// detection logs.
    pub async fn fetch_resources_catalog(
        &self,
        peer_id: PeerId,
    ) -> Result<ResourcesResponse, FetchResourcesCatalogError> {
        let response = self.runtime.request_resources_catalog(peer_id).await?;
        Ok(response)
    }

    /// Fetch a cluster peer's generalized resource catalog with an
    /// explicit request. Set [`ResourcesRequest::variants`] to fetch only
    /// selected resource variants; leave it empty to fetch all variants.
    pub async fn fetch_resources_catalog_with(
        &self,
        peer_id: PeerId,
        request: ResourcesRequest,
    ) -> Result<ResourcesResponse, FetchResourcesCatalogError> {
        let response = self
            .runtime
            .request_resources_catalog_with(peer_id, request)
            .await?;
        Ok(response)
    }

    /// Fetch and verify a peer's `SensorRegistryEntry` by exact
    /// `(sensor_id, sensor_hash)` over `/auki/registries/0.0.1`.
    ///
    /// The SDK verifies the returned canonical JSON bytes hash to the
    /// requested hash before decoding, then checks the decoded
    /// `sensor_id` matches the requested id.
    pub async fn fetch_sensor_entry(
        &self,
        peer_id: PeerId,
        sensor_id: impl Into<String>,
        sensor_hash: impl Into<String>,
    ) -> Result<SensorRegistryEntry, FetchRegistryEntryError> {
        let id = sensor_id.into();
        let hash = sensor_hash.into();
        let envelope = self
            .fetch_registry_envelope(peer_id, RegistryKind::Sensor, id.clone(), hash.clone())
            .await?;
        let entry: SensorRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        if entry.sensor_id != id {
            return Err(FetchRegistryEntryError::InvalidEnvelope(format!(
                "decoded sensor_id mismatch: expected {:?}, found {:?}",
                id, entry.sensor_id
            )));
        }
        Ok(entry)
    }

    /// Fetch and verify a peer's `ClockRegistryEntry` by exact
    /// `(clock_id, clock_hash)` over `/auki/registries/0.0.1`.
    pub async fn fetch_clock_entry(
        &self,
        peer_id: PeerId,
        clock_id: impl Into<String>,
        clock_hash: impl Into<String>,
    ) -> Result<ClockRegistryEntry, FetchRegistryEntryError> {
        let id = clock_id.into();
        let hash = clock_hash.into();
        let envelope = self
            .fetch_registry_envelope(peer_id, RegistryKind::Clock, id.clone(), hash.clone())
            .await?;
        let entry: ClockRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        if entry.clock_id != id {
            return Err(FetchRegistryEntryError::InvalidEnvelope(format!(
                "decoded clock_id mismatch: expected {:?}, found {:?}",
                id, entry.clock_id
            )));
        }
        Ok(entry)
    }

    /// Fetch and verify a peer's `FrameRegistryEntry` by exact
    /// `(frame_id, frame_hash)` over `/auki/registries/0.0.1`.
    pub async fn fetch_frame_entry(
        &self,
        peer_id: PeerId,
        frame_id: impl Into<String>,
        frame_hash: impl Into<String>,
    ) -> Result<FrameRegistryEntry, FetchRegistryEntryError> {
        let id = frame_id.into();
        let hash = frame_hash.into();
        let envelope = self
            .fetch_registry_envelope(peer_id, RegistryKind::Frame, id.clone(), hash.clone())
            .await?;
        let entry: FrameRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        if entry.frame_id != id {
            return Err(FetchRegistryEntryError::InvalidEnvelope(format!(
                "decoded frame_id mismatch: expected {:?}, found {:?}",
                id, entry.frame_id
            )));
        }
        Ok(entry)
    }

    /// Fetch and verify a peer's `DetectorRegistryEntry` by exact
    /// `(detector_id, detector_hash)` over `/auki/registries/0.0.1`.
    /// Cuba T4 — closes Park-side detector enumeration without an HTTP
    /// shim. Symmetric with `fetch_sensor_entry`/`fetch_frame_entry`.
    pub async fn fetch_detector_entry(
        &self,
        peer_id: PeerId,
        detector_id: impl Into<String>,
        detector_hash: impl Into<String>,
    ) -> Result<DetectorRegistryEntry, FetchRegistryEntryError> {
        let id = detector_id.into();
        let hash = detector_hash.into();
        let envelope = self
            .fetch_registry_envelope(peer_id, RegistryKind::Detector, id.clone(), hash.clone())
            .await?;
        let entry: DetectorRegistryEntry = serde_json::from_str(&envelope.canonical_json)?;
        if entry.detector_id != id {
            return Err(FetchRegistryEntryError::InvalidEnvelope(format!(
                "decoded detector_id mismatch: expected {:?}, found {:?}",
                id, entry.detector_id
            )));
        }
        Ok(entry)
    }

    async fn fetch_registry_envelope(
        &self,
        peer_id: PeerId,
        kind: RegistryKind,
        id: String,
        hash: String,
    ) -> Result<RegistryEntryEnvelope, FetchRegistryEntryError> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(FetchRegistryEntryError::Stopped);
        }
        let response = self
            .runtime
            .request_registry_entry(
                peer_id,
                RegistryRequest {
                    kind,
                    id: id.clone(),
                    hash: hash.clone(),
                },
            )
            .await?;
        let Some(envelope) = response.entry else {
            return Err(FetchRegistryEntryError::NotFound { kind, id, hash });
        };
        verify_registry_envelope(&envelope, kind, &id, &hash)?;
        Ok(envelope)
    }

    /// Join an existing cluster by talking to its Manager. Lists
    /// Discovery, finds the entry for `cluster_name`, opens a
    /// libp2p `/auki/join/0.0.1` substream to the Manager, sends a
    /// `JoinRequest`, parses the Manager's `JoinResponse`, and
    /// returns a `ClusterManager` populated with the full membership
    /// the Manager gossiped.
    ///
    /// Operator-intent primitive — call this when the operator clicked
    /// "Join <name>" in a UI. For headless daemons, use
    /// [`Self::bootstrap`] with a [`ClusterTarget`] policy instead.
    ///
    /// The local peer is NOT the Manager — `is_manager()` returns
    /// `false`, `manager_peer_id()` points at whichever peer is
    /// recorded in Discovery's directory at the time of the call.
    /// No Discovery-liveness-check task is spawned (only Managers push
    /// liveness checks); the in-process successor election + Manager
    /// rotation machinery starts the liveness-check task when this
    /// peer is later promoted.
    pub async fn join_cluster(
        cluster_name: impl Into<String>,
        local_identity: PeerIdentity,
        local_multiaddrs: Vec<Multiaddr>,
        discovery_url: impl Into<String>,
        swarm: Swarm<Behaviour>,
        stream_provider: StreamProvider,
        daemon_info: DaemonInfo,
    ) -> Result<Self, JoinClusterError> {
        let discovery = DiscoveryClient::new(discovery_url.into());
        let cluster_name = cluster_name.into();
        let local_peer_id = local_identity.peer_id();
        let session_clock = SessionClock::new(
            local_peer_id.to_string(),
            daemon_info.session_id.clone(),
            "monotonic",
        );
        let cluster_joined_at_ns: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));
        let domain_clock_sources = domain_clock_source_store();
        let advertised_domain_clock_source: Arc<Mutex<Option<HeartbeatDomainClock>>> =
            Arc::new(Mutex::new(None));

        // 1. Look up the cluster in Discovery's directory.
        let clusters = discovery.list_clusters().await?;
        let entry = clusters
            .into_iter()
            .find(|c| c.name == cluster_name)
            .ok_or_else(|| JoinClusterError::NotFound(cluster_name.clone()))?;
        let manager_peer = entry.manager_peer_id;
        let manager_multiaddrs = entry.manager_multiaddrs.clone();
        let relay_multiaddrs = entry.relay_multiaddrs.clone();

        // 2. Spawn the runtime with the Manager pre-allowed (so we
        //    can dial it for the join handshake). The allow-list
        //    expands once the Manager gossips back the full
        //    membership.
        let (
            runtime,
            join_events_rx,
            liveness_rx,
            membership_events_rx,
            info_events_rx,
            resources_events_rx,
            registry_events_rx,
            diagnostic_events_rx,
        ) = NetworkRuntime::spawn(
            swarm,
            vec![AllowedPeer {
                peer_id: manager_peer,
                multiaddrs: manager_multiaddrs.clone(),
            }],
            stream_provider,
            heartbeat_timestamp_source(
                session_clock.clone(),
                advertised_domain_clock_source.clone(),
            ),
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
                return Err(JoinClusterError::SendJoin(SendJoinRequestError::Timeout(
                    Duration::from_secs(10),
                )));
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
        runtime
            .set_heartbeat_targets(vec![])
            .await
            .map_err(|e| JoinClusterError::Runtime(SpawnError::NoTokioRuntime).into_with(e))?;

        let membership = Arc::new(Mutex::new(membership));
        let manager_peer_id = Arc::new(Mutex::new(manager_peer));
        let clock_sync = ClockSyncHandle::default();

        // 6. Drain inbound join events. As a non-Manager our handler
        //    always rejects with "not the manager"; once an election
        //    promotes us the same handler starts admitting (it reads
        //    `manager_peer_id` per call).
        let join_handler_task = Mutex::new(Some(spawn_join_handler(
            join_events_rx,
            cluster_name.clone(),
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
        )));

        // 7. Drain heartbeat carrier events and run the domain-side
        //    liveness timer. On Manager death, run the cluster-
        //    internal election; if we win, become the new Manager
        //    (update state, rotate Discovery, start the liveness
        //    check tick).
        let liveness_check_task: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::new(Mutex::new(None));
        let liveness_handler_task = Mutex::new(Some(spawn_liveness_handler(
            liveness_rx,
            cluster_name.clone(),
            local_peer_id,
            local_multiaddrs.clone(),
            relay_multiaddrs.clone(),
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
            discovery.clone(),
            liveness_check_task.clone(),
            clock_sync.clone(),
            domain_clock_sources.clone(),
            advertised_domain_clock_source.clone(),
            session_clock.clone(),
        )));

        // 8. Drain inbound /auki/membership/0.0.1 gossip events.
        //    The Manager pushes updates here when peers join / leave
        //    after our own join; we apply them last-write-wins.
        let membership_handler_task = Mutex::new(Some(spawn_membership_handler(
            membership_events_rx,
            cluster_name.clone(),
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            runtime.handle(),
        )));

        // 9. Drain inbound /auki/info/0.0.1 requests. Build a fresh
        //    `ParticipantInfo` from stored daemon_info + dynamic SDK
        //    state on each request and reply.
        let info_handler_task = Mutex::new(Some(spawn_info_handler(
            info_events_rx,
            local_peer_id,
            manager_peer_id.clone(),
            membership.clone(),
            daemon_info.clone(),
            session_clock.clone(),
            cluster_joined_at_ns.clone(),
        )));

        let registry_app_root: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

        // 10. Drain inbound /auki/resources/0.2.0 requests. The
        //     resources handler uses an app-supplied provider when
        //     present, otherwise falls back to a SessionHandle.
        let resource_catalog_provider: Arc<Mutex<Option<Arc<dyn ResourceCatalogProvider>>>> =
            Arc::new(Mutex::new(None));
        let session_handle: Arc<Mutex<Option<Arc<dyn SessionHandle>>>> = Arc::new(Mutex::new(None));
        let resources_handler_task = Mutex::new(Some(spawn_resources_handler(
            resources_events_rx,
            resource_catalog_provider.clone(),
            session_handle.clone(),
        )));

        // 11. Drain inbound /auki/registries/0.0.1 requests. Read
        //     the exact registry entry from app-root storage if the
        //     daemon has registered an app root.
        let registry_handler_task = Mutex::new(Some(spawn_registry_handler(
            registry_events_rx,
            registry_app_root.clone(),
            local_peer_id.to_string(),
        )));
        let diagnostic_messages = Arc::new(Mutex::new(Vec::new()));
        let diagnostic_handler_task = Mutex::new(Some(spawn_diagnostic_handler(
            diagnostic_events_rx,
            diagnostic_messages.clone(),
        )));

        Ok(Self {
            cluster_name,
            local_peer_id,
            membership,
            manager_peer_id,
            runtime,
            discovery,
            local_multiaddrs,
            relay_multiaddrs,
            daemon_info,
            session_clock,
            cluster_joined_at_ns,
            clock_sync,
            domain_clock_sources,
            liveness_check_task,
            join_handler_task,
            liveness_handler_task,
            membership_handler_task,
            info_handler_task,
            resources_handler_task,
            registry_handler_task,
            diagnostic_handler_task,
            diagnostic_messages,
            resource_catalog_provider,
            session_handle,
            registry_app_root,
            stopped: AtomicBool::new(false),
        })
    }

    /// Shutdown — cancels all background tasks, deregisters the
    /// cluster from Discovery **only if we're the last member**,
    /// and shuts down the runtime.
    ///
    /// **Manager handoff on graceful exit.** Per the Hagall quest
    /// design ("graceful and ungraceful Manager exits are the same
    /// code path — peers detect the loss + run the election +
    /// rotate"): if the Manager calls `shutdown` while other peers
    /// are in the cluster, we do NOT deregister. The surviving peers
    /// detect our libp2p disconnection, run the cluster-internal
    /// election, and the winner calls `rotate_manager` on Discovery.
    /// Deregistering on the way out would 404 the winner's rotation
    /// and orphan the cluster. Only when we are the last member
    /// (`membership.peers.len() <= 1`) do we deregister, since there
    /// is no successor to take over.
    ///
    /// Callable from `&self` so daemon callers holding the manager
    /// behind an `Arc` (Park's stream-consumer pattern, Boosterapp's
    /// stream-provider closure) can shut down without first uniquely
    /// owning the handle. Idempotent: subsequent calls find the
    /// `stopped` flag set, observe the empty `Mutex<Option<_>>`s,
    /// and return `Ok(())` without re-issuing the Discovery
    /// DELETE.
    ///
    /// After this returns, pub I/O methods on this handle
    /// (`admit_peer`, `fetch_participant_info`) fast-fail with a
    /// `Stopped` error variant. Snapshot accessors continue to
    /// return their last-observed state.
    pub async fn shutdown(&self) -> Result<(), DiscoveryError> {
        // 0. Claim the shutdown — the AtomicBool exchange picks one
        //    caller as the deregistration owner; concurrent / repeat
        //    callers observe `true` and short-circuit.
        if self.stopped.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        // 1. Cancel background tasks FIRST so we stop touching
        //    Discovery / membership between teardown steps.
        if let Some(task) = self
            .liveness_check_task
            .lock()
            .expect("liveness_check_task lock")
            .take()
        {
            task.abort();
        }
        if let Some(task) = self
            .join_handler_task
            .lock()
            .expect("join_handler_task lock")
            .take()
        {
            task.abort();
        }
        if let Some(task) = self
            .liveness_handler_task
            .lock()
            .expect("liveness_handler_task lock")
            .take()
        {
            task.abort();
        }
        if let Some(task) = self
            .membership_handler_task
            .lock()
            .expect("membership_handler_task lock")
            .take()
        {
            task.abort();
        }
        if let Some(task) = self
            .info_handler_task
            .lock()
            .expect("info_handler_task lock")
            .take()
        {
            task.abort();
        }
        if let Some(task) = self
            .resources_handler_task
            .lock()
            .expect("resources_handler_task lock")
            .take()
        {
            task.abort();
        }
        if let Some(task) = self
            .registry_handler_task
            .lock()
            .expect("registry_handler_task lock")
            .take()
        {
            task.abort();
        }
        if let Some(task) = self
            .diagnostic_handler_task
            .lock()
            .expect("diagnostic_handler_task lock")
            .take()
        {
            task.abort();
        }

        // 2. Deregister from Discovery only if we're the last
        //    member. Otherwise leave the cluster alive for the
        //    survivors' election + handoff (see fn doc).
        let was_manager =
            *self.manager_peer_id.lock().expect("manager_peer_id lock") == self.local_peer_id;
        let am_last = self.membership.lock().expect("membership lock").peers.len() <= 1;
        let result = if was_manager && am_last {
            self.discovery.deregister(self.cluster_name.clone()).await
        } else {
            Ok(())
        };

        // 3. Shut down the runtime regardless of Discovery's result.
        //    `NetworkRuntime::shutdown` is `&self` + idempotent.
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

fn heartbeat_targets_for(
    local_peer_id: PeerId,
    manager_peer_id: PeerId,
    membership: &ClusterMembership,
) -> Vec<PeerId> {
    if manager_peer_id != local_peer_id {
        return Vec::new();
    }
    membership
        .peers
        .iter()
        .map(|p| p.peer_id)
        .filter(|pid| *pid != local_peer_id)
        .collect()
}

fn heartbeat_watchlist_for(
    local_peer_id: PeerId,
    manager_peer_id: PeerId,
    membership: &ClusterMembership,
) -> HashSet<PeerId> {
    if manager_peer_id == local_peer_id {
        membership
            .peers
            .iter()
            .map(|p| p.peer_id)
            .filter(|pid| *pid != local_peer_id)
            .collect()
    } else {
        membership
            .peers
            .iter()
            .any(|p| p.peer_id == manager_peer_id)
            .then_some(manager_peer_id)
            .into_iter()
            .collect()
    }
}

async fn sync_heartbeat_targets(
    runtime: &auki_network::NetworkRuntimeHandle,
    local_peer_id: PeerId,
    manager_peer_id: &Arc<Mutex<PeerId>>,
    membership: &Arc<Mutex<ClusterMembership>>,
    cluster_name: &str,
) {
    let manager = *manager_peer_id.lock().expect("manager_peer_id lock");
    let targets = {
        let m = membership.lock().expect("membership lock");
        heartbeat_targets_for(local_peer_id, manager, &m)
    };
    if let Err(e) = runtime.set_heartbeat_targets(targets).await {
        eprintln!("auki-domain: sync heartbeat targets failed for cluster {cluster_name:?}: {e}");
    }
}

/// How long a rejoin waits for the runtime auto-dial (500ms tick) to
/// connect the target Manager before giving up; the caller retries on
/// a later heartbeat timeout.
const REJOIN_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors from [`rejoin_via_manager`].
// TODO(#295): wired in by the arbiter-tick / loss-handler tasks.
#[allow(dead_code)]
#[derive(Debug, Error)]
enum RejoinError {
    #[error("allow-list update: {0}")]
    Runtime(#[from] auki_network::network_runtime::UpdateError),
    #[error("manager not connected within {0:?}")]
    Connect(Duration),
    #[error("join round-trip: {0}")]
    Send(#[from] SendJoinRequestError),
    #[error("manager rejected rejoin: {0}")]
    Rejected(String),
    #[error("invalid membership JSON in rejoin accept: {0}")]
    InvalidMembership(#[from] serde_json::Error),
}

/// Ask `manager` to (re-)admit the local peer over `/auki/join/0.0.1`
/// and adopt the membership document it returns. Used by a deferring
/// follower whose heartbeat link to the Manager lapsed (the Manager
/// may have evicted it) and by a displaced ex-Manager adopting the
/// Discovery-named winner.
// TODO(#295): wired in by the arbiter-tick / loss-handler tasks.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
async fn rejoin_via_manager(
    cluster_name: &str,
    local_peer_id: PeerId,
    local_multiaddrs: &[Multiaddr],
    manager: PeerId,
    manager_multiaddrs: Vec<Multiaddr>,
    manager_peer_id_slot: &Arc<Mutex<PeerId>>,
    membership: &Arc<Mutex<ClusterMembership>>,
    runtime: &auki_network::NetworkRuntimeHandle,
) -> Result<(), RejoinError> {
    // 1. Make the Manager dialable again: current membership ∪ target.
    //    (An evicting Manager dropped us from ITS allow-list; ours may
    //    have dropped IT after an election. The union restores the dial;
    //    the runtime auto-dial tick picks it up within 500ms.)
    let mut allow: Vec<AllowedPeer> = {
        let m = membership.lock().expect("membership lock");
        m.peers
            .iter()
            .filter(|p| p.peer_id != local_peer_id)
            .map(|p| AllowedPeer {
                peer_id: p.peer_id,
                multiaddrs: p.multiaddrs.clone(),
            })
            .collect()
    };
    if let Some(slot) = allow.iter_mut().find(|p| p.peer_id == manager) {
        for a in manager_multiaddrs {
            push_unique_multiaddr(&mut slot.multiaddrs, a);
        }
    } else {
        allow.push(AllowedPeer {
            peer_id: manager,
            multiaddrs: manager_multiaddrs,
        });
    }
    runtime.set_allowed_peers(allow).await?;

    // 2. Wait for the auto-dial to connect it (mirrors join_cluster's
    //    bootstrap wait, with a shorter deadline — the caller retries).
    let deadline = Instant::now() + REJOIN_CONNECT_TIMEOUT;
    while !runtime.connected_peers().contains(&manager) {
        if Instant::now() >= deadline {
            return Err(RejoinError::Connect(REJOIN_CONNECT_TIMEOUT));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 3. Join round-trip.
    let response = runtime
        .send_join_request(
            manager,
            JoinRequest {
                multiaddrs: local_multiaddrs.to_vec(),
            },
        )
        .await?;
    let membership_json = match response {
        JoinResponse::Accept {
            membership_json, ..
        } => membership_json,
        JoinResponse::Reject { reason } => return Err(RejoinError::Rejected(reason)),
    };
    let parsed: ClusterMembership = serde_json::from_str(&membership_json)?;

    // 4. Adopt: document, Manager view, allow-list, heartbeat targets.
    let new_allow: Vec<AllowedPeer> = parsed
        .peers
        .iter()
        .filter(|p| p.peer_id != local_peer_id)
        .map(|p| AllowedPeer {
            peer_id: p.peer_id,
            multiaddrs: p.multiaddrs.clone(),
        })
        .collect();
    *membership.lock().expect("membership lock") = parsed;
    *manager_peer_id_slot.lock().expect("manager_peer_id lock") = manager;
    runtime.set_allowed_peers(new_allow).await?;
    sync_heartbeat_targets(
        runtime,
        local_peer_id,
        manager_peer_id_slot,
        membership,
        cluster_name,
    )
    .await;
    Ok(())
}

fn reconcile_heartbeat_watchlist(
    last_heartbeat_at: &mut HashMap<PeerId, Instant>,
    lost_already: &mut HashSet<PeerId>,
    local_peer_id: PeerId,
    manager_peer_id: PeerId,
    membership: &ClusterMembership,
) {
    let now = Instant::now();
    let watchlist = heartbeat_watchlist_for(local_peer_id, manager_peer_id, membership);
    last_heartbeat_at.retain(|pid, _| watchlist.contains(pid));
    lost_already.retain(|pid| watchlist.contains(pid));
    for pid in watchlist {
        last_heartbeat_at.entry(pid).or_insert(now);
    }
}

fn note_watched_peer_alive(
    watched: &HashSet<PeerId>,
    last_heartbeat_at: &mut HashMap<PeerId, Instant>,
    lost_already: &mut HashSet<PeerId>,
    peer_id: PeerId,
) {
    if watched.contains(&peer_id) {
        last_heartbeat_at.insert(peer_id, Instant::now());
        lost_already.remove(&peer_id);
    }
}

fn note_watched_transport_closed(
    watched: &HashSet<PeerId>,
    last_heartbeat_at: &mut HashMap<PeerId, Instant>,
    peer_id: PeerId,
) {
    if watched.contains(&peer_id) {
        // Raw libp2p connection and heartbeat-carrier closure are
        // transport symptoms, not semantic peer death. Keep the
        // existing last-seen time and let the heartbeat timeout
        // decide. A reconnect or fresh heartbeat before the timeout
        // refreshes the timestamp via `note_watched_peer_alive`.
        last_heartbeat_at
            .entry(peer_id)
            .or_insert_with(Instant::now);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagerRotationDiscoveryRequest {
    DirectOnly {
        manager_multiaddrs: Vec<Multiaddr>,
    },
    WithRelays {
        manager_multiaddrs: Vec<Multiaddr>,
        relay_multiaddrs: Vec<Multiaddr>,
    },
}

fn manager_rotation_discovery_request(
    manager_multiaddrs: &[Multiaddr],
    relay_multiaddrs: &[Multiaddr],
) -> ManagerRotationDiscoveryRequest {
    if relay_multiaddrs.is_empty() {
        ManagerRotationDiscoveryRequest::DirectOnly {
            manager_multiaddrs: manager_multiaddrs.to_vec(),
        }
    } else {
        ManagerRotationDiscoveryRequest::WithRelays {
            manager_multiaddrs: manager_multiaddrs.to_vec(),
            relay_multiaddrs: relay_multiaddrs.to_vec(),
        }
    }
}

/// `true` when Discovery answered "no such cluster row" — the row was
/// swept (or Discovery restarted). Callers re-create instead of rotate.
fn discovery_row_missing(e: &DiscoveryClientError) -> bool {
    matches!(e, DiscoveryClientError::Status { status: 404, .. })
}

/// Why a Manager claim did not commit at Discovery.
#[derive(Debug)]
// TODO(#295): wired in by the arbiter-tick / loss-handler tasks.
#[allow(dead_code)]
enum RegisterAsManagerError {
    /// Another peer holds the row (lost the re-create race). The
    /// caller must NOT promote; it should follow the row instead.
    Displaced,
    /// Discovery unreachable or errored; retry on a later timeout.
    Discovery(DiscoveryClientError),
}

/// Commit a Manager claim at Discovery: rotate the existing row, or
/// re-create it when Discovery already swept it (hardening-plan Task
/// 6). `Ok(())` is the commit signal — only then may the caller
/// mutate local Manager state.
// TODO(#295): wired in by the arbiter-tick / loss-handler tasks.
#[allow(dead_code)]
async fn register_as_manager(
    discovery: &DiscoveryClient,
    cluster_name: &str,
    local_peer_id: PeerId,
    local_multiaddrs: &[Multiaddr],
    relay_multiaddrs: &[Multiaddr],
) -> Result<(), RegisterAsManagerError> {
    let rotate_result = match manager_rotation_discovery_request(local_multiaddrs, relay_multiaddrs)
    {
        ManagerRotationDiscoveryRequest::DirectOnly { manager_multiaddrs } => {
            discovery
                .rotate_manager(cluster_name.to_string(), local_peer_id, manager_multiaddrs)
                .await
        }
        ManagerRotationDiscoveryRequest::WithRelays {
            manager_multiaddrs,
            relay_multiaddrs,
        } => {
            discovery
                .rotate_manager_with_relay_multiaddrs(
                    cluster_name.to_string(),
                    local_peer_id,
                    manager_multiaddrs,
                    relay_multiaddrs,
                )
                .await
        }
    };
    let err = match rotate_result {
        Ok(_) => return Ok(()),
        Err(e) => e,
    };
    if !discovery_row_missing(&err) {
        return Err(RegisterAsManagerError::Discovery(err));
    }
    let create_result = if relay_multiaddrs.is_empty() {
        discovery
            .create_cluster(
                cluster_name.to_string(),
                local_peer_id,
                local_multiaddrs.to_vec(),
            )
            .await
    } else {
        discovery
            .create_cluster_with_relay_multiaddrs(
                cluster_name.to_string(),
                local_peer_id,
                local_multiaddrs.to_vec(),
                relay_multiaddrs.to_vec(),
            )
            .await
    };
    match create_result {
        Ok(CreateClusterOutcome::Created(_)) => Ok(()),
        Ok(CreateClusterOutcome::AlreadyExists) => Err(RegisterAsManagerError::Displaced),
        Err(e) => Err(RegisterAsManagerError::Discovery(e)),
    }
}

async fn reserve_manager_relay_multiaddr(
    swarm: &mut Swarm<Behaviour>,
    manager_multiaddrs: &mut Vec<Multiaddr>,
    relay_multiaddrs: &mut Vec<Multiaddr>,
    reservation: &ManagerRelayReservation,
) -> Result<(), auki_network::swarm::RelayReservationError> {
    let circuit_addr = auki_network::swarm::reserve_relay_circuit_addr_with_advertised_addr(
        swarm,
        reservation.relay_dial_multiaddr.clone(),
        reservation.relay_advertise_multiaddr.clone(),
        reservation.timeout,
    )
    .await?;
    push_unique_multiaddr(manager_multiaddrs, circuit_addr);
    push_unique_multiaddr(
        relay_multiaddrs,
        reservation.relay_advertise_multiaddr.clone(),
    );
    Ok(())
}

fn push_unique_multiaddr(addrs: &mut Vec<Multiaddr>, addr: Multiaddr) {
    if !addrs.contains(&addr) {
        addrs.push(addr);
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_domain_peer_lost(
    cluster_name: &str,
    local_peer_id: PeerId,
    local_multiaddrs: &[Multiaddr],
    relay_multiaddrs: &[Multiaddr],
    session_clock: &SessionClock,
    manager_peer_id: &Arc<Mutex<PeerId>>,
    membership: &Arc<Mutex<ClusterMembership>>,
    runtime: &auki_network::NetworkRuntimeHandle,
    discovery: &Arc<DiscoveryClient>,
    liveness_check_task: &Arc<Mutex<Option<JoinHandle<()>>>>,
    advertised_domain_clock_source: &Arc<Mutex<Option<HeartbeatDomainClock>>>,
    clock_sync: &ClockSyncHandle,
    domain_clock_sources: &DomainClockSources,
    handled_manager_losses: &mut HashSet<PeerId>,
    lost_pid: PeerId,
) {
    let current_manager = *manager_peer_id.lock().expect("manager_peer_id lock");
    let am_manager = current_manager == local_peer_id;

    if !am_manager && lost_pid == current_manager {
        if !handled_manager_losses.insert(lost_pid) {
            return;
        }

        // For other reachable peers, give connection teardown a
        // brief moment to settle. The heartbeat-lost peer is still
        // excluded below even if the transport connected set lags.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let connected = runtime.connected_peers();
        let membership_snapshot = membership.lock().expect("membership lock").clone();
        let winner = elect_successor_excluding_lost(
            &membership_snapshot,
            local_peer_id,
            &connected,
            lost_pid,
        );
        if winner == Some(local_peer_id) {
            // Become Manager.
            *manager_peer_id.lock().expect("manager_peer_id lock") = local_peer_id;

            let local_clock_now_ns = session_clock.now_i64_ns();
            if let Err(e) = advertise_promoted_domain_clock_source(
                advertised_domain_clock_source,
                clock_sync,
                domain_clock_sources,
                cluster_name,
                local_peer_id,
                session_clock,
                local_clock_now_ns,
            ) {
                eprintln!(
                    "auki-domain: cluster {cluster_name:?}: promoted Manager \
                    {local_peer_id} cannot advertise domain clock yet: {e}"
                );
            }

            // Tell Discovery about the rotation.
            let rotate_result =
                match manager_rotation_discovery_request(local_multiaddrs, relay_multiaddrs) {
                    ManagerRotationDiscoveryRequest::DirectOnly { manager_multiaddrs } => {
                        discovery
                            .rotate_manager(
                                cluster_name.to_string(),
                                local_peer_id,
                                manager_multiaddrs,
                            )
                            .await
                    }
                    ManagerRotationDiscoveryRequest::WithRelays {
                        manager_multiaddrs,
                        relay_multiaddrs,
                    } => {
                        discovery
                            .rotate_manager_with_relay_multiaddrs(
                                cluster_name.to_string(),
                                local_peer_id,
                                manager_multiaddrs,
                                relay_multiaddrs,
                            )
                            .await
                    }
                };
            if let Err(e) = rotate_result {
                eprintln!(
                    "auki-domain: rotate_manager failed for cluster \
                    {cluster_name:?}: {e}"
                );
            }

            // Start the Manager-side Discovery liveness-check tick.
            let new_tick = spawn_manager_liveness_check(
                discovery.clone(),
                cluster_name.to_string(),
                membership.clone(),
            );
            let prev = liveness_check_task
                .lock()
                .expect("liveness_check_task lock")
                .replace(new_tick);
            if let Some(p) = prev {
                p.abort();
            }

            // Evict the dead Manager from membership + push the
            // updated allow-list. (We won the election, so we own
            // membership now.)
            let new_allow_list = {
                let mut m = membership.lock().expect("membership lock");
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
            if let Err(e) = runtime.set_allowed_peers(new_allow_list).await {
                eprintln!(
                    "auki-domain: post-election set_allowed_peers \
                    failed for {cluster_name:?}: {e}"
                );
            }
            sync_heartbeat_targets(
                runtime,
                local_peer_id,
                manager_peer_id,
                membership,
                cluster_name,
            )
            .await;

            // Gossip the post-handoff view so survivors converge on
            // the new Manager identity + the post-eviction
            // membership.
            broadcast_current_membership(runtime, manager_peer_id, membership);
            eprintln!(
                "auki-domain: cluster {cluster_name:?}: local peer \
                {local_peer_id} promoted to Manager after detecting Lost {lost_pid}"
            );
        } else if let Some(new_manager) = winner {
            // Someone else (earlier-joined, still reachable) wins.
            // Update the local view and wait for them to register
            // with Discovery.
            *manager_peer_id.lock().expect("manager_peer_id lock") = new_manager;
            sync_heartbeat_targets(
                runtime,
                local_peer_id,
                manager_peer_id,
                membership,
                cluster_name,
            )
            .await;
        }
    } else if am_manager {
        // We're the Manager and a peer died. Evict from membership +
        // push the updated allow-list.
        let new_allow_list = {
            let mut m = membership.lock().expect("membership lock");
            let before = m.peers.len();
            m.peers.retain(|p| p.peer_id != lost_pid);
            if m.peers.len() == before {
                return;
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
            eprintln!("auki-domain: Manager evict post-Lost set_allowed_peers failed: {e}");
        }
        sync_heartbeat_targets(
            runtime,
            local_peer_id,
            manager_peer_id,
            membership,
            cluster_name,
        )
        .await;
        // Gossip the shrunken membership so remaining peers also
        // evict the dead one.
        broadcast_current_membership(runtime, manager_peer_id, membership);
    }
}

/// Drain heartbeat-carrier events from the runtime and maintain the
/// domain-owned heartbeat watchlist. Carrier frames refresh
/// per-peer timestamps; carrier closure or an expired timestamp is
/// interpreted here as a semantic peer loss.
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
/// liveness-check tick.
///
/// **Reachability** is approximated as "in the runtime's
/// `connected_peers()` set OR equal to local_peer_id," with the
/// heartbeat-lost peer explicitly excluded from the handoff
/// election. When the Manager dies, the local peer is always
/// "reachable to itself"; the other reachable peers are those still
/// libp2p-connected via the runtime. The earliest peer with a
/// join_ts_ns less than the local peer's own that's also reachable
/// wins; if none such exists, the local peer wins.
#[allow(clippy::too_many_arguments)]
fn spawn_liveness_handler(
    mut rx: mpsc::Receiver<PeerLivenessEvent>,
    cluster_name: String,
    local_peer_id: PeerId,
    local_multiaddrs: Vec<Multiaddr>,
    relay_multiaddrs: Vec<Multiaddr>,
    manager_peer_id: Arc<Mutex<PeerId>>,
    membership: Arc<Mutex<ClusterMembership>>,
    runtime: auki_network::NetworkRuntimeHandle,
    discovery: Arc<DiscoveryClient>,
    liveness_check_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    clock_sync: ClockSyncHandle,
    domain_clock_sources: DomainClockSources,
    advertised_domain_clock_source: Arc<Mutex<Option<HeartbeatDomainClock>>>,
    session_clock: SessionClock,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_heartbeat_at: HashMap<PeerId, Instant> = HashMap::new();
        let mut lost_already: HashSet<PeerId> = HashSet::new();
        let mut handled_manager_losses: HashSet<PeerId> = HashSet::new();
        let mut tick = tokio::time::interval(HEARTBEAT_TIMEOUT / 2);

        loop {
            let manager = *manager_peer_id.lock().expect("manager_peer_id lock");
            let membership_snapshot = membership.lock().expect("membership lock").clone();
            reconcile_heartbeat_watchlist(
                &mut last_heartbeat_at,
                &mut lost_already,
                local_peer_id,
                manager,
                &membership_snapshot,
            );

            tokio::select! {
                biased;

                evt = rx.recv() => {
                    let Some(evt) = evt else { break; };
                    let watched = {
                        let manager = *manager_peer_id.lock().expect("manager_peer_id lock");
                        let m = membership.lock().expect("membership lock");
                        heartbeat_watchlist_for(local_peer_id, manager, &m)
                    };
                    match evt {
                        PeerLivenessEvent::Connected { peer_id } => {
                            note_watched_peer_alive(
                                &watched,
                                &mut last_heartbeat_at,
                                &mut lost_already,
                                peer_id,
                            );
                        }
                        PeerLivenessEvent::HeartbeatReceived { peer_id, observation } => {
                            note_watched_peer_alive(
                                &watched,
                                &mut last_heartbeat_at,
                                &mut lost_already,
                                peer_id,
                            );
                            observe_heartbeat_domain_clock_source(
                                &domain_clock_sources,
                                &cluster_name,
                                peer_id,
                                observation.heartbeat.domain_clock,
                            );
                        }
                        PeerLivenessEvent::Disconnected { peer_id }
                        | PeerLivenessEvent::HeartbeatStreamClosed { peer_id } => {
                            note_watched_transport_closed(
                                &watched,
                                &mut last_heartbeat_at,
                                peer_id,
                            );
                        }
                        PeerLivenessEvent::HeartbeatNtpSampleObserved { observation, .. } => {
                            observe_heartbeat_ntp_sample(&clock_sync, observation);
                        }
                    }
                }

                _ = tick.tick() => {
                    let now = Instant::now();
                    let timed_out: Vec<PeerId> = last_heartbeat_at
                        .iter()
                        .filter_map(|(pid, ts)| {
                            (now.duration_since(*ts) > HEARTBEAT_TIMEOUT
                                && !lost_already.contains(pid))
                                .then_some(*pid)
                        })
                        .collect();
                    for peer_id in timed_out {
                        lost_already.insert(peer_id);
                        handle_domain_peer_lost(
                            &cluster_name,
                            local_peer_id,
                            &local_multiaddrs,
                            &relay_multiaddrs,
                            &session_clock,
                            &manager_peer_id,
                            &membership,
                            &runtime,
                            &discovery,
                            &liveness_check_task,
                            &advertised_domain_clock_source,
                            &clock_sync,
                            &domain_clock_sources,
                            &mut handled_manager_losses,
                            peer_id,
                        )
                        .await;
                    }
                }
            }
        }
    })
}

fn observe_heartbeat_ntp_sample(
    clock_sync: &ClockSyncHandle,
    observation: HeartbeatNtpSampleObservation,
) -> Option<ClockTransformEstimate> {
    let HeartbeatNtpSampleObservation {
        peer_id: _,
        local_clock_id,
        local_clock_hash,
        remote_clock_id,
        remote_clock_hash,
        sample,
    } = observation;
    clock_sync.observe(ClockSyncObservation::new(
        local_clock_id,
        local_clock_hash,
        remote_clock_id,
        remote_clock_hash,
        sample,
    ))
}

/// Spawn a task that drains inbound `/auki/membership/0.0.1` gossip
/// events from `rx`, applies each update last-write-wins to the
/// local `membership`, and pushes the recomputed allow-list to the
/// `runtime`. Lives for the lifetime of the `ClusterManager`;
/// cancelled on `shutdown`.
///
/// Manager gossip carries the authoring Manager's peer id. Receivers
/// apply it only when the sender is the claimed Manager and the
/// claimed Manager exists in the membership snapshot; this gives
/// post-handoff broadcasts a convergence signal without letting an
/// arbitrary member claim the role for another peer.
fn spawn_membership_handler(
    mut rx: mpsc::Receiver<MembershipEvent>,
    cluster_name: String,
    local_peer_id: PeerId,
    manager_peer_id: Arc<Mutex<PeerId>>,
    membership: Arc<Mutex<ClusterMembership>>,
    runtime: auki_network::NetworkRuntimeHandle,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(MembershipEvent { peer, update }) = rx.recv().await {
            let parsed: ClusterMembership = match serde_json::from_str(&update.membership_json) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("auki-domain: membership gossip from {peer}: invalid JSON: {e}");
                    continue;
                }
            };
            let advertised_manager = update.manager_peer_id;
            if !valid_membership_update_manager(peer, advertised_manager, &parsed) {
                eprintln!(
                    "auki-domain: membership gossip from {peer}: invalid advertised Manager {advertised_manager}"
                );
                continue;
            }

            // Last-write-wins: replace local membership, adopt the
            // advertised Manager, and rebuild the allow-list. The
            // cluster-trust gate on the runtime side already refused
            // non-cluster senders; the sender==manager check above
            // prevents an arbitrary member from claiming the role for
            // another peer.
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
            *manager_peer_id.lock().expect("manager_peer_id lock") = advertised_manager;
            if let Err(e) = runtime.set_allowed_peers(new_allow_list).await {
                eprintln!(
                    "auki-domain: membership gossip apply: set_allowed_peers \
                    failed: {e}"
                );
            }
            sync_heartbeat_targets(
                &runtime,
                local_peer_id,
                &manager_peer_id,
                &membership,
                &cluster_name,
            )
            .await;
        }
    })
}

fn valid_membership_update_manager(
    sender: PeerId,
    advertised_manager: PeerId,
    membership: &ClusterMembership,
) -> bool {
    sender == advertised_manager
        && membership
            .peers
            .iter()
            .any(|p| p.peer_id == advertised_manager)
}

/// Serialize the current `membership` and `broadcast_membership` it
/// over `/auki/membership/0.0.1`. Logged-and-swallow on encode
/// failure; per-peer write failures are logged inside the runtime's
/// per-task spawns. The Manager calls this after admit, after
/// eviction, and on Manager-promotion.
fn broadcast_current_membership(
    runtime: &auki_network::NetworkRuntimeHandle,
    manager_peer_id: &Arc<Mutex<PeerId>>,
    membership: &Arc<Mutex<ClusterMembership>>,
) {
    let manager = *manager_peer_id.lock().expect("manager_peer_id lock");
    let json = {
        let m = membership.lock().expect("membership lock");
        match serde_json::to_string(&*m) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("auki-domain: serializing membership for gossip failed: {e}");
                return;
            }
        }
    };
    if let Err(e) = runtime.broadcast_membership(manager, json) {
        eprintln!("auki-domain: broadcast_membership failed: {e}");
    }
}

/// Spawn a task that drains inbound `/auki/info/0.0.1` requests
/// from `rx` and replies on each `ack` with a freshly-built
/// [`ParticipantInfo`] (serialized to JSON). Combines the stored
/// `daemon_info` with SDK-tracked dynamic fields (`session_now_ns`,
/// `cluster_joined_at_ns`, `is_manager`, `manager_peer_id`,
/// `peer_id`) to build the response.
///
/// Lives for the lifetime of the `ClusterManager`; cancelled on
/// `shutdown`.
#[allow(clippy::too_many_arguments)]
fn spawn_info_handler(
    mut rx: mpsc::Receiver<InfoRequestEvent>,
    local_peer_id: PeerId,
    manager_peer_id: Arc<Mutex<PeerId>>,
    membership: Arc<Mutex<ClusterMembership>>,
    daemon_info: DaemonInfo,
    session_clock: SessionClock,
    cluster_joined_at_ns: Arc<Mutex<Option<u64>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(InfoRequestEvent { peer, ack, .. }) = rx.recv().await {
            let info = build_participant_info(
                &daemon_info,
                local_peer_id,
                &manager_peer_id,
                &membership,
                &session_clock,
                &cluster_joined_at_ns,
            );
            let json = match serde_json::to_string(&info) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("auki-domain: info handler for {peer}: serialize failed: {e}");
                    continue;
                }
            };
            let _ = ack.send(InfoResponse {
                participant_info_json: json,
            });
        }
    })
}

/// Shared `ParticipantInfo` builder used by both
/// [`ClusterManager::participant_info`] (the local accessor) and
/// [`spawn_info_handler`] (the inbound `/auki/info/0.0.1` reply
/// path). Reading the dynamic fields once on each build keeps both
/// surfaces consistent.
fn build_participant_info(
    daemon: &DaemonInfo,
    local_peer_id: PeerId,
    manager_peer_id: &Arc<Mutex<PeerId>>,
    membership: &Arc<Mutex<ClusterMembership>>,
    session_clock: &SessionClock,
    cluster_joined_at_ns: &Arc<Mutex<Option<u64>>>,
) -> ParticipantInfo {
    let manager = *manager_peer_id.lock().expect("manager_peer_id lock");
    let session_now_ns = session_clock.now_ns();
    let cj = {
        let mut guard = cluster_joined_at_ns
            .lock()
            .expect("cluster_joined_at_ns mutex poisoned");
        if guard.is_none() {
            let has_other = membership
                .lock()
                .expect("membership lock")
                .peers
                .iter()
                .any(|p| p.peer_id != local_peer_id);
            if has_other {
                *guard = Some(session_now_ns);
            }
        }
        *guard
    };
    ParticipantInfo {
        app: daemon.app.clone(),
        name: daemon.name.clone(),
        session_id: daemon.session_id.clone(),
        session_clock_id: session_clock.clock_id().to_string(),
        session_clock_hash: session_clock.clock_hash(),
        session_now_ns,
        cluster_joined_at_ns: cj,
        peer_id: local_peer_id,
        app_instance: daemon.app_instance.clone(),
        is_manager: manager == local_peer_id,
        manager_peer_id: manager.to_string(),
    }
}

fn domain_clock_id(cluster_name: &str) -> String {
    format!("{cluster_name}/domain-clock")
}

fn domain_clock_source_store() -> DomainClockSources {
    Arc::new(Mutex::new(HashMap::new()))
}

fn domain_clock_hash(cluster_name: &str) -> String {
    let declaration = serde_json::json!({
        "cluster_name": cluster_name,
        "domain_clock_id": domain_clock_id(cluster_name),
        "kind": "domain_clock",
        "schema_version": 1,
    });
    let bytes = auki_jcs::canonicalize(&declaration);
    auki_hash::hash_jcs_bytes(&bytes)
}

fn initial_domain_clock_source(
    cluster_name: &str,
    local_peer_id: PeerId,
    session_clock: &SessionClock,
) -> HeartbeatDomainClock {
    HeartbeatDomainClock {
        cluster_name: cluster_name.to_string(),
        domain_clock_id: domain_clock_id(cluster_name),
        domain_clock_hash: domain_clock_hash(cluster_name),
        backing_peer_id: local_peer_id.to_string(),
        backing_clock_id: session_clock.clock_id().to_string(),
        backing_clock_hash: session_clock.clock_hash(),
        backing_to_domain_offset_ns: 0,
    }
}

fn observe_heartbeat_domain_clock_source(
    sources: &DomainClockSources,
    cluster_name: &str,
    peer_id: PeerId,
    source: Option<HeartbeatDomainClock>,
) {
    let Some(source) = source else {
        return;
    };
    if source.cluster_name != cluster_name || source.backing_peer_id != peer_id.to_string() {
        return;
    }
    sources
        .lock()
        .expect("domain clock source lock poisoned")
        .insert(DomainClockSourceKey::from_source(&source), source);
}

fn select_domain_clock_source(
    sources: &DomainClockSources,
    cluster_name: &str,
    preferred_backing_peer_id: Option<&str>,
) -> Option<HeartbeatDomainClock> {
    let mut matches = sources
        .lock()
        .expect("domain clock source lock poisoned")
        .values()
        .filter(|source| source.cluster_name == cluster_name)
        .cloned()
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| {
        a.backing_peer_id
            .cmp(&b.backing_peer_id)
            .then_with(|| a.backing_clock_id.cmp(&b.backing_clock_id))
    });
    if let Some(preferred) = preferred_backing_peer_id {
        if let Some(source) = matches
            .iter()
            .find(|source| source.backing_peer_id == preferred)
            .cloned()
        {
            return Some(source);
        }
    }
    matches.into_iter().next()
}

fn descriptor_from_heartbeat_domain_clock(source: &HeartbeatDomainClock) -> DomainClockDescriptor {
    DomainClockDescriptor::new(
        source.cluster_name.clone(),
        source.domain_clock_id.clone(),
        source.domain_clock_hash.clone(),
        source.backing_peer_id.clone(),
        source.backing_clock_id.clone(),
        source.backing_clock_hash.clone(),
        source.backing_to_domain_offset_ns,
    )
}

fn estimate_cluster_domain_clock(
    clock_sync: &ClockSyncHandle,
    sources: &DomainClockSources,
    cluster_name: &str,
    preferred_backing_peer_id: Option<&str>,
    local_clock_id: &str,
    local_clock_hash: &str,
    local_clock_now_ns: i64,
) -> Result<DomainClockEstimate, DomainClockEstimateUnavailable> {
    let source = select_domain_clock_source(sources, cluster_name, preferred_backing_peer_id)
        .ok_or_else(|| DomainClockEstimateUnavailable::SourceUnavailable {
            cluster_name: cluster_name.to_string(),
        })?;

    let local_to_backing = if source.backing_clock_id == local_clock_id
        && source.backing_clock_hash == local_clock_hash
    {
        ClockTransformEstimate::identity(local_clock_id, local_clock_hash, local_clock_now_ns)
    } else {
        clock_sync
            .estimate(local_clock_id, &source.backing_clock_id)
            .ok_or_else(
                || DomainClockEstimateUnavailable::BackingEstimateUnavailable {
                    local_clock_id: local_clock_id.to_string(),
                    backing_clock_id: source.backing_clock_id.clone(),
                },
            )?
    };

    auki_time::estimate_domain_clock(
        local_to_backing,
        descriptor_from_heartbeat_domain_clock(&source),
    )
    .map_err(DomainClockEstimateUnavailable::InvalidSource)
}

fn convert_session_now_to_domain_time(
    estimate: &DomainClockEstimate,
    session_now_ns: i64,
) -> Result<i64, DomainTimeNowError> {
    estimate.time_transform().convert_ns(session_now_ns).ok_or(
        DomainTimeNowError::ConversionOutOfRange {
            session_now_ns,
            offset_ns: estimate.total_offset_ns,
        },
    )
}

fn advertise_promoted_domain_clock_source(
    advertised_domain_clock_source: &Arc<Mutex<Option<HeartbeatDomainClock>>>,
    clock_sync: &ClockSyncHandle,
    sources: &DomainClockSources,
    cluster_name: &str,
    local_peer_id: PeerId,
    session_clock: &SessionClock,
    local_clock_now_ns: i64,
) -> Result<HeartbeatDomainClock, DomainClockEstimateUnavailable> {
    let session_clock_hash = session_clock.clock_hash();
    let estimate = estimate_cluster_domain_clock(
        clock_sync,
        sources,
        cluster_name,
        None,
        session_clock.clock_id(),
        &session_clock_hash,
        local_clock_now_ns,
    )?;
    let source = HeartbeatDomainClock {
        cluster_name: cluster_name.to_string(),
        domain_clock_id: estimate.domain_clock_id,
        domain_clock_hash: estimate.domain_clock_hash,
        backing_peer_id: local_peer_id.to_string(),
        backing_clock_id: session_clock.clock_id().to_string(),
        backing_clock_hash: session_clock_hash,
        backing_to_domain_offset_ns: estimate.total_offset_ns,
    };

    observe_heartbeat_domain_clock_source(
        sources,
        cluster_name,
        local_peer_id,
        Some(source.clone()),
    );
    *advertised_domain_clock_source
        .lock()
        .expect("advertised domain clock source lock poisoned") = Some(source.clone());
    Ok(source)
}

fn heartbeat_timestamp_source(
    session_clock: SessionClock,
    advertised_domain_clock_source: Arc<Mutex<Option<HeartbeatDomainClock>>>,
) -> HeartbeatTimestampSource {
    let clock_id = session_clock.clock_id().to_string();
    let clock_hash = session_clock.clock_hash();
    HeartbeatTimestampSource {
        clock_id,
        clock_hash,
        now_ns: Arc::new(move || session_clock.now_i64_ns()),
        domain_clock: Arc::new(move || {
            advertised_domain_clock_source
                .lock()
                .expect("advertised domain clock source lock poisoned")
                .clone()
        }),
    }
}

/// Errors from [`ClusterManager::fetch_participant_info`].
#[derive(Debug, Error)]
pub enum FetchParticipantInfoError {
    /// libp2p / wire / timeout failure during the request.
    #[error("request_participant_info: {0}")]
    Request(#[from] RequestInfoError),
    /// Responder's payload was not parseable as `ParticipantInfo`.
    #[error("invalid ParticipantInfo JSON from peer: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// [`ClusterManager::shutdown`] has been called. Callers
    /// holding a stale `Arc<ClusterManager>` clone see this
    /// rather than a cascading libp2p substream error.
    #[error("ClusterManager has been shut down")]
    Stopped,
}

/// Errors from [`ClusterManager::fetch_resources_catalog`].
#[derive(Debug, Error)]
pub enum FetchResourcesCatalogError {
    /// libp2p / wire / timeout failure during the request.
    #[error("request_resources_catalog: {0}")]
    Request(#[from] RequestResourcesError),
}

/// Errors from `ClusterManager::fetch_*_entry`.
#[derive(Debug, Error)]
pub enum FetchRegistryEntryError {
    /// libp2p / wire / timeout failure during the request.
    #[error("request_registry_entry: {0}")]
    Request(#[from] RequestRegistryError),
    /// The peer replied cleanly but does not have the exact entry.
    #[error("registry entry not found: kind={kind} id={id:?} hash={hash}")]
    NotFound {
        /// Registry namespace.
        kind: RegistryKind,
        /// Requested registry id.
        id: String,
        /// Requested registry hash.
        hash: String,
    },
    /// Returned envelope did not match the requested kind/id/hash
    /// contract.
    #[error("invalid registry envelope: {0}")]
    InvalidEnvelope(String),
    /// Returned canonical JSON bytes did not hash to the requested
    /// hash.
    #[error("registry hash mismatch: expected {expected}, got {actual}")]
    HashMismatch {
        /// Requested hash.
        expected: String,
        /// Hash computed from returned `canonical_json` bytes.
        actual: String,
    },
    /// Returned canonical JSON could not be decoded into the requested
    /// typed registry entry.
    #[error("invalid registry JSON from peer: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// [`ClusterManager::shutdown`] has been called.
    #[error("ClusterManager has been shut down")]
    Stopped,
}

fn verify_registry_envelope(
    envelope: &RegistryEntryEnvelope,
    expected_kind: RegistryKind,
    expected_id: &str,
    expected_hash: &str,
) -> Result<(), FetchRegistryEntryError> {
    if envelope.kind != expected_kind {
        return Err(FetchRegistryEntryError::InvalidEnvelope(format!(
            "kind mismatch: expected {}, found {}",
            expected_kind, envelope.kind
        )));
    }
    if envelope.id != expected_id {
        return Err(FetchRegistryEntryError::InvalidEnvelope(format!(
            "id mismatch: expected {:?}, found {:?}",
            expected_id, envelope.id
        )));
    }
    if envelope.hash != expected_hash {
        return Err(FetchRegistryEntryError::InvalidEnvelope(format!(
            "hash field mismatch: expected {}, found {}",
            expected_hash, envelope.hash
        )));
    }
    let actual_hash = auki_hash::hash_jcs_bytes(envelope.canonical_json.as_bytes());
    if actual_hash != expected_hash {
        return Err(FetchRegistryEntryError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }
    Ok(())
}

/// Spawn a task that drains inbound `/auki/resources/0.2.0` requests
/// from `rx` and replies on each `ack` with a freshly-snapshotted
/// [`ResourcesResponse`] sourced from the registered
/// [`ResourceCatalogProvider`] or, if absent, [`SessionHandle`].
///
/// If neither source is set yet (pre-session bootstrap), returns an
/// empty catalog. Variant filtering: if `request.variants` is
/// non-empty, only rows whose `variant_content` tag appears in
/// `variants` are returned. An empty `variants` list means "all
/// variants".
fn spawn_resources_handler(
    mut rx: mpsc::Receiver<ResourcesRequestEvent>,
    resource_catalog_provider: Arc<Mutex<Option<Arc<dyn ResourceCatalogProvider>>>>,
    session_handle: Arc<Mutex<Option<Arc<dyn SessionHandle>>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(ResourcesRequestEvent { request, ack, .. }) = rx.recv().await {
            let resources = {
                let provider = resource_catalog_provider
                    .lock()
                    .expect("resource_catalog_provider lock")
                    .clone();
                if let Some(provider) = provider {
                    provider.snapshot_for_request(&request, None)
                } else {
                    let all_resources = {
                        let guard = session_handle.lock().expect("session_handle lock");
                        match guard.as_ref() {
                            Some(h) => h.catalog(),
                            None => Vec::new(),
                        }
                    };
                    if request.variants.is_empty() {
                        all_resources
                    } else {
                        use auki_network::resources_protocol::{Variant, VariantContent};
                        all_resources
                            .into_iter()
                            .filter(|r| {
                                let row_variant = match &r.variant_content {
                                    VariantContent::SensorLog { .. } => Variant::SensorLog,
                                    VariantContent::PoseLog { .. } => Variant::PoseLog,
                                    VariantContent::TimeTransformLog { .. } => {
                                        Variant::TimeTransformLog
                                    }
                                    VariantContent::DetectionLog { .. } => Variant::DetectionLog,
                                };
                                request.variants.contains(&row_variant)
                            })
                            .collect()
                    }
                }
            };
            let _ = ack.send(ResourcesResponse { resources });
        }
    })
}

/// Spawn a task that drains inbound `/auki/registries/0.0.1` requests
/// from `rx` and replies with the requested canonical JSON registry
/// entry if it exists under the registered app root.
///
/// `entry: None` is the v0 not-found response: the peer understood
/// the protocol but does not have that exact `(kind, id, hash)` entry.
fn spawn_registry_handler(
    mut rx: mpsc::Receiver<RegistryRequestEvent>,
    app_root: Arc<Mutex<Option<PathBuf>>>,
    local_peer_id: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(RegistryRequestEvent { peer, request, ack }) = rx.recv().await {
            let root = app_root.lock().expect("registry_app_root lock").clone();
            let entry = match root {
                Some(root) => match read_registry_envelope(&root, &local_peer_id, &request) {
                    Ok(entry) => entry,
                    Err(e) => {
                        eprintln!(
                            "auki-domain: registry handler for {peer}: {:?} {:?}@{} failed: {e}",
                            request.kind, request.id, request.hash
                        );
                        None
                    }
                },
                None => None,
            };
            let _ = ack.send(RegistryResponse { entry });
        }
    })
}

fn spawn_diagnostic_handler(
    mut rx: mpsc::Receiver<DiagnosticEvent>,
    messages: Arc<Mutex<Vec<InboundDiagnosticMessage>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(DiagnosticEvent { peer, message }) = rx.recv().await {
            let mut messages = messages.lock().expect("diagnostic_messages lock");
            messages.push(InboundDiagnosticMessage {
                peer_id: peer,
                message,
            });
            if messages.len() > 256 {
                messages.remove(0);
            }
        }
    })
}

fn read_registry_envelope(
    app_root: &std::path::Path,
    peer_id: &str,
    request: &RegistryRequest,
) -> Result<Option<RegistryEntryEnvelope>, auki_registry::Error> {
    match request.kind {
        RegistryKind::Sensor => {
            let Some(entry) =
                auki_registry::read_sensor(app_root, peer_id, &request.id, &request.hash)?
            else {
                return Ok(None);
            };
            Ok(Some(envelope_for_sensor(entry)))
        }
        RegistryKind::Clock => {
            let Some(entry) =
                auki_registry::read_clock(app_root, peer_id, &request.id, &request.hash)?
            else {
                return Ok(None);
            };
            Ok(Some(envelope_for_clock(entry)))
        }
        RegistryKind::Frame => {
            let Some(entry) =
                auki_registry::read_frame(app_root, peer_id, &request.id, &request.hash)?
            else {
                return Ok(None);
            };
            Ok(Some(envelope_for_frame(entry)))
        }
        RegistryKind::Detector => {
            let Some(entry) =
                auki_registry::read_detector(app_root, peer_id, &request.id, &request.hash)?
            else {
                return Ok(None);
            };
            Ok(Some(envelope_for_detector(entry)))
        }
    }
}

fn envelope_for_sensor(entry: SensorRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::Sensor,
        id: entry.sensor_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
    }
}

fn envelope_for_clock(entry: ClockRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::Clock,
        id: entry.clock_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
    }
}

fn envelope_for_frame(entry: FrameRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::Frame,
        id: entry.frame_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
    }
}

fn envelope_for_detector(entry: DetectorRegistryEntry) -> RegistryEntryEnvelope {
    let bytes = entry.canonical_bytes();
    RegistryEntryEnvelope {
        kind: RegistryKind::Detector,
        id: entry.detector_id,
        hash: auki_hash::hash_jcs_bytes(&bytes),
        canonical_json: String::from_utf8(bytes).expect("JCS output is UTF-8 JSON"),
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

/// What a follower does after its Manager heartbeat timed out, given
/// Discovery's current row for the cluster. Pure for testing; the
/// liveness handler acts on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerLossAction {
    /// Discovery still names the lost peer: someone is keeping the
    /// row alive, so the timeout is a transport problem on our side.
    /// Keep the Manager view, reset the heartbeat watch, rejoin.
    Defer {
        /// Multiaddrs Discovery reports for the still-named Manager.
        manager_multiaddrs: Vec<Multiaddr>,
    },
    /// Discovery names a different Manager: that claim already
    /// committed. Adopt it and rejoin it.
    Follow {
        /// The peer Discovery currently lists as Manager.
        manager: PeerId,
        /// Multiaddrs Discovery reports for the new Manager.
        manager_multiaddrs: Vec<Multiaddr>,
    },
    /// No row at Discovery (swept): the Manager fell silent long
    /// enough for Discovery to evict the row. Run the local election.
    ElectLocally,
}

/// Decide what a follower should do after its Manager heartbeat timed
/// out, consulting Discovery's current cluster row.
///
/// This is a pure function for ease of testing.
pub fn decide_manager_loss_action(
    entry: Option<&DiscoveryClusterEntry>,
    lost_manager: PeerId,
    local_peer_id: PeerId,
) -> ManagerLossAction {
    match entry {
        None => ManagerLossAction::ElectLocally,
        Some(e) if e.manager_peer_id == lost_manager => ManagerLossAction::Defer {
            manager_multiaddrs: e.manager_multiaddrs.clone(),
        },
        // Discovery already names us (e.g. a commit landed and the
        // process restarted mid-promotion): run the election —
        // re-registering as ourselves is idempotent.
        Some(e) if e.manager_peer_id == local_peer_id => ManagerLossAction::ElectLocally,
        Some(e) => ManagerLossAction::Follow {
            manager: e.manager_peer_id,
            manager_multiaddrs: e.manager_multiaddrs.clone(),
        },
    }
}

fn elect_successor_excluding_lost(
    membership: &ClusterMembership,
    local_peer_id: PeerId,
    connected: &[PeerId],
    lost_peer_id: PeerId,
) -> Option<PeerId> {
    let connected: Vec<PeerId> = connected
        .iter()
        .copied()
        .filter(|pid| *pid != lost_peer_id)
        .collect();
    let mut membership = membership.clone();
    membership.peers.retain(|p| p.peer_id != lost_peer_id);
    elect_successor(&membership, local_peer_id, &connected)
}

fn spawn_manager_liveness_check(
    discovery: Arc<DiscoveryClient>,
    cluster_name: String,
    membership: Arc<Mutex<ClusterMembership>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(LIVENESS_CHECK_INTERVAL);
        // First tick fires immediately; skip it because Discovery
        // already has our state from `create_cluster`'s synchronous
        // `create` call. The next tick happens at +1s.
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
            // sweeps after 3s of no liveness check (3 missed at 1s
            // cadence), so persistent failures self-resolve.
            if let Err(e) = discovery
                .liveness_check(cluster_name.clone(), peer_count)
                .await
            {
                eprintln!(
                    "auki-domain: Discovery liveness_check for cluster {cluster_name:?} failed: {e}"
                );
            }
        }
    })
}

/// Outcome of a Manager-side join request against the current doc.
#[derive(Debug, PartialEq, Eq)]
enum AdmitOutcome {
    /// New member appended.
    Admitted,
    /// Already a member (a deferring peer re-establishing after a
    /// heartbeat lapse): dialable addrs refreshed; `join_ts_ns` is
    /// untouched so the election order stays stable.
    Refreshed,
}

fn admit_or_refresh(
    m: &mut ClusterMembership,
    peer: PeerId,
    multiaddrs: Vec<Multiaddr>,
    join_ts_ns: i64,
) -> AdmitOutcome {
    if let Some(existing) = m.peers.iter_mut().find(|p| p.peer_id == peer) {
        existing.multiaddrs = multiaddrs;
        return AdmitOutcome::Refreshed;
    }
    m.admit(ClusterMember {
        peer_id: peer,
        multiaddrs,
        join_ts_ns,
        // v1: empty successor token (signature verification disabled per Discovery v1 contract).
        successor_token: Some(Vec::new()),
    });
    AdmitOutcome::Admitted
}

/// Spawn a task that drains inbound join events from `rx` and
/// replies on each `ack`. Manager peers admit — or idempotently
/// refresh a current member's multiaddrs on re-join — and push
/// the updated allow-list via the runtime handle; non-Manager
/// peers reject with `"not the manager"`. The task lives for the
/// lifetime of the `ClusterManager`; cancelled on `shutdown`.
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
            let JoinEvent { peer, request, ack } = event;

            // Manager check.
            let am_manager =
                *manager_peer_id.lock().expect("manager_peer_id lock") == local_peer_id;
            if !am_manager {
                let _ = ack.send(JoinResponse::Reject {
                    reason: "not the manager".into(),
                });
                continue;
            }

            // Admit a new peer or refresh the multiaddrs of a peer
            // that is re-joining after a heartbeat lapse (idempotent).
            // Either way, rebuild the allow-list and snapshot the
            // membership JSON inside a short lock window. The runtime
            // call happens afterwards (locks released, no holding
            // across await).
            let (outcome, new_allow_list, full_membership_json) = {
                let mut m = membership.lock().expect("membership lock");
                let outcome =
                    admit_or_refresh(&mut m, peer, request.multiaddrs.clone(), now_unix_nanos());
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
                (outcome, allow_list, json)
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
            sync_heartbeat_targets(
                &runtime,
                local_peer_id,
                &manager_peer_id,
                &membership,
                &cluster_name,
            )
            .await;

            let _ = ack.send(JoinResponse::Accept {
                membership_json: full_membership_json,
                // v1: empty successor token (signature verification disabled per Discovery v1 contract).
                successor_token: Vec::new(),
            });
            if outcome == AdmitOutcome::Refreshed {
                eprintln!(
                    "auki-domain: join handler for cluster {cluster_name:?}: refreshed multiaddrs for re-joining member {peer}"
                );
            }

            // Gossip the updated membership to every other connected
            // peer so existing members learn about the new joiner.
            // The new joiner itself just received the same JSON in
            // the JoinResponse::Accept above; the broadcast targets
            // everyone else.
            broadcast_current_membership(&runtime, &manager_peer_id, &membership);
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
    use auki_time::SessionClock;

    #[test]
    fn daemon_info_is_cheap_to_clone() {
        let d = DaemonInfo {
            app: "x".into(),
            name: "y".into(),
            session_id: "z".into(),
            session_clock_id: "c".into(),
            session_clock_hash: "h".into(),
            app_instance: "abc".into(),
        };
        let _ = d.clone();
    }

    #[test]
    fn discovery_row_missing_only_on_404() {
        let missing = DiscoveryClientError::Status {
            status: 404,
            body: String::new(),
        };
        let other = DiscoveryClientError::Status {
            status: 500,
            body: String::new(),
        };
        assert!(discovery_row_missing(&missing));
        assert!(!discovery_row_missing(&other));
    }

    #[test]
    fn manager_rotation_request_omits_empty_relay_multiaddrs() {
        let manager_addr = "/ip4/127.0.0.1/tcp/4001".parse::<Multiaddr>().unwrap();

        assert_eq!(
            manager_rotation_discovery_request(&[manager_addr.clone()], &[]),
            ManagerRotationDiscoveryRequest::DirectOnly {
                manager_multiaddrs: vec![manager_addr],
            }
        );
    }

    #[test]
    fn manager_rotation_request_preserves_relay_multiaddrs() {
        let manager_addr = "/ip4/127.0.0.1/tcp/4001".parse::<Multiaddr>().unwrap();
        let relay_addr =
            "/ip4/127.0.0.1/tcp/4002/ws/p2p/12D3KooWESKUn3Fh3xTMq1KzoxbQQ6PypHodP1JAb4p7qkxJxJ7n"
                .parse::<Multiaddr>()
                .unwrap();

        assert_eq!(
            manager_rotation_discovery_request(&[manager_addr.clone()], &[relay_addr.clone()]),
            ManagerRotationDiscoveryRequest::WithRelays {
                manager_multiaddrs: vec![manager_addr],
                relay_multiaddrs: vec![relay_addr],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn relay_reservation_appends_manager_circuit_and_relay_hint() {
        use auki_network::swarm::{SwarmConfig, build_swarm};
        use futures::StreamExt as _;

        let relay_identity = auki_network::PeerIdentity::from_seed(&[71u8; 32]);
        let manager_identity = auki_network::PeerIdentity::from_seed(&[72u8; 32]);
        let mut relay_swarm = build_swarm(
            &relay_identity,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                agent_version: "relay/0".into(),
                enable_relay_server: true,
            },
        )
        .expect("relay swarm builds");
        let relay_addr = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(libp2p::swarm::SwarmEvent::NewListenAddr { address, .. }) =
                    relay_swarm.next().await
                {
                    return address;
                }
            }
        })
        .await
        .expect("relay listen address appears");
        relay_swarm.add_external_address(relay_addr.clone());
        let relay_dial_multiaddr =
            relay_addr.with(libp2p::multiaddr::Protocol::P2p(relay_identity.peer_id()));
        let relay_advertise_multiaddr: Multiaddr = format!(
            "/ip4/203.0.113.72/tcp/4002/ws/p2p/{}",
            relay_identity.peer_id()
        )
        .parse()
        .unwrap();
        let expected_circuit = relay_advertise_multiaddr
            .clone()
            .with(libp2p::multiaddr::Protocol::P2pCircuit)
            .with(libp2p::multiaddr::Protocol::P2p(manager_identity.peer_id()));
        let mut manager_swarm = build_swarm(
            &manager_identity,
            SwarmConfig {
                listen_addresses: vec![],
                agent_version: "manager/0".into(),
                enable_relay_server: false,
            },
        )
        .expect("manager swarm builds");
        let direct_manager_addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let mut manager_multiaddrs = vec![direct_manager_addr.clone()];
        let mut relay_multiaddrs = Vec::new();
        let reservation = ManagerRelayReservation {
            relay_dial_multiaddr,
            relay_advertise_multiaddr: relay_advertise_multiaddr.clone(),
            timeout: Duration::from_secs(10),
        };

        tokio::time::timeout(Duration::from_secs(15), async {
            tokio::select! {
                result = reserve_manager_relay_multiaddr(
                    &mut manager_swarm,
                    &mut manager_multiaddrs,
                    &mut relay_multiaddrs,
                    &reservation,
                ) => result,
                _ = async {
                    loop {
                        let _ = relay_swarm.next().await;
                    }
                } => unreachable!("relay swarm stream ended"),
            }
        })
        .await
        .expect("relay reservation timed out")
        .expect("relay reservation succeeds");

        assert_eq!(
            manager_multiaddrs,
            vec![direct_manager_addr, expected_circuit]
        );
        assert_eq!(relay_multiaddrs, vec![relay_advertise_multiaddr]);
    }

    #[test]
    fn heartbeat_source_uses_session_clock_for_initial_manager_domain_clock_metadata() {
        let peer = auki_network::PeerIdentity::from_seed(&[11u8; 32]).peer_id();
        let daemon = DaemonInfo {
            app: "boosterapp".into(),
            name: "booster".into(),
            session_id: "session-123".into(),
            session_clock_id: "legacy/stale/session-clock".into(),
            session_clock_hash: "legacy-hash".into(),
            app_instance: "instance".into(),
        };
        let session_clock =
            SessionClock::new(peer.to_string(), daemon.session_id.clone(), "monotonic");
        let advertised = Arc::new(Mutex::new(Some(initial_domain_clock_source(
            "cluster-a",
            peer,
            &session_clock,
        ))));
        let source = heartbeat_timestamp_source(session_clock.clone(), advertised);

        let domain_clock =
            (source.domain_clock)().expect("initial Manager advertises domain clock");

        assert_eq!(source.clock_id, session_clock.clock_id());
        assert_eq!(source.clock_hash, session_clock.clock_hash());
        assert_eq!(domain_clock.cluster_name, "cluster-a");
        assert_eq!(domain_clock.domain_clock_id, "cluster-a/domain-clock");
        assert_eq!(
            domain_clock.domain_clock_hash,
            domain_clock_hash("cluster-a")
        );
        assert_eq!(domain_clock.backing_peer_id, peer.to_string());
        assert_eq!(domain_clock.backing_clock_id, session_clock.clock_id());
        assert_eq!(domain_clock.backing_clock_hash, session_clock.clock_hash());
        assert_eq!(domain_clock.backing_to_domain_offset_ns, 0);
    }

    #[test]
    fn heartbeat_source_omits_domain_clock_when_not_advertised() {
        let daemon = DaemonInfo {
            app: "park".into(),
            name: "park".into(),
            session_id: "session-456".into(),
            session_clock_id: "peer/12D3Follower/session-456/monotonic".into(),
            session_clock_hash: "follower-clock-hash".into(),
            app_instance: "instance".into(),
        };
        let session_clock =
            SessionClock::new("12D3Follower", daemon.session_id.clone(), "monotonic");
        let advertised = Arc::new(Mutex::new(None));
        let source = heartbeat_timestamp_source(session_clock, advertised);

        assert!((source.domain_clock)().is_none());
    }

    #[test]
    fn domain_clock_estimate_is_unavailable_without_source_metadata() {
        let sources = domain_clock_source_store();
        let clock_sync = auki_time::ClockSyncHandle::default();

        let err = estimate_cluster_domain_clock(
            &clock_sync,
            &sources,
            "cluster-a",
            None,
            "peer/follower/session-456/monotonic",
            "follower-clock-hash",
            10_000,
        )
        .unwrap_err();

        assert_eq!(
            err,
            DomainClockEstimateUnavailable::SourceUnavailable {
                cluster_name: "cluster-a".into()
            }
        );
    }

    #[test]
    fn domain_clock_estimate_is_unavailable_without_peer_clock_estimate() {
        let sources = domain_clock_source_store();
        let clock_sync = auki_time::ClockSyncHandle::default();
        let peer = auki_network::PeerIdentity::from_seed(&[12u8; 32]).peer_id();
        let source = HeartbeatDomainClock {
            cluster_name: "cluster-a".into(),
            domain_clock_id: "cluster-a/domain-clock".into(),
            domain_clock_hash: domain_clock_hash("cluster-a"),
            backing_peer_id: peer.to_string(),
            backing_clock_id: "peer/manager/session-123/monotonic".into(),
            backing_clock_hash: "manager-clock-hash".into(),
            backing_to_domain_offset_ns: 0,
        };
        observe_heartbeat_domain_clock_source(&sources, "cluster-a", peer, Some(source.clone()));

        let err = estimate_cluster_domain_clock(
            &clock_sync,
            &sources,
            "cluster-a",
            None,
            "peer/follower/session-456/monotonic",
            "follower-clock-hash",
            10_000,
        )
        .unwrap_err();

        assert_eq!(
            err,
            DomainClockEstimateUnavailable::BackingEstimateUnavailable {
                local_clock_id: "peer/follower/session-456/monotonic".into(),
                backing_clock_id: source.backing_clock_id,
            }
        );
    }

    #[test]
    fn domain_clock_estimate_composes_stored_source_with_peer_clock_estimate() {
        let sources = domain_clock_source_store();
        let clock_sync = auki_time::ClockSyncHandle::default();
        let peer = auki_network::PeerIdentity::from_seed(&[13u8; 32]).peer_id();
        let source = HeartbeatDomainClock {
            cluster_name: "cluster-a".into(),
            domain_clock_id: "cluster-a/domain-clock".into(),
            domain_clock_hash: domain_clock_hash("cluster-a"),
            backing_peer_id: peer.to_string(),
            backing_clock_id: "peer/manager/session-123/monotonic".into(),
            backing_clock_hash: "manager-clock-hash".into(),
            backing_to_domain_offset_ns: 250,
        };
        observe_heartbeat_domain_clock_source(&sources, "cluster-a", peer, Some(source));
        clock_sync.observe(ClockSyncObservation::new(
            "peer/follower/session-456/monotonic",
            "follower-clock-hash",
            "peer/manager/session-123/monotonic",
            "manager-clock-hash",
            auki_time::NtpSample {
                offset_ns: 1_000_000,
                uncertainty_ns: 20,
                round_trip_ns: 50,
                remote_processing_ns: 30,
                observed_at_clock_ns: 10_000,
            },
        ));

        let estimate = estimate_cluster_domain_clock(
            &clock_sync,
            &sources,
            "cluster-a",
            None,
            "peer/follower/session-456/monotonic",
            "follower-clock-hash",
            10_000,
        )
        .unwrap();

        assert_eq!(estimate.cluster_name, "cluster-a");
        assert_eq!(
            estimate.local_clock_id,
            "peer/follower/session-456/monotonic"
        );
        assert_eq!(estimate.domain_clock_id, "cluster-a/domain-clock");
        assert_eq!(estimate.peer_to_backing_offset_ns, 1_000_000);
        assert_eq!(estimate.backing_to_domain_offset_ns, 250);
        assert_eq!(estimate.total_offset_ns, 1_000_250);
        assert_eq!(estimate.uncertainty_ns, 20);
    }

    #[test]
    fn domain_clock_estimate_for_initial_manager_uses_local_identity_transform() {
        let sources = domain_clock_source_store();
        let clock_sync = auki_time::ClockSyncHandle::default();
        let peer = auki_network::PeerIdentity::from_seed(&[14u8; 32]).peer_id();
        let daemon = DaemonInfo {
            app: "boosterapp".into(),
            name: "booster".into(),
            session_id: "session-123".into(),
            session_clock_id: "legacy/stale/session-clock".into(),
            session_clock_hash: "legacy-hash".into(),
            app_instance: "instance".into(),
        };
        let session_clock =
            SessionClock::new(peer.to_string(), daemon.session_id.clone(), "monotonic");
        let source = initial_domain_clock_source("cluster-a", peer, &session_clock);
        observe_heartbeat_domain_clock_source(&sources, "cluster-a", peer, Some(source));

        let estimate = estimate_cluster_domain_clock(
            &clock_sync,
            &sources,
            "cluster-a",
            None,
            session_clock.clock_id(),
            &session_clock.clock_hash(),
            12_345,
        )
        .unwrap();

        assert_eq!(estimate.local_clock_id, session_clock.clock_id());
        assert_eq!(estimate.domain_clock_id, "cluster-a/domain-clock");
        assert_eq!(estimate.total_offset_ns, 0);
        assert_eq!(estimate.uncertainty_ns, 0);
        assert_eq!(estimate.observed_at_clock_ns, 12_345);
    }

    #[test]
    fn promoted_manager_advertises_domain_clock_when_offset_is_proven() {
        let sources = domain_clock_source_store();
        let clock_sync = auki_time::ClockSyncHandle::default();
        let old_manager = auki_network::PeerIdentity::from_seed(&[15u8; 32]).peer_id();
        let promoted = auki_network::PeerIdentity::from_seed(&[16u8; 32]).peer_id();
        let advertised = Arc::new(Mutex::new(None));
        let daemon = DaemonInfo {
            app: "park".into(),
            name: "park".into(),
            session_id: "session-456".into(),
            session_clock_id: "legacy/stale/session-clock".into(),
            session_clock_hash: "legacy-hash".into(),
            app_instance: "instance".into(),
        };
        let session_clock =
            SessionClock::new(promoted.to_string(), daemon.session_id.clone(), "monotonic");
        observe_heartbeat_domain_clock_source(
            &sources,
            "cluster-a",
            old_manager,
            Some(HeartbeatDomainClock {
                cluster_name: "cluster-a".into(),
                domain_clock_id: "cluster-a/domain-clock".into(),
                domain_clock_hash: domain_clock_hash("cluster-a"),
                backing_peer_id: old_manager.to_string(),
                backing_clock_id: "peer/old-manager/session-123/monotonic".into(),
                backing_clock_hash: "old-manager-clock-hash".into(),
                backing_to_domain_offset_ns: 250,
            }),
        );
        let session_clock_hash = session_clock.clock_hash();
        clock_sync.observe(ClockSyncObservation::new(
            session_clock.clock_id(),
            &session_clock_hash,
            "peer/old-manager/session-123/monotonic",
            "old-manager-clock-hash",
            auki_time::NtpSample {
                offset_ns: 1_000_000,
                uncertainty_ns: 20,
                round_trip_ns: 50,
                remote_processing_ns: 30,
                observed_at_clock_ns: 10_000,
            },
        ));

        let source = advertise_promoted_domain_clock_source(
            &advertised,
            &clock_sync,
            &sources,
            "cluster-a",
            promoted,
            &session_clock,
            12_345,
        )
        .expect("promoted Manager should prove inherited domain offset");

        assert_eq!(source.cluster_name, "cluster-a");
        assert_eq!(source.domain_clock_id, "cluster-a/domain-clock");
        assert_eq!(source.domain_clock_hash, domain_clock_hash("cluster-a"));
        assert_eq!(source.backing_peer_id, promoted.to_string());
        assert_eq!(source.backing_clock_id, session_clock.clock_id());
        assert_eq!(source.backing_clock_hash, session_clock.clock_hash());
        assert_eq!(source.backing_to_domain_offset_ns, 1_000_250);
        assert_eq!(*advertised.lock().unwrap(), Some(source.clone()));

        let estimate = estimate_cluster_domain_clock(
            &clock_sync,
            &sources,
            "cluster-a",
            Some(&promoted.to_string()),
            session_clock.clock_id(),
            &session_clock_hash,
            12_345,
        )
        .unwrap();
        assert_eq!(estimate.backing_peer_id, promoted.to_string());
        assert_eq!(estimate.total_offset_ns, 1_000_250);
    }

    #[test]
    fn promoted_manager_does_not_advertise_without_proven_offset() {
        let sources = domain_clock_source_store();
        let clock_sync = auki_time::ClockSyncHandle::default();
        let old_manager = auki_network::PeerIdentity::from_seed(&[17u8; 32]).peer_id();
        let promoted = auki_network::PeerIdentity::from_seed(&[18u8; 32]).peer_id();
        let advertised = Arc::new(Mutex::new(None));
        let daemon = DaemonInfo {
            app: "park".into(),
            name: "park".into(),
            session_id: "session-456".into(),
            session_clock_id: "legacy/stale/session-clock".into(),
            session_clock_hash: "legacy-hash".into(),
            app_instance: "instance".into(),
        };
        let session_clock =
            SessionClock::new(promoted.to_string(), daemon.session_id.clone(), "monotonic");
        let source = HeartbeatDomainClock {
            cluster_name: "cluster-a".into(),
            domain_clock_id: "cluster-a/domain-clock".into(),
            domain_clock_hash: domain_clock_hash("cluster-a"),
            backing_peer_id: old_manager.to_string(),
            backing_clock_id: "peer/old-manager/session-123/monotonic".into(),
            backing_clock_hash: "old-manager-clock-hash".into(),
            backing_to_domain_offset_ns: 0,
        };
        observe_heartbeat_domain_clock_source(&sources, "cluster-a", old_manager, Some(source));

        let err = advertise_promoted_domain_clock_source(
            &advertised,
            &clock_sync,
            &sources,
            "cluster-a",
            promoted,
            &session_clock,
            12_345,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            DomainClockEstimateUnavailable::BackingEstimateUnavailable { .. }
        ));
        assert!(advertised.lock().unwrap().is_none());
    }

    fn domain_time_estimate(total_offset_ns: i64) -> DomainClockEstimate {
        DomainClockEstimate {
            cluster_name: "cluster-a".into(),
            local_clock_id: "peer/local/session-123/monotonic".into(),
            local_clock_hash: "local-clock-hash".into(),
            domain_clock_id: "cluster-a/domain-clock".into(),
            domain_clock_hash: domain_clock_hash("cluster-a"),
            backing_peer_id: "peer/local".into(),
            backing_clock_id: "peer/local/session-123/monotonic".into(),
            backing_clock_hash: "local-clock-hash".into(),
            peer_to_backing_offset_ns: 0,
            backing_to_domain_offset_ns: total_offset_ns,
            total_offset_ns,
            uncertainty_ns: 0,
            observed_at_clock_ns: 12_345,
        }
    }

    #[test]
    fn domain_time_now_conversion_adds_estimated_offset() {
        let estimate = domain_time_estimate(250);

        let domain_now = convert_session_now_to_domain_time(&estimate, 10_000).unwrap();

        assert_eq!(domain_now, 10_250);
    }

    #[test]
    fn domain_time_now_conversion_reports_overflow() {
        let estimate = domain_time_estimate(1);

        let err = convert_session_now_to_domain_time(&estimate, i64::MAX).unwrap_err();

        assert_eq!(
            err,
            DomainTimeNowError::ConversionOutOfRange {
                session_now_ns: i64::MAX,
                offset_ns: 1,
            }
        );
    }

    #[test]
    fn liveness_check_interval_matches_v1_contract() {
        // 1s liveness check / 3s sweep — matches the 2026-05-14
        // Hagall rename (was 3s / 10s under the original
        // aukilabs/discovery#5 contract).
        assert_eq!(LIVENESS_CHECK_INTERVAL, Duration::from_secs(1));
    }

    #[test]
    fn participant_info_uses_session_clock_primitive() {
        let daemon = DaemonInfo {
            app: "boosterapp".into(),
            name: "bracketbot-060".into(),
            session_id: "session-123".into(),
            session_clock_id: "legacy/stale/session-clock".into(),
            session_clock_hash: "legacy-hash".into(),
            app_instance: "abc".into(),
        };
        let local = make_peer(1, 100);
        let mut membership = ClusterMembership::new("foo");
        membership.admit(local.clone());
        let membership = Arc::new(Mutex::new(membership));
        let manager_peer_id = Arc::new(Mutex::new(local.peer_id));
        let cluster_joined_at_ns = Arc::new(Mutex::new(None));
        let clock = SessionClock::new(
            local.peer_id.to_string(),
            daemon.session_id.clone(),
            "monotonic",
        );

        let info = build_participant_info(
            &daemon,
            local.peer_id,
            &manager_peer_id,
            &membership,
            &clock,
            &cluster_joined_at_ns,
        );

        assert_eq!(
            info.session_clock_id,
            format!("{}/session-123/monotonic", local.peer_id)
        );
        assert_eq!(info.session_clock_hash, clock.clock_hash());
        assert!(info.session_now_ns <= clock.now_ns());
    }

    #[test]
    fn registry_envelope_reads_canonical_frame_from_app_root() {
        let dir = tempfile::tempdir().unwrap();
        let peer_id = "K1-FAKE";
        let entry = FrameRegistryEntry::ros_optical(peer_id, "K1-FAKE/head_cam_points");
        let hash = auki_registry::write_frame(dir.path(), &entry)
            .unwrap()
            .hash()
            .to_string();

        let envelope = read_registry_envelope(
            dir.path(),
            peer_id,
            &RegistryRequest {
                kind: RegistryKind::Frame,
                id: entry.frame_id.clone(),
                hash: hash.clone(),
            },
        )
        .unwrap()
        .expect("entry exists");

        assert_eq!(envelope.kind, RegistryKind::Frame);
        assert_eq!(envelope.id, entry.frame_id);
        assert_eq!(envelope.hash, hash);
        verify_registry_envelope(&envelope, RegistryKind::Frame, &envelope.id, &hash).unwrap();
        let decoded: FrameRegistryEntry = serde_json::from_str(&envelope.canonical_json).unwrap();
        assert_eq!(decoded, entry);
    }

    #[test]
    fn registry_envelope_reads_canonical_detector_from_app_root() {
        use auki_registry::{Aruco, DetectorBody, DetectorRegistryEntry};

        let dir = tempfile::tempdir().unwrap();
        let peer_id = "K1-FAKE";
        let entry = DetectorRegistryEntry {
            peer_id: peer_id.into(),
            detector_id: "aukilabs/aruco/v1".into(),
            body: DetectorBody::Aruco(Aruco {
                dictionary: "5x5_50".into(),
            }),
            output_types: vec!["aruco".into()],
        };
        let hash = auki_registry::write_detector(dir.path(), &entry)
            .unwrap()
            .hash()
            .to_string();

        let envelope = read_registry_envelope(
            dir.path(),
            peer_id,
            &RegistryRequest {
                kind: RegistryKind::Detector,
                id: entry.detector_id.clone(),
                hash: hash.clone(),
            },
        )
        .unwrap()
        .expect("entry exists");

        assert_eq!(envelope.kind, RegistryKind::Detector);
        assert_eq!(envelope.id, entry.detector_id);
        assert_eq!(envelope.hash, hash);
        // The independent hash from the entry itself must match the envelope hash.
        assert_eq!(envelope.hash, entry.hash());
        verify_registry_envelope(&envelope, RegistryKind::Detector, &envelope.id, &hash).unwrap();
        let decoded: DetectorRegistryEntry =
            serde_json::from_str(&envelope.canonical_json).unwrap();
        assert_eq!(decoded, entry);
    }

    #[test]
    fn registry_envelope_hash_mismatch_is_rejected_before_decode() {
        let envelope = RegistryEntryEnvelope {
            kind: RegistryKind::Frame,
            id: "frame".into(),
            hash: "deadbeef".into(),
            canonical_json: r#"{"frame_id":"frame"}"#.into(),
        };
        let err = verify_registry_envelope(&envelope, RegistryKind::Frame, "frame", "deadbeef")
            .unwrap_err();
        assert!(matches!(err, FetchRegistryEntryError::HashMismatch { .. }));
    }

    /// Verify `spawn_resources_handler` delegates to `SessionHandle::catalog`
    /// and that variant filtering works.
    #[tokio::test]
    async fn resources_handler_delegates_to_session_handle() {
        use auki_network::resources_protocol::{
            ResourceEntry, ResourcesRequest, ResourcesResponse, SensorKind, Variant,
        };

        let row = sensor_log_resource("galbot", "head_left_rgb", SensorKind::Camera, "rgb");

        struct MockSession(Vec<ResourceEntry>);
        impl SessionHandle for MockSession {
            fn catalog(&self) -> Vec<ResourceEntry> {
                self.0.clone()
            }
        }

        let handle: Arc<Mutex<Option<Arc<dyn SessionHandle>>>> =
            Arc::new(Mutex::new(Some(Arc::new(MockSession(vec![row.clone()])))));

        let (tx, rx) = mpsc::channel(8);
        let provider: Arc<Mutex<Option<Arc<dyn ResourceCatalogProvider>>>> =
            Arc::new(Mutex::new(None));
        let _task = spawn_resources_handler(rx, provider, handle);

        // All variants — should return the row.
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<ResourcesResponse>();
        tx.send(ResourcesRequestEvent {
            peer: libp2p_identity::PeerId::random(),
            request: ResourcesRequest::all(),
            ack: ack_tx,
        })
        .await
        .unwrap();
        let resp = ack_rx.await.unwrap();
        assert_eq!(resp.resources.len(), 1);
        assert_eq!(resp.resources[0].resource_id, "head_left_rgb");

        // Filter for PoseLog only — should return nothing.
        let (ack_tx2, ack_rx2) = tokio::sync::oneshot::channel::<ResourcesResponse>();
        tx.send(ResourcesRequestEvent {
            peer: libp2p_identity::PeerId::random(),
            request: ResourcesRequest {
                variants: vec![Variant::PoseLog],
            },
            ack: ack_tx2,
        })
        .await
        .unwrap();
        let resp2 = ack_rx2.await.unwrap();
        assert!(resp2.resources.is_empty());
    }

    /// Verify an app-supplied ResourceCatalogProvider is the authoritative
    /// source for inbound `/auki/resources` requests when registered.
    #[tokio::test]
    async fn resources_handler_uses_resource_catalog_provider() {
        use auki_network::resources_protocol::{
            ResourceEntry, ResourcesRequest, ResourcesResponse, SensorKind,
        };

        let row = sensor_log_resource(
            "bracketbot",
            "head_pointcloud",
            SensorKind::Rangefinder,
            "point_cloud",
        );

        struct MockProvider(Vec<ResourceEntry>);
        impl ResourceCatalogProvider for MockProvider {
            fn snapshot(&self) -> Vec<ResourceEntry> {
                self.0.clone()
            }
        }

        let provider: Arc<Mutex<Option<Arc<dyn ResourceCatalogProvider>>>> =
            Arc::new(Mutex::new(Some(Arc::new(MockProvider(vec![row.clone()])))));
        let handle: Arc<Mutex<Option<Arc<dyn SessionHandle>>>> = Arc::new(Mutex::new(None));

        let (tx, rx) = mpsc::channel(8);
        let _task = spawn_resources_handler(rx, provider, handle);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<ResourcesResponse>();
        tx.send(ResourcesRequestEvent {
            peer: libp2p_identity::PeerId::random(),
            request: ResourcesRequest::all(),
            ack: ack_tx,
        })
        .await
        .unwrap();

        let resp = ack_rx.await.unwrap();
        assert_eq!(resp.resources.len(), 1);
        assert_eq!(resp.resources[0].resource_id, "head_pointcloud");
    }

    /// Verify the app-supplied provider is sampled for each inbound
    /// `/auki/resources` request instead of caching the catalog once at
    /// provider registration time.
    #[tokio::test]
    async fn resources_handler_snapshots_provider_for_each_request() {
        use auki_network::resources_protocol::{
            ResourceEntry, ResourcesRequest, ResourcesResponse, SensorKind,
        };
        use std::sync::atomic::AtomicUsize;

        struct DynamicProvider {
            calls: AtomicUsize,
        }

        impl ResourceCatalogProvider for DynamicProvider {
            fn snapshot(&self) -> Vec<ResourceEntry> {
                let calls = self.calls.fetch_add(1, Ordering::SeqCst);
                let resource_id = if calls == 0 {
                    "head_left_rgb"
                } else {
                    "head_right_rgb"
                };
                vec![sensor_log_resource(
                    "galbot",
                    resource_id,
                    SensorKind::Camera,
                    "rgb",
                )]
            }
        }

        let provider = Arc::new(DynamicProvider {
            calls: AtomicUsize::new(0),
        });
        let provider_for_assert = provider.clone();
        let provider: Arc<Mutex<Option<Arc<dyn ResourceCatalogProvider>>>> =
            Arc::new(Mutex::new(Some(provider)));
        let handle: Arc<Mutex<Option<Arc<dyn SessionHandle>>>> = Arc::new(Mutex::new(None));

        let (tx, rx) = mpsc::channel(8);
        let _task = spawn_resources_handler(rx, provider, handle);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<ResourcesResponse>();
        tx.send(ResourcesRequestEvent {
            peer: libp2p_identity::PeerId::random(),
            request: ResourcesRequest::all(),
            ack: ack_tx,
        })
        .await
        .unwrap();
        let resp = ack_rx.await.unwrap();
        assert_eq!(resp.resources.len(), 1);
        assert_eq!(resp.resources[0].resource_id, "head_left_rgb");

        let (ack_tx2, ack_rx2) = tokio::sync::oneshot::channel::<ResourcesResponse>();
        tx.send(ResourcesRequestEvent {
            peer: libp2p_identity::PeerId::random(),
            request: ResourcesRequest::all(),
            ack: ack_tx2,
        })
        .await
        .unwrap();
        let resp2 = ack_rx2.await.unwrap();
        assert_eq!(resp2.resources.len(), 1);
        assert_eq!(resp2.resources[0].resource_id, "head_right_rgb");
        assert_eq!(provider_for_assert.calls.load(Ordering::SeqCst), 2);
    }

    fn sensor_log_resource(
        peer_id: &str,
        resource_id: &str,
        kind: auki_network::resources_protocol::SensorKind,
        sensor_type: &str,
    ) -> auki_network::resources_protocol::ResourceEntry {
        use auki_network::resources_protocol::{
            Available, Head, ResourceEntry, SensorBlock, SensorManifestPointer, VariantContent,
        };
        use auki_registry::RegistryRef;

        ResourceEntry {
            source_peer_id: peer_id.into(),
            writer_peer_id: peer_id.into(),
            resource_id: resource_id.into(),
            state: "live".into(),
            head: Some(Head::Rolling {
                retention_ns: 5_000_000_000,
            }),
            extent: None,
            available: Available {
                bytes: 1024,
                entries: 10,
                duration_ns: 5_000_000_000,
            },
            sensor: Some(SensorBlock {
                kind,
                r#type: sensor_type.into(),
                sensor_id: resource_id.into(),
                sensor_hash: "sh".into(),
            }),
            pose: None,
            variant_content: VariantContent::SensorLog {
                manifest: SensorManifestPointer {
                    clock: RegistryRef {
                        peer_id: peer_id.into(),
                        id: "session/sdk_clock".into(),
                        hash: "ch".into(),
                    },
                    frame: None,
                },
            },
        }
    }

    fn make_peer(seed: u8, join_ts: i64) -> ClusterMember {
        ClusterMember {
            peer_id: peer(seed),
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
    fn election_excludes_lost_manager_even_if_transport_still_connected() {
        // A joined first and is still in the runtime's connected set,
        // but the domain heartbeat has already timed it out. B must
        // win so it can rotate Discovery instead of re-electing the
        // dead Manager.
        let m_a = make_peer(1, 100);
        let m_b = make_peer(2, 200);
        let m_c = make_peer(3, 300);
        let mut membership = ClusterMembership::new("foo");
        for m in [m_a.clone(), m_b.clone(), m_c.clone()] {
            membership.admit(m);
        }

        let winner = elect_successor_excluding_lost(
            &membership,
            m_b.peer_id,
            &[m_a.peer_id, m_c.peer_id],
            m_a.peer_id,
        );

        assert_eq!(winner, Some(m_b.peer_id));
    }

    #[test]
    fn transport_close_keeps_last_seen_until_timeout() {
        let peer = auki_network::PeerIdentity::from_seed(&[4u8; 32]).peer_id();
        let mut watched = HashSet::new();
        watched.insert(peer);
        let mut last_heartbeat_at = HashMap::new();
        let first_seen = Instant::now() - Duration::from_millis(250);
        last_heartbeat_at.insert(peer, first_seen);

        note_watched_transport_closed(&watched, &mut last_heartbeat_at, peer);

        assert_eq!(last_heartbeat_at.get(&peer), Some(&first_seen));
    }

    #[test]
    fn heartbeat_refresh_clears_prior_loss_marker() {
        let peer = auki_network::PeerIdentity::from_seed(&[5u8; 32]).peer_id();
        let mut watched = HashSet::new();
        watched.insert(peer);
        let mut last_heartbeat_at = HashMap::new();
        let mut lost_already = HashSet::new();
        lost_already.insert(peer);

        note_watched_peer_alive(&watched, &mut last_heartbeat_at, &mut lost_already, peer);

        assert!(last_heartbeat_at.contains_key(&peer));
        assert!(!lost_already.contains(&peer));
    }

    #[test]
    fn heartbeat_ntp_sample_event_updates_clock_sync_handle() {
        let peer_id = auki_network::PeerIdentity::from_seed(&[10u8; 32]).peer_id();
        let clock_sync = auki_time::ClockSyncHandle::default();

        let estimate = observe_heartbeat_ntp_sample(
            &clock_sync,
            auki_network::network_runtime::HeartbeatNtpSampleObservation {
                peer_id,
                local_clock_id: "peer/local/session-1/monotonic".into(),
                local_clock_hash: "local-hash".into(),
                remote_clock_id: "peer/remote/session-7/monotonic".into(),
                remote_clock_hash: "remote-hash".into(),
                sample: auki_time::NtpSample {
                    offset_ns: 250_000,
                    uncertainty_ns: 20,
                    round_trip_ns: 50,
                    remote_processing_ns: 30,
                    observed_at_clock_ns: 10_000,
                },
            },
        )
        .expect("heartbeat NTP sample should produce a transform estimate");

        assert_eq!(estimate.from_clock_id(), "peer/local/session-1/monotonic");
        assert_eq!(estimate.to_clock_id(), "peer/remote/session-7/monotonic");
        assert_eq!(estimate.offset_ns, 250_000);

        let stored = clock_sync
            .estimate(
                "peer/local/session-1/monotonic",
                "peer/remote/session-7/monotonic",
            )
            .expect("estimate should be retained in shared clock sync handle");
        assert_eq!(stored, estimate);
    }

    #[test]
    fn membership_gossip_manager_must_be_sender_and_member() {
        let manager = make_peer(6, 100);
        let member = make_peer(7, 200);
        let outsider = make_peer(8, 300);
        let mut membership = ClusterMembership::new("foo");
        membership.admit(manager.clone());
        membership.admit(member.clone());

        assert!(valid_membership_update_manager(
            manager.peer_id,
            manager.peer_id,
            &membership
        ));
        assert!(
            !valid_membership_update_manager(member.peer_id, manager.peer_id, &membership),
            "non-Manager sender cannot claim another peer is Manager"
        );
        assert!(
            !valid_membership_update_manager(outsider.peer_id, outsider.peer_id, &membership),
            "advertised Manager must exist in the membership snapshot"
        );
    }

    #[test]
    fn election_empty_membership_returns_none() {
        let membership = ClusterMembership::new("foo");
        let local = auki_network::PeerIdentity::from_seed(&[9u8; 32]).peer_id();
        assert_eq!(elect_successor(&membership, local, &[]), None);
    }

    // ── Manager-loss decision gate ─────────────────────────────────────

    fn peer(seed: u8) -> PeerId {
        auki_network::PeerIdentity::from_seed(&[seed; 32]).peer_id()
    }

    fn discovery_entry(manager: PeerId) -> DiscoveryClusterEntry {
        DiscoveryClusterEntry {
            name: "foo".into(),
            manager_peer_id: manager,
            manager_multiaddrs: vec!["/ip4/10.0.0.9/tcp/4001".parse().unwrap()],
            relay_multiaddrs: vec![],
            peer_count: 2,
            created_ns: 1,
            last_liveness_check_ns: 1,
        }
    }

    #[test]
    fn loss_action_defers_when_discovery_still_names_lost_manager() {
        let (lost, local) = (peer(1), peer(2));
        let entry = discovery_entry(lost);
        assert_eq!(
            decide_manager_loss_action(Some(&entry), lost, local),
            ManagerLossAction::Defer {
                manager_multiaddrs: entry.manager_multiaddrs.clone(),
            }
        );
    }

    #[test]
    fn loss_action_follows_foreign_discovery_manager() {
        let (lost, local, third) = (peer(1), peer(2), peer(3));
        let entry = discovery_entry(third);
        assert_eq!(
            decide_manager_loss_action(Some(&entry), lost, local),
            ManagerLossAction::Follow {
                manager: third,
                manager_multiaddrs: entry.manager_multiaddrs.clone(),
            }
        );
    }

    #[test]
    fn loss_action_elects_when_row_absent() {
        assert_eq!(
            decide_manager_loss_action(None, peer(1), peer(2)),
            ManagerLossAction::ElectLocally
        );
    }

    #[test]
    fn loss_action_elects_when_discovery_names_local_peer() {
        let (lost, local) = (peer(1), peer(2));
        let entry = discovery_entry(local);
        assert_eq!(
            decide_manager_loss_action(Some(&entry), lost, local),
            ManagerLossAction::ElectLocally
        );
    }

    // ── admit_or_refresh ──────────────────────────────────────────────

    #[test]
    fn admit_or_refresh_appends_new_peer() {
        let mut m = ClusterMembership::new("foo");
        let outcome = admit_or_refresh(
            &mut m,
            peer(1),
            vec!["/ip4/10.0.0.1/tcp/4001".parse().unwrap()],
            42,
        );
        assert_eq!(outcome, AdmitOutcome::Admitted);
        assert_eq!(m.peers.len(), 1);
        assert_eq!(m.peers[0].join_ts_ns, 42);
    }

    #[test]
    fn admit_or_refresh_refreshes_existing_multiaddrs_keeps_join_ts() {
        let mut m = ClusterMembership::new("foo");
        admit_or_refresh(
            &mut m,
            peer(1),
            vec!["/ip4/10.0.0.1/tcp/4001".parse().unwrap()],
            42,
        );
        let new_addr: Multiaddr = "/ip4/192.168.1.5/tcp/4001".parse().unwrap();
        let outcome = admit_or_refresh(&mut m, peer(1), vec![new_addr.clone()], 99);
        assert_eq!(outcome, AdmitOutcome::Refreshed);
        assert_eq!(m.peers.len(), 1, "no duplicate entry");
        assert_eq!(m.peers[0].multiaddrs, vec![new_addr]);
        assert_eq!(
            m.peers[0].join_ts_ns, 42,
            "election order must not reshuffle on rejoin"
        );
    }
}
