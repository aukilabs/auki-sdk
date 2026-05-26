//! SDK-facing high-level node API.

use crate::{
    AukiP2pNode, AukiP2pNodeConfig, AukiP2pNodeError, ConfiguredPeer, LocalPeerIdentity,
    PeerRelationship, RelationshipFailureRecord, RelationshipFailureScope,
    RelationshipStatusBuildError, RelationshipStatusOptions, build_relationship_status_snapshot,
};
use auki_identity::PublicKey as WalletPublicKey;
use auki_protocol::v1::{
    domain::{DelegationScope, DomainDeclaration, DomainDelegation, DomainError},
    error,
    offer::{Offer, OfferCatalogResponse, OfferCatalogResponseError},
    status::{LocalDomainRole, LocalDomainStatus, StatusSnapshot},
};
use libp2p::{Multiaddr, PeerId};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fmt};

/// High-level RFC-first runtime handle for SDK and app code.
pub struct AukiNode {
    node: AukiP2pNode,
    local_domains: BTreeMap<String, LocalDomainRegistration>,
    local_offers: BTreeMap<(String, String), Offer>,
    relationships: BTreeMap<PeerId, PeerRelationship>,
}

/// Local domain authority material registered with the high-level node.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalDomainRegistration {
    domain_id: String,
    role: LocalDomainRole,
    declaration: DomainDeclaration,
    delegation: Option<DomainDelegation>,
    advertised: bool,
    delegation_scopes: Vec<DelegationScope>,
    delegation_expires_at: Option<String>,
}

/// High-level node events that do not expose libp2p stream or frame internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AukiNodeEvent {
    /// A local listener was bound.
    Listening {
        /// Bound listen address.
        address: Multiaddr,
    },
    /// A peer has at least one retained transport connection.
    PeerConnected {
        /// Remote peer id.
        peer_id: PeerId,
    },
    /// A peer connection closed.
    PeerConnectionClosed {
        /// Remote peer id.
        peer_id: PeerId,
        /// Retained connections still active after the close.
        active_connections: usize,
    },
    /// A duplicate connection was closed by local connection policy.
    PeerDuplicateConnectionClosed {
        /// Remote peer id.
        peer_id: PeerId,
    },
    /// An outbound dial failed.
    PeerDialFailed {
        /// Remote peer id, when libp2p associated the failure with one.
        peer_id: Option<PeerId>,
        /// Local diagnostic message.
        error: String,
    },
    /// An inbound connection failed before becoming a peer relationship.
    IncomingConnectionFailed {
        /// Local diagnostic message.
        error: String,
    },
}

/// High-level node API errors.
#[derive(Debug)]
pub enum AukiNodeError {
    /// Low-level node construction or command failed.
    Node(AukiP2pNodeError),
    /// No configured peer exists for the requested peer id.
    UnknownConfiguredPeer {
        /// Requested peer id.
        peer_id: PeerId,
    },
    /// The configured peer exists but has no dial addresses.
    ConfiguredPeerMissingDialAddresses {
        /// Requested peer id.
        peer_id: PeerId,
    },
    /// Status projection failed.
    Status(RelationshipStatusBuildError),
    /// Local domain authority material failed validation.
    LocalDomain(DomainError),
    /// Owner domain registration does not belong to this node's wallet.
    LocalDomainWalletMismatch {
        /// Domain id being registered.
        domain_id: String,
        /// Owner wallet public key carried by the domain declaration.
        owner_wallet_public_key: WalletPublicKey,
        /// Local node wallet public key.
        local_wallet_public_key: WalletPublicKey,
    },
    /// Local offer references a domain that is not registered locally.
    LocalOfferDomainNotRegistered {
        /// Offer domain id.
        domain_id: String,
        /// Producer-scoped offer id.
        offer_id: String,
    },
    /// Local offer catalog projection failed.
    OfferCatalog(OfferCatalogResponseError),
}

impl LocalDomainRegistration {
    /// Register a domain owned by the local node wallet.
    pub fn owner(declaration: DomainDeclaration, advertised: bool) -> Result<Self, DomainError> {
        let verified = declaration.verify()?;
        Ok(Self {
            domain_id: verified.domain_id,
            role: LocalDomainRole::Owner,
            declaration,
            delegation: None,
            advertised,
            delegation_scopes: Vec::new(),
            delegation_expires_at: None,
        })
    }

