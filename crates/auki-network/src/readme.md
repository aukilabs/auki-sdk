# `auki-network/src/`

Implementation status for [`auki-network`](../README.md).

## Files

- [`lib.rs`](lib.rs) - always-on peer identity, reachability, capability types, feature-gated module exports, and re-exports.
- [`participant.rs`](participant.rs) - `ParticipantInfo`, the SDK-owned `/api/info` JSON shape.
- [`browser_probe_protocol.rs`](browser_probe_protocol.rs) - shared `/auki/browser-probe/0.0.1` request/response structs for the browser WebRTC probe.
- [`browser_probe.rs`](browser_probe.rs) - native-only WebRTC Direct probe listener for proving browser peers can open SDK-owned request/response streams.
- [`browser_session_protocol.rs`](browser_session_protocol.rs) - `/auki/browser-session/0.0.1` framed JSON browser roster/media presence control plane.
- [`swarm.rs`](swarm.rs) - libp2p `Swarm<Behaviour>` builder, relay support, advertise-address helpers.
- [`network_runtime.rs`](network_runtime.rs) - task-owned swarm driver, allowed-peer updates, heartbeat carrier targets/events, join/info/resources/sensors/registries/membership helpers, idempotent shutdown.
- [`join_protocol.rs`](join_protocol.rs) - `/auki/join/0.0.1` framed JSON request/response.
- [`heartbeat_protocol.rs`](heartbeat_protocol.rs) - `/auki/heartbeat/0.0.1` bidirectional heartbeat carrier frames.
- [`membership_protocol.rs`](membership_protocol.rs) - `/auki/membership/0.0.1` Manager-id + membership-gossip frames.
- [`info_protocol.rs`](info_protocol.rs) - `/auki/info/0.0.1` framed request/response for `ParticipantInfo` JSON.
- [`resources_protocol.rs`](resources_protocol.rs) - `/auki/resources/0.0.1` framed request/response for live resource catalogs (`sensor_stream` with optional pinhole intrinsics, and `transform_edge` rows in v0).
- [`sensors_protocol.rs`](sensors_protocol.rs) - `/auki/sensors/0.0.1` framed request/response for `SensorEntry` catalogs, with optional embedded Sensor / Frame Registry JSON.
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

pub const BROWSER_PROBE_PROTOCOL: &str = "/auki/browser-probe/0.0.1";
pub struct BrowserProbeRequest {
    pub nonce: String,
    pub payload: Vec<u8>,
}
pub struct BrowserProbeResponse {
    pub nonce: String,
    pub payload: Vec<u8>,
    pub responder: String,
}
```

Behind `browser_probe`:

```rust
// Native-only proof feature. Pulls in `swarm` plus `libp2p-webrtc`
// so the SDK can host a WebRTC Direct probe listener for browser peers.
pub mod browser_probe {
    pub fn responder_label(identity: &PeerIdentity) -> String;
    pub fn build_browser_probe_swarm(
        identity: &PeerIdentity,
    ) -> Result<libp2p::Swarm<BrowserProbeBehaviour>, BrowserProbeError>;
    pub async fn listen_and_serve(
        identity: PeerIdentity,
        listen_addr: multiaddr::Multiaddr,
    ) -> Result<(), BrowserProbeError>;
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

pub struct HeartbeatTimestampSource {
    pub clock_id: String,
    pub clock_hash: String,
    pub now_ns: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub domain_clock: Arc<dyn Fn() -> Option<heartbeat_protocol::HeartbeatDomainClock> + Send + Sync>,
}

pub struct HeartbeatDomainClock {
    pub cluster_name: String,
    pub domain_clock_id: String,
    pub domain_clock_hash: String,
    pub backing_peer_id: String,
    pub backing_clock_id: String,
    pub backing_clock_hash: String,
    pub backing_to_domain_offset_ns: i64,
}

pub struct HeartbeatTimingObservation {
    pub peer_id: libp2p_identity::PeerId,
    pub heartbeat: heartbeat_protocol::Heartbeat,
    pub received_at_clock_ns: i64,
    pub local_clock_id: String,
    pub local_clock_hash: String,
}

pub struct HeartbeatNtpSampleObservation {
    pub peer_id: libp2p_identity::PeerId,
    pub local_clock_id: String,
    pub local_clock_hash: String,
    pub remote_clock_id: String,
    pub remote_clock_hash: String,
    pub sample: auki_time::NtpSample,
}

pub struct NetworkRuntime { /* task-owned swarm */ }
impl NetworkRuntime {
    pub fn spawn(
        swarm: libp2p::Swarm<swarm::Behaviour>,
        allowed_peers: Vec<AllowedPeer>,
        stream_provider: stream_runtime::StreamProvider,
        heartbeat_timestamps: HeartbeatTimestampSource,
    ) -> Result<Self, SpawnError>;

    pub async fn set_allowed_peers(&self, peers: Vec<AllowedPeer>) -> Result<UpdateReport, UpdateError>;
    pub async fn set_heartbeat_targets(&self, peers: Vec<libp2p_identity::PeerId>) -> Result<(), UpdateError>;
    pub fn connected_peers(&self) -> Vec<libp2p_identity::PeerId>;
    pub fn shutdown(&self);

    pub async fn send_join_request(...);
    pub async fn request_participant_info(...);
    pub async fn request_resources_catalog(...);
    pub async fn request_resources_catalog_with(...);
    pub async fn request_sensors_catalog(...);
    pub async fn request_sensors_catalog_with(...);
    pub async fn request_registry_entry(...);
    pub fn broadcast_membership(manager_peer_id, membership_json);
}
```

Heartbeat frames carry `sent_at_unix_ns`, sender `clock_id` / `clock_hash`, a sequence number, `sent_at_clock_ns`, an optional echo of the last peer heartbeat as `(sequence, received_at_clock_ns)`, and optional `domain_clock` source metadata. The runtime does not invent a timestamp clock; `auki-domain` supplies the session monotonic clock from daemon info. The runtime only copies optional domain-clock metadata into outbound frames; it does not decide whether a peer has domain time.

`PeerLivenessEvent::HeartbeatReceived` carries a `HeartbeatTimingObservation`. If the received heartbeat echoes one of this runtime's remembered outbound heartbeat sequences, the runtime also emits `PeerLivenessEvent::HeartbeatNtpSampleObserved` with a raw `auki-time::NtpSample`. The runtime still does not choose a domain clock or produce cluster transforms.

Stream payloads and dispatch:

```rust
pub enum StreamDispatch {
    AcceptCamera { manifest: StreamManifest, source: SourceStream<CameraFrame> },
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

`auki-domain::ClusterManager` owns cluster semantics: create/join/bootstrap, membership mutation, heartbeat topology and timeouts, Manager election, Discovery liveness checks, Manager rotation, participant info, resource/sensor catalog providers, and stream access as the daemon-facing API.

## Verification

README edits are doc-only. For code changes in this crate, the usual local checks are:

```bash
cargo test -p auki-network --features swarm,discovery_client
cargo check -p auki-network --features browser_probe --example browser_probe_listener
cargo test -p auki-network --features browser_probe browser_probe
DISCOVERY_URL=http://127.0.0.1:8080 cargo test -p auki-network --features discovery_client --test discovery_integration -- --ignored
```
