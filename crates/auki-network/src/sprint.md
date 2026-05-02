# Sprint — auki-network

Current work and next steps to close the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now (M0 — landed)

- `PeerIdentity` deriving from a wallet via `derive_child("peer/v1")`.
- `ReachabilityRecord` with peer id, multiaddrs, capabilities, last-seen.
- `Capability` newtype with the four canonical `networking:*` constants.
- 11 tests covering the public contract.

## Next (M1 — libp2p Swarm)

The data types ship in M0 so Console + the Relay can start consuming them while the swarm-side defaults get worked out. M1 lands the actual transport:

- `Behaviour` aggregating `libp2p::core::transport` (TCP + QUIC + Noise + Yamux) and `libp2p::relay` (Circuit Relay v2 client + server).
- `Swarm` builder helper: takes a `PeerIdentity`, returns a configured swarm. `--relay-server` opt-in (off by default for consumer daemons; on for the dedicated `aukilabs/relay` infrastructure node).
- Listen-address selection from a `ReachabilityRecord`-shaped config.
- Dial-by-peer-id helper: given a `PeerId` and a list of multiaddrs (possibly circuit-relay multiaddrs), dial through.
- `_p2p._udp.local.` mDNS coexistence with the existing `_auki._tcp.local.` advertisement (parking-lot question; resolution drives the M1 default).

Tests: connect two swarms over loopback; verify circuit-relay client→server→client round-trip with the relay-server opt-in; verify the off-by-default consumer daemon refuses to act as a relay-server.

## After M1

- **Layer 2 — capability advertisement / discovery.** Per Reid: capability identifiers are the namespaced strings already in `Capability`; what's missing is the libp2p protocol that advertises a peer's capability list and lets others query it. Likely a `request-response` behaviour with a stable protocol id.
- **Layer 3 — cluster admission.** Manager-issued signed tokens (built on `auki-identity`'s `CreationCert`); cluster-registry as authoritative directory. Required for Domain participation.

## Open items

See [`parking_lot.md`](../parking_lot.md). The three biggest before M1 starts:

- Wallet → peer-key derivation path label format. `"peer/v1"` is shipped; whether to evolve to BIP32-style hardened paths later is in the parking lot.
- mDNS coexistence between libp2p's `_p2p._udp.local.` and the SDK's existing `_auki._tcp.local.`.
- Park-from-home access pattern: own Discovery Service query type, ride on capability recruitment, or stay manual-peer-id-paste for v1.