    /// Register a domain delegated to the local node.
    pub fn delegate(
        declaration: DomainDeclaration,
        delegation: DomainDelegation,
        advertised: bool,
    ) -> Result<Self, DomainError> {
        let verified_declaration = declaration.verify()?;
        let verified_delegation = delegation.verify()?;
        if verified_delegation.domain_id != verified_declaration.domain_id {
            return Err(DomainError::DelegationDomainMismatch {
                delegated: verified_delegation.domain_id,
                expected: verified_declaration.domain_id,
            });
        }
        if verified_delegation.domain_owner_public_key
            != verified_declaration.domain_owner_public_key
        {
            return Err(DomainError::DelegationOwnerMismatch {
                delegated: verified_delegation.domain_owner_public_key,
                expected: verified_declaration.domain_owner_public_key,
            });
        }

        Ok(Self {
            domain_id: verified_declaration.domain_id,
            role: LocalDomainRole::Delegate,
            declaration,
            delegation: Some(delegation),
            advertised,
            delegation_scopes: verified_delegation.scopes,
            delegation_expires_at: Some(verified_delegation.expires_at),
        })
    }

    /// Encoded v1 domain id.
    pub fn domain_id(&self) -> &str {
        &self.domain_id
    }

    /// Local role for this domain.
    pub fn role(&self) -> LocalDomainRole {
        self.role
    }

    /// Whether this domain should be advertised by future discovery adapters.
    pub fn advertised(&self) -> bool {
        self.advertised
    }

    /// Domain declaration carried by this registration.
    pub fn declaration(&self) -> &DomainDeclaration {
        &self.declaration
    }

    /// Optional domain delegation carried by this registration.
    pub fn delegation(&self) -> Option<&DomainDelegation> {
        self.delegation.as_ref()
    }
}

impl AukiNode {
    /// Build a high-level node from a local identity and runtime config.
    pub fn new(
        identity: LocalPeerIdentity,
        config: AukiP2pNodeConfig,
    ) -> Result<Self, AukiNodeError> {
        let node = AukiP2pNode::new(identity, config).map_err(AukiNodeError::Node)?;
        let mut this = Self {
            node,
            local_domains: BTreeMap::new(),
            local_offers: BTreeMap::new(),
            relationships: BTreeMap::new(),
        };
        for peer in this.node.configured_peers().to_vec() {
            this.relationship_mut(peer.peer_id).configured();
        }
        Ok(this)
    }

    /// Return the local libp2p peer id.
    pub fn peer_id(&self) -> PeerId {
        self.node.peer_id()
    }

    /// Borrow the local identity and current peer binding.
    pub fn identity(&self) -> &LocalPeerIdentity {
        self.node.identity()
    }

    /// Borrow explicitly configured peers.
    pub fn configured_peers(&self) -> &[ConfiguredPeer] {
        self.node.configured_peers()
    }

    /// Borrow one tracked peer relationship.
    pub fn relationship(&self, peer_id: PeerId) -> Option<&PeerRelationship> {
        self.relationships.get(&peer_id)
    }

    /// Return relationship snapshots in deterministic peer-id order.
    pub fn relationships(&self) -> Vec<PeerRelationship> {
        self.relationships.values().cloned().collect()
    }

    /// Return local domain registrations in deterministic domain-id order.
    pub fn local_domains(&self) -> Vec<LocalDomainRegistration> {
        self.local_domains.values().cloned().collect()
    }

    /// Return local offers for one domain in deterministic offer-id order.
    pub fn local_offers(&self, domain_id: &str) -> Vec<Offer> {
        self.local_offers
            .values()
            .filter(|offer| offer.domain_id == domain_id)
            .cloned()
            .collect()
    }

    /// Configured local bind addresses.
    pub fn configured_listen_addresses(&self) -> &[Multiaddr] {
        self.node.configured_listen_addresses()
    }

    /// Listen addresses observed from libp2p after binding.
    pub fn observed_listen_addresses(&self) -> &[Multiaddr] {
        self.node.observed_listen_addresses()
    }

    /// Operator-supplied advertised addresses.
    pub fn advertised_addresses(&self) -> &[Multiaddr] {
        self.node.advertised_addresses()
    }

