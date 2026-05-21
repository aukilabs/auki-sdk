//! Wallet primitive for the Auki SDK.
//!
//! A `Wallet` is an ed25519 keypair with two flavours of derivation built on top:
//!
//! 1. **Deterministic child derivation** ([`Wallet::derive_child`]). Child key is
//!    derived from the parent's seed + a label. Same parent + same label → same
//!    child every time. Used when the child key needs to be regenerable from
//!    backup (e.g. a peer key derived from a wallet so you don't have to back
//!    up both).
//!
//! 2. **Signed creation cert** ([`Wallet::issue_creation_cert`]). Child has its
//!    own *independent* keypair; the parent signs a cert binding parent pubkey,
//!    child pubkey, and a label. Used when the parent vouches for an unrelated
//!    wallet (e.g. a developer wallet vouching for an app wallet generated on
//!    first run; a domain owner vouching for a sub-wallet).
//!
//! Both have legitimate uses; they're not interchangeable.
//!
//! ## Persistent identity
//!
//! [`load_or_mint_seed`] is a small filesystem helper for daemons that need
//! a stable [`Wallet`] (and therefore a stable libp2p peer id) across
//! process restarts. It loads 32 bytes from a path if it exists, otherwise
//! mints fresh random bytes and persists them atomically with mode `0o600`.
//! Native-only (gated on `not(target_arch = "wasm32")`) — the rest of the
//! crate stays WASM-friendly for in-browser use.
//!
//! ## WASM
//!
//! Core wallet primitives use no `std::fs` and no platform syscalls.
//! `getrandom` is the only randomness source — works in browser via `js-sys`
//! feature when downstream consumers enable it. Suitable for in-browser
//! wallet management (Console). The [`load_or_mint_seed`] helper is the one
//! exception and is excluded from WASM builds.
//!
//! ## What this crate is *not*
//!
//! - Not a key store. Encryption-at-rest, OS keychain integration, and
//!   passphrase-protected exports are downstream consumer concerns.
//! - Not a network identity. The peer identity used for libp2p connections
//!   is *derived* from a wallet (via `derive_child`), not the wallet itself.
//!   Lives in the planned `auki-network` crate.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

// UniFFI scaffolding. The `uniffi::Object` proc-macro emits code that
// references `crate::UniFfiTag`; that type only exists where
// `setup_scaffolding!()` is invoked. Without this, building this crate
// with `--features swift-bindings` fails before the binding crate ever
// pulls it in. The setup is gated so default builds stay scaffolding-free.
#[cfg(feature = "swift-bindings")]
uniffi::setup_scaffolding!();

#[cfg(not(target_arch = "wasm32"))]
mod seed;
#[cfg(not(target_arch = "wasm32"))]
pub use seed::{SeedError, load_or_mint_seed};

/// Errors that can occur constructing an identity primitive across the
/// FFI boundary. The type is available in all builds (so the new
/// `Wallet::from_seed` signature is buildable everywhere); the
/// `uniffi::Error` / `thiserror::Error` derives only attach in
/// `swift-bindings` builds, where the FFI surface uses them.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("seed must be 32 bytes, got {actual}")]
    InvalidSeedLength { actual: u32 },
}

/// Default-build mirror of [`IdentityError`] — same variants and `Display`
/// impl, but without the uniffi/thiserror derives (those are
/// feature-gated deps). Hand-rolled `Display` keeps the message identical
/// across builds so error logs don't drift.
#[cfg(not(feature = "swift-bindings"))]
#[derive(Debug)]
pub enum IdentityError {
    InvalidSeedLength { actual: u32 },
}

#[cfg(not(feature = "swift-bindings"))]
impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::InvalidSeedLength { actual } => {
                write!(f, "seed must be 32 bytes, got {actual}")
            }
        }
    }
}

#[cfg(not(feature = "swift-bindings"))]
impl std::error::Error for IdentityError {}

// ─── Public types ────────────────────────────────────────────────────────────

/// An ed25519 keypair plus identity helpers. Holds the secret key — treat
/// instances as sensitive material.
#[derive(Clone)]
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
pub struct Wallet {
    signing_key: SigningKey,
}

