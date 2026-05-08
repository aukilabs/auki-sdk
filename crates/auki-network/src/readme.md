# `auki-network/src/`

Networking substrate for the SDK. Spec: this crate's [outer `README.md`](../README.md).

## What's here

- [`lib.rs`](lib.rs) — M0 data types: `PeerIdentity`, `ReachabilityRecord`, `Capability`, plus the `multiaddr_vec_serde` adapter.
- [`cluster_doc.rs`](cluster_doc.rs) — `cluster.json` discovery-doc loader (ansuz #1). Always available (no feature gate); `std::fs`-based, runs on native targets. Public types: `ClusterDoc`, `ClusterPeer`, `LoadError`. Public fns: `load`, `default_path`, `resolve_path`. Public consts: `SUPPORTED_VERSION = 1`, `ENV_OVERRIDE = "AUKI_CLUSTER_DOC"`, `DEFAULT_RELATIVE_PATH = "registries/cluster_registries/cluster.json"`.
- [`participant.rs`](participant.rs) — `ParticipantInfo`, the wire shape exchanged over `GET /api/info` (HTTP) and the `/auki/cluster/0.0.1` participant protocol (libp2p). M0 — available without the `swarm` feature.
- [`swarm.rs`](swarm.rs) — M1 libp2p `Swarm` builder, gated behind the `swarm` feature.
- [`cluster_protocol.rs`](cluster_protocol.rs) — `/auki/cluster/0.0.1` request-response protocol (ansuz #3), gated behind the `swarm` feature. Wraps `libp2p::request_response::json::Behaviour<ClusterRequest, ParticipantInfo>`; wired into `swarm::Behaviour` as the always-on `cluster:` field.
- [`cluster_runtime.rs`](cluster_runtime.rs) — opaque runtime that owns a `Swarm<Behaviour>` + tokio task and orchestrates the cluster (ansuz #4), gated behind the `swarm` feature. Auto-dials peers in a `ClusterDoc`, exchanges `ParticipantInfo`, exposes the live peer state via `peers()`, reconnects with per-peer exponential backoff. The wrapper `auki-network-py` `cluster.spawn` is built on top of this.
- [`stream_protocol.rs`](stream_protocol.rs) — `/auki/stream/0.1.0` typed-byte-stream wire primitives (grimsby #1, extended by Dagaz Batch 1), gated behind the `swarm` feature. Public types: `StreamRequest`, `StreamMessage<T>`, `AcceptInfo`, `DeclineReason`, `EndReason`, `JpegFrame` (`T` for grimsby v1), `PointCloudFrame` (`T` for Dagaz Batch 1; raw CDR `bytes` field, base64-encoded inside JSON via the local `base64_bytes` adapter to dodge the array-of-integers tax), `StreamProtocolError`. Public consts: `STREAM_PROTOCOL = "/auki/stream/0.1.0"`, `MAX_FRAME_BYTES = 16 MiB`. Public fns: `read_message<T>` / `write_message<T>` (length-prefixed JSON framing helpers over `futures::AsyncRead/Write`). The actual swarm-side multiplexer is `libp2p_stream::Behaviour`, wired into `swarm::Behaviour` as the always-on `stream:` field.
- [`stream_runtime.rs`](stream_runtime.rs) — typed `Stream<T>` Rust API on top of `stream_protocol`'s wire primitives (grimsby #2 + #3, lifted to multi-`T` dispatch by Dagaz Batch 1), gated behind the `swarm` feature. Producer-side: `ProducerFrame<T>`, `SourceStream<T>`, `StreamDispatch` (closed enum: `AcceptJpeg` / `AcceptPointCloud` / `Decline`), `StreamProvider` (non-generic), `decline_all_streams()` convenience for consumer-only nodes. Consumer-side: `ConsumerFrame<T>`, `StreamSubscription<T>`, `StreamError`, `OpenStreamError`, `OPEN_STREAM_TIMEOUT = 30s` — generic over `T` per call. The `open_stream<T>(peer_id, request)` async method on `ClusterRuntime` opens outbound subscriptions; the runtime task spawns per-substream `handle_inbound_substream` (non-generic outer + a generic `pump_typed::<T>` helper monomorphized per variant) for each accepted inbound substream on `STREAM_PROTOCOL`. Cluster-doc trust boundary applies — outsiders' substreams are dropped silently. Per Dagaz D1: each substream is mono-`T`; the producer dispatches by `request.sensor_id` to pick which `StreamDispatch` variant per call.
- [`app_instance.rs`](app_instance.rs) — per-machine identifier derivation (ansuz #5), gated behind the `app_instance` feature.
- [`discovery_client.rs`](discovery_client.rs) — REST client for [`aukilabs/discovery`](https://github.com/aukilabs/discovery) (Vinland Batch 1 piece #2), gated behind the `discovery_client` feature. Public types: `DiscoveryClient`, `DiscoveryError`. Public consts: `DEFAULT_TIMEOUT = 30s`. Methods: `new`, `with_http`, `base_url`; async `register`, `fetch`, `deregister`. Wire shape locked against Discovery's verifier — JCS-canonical signing payload includes `cluster_name` (cross-cluster replay guard); base64-32 ed25519 pubkey + base64-64 signature on the wire; ±60s replay window. Deregister signs `{cluster_name, peer_id, op: "delete", timestamp_ns}` (no `public_key` in the canonical bytes — `verify_peer_id` already binds it).

## Public types

```rust
// M0 (always available)
pub struct PeerIdentity { /* libp2p Keypair (ed25519), sensitive */ }

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
}

pub const PEER_DERIVATION_LABEL: &str = "peer/v1";

// `cluster_doc` module (always available, native-only)
pub mod cluster_doc {
    pub struct ClusterDoc {
        pub version: u32,
        pub cluster_name: String,
        pub peers: Vec<ClusterPeer>,
    }
    pub struct ClusterPeer {
        pub peer_id: libp2p_identity::PeerId,
        pub addresses: Vec<multiaddr::Multiaddr>,
        pub expected_app_id: Option<String>,
        pub note: Option<String>,
    }
    pub enum LoadError { Io(std::io::Error), Parse(serde_json::Error), UnsupportedVersion(u32), InvalidPeerId(String), InvalidMultiaddr(String) }
    pub const SUPPORTED_VERSION: u32 = 1;
    pub const ENV_OVERRIDE: &str = "AUKI_CLUSTER_DOC";
    pub const DEFAULT_RELATIVE_PATH: &str = "registries/cluster_registries/cluster.json";
    pub fn load(path: &Path) -> Result<ClusterDoc, LoadError>;
    pub fn default_path(app_root: &Path) -> PathBuf;
    pub fn resolve_path(app_root: &Path, cli_override: Option<&Path>) -> PathBuf;
}

// M1 (behind `swarm` feature)
pub mod swarm {
    pub struct Behaviour {
        /* identify + ping + Toggle<mdns> + relay_client + Toggle<relay> + cluster + stream */
    }
    pub struct SwarmConfig {
        listen_addresses: Vec<Multiaddr>,
        agent_version: String,
        enable_mdns: bool,           // default true
        enable_relay_server: bool,   // default false
    }
    pub enum BuildError { Transport(String), Listen { addr, source } }
    pub const IDENTIFY_PROTOCOL: &str = "/auki/identify/0.0.1";
    pub fn build_swarm(identity: &PeerIdentity, config: SwarmConfig) -> Result<libp2p::Swarm<Behaviour>, BuildError>;
    pub fn dial_peer(swarm: &mut Swarm<Behaviour>, peer: PeerId, addresses: Vec<Multiaddr>) -> Result<(), DialError>;
}

// ansuz #3 (behind `swarm` feature)
pub mod cluster_protocol {
    pub const CLUSTER_PROTOCOL: &str = "/auki/cluster/0.0.1";
    pub const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    pub struct ClusterRequest;                       // unit struct → JSON `null`
    pub type ClusterResponse = ParticipantInfo;
    pub type Behaviour =
        libp2p::request_response::json::Behaviour<ClusterRequest, ClusterResponse>;

    pub fn behaviour() -> Behaviour;
}

// grimsby #2 + #3 (behind `swarm` feature)
pub mod stream_runtime {
    pub const OPEN_STREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    pub struct ProducerFrame<T> { pub timestamp_ns: i64, pub payload: T }

    pub type SourceStream<T> =
        Pin<Box<dyn Stream<Item = Result<ProducerFrame<T>, String>> + Send>>;

    /// Closed-set producer dispatch (Dagaz Batch 1). New `T` is a
    /// coordinated SDK + consumer release: bump auki-network, add the
    /// variant, every consumer that wants the new sensor type opts in.
    pub enum StreamDispatch {
        AcceptJpeg       { info: AcceptInfo, source: SourceStream<JpegFrame>       },
        AcceptPointCloud { info: AcceptInfo, source: SourceStream<PointCloudFrame> },
        Decline          { reason: DeclineReason },
    }

    pub type StreamProvider = Arc<dyn Fn(StreamRequest) -> StreamDispatch + Send + Sync>;

    pub fn decline_all_streams() -> StreamProvider;

    pub struct ConsumerFrame<T> { pub timestamp_ns: i64, pub seq: u64, pub payload: T }

    pub struct StreamSubscription<T> {
        pub info: AcceptInfo,
        pub frames: Pin<Box<
            dyn Stream<Item = Result<ConsumerFrame<T>, StreamError>> + Send,
        >>,
    }

    pub enum StreamError {
        EndOfStream { reason: EndReason },
        ConnectionLost,
        Protocol(StreamProtocolError),
    }

    pub enum OpenStreamError {
        Declined { reason: DeclineReason },
        LibP2p(libp2p_stream::OpenStreamError),
        Protocol(StreamProtocolError),
        Timeout(std::time::Duration),
    }

    impl ClusterRuntime {
        pub async fn open_stream<T>(
            &self,
            peer_id: libp2p::PeerId,
            request: StreamRequest,
        ) -> Result<StreamSubscription<T>, OpenStreamError>
        where T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static;
    }
}

// grimsby #1 (behind `swarm` feature)
pub mod stream_protocol {
    pub const STREAM_PROTOCOL: &str = "/auki/stream/0.1.0";
    pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;          // 16 MiB

    pub struct StreamRequest { pub sensor_id: String }
    pub struct AcceptInfo {
        pub sensor_hash: String,
        pub clock_id:    String,
        pub clock_hash:  String,
    }
    pub enum DeclineReason {
        SensorNotFound,
        SensorUnavailable,
        ProducerShuttingDown,
        Other { detail: String },
    }
    pub enum EndReason {
        SourceEnded,
        ProducerShuttingDown,
        SessionEnded,
        ProducerError { detail: String },
    }
    pub enum StreamMessage<T> {                                  // tagged "kind" on JSON
        Request(StreamRequest),
        Accept(AcceptInfo),
        Decline { reason: DeclineReason },
        Frame { timestamp_ns: i64, seq: u64, payload: T },
        EndOfStream { reason: EndReason },
    }
    pub struct JpegFrame { pub bytes: Vec<u8> }                  // T for grimsby v1; bytes serialize as JSON int-array
    pub struct PointCloudFrame {                                  // T for Dagaz Batch 1; raw CDR PointCloud2
        #[serde(with = "base64_bytes")]                           // bytes serialize as base64 JSON string
        pub bytes: Vec<u8>,
    }

    pub enum StreamProtocolError { Io, Serialize, Deserialize, FrameTooLarge, EmptyFrame }

    pub async fn write_message<T: Serialize, S: AsyncWrite + Unpin>(
        stream: &mut S,
        msg: &StreamMessage<T>,
    ) -> Result<(), StreamProtocolError>;

    pub async fn read_message<T: DeserializeOwned, S: AsyncRead + Unpin>(
        stream: &mut S,
    ) -> Result<StreamMessage<T>, StreamProtocolError>;
}

// ansuz #4 (behind `swarm` feature)
pub mod cluster_runtime {
    pub const INITIAL_BACKOFF: std::time::Duration = std::time::Duration::from_secs(1);
    pub const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(60);
    pub const RECONNECT_TICK: std::time::Duration = std::time::Duration::from_millis(500);

    pub type ParticipantInfoProvider = std::sync::Arc<
        dyn Fn() -> Option<ParticipantInfo> + Send + Sync,
    >;
    // Returning `None` tells the runtime to drop the inbound request's
    // reply channel without sending a response — requester sees a
    // request timeout. Use cases: session clock not yet bound (sidecar
    // mid-startup), Python participant_provider raised an exception,
    // any other transient inability to fill in valid info.

    pub enum SpawnError {
        BuildSwarm(swarm::BuildError),
        NoTokioRuntime,
    }

    pub struct PeerSnapshot {
        pub peer_id: libp2p::PeerId,
        pub info: ParticipantInfo,
        pub first_seen_ns: u64,                      // sticky per peer-session
    }

    pub struct ClusterRuntime { /* state + task + shutdown handles */ }

    impl ClusterRuntime {
        pub fn spawn(
            seed: [u8; 32],
            doc: ClusterDoc,
            swarm_config: SwarmConfig,
            participant_provider: ParticipantInfoProvider,
            stream_provider: stream_runtime::StreamProvider,
        ) -> Result<Self, SpawnError>;

        pub fn from_swarm(
            swarm: libp2p::Swarm<swarm::Behaviour>,
            doc: ClusterDoc,
            participant_provider: ParticipantInfoProvider,
            stream_provider: stream_runtime::StreamProvider,
        ) -> Result<Self, SpawnError>;

        pub fn peers(&self) -> Vec<PeerSnapshot>;

        pub fn shutdown(self);
    }
}

// ansuz #5 (behind `app_instance` feature)
pub mod app_instance {
    pub enum DeriveError {
        NoNetworkInterfaces,
        NoSuitableMac,
        Io(std::io::Error),
    }
    pub fn derive() -> Result<String, DeriveError>;
    pub fn derive_from(macs: &[[u8; 6]]) -> Result<String, DeriveError>;
}

// Vinland Batch 1 piece #2 (behind `discovery_client` feature)
pub mod discovery_client {
    pub const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    pub struct DiscoveryClient { /* base_url + reqwest::Client */ }

    pub enum DiscoveryError {
        Transport(reqwest::Error),
        Status { status: u16, body: String },
        Clock(String),
    }

    impl DiscoveryClient {
        pub fn new(url: impl Into<String>) -> Self;
        pub fn with_http(url: impl Into<String>, http: reqwest::Client) -> Self;
        pub fn base_url(&self) -> &str;

        pub async fn register(
            &self,
            wallet: &auki_identity::Wallet,
            cluster_name: &str,
            addresses: &[multiaddr::Multiaddr],
            expected_app_id: Option<&str>,
            note: Option<&str>,
        ) -> Result<cluster_doc::ClusterDoc, DiscoveryError>;

        pub async fn fetch(
            &self,
            cluster_name: &str,
        ) -> Result<cluster_doc::ClusterDoc, DiscoveryError>;

        pub async fn deregister(
            &self,
            wallet: &auki_identity::Wallet,
            cluster_name: &str,
        ) -> Result<(), DiscoveryError>;
    }
}
```

## Public functions

```rust
impl PeerIdentity {
    pub fn from_wallet(wallet: &auki_identity::Wallet) -> Self;
    pub fn from_seed(seed: &[u8; 32]) -> Self;
    pub fn keypair(&self) -> &libp2p_identity::Keypair;
    pub fn public_key(&self) -> libp2p_identity::PublicKey;
    pub fn peer_id(&self) -> libp2p_identity::PeerId;
}

impl Capability {
    pub const MESSAGE_FORWARDING: &str = "networking:message-forwarding";
    pub const BULK_DATA_CHANNEL:  &str = "networking:bulk-data-channel";
    pub const TURN:               &str = "networking:turn";
    pub const SFU:                &str = "networking:sfu";

    pub fn new(s: impl Into<String>) -> Self;
    pub fn as_str(&self) -> &str;
    pub fn namespace(&self) -> Option<&str>;
}

impl From<&str>   for Capability;
impl From<String> for Capability;
```

## How `PeerIdentity::from_wallet` works

```text
peer_seed   = Wallet::derive_child("peer/v1").seed()
ed_keypair  = ed25519::Keypair::from_secret(peer_seed)
keypair     = libp2p_identity::Keypair::from(ed_keypair)
peer_id     = keypair.public().to_peer_id()      // protobuf + multihash
```

A backup of the wallet seed is sufficient to regenerate the peer identity. The derivation label `"peer/v1"` is fixed; rotating to `"peer/v2"` would be a coordinated SDK + consumer change.

## How `build_swarm` works (M1)

```text
SwarmBuilder::with_existing_identity(identity.keypair().clone())
    .with_tokio()
    .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)
    .with_quic()
    .with_relay_client(noise::Config::new, yamux::Config::default)
    .with_behaviour(|key, relay_client| Behaviour {
        identify:     identify::Behaviour::new(/* protocol /auki/identify/0.0.1, agent_version */),
        ping:         ping::Behaviour::default(),
        mdns:         Toggle::from(enable_mdns.then(|| mdns::tokio::Behaviour::new(...))),
        relay_client,
        relay:        Toggle::from(enable_relay_server.then(|| relay::Behaviour::new(local_pid, ...))),
    })
    .with_swarm_config(|c| c.with_idle_connection_timeout(60s))
    .build()
    + listen_on(each address in config.listen_addresses)
```

mDNS is constructed outside the closure because its constructor is fallible — the closure can only return `Behaviour` directly (or `Result<Behaviour, Box<dyn Error>>`), so mDNS errors are surfaced as `BuildError::Transport` before the swarm is built.

`build_swarm` does the listening — caller doesn't need to call `swarm.listen_on` afterwards.

## How `cluster_protocol` works (ansuz #3)

The behaviour is `libp2p::request_response::json::Behaviour` over the protocol id `/auki/cluster/0.0.1`. Request body is the unit struct `ClusterRequest` (serializes as JSON `null` — empty by design); response is `ParticipantInfo` (same JSON as `GET /api/info`). One round-trip per query; 30 s per-request timeout.

The behaviour is wired into the swarm `Behaviour` struct as the always-on `cluster:` field — there is no `Toggle`. Swarms that don't participate in a cluster (the dedicated `aukilabs/relay` infrastructure node) just never see traffic on it; a knob would have been ceremony.

The behaviour does **not** auto-respond. A peer that receives a request gets `request_response::Event::Message::Request{ channel, .. }` and is responsible for filling in its current `ParticipantInfo` and calling `behaviour.cluster.send_response(channel, info)`. This is the standard libp2p pattern, and it's what lets a Python sidecar's `participant_provider` callable invoke per-request so `session_now_ns` is fresh on every reply rather than stale at swarm-spawn time.

```text
A → B   ClusterRequest                     (JSON null)
B → A   ParticipantInfo of B               (same shape as GET /api/info)
```

The JSON is byte-for-byte identical to the `participant::golden_bytes_match_fixture` fixture — the codec uses `serde_json` end-to-end. Length framing is the underlying libp2p stream's, not application-layer.

Higher-level orchestration (auto-dialing peers from `cluster.json`, tracking `Joined`/`Left`, holding a peer state map) lives in the `cluster_runtime` module (ansuz #4 — shipped in v0.0.13); Rust consumers that want fine control (Sentinel) drive the swarm event loop themselves.

## How `cluster_runtime` works (ansuz #4)

The runtime takes a `ClusterDoc`, a `SwarmConfig` (or a pre-built `Swarm<Behaviour>` via `from_swarm`), and a `participant_provider` callable. It spawns a tokio task that owns the swarm and drives the cluster:

```text
                          ┌─────────────────────┐
                          │  ClusterDoc         │  pinned peers
                          └─────────┬───────────┘
                                    │
   ┌──────── ConnectionEstablished ─┴──────────────┐
   │                                                │
   │  for known peer:                               │
   │     send ClusterRequest                        │
   │     reset backoff                              │
   │                                                │
   │  on inbound Request from known peer:           │
   │     info = participant_provider()              │
   │     send_response(channel, info)               │
   │                                                │
   │  on inbound Request from unknown peer:         │
   │     drop channel (silent — doc is the          │
   │     trust boundary)                            │
   │                                                │
   │  on Response from known peer:                  │
   │     state.peers[pid].info = response           │
   │     state.peers[pid].connected = true          │
   │     if new session_id: reset first_seen_ns     │
   │                                                │
   │  on ConnectionClosed / OutgoingError:          │
   │     state.peers[pid].connected = false         │
   │     schedule retry @ now + backoff             │
   │     backoff = min(backoff * 2, MAX_BACKOFF)    │
   │                                                │
   │  every RECONNECT_TICK (500ms):                 │
   │     for each peer with next_dial_at <= now:    │
   │        if !is_connected: dial_peer(addrs)      │
   └────────────────────────────────────────────────┘
```

The runtime mutates only its own state map; it does not change the `ParticipantInfo` flowing through `participant_provider`. The consumer is responsible for setting `cluster_joined_at_ns` on its own outbound info — they read `peers()` to know whether at least one peer has connected, and set the field once on first non-empty `peers()`.

`peers()` returns `PeerSnapshot { peer_id, info, first_seen_ns }` for every entry where `connected: true`. Disconnected entries are retained internally so `first_seen_ns` survives a same-session reconnect; `peers()` filters them out. A peer-session change (different `session_id` in their response) replaces the entry and resets `first_seen_ns`.

`shutdown(self)` and the `Drop` impl both signal the task and abort it. Connections close at the TCP layer when the swarm drops. Idempotent in practice — `shutdown` consumes self and the unconsumed path runs the same `cleanup` from `Drop`.

The runtime is opaque by design: consumers don't drive the swarm event loop themselves. The Python sidecar in Boosterapp can't drive an async libp2p loop from Python and just wants `peers()` from the HTTP request handler thread; `auki-network-py`'s `cluster.spawn` will wrap this. Sentinel and other Rust consumers that want fine control use `cluster_protocol::Behaviour` directly and skip this module.

## How `app_instance::derive` works (ansuz #5)

```text
macs = mac_address::MacAddressIterator::new()  // platform-specific syscalls
candidates = macs
    .filter(|m| m != [0; 6])                   // skip loopback
    .filter(|m| m[0] & 0x02 == 0)              // skip locally-administered
candidates.sort()                                // lexicographic by raw bytes
first = candidates.first()
output = format!("{:02x}{:02x}…")                // 12 lowercase hex chars
```

Errors: `NoNetworkInterfaces` if the iterator yields nothing; `NoSuitableMac` if everything is filtered out (typical in containers); `Io(std::io::Error)` if the underlying syscall fails. `derive_from(&[[u8; 6]])` is the same logic exposed as the testing seam.

## `dial_peer` helper

```rust
swarm::dial_peer(&mut swarm, peer_id, vec![addr1, addr2, ...])
```

The addresses may be direct or circuit-relay-mediated. The swarm picks among them; the relay-client behaviour handles routing transparently. Park-from-home (Reid parking-lot 3c) is operator-paste of `(peer-id, [optional relay multiaddr])` into Park's UI; Park calls this helper.

## Serde shape

`ReachabilityRecord`, `Capability`, and `ParticipantInfo` round-trip through JSON. `PeerId` serializes as its canonical multibase-base58 string (via `libp2p-identity`'s `serde` feature). `Multiaddr` lacks serde in `multiaddr` 0.18, so the crate ships a small adapter that serializes each as its text form (`/ip4/.../tcp/...`). `ParticipantInfo` uses snake-case field names directly (no `#[serde(rename_all)]` needed) and serializes `cluster_joined_at_ns: None` as explicit `null`.

## Tests

99 unit tests + 3 integration tests + 2 doctest with `--all-features`; 81 unit + 3 integration + 2 doctest with `--features swarm`; 36 unit + 3 integration + 1 doctest with no features (M0 + `cluster_doc` + `participant`); 45 unit + 3 integration + 1 doctest with `--features app_instance`; 45 unit + 3 integration + 1 doctest with `--features discovery_client`. The `app_instance` tests (9) run under `--features app_instance`; the `discovery_client` tests (9) run under `--features discovery_client`; the `swarm` tests (8 + doctest), the `cluster_protocol` tests (3), the `cluster_runtime` tests (8), the `stream_protocol` tests (18 — `+5` for Dagaz Batch 1's `PointCloudFrame` round-trip + wire-size + cross-language conformance vector + framing-helpers round-trip), and the `stream_runtime` tests (8 — `+2` for Dagaz Batch 1's e2e `producer_accepts_and_streams_pointcloud_frames` and `one_producer_serves_jpeg_and_pointcloud_via_sensor_id_dispatch`) all run under `--features swarm`.

The `tests/discovery_integration.rs` integration suite (2 tests, both `#[ignore]`) boots a real Discovery binary on a tempdir + ephemeral loopback port. Run with `DISCOVERY_BIN=/path/to/discovery cargo test -p auki-network --features discovery_client -- --ignored discovery`.

| Test | Asserts |
|------|---------|
| `peer_identity_from_wallet_is_deterministic` | Same wallet → same `PeerId` |
| `peer_identity_differs_across_wallets` | Different wallets → different `PeerId`s |
| `from_wallet_matches_from_seed_of_derived_child` | Public contract: `from_wallet(w) ≡ from_seed(w.derive_child("peer/v1").seed())` |
| `from_seed_is_deterministic` | Same seed → same `PeerId` |
| `from_seed_does_not_mutate_caller_buffer` | Caller's seed buffer survives the call |
| `pubkey_bytes_match_derived_wallet_pubkey` | libp2p ed25519 pubkey bytes equal the derived wallet's pubkey bytes |
| `peer_id_matches_public_key_to_peer_id` | `peer_id() == public_key().to_peer_id()` (sanity) |
| `reachability_record_round_trips_through_json` | JSON serialize → deserialize is identity |
| `capability_constants_match_spec` | Wire-format strings unchanged |
| `capability_namespace_extraction` | `namespace()` returns the prefix before `:`, or `None` |
| `capability_round_trips_through_json` | JSON serialize → deserialize is identity |
| `participant::round_trip_with_cluster_joined_some` | JSON serialize → deserialize is identity with `Some` |
| `participant::round_trip_with_cluster_joined_none` | JSON serialize → deserialize is identity with `None`; field present with `null` value |
| `participant::json_keys_are_snake_case` | All JSON keys match the spec exactly (snake_case) |
| `participant::golden_bytes_match_fixture` | Locked wire format — fixture struct serializes to exactly the spec'd JSON |
| `participant::rejects_missing_field` | Missing required field fails to deserialize |
| `participant::rejects_wrong_type` | Wrong-type value (string for u64) fails to deserialize |
| `participant::rejects_invalid_peer_id` | Non-PeerId string in `peer_id` fails to deserialize |
| `participant::cluster_joined_field_is_explicit_null_not_omitted` | `None` serializes as explicit `null`, not field omission |
| `swarm::local_peer_id_matches_identity` | Built swarm's `local_peer_id` equals `identity.peer_id()` |
| `swarm::two_peers_identify_each_other_over_tcp` | TCP dial → Noise handshake → identify exchange both ways |
| `swarm::two_peers_identify_each_other_over_quic` | Same as above, over QUIC |
| `swarm::build_listens_on_all_provided_addresses` | Both listen addresses produce `NewListenAddr` events |
| `swarm::build_with_mdns_enabled_succeeds` | Construction-only sanity (real mDNS discovery requires a multicast-capable interface; verified by daemon-level integration) |
| `swarm::build_with_relay_server_enabled_succeeds` | Construction-only sanity |
| `swarm::relay_server_accepts_reservation` | Full reservation flow: client dials relay → identify exchange → listen on `/p2p/<relay>/p2p-circuit` → `RelayClient::ReservationReqAccepted` |
| `swarm::dial_peer_helper_dials_direct_address` | The `dial_peer` helper establishes a connection by `(PeerId, addresses)` and identify exchange completes |
| `cluster_protocol::protocol_id_is_locked` | Wire-format pin: `CLUSTER_PROTOCOL == "/auki/cluster/0.0.1"` |
| `cluster_protocol::request_serializes_as_json_null` | `ClusterRequest` (unit struct) serializes as JSON `null` and round-trips |
| `cluster_protocol::two_peers_exchange_participant_info_over_tcp` | End-to-end: peer A sends `ClusterRequest`, peer B replies with its `ParticipantInfo`, A asserts received == fixture |
| `cluster_runtime::two_runtimes_discover_each_other_via_cluster_doc` | 2-peer happy path: both spawn, converge in `peers()` within 10 s, cross-side ParticipantInfo correct, `first_seen_ns > 0` |
| `cluster_runtime::three_runtimes_form_full_mesh` | 3 runtimes, each ends with 2 peers in `peers()` within 15 s |
| `cluster_runtime::peer_leaving_drops_off_other_peers` | 3 runtimes converge → kill one → surviving 2 drop the departed peer from `peers()` while keeping each other |
| `cluster_runtime::unknown_peer_is_not_surfaced` | Outsider not in doc dials in and sends a request → runtime drops silently, `peers().len() == 0` (cluster doc is the trust boundary) |
| `cluster_runtime::provider_returning_none_drops_the_reply` | Provider returns `None` → runtime drops the reply channel (requester sees timeout); runtime survives; asymmetric peer view confirms (rt-with-normal-provider sees rt-with-none, but rt-with-none replies normally so the other side sees nothing) |
| `cluster_runtime::shutdown_is_idempotent_and_drops_state` | `shutdown(self)` returns promptly without deadlock |
| `cluster_runtime::drop_without_explicit_shutdown_cleans_up` | `Drop` runs the same cleanup as `shutdown` |
| `cluster_runtime::spawn_outside_tokio_runtime_returns_error` | Calling `from_swarm` from a `std::thread` (no tokio) → `SpawnError::NoTokioRuntime` |
| `stream_protocol::protocol_id_is_locked` | Wire-format pin: `STREAM_PROTOCOL == "/auki/stream/0.1.0"` |
| `stream_protocol::max_frame_bytes_is_locked` | Wire-format pin: `MAX_FRAME_BYTES == 16 MiB` |
| `stream_protocol::request_message_round_trips_through_json` | `StreamMessage::Request` round-trips through serde-JSON, with the `kind: "request"` tag pinned |
| `stream_protocol::accept_message_round_trips_through_json` | `StreamMessage::Accept` round-trips, `kind: "accept"` tag pinned |
| `stream_protocol::decline_message_round_trips_through_json` | `StreamMessage::Decline { reason: SensorNotFound }` round-trips, both outer and inner `kind` tags pinned |
| `stream_protocol::frame_message_round_trips_through_json` | `StreamMessage::Frame { ... }` round-trips, `kind: "frame"`, `seq` field pinned |
| `stream_protocol::end_of_stream_message_round_trips_through_json` | `StreamMessage::EndOfStream { reason: ProducerError { detail } }` round-trips |
| `stream_protocol::write_then_read_round_trips_a_request` | Single message survives `write_message → read_message` through an in-memory cursor; length prefix matches encoded body |
| `stream_protocol::write_then_read_round_trips_a_full_session` | Realistic order (Request → Accept → Frame×3 → EndOfStream) survives in the same buffer in order |
| `stream_protocol::read_rejects_oversized_frame_via_length_prefix` | Length prefix `MAX_FRAME_BYTES + 1` → `FrameTooLarge` before the payload is read (no allocation) |
| `stream_protocol::read_rejects_empty_frame` | Length prefix `0` → `EmptyFrame` |
| `stream_protocol::read_surfaces_eof_as_io_error` | Empty buffer → `Io(UnexpectedEof)` (consumer should treat as substream-closed) |
| `stream_protocol::write_rejects_oversized_payload_before_io` | Payload that JSON-encodes to over `MAX_FRAME_BYTES` → `FrameTooLarge` before writing the length prefix |
| `stream_protocol::point_cloud_frame_serializes_bytes_as_base64_string` | Wire shape pin: `PointCloudFrame { bytes: [0,1,2,3,255,254] }` → `{"bytes":"AAECA//+"}` exactly. `serde(with = "base64_bytes")` adapter drift fails loudly |
| `stream_protocol::point_cloud_frame_round_trips_a_kilobyte_payload` | 1 KB pseudo-random payload round-trips losslessly through serde via the base64 adapter |
| `stream_protocol::point_cloud_frame_wire_size_dodges_the_array_of_integers_tax` | 1 KB payload → < 1.5 KB JSON (vs ~3.4 KB the array-of-integers path would produce). If someone removes the adapter, this fails — Dagaz's bandwidth reasoning is preserved |
| `stream_protocol::locked_point_cloud_frame_wire_shape_vector` | **Cross-language conformance vector.** `StreamMessage::Frame { timestamp_ns: 1_700_000_000_000_000_000, seq: 42, payload: PointCloudFrame { bytes: [0x42..0xff] } }` → exact JSON `{"kind":"frame","timestamp_ns":1700000000000000000,"seq":42,"payload":{"bytes":"QkNERQAB/v8="}}`. Park's browser-side decoder + future cross-language reimplementations pin against this |
| `stream_protocol::point_cloud_frame_round_trips_through_framing_helpers` | Full wire-level round trip through `write_message` + `read_message` (length prefix + JSON body) for `PointCloudFrame` |
| `stream_runtime::producer_accepts_and_streams_jpeg_frames` | E2E happy path: two cluster runtimes converge, consumer opens stream, reads 3 typed frames + clean `EndOfStream { reason: SourceEnded }`; iterator exhausted after the terminator. Asserts `seq` stamping (0, 1, 2), `timestamp_ns`, `payload.bytes`, and `info.{sensor_hash, clock_id, clock_hash}` end-to-end |
| `stream_runtime::producer_declines_unknown_sensor` | Provider returns `Decline { reason: SensorNotFound }` for unknown `sensor_id`; consumer's `open_stream` returns `Err(OpenStreamError::Declined { reason: SensorNotFound })` |
| `stream_runtime::producer_error_signals_consumer_with_detail` | Source-Stream yields `Some(Err("encoder died"))`; consumer reads frame then sees `Err(EndOfStream { reason: ProducerError { detail: "encoder died" } })`; iterator exhausted after |
| `stream_runtime::decline_all_streams_returns_sensor_not_found` | Convenience helper for consumer-only nodes (Park) declines every request with `SensorNotFound` |
| `stream_runtime::producer_shutdown_signals_consumer_with_typed_end_of_stream` | Producer's source-Stream is `iter([first_frame]).chain(pending())`; consumer reads first frame, then `producer.shutdown()` is called; consumer's iterator yields `Err(EndOfStream { reason: ProducerShuttingDown })` (not `ConnectionLost`) within the `SHUTDOWN_GRACE` + RTT window, then `None`. Confirms grimsby D5b's "best-effort explicit" path |
| `stream_runtime::open_stream_against_unreachable_peer_surfaces_typed_error` | `cluster.json` lists a peer at an unreachable address (port 1, refused immediately); consumer's `open_stream` returns `Err(OpenStreamError::LibP2p(_))` or `Err(OpenStreamError::Timeout(_))` rather than hanging or panicking. Bounded by an outer 35s safety-net timeout vs the SDK's 30s `OPEN_STREAM_TIMEOUT` |
| `stream_runtime::producer_accepts_and_streams_pointcloud_frames` | **Dagaz Batch 1 e2e.** Two-runtime cluster convergence + `open_stream<PointCloudFrame>` + 3 typed CDR-shaped frames + `EndOfStream { SourceEnded }` terminator + iterator-exhausted-after. Mirrors the JpegFrame happy path through the new `StreamDispatch::AcceptPointCloud` arm |
| `stream_runtime::one_producer_serves_jpeg_and_pointcloud_via_sensor_id_dispatch` | **Dagaz D1 multi-`T` dispatch.** One `ClusterRuntime` with a `sensor_id`-keyed multi-`T` provider serves `"camera"` (JPEG) + `"pointcloud"` (PointCloud) over distinct libp2p substreams multiplexed on the same yamux/QUIC connection, plus `"no-such-sensor"` declined. The exact shape boosterapp uses in Batch 3 |
| `cluster_doc::round_trips_through_serde` | Two-peer doc serialize → load is identity |
| `cluster_doc::loads_canonical_example_from_spec` | The README's example schema parses end-to-end |
| `cluster_doc::missing_optional_fields_default_to_none` | `expected_app_id` and `note` absent → `None`; empty addresses allowed |
| `cluster_doc::io_error_for_missing_file` | Nonexistent path → `LoadError::Io` |
| `cluster_doc::parse_error_for_invalid_json` | Malformed JSON → `LoadError::Parse` |
| `cluster_doc::unsupported_version_rejected` | `version: 99` → `LoadError::UnsupportedVersion(99)` |
| `cluster_doc::version_one_accepted` | `version: 1` is the supported value |
| `cluster_doc::invalid_peer_id_rejected` | Garbage in `peer_id` → `LoadError::InvalidPeerId` |
| `cluster_doc::invalid_multiaddr_rejected` | Garbage in `addresses[]` → `LoadError::InvalidMultiaddr` |
| `cluster_doc::default_path_is_under_registries_cluster_registries` | Default path = `<app_root>/registries/cluster_registries/cluster.json` |
| `cluster_doc::resolve_path_falls_back_to_default` | No CLI, no env → default |
| `cluster_doc::resolve_path_honours_cli_override` | CLI override wins over default |
| `cluster_doc::resolve_path_honours_env_override` | `$AUKI_CLUSTER_DOC` wins over default |
| `cluster_doc::resolve_path_cli_beats_env` | CLI override wins over `$AUKI_CLUSTER_DOC` |
| `cluster_doc::resolve_path_treats_empty_env_as_unset` | `AUKI_CLUSTER_DOC=""` falls through to default |
| `cluster_doc::pretty_serialized_form_is_stable_under_round_trip` | None-valued optionals are skipped on serialize and round-trip clean |
| `cluster_doc[integration]::loads_from_default_path_layout` | Daemon-startup flow: `resolve_path` then `load` against on-disk doc under `<app_root>/registries/cluster_registries/cluster.json` |
| `cluster_doc[integration]::loads_from_cli_override_path` | `--cluster-doc <path>` flow: `resolve_path` with override → `load` |
| `cluster_doc[integration]::surfaces_invalid_peer_id_with_value_in_error` | Operator typo path: `LoadError::InvalidPeerId` carries the offending value |
| doctest in `swarm.rs` | Builder example compiles |
| `app_instance::derive_from_locked_mac_renders_lowercase_no_separators` | Locked: `[0x00,0x16,0x3e,0xab,0xcd,0xef]` → `"00163eabcdef"` (cross-language conformance) |
| `app_instance::derive_from_returns_no_network_interfaces_on_empty_input` | Empty input → `NoNetworkInterfaces` |
| `app_instance::derive_from_returns_no_suitable_mac_when_only_loopback` | All-zero MAC → `NoSuitableMac` |
| `app_instance::derive_from_returns_no_suitable_mac_when_only_locally_administered` | Every MAC has U/L bit set → `NoSuitableMac` |
| `app_instance::derive_from_skips_loopback_and_picks_remaining_ieee_mac` | Loopback + IEEE → returns the IEEE one |
| `app_instance::derive_from_skips_locally_administered_mac` | Random + IEEE → returns the IEEE one |
| `app_instance::derive_from_picks_lexicographically_first_when_multiple_ieee_macs` | Multiple IEEE MACs → smallest by raw bytes (deterministic) |
| `app_instance::derive_from_output_is_exactly_twelve_lowercase_hex_chars` | Schema check: any success returns 12 lowercase hex chars |
| `app_instance::ul_bit_logic_isolates_first_octet_bit_one` | U/L-bit math: `0x02` set → locally administered; `0x01` (multicast) unrelated |
| `discovery_client::register_signature_verifies_under_wire_pubkey` | Signed bytes verify under the wire pubkey via `auki_identity::verify` (rule 4 of Discovery's verify order) |
| `discovery_client::register_pubkey_reconstructs_peer_id` | `libp2p_identity::PublicKey::from_bytes(wire_pubkey).to_peer_id() == wire_peer_id` (rule 2 of Discovery's verify order) |
| `discovery_client::tampered_addresses_break_signature` | Mutating `addresses` after signing breaks signature verify — `addresses` is in the canonical bytes |
| `discovery_client::cross_cluster_replay_breaks_signature` | A signature computed for cluster A doesn't verify under cluster B's canonical bytes — `cluster_name` is in the canonical bytes |
| `discovery_client::optional_fields_omitted_when_none` | Parses canonical JSON: no `expected_app_id` / `note` keys when callers passed `None`; required keys always present |
| `discovery_client::deregister_signature_verifies` | Deregister canonical bytes match `{cluster_name, peer_id, op: "delete", timestamp_ns}` and signature self-verifies; pinned: `public_key` is on the wire body but NOT in the signed bytes |
| `discovery_client::locked_register_canonical_and_signature_vector` | Cross-language conformance vector: parent seed `[3u8; 32]` + fixed cluster + fixed addresses + fixed `timestamp_ns: 1_700_000_000_000_000_000` → exact RFC 8785 canonical bytes + exact 64-byte ed25519 signature |
| `discovery_client::with_http_uses_supplied_client_and_url` | `with_http` constructor uses the supplied `reqwest::Client`; URL trailing slash trimmed |
| `discovery_client::new_trims_trailing_slash` | `DiscoveryClient::new("...")` and `DiscoveryClient::new(".../")` produce the same `base_url` |
| `discovery_client[integration]::discovery_round_trip` (`#[ignore]`) | E2E against real Discovery binary on a tempdir + ephemeral port: Sentinel + Booster register, fetch returns both, Sentinel deregister, fetch shows only Booster, second sentinel deregister returns 404, unknown cluster fetch returns 404 |
| `discovery_client[integration]::discovery_rejects_invalid_cluster_name` (`#[ignore]`) | Path-traversal `cluster_name` (`"../etc/passwd"`) is rejected by Discovery |

## Dependencies

- `auki-identity` — wallet primitive; source of `derive_child("peer/v1")`.
- `libp2p-identity` (0.2, `ed25519` + `peerid` + `serde` features, `default-features = false`) — keypair, public key, PeerId encoding.
- `multiaddr` (0.18) — typed multiaddr; serde adapter local to this crate.
- `serde` — derive on `Capability` and `ReachabilityRecord`.
- *(swarm feature)* `libp2p` (0.56, features: `tokio`, `tcp`, `quic`, `noise`, `yamux`, `identify`, `ping`, `mdns`, `relay`, `request-response`, `json`, `macros`, `ed25519`) — the swarm itself plus the `cluster_protocol` JSON request-response codec.
- *(swarm feature)* `libp2p-stream` (`=0.4.0-alpha`) — raw-substream multiplexer for grimsby's `/auki/stream/0.1.0` typed-byte-stream protocol. The libp2p umbrella crate doesn't expose `stream` as a feature in 0.56; pre-1.0; pinned exactly until the upstream surface stabilizes.
- *(swarm feature)* `thiserror` (2) — `BuildError`, `SpawnError`.
- *(swarm feature)* `tokio` (1, features: `macros`, `rt`, `sync`, `time`) — `cluster_runtime`'s task primitives (`select!`, `oneshot`, `interval`, `Handle::try_current`).
- *(swarm feature)* `futures` (0.3, default-features off) — `StreamExt::next` for polling `swarm.next()` in the runtime task.
- *(app_instance feature)* `mac_address` (1) — cross-platform interface enumeration via `getifaddrs` / `GetAdaptersAddresses`. Non-WASM by nature.
- *(discovery_client feature)* `reqwest` (0.12, default-features off, features: `json`, `rustls-tls`) — async HTTP client. rustls-tls (not native-tls) keeps us off OpenSSL for cross-platform reproducibility; HTTPS isn't strictly required for ansuz LAN deploys but the cost is small and Discovery may grow beyond the LAN later.
- *(discovery_client feature)* `base64` (0.22, default-features off, features: `std`) — encoding / decoding the 32-byte ed25519 pubkey + 64-byte signature on the wire.
- *(discovery_client feature)* `auki-jcs` (path) — RFC 8785 canonical bytes for the signed payload. Same crate Discovery uses on its verify side, so canonical-byte equality is mechanical.
- *(discovery_client feature)* `thiserror` (2) — `DiscoveryError`. Already optional for `swarm`; the same dep, two features both opting it in.
- *(dev)* `tempfile` for `cluster_doc` fixture-on-disk round-trips and the `discovery_integration` test's tempdir; `tokio` (`macros`, `rt-multi-thread`, `time`) + `futures` for the swarm tests.

## Consumers in this workspace

- *(planned, downstream)* `aukilabs/relay` — sets `enable_relay_server: true`; advertises the four `networking:*` capabilities it implements.
- *(planned, downstream)* BoosterApp / Sentinel — set `enable_relay_server: false`; consume the swarm, register `ReachabilityRecord`s with a configured Relay.
- *(planned, downstream)* Park — uses `dial_peer(peer_id, [relay_multiaddr/p2p-circuit])` for Park-from-home.
- *(planned, downstream)* Console — depends on `auki-network` *without* the `swarm` feature; uses M0 only to display a wallet's `peer_id` in-browser.