    /// Add or replace one explicitly configured peer.
    pub fn upsert_configured_peer(&mut self, peer: ConfiguredPeer) -> Result<(), AukiNodeError> {
        let peer_id = peer.peer_id;
        self.node
            .upsert_configured_peer(peer)
            .map_err(AukiNodeError::Node)?;
        self.relationship_mut(peer_id).configured();
        Ok(())
    }

    /// Add or replace a local domain after validating local authority.
    ///
    /// `now` is supplied by the caller so this runtime does not create or
    /// interpret a canonical clock for delegation validity.
    pub fn upsert_local_domain(
        &mut self,
        registration: LocalDomainRegistration,
        now: &str,
    ) -> Result<(), AukiNodeError> {
        self.validate_local_domain_registration(&registration, now)?;
        self.local_domains
            .insert(registration.domain_id.clone(), registration);
        Ok(())
    }

    /// Add or replace a local offer scoped to a registered local domain.
    pub fn upsert_local_offer(&mut self, offer: Offer) -> Result<(), AukiNodeError> {
        if !self.local_domains.contains_key(&offer.domain_id) {
            return Err(AukiNodeError::LocalOfferDomainNotRegistered {
                domain_id: offer.domain_id,
                offer_id: offer.offer_id,
            });
        }
        self.local_offers
            .insert((offer.domain_id.clone(), offer.offer_id.clone()), offer);
        Ok(())
    }

    /// Build a local offer-catalog response from registered local offers.
    pub fn local_offer_catalog_response(
        &self,
        generated_at: Option<&str>,
    ) -> Result<OfferCatalogResponse, AukiNodeError> {
        OfferCatalogResponse::create(
            self.local_offers.values().cloned().collect(),
            generated_at,
            Vec::new(),
        )
        .map_err(AukiNodeError::OfferCatalog)
    }

    /// Dial a configured peer through its configured addresses.
    pub fn dial_configured_peer(&mut self, peer_id: PeerId) -> Result<(), AukiNodeError> {
        let peer = self
            .node
            .configured_peer(peer_id)
            .cloned()
            .ok_or(AukiNodeError::UnknownConfiguredPeer { peer_id })?;
        if peer.dial_addresses.is_empty() {
            return Err(AukiNodeError::ConfiguredPeerMissingDialAddresses { peer_id });
        }

        self.node
            .dial_peer(peer_id, peer.dial_addresses)
            .map_err(AukiNodeError::Node)?;
        self.relationship_mut(peer_id).dialing();
        Ok(())
    }

    /// Wait for the next high-level node event and update relationship state.
    ///
    /// `observed_at` is supplied by the caller so this runtime does not create
    /// or interpret a canonical clock.
    pub async fn next_event(&mut self, observed_at: &str) -> Option<AukiNodeEvent> {
        let event = self.node.next_event().await?;
        let failure_cap = self.node.config().p2p.limits.retained_status_failures;
        Some(match event {
            crate::AukiP2pEvent::Listening { address } => AukiNodeEvent::Listening { address },
            crate::AukiP2pEvent::ConnectionEstablished { peer_id } => {
                self.relationship_mut(peer_id).connected();
                AukiNodeEvent::PeerConnected { peer_id }
            }
            crate::AukiP2pEvent::DuplicateConnectionClosed { peer_id } => {
                AukiNodeEvent::PeerDuplicateConnectionClosed { peer_id }
            }
            crate::AukiP2pEvent::ConnectionClosed { peer_id } => {
                let active_connections = self.node.active_connection_count(peer_id);
                if active_connections == 0 {
                    self.relationship_mut(peer_id)
                        .lost(observed_at.to_owned(), failure_cap);
                }
                AukiNodeEvent::PeerConnectionClosed {
                    peer_id,
                    active_connections,
                }
            }
            crate::AukiP2pEvent::OutgoingConnectionError { peer_id, error } => {
                if let Some(peer_id) = peer_id {
                    let mut failure = RelationshipFailureRecord::new(
                        error::TRANSPORT_FAILED,
                        observed_at.to_owned(),
                        RelationshipFailureScope::Transport,
                    );
                    failure.message = Some(error.clone());
                    self.relationship_mut(peer_id)
                        .degraded(failure, failure_cap);
                }
                AukiNodeEvent::PeerDialFailed { peer_id, error }
            }
            crate::AukiP2pEvent::IncomingConnectionError { error } => {
                AukiNodeEvent::IncomingConnectionFailed { error }
            }
        })
    }

