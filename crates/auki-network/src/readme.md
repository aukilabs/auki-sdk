# `auki-network/src/`

Implementation status for [`auki-network`](../README.md).

## Files

- [`lib.rs`](lib.rs) - feature gates, binding adapter wiring, module exports, and root re-exports.
- [`core.rs`](core.rs) - binding-free peer identity, reachability, capability types, and locked identity tests.
- [`ffi.rs`](ffi.rs) - native UniFFI adapter for identity/capabilities, `AukiNetworkRuntime`, request/response/event facades, byte streams, Discovery, app-instance helpers, and the `message_node` Swift host surface, behind `uniffi`.
- [`wasm.rs`](wasm.rs) - wasm-bindgen adapter for browser identity/protocol helpers, DTO byte helpers, and browser-probe framing, behind `wasm`.
- [`bin/uniffi-bindgen.rs`](bin/uniffi-bindgen.rs) - crate-local UniFFI CLI entry point used by the root binding generator.
- [`participant.rs`](participant.rs) - `ParticipantInfo`, the SDK-owned `/api/info` JSON shape.
- [`browser_probe_protocol.rs`](browser_probe_protocol.rs) - shared `/auki/browser-probe/0.0.1` request/response structs for the browser WebRTC probe.
- [`browser_probe.rs`](browser_probe.rs) - native-only WebRTC Direct probe listener for proving browser peers can open SDK-owned request/response streams.
- [`swarm.rs`](swarm.rs) - libp2p `Swarm<Behaviour>` builder, relay support, advertise-address helpers.
- [`network_runtime.rs`](network_runtime.rs) - task-owned swarm driver, allowed-peer updates, heartbeat carrier targets/events, join/info/resources/sensors/registries/membership helpers, idempotent shutdown.
- [`join_protocol.rs`](join_protocol.rs) - `/auki/join/0.0.1` framed JSON request/response.
- [`heartbeat_protocol.rs`](heartbeat_protocol.rs) - `/auki/heartbeat/0.0.1` bidirectional heartbeat carrier frames.
- [`membership_protocol.rs`](membership_protocol.rs) - `/auki/membership/0.0.1` Manager-id + membership-gossip frames.
- [`info_protocol.rs`](info_protocol.rs) - `/auki/info/0.0.1` framed request/response for `ParticipantInfo` JSON.
- [`resources_protocol.rs`](resources_protocol.rs) - `/auki/resources/0.0.1` framed request/response for live resource catalogs (`sensor_stream` with optional pinhole intrinsics, and `transform_edge` rows in v0).
- [`sensors_protocol.rs`](sensors_protocol.rs) - `/auki/sensors/0.0.1` framed request/response for `SensorEntry` catalogs, with optional embedded Sensor / Frame Registry JSON.
- [`registries_protocol.rs`](registries_protocol.rs) - `/auki/registries/0.0.1` framed request/response for hash-pinned registry entries.
- [`stream_protocol.rs`](stream_protocol.rs) - `/auki/stream/0.1.0` prost framing and re-exports from `auki-proto`.
- [`message_protocol.rs`](message_protocol.rs) - `/auki/message/0.0.1` prost framing for generic peer messages.
- [`message_node.rs`](message_node.rs) - native WebRTC Direct message-node facade for Swift/UniFFI hosts, behind `message_node`.
- [`stream_runtime.rs`](stream_runtime.rs) - typed `Stream<T>` producer/consumer API on top of `stream_protocol`.
- [`discovery_client.rs`](discovery_client.rs) - Discovery HTTP client, behind `discovery_client`.
- [`app_instance.rs`](app_instance.rs) - MAC-derived app-instance helper, behind `app_instance`.

Deleted/obsolete surfaces that should not reappear in docs: `cluster_doc`, `cluster_protocol`, `cluster_runtime`, static `cluster.json` loading, mDNS cluster discovery, and Python `cluster.spawn`.

## Public Surface

Always available:

```rust
pub const PEER_DERIVATION_LABEL: &str = "peer/v1";
pub const MESSAGE_PROTOCOL: &str = "/auki/message/0.0.1";

pub struct PeerIdentity { /* libp2p ed25519 keypair */ }
impl PeerIdentity {
    pub fn from_wallet(wallet: &auki_identity::Wallet) -> Self;
    pub fn from_seed(seed: &[u8; 32]) -> Self;
    pub fn keypair(&self) -> &libp2p_identity::Keypair;
    pub fn private_key_protobuf(&self) -> Vec<u8>;
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

Behind `wasm` on `wasm32`:

```rust
// Exported through wasm-bindgen for the generated JavaScript package.
peerDerivationLabel() -> string
peerIdFromSeed(seed: Uint8Array) -> string
peerIdFromWalletSeed(seed: Uint8Array) -> string
peerPrivateKeyProtobufFromSeed(seed: Uint8Array) -> Uint8Array
peerPrivateKeyProtobufFromWalletSeed(seed: Uint8Array) -> Uint8Array
browserProbeProtocol() -> string
messageProtocol() -> string
encodeBrowserProbeRequest(nonce, payload) -> Uint8Array
decodeBrowserProbeResponse(bytes) -> string
joinProtocol() -> string
infoProtocol() -> string
sensorsProtocol() -> string
resourcesProtocol() -> string
registriesProtocol() -> string
encodeMessageEnvelopeJson(json) -> Uint8Array
decodeMessageEnvelopeJson(bytes) -> string
encodeJoinRequestJson(json) -> Uint8Array
decodeJoinResponseJson(bytes) -> string
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