// Hand-rolled `Debug` that elides the secret key bytes — used by the
// `swift-bindings` regression tests' `panic!("…{other:?}")` formatter via
// `Result<Arc<Wallet>, IdentityError>`. `SigningKey` doesn't implement
// `Debug`, and we wouldn't want it to: dumping secret-key bytes into a
// panic message would be a hazard. The placeholder mirrors how `auki-py`
// formats sensitive seed wrappers downstream.
impl std::fmt::Debug for Wallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallet")
            .field("signing_key", &"<redacted>")
            .finish()
    }
}

/// Public half of a wallet's keypair. 32 bytes, ed25519.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicKey(#[serde(with = "serde_bytes_array")] pub [u8; 32]);

/// Content-addressed identity for a wallet — `auki-hash`'s 32-character lower-
/// case hex of the public key bytes. Stable for a given pubkey; safe to
/// publish; the canonical short form an operator uses to refer to a wallet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WalletId(pub String);

/// Output of [`Wallet::sign`]. 64 bytes, ed25519.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature(#[serde(with = "serde_bytes_array_64")] pub [u8; 64]);

/// Signed assertion that `parent` created / vouches-for `child`. The parent
/// signs over `(parent_pubkey, child_pubkey, label, created_at_ns)`. Verifiers
/// check the signature against `parent_pubkey`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreationCert {
    pub parent_pubkey: PublicKey,
    pub child_pubkey: PublicKey,
    pub label: String,
    pub created_at_ns: i64,
    pub signature: Signature,
}

// ─── Wallet impl ─────────────────────────────────────────────────────────────

// ─── Wallet impl — UniFFI-exposed surface ────────────────────────────────────
//
// Split from the rest of the impl below so the `swift-bindings` feature can
// hang `#[uniffi::export]` on just the methods the Swift binding consumes.
// Default builds compile the macros to nothing; semantics are unchanged for
// upstream Rust callers (modulo the deliberate Vec<u8>/Result/Arc reshape of
// `from_seed` and `seed` — UniFFI 0.31 doesn't implement Lower/Lift for
// `[u8; 32]`, so the FFI types must be runtime-sized).

#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl Wallet {
    /// Generate a fresh wallet with a cryptographically random ed25519 keypair.
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn new() -> std::sync::Arc<Self> {
        let mut csprng = rand_core::OsRng;
        std::sync::Arc::new(Self {
            signing_key: SigningKey::generate(&mut csprng),
        })
    }

    /// Construct a wallet from a 32-byte seed (the ed25519 secret key bytes).
    /// Same seed → same wallet, deterministically.
    ///
    /// Returns [`IdentityError::InvalidSeedLength`] if `seed` is not exactly
    /// 32 bytes. The runtime-sized `Vec<u8>` parameter is a UniFFI 0.31
    /// constraint (no `Lift` for `[u8; 32]`); upstream Rust callers can pass
    /// `array.to_vec()`.
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn from_seed(seed: Vec<u8>) -> Result<std::sync::Arc<Self>, IdentityError> {
        let array: [u8; 32] = seed.as_slice().try_into().map_err(|_| {
            IdentityError::InvalidSeedLength {
                actual: seed.len() as u32,
            }
        })?;
        Ok(std::sync::Arc::new(Self {
            signing_key: SigningKey::from_bytes(&array),
        }))
    }

    /// 32-byte seed (the ed25519 secret key bytes). Treat as sensitive — anyone
    /// holding these bytes can sign as this wallet.
    ///
    /// Returns a `Vec<u8>` (runtime-sized) rather than `[u8; 32]` because
    /// UniFFI 0.31 doesn't implement `Lower` for fixed-size arrays. The
    /// length is structurally 32; callers that need an array can
    /// `seed.as_slice().try_into().expect("32-byte seed")`.
    pub fn seed(&self) -> Vec<u8> {
        self.signing_key.to_bytes().to_vec()
    }

    /// String form of [`Self::id`] for FFI callers — UniFFI 0.31 doesn't
    /// derive `Record` on the wrapper `WalletId(String)`, so we expose the
    /// inner string directly. Upstream Rust callers continue to use
    /// [`Self::id`].
    pub fn wallet_id_str(&self) -> String {
        self.id().0
    }
}

// ─── Wallet impl — Rust-only surface ─────────────────────────────────────────
//
// Methods below stay un-annotated. Either they're not consumed by the Swift
// binding at v0, or their signatures use types UniFFI 0.31 can't lower
// (e.g. `&serde_json::Value`, `[u8; N]` slices). Upstream Rust callers
// continue to use them exactly as before.

