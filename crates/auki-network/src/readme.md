# `auki-network/src/`

Networking substrate for the SDK. Spec: this crate's [outer `README.md`](../README.md).

## What's here

- [`lib.rs`](lib.rs) — M0 data types: `PeerIdentity`, `ReachabilityRecord`, `Capability`, plus the `multiaddr_vec_serde` adapter.
- [`cluster_doc.rs`](cluster_doc.rs) — `cluster.json` discovery-doc loader (ansuz #1). Always available (no feature gate); `std::fs`-based, runs on native targets. Public types: `ClusterDoc`, `ClusterPeer`, `LoadError`. Public fns: `load`, `default_path`, `resolve_path`. Public consts: `SUPPORTED_VERSION = 1`, `ENV_OVERRIDE = "AUKI_CLUSTER_DOC"`, `DEFAULT_RELATIVE_PATH = "registries/cluster_registries/cluster.json"`.
- [`participant.rs`](participant.rs) — `ParticipantInfo`, the wire shape exchanged over `GET /api/info` (HTTP) and the `/auki/cluster/1.0.0` participant protocol (libp2p). M0 — available without the `swarm` feature.
- [`swarm.rs`](swarm.rs) — M1 libp2p `Swarm` builder, gated behind the `swarm` feature.
- [`cluster_protocol.rs`](cluster_protocol.rs) — `/auki/cluster/1.0.0` request-response protocol (ansuz #3), gated behind the `swarm` feature. Wraps `libp2p::request_response::json::Behaviour<ClusterRequest, ParticipantInfo>`; wired into `swarm::Behaviour` as the always-on `cluster:` field.
- [`cluster_runtime.rs`](cluster_runtime.rs) — opaque runtime that owns a `Swarm<Behaviour>` + tokio task and orchestrates the cluster (ansuz #4), gated behind the `swarm` feature. Auto-dials peers in a `ClusterDoc`, exchanges `ParticipantInfo`, exposes the live peer state via `peers()`, reconnects with per-peer exponential backoff. The wrapper `auki-py` `cluster.spawn` is built on top of this.
- [`app_instance.rs`](app_instance.rs) — per-machine identifier derivation (ansuz #5), gated behind the `app_instance` feature.

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
        /* identify + ping + Toggle<mdns> + relay_client + Toggle<relay> + cluster */
    }
    pub struct SwarmConfig {
        listen_addresses: Vec<Multiaddr>,
        agent_version: String,
        enable_mdns: bool,           // default true
        enable_relay_server: bool,   // default false
    }
    pub enum BuildError { Transport(String), Listen { addr, source } }
    pub const IDENTIFY_PROTOCOL: &str = "/auki/identify/1.0.0";
    pub fn build_swarm(identity: &PeerIdentity, config: SwarmConfig) -> Result<libp2p::Swarm<Behaviour>, BuildError>;
    pub fn dial_peer(swarm: &mut Swarm<Behaviour>, peer: PeerId, addresses: Vec<Multiaddr>) -> Result<(), DialError>;
}

// ansuz #3 (behind `swarm` feature)
pub mod cluster_protocol {
    pub const CLUSTER_PROTOCOL: &str = "/auki/cluster/1.0.0";
    pub const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    pub struct ClusterRequest;                       // unit struct → JSON `null`
    pub type ClusterResponse = ParticipantInfo;
    pub type Behaviour =
        libp2p::request_response::json::Behaviour<ClusterRequest, ClusterResponse>;

    pub fn behaviour() -> Behaviour;
}

// ansuz #4 (behind `swarm` feature)
pub mod cluster_runtime {
    pub const INITIAL_BACKOFF: std::time::Duration = std::time::Duration::from_secs(1);
    pub const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(60);
    pub const RECONNECT_TICK: std::time::Duration = std::time::Duration::from_millis(500);

    pub type ParticipantInfoProvider = std::sync::Arc<
        dyn Fn() -> ParticipantInfo + Send + Sync,
    >;

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
        ) -> Result<Self, SpawnError>;

        pub fn from_swarm(
            swarm: libp2p::Swarm<swarm::Behaviour>,
            doc: ClusterDoc,
            participant_provider: ParticipantInfoProvider,
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
        identify:     identify::Behaviour::new(/* protocol /auki/identify/1.0.0, agent_version */),
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

The behaviour is `libp2p::request_response::json::Behaviour` over the protocol id `/auki/cluster/1.0.0`. Request body is the unit struct `ClusterRequest` (serializes as JSON `null` — empty by design); response is `ParticipantInfo` (same JSON as `GET /api/info`). One round-trip per query; 30 s per-request timeout.

The behaviour is wired into the swarm `Behaviour` struct as the always-on `cluster:` field — there is no `Toggle`. Swarms that don't participate in a cluster (the dedicated `aukilabs/relay` infrastructure node) just never see traffic on it; a knob would have been ceremony.

The behaviour does **not** auto-respond. A peer that receives a request gets `request_response::Event::Message::Request{ channel, .. }` and is responsible for filling in its current `ParticipantInfo` and calling `behaviour.cluster.send_response(channel, info)`. This is the standard libp2p pattern, and it's what lets a Python sidecar's `participant_provider` callable invoke per-request so `session_now_ns` is fresh on every reply rather than stale at swarm-spawn time.

```text
A → B   ClusterRequest                     (JSON null)
B → A   ParticipantInfo of B               (same shape as GET /api/info)
```

The JSON is byte-for-byte identical to the `participant::golden_bytes_match_fixture` fixture — the codec uses `serde_json` end-to-end. Length framing is the underlying libp2p stream's, not application-layer.

Higher-level orchestration (auto-dialing peers from `cluster.json`, tracking `Joined`/`Left`, holding a peer state map) lives in the upcoming cluster-runtime module (ansuz #4); Rust consumers that want fine control (Sentinel) drive the swarm event loop themselves.

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

The runtime is opaque by design: consumers don't drive the swarm event loop themselves. The Python sidecar in Boosterapp can't drive an async libp2p loop from Python and just wants `peers()` from the HTTP request handler thread; `auki-py`'s `cluster.spawn` will wrap this. Sentinel and other Rust consumers that want fine control use `cluster_protocol::Behaviour` directly and skip this module.

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

63 unit tests + 3 integration tests + 2 doctest with `--all-features`; 54 unit + 3 integration + 2 doctest with `--features swarm`; 36 unit + 3 integration + 1 doctest with no features (M0 + `cluster_doc` + `participant`); 45 unit + 3 integration + 1 doctest with `--features app_instance`. The `app_instance` tests (9) run under `--features app_instance`; the `swarm` tests (8 + doctest), the `cluster_protocol` tests (3), and the `cluster_runtime` tests (7) all run under `--features swarm`.

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
| `cluster_protocol::protocol_id_is_locked` | Wire-format pin: `CLUSTER_PROTOCOL == "/auki/cluster/1.0.0"` |
| `cluster_protocol::request_serializes_as_json_null` | `ClusterRequest` (unit struct) serializes as JSON `null` and round-trips |
| `cluster_protocol::two_peers_exchange_participant_info_over_tcp` | End-to-end: peer A sends `ClusterRequest`, peer B replies with its `ParticipantInfo`, A asserts received == fixture |
| `cluster_runtime::two_runtimes_discover_each_other_via_cluster_doc` | 2-peer happy path: both spawn, converge in `peers()` within 10 s, cross-side ParticipantInfo correct, `first_seen_ns > 0` |
| `cluster_runtime::three_runtimes_form_full_mesh` | 3 runtimes, each ends with 2 peers in `peers()` within 15 s |
| `cluster_runtime::peer_leaving_drops_off_other_peers` | 3 runtimes converge → kill one → surviving 2 drop the departed peer from `peers()` while keeping each other |
| `cluster_runtime::unknown_peer_is_not_surfaced` | Outsider not in doc dials in and sends a request → runtime drops silently, `peers().len() == 0` (cluster doc is the trust boundary) |
| `cluster_runtime::shutdown_is_idempotent_and_drops_state` | `shutdown(self)` returns promptly without deadlock |
| `cluster_runtime::drop_without_explicit_shutdown_cleans_up` | `Drop` runs the same cleanup as `shutdown` |
| `cluster_runtime::spawn_outside_tokio_runtime_returns_error` | Calling `from_swarm` from a `std::thread` (no tokio) → `SpawnError::NoTokioRuntime` |
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

## Dependencies

- `auki-identity` — wallet primitive; source of `derive_child("peer/v1")`.
- `libp2p-identity` (0.2, `ed25519` + `peerid` + `serde` features, `default-features = false`) — keypair, public key, PeerId encoding.
- `multiaddr` (0.18) — typed multiaddr; serde adapter local to this crate.
- `serde` — derive on `Capability` and `ReachabilityRecord`.
- *(swarm feature)* `libp2p` (0.56, features: `tokio`, `tcp`, `quic`, `noise`, `yamux`, `identify`, `ping`, `mdns`, `relay`, `request-response`, `json`, `macros`, `ed25519`) — the swarm itself plus the `cluster_protocol` JSON request-response codec.
- *(swarm feature)* `thiserror` (2) — `BuildError`, `SpawnError`.
- *(swarm feature)* `tokio` (1, features: `macros`, `rt`, `sync`, `time`) — `cluster_runtime`'s task primitives (`select!`, `oneshot`, `interval`, `Handle::try_current`).
- *(swarm feature)* `futures` (0.3, default-features off) — `StreamExt::next` for polling `swarm.next()` in the runtime task.
- *(app_instance feature)* `mac_address` (1) — cross-platform interface enumeration via `getifaddrs` / `GetAdaptersAddresses`. Non-WASM by nature.
- *(dev)* `tempfile` for `cluster_doc` fixture-on-disk round-trips; `tokio` (`macros`, `rt-multi-thread`, `time`) + `futures` for the swarm tests.

## Consumers in this workspace

- *(planned, downstream)* `aukilabs/relay` — sets `enable_relay_server: true`; advertises the four `networking:*` capabilities it implements.
- *(planned, downstream)* BoosterApp / Sentinel — set `enable_relay_server: false`; consume the swarm, register `ReachabilityRecord`s with a configured Relay.
- *(planned, downstream)* Park — uses `dial_peer(peer_id, [relay_multiaddr/p2p-circuit])` for Park-from-home.
- *(planned, downstream)* Console — depends on `auki-network` *without* the `swarm` feature; uses M0 only to display a wallet's `peer_id` in-browser.
