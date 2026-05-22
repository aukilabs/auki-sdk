use crate::core;
use std::sync::Arc;

uniffi::setup_scaffolding!();

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct PublicKey {
    pub bytes: Vec<u8>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub bytes: Vec<u8>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct WalletId {
    pub value: String,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct CreationCert {
    pub parent_pubkey: PublicKey,
    pub child_pubkey: PublicKey,
    pub label: String,
    pub created_at_ns: i64,
    pub signature: Signature,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct SignedCanonicalJson {
    pub canonical_bytes: Vec<u8>,
    pub signature: Signature,
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("seed must be exactly 32 bytes, found {len}")]
    InvalidSeedLength { len: u64 },
    #[error("public key must be exactly 32 bytes, found {len}")]
    InvalidPublicKeyLength { len: u64 },
    #[error("signature must be exactly 64 bytes, found {len}")]
    InvalidSignatureLength { len: u64 },
    #[error("public key is not a valid ed25519 point")]
    BadPublicKey,
    #[error("signature does not verify")]
    SignatureMismatch,
    #[error("JSON is not valid: {message}")]
    InvalidJson { message: String },
    #[error("seed file I/O error: {message}")]
    SeedIo { message: String },
    #[error("seed file must be exactly 32 bytes, found {len}")]
    SeedInvalidLength { len: u64 },
}

#[derive(uniffi::Object)]
pub struct Wallet {
    inner: core::Wallet,
}

#[uniffi::export]
impl Wallet {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: core::Wallet::new(),
        })
    }

    #[uniffi::constructor]
    pub fn from_seed(seed: Vec<u8>) -> Result<Arc<Self>, IdentityError> {
        let seed = seed32(seed)?;
        Ok(Arc::new(Self {
            inner: core::Wallet::from_seed(&seed),
        }))
    }

    pub fn seed(&self) -> Vec<u8> {
        self.inner.seed().to_vec()
    }

    pub fn public_key(&self) -> PublicKey {
        self.inner.public_key().into()
    }

    pub fn id(&self) -> WalletId {
        self.inner.id().into()
    }

    pub fn sign(&self, msg: Vec<u8>) -> Signature {
        self.inner.sign(&msg).into()
    }

    pub fn sign_canonical_json(&self, json: String) -> Result<SignedCanonicalJson, IdentityError> {
        let value: serde_json::Value =
            serde_json::from_str(&json).map_err(|err| IdentityError::InvalidJson {
                message: err.to_string(),
            })?;
        let (canonical_bytes, signature) = self.inner.sign_canonical_json(&value);
        Ok(SignedCanonicalJson {
            canonical_bytes,
            signature: signature.into(),
        })
    }

    pub fn derive_child(&self, label: String) -> Arc<Self> {
        Arc::new(Self {
            inner: self.inner.derive_child(&label),
        })
    }

    pub fn issue_creation_cert(
        &self,
        child: Arc<Wallet>,
        label: String,
        created_at_ns: i64,
    ) -> CreationCert {
        self.inner
            .issue_creation_cert(&child.inner, &label, created_at_ns)
            .into()
    }
}

#[uniffi::export]
pub fn verify(pubkey: PublicKey, msg: Vec<u8>, sig: Signature) -> Result<(), IdentityError> {
    let pubkey = pubkey.try_into_core()?;
    let sig = sig.try_into_core()?;
    core::verify(&pubkey, &msg, &sig).map_err(Into::into)
}

#[uniffi::export]
pub fn verify_creation_cert(cert: CreationCert) -> Result<(), IdentityError> {
    cert.try_into_core()?.verify().map_err(Into::into)
}

#[uniffi::export]
pub fn load_or_mint_seed(path: String) -> Result<Vec<u8>, IdentityError> {
    core::load_or_mint_seed(std::path::Path::new(&path))
        .map(|seed| seed.to_vec())
        .map_err(Into::into)
}

