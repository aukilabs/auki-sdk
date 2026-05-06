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
    pub fn sign_canonical_json(&self, value: &serde_json::Value) -> (Vec<u8>, Signature);
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

## How `sign_canonical_json` works

JCS-canonicalize the input `serde_json::Value` via `auki-jcs::canonicalize` (RFC 8785 — keys sorted by ASCII order, no whitespace, deterministic number serialisation), sign the canonical bytes with this wallet's ed25519 key, return both:

```text
canonical_bytes = auki_jcs::canonicalize(value)   // RFC 8785
signature       = ed25519_sign(this_wallet.seed, canonical_bytes)
return (canonical_bytes, signature)
```

The verifier side reproduces the canonical bytes locally with the same `auki-jcs` canonicaliser and verifies the signature against them. Both sides using one canonicaliser means there's no risk of "the bytes I signed aren't the bytes you verified."

Used by [Vinland](https://www.notion.so/3585c8e9659280699681caec256e0616)'s signed registration to Discovery: the daemon builds a registration JSON minus the `signature` field, calls this method, embeds the resulting signature in the JSON under `signature`, and POSTs to `/clusters/:cluster_name/peers`. Discovery strips `signature`, canonicalises the rest, and verifies under the public key embedded in the payload. Replay protection: `cluster_name` and `timestamp_ns` are inside the canonical bytes, so a registration captured for cluster A cannot be replayed against cluster B, and Discovery rejects timestamps outside ±60s.

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

## Tests (31 total — 22 wallet + 9 seed)

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
| `locked_derive_child_peer_v1_pubkey_vector` | Cross-language vector pinning seed `[3u8;32]` + label `"peer/v1"` → fixed 32-byte pubkey |
| `derive_child_differs_across_labels` | Different labels → different children |
| `derive_child_differs_across_parents` | Different parents → different children |
| `creation_cert_verifies` | Issued cert verifies cleanly |
| `creation_cert_rejects_tampered_label` | Mutating `label` after signing fails verification |
| `creation_cert_rejects_swapped_child` | Mutating `child_pubkey` fails verification |
| `creation_cert_rejects_swapped_parent` | Mutating `parent_pubkey` fails verification |
| `creation_cert_serializes_via_serde_json` | Round-trips through JSON; verifies after round-trip |
| `sign_canonical_json_round_trips` | Verify accepts the signature against the returned canonical bytes |
| `sign_canonical_json_is_deterministic_for_same_input` | Same input value + same wallet → identical canonical bytes + signature |
| `sign_canonical_json_normalises_key_order` | Two values differing only in object-key order produce identical canonical bytes + signature (JCS sorting) |
| `sign_canonical_json_verifier_rejects_tampered_field` | Mutating a field after signing makes verification against the new canonical bytes fail |
| `locked_sign_canonical_json_vector` | Cross-language vector pinning `Wallet::from_seed([3u8;32]).sign_canonical_json(<vinland-shaped registration JSON>)` → exact RFC 8785 canonical bytes + 64-byte ed25519 signature |

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
- `auki-jcs` — RFC 8785 JSON canonicalization for `sign_canonical_json`. Pure Rust, WASM-clean.
- `serde` + `serde_bytes` — serialization for the public types.
- `serde_json` — `serde_json::Value` appears in the public signature of `sign_canonical_json`.
- `rand_core` — the OS RNG (`OsRng`) that feeds `Wallet::new()` and `load_or_mint_seed`.
- `tempfile` (dev-only) — test isolation for seed-persistence tests.

## Consumers in this workspace

- `auki-network` — uses `derive_child("peer/v1")` to produce libp2p peer keys; planned `discovery_client` uses `sign_canonical_json` for Vinland's signed registration to Discovery.
- `auki-network-py` — re-exports the wallet primitive into Python via PyO3.
- `ansuz` / `vinland` networking-demo daemons (Boosterapp, Sentinel, Park) — use `load_or_mint_seed` so each daemon's `peer_id` stays stable across restarts.
- *(planned, downstream)* Console — uses the wallet primitive directly (compiled to WASM) for the in-browser keystore. `load_or_mint_seed` is the one piece of the API not available in the browser build.
