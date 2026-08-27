# auki-identity

The SDK's wallet primitive — ed25519 keypair, deterministic label-based child derivation, and signed creation certs. One wallet seed regenerates every derived key on a fresh machine, including the seed a host passes from `derive_child("peer/v1")` to the canonical `auki_p2p::Identity`. WASM-friendly; this crate does not own a network runtime.

**Status:** Shipped.

## Public surface

- `Wallet`, `PublicKey`, `WalletId`, `Signature`, `CreationCert`
- `Wallet::from_seed`, `derive_child(label)`, `public_key()`, `sign_canonical_json(value)`
- `verify(...)`, `load_or_mint_seed(path) -> Vec<u8>`
- Locked vectors pin `derive_child("peer/v1")` and the `sign_canonical_json` chain across languages.

## Depends on

- [`auki-hash`](../auki-hash) — for content-hashing signed payloads.
- [`auki-jcs`](../auki-jcs) — for canonicalizing JSON before signing.