fn seed32(seed: Vec<u8>) -> Result<[u8; 32], IdentityError> {
    let len = seed.len();
    seed.try_into()
        .map_err(|_| IdentityError::InvalidSeedLength { len: len as u64 })
}

fn bytes32(bytes: Vec<u8>) -> Result<[u8; 32], IdentityError> {
    let len = bytes.len();
    bytes
        .try_into()
        .map_err(|_| IdentityError::InvalidPublicKeyLength { len: len as u64 })
}

fn bytes64(bytes: Vec<u8>) -> Result<[u8; 64], IdentityError> {
    let len = bytes.len();
    bytes
        .try_into()
        .map_err(|_| IdentityError::InvalidSignatureLength { len: len as u64 })
}

impl PublicKey {
    fn try_into_core(self) -> Result<core::PublicKey, IdentityError> {
        Ok(core::PublicKey(bytes32(self.bytes)?))
    }
}

impl Signature {
    fn try_into_core(self) -> Result<core::Signature, IdentityError> {
        Ok(core::Signature(bytes64(self.bytes)?))
    }
}

impl CreationCert {
    fn try_into_core(self) -> Result<core::CreationCert, IdentityError> {
        Ok(core::CreationCert {
            parent_pubkey: self.parent_pubkey.try_into_core()?,
            child_pubkey: self.child_pubkey.try_into_core()?,
            label: self.label,
            created_at_ns: self.created_at_ns,
            signature: self.signature.try_into_core()?,
        })
    }
}

impl From<core::PublicKey> for PublicKey {
    fn from(pubkey: core::PublicKey) -> Self {
        Self {
            bytes: pubkey.0.to_vec(),
        }
    }
}

impl From<core::Signature> for Signature {
    fn from(signature: core::Signature) -> Self {
        Self {
            bytes: signature.0.to_vec(),
        }
    }
}

impl From<core::WalletId> for WalletId {
    fn from(id: core::WalletId) -> Self {
        Self { value: id.0 }
    }
}

impl From<core::CreationCert> for CreationCert {
    fn from(cert: core::CreationCert) -> Self {
        Self {
            parent_pubkey: cert.parent_pubkey.into(),
            child_pubkey: cert.child_pubkey.into(),
            label: cert.label,
            created_at_ns: cert.created_at_ns,
            signature: cert.signature.into(),
        }
    }
}

impl From<core::VerifyError> for IdentityError {
    fn from(err: core::VerifyError) -> Self {
        match err {
            core::VerifyError::BadPublicKey => Self::BadPublicKey,
            core::VerifyError::SignatureMismatch => Self::SignatureMismatch,
        }
    }
}

impl From<core::SeedError> for IdentityError {
    fn from(err: core::SeedError) -> Self {
        match err {
            core::SeedError::Io(err) => Self::SeedIo {
                message: err.to_string(),
            },
            core::SeedError::InvalidLength(len) => Self::SeedInvalidLength { len: len as u64 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn binding_wallet_sign_verify_round_trip() {
        let wallet = Wallet::from_seed(vec![7u8; 32]).unwrap();
        let msg = b"hello from bindings".to_vec();
        let signature = wallet.sign(msg.clone());
        verify(wallet.public_key(), msg, signature).unwrap();
    }

    #[test]
    fn binding_rejects_bad_seed_length() {
        match Wallet::from_seed(vec![1u8; 31]) {
            Err(IdentityError::InvalidSeedLength { len }) => assert_eq!(len, 31),
            _ => panic!("expected InvalidSeedLength"),
        }
    }

    #[test]
    fn binding_load_or_mint_seed_uses_native_seed_helper() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("identity.seed");

        let first = load_or_mint_seed(path.to_string_lossy().into_owned()).unwrap();
        let second = load_or_mint_seed(path.to_string_lossy().into_owned()).unwrap();

        assert_eq!(first.len(), 32);
        assert_eq!(first, second);
    }
}
