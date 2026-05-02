# Changelog — auki-identity

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 2, 14:30 HKT, 2026

Crate created. Foundation for everything cryptographic in the SDK — wallet primitive, ed25519 sign / verify, deterministic child derivation (`derive_child(label)`) for keys regenerable from a wallet seed, and signed creation certs (`issue_creation_cert(child, label, ts)`) for vouching for independent child keypairs. Two distinct flavours; not interchangeable. WASM-friendly (no `std::fs`, no platform syscalls); designed so Console can compile this crate to in-browser WASM. 16 tests covering keypair generation, determinism, sign/verify, ID derivation, both child-derivation modes, and creation-cert tampering detection. Built on `ed25519-dalek` 2.x; `WalletId` and `derive_child` seed expansion both use `auki-hash`. Parking-lot items: BIP32-style HD vs labeled-hash derivation; encrypted-at-rest format; BIP39 mnemonics; v2 signing-scheme migration shape. No tag yet — `auki-network` and Console will be the first consumers; tag when one of them earns it.
