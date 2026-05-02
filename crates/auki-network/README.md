# auki-network

Networking substrate for the Auki SDK. Layer 1 of the Reid milestone-2 networking stack: peer identity, reachability records, and named capabilities — the data types every other piece of the network rides on. The libp2p `Swarm` (TCP/QUIC + Noise + Yamux + Circuit Relay v2) lands on top of these in M1.

## What a peer is

Per the broader Auki architecture, every node has *two* identities:

- **Wallet** — economic / policy / ownership. Lives in [`auki-identity`](../auki-identity).
- **Peer** — network / dialability. Lives here.

The peer identity is *derived* from the principal wallet via `Wallet::derive_child("peer/v1")`, so a backup of the wallet seed lets you regenerate the peer key. The peer key has its own libp2p `PeerId` and is what shows up in multiaddrs as `/p2p/<peer-id>`. Compromise blast-radius is separated: rotating the peer key (a re-derivation under a future label like `peer/v2`) doesn't invalidate the wallet.

## Three primitives

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

### `Capability`

A namespaced string identifying what a peer offers. Format is `"<namespace>:<name>"`. Forward-extensible without crate changes — new capabilities are just new strings. The four canonical networking capabilities (per the Reid milestone-2 architecture) are exposed as `&str` constants:

| Constant | String | Role |
|----------|--------|------|
| `Capability::MESSAGE_FORWARDING` | `networking:message-forwarding` | Hagall-`rosrelay` parity — small frequent control-plane messages |
| `Capability::BULK_DATA_CHANNEL` | `networking:bulk-data-channel` | Large non-real-time binary transfer |
| `Capability::TURN` | `networking:turn` | Real-time media P2P fallback |
| `Capability::SFU` | `networking:sfu` | Real-time media one-to-many fan-out |

Other namespaces (`discovery:*`, `compute:*`, etc.) are open. The Relay app implements the four `networking:*` capabilities; daemons advertise the ones they offer; consumers filter by namespace or specific value.

## What this crate is *not*

- **Not a swarm.** No transport, no NAT traversal, no circuit relay yet. M1 adds the libp2p `Swarm` with TCP/QUIC + Noise + Yamux + Circuit Relay v2 (client + server, both off by default for consumer daemons; opt-in for stable infrastructure nodes).
- **Not a directory.** Discovery — both LAN mDNS and remote Discovery Service — produces and consumes `ReachabilityRecord`s but lives elsewhere. The crate publishes the wire shape, not the lookup mechanism.
- **Not a key store.** Same separation as `auki-identity`: this crate hands you a peer key derived from a wallet; persistence (encrypted-at-rest, OS keychain) is downstream.
- **Not a capability registry.** The crate fixes the format and surfaces the four canonical networking constants. Authoritative semantics for each capability live with the implementation that provides it (the Relay app for the four `networking:*` ones).

## WASM compatibility

M0 is WASM-friendly by construction — `auki-identity`, `libp2p-identity`, and `multiaddr` all compile to WASM. Console can derive a peer id from an in-browser wallet without the swarm. M1's swarm pulls in transports and async runtime that aren't WASM; the data-types/swarm split keeps Console's path uncluttered.

## Cross-language conformance

The peer-derivation recipe is two stable contracts plus libp2p's published encoding:

1. `peer_seed = Wallet::derive_child("peer/v1").seed()` — see `auki-identity`'s `derive_child` recipe.
2. `peer_keypair = ed25519::Keypair::from_secret(peer_seed)` — standard ed25519, RFC 8032.
3. `peer_id = libp2p PeerId(public_key)` — protobuf-encoded public key, then multihash. See [libp2p PeerId spec](https://github.com/libp2p/specs/blob/master/peer-ids/peer-ids.md).

`Capability` and `ReachabilityRecord` serialize as plain JSON; field names are stable and lower-snake-case.

## Versioning

`PEER_DERIVATION_LABEL` is `"peer/v1"`. A v2 label rotates the peer key without breaking the wallet (e.g. if the libp2p PeerId encoding changes). The four `networking:*` capability strings are wire-format and treated as immutable; new networking capabilities take new names.