    /// Build an in-process diagnostic status snapshot.
    pub fn status_snapshot(&self, generated_at: &str) -> Result<StatusSnapshot, AukiNodeError> {
        let local_peer = self.node.local_peer_status().map_err(AukiNodeError::Node)?;
        let local_domains = self.local_domain_statuses()?;
        let relationships = self.relationships();
        let options = RelationshipStatusOptions::from_config(&self.node.config().p2p);
        let relationship_status = build_relationship_status_snapshot(
            generated_at,
            local_peer.clone(),
            &relationships,
            options,
        )
        .map_err(AukiNodeError::Status)?;
        StatusSnapshot::create(
            generated_at,
            local_peer,
            local_domains,
            relationship_status.remote_peers,
            relationship_status.active_paths,
            relationship_status.last_failures,
            relationship_status.discovery,
            relationship_status.metadata,
        )
        .map_err(|error| AukiNodeError::Status(RelationshipStatusBuildError::Status(error)))
    }

    fn relationship_mut(&mut self, peer_id: PeerId) -> &mut PeerRelationship {
        self.relationships
            .entry(peer_id)
            .or_insert_with(|| PeerRelationship::new(peer_id))
    }

    fn validate_local_domain_registration(
        &self,
        registration: &LocalDomainRegistration,
        now: &str,
    ) -> Result<(), AukiNodeError> {
        let verified_declaration = registration
            .declaration
            .verify()
            .map_err(AukiNodeError::LocalDomain)?;
        if verified_declaration.domain_id != registration.domain_id {
            return Err(AukiNodeError::LocalDomain(
                DomainError::DelegationDomainMismatch {
                    delegated: registration.domain_id.clone(),
                    expected: verified_declaration.domain_id,
                },
            ));
        }

        match registration.role {
            LocalDomainRole::Owner => {
                let local_wallet_public_key = self.identity().wallet_public_key();
                if verified_declaration.domain_owner_public_key != local_wallet_public_key {
                    return Err(AukiNodeError::LocalDomainWalletMismatch {
                        domain_id: registration.domain_id.clone(),
                        owner_wallet_public_key: verified_declaration.domain_owner_public_key,
                        local_wallet_public_key,
                    });
                }
            }
            LocalDomainRole::Delegate => {
                let delegation = registration.delegation.as_ref().ok_or_else(|| {
                    AukiNodeError::LocalDomain(DomainError::MissingScope {
                        scope: DelegationScope::Serve,
                    })
                })?;
                delegation
                    .verify_for_authority(
                        &registration.domain_id,
                        &verified_declaration.domain_owner_public_key,
                        &self.identity().wallet_public_key(),
                        &self.peer_id(),
                        DelegationScope::Serve,
                        now,
                    )
                    .map_err(AukiNodeError::LocalDomain)?;
                if registration.advertised {
                    delegation
                        .verify_for_authority(
                            &registration.domain_id,
                            &verified_declaration.domain_owner_public_key,
                            &self.identity().wallet_public_key(),
                            &self.peer_id(),
                            DelegationScope::Advertise,
                            now,
                        )
                        .map_err(AukiNodeError::LocalDomain)?;
                }
            }
            LocalDomainRole::Managed => {}
        }

        Ok(())
    }

