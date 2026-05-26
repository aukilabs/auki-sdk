//! SDK-facing high-level node API.

use crate::{
    AppAllowedOffer, AppDomainAccess, AppOfferPolicy, AukiP2pNode, AukiP2pNodeConfig,
    AukiP2pNodeError, ConfiguredPeer, GetInput, GetOutcome, HandshakePolicyError,
    HandshakeValidationInput, HandshakeValidationResult, Libp2pOfferCatalogClient,
    Libp2pPathClient, Libp2pSubscription, LifecycleOpenStreamError, LifecycleProtocolError,
    LifecycleStreamGuard, LoadedRemoteOffer, LocalPeerIdentity, OfferCatalogLoadState,
    OfferLoadContext, OfferLoadError, OfferLoadReport, PathClientError, PathContext,
    PathOrchestrationError, PeerRelationship, RelationshipFailureRecord, RelationshipFailureScope,
    RelationshipLoadedOffer, RelationshipRegistryReferenceStatus, RelationshipStatusBuildError,
    RelationshipStatusOptions, SubscribeInput, accept_subscribe_data_frame,
    build_relationship_status_snapshot, exchange_peer_handshake_strict, get_over_libp2p,
    load_remote_offers_over_libp2p, open_lifecycle_stream_once, subscribe_over_libp2p,
    validate_remote_handshake,
};
use auki_identity::PublicKey as WalletPublicKey;
use auki_protocol::v1::{
    authority::{DeclaredDomain, PeerAuthorization},
    domain::{DelegationScope, DomainDeclaration, DomainDelegation, DomainError},
    error,
    frame::FrameError,
    handshake::{HandshakeError, PeerHandshake},
    message::SpatialMessage,
    offer::{
        Offer, OfferAccessMode, OfferCatalogPath, OfferCatalogPathError, OfferCatalogRequest,
        OfferCatalogRequestError, OfferCatalogResponse, OfferCatalogResponseError,
    },
    status::{LocalDomainRole, LocalDomainStatus, StatusSnapshot},
};
use libp2p::{Multiaddr, PeerId};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fmt, future::Future};

/// High-level RFC-first runtime handle for SDK and app code.
pub struct AukiNode {
    node: AukiP2pNode,
    local_domains: BTreeMap<String, LocalDomainRegistration>,
    local_offers: BTreeMap<(String, String), Offer>,
    remote_offer_reports: BTreeMap<PeerId, OfferLoadReport>,
    relationships: BTreeMap<PeerId, PeerRelationship>,
    lifecycle_stream_guard: LifecycleStreamGuard,
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

/// High-level accepted subscription handle.
pub struct AukiSubscription {
    inner: Libp2pSubscription,
}

/// High-level remote offer-catalog load input. Callers do not pass protocol frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOfferLoadInput {
    /// Domain filters requested by the consumer. Empty means all visible domains.
    pub domain_ids: Vec<String>,
    /// Offer-kind filters requested by the consumer. Empty means all kinds.
    pub kinds: Vec<String>,
    /// Whether inline canonical registry entries should be requested.
    pub include_inline_registry_entries: bool,
    /// Access mode the caller intends to use, if known.
    pub requested_access_mode: Option<OfferAccessMode>,
    /// Locally supported offer kinds. Empty defers this check.
    pub supported_kinds: Vec<String>,
    /// Locally supported payload types. Empty defers this check.
    pub supported_payload_types: Vec<String>,
    /// Application offer-policy decision when config uses app-policy.
    pub app_offer_policy: RemoteOfferAppPolicy,
}

/// Application offer-policy input for one remote catalog load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteOfferAppPolicy {
    /// No application policy decision was supplied.
    NotProvided,
    /// Application policy allows every otherwise-usable offer.
    AllowAll,
    /// Application policy allows only these domain/offer tuples.
    AllowOnly {
        /// Allowed offer tuples.
        offers: Vec<RemoteAllowedOffer>,
        /// Stable failure code to report for offers outside the allow-list.
        failure_code: &'static str,
    },
    /// Application policy rejects every offer.
    RejectAll {
        /// Stable failure code to report for rejected offers.
        failure_code: &'static str,
    },
}

/// Application-selected remote offer allowed by local app policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAllowedOffer {
    /// Offer domain id.
    pub domain_id: String,
    /// Producer-scoped offer id.
    pub offer_id: String,
}