#[cfg(not(any()))] // always compiled; placeholder so future cfg gating is local
impl Wallet {
    /// Public half of the wallet's keypair.
    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.signing_key.verifying_key().to_bytes())
    }

    /// Content-addressed identity. Stable for a given pubkey.
    pub fn id(&self) -> WalletId {
        public_key_id(&self.public_key())
    }

    /// Sign `msg` with this wallet's private key.
    pub fn sign(&self, msg: &[u8]) -> Signature {
        Signature(self.signing_key.sign(msg).to_bytes())
    }

    /// JCS-canonicalize `value` (RFC 8785) and sign the canonical bytes with
    /// this wallet's private key. Returns `(canonical_bytes, signature)` so
    /// callers can ship the signature on the wire AND inspect or log the
    /// bytes that got signed.
    ///
    /// JCS makes the signing reproducible across processes and languages:
    /// same `serde_json::Value` → same bytes → same signature (modulo the
    /// wallet). Verifiers reproduce the canonical bytes locally with
    /// `auki_jcs::canonicalize` and verify the signature against them — the
    /// two sides cannot drift because both use the same canonicaliser.
    ///
    /// Used by Vinland's signed registration to Discovery: the daemon builds
    /// a registration JSON minus the `signature` field, calls this method,
    /// adds the resulting signature to the JSON, and POSTs. Discovery
    /// reproduces the canonical bytes from the JSON-minus-signature it
    /// receives, looks up the public key embedded in the payload, and
    /// verifies.
    pub fn sign_canonical_json(&self, value: &serde_json::Value) -> (Vec<u8>, Signature) {
        let canonical_bytes = auki_jcs::canonicalize(value);
        let signature = self.sign(&canonical_bytes);
        (canonical_bytes, signature)
    }

    /// **Deterministic** child derivation. Child seed is `XXH3-128(seed || label)`
    /// expanded to 32 bytes by re-hashing — small, reproducible, no external
    /// HKDF dependency. The child has no creation cert; the relationship is
    /// implicit (anyone who knows the parent seed can re-derive the child).
    ///
    /// Use for keys that need to be regenerable from the wallet's seed
    /// (e.g. a peer key for libp2p — back up the wallet, re-derive the peer
    /// key on demand). Use [`Self::issue_creation_cert`] instead when the
    /// child must be a *separate, independent* keypair.
    pub fn derive_child(&self, label: &str) -> Wallet {
        let mut buf = Vec::with_capacity(32 + label.len());
        buf.extend_from_slice(&self.seed());
        buf.extend_from_slice(label.as_bytes());
        // XXH3-128 returns 16 bytes (32-char hex); we need 32 bytes for an
        // ed25519 seed. Re-hash with a "/2" suffix for the second half.
        let first_half_hex = auki_hash::hash_jcs_bytes(&buf);
        buf.extend_from_slice(b"/expand");
        let second_half_hex = auki_hash::hash_jcs_bytes(&buf);

        let mut child_seed = [0u8; 32];
        // Each hex char is 4 bits; 32 chars = 16 bytes per hash output.
        decode_hex(&first_half_hex, &mut child_seed[..16]).expect("auki-hash returns valid hex");
        decode_hex(&second_half_hex, &mut child_seed[16..]).expect("auki-hash returns valid hex");
        // Inlined Wallet construction — the UniFFI-exposed `from_seed` now
        // returns `Result<Arc<Self>, _>` (runtime-sized `Vec<u8>` for FFI),
        // but inside the crate we already have a `[u8; 32]` and want a plain
        // owned `Wallet`. Routing through the public constructor would force
        // a Vec round-trip and `Arc::into_inner` for no gain.
        Wallet {
            signing_key: SigningKey::from_bytes(&child_seed),
        }
    }

    /// Issue a signed creation cert vouching that `child` was created by /
    /// is endorsed by this wallet. `child`'s keypair is *independent* of this
    /// one (typically generated fresh; not derived). The cert binds them
    /// cryptographically without giving up the child's private key.
    ///
    /// `label` is operator-meaningful (e.g. `"app:boosterapp"`,
    /// `"domain:steve-warehouse"`).
    /// `created_at_ns` is wall-clock UTC nanoseconds.
    pub fn issue_creation_cert(
        &self,
        child: &Wallet,
        label: &str,
        created_at_ns: i64,
    ) -> CreationCert {
        let parent_pubkey = self.public_key();
        let child_pubkey = child.public_key();
        let msg = creation_cert_signing_bytes(&parent_pubkey, &child_pubkey, label, created_at_ns);
        let signature = self.sign(&msg);
        CreationCert {
            parent_pubkey,
            child_pubkey,
            label: label.to_string(),
            created_at_ns,
            signature,
        }
    }
}

