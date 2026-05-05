# auki-network

Networking substrate for the Auki SDK. Layer 1 of the Reid milestone-2 networking stack: peer identity, reachability records, and named capabilities (M0 — always available, WASM-friendly), plus a libp2p `Swarm` builder with TCP + QUIC + Circuit Relay v2 + mDNS, an `identify` + `ping` behaviour, a dial-by-peer-id helper, and the `/auki/cluster/1.0.0` participant-exchange request-response protocol (M1 — behind the `swarm` feature). For the ansuz networking-demo milestone, also ships the static `cluster.json` discovery doc loader (always-on) and the `app_instance::derive` per-machine identifier helper (behind the `app_instance` feature).

## What a peer is

Per the broader Auki architecture, every node has *two* identities:

- **Wallet** — economic / policy / ownership. Lives in [`auki-identity`](../auki-identity).
- **Peer** — network / dialability. Lives here.

The peer identity is *derived* from the principal wallet via `Wallet::derive_child("peer/v1")`, so a backup of the wallet seed lets you regenerate the peer key. The peer key has its own libp2p `PeerId` and is what shows up in multiaddrs as `/p2p/<peer-id>`. Compromise blast-radius is separated: rotating the peer key (a re-derivation under a future label like `peer/v2`) doesn't invalidate the wallet.

## Four primitives

### `PeerIdentity`

Wraps a libp2p `Keypair` (ed25519). Constructed via `from_wallet(&wallet)` (canonical) or `from_seed(&seed)` (for tooling that already has the derived peer seed cached).

```rust
use auki_identity::Wallet;
use auki_network::PeerIdentity;

let wallet = Wallet::from_seed(&[7u8; 32]);
let peer = PeerIdentity::from_wallet(&wallet);

let pid = peer.peer_id();          // libp2p PeerId
let pk  = peer.public_key();       // libp2p PublicKey (safe to publish)
let kp  = peer.keypair();          // libp2p Keypair (sensitive — for swarm only)
```

The contract is a fixed recipe: `from_wallet(w) ≡ from_seed(&w.derive_child("peer/v1").seed())`. Cross-language consumers can reproduce it without depending on this crate.

### `ReachabilityRecord`

What a peer advertises about how to reach it: peer id, dialable multiaddrs (TCP, QUIC, circuit-relay-mediated), the named capabilities it offers, a last-seen timestamp for staleness pruning. Serializable JSON; the wire shape for peer discovery whether the directory is LAN mDNS or a remote Discovery Service.

```rust
use auki_network::{Capability, PeerIdentity, ReachabilityRecord};

ReachabilityRecord {
    peer_id: peer.peer_id(),
    addresses: vec![
        "/ip4/192.168.9.130/tcp/4001".parse().unwrap(),
        "/ip4/192.168.9.130/udp/4001/quic-v1".parse().unwrap(),
    ],
    capabilities: vec![Capability::new(Capability::MESSAGE_FORWARDING)],
    last_seen_ns: now_ns(),
};
```

### `ParticipantInfo`

The wire shape every Auki participant exchanges to introduce itself. **One schema, two transports**:

