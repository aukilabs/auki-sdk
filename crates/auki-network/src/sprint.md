# Sprint — auki-network

Current work and next steps to close the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now (M0 + M1a — landed)

- **M0** — `PeerIdentity`, `ReachabilityRecord`, `Capability`. WASM-friendly. 11 tests.
- **M1a** — libp2p `Swarm` builder behind the `swarm` feature. Transport: TCP + QUIC, Noise, Yamux. Behaviour: `identify` + `ping`. Two peers can dial each other and exchange identify info on either transport. 4 swarm tests + 1 doctest.

## Next (M1b — Circuit Relay v2 + mDNS coexistence)

- Add `libp2p::relay` to the `Behaviour`: `relay::client::Behaviour` always, `relay::Behaviour` (the server side) wrapped in `Toggle` so it's off by default for consumer daemons. Opt-in via a `SwarmConfig.enable_relay_server: bool` field (or equivalent — see the parking lot for the design question).
- `_p2p._udp.local.` mDNS coexistence with the existing `_auki._tcp.local.` advertisement. Resolve the parking-lot question first; default likely dual-channel for the demo.
- Dial-by-peer-id helper that handles circuit-relay multiaddrs (`/p2p/<relay>/p2p-circuit/p2p/<target>`).
- Tests: relay-server-on swarm forwards a circuit-relay-mediated dial between two relay-clients; relay-server-off swarm refuses to act as a hop.

## After M1b

- **Layer 2 — capability advertisement / discovery.** Per Reid: capability identifiers are the namespaced strings already in `Capability`; what's missing is the libp2p protocol that advertises a peer's capability list and lets others query it. Likely a `request-response` behaviour with a stable protocol id.
- **Layer 3 — cluster admission.** Manager-issued signed tokens (built on `auki-identity`'s `CreationCert`); cluster-registry as authoritative directory. Required for Domain participation.

## Open items

See [`parking_lot.md`](../parking_lot.md). The three biggest before M1b starts:

- mDNS coexistence between libp2p's `_p2p._udp.local.` and the SDK's existing `_auki._tcp.local.`.
- Off-by-default relay-server plumbing — boolean in `SwarmConfig`, per-capability gate, or both.
- Park-from-home access pattern: own Discovery Service query type, ride on capability recruitment, or stay manual-peer-id-paste for v1.
