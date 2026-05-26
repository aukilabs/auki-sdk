//! Clean libp2p runtime for the RFC-first Auki peer-to-peer protocol.
//!
//! This crate is libp2p-shaped from the boundary inward. Pure policy and
//! validation helpers live here only when they need runtime configuration or
//! libp2p context; protocol truth remains in `auki-protocol`.

pub mod config;
pub mod identity;
pub mod node;
pub mod protocols;

pub use config::{
    AukiP2pConfig, AuthorityDeadlineConfig, ConfigError, ConfiguredPeer, DialPolicy,
    DialPolicyError, DomainAccessPolicy, OfferPolicy, PeerAdmissionConfig,
    PeerBindingFreshnessConfig, RuntimeLimits, StatusPrivacyConfig,
};
pub use identity::{LocalPeerIdentity, LocalPeerIdentityError, PEER_DERIVATION_LABEL};
pub use node::{AukiP2pEvent, AukiP2pNode, AukiP2pNodeConfig, AukiP2pNodeError};
