# auki-identity

Wallet primitive for the Auki SDK — ed25519 keypairs with two derivation flavours layered on top. Foundational; many other crates and downstream apps depend on it.

## What a wallet is

A `Wallet` is an ed25519 keypair plus identity helpers. It has:

- A **public key** (32 bytes) — safe to publish.
- A **secret key** (32 bytes, "seed") — never published; treat as sensitive material.
- A **`WalletId`** — `auki-hash`'s 32-character lowercase hex of the public key. Content-addressed; stable for a given pubkey; the canonical short form an operator uses to refer to a wallet.

Per the broader Auki architecture, every node has *two* identities: a **wallet** (economic / policy / ownership) and a **peer** (network / dialability). Both are wallets in the sense of "ed25519 keypair," but they're rooted differently. The peer identity is *derived* from the principal wallet (`derive_child("peer/v1")`) so that a backup of the wallet seed lets you regenerate the peer key.

## Two derivation flavours

These are not interchangeable. Pick based on what relationship you're modelling.

### Deterministic child derivation — `Wallet::derive_child(label)`

Child seed is `XXH3-128(parent_seed || label)`, expanded to 32 bytes by re-hashing with a `/expand` suffix. Same parent + same label → same child every time. The relationship is *implicit*: anyone who knows the parent's seed can re-derive the child, but there's no signature linking them.

Use when the child key needs to be **regenerable from the wallet's seed** — so backing up the wallet alone is sufficient. Examples:

- Wallet → libp2p peer key (`derive_child("peer/v1")`). One backup, regenerate peer key on demand.
- Wallet → a signing key for a specific protocol plane.

### Signed creation cert — `Wallet::issue_creation_cert(child, label, ts)`

Child has its own *independent* keypair (typically generated fresh, not derived). The parent signs a cert binding `(parent_pubkey, child_pubkey, label, created_at_ns)`. Verifiers walk up the chain by checking signatures against parent pubkeys.

Use when the child must be **a separate, independent keypair** that the parent vouches for. Examples:

- Developer wallet vouches for an app wallet (the app generates its own random key on first run; developer signs a cert; verifiers trust the chain).
- Domain owner vouches for a sub-wallet that operates the domain's compute infrastructure.
- Capability/delegation tokens — same shape, different label.

## API

```rust
// Generate / load
pub fn Wallet::new() -> Wallet                 // fresh random
pub fn Wallet::from_seed(&[u8; 32]) -> Wallet  // deterministic from seed
pub fn wallet.seed() -> [u8; 32]               // backup material; sensitive

// Identity
pub fn wallet.public_key() -> PublicKey        // 32 bytes
pub fn wallet.id() -> WalletId                 // 32-char hex

// Sign / verify
pub fn wallet.sign(msg: &[u8]) -> Signature
pub fn verify(pubkey: &PublicKey, msg: &[u8], sig: &Signature) -> Result<(), VerifyError>

// Derivation
pub fn wallet.derive_child(label: &str) -> Wallet
pub fn wallet.issue_creation_cert(child: &Wallet, label: &str, created_at_ns: i64) -> CreationCert

// Verification
pub fn cert.verify() -> Result<(), VerifyError>

// Persistent seed (native-only; not available on wasm32)
pub fn load_or_mint_seed(path: &Path) -> Result<[u8; 32], SeedError>
```

Public types are serde-serializable. Bytes fields use `serde_bytes` so JSON and CBOR encodings stay compact.

## load_or_mint_seed

Small filesystem helper for daemons that need a stable wallet — and therefore a stable libp2p peer id — across process restarts. It is the mechanism behind the SDK's stable peer-key guarantee: `auki-network::PeerIdentity::from_wallet(&wallet)` derives the same peer id every time, and `auki-domain::ClusterManager` can safely advertise that peer id to Discovery and other cluster members across daemon restarts as long as the wallet seed is persisted somewhere.

