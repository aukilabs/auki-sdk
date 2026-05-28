//! SDK-facing high-level node API.

use crate::publication::LocalOfferPublication;
use crate::{
    AppAllowedOffer, AppDomainAccess, AppOfferPolicy, AukiBrowserBootstrapRecord, AukiP2pConfig,
    AukiP2pNode, AukiP2pNodeConfig, AukiP2pNodeError, ConfiguredPeer, GetInput, GetOutcome,
    GetServeError, HandshakePolicyError, HandshakeValidationInput, HandshakeValidationResult,
    Libp2pOfferCatalogClient, Libp2pPathClient, Libp2pSubscription, LifecycleHandshakeExchange,
    LifecycleOpenStreamError, LifecycleProtocolError, LifecycleStreamDirection,
    LifecycleStreamGuard, LifecycleStreamGuardError, LoadedRemoteOffer, LocalPeerIdentity,
    OfferCatalogLoadState, OfferCatalogServeError, OfferLoadContext, OfferLoadError,
    OfferLoadReport, PathClientError, PathContext, PathOrchestrationError, PeerRelationship,
    PublicationMessageError, PublishOfferError, PublishOfferInput, PublishedByteFrame,
    PublishedOfferHandle, RelationshipFailureRecord, RelationshipFailureScope,
    RelationshipLoadedOffer, RelationshipRegistryReferenceStatus, RelationshipStatusBuildError,
    RelationshipStatusOptions, ServedPublishedSubscription, SubscribeInput, SubscribeServeError,
    accept_get_streams, accept_lifecycle_streams, accept_offer_catalog_streams,
    accept_subscribe_data_frame, accept_subscribe_streams, build_relationship_status_snapshot,
    close_subscribe_stream, encode_subscribe_data_frame, exchange_peer_handshake_strict,
    get_over_libp2p, load_remote_offers_over_libp2p, open_lifecycle_stream_once, read_get_request,
    read_subscribe_end, read_subscribe_request, serve_offer_catalog_response,
    subscribe_over_libp2p, validate_remote_handshake, write_encoded_subscribe_frame,
    write_get_response, write_subscribe_end, write_subscribe_start_result,
};
use auki_identity::PublicKey as WalletPublicKey;
use auki_protocol::v1::{
    authority::{DeclaredDomain, PeerAuthorization},
    domain::{DelegationScope, DomainDeclaration, DomainDelegation, DomainError},
    error,
    frame::FrameError,
    get::{GetRequest, GetResponse},
    handshake::{HandshakeError, PeerHandshake},
    message::{ErrorObject, SpatialMessage},
    offer::{
        Offer, OfferAccessMode, OfferCatalogPath, OfferCatalogPathError, OfferCatalogRequest,
        OfferCatalogRequestError, OfferCatalogResponse, OfferCatalogResponseError,
    },
    status::{LocalDomainRole, LocalDomainStatus, StatusSnapshot, StatusSnapshotParams},
    subscribe::{
        SubscribeAccept, SubscribeEnd, SubscribeEndReason, SubscribeReject, SubscribeRequest,
        SubscribeStartResult,
    },
};
use futures::{AsyncReadExt as _, StreamExt as _, io::WriteHalf};
use libp2p::{Multiaddr, PeerId};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    future::Future,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

const INBOUND_ACCEPT_BUFFER: usize = 1024;

/// High-level RFC-first runtime handle for SDK and app code.
pub struct AukiNode {
    node: AukiP2pNode,
    local_domains: BTreeMap<String, LocalDomainRegistration>,
    local_offers: BTreeMap<(String, String), Offer>,
    remote_offer_reports: BTreeMap<PeerId, OfferLoadReport>,
    relationships: BTreeMap<PeerId, PeerRelationship>,
    lifecycle_stream_guard: LifecycleStreamGuard,
    lifecycle_accept_task: Option<JoinHandle<()>>,
    offer_catalog_accept_task: Option<JoinHandle<()>>,
    get_accept_task: Option<JoinHandle<()>>,
    subscribe_accept_task: Option<JoinHandle<()>>,
    inbound_accept_tx: mpsc::Sender<AcceptedInboundStream>,
    inbound_accept_rx: mpsc::Receiver<AcceptedInboundStream>,
    get_providers: BTreeMap<(String, String), Box<dyn AukiGetProvider>>,
    subscribe_providers: BTreeMap<(String, String), Box<dyn AukiSubscribeProvider>>,
    local_publications: BTreeMap<(String, String), LocalOfferPublication>,
    pending_inbound: VecDeque<AcceptedInboundStream>,
}

enum AcceptedInboundStream {
    Lifecycle(PeerId, libp2p::Stream),
    OfferCatalog(PeerId, libp2p::Stream),
    Get(PeerId, libp2p::Stream),
    Subscribe(PeerId, libp2p::Stream),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcceptedInboundKind {
    Lifecycle,
    OfferCatalog,
    Get,
    Subscribe,
}

impl AcceptedInboundStream {
    fn kind(&self) -> AcceptedInboundKind {
        match self {
            Self::Lifecycle(_, _) => AcceptedInboundKind::Lifecycle,
            Self::OfferCatalog(_, _) => AcceptedInboundKind::OfferCatalog,
            Self::Get(_, _) => AcceptedInboundKind::Get,
            Self::Subscribe(_, _) => AcceptedInboundKind::Subscribe,
        }
    }
}

impl Drop for AukiNode {
    fn drop(&mut self) {
        for task in [
            &self.lifecycle_accept_task,
            &self.offer_catalog_accept_task,
            &self.get_accept_task,
            &self.subscribe_accept_task,
        ]
        .into_iter()
        .flatten()
        {
            task.abort();
        }
    }
}

fn spawn_inbound_accept_task(
    mut incoming: libp2p_stream::IncomingStreams,
    kind: AcceptedInboundKind,
    accepted_tx: mpsc::Sender<AcceptedInboundStream>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some((peer_id, stream)) = incoming.next().await {
            let accepted = match kind {
                AcceptedInboundKind::Lifecycle => AcceptedInboundStream::Lifecycle(peer_id, stream),
                AcceptedInboundKind::OfferCatalog => {
                    AcceptedInboundStream::OfferCatalog(peer_id, stream)
                }
                AcceptedInboundKind::Get => AcceptedInboundStream::Get(peer_id, stream),
                AcceptedInboundKind::Subscribe => AcceptedInboundStream::Subscribe(peer_id, stream),
            };
            if accepted_tx.send(accepted).await.is_err() {
                break;
            }
        }
    })
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

/// High-level accepted served subscription handle.
pub struct AukiServedSubscription {
    peer_id: PeerId,
    request: SubscribeRequest,
    accept: SubscribeAccept,
    stream: WriteHalf<libp2p::Stream>,
    consumer_end_rx: oneshot::Receiver<Result<SubscribeEnd, SubscribeServeError>>,
    consumer_end_task: tokio::task::JoinHandle<()>,
    max_message_bytes: Option<u64>,
    ended: bool,
}

/// Application provider for one local Get offer.
pub trait AukiGetProvider: Send {
    /// Produce one spatial message for a parsed Get request.
    fn get(
        &mut self,
        request: &GetRequest,
        now: &str,
    ) -> Result<SpatialMessage, AukiGetProviderError>;
}

/// Application Get-provider failure returned as a structured Get response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AukiGetProviderError {
    /// Stable failure code to return to the requester.
    pub code: String,
}

/// Result of serving one inbound Get stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedGet {
    /// Requesting peer id.
    pub peer_id: PeerId,
    /// Requested domain id, when the request parsed.
    pub domain_id: Option<String>,
    /// Requested offer id, when the request parsed.
    pub offer_id: Option<String>,
    /// Whether a successful Get response was served.
    pub success: bool,
    /// Stable failure code when a failed Get response was served.
    pub failure_code: Option<String>,
}

/// Result of serving one inbound offer-catalog request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedOfferCatalog {
    /// Requesting peer id.
    pub peer_id: PeerId,
    /// Requested domain filters. Empty means all visible domains.
    pub domain_ids: Vec<String>,
    /// Requested kind filters. Empty means all kinds.
    pub kinds: Vec<String>,
    /// Whether inline canonical registry entries were requested.
    pub include_inline_registry_entries: bool,
}

/// Application provider for one local Subscribe offer.
pub trait AukiSubscribeProvider: Send {
    /// Decide whether to accept one parsed Subscribe request.
    fn accept(
        &mut self,
        request: &SubscribeRequest,
        now: &str,
    ) -> Result<AukiSubscribeProviderAccept, AukiSubscribeProviderError>;
}

/// Application Subscribe-provider accept metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct AukiSubscribeProviderAccept {
    /// Optional first sequence value for the accepted stream.
    pub initial_sequence: Option<u64>,
    /// Optional accept generation timestamp.
    pub generated_at: Option<String>,
    /// Optional non-authoritative metadata.
    pub metadata: Option<Value>,
}

/// Application Subscribe-provider failure returned as a structured reject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AukiSubscribeProviderError {
    /// Stable failure code to return to the requester.
    pub code: String,
}

/// Result of serving one inbound Subscribe start stream.
pub struct ServedSubscribe {
    /// Requesting peer id.
    pub peer_id: PeerId,
    /// Requested domain id, when the request parsed.
    pub domain_id: Option<String>,
    /// Requested offer id, when the request parsed.
    pub offer_id: Option<String>,
    /// Whether a Subscribe accept was served.
    pub accepted: bool,
    /// Stable failure code when a reject was served.
    pub failure_code: Option<String>,
    subscription: Option<AukiServedSubscription>,
}

/// Result of serving one inbound SDK protocol stream.
pub enum AukiServedInbound {
    /// One lifecycle handshake was served.
    Lifecycle(HandshakeValidationResult),
    /// One offer-catalog request was served.
    OfferCatalog(ServedOfferCatalog),
    /// One Get request was served.
    Get(ServedGet),
    /// One Subscribe start request was served.
    Subscribe(ServedSubscribe),
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
    Node(Box<AukiP2pNodeError>),
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
    /// A high-level publication attempted to replace an existing local offer.
    LocalOfferAlreadyRegistered {
        /// Offer domain id.
        domain_id: String,
        /// Producer-scoped offer id.
        offer_id: String,
    },
    /// Local provider references an offer that is not registered locally.
    LocalOfferNotRegistered {
        /// Offer domain id.
        domain_id: String,
        /// Producer-scoped offer id.
        offer_id: String,
    },
    /// A published-offer helper referenced an unknown local publication.
    LocalPublicationNotRegistered {
        /// Offer domain id.
        domain_id: String,
        /// Producer-scoped offer id.
        offer_id: String,
    },
    /// High-level publication offer construction failed.
    PublishOffer(PublishOfferError),
    /// High-level publication message construction failed.
    PublicationMessage(PublicationMessageError),
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
    /// Inbound lifecycle stream registration failed.
    LifecycleAccept(libp2p_stream::AlreadyRegistered),
    /// Local lifecycle stream policy rejected an inbound stream.
    LifecycleGuard(LifecycleStreamGuardError),
    /// Remote handshake failed local policy validation.
    HandshakePolicy(HandshakePolicyError),
    /// Inbound offer-catalog stream registration failed.
    OfferCatalogAccept(libp2p_stream::AlreadyRegistered),
    /// Inbound offer-catalog serving failed.
    OfferCatalogServe(OfferCatalogServeError),
    /// Inbound Get stream registration failed.
    GetAccept(libp2p_stream::AlreadyRegistered),
    /// Inbound Get serving failed.
    GetServe(GetServeError),
    /// Inbound Subscribe stream registration failed.
    SubscribeAccept(libp2p_stream::AlreadyRegistered),
    /// Inbound Subscribe serving failed.
    SubscribeServe(SubscribeServeError),
    /// No loaded remote offers are available for the requested peer.
    RemoteOffersNotLoaded {
        /// Remote peer id.
        peer_id: PeerId,
    },
    /// Get or Subscribe path orchestration failed.
    Path(PathOrchestrationError),
}

impl From<AukiP2pNodeError> for AukiNodeError {
    fn from(error: AukiP2pNodeError) -> Self {
        Self::Node(Box::new(error))
    }
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

impl AukiServedSubscription {
    /// Requesting peer id.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Served domain id.
    pub fn domain_id(&self) -> &str {
        &self.request.domain_id
    }

    /// Served offer id.
    pub fn offer_id(&self) -> &str {
        &self.request.offer_id
    }

    /// Selected payload type for this served stream.
    pub fn payload_type(&self) -> &str {
        &self.accept.payload.payload_type
    }

    /// Optional first sequence value advertised in the accept.
    pub fn initial_sequence(&self) -> Option<u64> {
        self.accept.initial_sequence
    }

    /// Effective message byte limit for this served stream.
    pub fn max_message_bytes(&self) -> Option<u64> {
        self.max_message_bytes
    }

    /// Return a consumer-sent SubscribeEnd frame, if one has arrived.
    pub fn try_consumer_end(&mut self) -> Result<Option<SubscribeEnd>, SubscribeServeError> {
        if self.ended {
            return Ok(None);
        }

        match self.consumer_end_rx.try_recv() {
            Ok(Ok(end)) => {
                end.validate_for_offer(self.domain_id(), self.offer_id())
                    .map_err(SubscribeServeError::End)?;
                self.ended = true;
                self.consumer_end_task.abort();
                Ok(Some(end))
            }
            Ok(Err(error)) => Err(error),
            Err(oneshot::error::TryRecvError::Empty) => Ok(None),
            Err(oneshot::error::TryRecvError::Closed) => Ok(None),
        }
    }
}

impl Drop for AukiServedSubscription {
    fn drop(&mut self) {
        self.consumer_end_task.abort();
    }
}

impl AukiGetProviderError {
    /// Create a provider failure with a stable response code.
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }
}

impl<F> AukiGetProvider for F
where
    F: FnMut(&GetRequest, &str) -> Result<SpatialMessage, AukiGetProviderError> + Send,
{
    fn get(
        &mut self,
        request: &GetRequest,
        now: &str,
    ) -> Result<SpatialMessage, AukiGetProviderError> {
        self(request, now)
    }
}

impl AukiSubscribeProviderAccept {
    /// Create default Subscribe-provider accept metadata.
    pub fn new() -> Self {
        Self {
            initial_sequence: None,
            generated_at: None,
            metadata: None,
        }
    }
}

impl Default for AukiSubscribeProviderAccept {
    fn default() -> Self {
        Self::new()
    }
}