- **HTTP** — `GET /api/info` on the cross-app Control API ([`docs/control-api.md`](../../docs/control-api.md)) returns this exact JSON.
- **libp2p** — the `/auki/cluster/1.0.0` participant protocol (see [the cluster protocol](#the-cluster-protocol-m1) below), a request/response exchange where each side sends its own `ParticipantInfo` to the other.

```rust
pub struct ParticipantInfo {
    pub app: String,                          // e.g. "boosterapp", "sentinel", "park"
    pub name: String,                         // operator-friendly label, e.g. "k1-walker"
    pub session_id: String,                   // UUIDv4 minted at session boot
    pub session_clock_id: String,             // identifier for the session's monotonic clock
    pub session_clock_hash: String,           // content-addressed hash pinning the clock-registry entry
    pub session_now_ns: u64,                  // session clock's value at the moment this struct was filled
    pub cluster_joined_at_ns: Option<u64>,    // session-clock value at first peer connection; None while alone
    pub peer_id: libp2p::PeerId,              // libp2p PeerId derived from Wallet::derive_child("peer/v1")
    pub app_instance: String,                 // first non-loopback IEEE-administered MAC, lowercased hex without separators
}
```

Snake-case JSON; `cluster_joined_at_ns` serializes as `null` when `None`; `peer_id` serializes as the canonical multibase-base58 string (`12D3KooW…`).

```json
{
  "app": "boosterapp",
  "name": "k1-walker",
  "session_id": "abc-123-...",
  "session_clock_id": "K1-AABBCCDDEEFF/session-monotonic",
  "session_clock_hash": "abc123...",
  "session_now_ns": 12345678900,
  "cluster_joined_at_ns": 1745000000,
  "peer_id": "12D3KooWAbc...",
  "app_instance": "aabbccddeeff"
}
```

`session_id`, `app`, and `app_instance` carry the same meaning across the SDK: `session_id` matches the directory name and the `session_id` field on every manifest written during the run (see [`auki-session`](../auki-session/README.md)); `app` matches the manifest `app_id` field and the `app` value advertised over mDNS by the daemon's Control API; `app_instance` is a hardware-stable identifier reused across runs on the same device.

The struct is part of the M0 path — available without the `swarm` feature so consumers (Park, Console) can construct and parse it without pulling in libp2p's transport stack.

### `Capability`

A namespaced string identifying what a peer offers. Format is `"<namespace>:<name>"`. Forward-extensible without crate changes — new capabilities are just new strings. The four canonical networking capabilities (per the Reid milestone-2 architecture) are exposed as `&str` constants:

| Constant | String | Role |
|----------|--------|------|
| `Capability::MESSAGE_FORWARDING` | `networking:message-forwarding` | Hagall-`rosrelay` parity — small frequent control-plane messages |
| `Capability::BULK_DATA_CHANNEL` | `networking:bulk-data-channel` | Large non-real-time binary transfer |
| `Capability::TURN` | `networking:turn` | Real-time media P2P fallback |
| `Capability::SFU` | `networking:sfu` | Real-time media one-to-many fan-out |

Other namespaces (`discovery:*`, `compute:*`, etc.) are open. The Relay app implements the four `networking:*` capabilities; daemons advertise the ones they offer; consumers filter by namespace or specific value.

## The swarm builder (M1)

Behind the `swarm` feature. `auki_network::swarm::build_swarm(&identity, config)` returns a `libp2p::Swarm<Behaviour>` already listening on the configured addresses.

```rust
use auki_identity::Wallet;
use auki_network::{PeerIdentity, swarm::{build_swarm, SwarmConfig}};

let wallet = Wallet::from_seed(&[7u8; 32]);
let identity = PeerIdentity::from_wallet(&wallet);
let swarm = build_swarm(&identity, SwarmConfig {
    listen_addresses: vec![
        "/ip4/0.0.0.0/tcp/0".parse().unwrap(),
        "/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap(),
    ],
    agent_version: "boosterapp/0.1".into(),
    enable_mdns: true,           // _p2p._udp.local. for LAN discovery
    enable_relay_server: false,  // off for daemons; true for the Relay app
})?;
```

**Transport stack:** TCP + QUIC, both authenticated with Noise (using the peer's ed25519 keypair) and multiplexed with Yamux. Circuit Relay v2 client transport is wired in always, so any peer can dial through a relay; the relay-*server* behaviour is gated on `enable_relay_server`.

**Behaviour composition:**

| Field | Always-on | Notes |
|-------|-----------|-------|
| `identify` | yes | Protocol id `/auki/identify/1.0.0`; `agent_version` is the per-deployment knob |
| `ping` | yes | Resets the 60 s idle-connection timer |
| `mdns` (Toggle) | gated on `enable_mdns` | `_p2p._udp.local.` advertisement; on by default for daemons. Daemons keep their existing `_auki._tcp.local.` advertisement separately (control-API discovery, unchanged) — **dual-channel** per Reid parking-lot 1a |
| `relay_client` | yes | Lets any peer dial through a relay; consumes circuit-relay multiaddrs |
| `relay` (Toggle) | gated on `enable_relay_server` | The relay-*server* role; off by default for consumer daemons; on for the dedicated `aukilabs/relay` infrastructure node — **both-gates** per Reid parking-lot 2c |
| `cluster` | yes | `/auki/cluster/1.0.0` participant-exchange request-response (ansuz #3); JSON codec, 30 s per-request timeout. See [the cluster protocol](#the-cluster-protocol-m1) for usage. Always-on; sits idle on swarms that don't participate in a cluster |

The swarm's `local_peer_id` matches `identity.peer_id()` exactly — caller can rely on this for advertising. Idle connections close after 60 s.

**Park-from-home dialing:** `swarm::dial_peer(&mut swarm, peer_id, vec![addr1, addr2, ...])`. Addresses may be direct (`/ip4/.../tcp/...`) or circuit-relay-mediated (`/p2p/<relay>/p2p-circuit/p2p/<target>`). Per Reid parking-lot 3c, the operator pastes the daemon's peer-id and (if needed) a relay multiaddr into Park's UI; no Discovery Service dependency for the M2 demo.

The `swarm` feature pulls in `libp2p` 0.56 + tokio runtime; non-WASM. Console depends on this crate without the feature (default-off) to derive peer ids from wallets in-browser.

## The cluster protocol (M1)

`/auki/cluster/1.0.0` is the libp2p half of `ParticipantInfo`'s **one schema, two transports** promise. A peer that reaches a daemon over HTTP (Park, querying `GET /api/info`) and a peer that reaches it over libp2p parse identical JSON out of either wire. The libp2p side is a request/response exchange; the request body is empty (`null`); the response is the responder's current `ParticipantInfo`.

```rust
use auki_network::cluster_protocol::{self, CLUSTER_PROTOCOL, ClusterRequest};
use libp2p::request_response;

// Build the swarm — the cluster protocol behaviour is wired in
// automatically as the always-on `cluster:` field on `swarm::Behaviour`.
let mut swarm = build_swarm(&identity, swarm_config)?;

// Ask another peer for its identity card.
let request_id = swarm.behaviour_mut().cluster.send_request(&peer_id, ClusterRequest);

// Receivers handle Request events themselves and call send_response —
// see the swarm event loop in your daemon for the typical pattern.
```

| | |
|---|---|
| Protocol id | `/auki/cluster/1.0.0` (constant `cluster_protocol::CLUSTER_PROTOCOL`) |
| Codec | `libp2p::request_response::json::Behaviour<ClusterRequest, ParticipantInfo>` (length-framed JSON over the libp2p stream) |
| Request body | `ClusterRequest` (unit struct → JSON `null`) |
| Response body | `ParticipantInfo` (same JSON as `GET /api/info`) |
| Per-request timeout | 30 s (constant `cluster_protocol::REQUEST_TIMEOUT`) |
| Wired into | `swarm::Behaviour.cluster` — always-on, no `Toggle` |

**The behaviour does not auto-respond.** A peer that receives a request gets `request_response::Event::Message::Request{ channel, .. }` and is responsible for filling in its current `ParticipantInfo` and calling `behaviour.cluster.send_response(channel, info)`. This is the standard libp2p pattern and it's what lets a Python sidecar's `participant_provider` callable run per-request, so `session_now_ns` is fresh on each reply rather than stale at swarm-spawn time.

**Always-on, no toggle.** The protocol sits idle for swarms that don't participate in a cluster (the dedicated `aukilabs/relay` infrastructure node) — there's no traffic until somebody sends a request. A toggle would just be ceremony.

**Higher-level orchestration lives separately.** Auto-dialing every peer in `cluster.json`, tracking `Joined`/`Left` events, holding a peer-state map — all of that lands in the upcoming cluster-runtime module (ansuz #4). Rust consumers that want fine control (Sentinel) drive the swarm event loop themselves and call `send_request` / `send_response` directly. The Python sidecar wraps the runtime opaquely via the planned `auki-py` `cluster.spawn`.

## `cluster.json` — the discovery doc

For the ansuz networking-demo milestone, peer discovery on a small cluster is a **static, hand-edited directory file** rather than a discovery service. A daemon reads `cluster.json` at startup to learn the peer-ids and dialable multiaddrs of every other daemon in the cluster. There is no liveness gossip and no auto-update — operator edits the file when the cluster topology changes.

### Why it's a directory, not a bootstrap list

Every entry has a known `peer_id`. libp2p Noise rejects connection-time mismatches, so the doc gives **identity continuity across daemon restarts**: a Boosterapp that reboots derives the same peer-id from its persisted wallet seed (per [`PeerIdentity`](#peeridentity)) and is therefore recognizable as the same Boosterapp by every other node that has it pinned.

This is intentionally narrower than a "bootstrap address list" (where addresses are hints and identities are learned). Pinned peer-ids are what makes long-running clusters survive operator restarts, IP churn, and certificate rotations.

### Schema

```json
{
  "version": 1,
  "cluster_name": "demo-2026-05",
  "peers": [
    {
      "peer_id": "12D3KooW...",
      "addresses": [
        "/ip4/192.168.1.10/tcp/4001",
        "/ip4/192.168.1.10/udp/4001/quic-v1"
      ],
      "expected_app_id": "boosterapp",
      "note": "robot 1 — K1 NUC"
    }
  ]
}
```

| Field | Required? | Meaning |
|-------|-----------|---------|
| `version` | yes | Schema version. v1 is the only currently supported value. |
| `cluster_name` | yes | Human-readable cluster identifier; surfaced in operator logs. |
| `peers` | yes | Ordered list of pinned peers. Empty list is valid. |
| `peers[].peer_id` | yes | libp2p `PeerId` (canonical base58 form). Used as the connection-time identity check. |
| `peers[].addresses` | yes (may be empty) | Dialable multiaddrs. Direct (`/ip4/.../tcp/...`) or circuit-relay-mediated (`/p2p/<relay>/p2p-circuit/p2p/<peer>`) both accepted. Empty list is allowed (operator may have temporarily removed all addresses while keeping the peer pinned). |
| `peers[].expected_app_id` | optional | Advisory `app_id` (e.g. `"boosterapp"`). **Not authoritative** — the wire-borne value (from `/api/info`) wins; the doc value is for fail-fast logging on mismatch. |
| `peers[].note` | optional | Free-form human note; the SDK preserves it but never reads it. |

### Path layout

```text
<app_root>/registries/cluster_registries/cluster.json
```

Sibling to the existing hash-keyed registries (`registries/sensors/`, `registries/clocks/`, `registries/frames/`). Unlike those, `cluster_registries/` is **flat** — one `cluster.json`, no per-cluster subdir, no hash-keyed entry files. Lifting the cluster doc into a Cluster Registry primitive is a future evolution if it earns one; ansuz keeps it a single file.

### Resolution

Daemons resolve the doc path by precedence: **CLI override → environment variable → default**. The CLI override is wired up by each integrator (typically a `--cluster-doc <path>` flag); the SDK exposes:

```rust
use auki_network::cluster_doc;

let path = cluster_doc::resolve_path(app_root, cli_override);  // honors AUKI_CLUSTER_DOC
let doc  = cluster_doc::load(&path)?;
```

The env var name is `AUKI_CLUSTER_DOC`; an empty value is treated as unset.

### Loader API

```rust
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

pub enum LoadError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    UnsupportedVersion(u32),
    InvalidPeerId(String),
    InvalidMultiaddr(String),
}

pub const SUPPORTED_VERSION: u32 = 1;
pub const ENV_OVERRIDE: &str = "AUKI_CLUSTER_DOC";
pub const DEFAULT_RELATIVE_PATH: &str = "registries/cluster_registries/cluster.json";

pub fn load(path: &Path) -> Result<ClusterDoc, LoadError>;
pub fn default_path(app_root: &Path) -> PathBuf;
pub fn resolve_path(app_root: &Path, cli_override: Option<&Path>) -> PathBuf;
```

Both `peer_id` and each multiaddr are typed in the parsed struct — invalid strings surface as `InvalidPeerId(String)` / `InvalidMultiaddr(String)` carrying the offending text, so an operator can fix the doc from the error message alone. Unknown future versions surface as `UnsupportedVersion(u32)` from a two-phase parse that peeks at `version` before attempting the typed deserialize.

## `app_instance`

Behind the `app_instance` feature. `auki_network::app_instance::derive() -> Result<String, DeriveError>` returns a per-machine identifier — the value the `app_instance` field of `/api/info` carries to distinguish two daemons of the same `app` running on different hardware.

**Recipe (locked per ansuz D4):** the first non-loopback IEEE-administered MAC address, lowercased hex without separators (`aabbccddeeff`).

```rust
use auki_network::app_instance;

let id = app_instance::derive()?; // e.g. "00163eabcdef"
```

The recipe in detail:

1. Enumerate the host's network interfaces (via the `mac_address` crate, which wraps `getifaddrs` / `GetAdaptersAddresses`).
2. Skip interfaces with no MAC.
3. Skip the loopback MAC (`00:00:00:00:00:00`).
4. Skip locally-administered MACs — the U/L bit (`0x02` on the first octet) is `1`. These are randomized / generated MACs (macOS Private Wi-Fi, Docker bridges, VMs, hypervisors) and not stable across reboots.
5. Sort the remaining MACs lexicographically by raw bytes; pick the first. Same hardware → same selection regardless of OS-level interface enumeration order.
6. Render as 12 lowercase hex chars, no separators.

**Stability caveats — fragile by design.** ansuz accepts these tradeoffs; a stable-id alternative is parked for later.

- **Containers / Docker** typically only see locally-administered MACs → `DeriveError::NoSuitableMac`.
- **VMs** with hypervisor-minted (locally-administered) MACs change identity across host migrations.
- **Multi-NIC machines** can change selection if a NIC is added or removed (the lexicographically smallest MAC may shift).
- **MAC randomization** (macOS Private Wi-Fi, `MACAddressPolicy=random`) is skipped; a wired NIC on the same machine still resolves.

The `app_instance` feature is non-WASM by design — it depends on `mac_address`, which uses platform syscalls. Lives behind its own feature so the M0 path (Console, in-browser) stays WASM-friendly.

```rust
pub mod app_instance {
    pub fn derive() -> Result<String, DeriveError>;
    pub fn derive_from(macs: &[[u8; 6]]) -> Result<String, DeriveError>;

    pub enum DeriveError {
        NoNetworkInterfaces,
        NoSuitableMac,
        Io(std::io::Error),
    }
}
```

`derive_from` is the testing seam — production calls `derive()` (which gathers MACs from the host); tests call `derive_from(&[…])` with fixtures.

## What this crate is *not*

- **Not a Discovery Service.** `ReachabilityRecord` is the wire shape; the lookup mechanism (mDNS for LAN, Discovery Service for cross-network) lives elsewhere. Park-from-home in v1 is operator-paste, not query.
- **Not DCUtR / hole-punching.** Connections through circuit-relay stay relayed for now; upgrading to direct via DCUtR is a future addition (small; not load-bearing for the M2 demo).
- **Not Layer 2 capability discovery.** A peer's `Capability` list is in its `ReachabilityRecord`; the libp2p protocol that advertises and queries capability lists at runtime is Layer 2 (post-M1).
- **Not a key store.** Same separation as `auki-identity`: this crate hands you a peer key derived from a wallet; persistence (encrypted-at-rest, OS keychain) is downstream.
- **Not a capability registry.** The crate fixes the format and surfaces the four canonical networking constants. Authoritative semantics for each capability live with the implementation that provides it (the Relay app for the four `networking:*` ones).
- **Not a Cluster Registry primitive.** `cluster.json` is a flat single-file directory, not hash-keyed like `Sensor` / `Clock` / `Frame` registries. Lifting it into a registry is a future evolution if it earns one; ansuz deliberately keeps it a config file. The doc is also unsigned for ansuz; cryptographic attestation of the cluster membership list is a future concern.

## WASM compatibility

M0 (default features off) is WASM-friendly by construction — `auki-identity`, `libp2p-identity`, and `multiaddr` all compile to WASM. Console can derive a peer id from an in-browser wallet without pulling in the transport stack. The `swarm` feature is non-WASM by design (libp2p's transports + tokio are native-only).

## Cross-language conformance

The peer-derivation recipe is two stable contracts plus libp2p's published encoding:

1. `peer_seed = Wallet::derive_child("peer/v1").seed()` — see `auki-identity`'s `derive_child` recipe.
2. `peer_keypair = ed25519::Keypair::from_secret(peer_seed)` — standard ed25519, RFC 8032.
3. `peer_id = libp2p PeerId(public_key)` — protobuf-encoded public key, then multihash. See [libp2p PeerId spec](https://github.com/libp2p/specs/blob/master/peer-ids/peer-ids.md).

`Capability` and `ReachabilityRecord` serialize as plain JSON; field names are stable and lower-snake-case.

## Versioning

`PEER_DERIVATION_LABEL` is `"peer/v1"`. A v2 label rotates the peer key without breaking the wallet (e.g. if the libp2p PeerId encoding changes). The four `networking:*` capability strings are wire-format and treated as immutable; new networking capabilities take new names. The identify protocol id `/auki/identify/1.0.0` is stable; bump the version segment if the agent_version semantics change in a way that affects parsers.
