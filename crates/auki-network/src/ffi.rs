use crate::core;
#[cfg(feature = "message_node")]
use crate::message_node::{MessageNode, MessageNodeConfig};
use auki_identity::Wallet;
#[cfg(feature = "message_node")]
use libp2p_identity::PeerId as Libp2pPeerId;
#[cfg(feature = "message_node")]
use multiaddr::Multiaddr;
#[cfg(feature = "message_node")]
use prost::Message as _;
use std::sync::Arc;

uniffi::setup_scaffolding!();

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("seed must be exactly 32 bytes, found {len}")]
    InvalidSeedLength { len: u64 },
    #[error("invalid peer id: {value}")]
    InvalidPeerId { value: String },
    #[error("invalid multiaddr: {value}")]
    InvalidMultiaddr { value: String },
    #[error("message node: {message}")]
    MessageNode { message: String },
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    pub value: String,
}

#[cfg(feature = "message_node")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AukiMessageEvent {
    pub peer_id: String,
    pub envelope: Vec<u8>,
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

#[cfg(feature = "message_node")]
#[derive(uniffi::Object)]
pub struct AukiMessageNode {
    inner: MessageNode,
}

#[cfg(feature = "message_node")]
#[uniffi::export]
impl AukiMessageNode {
    #[uniffi::constructor]
    pub fn from_wallet_seed(
        seed: Vec<u8>,
        listen_addrs: Vec<String>,
        agent_version: String,
    ) -> Result<Arc<Self>, NetworkError> {
        let wallet = Wallet::from_seed(&seed32(seed)?);
        let identity = core::PeerIdentity::from_wallet(&wallet);
        let inner = MessageNode::spawn(
            identity,
            MessageNodeConfig {
                listen_addresses: parse_multiaddrs(listen_addrs)?,
                agent_version,
            },
        )
        .map_err(network_error)?;
        Ok(Arc::new(Self { inner }))
    }

    pub fn peer_id(&self) -> String {
        self.inner.local_peer_id().to_string()
    }

    pub fn listen_addrs(&self) -> Vec<String> {
        self.inner
            .listen_addrs()
            .into_iter()
            .map(|addr| addr.to_string())
            .collect()
    }

    pub fn dial(&self, peer_id: String, addrs: Vec<String>) -> Result<(), NetworkError> {
        self.inner
            .dial(parse_peer_id(&peer_id)?, parse_multiaddrs(addrs)?)
            .map_err(network_error)
    }

    pub fn send_message_envelope_bytes(
        &self,
        peer_id: String,
        envelope: Vec<u8>,
    ) -> Result<Vec<u8>, NetworkError> {
        let ack = self
            .inner
            .send_envelope_bytes(parse_peer_id(&peer_id)?, envelope)
            .map_err(network_error)?;
        Ok(ack.encode_to_vec())
    }

    pub fn next_event(&self) -> Result<Option<AukiMessageEvent>, NetworkError> {
        let Some(event) = self.inner.next_event().map_err(network_error)? else {
            return Ok(None);
        };
        Ok(Some(AukiMessageEvent {
            peer_id: event.peer_id.to_string(),
            envelope: event.envelope.encode_to_vec(),
        }))
    }

    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}

fn seed32(seed: Vec<u8>) -> Result<[u8; 32], NetworkError> {
    let len = seed.len();
    seed.try_into()
        .map_err(|_| NetworkError::InvalidSeedLength { len: len as u64 })
}

#[cfg(feature = "message_node")]
fn parse_peer_id(value: &str) -> Result<Libp2pPeerId, NetworkError> {
    value.parse().map_err(|_| NetworkError::InvalidPeerId {
        value: value.to_string(),
    })
}

#[cfg(feature = "message_node")]
fn parse_multiaddrs(values: Vec<String>) -> Result<Vec<Multiaddr>, NetworkError> {
    values
        .into_iter()
        .map(|value| {
            value
                .parse()
                .map_err(|_| NetworkError::InvalidMultiaddr { value })
        })
        .collect()
}

#[cfg(feature = "message_node")]
fn network_error(error: crate::message_node::MessageNodeError) -> NetworkError {
    NetworkError::MessageNode {
        message: error.to_string(),
    }
}
