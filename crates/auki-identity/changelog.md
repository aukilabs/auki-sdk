# Changelog — auki-identity

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 4, 16:39 HKT, 2026

`load_or_mint_seed(path: &Path) -> Result<[u8; 32], SeedError>` added in new [`src/seed.rs`](src/seed.rs) — the small filesystem helper that backs ansuz's "stable peer key across restarts" guarantee (deliverable #6 of the four parallel Batch-1 deliverables). On first call it mints 32 cryptographically-random bytes from `OsRng`, creates parent directories with `create_dir_all`, writes atomically (`<path>.tmp` → fsync → rename), sets mode `0o600` on Unix, and returns the bytes; on subsequent calls it reads the existing file and rejects anything that isn't exactly 32 bytes via `SeedError::InvalidLength(usize)`. The function takes any `&Path` — the `~/.auki/<app>/identity.seed` convention is documented in prose only, never baked into the signature, so tests, ephemeral daemons, and alternative layouts aren't locked in. Module gated `#[cfg(not(target_arch = "wasm32"))]` so the rest of the crate stays WASM-clean for Console; the seed helper is the single fs-touching exception, re-exported at the crate root. `tempfile` added as a dev-dep. 9 new tests (16 → 25): mint-and-persist; idempotent second call; pre-existing 32-byte file round-trip; wrong-length rejection (0/1/31/33/64 bytes); deep parent-directory creation; no `.tmp` sidecar after a successful write; weak entropy smoke (not all zeros, two mints differ); Unix-only `0o600` mode check; round-trip with `Wallet::from_seed`. Out of scope by design (deferred per existing parking-lot items): encryption-at-rest, OS-keychain integration, BIP39 mnemonic backup. Will be consumed by the planned `ansuz` networking-demo daemon. No tag yet — wait until ansuz earns one.

---

### broodsugar's claude · May 2, 14:30 HKT, 2026

Crate created. Foundation for everything cryptographic in the SDK — wallet primitive, ed25519 sign / verify, deterministic child derivation (`derive_child(label)`) for keys regenerable from a wallet seed, and signed creation certs (`issue_creation_cert(child, label, ts)`) for vouching for independent child keypairs. Two distinct flavours; not interchangeable. WASM-friendly (no `std::fs`, no platform syscalls); designed so Console can compile this crate to in-browser WASM. 16 tests covering keypair generation, determinism, sign/verify, ID derivation, both child-derivation modes, and creation-cert tampering detection. Built on `ed25519-dalek` 2.x; `WalletId` and `derive_child` seed expansion both use `auki-hash`. Parking-lot items: BIP32-style HD vs labeled-hash derivation; encrypted-at-rest format; BIP39 mnemonics; v2 signing-scheme migration shape. No tag yet — `auki-network` and Console will be the first consumers; tag when one of them earns it.
