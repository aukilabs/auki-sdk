# `auki-network/src/`

Implementation status for [`auki-network`](../README.md).

## Files

- [`lib.rs`](lib.rs) - always-on peer identity, reachability, capability types, feature-gated module exports, and re-exports.
- [`participant.rs`](participant.rs) - `ParticipantInfo`, the SDK-owned `/api/info` JSON shape.
- [`swarm.rs`](swarm.rs) - libp2p `Swarm<Behaviour>` builder, relay support, advertise-address helpers.
- [`network_runtime.rs`](network_runtime.rs) - task-owned swarm driver, allowed-peer updates, join/info/sensors/registries/membership helpers, peer-liveness events, idempotent shutdown.
- [`join_protocol.rs`](join_protocol.rs) - `/auki/join/0.0.1` framed JSON request/response.
- [`heartbeat_protocol.rs`](heartbeat_protocol.rs) - `/auki/heartbeat/0.0.1` pairwise liveness frames.
- [`membership_protocol.rs`](membership_protocol.rs) - `/auki/membership/0.0.1` membership-gossip frames.
- [`info_protocol.rs`](info_protocol.rs) - `/auki/info/0.0.1` framed request/response for `ParticipantInfo` JSON.
- [`sensors_protocol.rs`](sensors_protocol.rs) - `/auki/sensors/0.0.1` framed request/response for `SensorEntry` catalogs.
- [`registries_protocol.rs`](registries_protocol.rs) - `/auki/registries/0.0.1` framed request/response for hash-pinned registry entries.
- [`stream_protocol.rs`](stream_protocol.rs) - `/auki/stream/0.1.0` prost framing and re-exports from `auki-datatypes`.
- [`stream_runtime.rs`](stream_runtime.rs) - typed `Stream<T>` producer/consumer API on top of `stream_protocol`.
- [`discovery_client.rs`](discovery_client.rs) - Discovery HTTP client, behind `discovery_client`.
- [`app_instance.rs`](app_instance.rs) - MAC-derived app-instance helper, behind `app_instance`.

Deleted/obsolete surfaces that should not reappear in docs: `cluster_doc`, `cluster_protocol`, `cluster_runtime`, static `cluster.json` loading, mDNS cluster discovery, and Python `cluster.spawn`.

## Public Surface

Always available:

```rust
pub const PEER_DERIVATION_LABEL: &str = "peer/v1";

pub struct PeerIdentity { /* libp2p ed25519 keypair */ }
impl PeerIdentity {
    pub fn from_wallet(wallet: &auki_identity::Wallet) -> Self;
    pub fn from_seed(seed: &[u8; 32]) -> Self;
    pub fn keypair(&self) -> &libp2p_identity::Keypair;
    pub fn public_key(&self) -> libp2p_identity::PublicKey;
    pub fn peer_id(&self) -> libp2p_identity::PeerId;
}

pub struct ReachabilityRecord {
    pub peer_id: libp2p_identity::PeerId,
    pub addresses: Vec<multiaddr::Multiaddr>,
    pub capabilities: Vec<Capability>,
    pub last_seen_ns: i64,
}

pub struct Capability(pub String);

pub struct ParticipantInfo {
    pub app: String,
    pub name: String,
    pub session_id: String,
    pub session_clock_id: String,
    pub session_clock_hash: String,
    pub session_now_ns: u64,
    pub cluster_joined_at_ns: Option<u64>,
    pub peer_id: libp2p_identity::PeerId,
    pub app_instance: String,
    pub is_manager: bool,
    pub manager_peer_id: String,
}
```

Behind `swarm`:

```rust
pub mod swarm {
    pub struct SwarmConfig {
        pub listen_addresses: Vec<multiaddr::Multiaddr>,
        pub agent_version: String,
        pub enable_relay_server: bool,
    }

    pub fn build_swarm(
        identity: &PeerIdentity,
        config: SwarmConfig,
    ) -> Result<libp2p::Swarm<Behaviour>, BuildError>;

    pub fn dial_peer(...);
    pub fn is_routable_multiaddr(addr: &multiaddr::Multiaddr) -> bool;
    pub async fn collect_routable_listen_addrs(...);
    pub async fn resolve_advertise_multiaddrs(...);
}

pub struct AllowedPeer {
    pub peer_id: libp2p_identity::PeerId,
    pub multiaddrs: Vec<multiaddr::Multiaddr>,
}

pub struct NetworkRuntime { /* task-owned swarm */ }
impl NetworkRuntime {
    pub fn spawn(
        swarm: libp2p::Swarm<swarm::Behaviour>,
        allowed_peers: Vec<AllowedPeer>,
        stream_provider: stream_runtime::StreamProvider,
    ) -> Result<Self, SpawnError>;

    pub async fn set_allowed_peers(&self, peers: Vec<AllowedPeer>) -> Result<UpdateReport, UpdateError>;
    pub fn connected_peers(&self) -> Vec<libp2p_identity::PeerId>;
    pub fn shutdown(&self);

    pub async fn send_join_request(...);
    pub async fn request_participant_info(...);
    pub async fn request_sensors_catalog(...);
    pub fn broadcast_membership(...);
}
```

Stream payloads and dispatch:

```rust
pub enum StreamDispatch {
    AcceptJpeg { manifest: StreamManifest, source: SourceStream<JpegFrame> },
    AcceptPointCloud { manifest: StreamManifest, source: SourceStream<PointCloudFrame> },
    AcceptJointEncoders { manifest: StreamManifest, source: SourceStream<JointEncodersFrame> },
    AcceptAudio { manifest: StreamManifest, source: SourceStream<AudioFrame> },
    Decline { reason: DeclineReason },
}

pub type StreamProvider =
    Arc<dyn Fn(libp2p_identity::PeerId, StreamRequest) -> StreamDispatch + Send + Sync>;
```

Behind `discovery_client`:

```rust
pub struct ClusterEntry {
    pub name: String,
    pub manager_peer_id: libp2p_identity::PeerId,
    pub manager_multiaddrs: Vec<multiaddr::Multiaddr>,
    pub peer_count: u32,
    pub created_ns: i64,
    pub last_liveness_check_ns: i64,
}

pub enum CreateClusterOutcome { Created(ClusterEntry), AlreadyExists }

impl DiscoveryClient {
    pub fn new(base_url: impl Into<String>) -> Self;
    pub fn with_http(base_url: impl Into<String>, http: reqwest::Client) -> Self;
    pub fn base_url(&self) -> &str;
    pub async fn list_clusters(&self) -> Result<Vec<ClusterEntry>, DiscoveryError>;
    pub async fn create_cluster(...);
    pub async fn liveness_check(...);
    pub async fn rotate_manager(...);
    pub async fn deregister(...);
}
```

## Trust Model

`NetworkRuntime` is plumbing, not policy. It tracks the current allowed peers handed to it by `auki-domain`, auto-dials their multiaddrs, and uses that set to gate member-only protocols. It intentionally allows the join protocol to be reached by non-members so a new peer can ask to join.

`auki-domain::ClusterManager` owns cluster semantics: create/join/bootstrap, membership mutation, Manager election, Discovery liveness checks, Manager rotation, participant info, sensor catalog providers, and stream access as the daemon-facing API.

## Verification

README edits are doc-only. For code changes in this crate, the usual local checks are:

```bash
cargo test -p auki-network --features swarm,discovery_client
DISCOVERY_URL=http://127.0.0.1:8080 cargo test -p auki-network --features discovery_client --test discovery_integration -- --ignored
```
