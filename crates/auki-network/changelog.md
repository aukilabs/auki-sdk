# Changelog — auki-network

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 2, 16:10 HKT, 2026

Crate created — Layer 1 of the Reid milestone-2 networking stack. Ships the data types only: `PeerIdentity` (libp2p ed25519 keypair derived from a wallet via `derive_child("peer/v1")`, with `peer_id()` / `public_key()` / `keypair()` accessors), `ReachabilityRecord` (peer id + multiaddrs + capabilities + last-seen, JSON-serializable wire shape for peer discovery), and `Capability` (namespaced-string newtype with the four canonical `networking:*` constants from the Reid milestone-2 architecture: `MESSAGE_FORWARDING`, `BULK_DATA_CHANNEL`, `TURN`, `SFU`). 11 tests covering determinism of derivation, the public `from_wallet ≡ from_seed(derive_child("peer/v1").seed())` contract, JSON round-trips, and capability namespace extraction. WASM-friendly (`libp2p-identity` + `multiaddr` both compile to WASM); deliberately split from M1's libp2p `Swarm` so Console can derive a peer id from an in-browser wallet without pulling in the transport stack. Built on `auki-identity`, `libp2p-identity` 0.2 (`ed25519` + `peerid` + `serde` features, `default-features = false`), and `multiaddr` 0.18 (with a small local serde adapter — `multiaddr` 0.18 dropped its serde feature). M1 will add the libp2p `Swarm` (TCP/QUIC + Noise + Yamux + Circuit Relay v2) on top of these primitives. Parking-lot items: mDNS coexistence (`_p2p._udp.local.` vs `_auki._tcp.local.`); Wallet→peer-key derivation label evolution; Park-from-home access pattern; off-by-default relay-server plumbing; `ReachabilityRecord` extensibility / versioning. No tag yet — wait until M1 lands or the Relay app earns it.
