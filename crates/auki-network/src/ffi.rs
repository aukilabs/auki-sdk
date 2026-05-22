use crate::core;
use auki_identity::Wallet;
use std::sync::Arc;

uniffi::setup_scaffolding!();

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("seed must be exactly 32 bytes, found {len}")]
    InvalidSeedLength { len: u64 },
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    pub value: String,
}

#[derive(uniffi::Object)]
pub struct PeerIdentity {
    inner: core::PeerIdentity,
}

#[uniffi::export]
impl PeerIdentity {
    #[uniffi::constructor]
    pub fn from_seed(seed: Vec<u8>) -> Result<Arc<Self>, NetworkError> {
        Ok(Arc::new(Self {
            inner: core::PeerIdentity::from_seed(&seed32(seed)?),
        }))
    }

    #[uniffi::constructor]
    pub fn from_wallet_seed(seed: Vec<u8>) -> Result<Arc<Self>, NetworkError> {
        let wallet = Wallet::from_seed(&seed32(seed)?);
        Ok(Arc::new(Self {
            inner: core::PeerIdentity::from_wallet(&wallet),
        }))
    }

    pub fn peer_id(&self) -> String {
        self.inner.peer_id().to_string()
    }

    pub fn public_key_protobuf(&self) -> Vec<u8> {
        self.inner.public_key().encode_protobuf()
    }
}

#[uniffi::export]
pub fn peer_derivation_label() -> String {
    core::PEER_DERIVATION_LABEL.to_string()
}

#[uniffi::export]
pub fn peer_id_from_wallet_seed(seed: Vec<u8>) -> Result<String, NetworkError> {
    let wallet = Wallet::from_seed(&seed32(seed)?);
    Ok(core::PeerIdentity::from_wallet(&wallet)
        .peer_id()
        .to_string())
}

#[uniffi::export]
pub fn networking_capabilities() -> Vec<Capability> {
    [
        core::Capability::MESSAGE_FORWARDING,
        core::Capability::BULK_DATA_CHANNEL,
        core::Capability::TURN,
        core::Capability::SFU,
    ]
    .into_iter()
    .map(|value| Capability {
        value: value.to_string(),
    })
    .collect()
}

fn seed32(seed: Vec<u8>) -> Result<[u8; 32], NetworkError> {
    let len = seed.len();
    seed.try_into()
        .map_err(|_| NetworkError::InvalidSeedLength { len: len as u64 })
}