impl AukiSubscribeProviderError {
    /// Create a provider failure with a stable reject code.
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }
}

impl<F> AukiSubscribeProvider for F
where
    F: FnMut(
            &SubscribeRequest,
            &str,
        ) -> Result<AukiSubscribeProviderAccept, AukiSubscribeProviderError>
        + Send,
{
    fn accept(
        &mut self,
        request: &SubscribeRequest,
        now: &str,
    ) -> Result<AukiSubscribeProviderAccept, AukiSubscribeProviderError> {
        self(request, now)
    }
}

impl ServedSubscribe {
    /// Take the accepted served subscription handle, if the request was accepted.
    pub fn into_subscription(mut self) -> Option<AukiServedSubscription> {
        self.subscription.take()
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
        let node = AukiP2pNode::new(identity, config).map_err(AukiNodeError::from)?;
        let (inbound_accept_tx, inbound_accept_rx) = mpsc::channel(INBOUND_ACCEPT_BUFFER);
        let mut this = Self {
            node,
            local_domains: BTreeMap::new(),
            local_offers: BTreeMap::new(),
            remote_offer_reports: BTreeMap::new(),
            relationships: BTreeMap::new(),
            lifecycle_stream_guard: LifecycleStreamGuard::default(),
            lifecycle_accept_task: None,
            offer_catalog_accept_task: None,
            get_accept_task: None,
            subscribe_accept_task: None,
            inbound_accept_tx,
            inbound_accept_rx,
            get_providers: BTreeMap::new(),
            subscribe_providers: BTreeMap::new(),
            local_publications: BTreeMap::new(),
            pending_inbound: VecDeque::new(),
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

    /// Observed listen addresses with the local `/p2p/<peer-id>` suffix.
    pub fn observed_dialable_listen_addresses(&self) -> Vec<Multiaddr> {
        self.node.observed_dialable_listen_addresses()
    }

    /// Observed relay-server addresses with the local `/p2p/<peer-id>` suffix.
    pub fn observed_relay_server_addresses(&self) -> Vec<Multiaddr> {
        self.node.observed_relay_server_addresses()
    }

    /// Observed WebSocket relay-server addresses usable by browser peers.
    pub fn observed_browser_relay_server_addresses(&self) -> Vec<Multiaddr> {
        self.node.observed_browser_relay_server_addresses()
    }

    /// Connectivity-only bootstrap record for browser peers.
    pub fn browser_bootstrap_record(&self) -> AukiBrowserBootstrapRecord {
        self.node.browser_bootstrap_record()
    }

    /// Wait for configured listeners to appear in the browser bootstrap record.
    ///
    /// This consumes and applies node events while waiting, so relationship and
    /// observed-listen-address state stay current for later diagnostics.
    pub async fn wait_for_browser_bootstrap_record(
        &mut self,
        max_wait: Duration,
        observed_at: &str,
    ) -> AukiBrowserBootstrapRecord {
        let expected_addresses = self.configured_listen_addresses().len();
        let started_at = Instant::now();
        while self.observed_listen_addresses().len() < expected_addresses
            && started_at.elapsed() < max_wait
        {
            let _ = tokio::time::timeout(Duration::from_millis(500), self.next_event(observed_at))
                .await;
        }
        self.browser_bootstrap_record()
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
            .map_err(AukiNodeError::from)?;
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

    /// Add or replace the local provider for a registered Get offer.
    pub fn upsert_get_provider<P>(
        &mut self,
        domain_id: impl Into<String>,
        offer_id: impl Into<String>,
        provider: P,
    ) -> Result<(), AukiNodeError>
    where
        P: AukiGetProvider + 'static,
    {
        let domain_id = domain_id.into();
        let offer_id = offer_id.into();
        if !self
            .local_offers
            .contains_key(&(domain_id.clone(), offer_id.clone()))
        {
            return Err(AukiNodeError::LocalOfferNotRegistered {
                domain_id,
                offer_id,
            });
        }

        self.get_providers
            .insert((domain_id, offer_id), Box::new(provider));
        Ok(())
    }

    /// Add or replace the local provider for a registered Subscribe offer.
    pub fn upsert_subscribe_provider<P>(
        &mut self,
        domain_id: impl Into<String>,
        offer_id: impl Into<String>,
        provider: P,
    ) -> Result<(), AukiNodeError>
    where
        P: AukiSubscribeProvider + 'static,
    {
        let domain_id = domain_id.into();
        let offer_id = offer_id.into();
        if !self
            .local_offers
            .contains_key(&(domain_id.clone(), offer_id.clone()))
        {
            return Err(AukiNodeError::LocalOfferNotRegistered {
                domain_id,
                offer_id,
            });
        }

        self.subscribe_providers
            .insert((domain_id, offer_id), Box::new(provider));
        Ok(())
    }

    /// Publish a generic local byte source as a Subscribe offer.
    pub fn publish_offer(
        &mut self,
        input: PublishOfferInput,
    ) -> Result<PublishedOfferHandle, AukiNodeError> {
        let key = input.key();
        if self.local_offers.contains_key(&key) || self.local_publications.contains_key(&key) {
            return Err(AukiNodeError::LocalOfferAlreadyRegistered {
                domain_id: key.0,
                offer_id: key.1,
            });
        }

        let publication = input
            .into_publication()
            .map_err(AukiNodeError::PublishOffer)?;
        let offer = publication.offer().clone();
        let handle = PublishedOfferHandle::new(offer.domain_id.clone(), offer.offer_id.clone());

        self.upsert_local_offer(offer)?;
        self.upsert_subscribe_provider(
            handle.domain_id().to_owned(),
            handle.offer_id().to_owned(),
            |_request: &SubscribeRequest, now: &str| {
                Ok(AukiSubscribeProviderAccept {
                    initial_sequence: None,
                    generated_at: Some(now.to_owned()),
                    metadata: None,
                })
            },
        )?;
        self.local_publications.insert(handle.key(), publication);
        Ok(handle)
    }

    /// Withdraw a generic local publication created by [`Self::publish_offer`].
    pub fn unpublish_offer(&mut self, handle: &PublishedOfferHandle) -> Result<(), AukiNodeError> {
        let key = handle.key();
        if self.local_publications.remove(&key).is_none() {
            return Err(AukiNodeError::LocalPublicationNotRegistered {
                domain_id: key.0,
                offer_id: key.1,
            });
        }
        self.local_offers.remove(&key);
        self.subscribe_providers.remove(&key);
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

        self.validate_lifecycle_exchange(exchange, input, &p2p_config, now)
    }

    /// Serve one inbound v1 lifecycle exchange and authorize the remote peer.
    pub async fn serve_next_lifecycle(
        &mut self,
        input: LifecycleInput,
        now: &str,
    ) -> Result<Option<HandshakeValidationResult>, AukiNodeError> {
        let Some(AcceptedInboundStream::Lifecycle(peer_id, mut stream)) = self
            .accept_next_matching_inbound_stream(AcceptedInboundKind::Lifecycle)
            .await?
        else {
            return Ok(None);
        };

        self.serve_accepted_lifecycle(peer_id, &mut stream, input, now)
            .await
            .map(Some)
    }

    /// Serve one ready inbound SDK protocol stream from the shared accept loop.
    pub async fn serve_next_inbound(
        &mut self,
        lifecycle_input: LifecycleInput,
        now: &str,
    ) -> Result<Option<AukiServedInbound>, AukiNodeError> {
        let Some(accepted) = self.accept_next_inbound_stream().await? else {
            return Ok(None);
        };

        match accepted {
            AcceptedInboundStream::Lifecycle(peer_id, mut stream) => self
                .serve_accepted_lifecycle(peer_id, &mut stream, lifecycle_input, now)
                .await
                .map(|served| Some(AukiServedInbound::Lifecycle(served))),
            AcceptedInboundStream::OfferCatalog(peer_id, mut stream) => self
                .serve_accepted_offer_catalog(peer_id, &mut stream, Some(now))
                .await
                .map(|served| Some(AukiServedInbound::OfferCatalog(served))),
            AcceptedInboundStream::Get(peer_id, mut stream) => self
                .serve_accepted_get(peer_id, &mut stream, now)
                .await
                .map(|served| Some(AukiServedInbound::Get(served))),
            AcceptedInboundStream::Subscribe(peer_id, stream) => self
                .serve_accepted_subscribe(peer_id, stream, now)
                .await
                .map(|served| Some(AukiServedInbound::Subscribe(served))),
        }
    }

    async fn serve_accepted_lifecycle(
        &mut self,
        peer_id: PeerId,
        stream: &mut libp2p::Stream,
        input: LifecycleInput,
        now: &str,
    ) -> Result<HandshakeValidationResult, AukiNodeError> {
        let local_handshake = self.local_peer_handshake()?;
        let p2p_config = self.node.config().p2p.clone();
        self.lifecycle_stream_guard
            .begin(peer_id, LifecycleStreamDirection::Inbound)
            .map_err(AukiNodeError::LifecycleGuard)?;
        let exchange = exchange_peer_handshake_strict(
            stream,
            peer_id,
            &local_handshake,
            p2p_config.limits.handshake_frame_body_bytes,
        );
        let exchange = match self.drive_node_until_accepting_inbound(exchange).await? {
            Ok(exchange) => exchange,
            Err(error) => {
                self.lifecycle_stream_guard.fail(peer_id);
                self.record_lifecycle_failure(peer_id, &error, now);
                return Err(AukiNodeError::Lifecycle(error));
            }
        };

        self.validate_lifecycle_exchange(exchange, input, &p2p_config, now)
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

    /// Serve one inbound offer-catalog request from registered local offers.
    pub async fn serve_next_offer_catalog(
        &mut self,
        generated_at: Option<&str>,
    ) -> Result<Option<ServedOfferCatalog>, AukiNodeError> {
        let Some(AcceptedInboundStream::OfferCatalog(peer_id, mut stream)) = self
            .accept_next_matching_inbound_stream(AcceptedInboundKind::OfferCatalog)
            .await?
        else {
            return Ok(None);
        };

        self.serve_accepted_offer_catalog(peer_id, &mut stream, generated_at)
            .await
            .map(Some)
    }

    async fn serve_accepted_offer_catalog(
        &mut self,
        peer_id: PeerId,
        stream: &mut libp2p::Stream,
        generated_at: Option<&str>,
    ) -> Result<ServedOfferCatalog, AukiNodeError> {
        let response = self.local_offer_catalog_response(generated_at)?;
        let limits = self.node.config().p2p.limits;
        let request = self
            .drive_node_until_accepting_inbound(serve_offer_catalog_response(
                stream, &response, limits,
            ))
            .await?
            .map_err(AukiNodeError::OfferCatalogServe)?;
        Ok(ServedOfferCatalog {
            peer_id,
            domain_ids: request.domain_ids,
            kinds: request.kinds,
            include_inline_registry_entries: request.include_inline_registry_entries,
        })
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
            .map_err(AukiNodeError::from)?;
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

    /// Serve one inbound Get stream with a registered local provider.
    pub async fn serve_next_get(&mut self, now: &str) -> Result<Option<ServedGet>, AukiNodeError> {
        let Some(AcceptedInboundStream::Get(peer_id, mut stream)) = self
            .accept_next_matching_inbound_stream(AcceptedInboundKind::Get)
            .await?
        else {
            return Ok(None);
        };

        self.serve_accepted_get(peer_id, &mut stream, now)
            .await
            .map(Some)
    }

    async fn serve_accepted_get(
        &mut self,
        peer_id: PeerId,
        stream: &mut libp2p::Stream,
        now: &str,
    ) -> Result<ServedGet, AukiNodeError> {
        let max_body_len = self.node.config().p2p.limits.get_response_frame_body_bytes;
        let request = match self
            .drive_node_until_accepting_inbound(read_get_request(stream, max_body_len))
            .await?
        {
            Ok(request) => request,
            Err(GetServeError::Request(error)) => {
                let response = get_failure_response(error.failure_code());
                let served = ServedGet {
                    peer_id,
                    domain_id: None,
                    offer_id: None,
                    success: false,
                    failure_code: Some(error.failure_code().to_owned()),
                };
                self.drive_node_until_accepting_inbound(write_get_response(
                    stream,
                    &response,
                    max_body_len,
                ))
                .await?
                .map_err(AukiNodeError::GetServe)?;
                return Ok(served);
            }
            Err(error) => return Err(AukiNodeError::GetServe(error)),
        };

        let (response, served) = self.local_get_response(peer_id, &request, now);
        self.drive_node_until_accepting_inbound(write_get_response(
            stream,
            &response,
            max_body_len,
        ))
        .await?
        .map_err(AukiNodeError::GetServe)?;
        Ok(served)
    }

    /// Serve one inbound Subscribe start stream with a registered local provider.
    pub async fn serve_next_subscribe(
        &mut self,
        now: &str,
    ) -> Result<Option<ServedSubscribe>, AukiNodeError> {
        let Some(AcceptedInboundStream::Subscribe(peer_id, stream)) = self
            .accept_next_matching_inbound_stream(AcceptedInboundKind::Subscribe)
            .await?
        else {
            return Ok(None);
        };

        self.serve_accepted_subscribe(peer_id, stream, now)
            .await
            .map(Some)
    }

    async fn serve_accepted_subscribe(
        &mut self,
        peer_id: PeerId,
        mut stream: libp2p::Stream,
        now: &str,
    ) -> Result<ServedSubscribe, AukiNodeError> {
        let max_body_len = self
            .node
            .config()
            .p2p
            .limits
            .subscribe_message_frame_body_bytes;
        let request = match self
            .drive_node_until_accepting_inbound(read_subscribe_request(&mut stream, max_body_len))
            .await?
        {
            Ok(request) => request,
            Err(SubscribeServeError::Request(error)) => {
                let (start, served) =
                    subscribe_reject_result(peer_id, None, None, error.failure_code());
                self.drive_node_until_accepting_inbound(write_subscribe_start_result(
                    &mut stream,
                    &start,
                    max_body_len,
                ))
                .await?
                .map_err(AukiNodeError::SubscribeServe)?;
                self.drive_node_until_accepting_inbound(close_subscribe_stream(&mut stream))
                    .await?
                    .map_err(AukiNodeError::SubscribeServe)?;
                return Ok(served);
            }
            Err(error) => return Err(AukiNodeError::SubscribeServe(error)),
        };

        let start = self.local_subscribe_start(peer_id, &request, now);
        self.drive_node_until_accepting_inbound(write_subscribe_start_result(
            &mut stream,
            &start.response,
            max_body_len,
        ))
        .await?
        .map_err(AukiNodeError::SubscribeServe)?;

        let mut served = start.served;
        if let Some(accepted) = start.accepted {
            let (mut read_stream, write_stream) = stream.split();
            let (consumer_end_tx, consumer_end_rx) = oneshot::channel();
            let consumer_end_task = tokio::spawn(async move {
                let result = read_subscribe_end(&mut read_stream, max_body_len).await;
                let _ = consumer_end_tx.send(result);
            });
            served.subscription = Some(AukiServedSubscription {
                peer_id,
                request,
                accept: accepted.accept,
                stream: write_stream,
                consumer_end_rx,
                consumer_end_task,
                max_message_bytes: accepted.max_message_bytes,
                ended: false,
            });
        } else {
            self.drive_node_until_accepting_inbound(close_subscribe_stream(&mut stream))
                .await?
                .map_err(AukiNodeError::SubscribeServe)?;
        }

        Ok(served)
    }

    /// Serve one inbound Subscribe stream for a generic published byte source.
    pub async fn serve_next_published_subscription(
        &mut self,
        now: &str,
    ) -> Result<Option<ServedPublishedSubscription>, AukiNodeError> {
        let Some(served) = self.serve_next_subscribe(now).await? else {
            return Ok(None);
        };
        if !served.accepted {
            return Ok(Some(ServedPublishedSubscription::rejected(
                served.peer_id,
                served.domain_id,
                served.offer_id,
                served.failure_code,
            )));
        }

        let peer_id = served.peer_id;
        let mut subscription = served
            .into_subscription()
            .ok_or(AukiNodeError::SubscribeServe(
                SubscribeServeError::AlreadyEnded,
            ))?;
        let domain_id = subscription.domain_id().to_owned();
        let offer_id = subscription.offer_id().to_owned();
        let mut source = self.open_publication_source(&domain_id, &offer_id)?;
        let mut messages_sent = 0_u64;

        while let Some(chunk) = source.next().await {
            if let Some(end) = subscription
                .try_consumer_end()
                .map_err(AukiNodeError::SubscribeServe)?
            {
                return Ok(Some(ServedPublishedSubscription::accepted(
                    peer_id,
                    domain_id,
                    offer_id,
                    messages_sent,
                    end.reason,
                )));
            }
            let message = self.next_publication_message(&domain_id, &offer_id, chunk, now)?;
            self.send_served_subscription_message(&mut subscription, &message)
                .await?;
            messages_sent = messages_sent.saturating_add(1);
        }

        let end_reason = SubscribeEndReason::Complete;
        self.end_served_subscription(subscription, end_reason, None, None)
            .await?;

        Ok(Some(ServedPublishedSubscription::accepted(
            peer_id,
            domain_id,
            offer_id,
            messages_sent,
            end_reason,
        )))
    }

    /// Send one spatial message on an accepted served Subscribe stream.
    pub async fn send_served_subscription_message(
        &mut self,
        subscription: &mut AukiServedSubscription,
        message: &SpatialMessage,
    ) -> Result<(), AukiNodeError> {
        if subscription.ended {
            return Err(AukiNodeError::SubscribeServe(
                SubscribeServeError::AlreadyEnded,
            ));
        }

        let max_body_len = self
            .node
            .config()
            .p2p
            .limits
            .subscribe_message_frame_body_bytes;
        let frame = encode_subscribe_data_frame(message, max_body_len)
            .map_err(AukiNodeError::SubscribeServe)?;
        subscription
            .accept
            .validate_data_message_with_body_len(
                message,
                frame.body_len(),
                subscription.max_message_bytes,
            )
            .map_err(|error| AukiNodeError::SubscribeServe(SubscribeServeError::Data(error)))?;

        self.drive_node_until_accepting_inbound(write_encoded_subscribe_frame(
            &mut subscription.stream,
            &frame,
        ))
        .await?
        .map_err(AukiNodeError::SubscribeServe)
    }

    /// End an accepted served Subscribe stream and close it.
    pub async fn end_served_subscription(
        &mut self,
        mut subscription: AukiServedSubscription,
        reason: SubscribeEndReason,
        error_code: Option<String>,
        retryable: Option<bool>,
    ) -> Result<(), AukiNodeError> {
        if subscription.ended {
            return Err(AukiNodeError::SubscribeServe(
                SubscribeServeError::AlreadyEnded,
            ));
        }

        let error = error_code.map(ErrorObject::create);
        let end = SubscribeEnd::create(
            subscription.domain_id(),
            subscription.offer_id(),
            reason,
            error,
            retryable,
            None,
        )
        .map_err(|error| AukiNodeError::SubscribeServe(SubscribeServeError::End(error)))?;
        end.validate_for_offer(subscription.domain_id(), subscription.offer_id())
            .map_err(|error| AukiNodeError::SubscribeServe(SubscribeServeError::End(error)))?;

        let max_body_len = self
            .node
            .config()
            .p2p
            .limits
            .subscribe_message_frame_body_bytes;
        self.drive_node_until_accepting_inbound(write_subscribe_end(
            &mut subscription.stream,
            &end,
            max_body_len,
        ))
        .await?
        .map_err(AukiNodeError::SubscribeServe)?;
        subscription.ended = true;
        Ok(())
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
        if let Some(event) = self.node.pop_pending_event() {
            return Some(self.apply_p2p_event(event, observed_at));
        }

        let event = self.node.next_event().await?;
        Some(self.apply_p2p_event(event, observed_at))
    }

    /// Build an in-process diagnostic status snapshot.
    ///
    /// This first applies transport events buffered while protocol operations
    /// were being served, so diagnostics reflect recent connects/disconnects.
    pub fn status_snapshot(&mut self, generated_at: &str) -> Result<StatusSnapshot, AukiNodeError> {
        self.drain_pending_p2p_events(generated_at);
        let local_peer = self.node.local_peer_status().map_err(AukiNodeError::from)?;
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
        StatusSnapshot::create(StatusSnapshotParams {
            generated_at: generated_at.to_owned(),
            local_peer,
            local_domains,
            remote_peers: relationship_status.remote_peers,
            active_paths: relationship_status.active_paths,
            last_failures: relationship_status.last_failures,
            discovery: relationship_status.discovery,
            metadata: relationship_status.metadata,
        })
        .map_err(|error| AukiNodeError::Status(RelationshipStatusBuildError::Status(error)))
    }

    fn relationship_mut(&mut self, peer_id: PeerId) -> &mut PeerRelationship {
        self.relationships
            .entry(peer_id)
            .or_insert_with(|| PeerRelationship::new(peer_id))
    }

    fn drain_pending_p2p_events(&mut self, observed_at: &str) {
        while let Some(event) = self.node.pop_pending_event() {
            self.apply_p2p_event(event, observed_at);
        }
    }

    fn apply_p2p_event(&mut self, event: crate::AukiP2pEvent, observed_at: &str) -> AukiNodeEvent {
        let failure_cap = self.node.config().p2p.limits.retained_status_failures;
        match event {
            crate::AukiP2pEvent::Listening { address } => AukiNodeEvent::Listening { address },
            crate::AukiP2pEvent::ConnectionEstablished {
                peer_id,
                active_paths,
            } => {
                self.relationship_mut(peer_id)
                    .connected_with_paths(active_paths);
                AukiNodeEvent::PeerConnected { peer_id }
            }
            crate::AukiP2pEvent::DuplicateConnectionClosed { peer_id } => {
                AukiNodeEvent::PeerDuplicateConnectionClosed { peer_id }
            }
            crate::AukiP2pEvent::ConnectionClosed {
                peer_id,
                active_connections,
                active_paths,
            } => {
                if active_connections == 0 {
                    self.lifecycle_stream_guard.reset(peer_id);
                    self.relationship_mut(peer_id)
                        .lost(observed_at.to_owned(), failure_cap);
                } else {
                    self.relationship_mut(peer_id)
                        .set_transport_paths(active_paths);
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
        }
    }

    fn ensure_get_incoming(&mut self) -> Result<(), AukiNodeError> {
        if self.get_accept_task.is_some() {
            return Ok(());
        }

        let mut control = self.node.stream_control();
        let incoming = accept_get_streams(&mut control).map_err(AukiNodeError::GetAccept)?;
        self.get_accept_task = Some(spawn_inbound_accept_task(
            incoming,
            AcceptedInboundKind::Get,
            self.inbound_accept_tx.clone(),
        ));
        Ok(())
    }

    fn ensure_lifecycle_incoming(&mut self) -> Result<(), AukiNodeError> {
        if self.lifecycle_accept_task.is_some() {
            return Ok(());
        }

        let mut control = self.node.stream_control();
        let incoming =
            accept_lifecycle_streams(&mut control).map_err(AukiNodeError::LifecycleAccept)?;
        self.lifecycle_accept_task = Some(spawn_inbound_accept_task(
            incoming,
            AcceptedInboundKind::Lifecycle,
            self.inbound_accept_tx.clone(),
        ));
        Ok(())
    }

    fn ensure_offer_catalog_incoming(&mut self) -> Result<(), AukiNodeError> {
        if self.offer_catalog_accept_task.is_some() {
            return Ok(());
        }

        let mut control = self.node.stream_control();
        let incoming = accept_offer_catalog_streams(&mut control)
            .map_err(AukiNodeError::OfferCatalogAccept)?;
        self.offer_catalog_accept_task = Some(spawn_inbound_accept_task(
            incoming,
            AcceptedInboundKind::OfferCatalog,
            self.inbound_accept_tx.clone(),
        ));
        Ok(())
    }

    fn ensure_subscribe_incoming(&mut self) -> Result<(), AukiNodeError> {
        if self.subscribe_accept_task.is_some() {
            return Ok(());
        }

        let mut control = self.node.stream_control();
        let incoming =
            accept_subscribe_streams(&mut control).map_err(AukiNodeError::SubscribeAccept)?;
        self.subscribe_accept_task = Some(spawn_inbound_accept_task(
            incoming,
            AcceptedInboundKind::Subscribe,
            self.inbound_accept_tx.clone(),
        ));
        Ok(())
    }

    fn ensure_inbound_acceptors(&mut self) -> Result<(), AukiNodeError> {
        self.ensure_lifecycle_incoming()?;
        self.ensure_offer_catalog_incoming()?;
        self.ensure_get_incoming()?;
        self.ensure_subscribe_incoming()
    }

    fn take_pending_inbound(&mut self, kind: AcceptedInboundKind) -> Option<AcceptedInboundStream> {
        let index = self
            .pending_inbound
            .iter()
            .position(|accepted| accepted.kind() == kind)?;
        self.pending_inbound.remove(index)
    }

    async fn accept_next_matching_inbound_stream(
        &mut self,
        kind: AcceptedInboundKind,
    ) -> Result<Option<AcceptedInboundStream>, AukiNodeError> {
        if let Some(accepted) = self.take_pending_inbound(kind) {
            return Ok(Some(accepted));
        }

        loop {
            let Some(accepted) = self.accept_live_inbound_stream().await? else {
                return Ok(None);
            };
            if accepted.kind() == kind {
                return Ok(Some(accepted));
            }
            self.pending_inbound.push_back(accepted);
        }
    }

    async fn accept_next_inbound_stream(
        &mut self,
    ) -> Result<Option<AcceptedInboundStream>, AukiNodeError> {
        if let Some(accepted) = self.pending_inbound.pop_front() {
            return Ok(Some(accepted));
        }

        self.accept_live_inbound_stream().await
    }

    pub(crate) fn has_pending_inbound_streams(&self) -> bool {
        !self.pending_inbound.is_empty() || !self.inbound_accept_rx.is_empty()
    }

    async fn accept_live_inbound_stream(
        &mut self,
    ) -> Result<Option<AcceptedInboundStream>, AukiNodeError> {
        self.ensure_inbound_acceptors()?;
        loop {
            tokio::select! {
                biased;
                accepted = self.inbound_accept_rx.recv() => {
                    return Ok(accepted);
                }
                event = self.node.next_event() => {
                    let Some(event) = event else {
                        return Ok(None);
                    };
                    self.node.push_pending_event(event);
                }
            }
        }
    }

    async fn drive_node_until_accepting_inbound<T>(
        &mut self,
        future: impl Future<Output = T>,
    ) -> Result<T, AukiNodeError> {
        tokio::pin!(future);
        loop {
            tokio::select! {
                biased;
                accepted = self.accept_live_inbound_stream() => {
                    if let Some(accepted) = accepted? {
                        self.pending_inbound.push_back(accepted);
                    }
                }
                result = &mut future => return Ok(result),
            }
        }
    }

    pub(crate) fn has_local_publication(&self, domain_id: &str, offer_id: &str) -> bool {
        self.local_publications
            .contains_key(&(domain_id.to_owned(), offer_id.to_owned()))
    }

    pub(crate) fn local_publication_backpressure_policy(
        &self,
        domain_id: &str,
        offer_id: &str,
    ) -> Option<crate::AukiSubscriptionBackpressurePolicy> {
        self.local_publications
            .get(&(domain_id.to_owned(), offer_id.to_owned()))
            .map(LocalOfferPublication::backpressure_policy)
    }

    pub(crate) fn open_publication_source(
        &mut self,
        domain_id: &str,
        offer_id: &str,
    ) -> Result<crate::PublishedByteSource, AukiNodeError> {
        self.local_publications
            .get_mut(&(domain_id.to_owned(), offer_id.to_owned()))
            .map(LocalOfferPublication::open_source)
            .ok_or_else(|| AukiNodeError::LocalPublicationNotRegistered {
                domain_id: domain_id.to_owned(),
                offer_id: offer_id.to_owned(),
            })
    }

    pub(crate) fn next_publication_message(
        &mut self,
        domain_id: &str,
        offer_id: &str,
        frame: PublishedByteFrame,
        generated_at: &str,
    ) -> Result<SpatialMessage, AukiNodeError> {
        self.local_publications
            .get_mut(&(domain_id.to_owned(), offer_id.to_owned()))
            .ok_or_else(|| AukiNodeError::LocalPublicationNotRegistered {
                domain_id: domain_id.to_owned(),
                offer_id: offer_id.to_owned(),
            })?
            .next_message(frame, Some(generated_at))
            .map_err(AukiNodeError::PublicationMessage)
    }

    fn validate_lifecycle_exchange(
        &mut self,
        exchange: LifecycleHandshakeExchange,
        input: LifecycleInput,
        p2p_config: &AukiP2pConfig,
        now: &str,
    ) -> Result<HandshakeValidationResult, AukiNodeError> {
        let required_material_types = input
            .required_authorization_material_types
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let allowed_domain_ids = input.app_domain_access.allowed_domain_refs();
        let mut validation_input = HandshakeValidationInput::new(
            &exchange.authenticated_peer_id,
            &exchange.handshake,
            p2p_config,
            now,
        );
        validation_input.app_peer_authorization = input.app_peer_authorization;
        validation_input.app_domain_access = input
            .app_domain_access
            .as_app_domain_access(&allowed_domain_ids);
        validation_input.required_authorization_material_types = &required_material_types;

        match validate_remote_handshake(validation_input) {
            Ok(result) => {
                self.lifecycle_stream_guard
                    .complete(exchange.authenticated_peer_id);
                self.relationship_mut(exchange.authenticated_peer_id)
                    .handshake_accepted(result.clone());
                Ok(result)
            }
            Err(error) => {
                self.lifecycle_stream_guard
                    .fail(exchange.authenticated_peer_id);
                let failure_cap = self.node.config().p2p.limits.retained_status_failures;
                self.relationship_mut(exchange.authenticated_peer_id)
                    .handshake_failed(&error, now.to_owned(), failure_cap);
                Err(AukiNodeError::HandshakePolicy(error))
            }
        }
    }

    fn local_get_response(
        &mut self,
        peer_id: PeerId,
        request: &GetRequest,
        now: &str,
    ) -> (GetResponse, ServedGet) {
        let domain_id = request.domain_id.clone();
        let offer_id = request.offer_id.clone();
        let key = (domain_id.clone(), offer_id.clone());
        let (payload_type, supports_get) = match self.local_offers.get(&key) {
            Some(offer) => (
                offer.payload.payload_type.clone(),
                offer.access_modes.contains(&OfferAccessMode::Get),
            ),
            None => {
                return get_failure_result(
                    peer_id,
                    Some(domain_id),
                    Some(offer_id),
                    error::OFFER_UNKNOWN_OFFER,
                );
            }
        };

        if !supports_get {
            return get_failure_result(
                peer_id,
                Some(domain_id),
                Some(offer_id),
                error::OFFER_UNSUPPORTED_ACCESS_MODE,
            );
        }
        if !request.accepts_payload_type(&payload_type) {
            return get_failure_result(
                peer_id,
                Some(domain_id),
                Some(offer_id),
                error::OFFER_UNSUPPORTED_PAYLOAD_TYPE,
            );
        }

        let Some(provider) = self.get_providers.get_mut(&key) else {
            return get_failure_result(
                peer_id,
                Some(domain_id),
                Some(offer_id),
                error::OFFER_TEMPORARILY_UNAVAILABLE,
            );
        };
        let message = match provider.get(request, now) {
            Ok(message) => message,
            Err(error) => {
                return get_failure_result(peer_id, Some(domain_id), Some(offer_id), error.code);
            }
        };
        let response = GetResponse::success(message);
        if let Err(error) = response.validate_success_for_request(request, &payload_type) {
            return get_failure_result(
                peer_id,
                Some(domain_id),
                Some(offer_id),
                error.failure_code(),
            );
        }

        (
            response,
            ServedGet {
                peer_id,
                domain_id: Some(domain_id),
                offer_id: Some(offer_id),
                success: true,
                failure_code: None,
            },
        )
    }

    fn local_subscribe_start(
        &mut self,
        peer_id: PeerId,
        request: &SubscribeRequest,
        now: &str,
    ) -> LocalSubscribeStart {
        let domain_id = request.domain_id.clone();
        let offer_id = request.offer_id.clone();
        let key = (domain_id.clone(), offer_id.clone());
        let (payload, registry_refs, supports_subscribe) = match self.local_offers.get(&key) {
            Some(offer) => (
                offer.payload.clone(),
                offer.registry_refs.clone(),
                offer.access_modes.contains(&OfferAccessMode::Subscribe),
            ),
            None => {
                return subscribe_reject_start(
                    peer_id,
                    Some(domain_id),
                    Some(offer_id),
                    error::OFFER_UNKNOWN_OFFER,
                );
            }
        };

        if !supports_subscribe {
            return subscribe_reject_start(
                peer_id,
                Some(domain_id),
                Some(offer_id),
                error::OFFER_UNSUPPORTED_ACCESS_MODE,
            );
        }
        if !request.accepts_payload_type(&payload.payload_type) {
            return subscribe_reject_start(
                peer_id,
                Some(domain_id),
                Some(offer_id),
                error::OFFER_UNSUPPORTED_PAYLOAD_TYPE,
            );
        }

        let Some(provider) = self.subscribe_providers.get_mut(&key) else {
            return subscribe_reject_start(
                peer_id,
                Some(domain_id),
                Some(offer_id),
                error::OFFER_TEMPORARILY_UNAVAILABLE,
            );
        };
        let provider_accept = match provider.accept(request, now) {
            Ok(accept) => accept,
            Err(error) => {
                return subscribe_reject_start(
                    peer_id,
                    Some(domain_id),
                    Some(offer_id),
                    error.code,
                );
            }
        };
        let accept = match SubscribeAccept::create(
            domain_id.clone(),
            offer_id.clone(),
            payload,
            registry_refs,
            provider_accept.initial_sequence,
            provider_accept.generated_at,
            provider_accept.metadata,
        ) {
            Ok(accept) => accept,
            Err(error) => {
                return subscribe_reject_start(
                    peer_id,
                    Some(domain_id),
                    Some(offer_id),
                    error.failure_code(),
                );
            }
        };
        if let Err(error) = accept.validate_for_request(request) {
            return subscribe_reject_start(
                peer_id,
                Some(domain_id),
                Some(offer_id),
                error.failure_code(),
            );
        }

        let max_message_bytes = Some(
            request.max_message_bytes.map_or(
                self.node
                    .config()
                    .p2p
                    .limits
                    .subscribe_message_frame_body_bytes,
                |requested| {
                    requested.min(
                        self.node
                            .config()
                            .p2p
                            .limits
                            .subscribe_message_frame_body_bytes,
                    )
                },
            ),
        );
        LocalSubscribeStart {
            response: SubscribeStartResult::accept(accept.clone()),
            served: ServedSubscribe {
                peer_id,
                domain_id: Some(domain_id),
                offer_id: Some(offer_id),
                accepted: true,
                failure_code: None,
                subscription: None,
            },
            accepted: Some(AcceptedSubscribeStart {
                accept,
                max_message_bytes,
            }),
        }
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
            Self::LocalOfferAlreadyRegistered {
                domain_id,
                offer_id,
            } => write!(
                f,
                "local offer {domain_id}/{offer_id} is already registered"
            ),
            Self::LocalOfferNotRegistered {
                domain_id,
                offer_id,
            } => write!(f, "local offer {domain_id}/{offer_id} is not registered"),
            Self::LocalPublicationNotRegistered {
                domain_id,
                offer_id,
            } => write!(
                f,
                "local publication {domain_id}/{offer_id} is not registered"
            ),
            Self::PublishOffer(error) => write!(f, "{error}"),
            Self::PublicationMessage(error) => write!(f, "{error}"),
            Self::OfferCatalog(error) => write!(f, "{error}"),
            Self::OfferCatalogPath(error) => write!(f, "{error}"),
            Self::OfferCatalogRequest(error) => write!(f, "{error}"),
            Self::OfferLoad(error) => write!(f, "{error}"),
            Self::LocalHandshake(error) => write!(f, "{error}"),
            Self::LifecycleOpen(error) => write!(f, "{error}"),
            Self::Lifecycle(error) => write!(f, "{error}"),
            Self::LifecycleAccept(error) => write!(f, "accept lifecycle streams: {error}"),
            Self::LifecycleGuard(error) => write!(f, "{error}"),
            Self::HandshakePolicy(error) => write!(f, "{error}"),
            Self::OfferCatalogAccept(error) => {
                write!(f, "accept offer-catalog streams: {error}")
            }
            Self::OfferCatalogServe(error) => write!(f, "{error}"),
            Self::GetAccept(error) => write!(f, "accept get streams: {error}"),
            Self::GetServe(error) => write!(f, "{error}"),
            Self::SubscribeAccept(error) => write!(f, "accept subscribe streams: {error}"),
            Self::SubscribeServe(error) => write!(f, "{error}"),
            Self::RemoteOffersNotLoaded { peer_id } => {
                write!(f, "remote offers are not loaded for peer {peer_id}")
            }
            Self::Path(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AukiNodeError {}

struct LocalSubscribeStart {
    response: SubscribeStartResult,
    served: ServedSubscribe,
    accepted: Option<AcceptedSubscribeStart>,
}

struct AcceptedSubscribeStart {
    accept: SubscribeAccept,
    max_message_bytes: Option<u64>,
}

fn get_failure_result(
    peer_id: PeerId,
    domain_id: Option<String>,
    offer_id: Option<String>,
    code: impl Into<String>,
) -> (GetResponse, ServedGet) {
    let code = code.into();
    (
        get_failure_response(code.clone()),
        ServedGet {
            peer_id,
            domain_id,
            offer_id,
            success: false,
            failure_code: Some(code),
        },
    )
}

fn get_failure_response(code: impl Into<String>) -> GetResponse {
    GetResponse::failure(ErrorObject::create(code))
}

fn subscribe_reject_start(
    peer_id: PeerId,
    domain_id: Option<String>,
    offer_id: Option<String>,
    code: impl Into<String>,
) -> LocalSubscribeStart {
    let (response, served) = subscribe_reject_result(peer_id, domain_id, offer_id, code);
    LocalSubscribeStart {
        response,
        served,
        accepted: None,
    }
}

fn subscribe_reject_result(
    peer_id: PeerId,
    domain_id: Option<String>,
    offer_id: Option<String>,
    code: impl Into<String>,
) -> (SubscribeStartResult, ServedSubscribe) {
    let code = code.into();
    (
        subscribe_reject_response(code.clone()),
        ServedSubscribe {
            peer_id,
            domain_id,
            offer_id,
            accepted: false,
            failure_code: Some(code),
            subscription: None,
        },
    )
}

fn subscribe_reject_response(code: impl Into<String>) -> SubscribeStartResult {
    SubscribeStartResult::reject(SubscribeReject::create(ErrorObject::create(code)))
}

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
            event = node.next_event() => {
                if let Some(event) = event {
                    node.push_pending_event(event);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AukiServeRuntime, AukiServeRuntimeEvent, OfferPolicy, PeerRelationshipState,
        accept_lifecycle_streams, accept_offer_catalog_streams, exchange_peer_handshake_strict,
        protocols::{get_protocol, subscribe_protocol},
        serve_offer_catalog_response,
    };
    use auki_identity::Wallet;
    use auki_protocol::v1::{
        domain::{DOMAIN_NONCE_LEN, DomainDelegationParams},
        frame::{decode_json_frame, decode_length, encode_json_frame},
        get::{GetRequest, GetResponse, GetResponseBody},
        message::{SPATIAL_MESSAGE_TYPE, SpatialMessage},
        offer::{OfferAccessMode, OfferStatus, PayloadDescriptor},
        subscribe::{
            SubscribeAccept, SubscribeEnd, SubscribeEndReason, SubscribeRequest,
            SubscribeStartResult,
        },
    };
    use futures::{AsyncRead, AsyncReadExt, AsyncWriteExt, stream};
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

    fn assert_get_failure(response: GetResponse, expected_code: &str) {
        match response.body {
            GetResponseBody::Error(error) => assert_eq!(error.code, expected_code),
            GetResponseBody::Message(_) => panic!("expected get failure"),
        }
    }

    fn assert_subscribe_reject(response: SubscribeStartResult, expected_code: &str) {
        let reject = response.reject_body().expect("subscribe reject");
        assert_eq!(reject.error.code, expected_code);
    }

    fn test_connection_path(port: u16) -> crate::AukiConnectionPath {
        crate::AukiConnectionPath::from_endpoint(&libp2p::core::ConnectedPoint::Dialer {
            address: format!("/ip4/127.0.0.1/tcp/{port}/ws").parse().unwrap(),
            role_override: libp2p::core::Endpoint::Dialer,
            port_use: libp2p::core::transport::PortUse::New,
        })
    }

    #[test]
    fn initializes_configured_peer_relationships() {
        let remote_peer_id = identity(61).peer_id();
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config
            .p2p
            .configured_peers
            .push(ConfiguredPeer::new(remote_peer_id));

        let mut node = AukiNode::new(identity(60), config).expect("node");
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
    fn status_snapshot_applies_buffered_connection_path_snapshot() {
        let peer_id = identity(63).peer_id();
        let path = test_connection_path(4301);
        let mut node =
            AukiNode::new(identity(62), AukiP2pNodeConfig::dial_only_development()).expect("node");

        assert!(node.node.active_connection_paths(peer_id).is_empty());
        node.node
            .push_pending_event(crate::AukiP2pEvent::ConnectionEstablished {
                peer_id,
                active_paths: vec![path.clone()],
            });

        node.status_snapshot(ISSUED_AT).expect("status snapshot");

        let relationship = node
            .relationship(peer_id)
            .expect("relationship should be created from pending event");
        assert_eq!(relationship.state, PeerRelationshipState::Connected);
        assert!(relationship.connected);
        assert_eq!(relationship.transport_paths, vec![path]);
    }

    #[test]
    fn browser_bootstrap_record_is_available_from_high_level_node() {
        let advertised: Multiaddr = "/ip4/203.0.113.10/tcp/4001/ws".parse().unwrap();
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config.advertised_addresses.push(advertised);
        let node = AukiNode::new(identity(59), config).expect("node");

        let record = node.browser_bootstrap_record();
        let peer_id = node.peer_id().to_string();

        assert_eq!(record.peer_id, node.peer_id());
        assert_eq!(record.direct_addresses.len(), 1);
        assert!(
            record.direct_addresses[0]
                .to_string()
                .ends_with(&format!("/p2p/{peer_id}"))
        );
        assert_eq!(record.bootstrap_addresses, record.direct_addresses);
        assert!(record.to_value().get("local_domains").is_none());
        assert!(record.to_value().get("offers").is_none());
    }

    #[tokio::test]
    async fn wait_for_browser_bootstrap_record_observes_listener_addresses() {
        let mut node = AukiNode::new(
            identity(60),
            AukiP2pNodeConfig::loopback_relay_server_development(),
        )
        .expect("node");

        let record = node
            .wait_for_browser_bootstrap_record(Duration::from_secs(5), ISSUED_AT)
            .await;

        assert_eq!(node.observed_listen_addresses().len(), 1);
        assert_eq!(record.relay_server_addresses.len(), 1);
        assert_eq!(record.bootstrap_addresses.len(), 1);
        assert!(record.bootstrap_addresses[0].to_string().contains("/p2p/"));
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
            AukiNodeError::Node(error) if matches!(error.as_ref(), AukiP2pNodeError::DialPolicy(_))
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
    fn publish_offer_registers_subscribe_offer_and_publication_handle() {
        let local_wallet = wallet(71);
        let declaration =
            DomainDeclaration::create(&local_wallet, &[3; DOMAIN_NONCE_LEN], Some("publish-offer"))
                .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut node = AukiNode::new(
            identity_from_wallet(local_wallet),
            AukiP2pNodeConfig::dial_only_development(),
        )
        .expect("node");
        node.upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");

        let handle = node
            .publish_offer(
                PublishOfferInput::new(
                    domain_id.clone(),
                    "bytes-main",
                    "example.bytes",
                    PayloadDescriptor::create("example.bytes.v1"),
                    || stream::iter([vec![1, 2, 3]]),
                )
                .with_display_name("Bytes Main")
                .with_metadata(json!({"source": "unit"})),
            )
            .expect("publish offer");

        assert_eq!(handle.domain_id(), domain_id);
        assert_eq!(handle.offer_id(), "bytes-main");

        let local_offers = node.local_offers(&domain_id);
        assert_eq!(local_offers.len(), 1);
        assert_eq!(
            local_offers[0].access_modes,
            vec![OfferAccessMode::Subscribe]
        );
        assert_eq!(local_offers[0].payload.payload_type, "example.bytes.v1");
        assert_eq!(local_offers[0].display_name.as_deref(), Some("Bytes Main"));

        let duplicate = node
            .publish_offer(PublishOfferInput::new(
                domain_id.clone(),
                "bytes-main",
                "example.bytes",
                PayloadDescriptor::create("example.bytes.v1"),
                || stream::iter([vec![4, 5, 6]]),
            ))
            .expect_err("published offer should not replace an existing offer");
        assert!(matches!(
            duplicate,
            AukiNodeError::LocalOfferAlreadyRegistered { domain_id: rejected, offer_id }
                if rejected == domain_id && offer_id == "bytes-main"
        ));
    }

    #[test]
    fn unpublish_offer_withdraws_published_offer_and_provider() {
        let local_wallet = wallet(73);
        let declaration = DomainDeclaration::create(
            &local_wallet,
            &[4; DOMAIN_NONCE_LEN],
            Some("unpublish-offer"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut node = AukiNode::new(
            identity_from_wallet(local_wallet),
            AukiP2pNodeConfig::dial_only_development(),
        )
        .expect("node");
        node.upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        let handle = node
            .publish_offer(PublishOfferInput::new(
                domain_id.clone(),
                "bytes-main",
                "example.bytes",
                PayloadDescriptor::create("example.bytes.v1"),
                || stream::iter([vec![1, 2, 3]]),
            ))
            .expect("publish offer");

        node.unpublish_offer(&handle).expect("unpublish offer");
        assert!(node.local_offers(&domain_id).is_empty());

        let error = node
            .unpublish_offer(&handle)
            .expect_err("publication should already be gone");
        assert!(matches!(
            error,
            AukiNodeError::LocalPublicationNotRegistered { domain_id: rejected, offer_id }
                if rejected == domain_id && offer_id == "bytes-main"
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

    #[tokio::test]
    async fn auki_node_two_peer_flow_runs_lifecycle_offer_get_and_subscribe() {
        let provider_wallet = wallet(97);
        let provider_identity = identity_from_wallet(provider_wallet.clone());
        let declaration = DomainDeclaration::create(
            &provider_wallet,
            &[12; DOMAIN_NONCE_LEN],
            Some("full-flow-provider"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut consumer =
            AukiNode::new(identity(98), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut provider = AukiNode::new(
            provider_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let consumer_peer_id = consumer.peer_id();
        let provider_peer_id = provider.peer_id();
        let provider_addr = wait_for_listen_addr(&mut provider).await;
        let mut provider_peer = ConfiguredPeer::new(provider_peer_id);
        provider_peer.dial_addresses.push(provider_addr);

        provider
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        provider
            .upsert_local_offer(offer(&domain_id, "camera-main"))
            .expect("local offer");
        let get_domain_id = domain_id.clone();
        provider
            .upsert_get_provider(
                domain_id.clone(),
                "camera-main",
                move |_request: &GetRequest, _now: &str| {
                    Ok(message(&get_domain_id, "camera-main", 31))
                },
            )
            .expect("get provider");
        provider
            .upsert_subscribe_provider(
                domain_id.clone(),
                "camera-main",
                |_request: &SubscribeRequest, _now: &str| {
                    Ok(AukiSubscribeProviderAccept {
                        initial_sequence: Some(41),
                        generated_at: Some(ISSUED_AT.to_owned()),
                        metadata: None,
                    })
                },
            )
            .expect("subscribe provider");
        provider
            .ensure_lifecycle_incoming()
            .expect("lifecycle incoming");
        provider
            .ensure_offer_catalog_incoming()
            .expect("offer catalog incoming");
        provider.ensure_get_incoming().expect("get incoming");
        provider
            .ensure_subscribe_incoming()
            .expect("subscribe incoming");

        let (server_done_tx, server_done_rx) = tokio::sync::oneshot::channel();
        let (consumer_dropped_tx, consumer_dropped_rx) = tokio::sync::oneshot::channel();
        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let lifecycle = provider
                .serve_next_lifecycle(LifecycleInput::new(), ISSUED_AT)
                .await
                .expect("serve lifecycle")
                .expect("served lifecycle");
            assert_eq!(lifecycle.authenticated_peer_id, consumer_peer_id);
            let relationship = provider
                .relationship(consumer_peer_id)
                .expect("provider should track consumer after lifecycle");
            assert_eq!(relationship.state, PeerRelationshipState::Authorized);
            assert!(relationship.connected);
            assert!(relationship.transport_paths.is_empty());
            let _snapshot = provider
                .status_snapshot(ISSUED_AT)
                .expect("status snapshot");
            let relationship = provider
                .relationship(consumer_peer_id)
                .expect("provider should retain consumer relationship");
            assert_eq!(relationship.state, PeerRelationshipState::Authorized);
            assert!(relationship.connected);
            assert_eq!(relationship.transport_paths.len(), 1);

            let served_catalog = provider
                .serve_next_offer_catalog(Some(ISSUED_AT))
                .await
                .expect("serve offer catalog")
                .expect("served offer catalog");
            assert_eq!(served_catalog.peer_id, consumer_peer_id);
            assert_eq!(served_catalog.domain_ids, vec![server_domain_id.clone()]);
            assert_eq!(served_catalog.kinds, vec!["frame".to_owned()]);

            let served_get = provider
                .serve_next_get(ISSUED_AT)
                .await
                .expect("serve get")
                .expect("served get");
            assert!(served_get.success);
            assert_eq!(served_get.peer_id, consumer_peer_id);
            assert_eq!(
                served_get.domain_id.as_deref(),
                Some(server_domain_id.as_str())
            );
            assert_eq!(served_get.offer_id.as_deref(), Some("camera-main"));

            let served_subscribe = provider
                .serve_next_subscribe(ISSUED_AT)
                .await
                .expect("serve subscribe")
                .expect("served subscribe");
            assert!(served_subscribe.accepted);
            let mut served_subscription = served_subscribe
                .into_subscription()
                .expect("accepted served subscription");
            let data = message(&server_domain_id, "camera-main", 41);
            provider
                .send_served_subscription_message(&mut served_subscription, &data)
                .await
                .expect("send served subscription message");
            provider
                .end_served_subscription(
                    served_subscription,
                    SubscribeEndReason::Complete,
                    None,
                    None,
                )
                .await
                .expect("end served subscription");
            server_done_tx.send(()).expect("send server done");

            consumer_dropped_rx.await.expect("consumer dropped signal");
            drive_node_until(
                &mut provider.node,
                tokio::time::sleep(Duration::from_millis(250)),
            )
            .await;
            let _snapshot = provider
                .status_snapshot(ISSUED_AT)
                .expect("status snapshot after disconnect");
            let relationship = provider
                .relationship(consumer_peer_id)
                .expect("provider should retain lost consumer relationship");
            assert_eq!(relationship.state, PeerRelationshipState::Lost);
            assert!(!relationship.connected);
            assert!(relationship.transport_paths.is_empty());
        });

        consumer
            .upsert_configured_peer(provider_peer)
            .expect("configured peer");
        consumer
            .dial_configured_peer(provider_peer_id)
            .expect("dial configured peer");
        wait_for_peer_connected(&mut consumer, provider_peer_id).await;

        let lifecycle = consumer
            .run_lifecycle(provider_peer_id, LifecycleInput::new(), ISSUED_AT)
            .await
            .expect("run lifecycle");
        assert_eq!(lifecycle.accepted_served_domains.len(), 1);
        assert_eq!(lifecycle.accepted_served_domains[0].domain_id, domain_id);

        let mut offer_input = RemoteOfferLoadInput::new();
        offer_input.domain_ids.push(domain_id.clone());
        offer_input.kinds.push("frame".to_owned());
        offer_input
            .supported_payload_types
            .push("auki.frame".to_owned());
        let report = consumer
            .load_remote_offers(provider_peer_id, offer_input, ISSUED_AT)
            .await
            .expect("load remote offers");
        assert_eq!(report.offers.len(), 1);
        assert!(report.offers[0].usable);

        let get = consumer
            .get(
                provider_peer_id,
                GetInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            )
            .await
            .expect("get");
        assert_eq!(get.message.sequence, Some(31));

        let mut subscription = consumer
            .subscribe(
                provider_peer_id,
                SubscribeInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            )
            .await
            .expect("subscribe");
        let message = consumer
            .next_subscription_message(&mut subscription, "2026-05-26T12:01:00Z")
            .await
            .expect("subscription data");
        assert_eq!(message.sequence, Some(41));
        assert_eq!(subscription.last_sequence(), Some(41));

        server_done_rx.await.expect("server done");
        let relationship = consumer
            .relationship(provider_peer_id)
            .expect("relationship");
        assert_eq!(relationship.state, PeerRelationshipState::Ready);
        assert_eq!(
            relationship.offer_catalog_state,
            OfferCatalogLoadState::Loaded
        );
        assert!(relationship.paths.iter().any(|path| {
            path.path_type.as_deref() == Some("get") && path.state.as_deref() == Some("succeeded")
        }));
        assert!(relationship.paths.iter().any(|path| {
            path.path_type.as_deref() == Some("subscribe")
                && path.state.as_deref() == Some("active")
                && path.last_sequence == Some(41)
        }));

        drop(subscription);
        drop(consumer);
        consumer_dropped_tx
            .send(())
            .expect("signal consumer dropped");
        server.await.expect("server task");
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
            DomainDelegationParams {
                domain_id: &domain_id,
                delegate_wallet_public_key: &local_wallet.public_key(),
                delegate_peer_id: &local_identity.peer_id(),
                scopes: &[DelegationScope::Serve],
                valid_from: ISSUED_AT,
                expires_at: DELEGATION_EXPIRES_AT,
                label: None,
            },
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
            DomainDelegationParams {
                domain_id: &domain_id,
                delegate_wallet_public_key: &local_wallet.public_key(),
                delegate_peer_id: &local_identity.peer_id(),
                scopes: &[DelegationScope::Serve],
                valid_from: ISSUED_AT,
                expires_at: DELEGATION_EXPIRES_AT,
                label: None,
            },
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
    async fn get_can_be_served_by_registered_provider() {
        let listener_wallet = wallet(87);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[8; DOMAIN_NONCE_LEN],
            Some("get-provider"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer =
            AukiNode::new(identity(88), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .upsert_local_offer(offer(&domain_id, "camera-main"))
            .expect("local offer");
        let provider_domain_id = domain_id.clone();
        listener
            .upsert_get_provider(
                domain_id.clone(),
                "camera-main",
                move |_request: &GetRequest, _now: &str| {
                    Ok(message(&provider_domain_id, "camera-main", 21))
                },
            )
            .expect("get provider");

        let (served_tx, served_rx) = tokio::sync::oneshot::channel();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let served = listener
                .serve_next_get(ISSUED_AT)
                .await
                .expect("serve next get")
                .expect("served get");
            assert!(served.success);
            assert_eq!(served.domain_id.as_deref(), Some(server_domain_id.as_str()));
            assert_eq!(served.offer_id.as_deref(), Some("camera-main"));
            served_tx.send(served).expect("send served get");

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    event = listener.next_event(ISSUED_AT) => {
                        if event.is_none() {
                            break;
                        }
                    }
                }
            }
        });

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;
        dialer.upsert_remote_offer_report(offer_report(listener_peer_id, &domain_id));

        let outcome = dialer
            .get(
                listener_peer_id,
                GetInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            )
            .await;
        let served = served_rx.await.expect("served get");
        let _ = stop_tx.send(());
        server.await.expect("server task");
        let outcome = outcome.expect("get should succeed");

        assert!(served.success);
        assert_eq!(outcome.message.sequence, Some(21));
    }

    #[test]
    fn get_provider_requires_registered_local_offer() {
        let mut node =
            AukiNode::new(identity(89), AukiP2pNodeConfig::dial_only_development()).unwrap();

        let error = node
            .upsert_get_provider(
                "missing-domain",
                "camera-main",
                |_request: &GetRequest, _now: &str| {
                    Err(AukiGetProviderError::new(
                        error::OFFER_TEMPORARILY_UNAVAILABLE,
                    ))
                },
            )
            .expect_err("provider should require local offer");

        assert!(matches!(
            error,
            AukiNodeError::LocalOfferNotRegistered { domain_id, offer_id }
                if domain_id == "missing-domain" && offer_id == "camera-main"
        ));
    }

    #[test]
    fn local_get_response_returns_structured_provider_failures() {
        let local_wallet = wallet(90);
        let declaration =
            DomainDeclaration::create(&local_wallet, &[9; DOMAIN_NONCE_LEN], Some("get-failure"))
                .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut node = AukiNode::new(
            identity_from_wallet(local_wallet),
            AukiP2pNodeConfig::dial_only_development(),
        )
        .unwrap();
        node.upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        node.upsert_local_offer(offer(&domain_id, "camera-main"))
            .expect("local offer");
        let request = GetRequest::create(domain_id.clone(), "camera-main", None, Vec::new(), None)
            .expect("get request");
        let requester = identity(91).peer_id();

        let (response, served) = node.local_get_response(requester, &request, ISSUED_AT);
        assert_get_failure(response, error::OFFER_TEMPORARILY_UNAVAILABLE);
        assert_eq!(
            served.failure_code.as_deref(),
            Some(error::OFFER_TEMPORARILY_UNAVAILABLE)
        );

        node.upsert_get_provider(
            domain_id.clone(),
            "camera-main",
            |_request: &GetRequest, _now: &str| Err(AukiGetProviderError::new("provider.busy")),
        )
        .expect("provider");
        let (response, served) = node.local_get_response(requester, &request, ISSUED_AT);
        assert_get_failure(response, "provider.busy");
        assert_eq!(served.failure_code.as_deref(), Some("provider.busy"));

        let provider_domain_id = domain_id.clone();
        node.upsert_get_provider(
            domain_id.clone(),
            "camera-main",
            move |_request: &GetRequest, _now: &str| {
                Ok(message(&provider_domain_id, "wrong-offer", 1))
            },
        )
        .expect("provider");
        let (response, served) = node.local_get_response(requester, &request, ISSUED_AT);
        assert_get_failure(response, error::MESSAGE_INVALID_ENVELOPE);
        assert_eq!(
            served.failure_code.as_deref(),
            Some(error::MESSAGE_INVALID_ENVELOPE)
        );
    }

    #[tokio::test]
    async fn subscribe_can_be_served_by_registered_provider() {
        let listener_wallet = wallet(92);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[10; DOMAIN_NONCE_LEN],
            Some("subscribe-provider"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer =
            AukiNode::new(identity(93), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .upsert_local_offer(offer(&domain_id, "camera-main"))
            .expect("local offer");
        listener
            .upsert_subscribe_provider(
                domain_id.clone(),
                "camera-main",
                |_request: &SubscribeRequest, _now: &str| {
                    Ok(AukiSubscribeProviderAccept {
                        initial_sequence: Some(1),
                        generated_at: Some(ISSUED_AT.to_owned()),
                        metadata: None,
                    })
                },
            )
            .expect("subscribe provider");

        let (served_tx, served_rx) = tokio::sync::oneshot::channel();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let served = listener
                .serve_next_subscribe(ISSUED_AT)
                .await
                .expect("serve next subscribe")
                .expect("served subscribe");
            assert!(served.accepted);
            assert_eq!(served.domain_id.as_deref(), Some(server_domain_id.as_str()));
            assert_eq!(served.offer_id.as_deref(), Some("camera-main"));
            let mut subscription = served.into_subscription().expect("accepted subscription");
            assert_eq!(subscription.payload_type(), "auki.frame");
            assert_eq!(subscription.initial_sequence(), Some(1));

            let data = message(&server_domain_id, "camera-main", 1);
            listener
                .send_served_subscription_message(&mut subscription, &data)
                .await
                .expect("send subscription data");
            listener
                .end_served_subscription(subscription, SubscribeEndReason::Complete, None, None)
                .await
                .expect("end subscription");
            served_tx.send(()).expect("send served subscribe");

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    event = listener.next_event(ISSUED_AT) => {
                        if event.is_none() {
                            break;
                        }
                    }
                }
            }
        });

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;
        dialer.upsert_remote_offer_report(offer_report(listener_peer_id, &domain_id));

        let mut subscription = dialer
            .subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            )
            .await
            .expect("subscribe should start");
        let data = dialer
            .next_subscription_message(&mut subscription, "2026-05-26T12:01:00Z")
            .await
            .expect("subscription message");

        served_rx.await.expect("served subscribe");
        let _ = stop_tx.send(());
        server.await.expect("server task");

        assert_eq!(data.sequence, Some(1));
        assert_eq!(subscription.last_sequence(), Some(1));
        assert_eq!(subscription.sequence_gap_count(), 0);
    }

    #[tokio::test]
    async fn serve_runtime_serves_get_while_subscription_is_active() {
        let listener_wallet = wallet(113);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[14; DOMAIN_NONCE_LEN],
            Some("runtime-fairness"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer =
            AukiNode::new(identity(114), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .upsert_local_offer(offer(&domain_id, "camera-main"))
            .expect("local offer");
        let provider_domain_id = domain_id.clone();
        listener
            .upsert_get_provider(
                domain_id.clone(),
                "camera-main",
                move |_request: &GetRequest, _now: &str| {
                    Ok(message(&provider_domain_id, "camera-main", 55))
                },
            )
            .expect("get provider");
        listener
            .upsert_subscribe_provider(
                domain_id.clone(),
                "camera-main",
                |_request: &SubscribeRequest, _now: &str| {
                    Ok(AukiSubscribeProviderAccept {
                        initial_sequence: Some(55),
                        generated_at: Some(ISSUED_AT.to_owned()),
                        metadata: None,
                    })
                },
            )
            .expect("subscribe provider");

        let (served_tx, served_rx) = tokio::sync::oneshot::channel();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let mut runtime = AukiServeRuntime::new(listener);
            let mut active_subscription: Option<AukiServedSubscription> = None;
            let mut served_get = None;

            timeout(Duration::from_secs(5), async {
                while served_get.is_none() {
                    let served = runtime
                        .serve_next(ISSUED_AT)
                        .await
                        .expect("serve next inbound")
                        .expect("served inbound");
                    match served {
                        AukiServeRuntimeEvent::Inbound(AukiServedInbound::Subscribe(served)) => {
                            assert!(served.accepted);
                            assert_eq!(
                                served.domain_id.as_deref(),
                                Some(server_domain_id.as_str())
                            );
                            assert_eq!(served.offer_id.as_deref(), Some("camera-main"));
                            active_subscription =
                                Some(served.into_subscription().expect("accepted subscription"));
                        }
                        AukiServeRuntimeEvent::Inbound(AukiServedInbound::Get(served)) => {
                            assert!(served.success);
                            assert_eq!(
                                served.domain_id.as_deref(),
                                Some(server_domain_id.as_str())
                            );
                            assert_eq!(served.offer_id.as_deref(), Some("camera-main"));
                            served_get = Some(served);
                        }
                        AukiServeRuntimeEvent::Inbound(
                            AukiServedInbound::Lifecycle(_) | AukiServedInbound::OfferCatalog(_),
                        )
                        | AukiServeRuntimeEvent::PublishedSubscriptionStarted(_)
                        | AukiServeRuntimeEvent::PublishedSubscriptionMessageSent(_)
                        | AukiServeRuntimeEvent::PublishedSubscriptionEnded(_) => {}
                    }
                }
            })
            .await
            .expect("runtime should serve get while subscription is active");

            assert!(active_subscription.is_some());
            let status = runtime.status().clone();
            assert_eq!(status.subscriptions_accepted, 1);
            assert_eq!(status.gets_served, 1);
            served_tx.send(status.clone()).expect("send runtime status");

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    event = runtime.node_mut().next_event(ISSUED_AT) => {
                        if event.is_none() {
                            break;
                        }
                    }
                }
            }

            status
        });

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;
        dialer.upsert_remote_offer_report(offer_report(listener_peer_id, &domain_id));

        let subscription = timeout(
            Duration::from_secs(5),
            dialer.subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            ),
        )
        .await
        .expect("subscribe should not time out")
        .expect("subscribe should start");
        assert_eq!(subscription.payload_type(), "auki.frame");

        let outcome = timeout(
            Duration::from_secs(5),
            dialer.get(
                listener_peer_id,
                GetInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            ),
        )
        .await
        .expect("get should not time out")
        .expect("get should succeed");
        let status = served_rx.await.expect("runtime status");

        drop(subscription);
        let _ = stop_tx.send(());
        let final_status = server.await.expect("server task");
        assert_eq!(outcome.message.sequence, Some(55));
        assert_eq!(status.subscriptions_accepted, 1);
        assert_eq!(status.gets_served, 1);
        assert_eq!(final_status, status);
    }

    #[tokio::test]
    async fn serve_runtime_streams_published_offer_and_serves_get() {
        let listener_wallet = wallet(115);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[15; DOMAIN_NONCE_LEN],
            Some("runtime-published"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer =
            AukiNode::new(identity(116), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .publish_offer(
                PublishOfferInput::new(
                    domain_id.clone(),
                    "bytes-main",
                    "frame",
                    PayloadDescriptor::create("auki.frame"),
                    || {
                        stream::unfold(0_u8, |index| async move {
                            if index >= 2 {
                                None
                            } else {
                                tokio::time::sleep(Duration::from_millis(10)).await;
                                Some((vec![index], index.saturating_add(1)))
                            }
                        })
                    },
                )
                .with_access_modes(vec![OfferAccessMode::Get, OfferAccessMode::Subscribe]),
            )
            .expect("publish offer");
        let get_domain_id = domain_id.clone();
        listener
            .upsert_get_provider(
                domain_id.clone(),
                "bytes-main",
                move |_request: &GetRequest, _now: &str| {
                    Ok(message(&get_domain_id, "bytes-main", 77))
                },
            )
            .expect("get provider");
        let remote_report = OfferLoadReport {
            peer_id: listener_peer_id,
            offers: listener
                .local_offers(&domain_id)
                .into_iter()
                .map(|offer| LoadedRemoteOffer {
                    offer,
                    usable: true,
                    unusable_reason: None,
                })
                .collect(),
            diagnostics: Vec::new(),
            generated_at: Some(ISSUED_AT.to_owned()),
        };

        let (served_tx, served_rx) = tokio::sync::oneshot::channel();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let dialer_peer_id = dialer.peer_id();
        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let mut runtime = AukiServeRuntime::new(listener);
            let mut started = false;
            let mut get_served = false;
            let mut ended = false;

            timeout(Duration::from_secs(5), async {
                while !ended || !get_served {
                    let event = runtime
                        .serve_next(ISSUED_AT)
                        .await
                        .expect("serve next runtime event")
                        .expect("runtime event");
                    match event {
                        AukiServeRuntimeEvent::PublishedSubscriptionStarted(status) => {
                            assert_eq!(status.peer_id, dialer_peer_id);
                            assert_eq!(status.domain_id.as_str(), server_domain_id.as_str());
                            assert_eq!(status.offer_id, "bytes-main");
                            assert_eq!(status.messages_sent, 0);
                            started = true;
                        }
                        AukiServeRuntimeEvent::PublishedSubscriptionMessageSent(status) => {
                            assert!(started);
                            assert_eq!(status.domain_id.as_str(), server_domain_id.as_str());
                            assert_eq!(status.offer_id, "bytes-main");
                            assert!(status.messages_sent >= 1);
                        }
                        AukiServeRuntimeEvent::PublishedSubscriptionEnded(status) => {
                            assert_eq!(status.reason, SubscribeEndReason::Complete);
                            assert_eq!(status.messages_sent, 2);
                            ended = true;
                        }
                        AukiServeRuntimeEvent::Inbound(AukiServedInbound::Get(served)) => {
                            assert!(served.success);
                            assert_eq!(
                                served.domain_id.as_deref(),
                                Some(server_domain_id.as_str())
                            );
                            assert_eq!(served.offer_id.as_deref(), Some("bytes-main"));
                            get_served = true;
                        }
                        AukiServeRuntimeEvent::Inbound(_) => {}
                    }
                }
            })
            .await
            .expect("runtime should stream and serve get");

            let status = runtime.status().clone();
            assert_eq!(status.active_subscriptions, 0);
            assert_eq!(status.subscriptions_accepted, 1);
            assert_eq!(status.subscriptions_completed, 1);
            assert_eq!(status.gets_served, 1);
            assert_eq!(status.frames_sent, 2);
            served_tx.send(status.clone()).expect("send runtime status");

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    event = runtime.node_mut().next_event(ISSUED_AT) => {
                        if event.is_none() {
                            break;
                        }
                    }
                }
            }

            status
        });

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;
        dialer.upsert_remote_offer_report(remote_report);

        let mut subscription = timeout(
            Duration::from_secs(5),
            dialer.subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id.clone(), "bytes-main"),
                ISSUED_AT,
            ),
        )
        .await
        .expect("subscribe should not time out")
        .expect("subscribe should start");
        let first = dialer
            .next_subscription_message(&mut subscription, "2026-05-26T12:01:00Z")
            .await
            .expect("first runtime-managed published message");
        assert_eq!(first.sequence, Some(0));

        let outcome = timeout(
            Duration::from_secs(5),
            dialer.get(
                listener_peer_id,
                GetInput::new(domain_id.clone(), "bytes-main"),
                ISSUED_AT,
            ),
        )
        .await
        .expect("get should not time out")
        .expect("get should succeed");

        let second = dialer
            .next_subscription_message(&mut subscription, "2026-05-26T12:01:01Z")
            .await
            .expect("second runtime-managed published message");
        let status = served_rx.await.expect("runtime status");
        drop(subscription);
        let _ = stop_tx.send(());

        let final_status = server.await.expect("server task");
        assert_eq!(outcome.message.sequence, Some(77));
        assert_eq!(second.sequence, Some(1));
        assert_eq!(status.frames_sent, 2);
        assert_eq!(status.gets_served, 1);
        assert_eq!(final_status, status);
    }

    #[tokio::test]
    async fn serve_runtime_serves_burst_gets_from_second_peer_while_subscription_streams() {
        let listener_wallet = wallet(119);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[19; DOMAIN_NONCE_LEN],
            Some("runtime-burst-get"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut subscriber =
            AukiNode::new(identity(120), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut getter =
            AukiNode::new(identity(121), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr.clone());

        let source = crate::LatestPublishedByteSource::new();
        assert!(source.publish(PublishedByteFrame::new(vec![0]).with_sequence(0)));
        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .publish_offer(
                PublishOfferInput::new(
                    domain_id.clone(),
                    "camera-main",
                    "frame",
                    PayloadDescriptor::create("auki.frame"),
                    source.clone(),
                )
                .with_access_modes(vec![OfferAccessMode::Get, OfferAccessMode::Subscribe])
                .with_backpressure_policy(crate::AukiSubscriptionBackpressurePolicy::LatestOnly),
            )
            .expect("publish offer");
        let get_domain_id = domain_id.clone();
        listener
            .upsert_get_provider(
                domain_id.clone(),
                "camera-main",
                move |_request: &GetRequest, _now: &str| {
                    Ok(message(&get_domain_id, "camera-main", 77))
                },
            )
            .expect("get provider");

        let producer_source = source.clone();
        let producer = tokio::spawn(async move {
            let mut sequence = 1_u64;
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                if !producer_source
                    .publish(PublishedByteFrame::new(vec![1]).with_sequence(sequence))
                {
                    return;
                }
                sequence = sequence.saturating_add(1);
            }
        });

        let (served_tx, served_rx) = tokio::sync::oneshot::channel();
        let (server_stop_tx, mut server_stop_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut runtime = AukiServeRuntime::new(listener);
            let mut served_tx = Some(served_tx);
            timeout(Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        _ = &mut server_stop_rx, if served_tx.is_none() => break,
                        result = runtime.serve_next(ISSUED_AT) => {
                            result.expect("serve next runtime event");
                            if runtime.status().gets_served >= 20
                                && let Some(tx) = served_tx.take()
                            {
                                tx.send(runtime.status().clone())
                                    .expect("send runtime status");
                            }
                        }
                    }
                }
            })
            .await
            .expect("runtime should serve burst gets while subscription is active");

            runtime.status().clone()
        });

        subscriber
            .upsert_configured_peer(listener_peer.clone())
            .expect("subscriber configured peer");
        subscriber
            .dial_configured_peer(listener_peer_id)
            .expect("subscriber dials listener");
        wait_for_peer_connected(&mut subscriber, listener_peer_id).await;
        subscriber.upsert_remote_offer_report(offer_report(listener_peer_id, &domain_id));

        let mut subscription = timeout(
            Duration::from_secs(5),
            subscriber.subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            ),
        )
        .await
        .expect("subscribe should not time out")
        .expect("subscribe should start");
        let first = subscriber
            .next_subscription_message(&mut subscription, "2026-05-26T12:01:00Z")
            .await
            .expect("first subscription message");
        assert!(first.sequence.is_some());

        let (reader_stop_tx, mut reader_stop_rx) = tokio::sync::oneshot::channel();
        let subscriber_reader = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut reader_stop_rx => break,
                    result = subscriber.next_subscription_message(
                        &mut subscription,
                        "2026-05-26T12:01:01Z",
                    ) => {
                        if result.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        getter
            .upsert_configured_peer(listener_peer)
            .expect("getter configured peer");
        getter
            .dial_configured_peer(listener_peer_id)
            .expect("getter dials listener");
        wait_for_peer_connected(&mut getter, listener_peer_id).await;
        getter.upsert_remote_offer_report(offer_report(listener_peer_id, &domain_id));

        for index in 0..20 {
            let result = timeout(
                Duration::from_secs(5),
                getter.get(
                    listener_peer_id,
                    GetInput::new(domain_id.clone(), "camera-main"),
                    ISSUED_AT,
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("get {index} should not time out"));
            let outcome =
                result.unwrap_or_else(|error| panic!("get {index} should succeed: {error:?}"));
            assert_eq!(outcome.message.sequence, Some(77));
        }

        let status = served_rx.await.expect("runtime status");
        let _ = server_stop_tx.send(());
        let final_status = server.await.expect("server task");
        source.close();
        producer.abort();
        let _ = reader_stop_tx.send(());
        subscriber_reader.await.expect("subscriber reader task");

        assert_eq!(status.subscriptions_accepted, 1);
        assert_eq!(status.active_subscriptions, 1);
        assert!(status.gets_served >= 20);
        assert!(final_status.gets_served >= 20);
        assert_eq!(final_status.subscriptions_accepted, 1);
    }

    #[tokio::test]
    async fn serve_runtime_accepts_burst_subscribes_from_second_peer_while_subscription_streams() {
        let listener_wallet = wallet(124);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[20; DOMAIN_NONCE_LEN],
            Some("runtime-burst-subscribe"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut steady_subscriber =
            AukiNode::new(identity(125), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut burst_subscriber =
            AukiNode::new(identity(126), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr.clone());

        let source = crate::LatestPublishedByteSource::new();
        assert!(source.publish(PublishedByteFrame::new(vec![0]).with_sequence(0)));
        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .publish_offer(
                PublishOfferInput::new(
                    domain_id.clone(),
                    "camera-main",
                    "frame",
                    PayloadDescriptor::create("auki.frame"),
                    source.clone(),
                )
                .with_access_modes(vec![OfferAccessMode::Subscribe])
                .with_backpressure_policy(crate::AukiSubscriptionBackpressurePolicy::LatestOnly),
            )
            .expect("publish offer");

        let producer_source = source.clone();
        let producer = tokio::spawn(async move {
            let mut sequence = 1_u64;
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                if !producer_source
                    .publish(PublishedByteFrame::new(vec![1]).with_sequence(sequence))
                {
                    return;
                }
                sequence = sequence.saturating_add(1);
            }
        });

        let expected_subscriptions = 11_u64;
        let (served_tx, served_rx) = tokio::sync::oneshot::channel();
        let (server_stop_tx, mut server_stop_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut runtime = AukiServeRuntime::new(listener);
            let mut served_tx = Some(served_tx);
            timeout(Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        _ = &mut server_stop_rx, if served_tx.is_none() => break,
                        result = runtime.serve_next(ISSUED_AT) => {
                            result.expect("serve next runtime event");
                            if runtime.status().subscriptions_accepted >= expected_subscriptions
                                && let Some(tx) = served_tx.take()
                            {
                                tx.send(runtime.status().clone())
                                    .expect("send runtime status");
                            }
                        }
                    }
                }
            })
            .await
            .expect("runtime should accept burst subscribes while another subscription is active");

            runtime.status().clone()
        });

        steady_subscriber
            .upsert_configured_peer(listener_peer.clone())
            .expect("steady subscriber configured peer");
        steady_subscriber
            .dial_configured_peer(listener_peer_id)
            .expect("steady subscriber dials listener");
        wait_for_peer_connected(&mut steady_subscriber, listener_peer_id).await;
        steady_subscriber.upsert_remote_offer_report(offer_report(listener_peer_id, &domain_id));

        let mut steady_subscription = timeout(
            Duration::from_secs(5),
            steady_subscriber.subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id.clone(), "camera-main"),
                ISSUED_AT,
            ),
        )
        .await
        .expect("steady subscribe should not time out")
        .expect("steady subscribe should start");
        let first = steady_subscriber
            .next_subscription_message(&mut steady_subscription, "2026-05-26T12:01:00Z")
            .await
            .expect("first steady subscription message");
        assert!(first.sequence.is_some());

        let (reader_stop_tx, mut reader_stop_rx) = tokio::sync::oneshot::channel();
        let steady_reader = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut reader_stop_rx => break,
                    result = steady_subscriber.next_subscription_message(
                        &mut steady_subscription,
                        "2026-05-26T12:01:01Z",
                    ) => {
                        if result.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        burst_subscriber
            .upsert_configured_peer(listener_peer)
            .expect("burst subscriber configured peer");
        burst_subscriber
            .dial_configured_peer(listener_peer_id)
            .expect("burst subscriber dials listener");
        wait_for_peer_connected(&mut burst_subscriber, listener_peer_id).await;
        burst_subscriber.upsert_remote_offer_report(offer_report(listener_peer_id, &domain_id));

        for index in 0..10 {
            let mut subscription = timeout(
                Duration::from_secs(5),
                burst_subscriber.subscribe(
                    listener_peer_id,
                    SubscribeInput::new(domain_id.clone(), "camera-main"),
                    ISSUED_AT,
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("burst subscribe {index} should not time out"))
            .unwrap_or_else(|error| panic!("burst subscribe {index} should start: {error:?}"));
            let message = timeout(
                Duration::from_secs(5),
                burst_subscriber
                    .next_subscription_message(&mut subscription, "2026-05-26T12:01:02Z"),
            )
            .await
            .unwrap_or_else(|_| panic!("burst subscription {index} message should not time out"))
            .unwrap_or_else(|error| {
                panic!("burst subscription {index} should receive a message: {error:?}")
            });
            assert!(message.sequence.is_some());
            drop(subscription);
        }

        let status = served_rx.await.expect("runtime status");
        let _ = server_stop_tx.send(());
        let final_status = server.await.expect("server task");
        source.close();
        producer.abort();
        let _ = reader_stop_tx.send(());
        steady_reader.await.expect("steady reader task");

        assert!(status.subscriptions_accepted >= expected_subscriptions);
        assert!(status.frames_sent >= expected_subscriptions);
        assert!(final_status.subscriptions_accepted >= expected_subscriptions);
    }

    #[tokio::test]
    async fn serve_runtime_streams_published_offer_to_multiple_subscribers() {
        let listener_wallet = wallet(117);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[16; DOMAIN_NONCE_LEN],
            Some("runtime-multisub"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer_a =
            AukiNode::new(identity(118), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut dialer_b =
            AukiNode::new(identity(119), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .publish_offer(PublishOfferInput::new(
                domain_id.clone(),
                "bytes-main",
                "frame",
                PayloadDescriptor::create("auki.frame"),
                || {
                    stream::unfold(0_u8, |index| async move {
                        if index >= 2 {
                            None
                        } else {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Some((vec![index.saturating_add(1)], index.saturating_add(1)))
                        }
                    })
                },
            ))
            .expect("publish offer");
        let loaded_offers = listener
            .local_offers(&domain_id)
            .into_iter()
            .map(|offer| LoadedRemoteOffer {
                offer,
                usable: true,
                unusable_reason: None,
            })
            .collect::<Vec<_>>();
        let remote_report_a = OfferLoadReport {
            peer_id: listener_peer_id,
            offers: loaded_offers.clone(),
            diagnostics: Vec::new(),
            generated_at: Some(ISSUED_AT.to_owned()),
        };
        let remote_report_b = OfferLoadReport {
            peer_id: listener_peer_id,
            offers: loaded_offers,
            diagnostics: Vec::new(),
            generated_at: Some(ISSUED_AT.to_owned()),
        };

        let (served_tx, served_rx) = tokio::sync::oneshot::channel();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut runtime = AukiServeRuntime::new(listener);

            timeout(Duration::from_secs(5), async {
                while runtime.status().subscriptions_completed < 2 {
                    let _ = runtime
                        .serve_next(ISSUED_AT)
                        .await
                        .expect("serve next runtime event")
                        .expect("runtime event");
                }
            })
            .await
            .expect("runtime should complete both subscribers");

            let status = runtime.status().clone();
            assert_eq!(status.active_subscriptions, 0);
            assert_eq!(status.subscriptions_accepted, 2);
            assert_eq!(status.subscriptions_completed, 2);
            assert_eq!(status.frames_sent, 4);
            served_tx.send(status.clone()).expect("send runtime status");

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    event = runtime.node_mut().next_event(ISSUED_AT) => {
                        if event.is_none() {
                            break;
                        }
                    }
                }
            }

            status
        });

        dialer_a
            .upsert_configured_peer(listener_peer.clone())
            .expect("configured peer a");
        dialer_b
            .upsert_configured_peer(listener_peer)
            .expect("configured peer b");
        dialer_a
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer a");
        dialer_b
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer b");
        wait_for_peer_connected(&mut dialer_a, listener_peer_id).await;
        wait_for_peer_connected(&mut dialer_b, listener_peer_id).await;
        dialer_a.upsert_remote_offer_report(remote_report_a);
        dialer_b.upsert_remote_offer_report(remote_report_b);

        let mut subscription_a = dialer_a
            .subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id.clone(), "bytes-main"),
                ISSUED_AT,
            )
            .await
            .expect("subscriber a should start");
        let mut subscription_b = dialer_b
            .subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id.clone(), "bytes-main"),
                ISSUED_AT,
            )
            .await
            .expect("subscriber b should start");

        let first_a = dialer_a
            .next_subscription_message(&mut subscription_a, "2026-05-26T12:01:00Z")
            .await
            .expect("first subscriber a message");
        let second_a = dialer_a
            .next_subscription_message(&mut subscription_a, "2026-05-26T12:01:01Z")
            .await
            .expect("second subscriber a message");
        let first_b = dialer_b
            .next_subscription_message(&mut subscription_b, "2026-05-26T12:01:00Z")
            .await
            .expect("first subscriber b message");
        let second_b = dialer_b
            .next_subscription_message(&mut subscription_b, "2026-05-26T12:01:01Z")
            .await
            .expect("second subscriber b message");
        let status = served_rx.await.expect("runtime status");
        drop(subscription_a);
        drop(subscription_b);
        let _ = stop_tx.send(());

        let final_status = server.await.expect("server task");
        assert_eq!(first_a.payload.bytes, Some(vec![1]));
        assert_eq!(second_a.payload.bytes, Some(vec![2]));
        assert_eq!(first_b.payload.bytes, Some(vec![1]));
        assert_eq!(second_b.payload.bytes, Some(vec![2]));
        assert_eq!(status.frames_sent, 4);
        assert_eq!(final_status, status);
    }

    #[tokio::test]
    async fn serve_runtime_shutdown_ends_active_published_subscription() {
        let listener_wallet = wallet(122);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[18; DOMAIN_NONCE_LEN],
            Some("runtime-shutdown"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer =
            AukiNode::new(identity(123), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .publish_offer(PublishOfferInput::new(
                domain_id.clone(),
                "bytes-main",
                "frame",
                PayloadDescriptor::create("auki.frame"),
                || stream::pending::<Vec<u8>>(),
            ))
            .expect("publish offer");
        let remote_report = OfferLoadReport {
            peer_id: listener_peer_id,
            offers: listener
                .local_offers(&domain_id)
                .into_iter()
                .map(|offer| LoadedRemoteOffer {
                    offer,
                    usable: true,
                    unusable_reason: None,
                })
                .collect(),
            diagnostics: Vec::new(),
            generated_at: Some(ISSUED_AT.to_owned()),
        };

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let mut runtime = AukiServeRuntime::new(listener);
            let event = timeout(Duration::from_secs(5), runtime.serve_next(ISSUED_AT))
                .await
                .expect("serve next should not time out")
                .expect("serve next runtime event")
                .expect("runtime event");
            let AukiServeRuntimeEvent::PublishedSubscriptionStarted(status) = event else {
                panic!("expected runtime-managed subscription start");
            };
            assert_eq!(status.domain_id.as_str(), server_domain_id.as_str());
            assert_eq!(status.offer_id, "bytes-main");
            assert_eq!(runtime.status().active_subscriptions, 1);
            started_tx.send(()).expect("send started");

            shutdown_rx.await.expect("shutdown signal");
            let ended = runtime
                .shutdown_active_subscriptions(SubscribeEndReason::ProducerShutdown)
                .await
                .expect("shutdown subscriptions");
            assert_eq!(ended.len(), 1);
            assert_eq!(ended[0].reason, SubscribeEndReason::ProducerShutdown);

            runtime.status().clone()
        });

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;
        dialer.upsert_remote_offer_report(remote_report);

        let subscription = timeout(
            Duration::from_secs(5),
            dialer.subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id, "bytes-main"),
                ISSUED_AT,
            ),
        )
        .await
        .expect("subscribe should not time out")
        .expect("subscribe should start");
        started_rx.await.expect("started");
        shutdown_tx.send(()).expect("send shutdown");

        let status = server.await.expect("server task");
        drop(subscription);
        assert_eq!(status.active_subscriptions, 0);
        assert_eq!(status.subscriptions_accepted, 1);
        assert_eq!(status.subscriptions_closed_by_producer, 1);
    }

    #[tokio::test]
    async fn serve_runtime_closes_published_subscription_on_backpressure() {
        let listener_wallet = wallet(120);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[17; DOMAIN_NONCE_LEN],
            Some("runtime-backpressure"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer =
            AukiNode::new(identity(121), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .publish_offer(
                PublishOfferInput::new(
                    domain_id.clone(),
                    "bytes-main",
                    "frame",
                    PayloadDescriptor::create("auki.frame"),
                    || stream::iter((0_u8..64).map(|value| vec![value])),
                )
                .with_backpressure_policy(
                    crate::AukiSubscriptionBackpressurePolicy::CloseOnFull { capacity: 1 },
                ),
            )
            .expect("publish offer");
        let remote_report = OfferLoadReport {
            peer_id: listener_peer_id,
            offers: listener
                .local_offers(&domain_id)
                .into_iter()
                .map(|offer| LoadedRemoteOffer {
                    offer,
                    usable: true,
                    unusable_reason: None,
                })
                .collect(),
            diagnostics: Vec::new(),
            generated_at: Some(ISSUED_AT.to_owned()),
        };

        let (proceed_tx, proceed_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut runtime = AukiServeRuntime::new(listener);
            let mut ended = None;

            let started = runtime
                .serve_next(ISSUED_AT)
                .await
                .expect("serve subscribe start")
                .expect("subscription started");
            assert!(matches!(
                started,
                AukiServeRuntimeEvent::PublishedSubscriptionStarted(_)
            ));
            proceed_rx.await.expect("consumer subscribe completed");

            timeout(Duration::from_secs(5), async {
                while ended.is_none() {
                    let event = runtime
                        .serve_next(ISSUED_AT)
                        .await
                        .expect("serve next runtime event")
                        .expect("runtime event");
                    if let AukiServeRuntimeEvent::PublishedSubscriptionEnded(status) = event {
                        ended = Some(status);
                    }
                }
            })
            .await
            .expect("runtime should close for backpressure");

            let status = ended.expect("ended status");
            assert_eq!(status.reason, SubscribeEndReason::Error);
            assert_eq!(
                status.error_code.as_deref(),
                Some(error::SUBSCRIBE_BACKPRESSURE)
            );
            assert_eq!(status.retryable, Some(true));
            let runtime_status = runtime.status().clone();
            assert_eq!(runtime_status.active_subscriptions, 0);
            assert_eq!(runtime_status.subscriptions_accepted, 1);
            assert_eq!(runtime_status.subscriptions_closed_for_backpressure, 1);
            assert!(runtime_status.frames_dropped >= 1);
            runtime_status
        });

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;
        dialer.upsert_remote_offer_report(remote_report);

        let subscription = dialer
            .subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id, "bytes-main"),
                ISSUED_AT,
            )
            .await
            .expect("subscribe should start");
        proceed_tx.send(()).expect("send proceed");

        let status = server.await.expect("server task");
        drop(subscription);
        assert_eq!(status.subscriptions_closed_for_backpressure, 1);
    }

    #[tokio::test]
    async fn served_subscription_observes_consumer_cancel() {
        let listener_wallet = wallet(111);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[13; DOMAIN_NONCE_LEN],
            Some("subscribe-cancel"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer =
            AukiNode::new(identity(112), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .upsert_local_offer(offer(&domain_id, "camera-main"))
            .expect("local offer");
        listener
            .upsert_subscribe_provider(
                domain_id.clone(),
                "camera-main",
                |_request: &SubscribeRequest, _now: &str| {
                    Ok(AukiSubscribeProviderAccept {
                        initial_sequence: Some(1),
                        generated_at: Some(ISSUED_AT.to_owned()),
                        metadata: None,
                    })
                },
            )
            .expect("subscribe provider");

        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let served = listener
                .serve_next_subscribe(ISSUED_AT)
                .await
                .expect("serve next subscribe")
                .expect("served subscribe");
            assert!(served.accepted);
            let mut subscription = served.into_subscription().expect("accepted subscription");

            let end = timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(end) = subscription
                        .try_consumer_end()
                        .expect("consumer end should parse")
                    {
                        return end;
                    }
                    drive_node_until(
                        &mut listener.node,
                        tokio::time::sleep(Duration::from_millis(10)),
                    )
                    .await;
                }
            })
            .await
            .expect("consumer end should arrive");

            assert_eq!(end.domain_id, server_domain_id);
            assert_eq!(end.offer_id, "camera-main");
            assert_eq!(end.reason, SubscribeEndReason::Cancelled);
        });

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;

        let limits = dialer.node.config().p2p.limits;
        let request = SubscribeRequest::create(
            domain_id.clone(),
            "camera-main",
            None,
            vec!["auki.frame".to_owned()],
            None,
        )
        .expect("subscribe request");
        let end = SubscribeEnd::create(
            domain_id,
            "camera-main",
            SubscribeEndReason::Cancelled,
            None,
            None,
            None,
        )
        .expect("subscribe end");
        let mut control = dialer.node.stream_control();

        drive_node_until(&mut dialer.node, async move {
            let mut stream = control
                .open_stream(listener_peer_id, subscribe_protocol())
                .await
                .expect("open subscribe stream");
            let request_frame =
                encode_json_frame(request.value(), limits.subscribe_message_frame_body_bytes)
                    .expect("request frame");
            stream
                .write_all(&request_frame)
                .await
                .expect("write request");
            stream.flush().await.expect("flush request");

            let start_frame =
                read_frame_bytes(&mut stream, limits.subscribe_message_frame_body_bytes).await;
            let start = SubscribeStartResult::from_value(decode_request_frame(
                &start_frame,
                limits.subscribe_message_frame_body_bytes,
            ))
            .expect("subscribe start");
            assert!(start.accept_body().is_some());

            let end_frame =
                encode_json_frame(end.value(), limits.subscribe_message_frame_body_bytes)
                    .expect("end frame");
            stream.write_all(&end_frame).await.expect("write end");
            stream.flush().await.expect("flush end");
            stream.close().await.expect("close stream");
        })
        .await;

        server.await.expect("server task");
    }

    #[tokio::test]
    async fn published_offer_serves_finite_byte_source_over_subscribe() {
        let listener_wallet = wallet(101);
        let listener_identity = identity_from_wallet(listener_wallet.clone());
        let declaration = DomainDeclaration::create(
            &listener_wallet,
            &[12; DOMAIN_NONCE_LEN],
            Some("published-source"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut dialer =
            AukiNode::new(identity(102), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener = AukiNode::new(
            listener_identity,
            AukiP2pNodeConfig::loopback_tcp_development(),
        )
        .unwrap();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        listener
            .upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        listener
            .publish_offer(PublishOfferInput::new(
                domain_id.clone(),
                "bytes-main",
                "example.bytes",
                PayloadDescriptor::create("example.bytes.v1"),
                || stream::iter([vec![1, 2, 3], vec![4, 5]]),
            ))
            .expect("publish offer");
        let remote_report = OfferLoadReport {
            peer_id: listener_peer_id,
            offers: listener
                .local_offers(&domain_id)
                .into_iter()
                .map(|offer| LoadedRemoteOffer {
                    offer,
                    usable: true,
                    unusable_reason: None,
                })
                .collect(),
            diagnostics: Vec::new(),
            generated_at: Some(ISSUED_AT.to_owned()),
        };

        let (served_tx, served_rx) = tokio::sync::oneshot::channel();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let server_domain_id = domain_id.clone();
        let server = tokio::spawn(async move {
            let served = listener
                .serve_next_published_subscription(ISSUED_AT)
                .await
                .expect("serve next published subscription")
                .expect("served published subscription");
            assert!(served.accepted);
            assert_eq!(served.domain_id.as_deref(), Some(server_domain_id.as_str()));
            assert_eq!(served.offer_id.as_deref(), Some("bytes-main"));
            assert_eq!(served.messages_sent, 2);
            assert_eq!(served.end_reason, Some(SubscribeEndReason::Complete));
            served_tx.send(served).expect("send served publication");

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    event = listener.next_event(ISSUED_AT) => {
                        if event.is_none() {
                            break;
                        }
                    }
                }
            }
        });

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("dial configured peer");
        wait_for_peer_connected(&mut dialer, listener_peer_id).await;
        dialer.upsert_remote_offer_report(remote_report);

        let mut subscription = dialer
            .subscribe(
                listener_peer_id,
                SubscribeInput::new(domain_id.clone(), "bytes-main"),
                ISSUED_AT,
            )
            .await
            .expect("subscribe should start");
        let first = dialer
            .next_subscription_message(&mut subscription, "2026-05-26T12:01:00Z")
            .await
            .expect("first subscription message");
        let second = dialer
            .next_subscription_message(&mut subscription, "2026-05-26T12:01:01Z")
            .await
            .expect("second subscription message");

        served_rx.await.expect("served publication");
        let _ = stop_tx.send(());
        server.await.expect("server task");

        assert_eq!(first.sequence, Some(0));
        assert_eq!(first.payload.bytes, Some(vec![1, 2, 3]));
        assert_eq!(second.sequence, Some(1));
        assert_eq!(second.payload.bytes, Some(vec![4, 5]));
        assert_eq!(subscription.last_sequence(), Some(1));
        assert_eq!(subscription.sequence_gap_count(), 0);
    }

    #[test]
    fn subscribe_provider_requires_registered_local_offer() {
        let mut node =
            AukiNode::new(identity(94), AukiP2pNodeConfig::dial_only_development()).unwrap();

        let error = node
            .upsert_subscribe_provider(
                "missing-domain",
                "camera-main",
                |_request: &SubscribeRequest, _now: &str| {
                    Err(AukiSubscribeProviderError::new(
                        error::OFFER_TEMPORARILY_UNAVAILABLE,
                    ))
                },
            )
            .expect_err("provider should require local offer");

        assert!(matches!(
            error,
            AukiNodeError::LocalOfferNotRegistered { domain_id, offer_id }
                if domain_id == "missing-domain" && offer_id == "camera-main"
        ));
    }

    #[test]
    fn local_subscribe_start_returns_structured_provider_failures() {
        let local_wallet = wallet(95);
        let declaration = DomainDeclaration::create(
            &local_wallet,
            &[11; DOMAIN_NONCE_LEN],
            Some("subscribe-failure"),
        )
        .expect("domain declaration");
        let registration =
            LocalDomainRegistration::owner(declaration, true).expect("owner registration");
        let domain_id = registration.domain_id().to_owned();
        let mut node = AukiNode::new(
            identity_from_wallet(local_wallet),
            AukiP2pNodeConfig::dial_only_development(),
        )
        .unwrap();
        node.upsert_local_domain(registration, ISSUED_AT)
            .expect("local domain");
        node.upsert_local_offer(offer(&domain_id, "camera-main"))
            .expect("local offer");
        let request =
            SubscribeRequest::create(domain_id.clone(), "camera-main", None, Vec::new(), None)
                .expect("subscribe request");
        let requester = identity(96).peer_id();

        let start = node.local_subscribe_start(requester, &request, ISSUED_AT);
        assert_subscribe_reject(start.response, error::OFFER_TEMPORARILY_UNAVAILABLE);
        assert_eq!(
            start.served.failure_code.as_deref(),
            Some(error::OFFER_TEMPORARILY_UNAVAILABLE)
        );

        node.upsert_subscribe_provider(
            domain_id.clone(),
            "camera-main",
            |_request: &SubscribeRequest, _now: &str| {
                Err(AukiSubscribeProviderError::new("provider.busy"))
            },
        )
        .expect("provider");
        let start = node.local_subscribe_start(requester, &request, ISSUED_AT);
        assert_subscribe_reject(start.response, "provider.busy");
        assert_eq!(start.served.failure_code.as_deref(), Some("provider.busy"));

        node.upsert_subscribe_provider(
            domain_id.clone(),
            "camera-main",
            |_request: &SubscribeRequest, _now: &str| {
                Ok(AukiSubscribeProviderAccept {
                    initial_sequence: None,
                    generated_at: None,
                    metadata: Some(json!("not-object")),
                })
            },
        )
        .expect("provider");
        let start = node.local_subscribe_start(requester, &request, ISSUED_AT);
        assert_subscribe_reject(start.response, error::MESSAGE_INVALID_ENVELOPE);
        assert_eq!(
            start.served.failure_code.as_deref(),
            Some(error::MESSAGE_INVALID_ENVELOPE)
        );
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
        assert_eq!(
            dialer
                .relationship(listener_peer_id)
                .unwrap()
                .transport_paths
                .len(),
            1
        );
        let snapshot = dialer.status_snapshot(ISSUED_AT).expect("status snapshot");
        assert_eq!(
            snapshot.remote_peers[0].lifecycle_state.as_deref(),
            Some("connected")
        );
        let path = snapshot.remote_peers[0]
            .value()
            .get("transport_paths")
            .and_then(Value::as_array)
            .and_then(|paths| paths.first())
            .expect("transport path");
        assert_eq!(
            snapshot.remote_peers[0]
                .value()
                .get("relay_involved")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            path.get("direction").and_then(Value::as_str),
            Some("dialer")
        );
        assert_eq!(path.get("transport").and_then(Value::as_str), Some("tcp"));
        assert_eq!(
            path.get("relay_involved").and_then(Value::as_bool),
            Some(false)
        );
        assert!(path.get("remote_address").is_some());
    }
}