Behind `message_node`:

```rust
pub mod message_node {
    pub struct MessageNodeConfig {
        pub listen_addresses: Vec<multiaddr::Multiaddr>,
        pub agent_version: String,
    }

    pub struct MessageNodeEvent {
        pub peer_id: libp2p_identity::PeerId,
        pub envelope: auki_proto::message::MessageEnvelope,
    }

    pub struct MessageNode { /* owns a Tokio runtime and WebRTC Direct swarm task */ }
    impl MessageNode {
        pub fn spawn(identity: PeerIdentity, config: MessageNodeConfig) -> Result<Self, MessageNodeError>;
        pub fn local_peer_id(&self) -> libp2p_identity::PeerId;
        pub fn listen_addrs(&self) -> Vec<multiaddr::Multiaddr>;
        pub fn dial(&self, peer_id: libp2p_identity::PeerId, addrs: Vec<multiaddr::Multiaddr>) -> Result<(), MessageNodeError>;
        pub fn send_envelope_bytes(&self, peer_id: libp2p_identity::PeerId, envelope_bytes: Vec<u8>) -> Result<auki_proto::message::MessageAck, MessageNodeError>;
        pub fn next_event(&self) -> Result<Option<MessageNodeEvent>, MessageNodeError>;
        pub fn shutdown(&self);
    }
}
```

Behind `uniffi`, generated Python and Swift bindings expose the native binding
surface. Identity helpers are always available. `AukiNetworkRuntime` wraps the
operational libp2p runtime with binding-safe strings, JSON, byte vectors,
opaque subscriptions, and responder ids for host-language decisions. The
message-node and stream APIs use protobuf bytes so hosts can pair them with
generated `auki-proto` packages in their language.

Native runtime shape:

```text
AukiNetworkRuntime.spawn(config)
runtime.local_peer_id()
runtime.listen_multiaddrs()
runtime.connected_peers()
runtime.set_allowed_peers(peers)
runtime.set_heartbeat_targets(peer_ids)
runtime.drain_membership_events(max)
runtime.drain_liveness_events(max)
runtime.drain_join_requests(max)
runtime.respond_join_json(responder_id, response_json)
runtime.send_join_request_json(peer_id, request_json, timeout_ms)
runtime.open_stream_bytes(peer_id, request_json, payload_kind, timeout_ms)
runtime.drain_stream_open_requests(max)
runtime.accept_stream_open(responder_id, manifest_json)
runtime.push_stream_entry(stream_id, entry)
runtime.finish_stream(stream_id)
runtime.shutdown()
```

Swift shape:

```swift
public class AukiMessageNode {
    public static func fromWalletSeed(seed: Data, listenAddrs: [String], agentVersion: String) throws -> AukiMessageNode
    public func peerId() -> String
    public func listenAddrs() -> [String]
    public func dial(peerId: String, addrs: [String]) throws
    public func sendMessageEnvelopeBytes(peerId: String, envelope: Data) throws -> Data
    public func nextEvent() throws -> AukiMessageEvent?
    public func shutdown()
}

public struct AukiMessageEvent {
    public let peerId: String
    public let envelope: Data
}
```

Python shape:

```python
import auki_network

seed = bytes([3]) * 32
peer_id = auki_network.peer_id_from_wallet_seed(seed)
capabilities = auki_network.networking_capabilities()
node = auki_network.AukiMessageNode.from_wallet_seed(
    seed=seed,
    listen_addrs=[],
    agent_version="auki-python-host/0.0.0",
)
```

`just generate-python-bindings auki-network` generates the local ignored Python package under `bindings/python/auki-network/`. `just generate-swift-bindings auki-network` generates the local ignored SwiftPM package under `bindings/swift/auki-network/` and verifies the iOS/macOS XCFramework build. These packages are generated artifacts; crate-owned source policy lives in `bindings.toml`, `ffi.rs`, and `bindings/{python,swift}/`.

[`examples/ios/AukiNetworkTestApp`](../../../examples/ios/AukiNetworkTestApp) is the committed iOS host for this generated package. It imports generated `auki_network` and generated SwiftProtobuf `AukiProto`; it does not import a separate `auki_identity` Swift package because wallet seed handling stays behind the Rust `AukiMessageNode` facade. The companion `browser-message-smoke.mjs` script uses the generated browser package and js-libp2p to send a length-prefixed `MessageEnvelope` to the iOS app over `/auki/message/0.0.1`.

The generated JavaScript package also exposes `dialBrowserProbe(...)`. It creates a js-libp2p WebRTC Direct peer from Rust-derived key material, dials the native `/auki/browser-probe/0.0.1` protocol, sends a wasm-encoded length-prefixed request, and decodes the native response. The generated `browser-probe-smoke.mjs` plus root `scripts/smoke-network-browser-probe.sh` run that path against the native `browser_probe_listener` example.

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
cargo test -p auki-network --test surface
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
cargo test -p auki-network --features swarm,discovery_client
cargo check -p auki-network --features browser_probe --example browser_probe_listener
cargo test -p auki-network --features browser_probe browser_probe
bash scripts/smoke-network-browser-probe.sh
cargo test -p auki-network --features message_node message_node
python3 scripts/bindings/generate_bindings.py plan python auki-network
just generate-python-bindings auki-network
just generate-javascript-bindings auki-network
DISCOVERY_URL=http://127.0.0.1:8080 cargo test -p auki-network --features discovery_client --test discovery_integration -- --ignored
```