    fn local_domain_statuses(&self) -> Result<Vec<LocalDomainStatus>, AukiNodeError> {
        self.local_domains
            .values()
            .map(|domain| {
                let mut object = Map::new();
                object.insert(
                    "domain_id".to_owned(),
                    Value::String(domain.domain_id.clone()),
                );
                object.insert(
                    "role".to_owned(),
                    Value::String(domain.role.as_str().to_owned()),
                );
                object.insert("declaration_present".to_owned(), Value::Bool(true));
                object.insert("declaration_valid".to_owned(), Value::Bool(true));
                object.insert(
                    "delegation_present".to_owned(),
                    Value::Bool(domain.delegation.is_some()),
                );
                if domain.delegation.is_some() {
                    object.insert("delegation_valid".to_owned(), Value::Bool(true));
                    object.insert(
                        "delegation_scopes".to_owned(),
                        Value::Array(
                            domain
                                .delegation_scopes
                                .iter()
                                .map(|scope| Value::String(scope.as_str().to_owned()))
                                .collect(),
                        ),
                    );
                    if let Some(expires_at) = &domain.delegation_expires_at {
                        object.insert(
                            "delegation_expires_at".to_owned(),
                            Value::String(expires_at.clone()),
                        );
                    }
                }
                object.insert("advertised".to_owned(), Value::Bool(domain.advertised));
                object.insert(
                    "serving_offers".to_owned(),
                    Value::Bool(
                        self.local_offers
                            .values()
                            .any(|offer| offer.domain_id == domain.domain_id),
                    ),
                );

                LocalDomainStatus::from_value(Value::Object(object)).map_err(|error| {
                    AukiNodeError::Status(RelationshipStatusBuildError::Status(error))
                })
            })
            .collect()
    }
}

