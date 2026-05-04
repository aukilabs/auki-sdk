# `auki-identity/src/`

Wallet primitive for the SDK. Spec: this crate's [outer `README.md`](../README.md).

## What's here

- [`lib.rs`](lib.rs) — wallet primitive (ed25519 keypair, sign/verify, child derivation, creation certs).
- [`seed.rs`](seed.rs) — `load_or_mint_seed` filesystem helper. Native-only (`#[cfg(not(target_arch = "wasm32"))]`); the rest of the crate stays WASM-clean.

## Public types

```rust
pub struct Wallet { /* ed25519 SigningKey, sensitive */ }

pub struct PublicKey(pub [u8; 32]);
pub struct Signature(pub [u8; 64]);
pub struct WalletId(pub String);  // 32-char lowercase hex via auki-hash

pub struct CreationCert {
    pub parent_pubkey: PublicKey,
    pub child_pubkey: PublicKey,
    pub label: String,
    pub created_at_ns: i64,
    pub signature: Signature,
}

pub enum VerifyError {
    BadPublicKey,
    SignatureMismatch,
}

// Native-only (not on wasm32)
pub enum SeedError {
    Io(std::io::Error),
    InvalidLength(usize),
}
```

## Public functions

```rust
// Wallet construction
impl Wallet {
    pub fn new() -> Wallet;                         // fresh random
    pub fn from_seed(seed: &[u8; 32]) -> Wallet;
    pub fn seed(&self) -> [u8; 32];
    pub fn public_key(&self) -> PublicKey;
    pub fn id(&self) -> WalletId;
    pub fn sign(&self, msg: &[u8]) -> Signature;
    pub fn derive_child(&self, label: &str) -> Wallet;
    pub fn issue_creation_cert(&self, child: &Wallet, label: &str, created_at_ns: i64) -> CreationCert;
}

impl PublicKey {
    pub fn id(&self) -> WalletId;
}

impl CreationCert {
    pub fn verify(&self) -> Result<(), VerifyError>;
}

pub fn verify(pubkey: &PublicKey, msg: &[u8], sig: &Signature) -> Result<(), VerifyError>;

// Native-only (not on wasm32)
pub fn load_or_mint_seed(path: &Path) -> Result<[u8; 32], SeedError>;
```

## How `derive_child` works

```text
child_seed[0..16]  = XXH3-128( parent.seed() || label.as_bytes() )
child_seed[16..32] = XXH3-128( parent.seed() || label.as_bytes() || "/expand" )
child = Wallet::from_seed(&child_seed)
```

Two `auki-hash` outputs concatenated. No HKDF dep; reproducible bytes-for-bytes by any consumer using `auki-hash`. The `/expand` suffix is fixed; verifiers must reproduce the exact bytes.

## How `CreationCert` signing works

The signed bytes are:

```text
"auki.creation-cert.v1\n"  +
parent_pubkey  (32 bytes)  +
child_pubkey   (32 bytes)  +
label_len_le32 (4 bytes)   +
label_utf8     (label.len() bytes) +
created_at_ns_le64 (8 bytes)
```

Verifiers reproduce the same bytes locally and check the ed25519 signature against the parent pubkey. Cross-language: same recipe, same bytes, same ed25519 verification.

## Tests (25 total — 16 wallet + 9 seed)

### Wallet (`lib.rs`)

| Test | Asserts |
|------|---------|
| `new_generates_unique_wallets` | Fresh `Wallet::new()` calls produce distinct keys |
| `from_seed_is_deterministic` | Same seed → same pubkey + same seed bytes |
| `seed_round_trip` | `from_seed(w.seed()) == w` (modulo wrapper) |
| `sign_verify_round_trip` | Verify accepts the signer's own signature |
| `verify_rejects_tampered_message` | Tampering invalidates signature |
| `verify_rejects_wrong_pubkey` | Other wallet's pubkey doesn't verify |
| `wallet_id_is_stable_for_same_pubkey` | `WalletId` is deterministic + 32 chars |
| `wallet_id_differs_across_wallets` | Different wallets → different ids |
| `derive_child_is_deterministic` | Same parent + same label → same child |
| `derive_child_differs_across_labels` | Different labels → different children |
| `derive_child_differs_across_parents` | Different parents → different children |
| `creation_cert_verifies` | Issued cert verifies cleanly |
| `creation_cert_rejects_tampered_label` | Mutating `label` after signing fails verification |
| `creation_cert_rejects_swapped_child` | Mutating `child_pubkey` fails verification |
| `creation_cert_rejects_swapped_parent` | Mutating `parent_pubkey` fails verification |
| `creation_cert_serializes_via_serde_json` | Round-trips through JSON; verifies after round-trip |

### Seed persistence (`seed.rs`)

| Test | Asserts |
|------|---------|
| `missing_path_mints_persists_and_returns` | First call mints, persists 32 bytes, returns them |
| `second_call_returns_same_seed` | Idempotent — call twice, same bytes back |
| `existing_32_bytes_round_trips` | Pre-existing 32-byte file is read verbatim |
| `existing_wrong_length_is_rejected` | 0/1/31/33/64-byte files all return `InvalidLength(n)` |
| `parent_directory_is_created` | Multi-level missing parent directories are created |
| `no_tmp_file_left_behind_after_mint` | Atomic-write `.tmp` sidecar is consumed by rename |
| `minted_seed_has_some_entropy` | Fresh seed isn't all zeros; two distinct mints differ |
| `minted_file_has_mode_0600` | Unix-only; persisted file has owner-r/w-only mode |
| `minted_seed_drives_a_deterministic_wallet` | `Wallet::from_seed(load_or_mint_seed(p))` is stable across calls |

## Dependencies

- `ed25519-dalek` (2.x) — ed25519 implementation. WASM-friendly; pinned with the `rand_core` feature for `SigningKey::generate`.
- `auki-hash` — `WalletId` derivation + `derive_child` seed expansion.
- `serde` + `serde_bytes` — serialization for the public types.
- `rand_core` — the OS RNG (`OsRng`) that feeds `Wallet::new()` and `load_or_mint_seed`.
- `tempfile` (dev-only) — test isolation for seed-persistence tests.

## Consumers in this workspace

- *(planned)* `auki-network` — uses `derive_child("peer/v1")` to produce libp2p peer keys.
- *(planned)* the `ansuz` networking-demo daemon — uses `load_or_mint_seed` so its `peer_id` stays stable across restarts (required for clusters that pin peer ids in `cluster.json`).
- *(planned, downstream)* Console — uses the wallet primitive directly (compiled to WASM) for the in-browser keystore. `load_or_mint_seed` is the one piece of the API not available in the browser build.
