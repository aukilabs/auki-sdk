# `auki-network/src/`

Networking substrate for the SDK. Spec: this crate's [outer `README.md`](../README.md).

## What's here

- [`lib.rs`](lib.rs) — M0 data types: `PeerIdentity`, `ReachabilityRecord`, `Capability`, plus the `multiaddr_vec_serde` adapter.
- [`swarm.rs`](swarm.rs) — M1a libp2p `Swarm` builder, gated behind the `swarm` feature.

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

pub const PEER_DERIVATION_LABEL: &str = "peer/v1";

// M1a (behind `swarm` feature)
pub mod swarm {
    pub struct Behaviour { /* derived from #[derive(NetworkBehaviour)] */ }
    pub struct SwarmConfig { listen_addresses: Vec<Multiaddr>, agent_version: String }
    pub enum BuildError { Transport(String), Listen { addr, source } }
    pub const IDENTIFY_PROTOCOL: &str = "/auki/identify/1.0.0";
    pub fn build_swarm(identity: &PeerIdentity, config: SwarmConfig) -> Result<libp2p::Swarm<Behaviour>, BuildError>;
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

## How `build_swarm` works (M1a)

```text
SwarmBuilder::with_existing_identity(identity.keypair().clone())
    .with_tokio()
    .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)
    .with_quic()
    .with_behaviour(|key| Behaviour {
        identify: identify::Behaviour::new(
            identify::Config::new("/auki/identify/1.0.0", key.public())
                .with_agent_version(config.agent_version)
        ),
        ping:     ping::Behaviour::default(),
    })
    .with_swarm_config(|c| c.with_idle_connection_timeout(60s))
    .build()
    + listen_on(each address in config.listen_addresses)
```

`build_swarm` does the listening — caller doesn't need to call `swarm.listen_on` afterwards.

## Serde shape

`ReachabilityRecord` and `Capability` round-trip through JSON. `PeerId` serializes as its canonical multibase-base58 string (via `libp2p-identity`'s `serde` feature). `Multiaddr` lacks serde in `multiaddr` 0.18, so the crate ships a small adapter that serializes each as its text form (`/ip4/.../tcp/...`).

## Tests

15 unit tests + 1 doctest. Run with `cargo test -p auki-network --features swarm` for the full set; `cargo test -p auki-network` runs only the M0 tests (11).

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
| `swarm::local_peer_id_matches_identity` | Built swarm's `local_peer_id` equals `identity.peer_id()` |
| `swarm::two_peers_identify_each_other_over_tcp` | TCP dial → Noise handshake → identify exchange both ways |
| `swarm::two_peers_identify_each_other_over_quic` | Same as above, over QUIC |
| `swarm::build_listens_on_all_provided_addresses` | Both listen addresses produce `NewListenAddr` events |
| doctest in `swarm.rs` | Builder example compiles |

## Dependencies

- `auki-identity` — wallet primitive; source of `derive_child("peer/v1")`.
- `libp2p-identity` (0.2, `ed25519` + `peerid` + `serde` features, `default-features = false`) — keypair, public key, PeerId encoding.
- `multiaddr` (0.18) — typed multiaddr; serde adapter local to this crate.
- `serde` — derive on `Capability` and `ReachabilityRecord`.
- *(swarm feature)* `libp2p` (0.56, features: `tokio`, `tcp`, `quic`, `noise`, `yamux`, `identify`, `ping`, `macros`, `ed25519`) — the swarm itself.
- *(swarm feature)* `thiserror` (2) — `BuildError`.
- *(dev)* `tokio` (`macros`, `rt-multi-thread`, `time`) + `futures` for the swarm tests.

## Consumers in this workspace

- *(planned, downstream)* `aukilabs/relay` — implements the four `networking:*` capabilities on top of the M1a swarm; M1b adds Circuit Relay v2 to enable cross-LAN.
- *(planned, downstream)* BoosterApp / Sentinel — consume the swarm builder; register `ReachabilityRecord`s with a configured Relay (M1b).
- *(planned, downstream)* Park — dial daemons by peer-id via the Relay (M1b).
- *(planned, downstream)* Console — depends on `auki-network` *without* the `swarm` feature; uses M0 only to display a wallet's `peer_id` in-browser.
