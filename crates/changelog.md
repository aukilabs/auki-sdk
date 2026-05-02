# Changelog — crates

One-line summaries of changes in any crate, propagated up from per-crate `changelog.md` files. See [CLAUDE.md](../CLAUDE.md).

Latest entry on top.

---

### broodsugar's claude · May 2, 18:45 HKT, 2026

`auki-network`: M1b landed — Circuit Relay v2 (client always; server gated on `SwarmConfig.enable_relay_server`, off by default for consumer daemons), libp2p mDNS (`_p2p._udp.local.`, gated on `SwarmConfig.enable_mdns`, on by default for daemons — dual-channel with the existing `_auki._tcp.local.`), and a `dial_peer` helper for Park-from-home circuit-relay dialing. Encodes all three resolved Reid milestone-2 parking-lot answers (1a/2c/3c). 4 new tests; 19 unit tests + 1 doctest total. Layer 2 (capability advertisement / discovery) is the next chunk. See `auki-network/changelog.md` for detail.

### broodsugar's claude · May 2, 17:30 HKT, 2026

`auki-network`: M1a landed — libp2p `Swarm` builder behind a default-off `swarm` feature. TCP + QUIC transports under Noise + Yamux; `identify` + `ping` behaviour. `build_swarm(&PeerIdentity, SwarmConfig)` returns a configured swarm already listening on the requested addresses; identify protocol id `/auki/identify/1.0.0`. 4 swarm tests + 1 doctest cover dial-and-mutual-identify over both TCP and QUIC; the no-feature M0 path stays WASM-compilable for Console. M1b (Circuit Relay v2 + mDNS coexistence) is the next chunk. See `auki-network/changelog.md` for detail.

### broodsugar's claude · May 2, 16:10 HKT, 2026

`auki-network`: new crate — Layer 1 of the Reid milestone-2 networking stack, data types only. `PeerIdentity` (libp2p ed25519 keypair derived from a wallet via `derive_child("peer/v1")`), `ReachabilityRecord` (peer id + multiaddrs + capabilities + last-seen, JSON-serializable), `Capability` (namespaced-string newtype with the four canonical `networking:*` constants). M1 (libp2p Swarm with TCP/QUIC + Noise + Yamux + Circuit Relay v2) lands on top of these. WASM-friendly. See `auki-network/changelog.md` for detail.

### broodsugar's claude · May 2, 14:30 HKT, 2026

`auki-identity`: new crate. Wallet primitive (ed25519 keypair + sign/verify), deterministic child derivation, signed creation certs. WASM-friendly. Foundation for `auki-network` and the Console. See `auki-identity/changelog.md` for detail.

### broodsugar's claude · May 2, 13:50 HKT, 2026

`auki-registry`: added audio sensor support — `SensorBody::Microphone` variant + `AudioLogEntry` payload (PCM only in v1; multi-mic array = one sensor with `channels = N`). See `auki-registry/changelog.md` for detail.

### broodsugar's claude · May 1, 19:28 HKT, 2026

`auki-session`: `sensorlog_path` drops its `sensor_id` parameter — recording = one sensor stream; sensor identity lives in the manifest, not the path. Breaking; tagged for consumer coordination as v0.0.7. See `auki-session/changelog.md` for detail.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Per-crate changelogs bootstrapped — all seven crates now have their own `changelog.md`. Resolved the matching parking-lot item.