/// High-level lifecycle input. Callers do not pass handshake frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleInput {
    /// Application peer-admission decision when config uses app-policy.
    pub app_peer_authorization: Option<PeerAuthorization>,
    /// Application domain-access decision when config uses app-policy.
    pub app_domain_access: LifecycleDomainAccess,
    /// Authorization-material `type` values required by local policy.
    pub required_authorization_material_types: Vec<String>,
}

/// Application domain-access input for one lifecycle validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleDomainAccess {
    /// No application domain-access decision was supplied.
    NotProvided,
    /// Application policy allows every authority-valid served domain.
    AllowAll,
    /// Application policy allows only these domain ids.
    AllowOnly(Vec<String>),
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
    /// Local offer-catalog path construction failed.
    OfferCatalogPath(OfferCatalogPathError),
    /// Local offer-catalog request construction failed.
    OfferCatalogRequest(OfferCatalogRequestError),
    /// Remote offer loading failed.
    OfferLoad(OfferLoadError),
    /// Local peer-handshake construction failed.
    LocalHandshake(HandshakeError),
    /// Lifecycle stream open failed.
    LifecycleOpen(LifecycleOpenStreamError),
    /// Lifecycle stream exchange failed.
    Lifecycle(LifecycleProtocolError),
    /// Remote handshake failed local policy validation.
    HandshakePolicy(HandshakePolicyError),
    /// No loaded remote offers are available for the requested peer.
    RemoteOffersNotLoaded {
        /// Remote peer id.
        peer_id: PeerId,
    },
    /// Get or Subscribe path orchestration failed.
    Path(PathOrchestrationError),
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

impl AukiSubscription {
    /// Path id used for status tracking.
    pub fn path_id(&self) -> &str {
        self.inner.handle().path_id()
    }

    /// Remote peer id.
    pub fn peer_id(&self) -> PeerId {
        self.inner.handle().peer_id()
    }

    /// Accepted payload type for this subscription.
    pub fn payload_type(&self) -> &str {
        self.inner.handle().payload_type()
    }

    /// Last accepted sequence value.
    pub fn last_sequence(&self) -> Option<u64> {
        self.inner.handle().last_sequence()
    }

    /// Observed sequence-gap count.
    pub fn sequence_gap_count(&self) -> u64 {
        self.inner.handle().sequence_gap_count()
    }
}

impl RemoteOfferLoadInput {
    /// Create remote offer-load input without filters.
    pub fn new() -> Self {
        Self {
            domain_ids: Vec::new(),
            kinds: Vec::new(),
            include_inline_registry_entries: false,
            requested_access_mode: None,
            supported_kinds: Vec::new(),
            supported_payload_types: Vec::new(),
            app_offer_policy: RemoteOfferAppPolicy::NotProvided,
        }
    }
}

impl Default for RemoteOfferLoadInput {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteOfferAppPolicy {
    fn allowed_offers(&self) -> Vec<AppAllowedOffer<'_>> {
        match self {
            Self::AllowOnly { offers, .. } => offers
                .iter()
                .map(|offer| AppAllowedOffer {
                    domain_id: &offer.domain_id,
                    offer_id: &offer.offer_id,
                })
                .collect(),
            Self::NotProvided | Self::AllowAll | Self::RejectAll { .. } => Vec::new(),
        }
    }

    fn as_app_offer_policy<'a>(
        &'a self,
        allowed_offers: &'a [AppAllowedOffer<'a>],
    ) -> AppOfferPolicy<'a> {
        match self {
            Self::NotProvided => AppOfferPolicy::NotProvided,
            Self::AllowAll => AppOfferPolicy::AllowAll,
            Self::AllowOnly { failure_code, .. } => AppOfferPolicy::AllowOnly {
                offers: allowed_offers,
                failure_code,
            },
            Self::RejectAll { failure_code } => AppOfferPolicy::RejectAll { failure_code },
        }
    }
}

impl RemoteAllowedOffer {
    /// Create one app-policy allowed remote offer tuple.
    pub fn new(domain_id: impl Into<String>, offer_id: impl Into<String>) -> Self {
        Self {
            domain_id: domain_id.into(),
            offer_id: offer_id.into(),
        }
    }
}

impl LifecycleInput {
    /// Create lifecycle input without app-policy decisions or required material.
    pub fn new() -> Self {
        Self {
            app_peer_authorization: None,
            app_domain_access: LifecycleDomainAccess::NotProvided,
            required_authorization_material_types: Vec::new(),
        }
    }
}