impl fmt::Display for AukiNodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(error) => write!(f, "{error}"),
            Self::UnknownConfiguredPeer { peer_id } => {
                write!(f, "unknown configured peer {peer_id}")
            }
            Self::ConfiguredPeerMissingDialAddresses { peer_id } => {
                write!(f, "configured peer {peer_id} has no dial addresses")
            }
            Self::Status(error) => write!(f, "{error}"),
            Self::LocalDomain(error) => write!(f, "{error}"),
            Self::LocalDomainWalletMismatch {
                domain_id,
                owner_wallet_public_key: _,
                local_wallet_public_key: _,
            } => write!(
                f,
                "local domain {domain_id} owner wallet does not match local node wallet"
            ),
            Self::LocalOfferDomainNotRegistered {
                domain_id,
                offer_id,
            } => write!(
                f,
                "local offer {offer_id} references unregistered domain {domain_id}"
            ),
            Self::OfferCatalog(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AukiNodeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PeerRelationshipState;
    use auki_identity::Wallet;
    use auki_protocol::v1::{
        domain::DOMAIN_NONCE_LEN,
        offer::{OfferAccessMode, OfferStatus, PayloadDescriptor},
    };
    use std::sync::Arc;
    use tokio::time::{Duration, timeout};

    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";
    const DELEGATION_EXPIRES_AT: &str = "2027-05-26T12:00:00Z";

    fn wallet(seed: u8) -> Arc<Wallet> {
        Wallet::from_seed(vec![seed; 32]).expect("32-byte seed")
    }

    fn identity_from_wallet(wallet: Arc<Wallet>) -> LocalPeerIdentity {
        LocalPeerIdentity::from_wallet(wallet, ISSUED_AT, Some("api-test"))
            .expect("local peer identity")
    }

    fn identity(seed: u8) -> LocalPeerIdentity {
        identity_from_wallet(wallet(seed))
    }

    fn domain_declaration(seed: u8, nonce_seed: u8) -> DomainDeclaration {
        let nonce = [nonce_seed; DOMAIN_NONCE_LEN];
        DomainDeclaration::create(&wallet(seed), &nonce, Some("api-domain"))
            .expect("domain declaration")
    }

    fn offer(domain_id: &str, offer_id: &str) -> Offer {
        Offer::create(
            offer_id,
            domain_id,
            "frame",
            OfferStatus::Available,
            vec![OfferAccessMode::Get],
            PayloadDescriptor::create("auki.frame"),
            Vec::new(),
        )
        .expect("offer")
    }

    async fn wait_for_listen_addr(node: &mut AukiNode) -> Multiaddr {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(AukiNodeEvent::Listening { address }) = node.next_event(ISSUED_AT).await
                {
                    return address;
                }
            }
        })
        .await
        .expect("listen address should be emitted")
    }

    #[test]
    fn initializes_configured_peer_relationships() {
        let remote_peer_id = identity(61).peer_id();
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config
            .p2p
            .configured_peers
            .push(ConfiguredPeer::new(remote_peer_id));

        let node = AukiNode::new(identity(60), config).expect("node");
        let relationship = node
            .relationship(remote_peer_id)
            .expect("configured relationship");

        assert_eq!(relationship.state, PeerRelationshipState::Configured);
        assert_eq!(node.configured_peers().len(), 1);

        let snapshot = node.status_snapshot(ISSUED_AT).expect("status snapshot");
        assert_eq!(snapshot.remote_peers.len(), 1);
        assert_eq!(
            snapshot.remote_peers[0].lifecycle_state.as_deref(),
            Some("configured")
        );
    }

    #[test]
    fn upsert_configured_peer_validates_dial_policy() {
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config.p2p.dial_policy = crate::DialPolicy::production_recommended();
        let mut node = AukiNode::new(identity(62), config).expect("node");
        let mut peer = ConfiguredPeer::new(identity(63).peer_id());
        peer.dial_addresses
            .push("/ip4/127.0.0.1/tcp/4001".parse().unwrap());

        let error = node
            .upsert_configured_peer(peer)
            .expect_err("loopback should be rejected by production dial policy");

        assert!(matches!(
            error,
            AukiNodeError::Node(AukiP2pNodeError::DialPolicy(_))
        ));
    }

    #[test]
    fn dial_configured_peer_requires_addresses() {
        let remote_peer_id = identity(65).peer_id();
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config
            .p2p
            .configured_peers
            .push(ConfiguredPeer::new(remote_peer_id));
        let mut node = AukiNode::new(identity(64), config).expect("node");

        let error = node
            .dial_configured_peer(remote_peer_id)
            .expect_err("addresses are required");

        assert!(matches!(
            error,
            AukiNodeError::ConfiguredPeerMissingDialAddresses { peer_id }
                if peer_id == remote_peer_id
        ));
    }

    #[test]
    fn local_owner_domain_and_offer_project_into_status() {
        let local_wallet = wallet(68);
        let identity = identity_from_wallet(local_wallet.clone());
        let declaration =
            DomainDeclaration::create(&local_wallet, &[1; DOMAIN_NONCE_LEN], Some("owned-domain"))
                .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut node =
            AukiNode::new(identity, AukiP2pNodeConfig::dial_only_development()).expect("node");

        node.upsert_local_domain(registration, ISSUED_AT)
            .expect("local owner domain should register");
        let snapshot = node.status_snapshot(ISSUED_AT).expect("status snapshot");
        assert_eq!(snapshot.local_domains.len(), 1);
        assert_eq!(
            snapshot.local_domains[0].domain_id.as_deref(),
            Some(domain_id.as_str())
        );
        assert_eq!(snapshot.local_domains[0].role, Some(LocalDomainRole::Owner));
        assert_eq!(snapshot.local_domains[0].advertised, Some(true));
        assert_eq!(snapshot.local_domains[0].serving_offers, Some(false));

        let offer = offer(&domain_id, "frame-main");
        node.upsert_local_offer(offer)
            .expect("local offer should register");
        assert_eq!(node.local_offers(&domain_id).len(), 1);
        let catalog = node
            .local_offer_catalog_response(Some(ISSUED_AT))
            .expect("local offer catalog");
        assert_eq!(catalog.offers.len(), 1);

        let snapshot = node.status_snapshot(ISSUED_AT).expect("status snapshot");
        assert_eq!(snapshot.local_domains[0].serving_offers, Some(true));
    }

    #[test]
    fn local_offer_requires_registered_domain() {
        let declaration = domain_declaration(69, 2);
        let domain_id = declaration.domain_id().unwrap().to_owned();
        let mut node =
            AukiNode::new(identity(70), AukiP2pNodeConfig::dial_only_development()).expect("node");

        let error = node
            .upsert_local_offer(offer(&domain_id, "unregistered"))
            .expect_err("domain must be registered");

        assert!(matches!(
            error,
            AukiNodeError::LocalOfferDomainNotRegistered { domain_id: rejected, offer_id }
                if rejected == domain_id && offer_id == "unregistered"
        ));
    }

    #[test]
    fn delegated_domain_registration_requires_advertise_scope_when_advertised() {
        let owner_wallet = wallet(71);
        let local_wallet = wallet(72);
        let local_identity = identity_from_wallet(local_wallet.clone());
        let declaration = DomainDeclaration::create(
            &owner_wallet,
            &[3; DOMAIN_NONCE_LEN],
            Some("delegated-domain"),
        )
        .expect("domain declaration");
        let domain_id = declaration.domain_id().unwrap().to_owned();
        let delegation = DomainDelegation::create(
            &owner_wallet,
            &domain_id,
            &local_wallet.public_key(),
            &local_identity.peer_id(),
            &[DelegationScope::Serve],
            ISSUED_AT,
            DELEGATION_EXPIRES_AT,
            None,
        )
        .expect("domain delegation");
        let registration = LocalDomainRegistration::delegate(declaration, delegation, true)
            .expect("delegate registration");
        let mut node = AukiNode::new(local_identity, AukiP2pNodeConfig::dial_only_development())
            .expect("node");

        let error = node
            .upsert_local_domain(registration, ISSUED_AT)
            .expect_err("advertised delegated domains need advertise scope");

        assert!(matches!(
            error,
            AukiNodeError::LocalDomain(DomainError::MissingScope {
                scope: DelegationScope::Advertise
            })
        ));
    }

    #[test]
    fn delegated_domain_registration_accepts_serve_scope_for_non_advertised_domain() {
        let owner_wallet = wallet(73);
        let local_wallet = wallet(74);
        let local_identity = identity_from_wallet(local_wallet.clone());
        let declaration = DomainDeclaration::create(
            &owner_wallet,
            &[4; DOMAIN_NONCE_LEN],
            Some("delegated-domain"),
        )
        .expect("domain declaration");
        let domain_id = declaration.domain_id().unwrap().to_owned();
        let delegation = DomainDelegation::create(
            &owner_wallet,
            &domain_id,
            &local_wallet.public_key(),
            &local_identity.peer_id(),
            &[DelegationScope::Serve],
            ISSUED_AT,
            DELEGATION_EXPIRES_AT,
            None,
        )
        .expect("domain delegation");
        let registration = LocalDomainRegistration::delegate(declaration, delegation, false)
            .expect("delegate registration");
        let mut node = AukiNode::new(local_identity, AukiP2pNodeConfig::dial_only_development())
            .expect("node");

        node.upsert_local_domain(registration, ISSUED_AT)
            .expect("non-advertised served delegation should register");
        let snapshot = node.status_snapshot(ISSUED_AT).expect("status snapshot");

        assert_eq!(snapshot.local_domains.len(), 1);
        assert_eq!(
            snapshot.local_domains[0].domain_id.as_deref(),
            Some(domain_id.as_str())
        );
        assert_eq!(
            snapshot.local_domains[0].role,
            Some(LocalDomainRole::Delegate)
        );
        assert_eq!(snapshot.local_domains[0].advertised, Some(false));
        assert_eq!(
            snapshot.local_domains[0].delegation_scopes,
            vec!["serve".to_owned()]
        );
        assert_eq!(
            snapshot.local_domains[0].delegation_expires_at.as_deref(),
            Some(DELEGATION_EXPIRES_AT)
        );
    }

    #[tokio::test]
    async fn configured_peer_dial_updates_relationship_status() {
        let mut dialer =
            AukiNode::new(identity(66), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener =
            AukiNode::new(identity(67), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();
        let dialer_peer_id = dialer.peer_id();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer should be accepted");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("configured dial should start");
        assert_eq!(
            dialer.relationship(listener_peer_id).unwrap().state,
            PeerRelationshipState::Dialing
        );

        timeout(Duration::from_secs(10), async {
            let mut dialer_observed_listener = false;
            let mut listener_observed_dialer = false;

            loop {
                tokio::select! {
                    event = dialer.next_event(ISSUED_AT) => {
                        if let Some(AukiNodeEvent::PeerConnected { peer_id }) = event {
                            dialer_observed_listener |= peer_id == listener_peer_id;
                        }
                    }
                    event = listener.next_event(ISSUED_AT) => {
                        if let Some(AukiNodeEvent::PeerConnected { peer_id }) = event {
                            listener_observed_dialer |= peer_id == dialer_peer_id;
                        }
                    }
                }

                if dialer_observed_listener && listener_observed_dialer {
                    break;
                }
            }
        })
        .await
        .expect("configured peers should connect");

        assert_eq!(
            dialer.relationship(listener_peer_id).unwrap().state,
            PeerRelationshipState::Connected
        );
        let snapshot = dialer.status_snapshot(ISSUED_AT).expect("status snapshot");
        assert_eq!(
            snapshot.remote_peers[0].lifecycle_state.as_deref(),
            Some("connected")
        );
    }
}
