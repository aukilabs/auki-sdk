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
//! ## WASM
//!
//! No `std::fs`, no platform syscalls. `getrandom` is the only randomness
//! source — works in browser via `js-sys` feature when downstream consumers
//! enable it. Suitable for in-browser wallet management (Console).
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

// ─── Public types ────────────────────────────────────────────────────────────

/// An ed25519 keypair plus identity helpers. Holds the secret key — treat
/// instances as sensitive material.
#[derive(Clone)]
pub struct Wallet {
    signing_key: SigningKey,
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

impl Wallet {
    /// Generate a fresh wallet with a cryptographically random ed25519 keypair.
    pub fn new() -> Self {
        let mut csprng = rand_core::OsRng;
        Self {
            signing_key: SigningKey::generate(&mut csprng),
        }
    }

    /// Construct a wallet from a 32-byte seed (the ed25519 secret key bytes).
    /// Same seed → same wallet, deterministically.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(seed),
        }
    }

    /// 32-byte seed (the ed25519 secret key bytes). Treat as sensitive — anyone
    /// holding these bytes can sign as this wallet.
    pub fn seed(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

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

        let mut seed = [0u8; 32];
        // Each hex char is 4 bits; 32 chars = 16 bytes per hash output.
        decode_hex(&first_half_hex, &mut seed[..16]).expect("auki-hash returns valid hex");
        decode_hex(&second_half_hex, &mut seed[16..]).expect("auki-hash returns valid hex");
        Wallet::from_seed(&seed)
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
        Self::new()
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
        let seed = [42u8; 32];
        let a = Wallet::from_seed(&seed);
        let b = Wallet::from_seed(&seed);
        assert_eq!(a.public_key(), b.public_key());
        assert_eq!(a.seed(), b.seed());
    }

    #[test]
    fn seed_round_trip() {
        let original = Wallet::new();
        let seed = original.seed();
        let restored = Wallet::from_seed(&seed);
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
        let w = Wallet::from_seed(&[7u8; 32]);
        let id1 = w.id();
        let id2 = w.id();
        assert_eq!(id1, id2);
        assert_eq!(id1.0.len(), 32); // 32-char hex
    }

    #[test]
    fn wallet_id_differs_across_wallets() {
        let a = Wallet::from_seed(&[1u8; 32]);
        let b = Wallet::from_seed(&[2u8; 32]);
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn derive_child_is_deterministic() {
        let parent = Wallet::from_seed(&[3u8; 32]);
        let c1 = parent.derive_child("peer/v1");
        let c2 = parent.derive_child("peer/v1");
        assert_eq!(c1.public_key(), c2.public_key());
    }

    #[test]
    fn derive_child_differs_across_labels() {
        let parent = Wallet::from_seed(&[4u8; 32]);
        let peer = parent.derive_child("peer/v1");
        let app = parent.derive_child("app/boosterapp");
        assert_ne!(peer.public_key(), app.public_key());
    }

    #[test]
    fn derive_child_differs_across_parents() {
        let p1 = Wallet::from_seed(&[5u8; 32]);
        let p2 = Wallet::from_seed(&[6u8; 32]);
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
}