impl Default for LifecycleInput {
    fn default() -> Self {
        Self::new()
    }
}

impl LifecycleDomainAccess {
    fn allowed_domain_refs(&self) -> Vec<&str> {
        match self {
            Self::AllowOnly(domain_ids) => domain_ids.iter().map(String::as_str).collect(),
            Self::NotProvided | Self::AllowAll => Vec::new(),
        }
    }

    fn as_app_domain_access<'a>(&self, allowed_domain_ids: &'a [&'a str]) -> AppDomainAccess<'a> {
        match self {
            Self::NotProvided => AppDomainAccess::NotProvided,
            Self::AllowAll => AppDomainAccess::AllowAll,
            Self::AllowOnly(_) => AppDomainAccess::AllowOnly(allowed_domain_ids),
        }
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
            remote_offer_reports: BTreeMap::new(),
            relationships: BTreeMap::new(),
            lifecycle_stream_guard: LifecycleStreamGuard::default(),
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

    /// Borrow one loaded remote offer report.
    pub fn remote_offer_report(&self, peer_id: PeerId) -> Option<&OfferLoadReport> {
        self.remote_offer_reports.get(&peer_id)
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

    /// Run the v1 lifecycle exchange and authorize one connected peer.
    pub async fn run_lifecycle(
        &mut self,
        peer_id: PeerId,
        input: LifecycleInput,
        now: &str,
    ) -> Result<HandshakeValidationResult, AukiNodeError> {
        let local_handshake = self.local_peer_handshake()?;
        let p2p_config = self.node.config().p2p.clone();
        let mut control = self.node.stream_control();
        let open =
            open_lifecycle_stream_once(&mut control, &mut self.lifecycle_stream_guard, peer_id);
        let mut stream = drive_node_until(&mut self.node, open)
            .await
            .map_err(AukiNodeError::LifecycleOpen)?;

        let exchange = exchange_peer_handshake_strict(
            &mut stream,
            peer_id,
            &local_handshake,
            p2p_config.limits.handshake_frame_body_bytes,
        );
        let exchange = match drive_node_until(&mut self.node, exchange).await {
            Ok(exchange) => exchange,
            Err(error) => {
                self.lifecycle_stream_guard.fail(peer_id);
                self.record_lifecycle_failure(peer_id, &error, now);
                return Err(AukiNodeError::Lifecycle(error));
            }
        };

        let required_material_types = input
            .required_authorization_material_types
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let allowed_domain_ids = input.app_domain_access.allowed_domain_refs();
        let mut validation_input = HandshakeValidationInput::new(
            &exchange.authenticated_peer_id,
            &exchange.handshake,
            &p2p_config,
            now,
        );
        validation_input.app_peer_authorization = input.app_peer_authorization;
        validation_input.app_domain_access = input
            .app_domain_access
            .as_app_domain_access(&allowed_domain_ids);
        validation_input.required_authorization_material_types = &required_material_types;

        match validate_remote_handshake(validation_input) {
            Ok(result) => {
                self.lifecycle_stream_guard.complete(peer_id);
                self.relationship_mut(peer_id)
                    .handshake_accepted(result.clone());
                Ok(result)
            }
            Err(error) => {
                self.lifecycle_stream_guard.fail(peer_id);
                let failure_cap = self.node.config().p2p.limits.retained_status_failures;
                self.relationship_mut(peer_id).handshake_failed(
                    &error,
                    now.to_owned(),
                    failure_cap,
                );
                Err(AukiNodeError::HandshakePolicy(error))
            }
        }
    }

    /// Load and validate one remote peer's offer catalog over libp2p.
    pub async fn load_remote_offers(
        &mut self,
        peer_id: PeerId,
        input: RemoteOfferLoadInput,
        now: &str,
    ) -> Result<OfferLoadReport, AukiNodeError> {
        let request = OfferCatalogRequest::create(
            input.domain_ids.clone(),
            input.kinds.clone(),
            input.include_inline_registry_entries,
        )
        .map_err(AukiNodeError::OfferCatalogRequest)?;

        let p2p_config = self.node.config().p2p.clone();
        let supported_kinds =
            (!input.supported_kinds.is_empty()).then_some(input.supported_kinds.as_slice());
        let supported_payload_types = (!input.supported_payload_types.is_empty())
            .then_some(input.supported_payload_types.as_slice());
        let allowed_offers = input.app_offer_policy.allowed_offers();
        let app_offer_policy = input.app_offer_policy.as_app_offer_policy(&allowed_offers);
        let context = OfferLoadContext {
            config: &p2p_config,
            now,
            requested_access_mode: input.requested_access_mode,
            supported_kinds,
            supported_payload_types,
            app_offer_policy,
        };

        let report = {
            let node = &mut self.node;
            let relationships = &mut self.relationships;
            let relationship = relationships
                .entry(peer_id)
                .or_insert_with(|| PeerRelationship::new(peer_id));
            let mut client =
                Libp2pOfferCatalogClient::new(node.stream_control(), p2p_config.limits);
            let load = load_remote_offers_over_libp2p(relationship, &mut client, request, context);

            drive_node_until(node, load)
                .await
                .map_err(AukiNodeError::OfferLoad)?
        };

        self.remote_offer_reports.insert(peer_id, report.clone());
        Ok(report)
    }

    /// Add or replace a loaded remote offer report for one peer.
    pub fn upsert_remote_offer_report(&mut self, report: OfferLoadReport) {
        let peer_id = report.peer_id;
        let loaded_offers = report
            .offers
            .iter()
            .map(relationship_loaded_offer)
            .collect();
        let relationship = self.relationship_mut(peer_id);
        relationship.offer_catalog_state = OfferCatalogLoadState::Loaded;
        relationship.loaded_offers = loaded_offers;
        self.remote_offer_reports.insert(peer_id, report);
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

    /// Run one high-level Get over the configured libp2p path.
    pub async fn get(
        &mut self,
        peer_id: PeerId,
        input: GetInput,
        now: &str,
    ) -> Result<GetOutcome, AukiNodeError> {
        let offers = self
            .remote_offer_reports
            .get(&peer_id)
            .cloned()
            .ok_or(AukiNodeError::RemoteOffersNotLoaded { peer_id })?;
        let node = &mut self.node;
        let relationships = &mut self.relationships;
        let p2p_config = node.config().p2p.clone();
        let mut client = Libp2pPathClient::new(node.stream_control(), p2p_config.limits);
        let relationship = relationships
            .entry(peer_id)
            .or_insert_with(|| PeerRelationship::new(peer_id));
        let get = get_over_libp2p(
            relationship,
            &offers,
            &mut client,
            input,
            PathContext::new(&p2p_config, now),
        );

        drive_node_until(node, get)
            .await
            .map_err(AukiNodeError::Path)
    }

    /// Start one high-level Subscribe over the configured libp2p path.
    pub async fn subscribe(
        &mut self,
        peer_id: PeerId,
        input: SubscribeInput,
        now: &str,
    ) -> Result<AukiSubscription, AukiNodeError> {
        let offers = self
            .remote_offer_reports
            .get(&peer_id)
            .cloned()
            .ok_or(AukiNodeError::RemoteOffersNotLoaded { peer_id })?;
        let node = &mut self.node;
        let relationships = &mut self.relationships;
        let p2p_config = node.config().p2p.clone();
        let mut client = Libp2pPathClient::new(node.stream_control(), p2p_config.limits);
        let relationship = relationships
            .entry(peer_id)
            .or_insert_with(|| PeerRelationship::new(peer_id));
        let subscribe = subscribe_over_libp2p(
            relationship,
            &offers,
            &mut client,
            input,
            PathContext::new(&p2p_config, now),
        );

        drive_node_until(node, subscribe)
            .await
            .map(|inner| AukiSubscription { inner })
            .map_err(AukiNodeError::Path)
    }

    /// Read and validate the next Subscribe data message.
    pub async fn next_subscription_message(
        &mut self,
        subscription: &mut AukiSubscription,
        now: &str,
    ) -> Result<SpatialMessage, AukiNodeError> {
        let p2p_config = self.node.config().p2p.clone();
        let max_body_len = p2p_config.limits.subscribe_message_frame_body_bytes;
        let read = subscription.inner.read_next_frame(max_body_len);
        let frame = drive_node_until(&mut self.node, read)
            .await
            .map_err(|error| AukiNodeError::Path(path_client_read_error(error)))?;
        let peer_id = subscription.peer_id();
        let relationship = self.relationship_mut(peer_id);
        accept_subscribe_data_frame(
            relationship,
            subscription.inner.handle_mut(),
            &frame,
            PathContext::new(&p2p_config, now),
        )
        .map_err(AukiNodeError::Path)
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

    fn local_peer_handshake(&self) -> Result<PeerHandshake, AukiNodeError> {
        let declared_domains = self
            .local_domains
            .values()
            .map(|registration| {
                DeclaredDomain::new(
                    registration.domain_id.clone(),
                    registration.declaration.clone(),
                    registration.delegation.clone(),
                )
            })
            .collect();

        if self.local_offers.is_empty() {
            Ok(PeerHandshake::create(
                self.identity().peer_binding().clone(),
                declared_domains,
            ))
        } else {
            let catalog_path =
                OfferCatalogPath::create(None).map_err(AukiNodeError::OfferCatalogPath)?;
            let handshake = PeerHandshake::create_with_offer_catalog(
                self.identity().peer_binding().clone(),
                declared_domains,
                catalog_path,
            );
            PeerHandshake::from_value(handshake.into_value()).map_err(AukiNodeError::LocalHandshake)
        }
    }

    fn record_lifecycle_failure(
        &mut self,
        peer_id: PeerId,
        error: &LifecycleProtocolError,
        observed_at: &str,
    ) {
        let mut failure = RelationshipFailureRecord::new(
            lifecycle_protocol_failure_code(error),
            observed_at.to_owned(),
            RelationshipFailureScope::Peer,
        );
        failure.peer_id = Some(peer_id);
        failure.message = Some(error.to_string());
        let failure_cap = self.node.config().p2p.limits.retained_status_failures;
        self.relationship_mut(peer_id)
            .degraded(failure, failure_cap);
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
            Self::OfferCatalogPath(error) => write!(f, "{error}"),
            Self::OfferCatalogRequest(error) => write!(f, "{error}"),
            Self::OfferLoad(error) => write!(f, "{error}"),
            Self::LocalHandshake(error) => write!(f, "{error}"),
            Self::LifecycleOpen(error) => write!(f, "{error}"),
            Self::Lifecycle(error) => write!(f, "{error}"),
            Self::HandshakePolicy(error) => write!(f, "{error}"),
            Self::RemoteOffersNotLoaded { peer_id } => {
                write!(f, "remote offers are not loaded for peer {peer_id}")
            }
            Self::Path(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AukiNodeError {}

fn relationship_loaded_offer(loaded: &LoadedRemoteOffer) -> RelationshipLoadedOffer {
    RelationshipLoadedOffer {
        domain_id: Some(loaded.offer.domain_id.clone()),
        offer_id: Some(loaded.offer.offer_id.clone()),
        kind: Some(loaded.offer.kind.clone()),
        status: Some(loaded.offer.status.as_str().to_owned()),
        access_modes: loaded
            .offer
            .access_modes
            .iter()
            .map(|mode| mode.as_str().to_owned())
            .collect(),
        payload_type: Some(loaded.offer.payload.payload_type.clone()),
        registry_refs: loaded
            .offer
            .registry_refs
            .iter()
            .map(|reference| RelationshipRegistryReferenceStatus {
                registry: reference.registry.clone(),
                role: reference.role.clone(),
                id: reference.id.clone(),
                hash: reference.hash.clone(),
            })
            .collect(),
        usable: Some(loaded.usable),
        unusable_reason: loaded.unusable_reason.map(ToOwned::to_owned),
        updated_at: loaded.offer.updated_at.clone(),
        expires_at: loaded.offer.expires_at.clone(),
    }
}

fn path_client_read_error(error: PathClientError) -> PathOrchestrationError {
    PathOrchestrationError::SubscribeClient(error)
}

fn lifecycle_protocol_failure_code(error: &LifecycleProtocolError) -> &'static str {
    match error {
        LifecycleProtocolError::Io(_) => error::TRANSPORT_FAILED,
        LifecycleProtocolError::Frame(FrameError::BodyTooLarge { .. }) => {
            error::MESSAGE_PAYLOAD_TOO_LARGE
        }
        LifecycleProtocolError::Frame(_) => error::TRANSPORT_FAILED,
        LifecycleProtocolError::Handshake(error) => error.failure_code(),
        LifecycleProtocolError::ExtraFrame => error::HANDSHAKE_INVALID_MESSAGE,
    }
}

async fn drive_node_until<T>(node: &mut AukiP2pNode, future: impl Future<Output = T>) -> T {
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            _ = node.next_event() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        OfferPolicy, PeerRelationshipState, accept_lifecycle_streams, accept_offer_catalog_streams,
        exchange_peer_handshake_strict,
        protocols::{get_protocol, subscribe_protocol},
        serve_offer_catalog_response,
    };
    use auki_identity::Wallet;
    use auki_protocol::v1::{
        domain::DOMAIN_NONCE_LEN,
        frame::{decode_json_frame, decode_length, encode_json_frame},
        get::{GetRequest, GetResponse},
        message::{SPATIAL_MESSAGE_TYPE, SpatialMessage},
        offer::{OfferAccessMode, OfferStatus, PayloadDescriptor},
        subscribe::{SubscribeAccept, SubscribeRequest},
    };
    use futures::{AsyncRead, AsyncReadExt, AsyncWriteExt, StreamExt as _};
    use serde_json::{Value, json};
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
            vec![OfferAccessMode::Get, OfferAccessMode::Subscribe],
            PayloadDescriptor::create("auki.frame"),
            Vec::new(),
        )
        .expect("offer")
    }

    fn offer_report(peer_id: PeerId, domain_id: &str) -> OfferLoadReport {
        OfferLoadReport {
            peer_id,
            offers: vec![LoadedRemoteOffer {
                offer: offer(domain_id, "camera-main"),
                usable: true,
                unusable_reason: None,
            }],
            diagnostics: Vec::new(),
            generated_at: Some(ISSUED_AT.to_owned()),
        }
    }

    fn message_value(domain_id: &str, offer_id: &str, sequence: u64) -> Value {
        json!({
            "type": SPATIAL_MESSAGE_TYPE,
            "domain_id": domain_id,
            "offer_id": offer_id,
            "payload": {
                "type": "auki.frame",
                "bytes": "AQID",
                "json": {"ok": true},
            },
            "sequence": sequence.to_string(),
            "generated_at": ISSUED_AT,
        })
    }

    fn message(domain_id: &str, offer_id: &str, sequence: u64) -> SpatialMessage {
        SpatialMessage::from_value(message_value(domain_id, offer_id, sequence)).expect("message")
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

    async fn wait_for_peer_connected(node: &mut AukiNode, peer_id: PeerId) {
        timeout(Duration::from_secs(10), async {
            loop {
                if let Some(AukiNodeEvent::PeerConnected { peer_id: connected }) =
                    node.next_event(ISSUED_AT).await
                    && connected == peer_id
                {
                    return;
                }
            }
        })
        .await
        .expect("peer should connect");
    }

    async fn drain_node(mut node: AukiNode) {
        while node.next_event(ISSUED_AT).await.is_some() {}
    }

    async fn read_frame_bytes<S>(stream: &mut S, max_body_len: u64) -> Vec<u8>
    where
        S: AsyncRead + Unpin,
    {
        let mut prefix = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).await.expect("read prefix");
            prefix.push(byte[0]);
            if let Ok((body_len, _)) = decode_length(&prefix, max_body_len) {
                let mut body = vec![0u8; body_len as usize];
                stream.read_exact(&mut body).await.expect("read body");
                prefix.extend_from_slice(&body);
                return prefix;
            }
        }
    }

    fn decode_request_frame(frame: &[u8], max_body_len: u64) -> Value {
        let (value, consumed) = decode_json_frame(frame, max_body_len).expect("request frame");
        assert_eq!(consumed, frame.len());
        value
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

    #[tokio::test]
    async fn load_remote_offers_requires_authorized_relationship() {
        let remote_peer_id = identity(83).peer_id();
        let mut node =
            AukiNode::new(identity(84), AukiP2pNodeConfig::dial_only_development()).expect("node");

        let error = node
            .load_remote_offers(remote_peer_id, RemoteOfferLoadInput::new(), ISSUED_AT)
            .await
            .expect_err("relationship must be authorized before offer loading");

        assert!(matches!(
            error,
            AukiNodeError::OfferLoad(OfferLoadError::RelationshipNotAuthorized { peer_id })
                if peer_id == remote_peer_id
        ));
    }

    #[tokio::test]
    async fn load_remote_offers_fetches_catalog_over_libp2p_and_stores_report() {
        let listener_wallet = wallet(85);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration =
            DomainDeclaration::create(&listener_wallet, &[7; DOMAIN_NONCE_LEN], Some("served"))
                .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer_config = AukiP2pNodeConfig::dial_only_development();
        dialer_config.p2p.peer_admission = crate::PeerAdmissionConfig::AppPolicy;
        dialer_config.p2p.domain_access_policy = crate::DomainAccessPolicy::AppPolicy;
        dialer_config.p2p.offer_policy = OfferPolicy::AppPolicy;
        let mut dialer = AukiNode::new(identity(86), dialer_config).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let dialer_peer_id = dialer.peer_id();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let limits = dialer.node.config().p2p.limits;
        let mut lifecycle_incoming = accept_lifecycle_streams(&mut listener.node.stream_control())
            .expect("accept lifecycle streams");
        let mut catalog_incoming =
            accept_offer_catalog_streams(&mut listener.node.stream_control())
                .expect("accept offer catalog streams");
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .upsert_local_offer(offer(&domain_id, "camera-main"))
            .expect("local offer");
        let response = listener
            .local_offer_catalog_response(Some(ISSUED_AT))
            .expect("local offer catalog");
        let listener_handshake = listener.local_peer_handshake().expect("local handshake");

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        let listener_task = tokio::spawn(drain_node(listener));
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;

        let lifecycle_server = tokio::spawn(async move {
            let (peer_id, mut stream) = lifecycle_incoming.next().await.expect("lifecycle stream");
            assert_eq!(peer_id, dialer_peer_id);
            exchange_peer_handshake_strict(
                &mut stream,
                peer_id,
                &listener_handshake,
                limits.handshake_frame_body_bytes,
            )
            .await
            .expect("exchange lifecycle handshake");
        });

        let mut lifecycle_input = LifecycleInput::new();
        lifecycle_input.app_peer_authorization = Some(PeerAuthorization::Authorized);
        lifecycle_input.app_domain_access =
            LifecycleDomainAccess::AllowOnly(vec![domain_id.clone()]);
        let lifecycle = dialer
            .run_lifecycle(listener_peer_id, lifecycle_input, ISSUED_AT)
            .await
            .expect("run lifecycle");
        assert_eq!(lifecycle.accepted_served_domains.len(), 1);
        assert_eq!(lifecycle.accepted_served_domains[0].domain_id, domain_id);
        assert_eq!(
            dialer.relationship(listener_peer_id).unwrap().state,
            PeerRelationshipState::Authorized
        );
        lifecycle_server.await.expect("lifecycle server task");

        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let (peer_id, mut stream) =
                catalog_incoming.next().await.expect("offer catalog stream");
            assert_eq!(peer_id, dialer_peer_id);
            let request = serve_offer_catalog_response(&mut stream, &response, limits)
                .await
                .expect("serve offer catalog");
            assert_eq!(request.domain_ids, vec![server_domain_id]);
            assert_eq!(request.kinds, vec!["frame".to_owned()]);
        });

        let mut input = RemoteOfferLoadInput::new();
        input.domain_ids.push(domain_id.clone());
        input.kinds.push("frame".to_owned());
        input.supported_payload_types.push("auki.frame".to_owned());
        input.app_offer_policy = RemoteOfferAppPolicy::AllowAll;
        let report = dialer
            .load_remote_offers(listener_peer_id, input, ISSUED_AT)
            .await
            .expect("load remote offers");

        assert_eq!(report.peer_id, listener_peer_id);
        assert_eq!(report.offers.len(), 1);
        assert!(report.offers[0].usable);
        assert_eq!(report.offers[0].offer.domain_id, domain_id);
        assert!(dialer.remote_offer_report(listener_peer_id).is_some());
        assert_eq!(
            dialer.relationship(listener_peer_id).unwrap().state,
            PeerRelationshipState::Ready
        );

        server.await.expect("server task");
        listener_task.abort();
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
    async fn get_requires_loaded_remote_offers() {
        let remote_peer_id = identity(75).peer_id();
        let mut node =
            AukiNode::new(identity(76), AukiP2pNodeConfig::dial_only_development()).expect("node");

        let error = node
            .get(
                remote_peer_id,
                GetInput::new("noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs", "camera-main"),
                ISSUED_AT,
            )
            .await
            .expect_err("remote offers must be loaded before get");

        assert!(matches!(
            error,
            AukiNodeError::RemoteOffersNotLoaded { peer_id } if peer_id == remote_peer_id
        ));
    }

    #[tokio::test]
    async fn get_uses_loaded_remote_offers_and_hides_streams() {
        let domain_id = domain_declaration(77, 5).domain_id().unwrap().to_owned();
        let mut dialer =
            AukiNode::new(identity(78), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener =
            AukiNode::new(identity(79), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();
        let dialer_peer_id = dialer.peer_id();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let limits = dialer.node.config().p2p.limits;
        let mut incoming = listener
            .node
            .stream_control()
            .accept(get_protocol())
            .expect("accept get streams");
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        let listener_task = tokio::spawn(drain_node(listener));
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;
        dialer.upsert_remote_offer_report(offer_report(listener_peer_id, &domain_id));

        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let (peer_id, mut stream) = incoming.next().await.expect("get stream");
            assert_eq!(peer_id, dialer_peer_id);
            let request_frame =
                read_frame_bytes(&mut stream, limits.get_response_frame_body_bytes).await;
            let request = GetRequest::from_value(decode_request_frame(
                &request_frame,
                limits.get_response_frame_body_bytes,
            ))
            .expect("get request");
            assert_eq!(request.domain_id, server_domain_id);
            assert_eq!(request.offer_id, "camera-main");

            let response = GetResponse::success(message(&server_domain_id, "camera-main", 11));
            let response_frame =
                encode_json_frame(response.value(), limits.get_response_frame_body_bytes)
                    .expect("response frame");
            stream
                .write_all(&response_frame)
                .await
                .expect("write get response");
            stream.close().await.expect("close get stream");
        });

        let outcome = dialer
            .get(
                listener_peer_id,
                GetInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            )
            .await
            .expect("get should succeed");

        assert_eq!(outcome.message.sequence, Some(11));
        assert_eq!(
            dialer
                .relationship(listener_peer_id)
                .unwrap()
                .paths
                .last()
                .unwrap()
                .state
                .as_deref(),
            Some("succeeded")
        );
        server.await.expect("server task");
        listener_task.abort();
    }

    #[tokio::test]
    async fn subscribe_returns_high_level_subscription_and_reads_messages() {
        let domain_id = domain_declaration(80, 6).domain_id().unwrap().to_owned();
        let mut dialer =
            AukiNode::new(identity(81), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener =
            AukiNode::new(identity(82), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();
        let dialer_peer_id = dialer.peer_id();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let limits = dialer.node.config().p2p.limits;
        let mut incoming = listener
            .node
            .stream_control()
            .accept(subscribe_protocol())
            .expect("accept subscribe streams");
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        let listener_task = tokio::spawn(drain_node(listener));
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;
        dialer.upsert_remote_offer_report(offer_report(listener_peer_id, &domain_id));

        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let (peer_id, mut stream) = incoming.next().await.expect("subscribe stream");
            assert_eq!(peer_id, dialer_peer_id);
            let request_frame =
                read_frame_bytes(&mut stream, limits.subscribe_message_frame_body_bytes).await;
            let request = SubscribeRequest::from_value(decode_request_frame(
                &request_frame,
                limits.subscribe_message_frame_body_bytes,
            ))
            .expect("subscribe request");
            assert_eq!(request.domain_id, server_domain_id);
            assert_eq!(request.offer_id, "camera-main");

            let accept = SubscribeAccept::create(
                &server_domain_id,
                "camera-main",
                PayloadDescriptor::create("auki.frame"),
                Vec::new(),
                Some(1),
                Some(ISSUED_AT.to_owned()),
                None,
            )
            .expect("subscribe accept");
            let accept_frame =
                encode_json_frame(accept.value(), limits.subscribe_message_frame_body_bytes)
                    .expect("accept frame");
            stream.write_all(&accept_frame).await.expect("write accept");

            let data_frame = encode_json_frame(
                &message_value(&server_domain_id, "camera-main", 1),
                limits.subscribe_message_frame_body_bytes,
            )
            .expect("data frame");
            stream.write_all(&data_frame).await.expect("write data");
            stream.close().await.expect("close subscribe stream");
        });

        let mut subscription = dialer
            .subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            )
            .await
            .expect("subscribe should start");
        assert_eq!(subscription.peer_id(), listener_peer_id);
        assert_eq!(subscription.payload_type(), "auki.frame");

        let message = dialer
            .next_subscription_message(&mut subscription, "2026-05-26T12:01:00Z")
            .await
            .expect("subscription message");

        assert_eq!(message.sequence, Some(1));
        assert_eq!(subscription.last_sequence(), Some(1));
        assert_eq!(subscription.sequence_gap_count(), 0);
        assert_eq!(
            dialer
                .relationship(listener_peer_id)
                .unwrap()
                .paths
                .last()
                .unwrap()
                .last_sequence,
            Some(1)
        );
        server.await.expect("server task");
        listener_task.abort();
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
