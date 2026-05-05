# Sprint — auki-network

Current work and next steps to close the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now (M0 + M1 + all ansuz — landed)

- **M0** — `PeerIdentity`, `ReachabilityRecord`, `Capability`. WASM-friendly. 11 tests.
- **M1a** — libp2p `Swarm` builder behind the `swarm` feature. Transport: TCP + QUIC, Noise, Yamux. Behaviour: `identify` + `ping`. 4 tests + 1 doctest.
- **M1b** — Circuit Relay v2 client (always) + relay-server (gated on `SwarmConfig.enable_relay_server`, `Toggle`-wrapped, off by default for consumer daemons). mDNS (`Toggle`-wrapped on `SwarmConfig.enable_mdns`, on by default for daemons — dual-channel coexistence with the existing `_auki._tcp.local.` advertisement). `dial_peer` helper for Park-from-home circuit-relay dialing. 4 new tests; 19 unit tests + 1 doctest total.
- **ansuz #1** — `cluster.json` discovery-doc spec + loader. `cluster_doc` module (always-on; native-only via `std::fs`); `ClusterDoc` / `ClusterPeer` types; `load`, `default_path`, `resolve_path`; `LoadError` with five variants. 16 new unit + 3 integration tests.
- **ansuz #2b** — `ParticipantInfo` wire shape (the `participant` module). One schema, two transports — same JSON over `GET /api/info` (HTTP) and `/auki/cluster/1.0.0` (libp2p). M0 path; no `swarm` feature required. 8 new tests including locked golden bytes.
- **ansuz #3** — `/auki/cluster/1.0.0` request-response protocol. New `cluster_protocol` module (`swarm`-gated) wraps `libp2p::request_response::json::Behaviour<ClusterRequest, ParticipantInfo>` with the protocol id pin and a 30s timeout; wired into the swarm `Behaviour` as an always-on field. The behaviour does **not** auto-respond — receivers handle `Request` events themselves and call `send_response`, which is the libp2p-idiomatic way to plug in a fresh-`session_now_ns` provider (`auki-py`'s `participant_provider` lands here). 3 new tests.
- **ansuz #4** — `ClusterRuntime` (`cluster_runtime` module, `swarm`-gated). Opaque runtime owning its own `Swarm<Behaviour>` + tokio task; auto-dials peers from `cluster.json`, exchanges `ParticipantInfo` on connect, exposes `peers() -> Vec<PeerSnapshot>` for any-thread reads, reconnects on disconnect with per-peer exponential backoff (1 s → cap 60 s). Trust boundary is the cluster doc — inbound from peers not in the doc is dropped. `participant_provider` invoked per inbound request so `session_now_ns` is fresh. **Closes ansuz Batch 2.** 7 new tests; 54 unit + 3 integration + 2 doctest with `--features swarm`.
- **ansuz #5** — `app_instance::derive()` behind the default-off `app_instance` feature. First non-loopback IEEE-administered MAC, lowercased hex without separators (`aabbccddeeff`); deterministic across reboots on a fixed hardware set. 9 new tests including the locked cross-language vector.

The three Reid milestone-2 parking-lot questions are resolved and encoded in code: dual-channel mDNS (1a), both-gates relay-server (2c), manual peer-id paste for Park-from-home (3c). The six ansuz decisions (D1–D6) are resolved on Notion; this crate has implemented D1 (strict — `peer_id` required in `cluster.json`), D2 (libp2p `/auki/cluster/1.0.0`, no HTTP fallback), D4 (MAC-derived `app_instance`).

## Next

The crate's ansuz scope is fully shipped. Forward-looking work, not blocking:

- **`auki-py` PyO3 wrapper** lives in a sibling crate (per the booster claude ask — Boosterapp's Python sidecar wraps `cluster_runtime` opaquely via `cluster.spawn`). That's a separate crate, not a follow-up here.
- **Daemon integration** (ansuz Batch 3 — Boosterapp #7, Sentinel #8, Park #10) consumes this crate; not done in this crate.
- **Layer 2 — capability advertisement / discovery.** Per the Reid architecture: capability identifiers are the namespaced strings already in `Capability`; what's missing is the libp2p protocol that advertises a peer's capability list at runtime and lets others query it. Likely a `libp2p::request_response` behaviour with a stable protocol id (`/auki/capabilities/1.0.0`); the `cluster_protocol` codec pattern is the template (request empty, response is a `ReachabilityRecord`). This becomes the runtime back-end for the Discovery Service shape that lands alongside Domain participation.

## Smaller follow-ups

- **DCUtR (hole-punching).** Optional; upgrades a relayed connection to a direct one. Add `libp2p::dcutr::Behaviour` to the composition. Small, additive; not load-bearing for the M2 demo.
- **AutoNAT.** Lets a peer determine whether it's directly reachable. Useful for daemons to decide whether to register reachability via a relay. `libp2p::autonat::Behaviour`.
- **Persistent peer-id**. Documented end-to-end: `auki-identity::load_or_mint_seed` ships, `cluster_runtime::spawn` accepts a 32-byte seed; the integrator persists the seed (`~/.auki/<app>/identity.seed` is the convention).

## Open items

See [`parking_lot.md`](../parking_lot.md). Remaining items are forward-looking, not M1-blocking:

- Wallet → peer-key derivation label evolution (`peer/v1` shipped; future BIP32-style migration).
- `ReachabilityRecord` extensibility / versioning before any consumer relies on the shape being stable.
- `SwarmConfig` knob minimalism — when to expose idle/ping/connection-limit knobs.
- `BuildError::Transport(String)` structure — String vs boxed source vs enumerated variants.