impl Default for Wallet {
    fn default() -> Self {
        // `Wallet::new()` now returns `Arc<Self>` so its UniFFI constructor
        // signature lines up with `uniffi::Object`. Inside the crate we
        // already hold the only reference; unwrap is infallible.
        std::sync::Arc::into_inner(Self::new())
            .expect("freshly constructed Arc<Wallet> has refcount 1")
    }
}

// PublicKey impl
impl PublicKey {
    /// 32-character lowercase hex of the public key bytes via `auki-hash`'s
    /// content-addressing convention.
    pub fn id(&self) -> WalletId {
        public_key_id(self)
    }
}

// CreationCert impl
impl CreationCert {
    /// Verify the signature is valid for the bound child pubkey under the
    /// claimed parent pubkey. Returns `Ok(())` on valid; `Err` describes why
    /// invalid.
    pub fn verify(&self) -> Result<(), VerifyError> {
        let msg = creation_cert_signing_bytes(
            &self.parent_pubkey,
            &self.child_pubkey,
            &self.label,
            self.created_at_ns,
        );
        verify(&self.parent_pubkey, &msg, &self.signature)
    }
}

// ─── Free functions ──────────────────────────────────────────────────────────

/// Verify a signature.
pub fn verify(pubkey: &PublicKey, msg: &[u8], sig: &Signature) -> Result<(), VerifyError> {
    let vk = VerifyingKey::from_bytes(&pubkey.0).map_err(|_| VerifyError::BadPublicKey)?;
    let s = ed25519_dalek::Signature::from_bytes(&sig.0);
    vk.verify(msg, &s)
        .map_err(|_| VerifyError::SignatureMismatch)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    BadPublicKey,
    SignatureMismatch,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::BadPublicKey => write!(f, "public key is not a valid ed25519 point"),
            VerifyError::SignatureMismatch => write!(f, "signature does not verify"),
        }
    }
}

impl std::error::Error for VerifyError {}

// ─── Internals ───────────────────────────────────────────────────────────────

fn public_key_id(pubkey: &PublicKey) -> WalletId {
    WalletId(auki_hash::hash_jcs_bytes(&pubkey.0))
}

/// Bytes that get signed when issuing a `CreationCert`. Format is fixed and
/// must stay stable; verifiers reproduce it locally.
fn creation_cert_signing_bytes(
    parent: &PublicKey,
    child: &PublicKey,
    label: &str,
    created_at_ns: i64,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + 32 + label.len() + 8 + 32);
    out.extend_from_slice(b"auki.creation-cert.v1\n");
    out.extend_from_slice(&parent.0);
    out.extend_from_slice(&child.0);
    out.extend_from_slice(&(label.len() as u32).to_le_bytes());
    out.extend_from_slice(label.as_bytes());
    out.extend_from_slice(&created_at_ns.to_le_bytes());
    out
}

fn decode_hex(s: &str, out: &mut [u8]) -> Result<(), ()> {
    if s.len() != out.len() * 2 {
        return Err(());
    }
    for (i, byte_str) in s.as_bytes().chunks(2).enumerate() {
        let high = hex_digit(byte_str[0])?;
        let low = hex_digit(byte_str[1])?;
        out[i] = (high << 4) | low;
    }
    Ok(())
}

fn hex_digit(c: u8) -> Result<u8, ()> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        _ => Err(()),
    }
}

// ─── Serde helpers for fixed-size byte arrays ────────────────────────────────

mod serde_bytes_array {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        serde_bytes::Bytes::new(bytes).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let v = <serde_bytes::ByteBuf>::deserialize(d)?;
        let bytes = v.into_vec();
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }

    use serde::Serialize;
}

