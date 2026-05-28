//! Ergonomic high-level node construction helpers.

use crate::{
    AukiNode, AukiNodeError, AukiP2pNodeConfig, LocalDomainRegistration, LocalPeerIdentity,
    LocalPeerIdentityError,
};
use auki_identity::Wallet;
use auki_protocol::v1::domain::{DOMAIN_NONCE_LEN, DomainDeclaration, DomainError};
use std::{fmt, sync::Arc};

/// Builder for a high-level RFC-first Auki node.
///
/// The builder keeps common SDK setup steps together: derive a local peer
/// identity, select a transport config, register local authority material, and
/// build an [`AukiNode`] with all registrations validated.
pub struct AukiNodeBuilder {
    identity: LocalPeerIdentity,
    config: AukiP2pNodeConfig,
    local_domains: Vec<LocalDomainRegistration>,
}

/// Errors produced while building a high-level node.
#[derive(Debug)]
pub enum AukiNodeBuilderError {
    /// Local peer identity construction failed.
    Identity(LocalPeerIdentityError),
    /// Domain authority material creation or verification failed.
    Domain(DomainError),
    /// High-level node construction or registration failed.
    Node(AukiNodeError),
}

impl AukiNodeBuilder {
    /// Start from an already-created local peer identity.
    pub fn from_identity(identity: LocalPeerIdentity) -> Self {
        Self {
            identity,
            config: AukiP2pNodeConfig::dial_only_development(),
            local_domains: Vec::new(),
        }
    }

    /// Derive a local peer identity from the wallet and start a builder.
    pub fn from_wallet(
        wallet: Arc<Wallet>,
        issued_at: &str,
        label: Option<&str>,
    ) -> Result<Self, AukiNodeBuilderError> {
        let identity = LocalPeerIdentity::from_wallet(wallet, issued_at, label)?;
        Ok(Self::from_identity(identity))
    }

    /// Borrow the local identity selected for this node.
    pub fn identity(&self) -> &LocalPeerIdentity {
        &self.identity
    }

    /// Borrow the selected node config.
    pub fn config(&self) -> &AukiP2pNodeConfig {
        &self.config
    }

    /// Replace the node transport/runtime config.
    pub fn with_config(mut self, config: AukiP2pNodeConfig) -> Self {
        self.config = config;
        self
    }

    /// Select the local browser-reachable development transport config.
    pub fn with_browser_reachable_development(self) -> Self {
        self.with_config(AukiP2pNodeConfig::loopback_browser_reachable_development())
    }

    /// Add one already-created local domain registration.
    pub fn with_local_domain(mut self, registration: LocalDomainRegistration) -> Self {
        self.local_domains.push(registration);
        self
    }

    /// Create and register a local owner domain declaration for this node wallet.
    pub fn with_owner_domain(
        mut self,
        nonce: [u8; DOMAIN_NONCE_LEN],
        label: Option<&str>,
        advertised: bool,
    ) -> Result<Self, AukiNodeBuilderError> {
        let declaration = DomainDeclaration::create(self.identity.wallet(), &nonce, label)?;
        let registration = LocalDomainRegistration::owner(declaration, advertised)?;
        self.local_domains.push(registration);
        Ok(self)
    }

    /// Return the first registered local domain id, when one exists.
    pub fn primary_domain_id(&self) -> Option<&str> {
        self.local_domains
            .first()
            .map(LocalDomainRegistration::domain_id)
    }

    /// Build the high-level node and register all local domains.
    pub fn build(self, now: &str) -> Result<AukiNode, AukiNodeBuilderError> {
        let mut node = AukiNode::new(self.identity, self.config)?;
        for registration in self.local_domains {
            node.upsert_local_domain(registration, now)?;
        }
        Ok(node)
    }
}

impl From<LocalPeerIdentityError> for AukiNodeBuilderError {
    fn from(error: LocalPeerIdentityError) -> Self {
        Self::Identity(error)
    }
}

impl From<DomainError> for AukiNodeBuilderError {
    fn from(error: DomainError) -> Self {
        Self::Domain(error)
    }
}

impl From<AukiNodeError> for AukiNodeBuilderError {
    fn from(error: AukiNodeError) -> Self {
        Self::Node(error)
    }
}

impl fmt::Display for AukiNodeBuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity(error) => write!(f, "{error}"),
            Self::Domain(error) => write!(f, "{error}"),
            Self::Node(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AukiNodeBuilderError {}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";

    #[test]
    fn builder_from_wallet_uses_dial_only_development_by_default() {
        let wallet = Wallet::from_seed(vec![21; 32]).expect("wallet");
        let builder =
            AukiNodeBuilder::from_wallet(wallet, ISSUED_AT, Some("builder")).expect("builder");

        assert!(builder.config().listen_addresses.is_empty());
        assert_eq!(
            builder.identity().peer_id(),
            builder.identity().public_key().to_peer_id()
        );
        assert_eq!(builder.primary_domain_id(), None);
    }

    #[test]
    fn builder_registers_owner_domain_before_building_node() {
        let wallet = Wallet::from_seed(vec![22; 32]).expect("wallet");
        let builder = AukiNodeBuilder::from_wallet(wallet, ISSUED_AT, Some("builder"))
            .expect("builder")
            .with_owner_domain([3; DOMAIN_NONCE_LEN], Some("builder-domain"), true)
            .expect("owner domain");
        let domain_id = builder
            .primary_domain_id()
            .expect("primary domain")
            .to_owned();

        let node = builder.build(ISSUED_AT).expect("node");
        let local_domains = node.local_domains();

        assert_eq!(local_domains.len(), 1);
        assert_eq!(local_domains[0].domain_id(), domain_id);
        assert!(local_domains[0].advertised());
    }

    #[test]
    fn browser_reachable_development_config_is_one_call() {
        let wallet = Wallet::from_seed(vec![23; 32]).expect("wallet");
        let builder = AukiNodeBuilder::from_wallet(wallet, ISSUED_AT, Some("builder"))
            .expect("builder")
            .with_browser_reachable_development();

        assert_eq!(builder.config().listen_addresses.len(), 2);
    }
}
