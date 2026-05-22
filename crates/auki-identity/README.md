# auki-identity

The SDK's identity primitive — ed25519 keypair, deterministic label-based child derivation, and signed creation certs. One wallet seed regenerates every derived key on a fresh machine: libp2p peer id (via `derive_child("peer/v1")`), per-Domain owner keys, signing keys, and so on. WASM-friendly.

**Status:** Shipped.

## Public surface

- `Wallet`, `PublicKey`, `WalletId`, `Signature`, `CreationCert`
- `Wallet::from_seed`, `derive_child(label)`, `public_key()`, `sign_canonical_json(value)`
- `verify(...)`, `load_or_mint_seed(path) -> Vec<u8>`
- Locked vectors pin `derive_child("peer/v1")` and the `sign_canonical_json` chain across languages.

## Depends on

- [`auki-hash`](../auki-hash) — for content-hashing signed payloads.
- [`auki-jcs`](../auki-jcs) — for canonicalizing JSON before signing.