mod serde_bytes_array_64 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        serde_bytes::Bytes::new(bytes).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let v = <serde_bytes::ByteBuf>::deserialize(d)?;
        let bytes = v.into_vec();
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 64 bytes"))
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_generates_unique_wallets() {
        let a = Wallet::new();
        let b = Wallet::new();
        assert_ne!(a.public_key(), b.public_key());
    }

    #[test]
    fn from_seed_is_deterministic() {
        let seed = vec![42u8; 32];
        let a = Wallet::from_seed(seed.clone()).expect("32-byte seed");
        let b = Wallet::from_seed(seed).expect("32-byte seed");
        assert_eq!(a.public_key(), b.public_key());
        assert_eq!(a.seed(), b.seed());
    }

    #[test]
    fn seed_round_trip() {
        let original = Wallet::new();
        let seed = original.seed();
        let restored = Wallet::from_seed(seed).expect("32-byte seed");
        assert_eq!(original.public_key(), restored.public_key());
    }

    #[test]
    fn sign_verify_round_trip() {
        let w = Wallet::new();
        let msg = b"hello, auki";
        let sig = w.sign(msg);
        assert!(verify(&w.public_key(), msg, &sig).is_ok());
    }

    #[test]
    fn verify_rejects_tampered_message() {
        let w = Wallet::new();
        let sig = w.sign(b"the original message");
        let result = verify(&w.public_key(), b"the tampered message", &sig);
        assert_eq!(result, Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn verify_rejects_wrong_pubkey() {
        let signer = Wallet::new();
        let other = Wallet::new();
        let sig = signer.sign(b"msg");
        let result = verify(&other.public_key(), b"msg", &sig);
        assert_eq!(result, Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn wallet_id_is_stable_for_same_pubkey() {
        let w = Wallet::from_seed(vec![7u8; 32]).expect("32-byte seed");
        let id1 = w.id();
        let id2 = w.id();
        assert_eq!(id1, id2);
        assert_eq!(id1.0.len(), 32); // 32-char hex
    }

    #[test]
    fn wallet_id_differs_across_wallets() {
        let a = Wallet::from_seed(vec![1u8; 32]).expect("32-byte seed");
        let b = Wallet::from_seed(vec![2u8; 32]).expect("32-byte seed");
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn derive_child_is_deterministic() {
        let parent = Wallet::from_seed(vec![3u8; 32]).expect("32-byte seed");
        let c1 = parent.derive_child("peer/v1");
        let c2 = parent.derive_child("peer/v1");
        assert_eq!(c1.public_key(), c2.public_key());
    }

    /// Locked cross-language conformance vector: `Wallet::from_seed([3u8; 32])
    /// .derive_child("peer/v1").public_key()` MUST produce the 32-byte ed25519
    /// pubkey below. The locked chain: seed → ed25519 keypair → labeled-hash
    /// child seed → child ed25519 keypair → child pubkey bytes. Any reimpl in
    /// another language is correct only if it reproduces these exact bytes from
    /// the same seed + label. Pairs with `auki_network::tests::locked_seed_to_peer_id_vector`
    /// — the parent wallet's PeerId there is derived from this same `[3u8; 32]`
    /// seed via the libp2p PeerId encoding. Don't update this without a
    /// coordinated version bump.
    #[test]
    fn locked_derive_child_peer_v1_pubkey_vector() {
        let parent = Wallet::from_seed(vec![3u8; 32]).expect("32-byte seed");
        let derived = parent.derive_child("peer/v1");
        let expected: [u8; 32] = [
            0x10, 0x80, 0x63, 0x3b, 0xcb, 0x57, 0xba, 0xc0, 0x66, 0xcf, 0x84, 0x46, 0xe2, 0xb7,
            0xae, 0x71, 0x15, 0x71, 0xcb, 0x04, 0xbe, 0x0b, 0x46, 0xbd, 0xaf, 0x03, 0x14, 0x63,
            0x17, 0xbf, 0xe7, 0x07,
        ];
        assert_eq!(
            derived.public_key().0,
            expected,
            "derive_child(\"peer/v1\") drifted — see crate docs for the locked recipe"
        );
    }

    #[test]
    fn derive_child_differs_across_labels() {
        let parent = Wallet::from_seed(vec![4u8; 32]).expect("32-byte seed");
        let peer = parent.derive_child("peer/v1");
        let app = parent.derive_child("app/boosterapp");
        assert_ne!(peer.public_key(), app.public_key());
    }

    #[test]
    fn derive_child_differs_across_parents() {
        let p1 = Wallet::from_seed(vec![5u8; 32]).expect("32-byte seed");
        let p2 = Wallet::from_seed(vec![6u8; 32]).expect("32-byte seed");
        let c1 = p1.derive_child("peer/v1");
        let c2 = p2.derive_child("peer/v1");
        assert_ne!(c1.public_key(), c2.public_key());
    }

    #[test]
    fn creation_cert_verifies() {
        let parent = Wallet::new();
        let child = Wallet::new();
        let cert = parent.issue_creation_cert(&child, "app:boosterapp", 1_745_000_000_000_000_000);
        assert!(cert.verify().is_ok());
    }

    #[test]
    fn creation_cert_rejects_tampered_label() {
        let parent = Wallet::new();
        let child = Wallet::new();
        let mut cert = parent.issue_creation_cert(&child, "app:boosterapp", 1);
        cert.label = "app:malicious".into();
        assert_eq!(cert.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn creation_cert_rejects_swapped_child() {
        let parent = Wallet::new();
        let child_a = Wallet::new();
        let child_b = Wallet::new();
        let mut cert = parent.issue_creation_cert(&child_a, "app:x", 1);
        cert.child_pubkey = child_b.public_key();
        assert_eq!(cert.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn creation_cert_rejects_swapped_parent() {
        let parent = Wallet::new();
        let other_parent = Wallet::new();
        let child = Wallet::new();
        let mut cert = parent.issue_creation_cert(&child, "app:x", 1);
        cert.parent_pubkey = other_parent.public_key();
        assert_eq!(cert.verify(), Err(VerifyError::SignatureMismatch));
    }

    #[test]
    fn creation_cert_serializes_via_serde_json() {
        let parent = Wallet::new();
        let child = Wallet::new();
        let cert = parent.issue_creation_cert(&child, "app:test", 42);
        let json = serde_json::to_string(&cert).unwrap();
        let back: CreationCert = serde_json::from_str(&json).unwrap();
        assert_eq!(cert, back);
        assert!(back.verify().is_ok());
    }

    // ─── sign_canonical_json ─────────────────────────────────────────────

    #[test]
    fn sign_canonical_json_round_trips() {
        let wallet = Wallet::new();
        let value = serde_json::json!({
            "cluster_name": "vinland",
            "peer_id": "12D3KooW...",
            "addresses": ["/ip4/127.0.0.1/tcp/4001"],
        });
        let (canonical_bytes, signature) = wallet.sign_canonical_json(&value);
        assert!(verify(&wallet.public_key(), &canonical_bytes, &signature).is_ok());
    }

    #[test]
    fn sign_canonical_json_is_deterministic_for_same_input() {
        let wallet = Wallet::from_seed(vec![7u8; 32]).expect("32-byte seed");
        let value = serde_json::json!({"a": 1, "b": [2, 3]});
        let (bytes1, sig1) = wallet.sign_canonical_json(&value);
        let (bytes2, sig2) = wallet.sign_canonical_json(&value);
        assert_eq!(bytes1, bytes2);
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn sign_canonical_json_normalises_key_order() {
        // JCS sorts object keys; two values that differ only in key order
        // must produce identical canonical bytes (and therefore identical
        // signatures). This is what makes the verifier-side check work
        // independent of how either side serialised the JSON originally.
        let wallet = Wallet::from_seed(vec![8u8; 32]).expect("32-byte seed");
        let v1 = serde_json::json!({"a": 1, "b": 2, "c": 3});
        let v2 = serde_json::json!({"c": 3, "a": 1, "b": 2});
        let (bytes1, sig1) = wallet.sign_canonical_json(&v1);
        let (bytes2, sig2) = wallet.sign_canonical_json(&v2);
        assert_eq!(bytes1, bytes2);
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn sign_canonical_json_verifier_rejects_tampered_field() {
        let wallet = Wallet::from_seed(vec![9u8; 32]).expect("32-byte seed");
        let original = serde_json::json!({"cluster_name": "vinland", "n": 1});
        let tampered = serde_json::json!({"cluster_name": "vinland", "n": 2});
        let (_orig_bytes, signature) = wallet.sign_canonical_json(&original);
        let tampered_bytes = auki_jcs::canonicalize(&tampered);
        assert_eq!(
            verify(&wallet.public_key(), &tampered_bytes, &signature),
            Err(VerifyError::SignatureMismatch)
        );
    }

    /// Locked cross-language conformance vector for `Wallet::sign_canonical_json`.
    ///
    /// Pins the chain `seed → wallet → JCS canonical bytes of fixed JSON →
    /// 64-byte ed25519 signature` to exact bytes. Any reimplementation in
    /// another language is correct only if it reproduces these exact
    /// canonical bytes AND signature from the same seed + JSON value. Drift
    /// in JCS, in ed25519, or in the seed-to-signing-key path will surface
    /// here. Pairs with the existing `auki-hash` / `auki-identity` /
    /// `auki-network` locked vectors as the cross-language conformance set
    /// downstream Vinland verifiers (Discovery, Python sidecar) will pin
    /// against. Don't update without a coordinated version bump.
    ///
    /// Source value uses a Vinland-shaped registration body in deliberately
    /// non-sorted insertion order so the assertion exercises JCS's
    /// key-sorting behaviour, not just byte-for-byte JSON serialisation.
    #[test]
    fn locked_sign_canonical_json_vector() {
        let wallet = Wallet::from_seed(vec![3u8; 32]).expect("32-byte seed");
        let value = serde_json::json!({
            "cluster_name": "vinland",
            "peer_id": "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar",
            "addresses": ["/ip4/127.0.0.1/tcp/4001"],
            "timestamp_ns": 1_700_000_000_000_000_000_i64,
        });
        let (canonical_bytes, signature) = wallet.sign_canonical_json(&value);

        // RFC 8785 (JCS) — keys in ASCII order, no whitespace, integers
        // without trailing zeros.
        let expected_canonical: &[u8] = br#"{"addresses":["/ip4/127.0.0.1/tcp/4001"],"cluster_name":"vinland","peer_id":"12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar","timestamp_ns":1700000000000000000}"#;
        assert_eq!(
            canonical_bytes, expected_canonical,
            "JCS canonical bytes drifted — see crate docs for the locked recipe"
        );

        let expected_signature: [u8; 64] = [
            0x6f, 0xdd, 0x19, 0x1c, 0xdd, 0xf7, 0xa2, 0xa5, 0x19, 0x23, 0xb4, 0x67, 0xf1, 0x64,
            0xde, 0x58, 0x7d, 0x12, 0x29, 0x85, 0x8c, 0x11, 0xa3, 0x3b, 0x3b, 0xeb, 0xdb, 0x54,
            0xa3, 0xed, 0x82, 0xdb, 0x1c, 0x86, 0x05, 0x34, 0xdc, 0x6a, 0xff, 0x14, 0xac, 0x7b,
            0x88, 0x04, 0xc2, 0x95, 0x10, 0x07, 0x5d, 0x6b, 0xd8, 0x5f, 0x6a, 0xaf, 0x30, 0xaa,
            0x4d, 0x47, 0xd5, 0xd8, 0x0e, 0x39, 0x22, 0x07,
        ];
        assert_eq!(
            signature.0, expected_signature,
            "ed25519 signature drifted — see crate docs for the locked recipe"
        );
    }
}

#[cfg(all(test, feature = "swift-bindings"))]
mod swift_bindings_tests {
    use super::*;

    #[test]
    fn wallet_id_str_is_deterministic_for_fixed_seed() {
        let seed: Vec<u8> = vec![7u8; 32];
        let w1 = Wallet::from_seed(seed.clone()).expect("32-byte seed");
        let w2 = Wallet::from_seed(seed).expect("32-byte seed");
        assert_eq!(w1.wallet_id_str(), w2.wallet_id_str());
    }

    #[test]
    fn wallet_new_returns_distinct_wallets() {
        let a = Wallet::new();
        let b = Wallet::new();
        assert_ne!(a.wallet_id_str(), b.wallet_id_str());
    }

    #[test]
    fn seed_round_trips() {
        let original = Wallet::new();
        let seed = original.seed();
        assert_eq!(seed.len(), 32);
        let reconstructed = Wallet::from_seed(seed).expect("32-byte seed");
        assert_eq!(original.wallet_id_str(), reconstructed.wallet_id_str());
    }

    #[test]
    fn from_seed_rejects_wrong_length() {
        let too_short: Vec<u8> = vec![0u8; 16];
        match Wallet::from_seed(too_short) {
            Err(IdentityError::InvalidSeedLength { actual }) => assert_eq!(actual, 16),
            other => panic!("expected InvalidSeedLength, got {other:?}"),
        }
    }
}
