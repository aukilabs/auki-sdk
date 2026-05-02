# Sprint — auki-network

Current work and next steps to close the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now (M0 + M1 — landed)

- **M0** — `PeerIdentity`, `ReachabilityRecord`, `Capability`. WASM-friendly. 11 tests.
- **M1a** — libp2p `Swarm` builder behind the `swarm` feature. Transport: TCP + QUIC, Noise, Yamux. Behaviour: `identify` + `ping`. 4 tests + 1 doctest.
- **M1b** — Circuit Relay v2 client (always) + relay-server (gated on `SwarmConfig.enable_relay_server`, `Toggle`-wrapped, off by default for consumer daemons). mDNS (`Toggle`-wrapped on `SwarmConfig.enable_mdns`, on by default for daemons — dual-channel coexistence with the existing `_auki._tcp.local.` advertisement). `dial_peer` helper for Park-from-home circuit-relay dialing. 4 new tests; 19 unit tests + 1 doctest total.

The three Reid milestone-2 parking-lot questions are resolved and encoded in code: dual-channel mDNS (1a), both-gates relay-server (2c), manual peer-id paste for Park-from-home (3c).

## Next (Layer 2 — capability advertisement / discovery)

Per the Reid architecture: capability identifiers are the namespaced strings already in `Capability`; what's missing is the libp2p protocol that advertises a peer's capability list at runtime and lets others query it. Likely a `libp2p::request_response` behaviour with a stable protocol id (`/auki/capabilities/1.0.0`).

Wire shape: the request is empty (`()`); the response is a `ReachabilityRecord`. Consumers cache the response keyed by `(PeerId, last_seen_ns)`.

This becomes the runtime back-end for the Discovery Service shape that lands alongside Domain participation.

## Smaller follow-ups

- **DCUtR (hole-punching).** Optional; upgrades a relayed connection to a direct one. Add `libp2p::dcutr::Behaviour` to the composition. Small, additive; not load-bearing for the M2 demo.
- **AutoNAT.** Lets a peer determine whether it's directly reachable. Useful for daemons to decide whether to register reachability via a relay. `libp2p::autonat::Behaviour`.
- **Persistent peer-id**. Today daemons mint a new `Wallet::new()` each run unless they persist the seed themselves. Document the expected seed-persistence pattern (per Console question, 2026-05-02: apps mint own; persist locally).

## Open items

See [`parking_lot.md`](../parking_lot.md). Remaining items are forward-looking, not M1-blocking:

- Wallet → peer-key derivation label evolution (`peer/v1` shipped; future BIP32-style migration).
- `ReachabilityRecord` extensibility / versioning before any consumer relies on the shape being stable.
- `SwarmConfig` knob minimalism — when to expose idle/ping/connection-limit knobs.
- `BuildError::Transport(String)` structure — String vs boxed source vs enumerated variants.
