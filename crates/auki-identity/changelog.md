# Changelog — auki-identity

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's claude · May 21, 15:41 HKT, 2026

**Added optional `swift-bindings` cargo feature; annotated `Wallet`.** Adds `swift-bindings = ["dep:uniffi", "dep:thiserror"]` feature gating UniFFI proc-macros on `Wallet` (`#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]` + split impl exposing `new`, `from_seed`, `seed`, new `wallet_id_str` helper). UniFFI 0.31 type-system constraints forced two upstream-shape changes (additive: defaults unchanged):
- `Wallet::from_seed`: `&[u8; 32] -> Self` → `Vec<u8> -> Result<Arc<Self>, IdentityError>` (length-checked, `Arc<Self>` per UniFFI Object constructor contract).
- `Wallet::seed`: `[u8; 32] -> Vec<u8>`.
- `Wallet::new`: `Self -> Arc<Self>` (UniFFI Object constructor contract).
- New `IdentityError` enum (`InvalidSeedLength { actual: u32 }`), feature-gated with `uniffi::Error` + `thiserror::Error`; mirror under default builds keeps message format byte-identical.
- Hand-rolled redacting `Debug for Wallet` so test panics don't leak the ed25519 secret key.
- `Wallet::derive_child`'s internal construction inlined to bypass the new Result/Arc shape.
- Feature-gated `uniffi::setup_scaffolding!()` at the crate root (UniFFI 0.31 proc-macros reference `crate::UniFfiTag`).

Consumed by the new `bindings/swift/auki-identity-swift` crate (Spec 1 PR A). Methods kept off the UniFFI export at PR A: `public_key`, `id`, `sign`, `sign_canonical_json`, `derive_child`, `issue_creation_cert` — they remain on `Wallet` in a non-annotated impl block; later PRs lift them when iosapp needs them.

### Nils's codex · May 15, 11:40 HKT, 2026

**Documentation refresh: stable peer-key prose now points at the current `PeerIdentity` / `ClusterManager` path.** The README no longer describes static cluster-doc peer pinning as the consumer of `load_or_mint_seed`; it now explains that a persisted wallet seed lets `auki-network::PeerIdentity::from_wallet` regenerate the same peer id, which `auki-domain::ClusterManager` advertises to Discovery and cluster members across daemon restarts. No code changed in this crate.

### broodsugar's dobby · May 8, 07:56 HKT, 2026

[`parking_lot.md`](parking_lot.md) gains an item flagging the crate's missing `Result<T>` alias for ergonomics. Sister crates [`auki-logs`](../auki-logs/src/lib.rs) and [`auki-registry`](../auki-registry/src/lib.rs) ship `pub type Result<T> = std::result::Result<T, Error>` at crate root; this one does not, so downstream consumers spell out `Result<T, VerifyError>` longhand. The crate has two distinct error types (`VerifyError` for signature verification, `SeedError` for seed I/O), so a single crate-level `Result<T>` would mismatch — three forward paths sketched, lean toward (1) two aliases (`VerifyResult<T>` + `SeedResult<T>`) mirroring the two-axis nature of the crate. No urgency; pin while still pre-1.0.

### broodsugar's claude · May 6, 10:30 HKT, 2026

`Wallet::sign_canonical_json(value: &serde_json::Value) -> (Vec<u8>, Signature)` added — JCS-canonicalize a JSON value via `auki-jcs` and sign the canonical bytes with the wallet's ed25519 key, returning both so callers can ship the signature on the wire AND inspect / log the bytes that got signed. Generic primitive; the [Vinland](https://www.notion.so/3585c8e9659280699681caec256e0616) signed-registration to Discovery is the first consumer (sign a registration body minus the `signature` field; verifier reproduces the canonical bytes locally and verifies the signature against them — same canonicaliser on both sides means the two cannot drift). Any future Vinland-shaped wire format with `cluster_name`-bound replay-resistant signed payloads reuses this directly. New deps: `auki-jcs` (workspace path-dep, pure Rust, WASM-clean) and `serde_json` (already a dev-dep; promoted to regular dep because `serde_json::Value` appears in the public signature). Plus a locked cross-language conformance vector — `tests::locked_sign_canonical_json_vector` pins `Wallet::from_seed([3u8; 32]).sign_canonical_json(<vinland-shaped registration JSON in non-sorted order>)` to exact canonical bytes (RFC 8785 sorted) plus the exact 64-byte ed25519 signature. Joins the existing `auki-hash` / `auki-identity` / `auki-network` cross-language conformance set; downstream Vinland verifiers (Discovery, the planned Python sidecar) pin against this. 5 new tests (round-trip, deterministic, JCS-key-order normalisation, verifier rejects tampered field, locked vector); auki-identity test count 25 → 30. Vinland Batch 1 deliverable #1; piece #2 is `auki-network::discovery_client`.