```rust
pub fn load_or_mint_seed(path: &Path) -> Result<[u8; 32], SeedError>;

pub enum SeedError {
    Io(std::io::Error),
    InvalidLength(usize), // file existed but is not 32 bytes
}
```

Behaviour:

- **`path` exists:** read it. Return the 32 bytes if the file is exactly 32 bytes long; otherwise return `InvalidLength` carrying the actual length found. Never silently truncate, pad, or overwrite an existing file with the wrong shape.
- **`path` does not exist:** create the parent directories with `std::fs::create_dir_all`, generate 32 cryptographically-random bytes from `OsRng`, write atomically (write to `<path>.tmp`, fsync, rename onto `path`) so a crash mid-write cannot leave a partial file behind, and on Unix set the file mode to `0o600` (owner read/write only) before returning. Successful writes leave no `.tmp` sidecar in place.
- The function does not validate the seed cryptographically. Any 32 bytes are accepted on read; minted bytes come straight from the OS RNG. Validation is the wallet's job.

### Path convention

The function takes any `&Path`; the convention — *not baked into the signature* — is each app picks `~/.auki/<app>/identity.seed` so multiple Auki daemons can coexist on one host. Tests, ephemeral daemons, and alternative layouts shouldn't be locked into a hardcoded location, so the choice is the caller's.

### What it deliberately doesn't do

- **No encryption-at-rest.** The defence on disk is the file mode. Threat models that need more wrap their own keystore around this primitive — see the parking-lot item on encrypted-at-rest format.
- **No OS-keychain integration.** Same reasoning. A future `auki-keystore` (or per-platform crate) would own that.
- **No mnemonic backup.** BIP39 is parked separately; the helper hands the caller raw bytes.

### WASM

`load_or_mint_seed` is gated `#[cfg(not(target_arch = "wasm32"))]` — it touches the filesystem, which doesn't exist in browser builds. The rest of the crate (the wallet primitive itself) remains WASM-clean.

## What this crate is *not*

- **Not a key store.** Encryption-at-rest, OS keychain integration, mnemonic backup, and passphrase-protected exports are downstream consumer concerns. Console will own the browser-side keystore; an OS-keychain crate could own the desktop side. This crate hands you the bytes; what you do with them is yours.
- **Not a network identity by itself.** Peer identity is *derived* from a wallet via `derive_child("peer/v1")`. The libp2p translation (raw ed25519 → libp2p PeerId via multihash) lives in the planned `auki-network` crate, which depends on this one.
- **Not a Domain owner registry.** Whether a particular wallet is "the steve domain owner" is a Discovery Service concern (out-of-scope here). This crate just gives you keys.

## WASM compatibility

The crate is designed to compile to WASM for in-browser use (Console). The wallet primitive — keypair generation, signing, verification, child derivation, creation certs — uses no `std::fs` and no platform syscalls. Randomness comes from `getrandom`, which works in the browser via downstream `js-sys` feature when consumers enable it. `ed25519-dalek` 2.x is WASM-friendly.

The single exception is [`load_or_mint_seed`](#load_or_mint_seed), which touches the filesystem and is therefore gated `#[cfg(not(target_arch = "wasm32"))]`. WASM builds compile cleanly without it; browser consumers manage seed material via Console's in-browser keystore instead.

## Cross-language conformance

The signing scheme is plain ed25519 (RFC 8032). Anyone with an ed25519 implementation can verify signatures produced by this crate. The `WalletId` is the `auki-hash` (XXH3-128) hex of the 32-byte public key — same convention used elsewhere in the SDK.

The `CreationCert` signing format is fixed (see source for the exact bytes); it's a versioned protocol prefix (`auki.creation-cert.v1\n`) followed by parent pubkey, child pubkey, length-prefixed label, and timestamp. Cross-language verifiers reproduce these bytes locally.

## Versioning

Schema version is **1** for `CreationCert`. Future revisions extend the discriminator (`auki.creation-cert.v2\n`); old verifiers see a new version, fail signature check, and reject — same shape as other versioned wire formats in the SDK.
