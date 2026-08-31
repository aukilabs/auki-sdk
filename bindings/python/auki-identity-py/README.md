# auki-identity-py

PyO3 bindings for [`auki-identity`](../../../crates/auki-identity) — the wallet primitive Booster's Python sidecar uses to derive stable P2P seed material and a per-machine `app_instance` value.

**Status:** Shipped.

## Public surface

```python
import auki_identity

seed   = auki_identity.load_or_mint_seed(path)        # bytes (32-byte ed25519 seed; minted if missing)
wallet = auki_identity.Wallet.from_seed(seed)
peer   = wallet.derive_child("peer/v1")
peer_id = peer.peer_id()                              # canonical libp2p PeerId string
peer_seed = peer.seed()                               # secret 32-byte seed
seed_back = wallet.seed()                              # round-trip the seed bytes
mac_id  = auki_identity.app_instance.derive()         # MAC-derived per-machine id
```

Native Rust constructs the canonical network identity with
`auki_sdk::Identity::from_ed25519_seed(peer_seed)` before authorizing and
starting an `AukiPeer`. Its Peer ID must equal `peer.peer_id()` above. A
canonical Python `AukiPeer` facade is pending, so this binding currently
provides identity material to trusted native host adapters rather than starting
networking itself. Treat both wallet and peer seeds as private keys.

## Depends on

- [`auki-identity`](../../../crates/auki-identity) — Rust crate it wraps.