---

### broodsugar's claude · May 4, 17:30 HKT, 2026

Locked cross-language conformance vector for `Wallet::from_seed([3u8; 32]).derive_child("peer/v1").public_key()` → fixed 32-byte ed25519 pubkey `1080633bcb57bac066cf8446e2b7ae711571cb04be0b46bdaf03146317bfe707`. New `tests::locked_derive_child_peer_v1_pubkey_vector` pins the bytes; if anything in the seed → ed25519 keypair → labeled-hash child seed → child ed25519 keypair → child pubkey chain drifts, every reimpl in another language drifts with it. Pairs with `auki-network`'s new `locked_seed_to_peer_id_vector` (uses the same `[3u8; 32]` seed, asserts the canonical libp2p PeerId derived from the parent wallet) — the two together pin the `Wallet → libp2p PeerId` chain that `cluster.json` relies on. Pattern matches `auki-hash`'s existing locked vectors. Surfaced via a new "Cross-language conformance vectors" section in the root [`README.md`](../../README.md). 1 new test (25 → 26). Cheap insurance ahead of v0.0.11.

---

### broodsugar's claude · May 4, 16:39 HKT, 2026

`load_or_mint_seed(path: &Path) -> Result<[u8; 32], SeedError>` added in new [`src/seed.rs`](src/seed.rs) — the small filesystem helper that backs ansuz's "stable peer key across restarts" guarantee (deliverable #6 of the four parallel Batch-1 deliverables). On first call it mints 32 cryptographically-random bytes from `OsRng`, creates parent directories with `create_dir_all`, writes atomically (`<path>.tmp` → fsync → rename), sets mode `0o600` on Unix, and returns the bytes; on subsequent calls it reads the existing file and rejects anything that isn't exactly 32 bytes via `SeedError::InvalidLength(usize)`. The function takes any `&Path` — the `~/.auki/<app>/identity.seed` convention is documented in prose only, never baked into the signature, so tests, ephemeral daemons, and alternative layouts aren't locked in. Module gated `#[cfg(not(target_arch = "wasm32"))]` so the rest of the crate stays WASM-clean for Console; the seed helper is the single fs-touching exception, re-exported at the crate root. `tempfile` added as a dev-dep. 9 new tests (16 → 25): mint-and-persist; idempotent second call; pre-existing 32-byte file round-trip; wrong-length rejection (0/1/31/33/64 bytes); deep parent-directory creation; no `.tmp` sidecar after a successful write; weak entropy smoke (not all zeros, two mints differ); Unix-only `0o600` mode check; round-trip with `Wallet::from_seed`. Out of scope by design (deferred per existing parking-lot items): encryption-at-rest, OS-keychain integration, BIP39 mnemonic backup. Will be consumed by the planned `ansuz` networking-demo daemon. No tag yet — wait until ansuz earns one.

---

### broodsugar's claude · May 2, 14:30 HKT, 2026

Crate created. Foundation for everything cryptographic in the SDK — wallet primitive, ed25519 sign / verify, deterministic child derivation (`derive_child(label)`) for keys regenerable from a wallet seed, and signed creation certs (`issue_creation_cert(child, label, ts)`) for vouching for independent child keypairs. Two distinct flavours; not interchangeable. WASM-friendly (no `std::fs`, no platform syscalls); designed so Console can compile this crate to in-browser WASM. 16 tests covering keypair generation, determinism, sign/verify, ID derivation, both child-derivation modes, and creation-cert tampering detection. Built on `ed25519-dalek` 2.x; `WalletId` and `derive_child` seed expansion both use `auki-hash`. Parking-lot items: BIP32-style HD vs labeled-hash derivation; encrypted-at-rest format; BIP39 mnemonics; v2 signing-scheme migration shape. No tag yet — `auki-network` and Console will be the first consumers; tag when one of them earns it.
