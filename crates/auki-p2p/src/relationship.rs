//! Runtime-owned peer relationship state and status projection.

use crate::{
    AukiConnectionPath, HandshakePolicyError, HandshakeValidationResult, StatusPrivacyConfig,
};
use auki_protocol::v1::{
    base64url,
    status::{
        FailureRecord, LocalPeerStatus, PathStatus, RemotePeerStatus, StatusError, StatusSnapshot,
    },
};
use libp2p_identity::PeerId;
use serde_json::{Map, Value};
use std::fmt;

/// Peer lifecycle states tracked by the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerRelationshipState {
    /// Peer exists only as an unknown id.
    Unknown,
    /// Peer was discovered from an external source.
    Discovered,
    /// Peer was explicitly configured.
    Configured,
    /// Runtime is dialing the peer.
    Dialing,
    /// Transport connection is established.
    Connected,
    /// Lifecycle handshake accepted the peer.
    Authorized,
    /// Runtime is loading remote offers.
    LoadingOffers,
    /// Peer is ready for usable paths.
    Ready,
    /// Peer is connected but degraded by a non-fatal condition.
    Degraded,
    /// Peer was lost or disconnected.
    Lost,
}

/// Offer-catalog loading state for one peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferCatalogLoadState {
    /// No catalog path is known.
    Unavailable,
    /// Catalog path exists but loading has not started.
    Available,
    /// Catalog load is in progress.
    Loading,
    /// Catalog load succeeded.
    Loaded,
    /// Catalog load failed.
    Failed,
}

/// Diagnostic failure scope used before projection to `auki-protocol`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipFailureScope {
    /// Peer-level failure.
    Peer,
    /// Domain-level failure.
    Domain,
    /// Offer-catalog failure.
    OfferCatalog,
    /// Offer-level failure.
    Offer,
    /// Generic path failure before the runtime can classify Get or Subscribe.
    Path,
    /// Get path failure.
    Get,
    /// Subscribe path failure.
    Subscribe,
    /// Message envelope or payload failure.
    Message,
    /// Discovery failure.
    Discovery,
    /// Local policy failure.
    Policy,
    /// Transport failure.
    Transport,
}

/// Runtime-owned failure record.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationshipFailureRecord {
    /// Stable failure code.
    pub code: String,
    /// Failure timestamp.
    pub at: String,
    /// Failure scope.
    pub scope: RelationshipFailureScope,
    /// Peer id, when relevant.
    pub peer_id: Option<PeerId>,
    /// Domain id, when relevant.
    pub domain_id: Option<String>,
    /// Offer id, when relevant.
    pub offer_id: Option<String>,
    /// Path id, when relevant.
    pub path_id: Option<String>,
    /// Retry hint.
    pub retryable: Option<bool>,
    /// Diagnostic message.
    pub message: Option<String>,
    /// Structured diagnostic details.
    pub details: Option<Value>,
    /// Aggregated occurrences of the same failure key.
    pub occurrences: u64,
}

/// Domain rejected during handshake or local policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipRejectedDomain {
    /// Rejected domain id, when known.
    pub domain_id: Option<String>,
    /// Stable failure code.
    pub code: String,
    /// Diagnostic message.
    pub message: Option<String>,
}

/// Loaded offer status tracked for projection. The offer-loading boundary will
/// fill this in later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipLoadedOffer {
    /// Domain id for the offer.
    pub domain_id: Option<String>,
    /// Producer-scoped offer id.
    pub offer_id: Option<String>,
    /// Offer kind.
    pub kind: Option<String>,
    /// Producer-reported offer status.
    pub status: Option<String>,
    /// Supported access modes.
    pub access_modes: Vec<String>,
    /// Payload type string, when known.
    pub payload_type: Option<String>,
    /// Registry-reference summaries.
    pub registry_refs: Vec<RelationshipRegistryReferenceStatus>,
    /// Whether the offer is usable locally.
    pub usable: Option<bool>,
    /// Stable unusable reason code.
    pub unusable_reason: Option<String>,
    /// Offer update timestamp.
    pub updated_at: Option<String>,
    /// Offer expiry timestamp.
    pub expires_at: Option<String>,
}

/// Registry-reference status summary for a loaded offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipRegistryReferenceStatus {
    /// Registry namespace.
    pub registry: String,
    /// Role of the referenced entry.
    pub role: String,
    /// Registry-local entry id.
    pub id: String,
    /// `sha256:<base64url>` content hash.
    pub hash: String,
}

/// Active or retained path status tracked for projection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RelationshipPathStatus {
    /// Implementation-defined path id.
    pub path_id: Option<String>,
    /// Path type string, usually `get` or `subscribe`.
    pub path_type: Option<String>,
    /// Domain id for the path.
    pub domain_id: Option<String>,
    /// Offer id for the path.
    pub offer_id: Option<String>,
    /// Path state.
    pub state: Option<String>,
    /// Path start timestamp.
    pub started_at: Option<String>,
    /// Last accepted data-message timestamp.
    pub last_message_at: Option<String>,
    /// Selected payload type.
    pub payload_type: Option<String>,
    /// Last observed sequence value.
    pub last_sequence: Option<u64>,
    /// Sequence gaps observed on this path.
    pub sequence_gap_count: u64,
    /// Last envelope-level failure.
    pub last_envelope_failure: Option<RelationshipFailureRecord>,
    /// Last payload-level failure.
    pub last_payload_failure: Option<RelationshipFailureRecord>,
    /// Last path-level failure.
    pub last_failure: Option<RelationshipFailureRecord>,
}

/// Options for status projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationshipStatusOptions {
    /// Privacy redaction policy.
    pub privacy: StatusPrivacyConfig,
    /// Maximum retained failure records.
    pub retained_status_failures: usize,
    /// Maximum retained path history entries.
    pub completed_path_history: usize,
}

/// Status projection errors.
#[derive(Debug)]
pub enum RelationshipStatusBuildError {
    /// `auki-protocol` rejected the generated status value.
    Status(StatusError),
}

/// Runtime-owned relationship state for one peer.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerRelationship {
    /// Remote libp2p peer id.
    pub peer_id: PeerId,
    /// Runtime lifecycle state.
    pub state: PeerRelationshipState,
    /// Source that introduced the peer.
    pub learned_from: Option<String>,
    /// Whether we have dialable addresses for this peer.
    pub dialable: Option<bool>,
    /// Whether transport is currently connected.
    pub connected: bool,
    /// Runtime-observed active transport paths.
    pub transport_paths: Vec<AukiConnectionPath>,
    /// Whether lifecycle authority validation accepted this peer.
    pub authorized: bool,
    /// Verified wallet public key from the peer binding.
    pub verified_wallet_public_key: Option<String>,
    /// Selected lifecycle version.
    pub selected_lifecycle_version: Option<String>,
    /// Accepted served-domain ids.
    pub accepted_served_domains: Vec<String>,
    /// Rejected domain diagnostics.
    pub rejected_domains: Vec<RelationshipRejectedDomain>,
    /// Offer-catalog state.
    pub offer_catalog_state: OfferCatalogLoadState,
    /// Loaded offers.
    pub loaded_offers: Vec<RelationshipLoadedOffer>,
    /// Active or retained paths.
    pub paths: Vec<RelationshipPathStatus>,
    /// Relationship authority deadline.
    pub authority_deadline: Option<String>,
    /// Last failures, bounded and aggregated.
    pub last_failures: Vec<RelationshipFailureRecord>,
}

impl PeerRelationshipState {
    /// Return the RFC-compatible lifecycle state string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Discovered => "discovered",
            Self::Configured => "configured",
            Self::Dialing => "dialing",
            Self::Connected => "connected",
            Self::Authorized => "authorized",
            Self::LoadingOffers => "loading_offers",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Lost => "lost",
        }
    }
}

impl fmt::Display for PeerRelationshipState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl RelationshipFailureScope {
    /// Return the status-snapshot scope string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Peer => "peer",
            Self::Domain => "domain",
            Self::OfferCatalog => "offer_catalog",
            Self::Offer => "offer",
            Self::Path => "path",
            Self::Get => "get",
            Self::Subscribe => "subscribe",
            Self::Message => "message",
            Self::Discovery => "discovery",
            Self::Policy => "policy",
            Self::Transport => "transport",
        }
    }
}

impl Default for RelationshipStatusOptions {
    fn default() -> Self {
        Self {
            privacy: StatusPrivacyConfig::development(),
            retained_status_failures: 128,
            completed_path_history: 128,
        }
    }
}

impl RelationshipStatusOptions {
    /// Build status options from runtime config values.
    pub fn from_config(config: &crate::AukiP2pConfig) -> Self {
        Self {
            privacy: config.status_privacy,
            retained_status_failures: config.limits.retained_status_failures,
            completed_path_history: config.limits.completed_path_history,
        }
    }
}

impl RelationshipFailureRecord {
    /// Create a peer-scoped failure record.
    pub fn new(
        code: impl Into<String>,
        at: impl Into<String>,
        scope: RelationshipFailureScope,
    ) -> Self {
        Self {
            code: code.into(),
            at: at.into(),
            scope,
            peer_id: None,
            domain_id: None,
            offer_id: None,
            path_id: None,
            retryable: None,
            message: None,
            details: None,
            occurrences: 1,
        }
    }

    fn aggregation_key_eq(&self, other: &Self) -> bool {
        self.code == other.code
            && self.scope == other.scope
            && self.peer_id == other.peer_id
            && self.domain_id == other.domain_id
            && self.offer_id == other.offer_id
            && self.path_id == other.path_id
    }

    fn absorb(&mut self, newer: Self) {
        self.at = newer.at;
        self.retryable = newer.retryable;
        self.message = newer.message;
        self.details = newer.details;
        self.occurrences = self.occurrences.saturating_add(newer.occurrences.max(1));
    }

    fn to_status(&self, privacy: StatusPrivacyConfig) -> Result<FailureRecord, StatusError> {
        let mut object = Map::new();
        object.insert("code".to_owned(), Value::String(self.code.clone()));
        object.insert("at".to_owned(), Value::String(self.at.clone()));
        object.insert(
            "scope".to_owned(),
            Value::String(self.scope.as_str().to_owned()),
        );
        if let Some(peer_id) = self.peer_id {
            object.insert("peer_id".to_owned(), Value::String(peer_id.to_string()));
        }
        if let Some(domain_id) = &self.domain_id {
            object.insert("domain_id".to_owned(), Value::String(domain_id.clone()));
        }
        if let Some(offer_id) = &self.offer_id {
            object.insert("offer_id".to_owned(), Value::String(offer_id.clone()));
        }
        if let Some(path_id) = &self.path_id {
            object.insert("path_id".to_owned(), Value::String(path_id.clone()));
        }
        if let Some(retryable) = self.retryable
            && !privacy.redact_diagnostics
        {
            object.insert("retryable".to_owned(), Value::Bool(retryable));
        }
        if !privacy.redact_diagnostics {
            if let Some(message) = &self.message {
                object.insert("message".to_owned(), Value::String(message.clone()));
            }
            let details = merged_details(self.details.clone(), self.occurrences);
            if let Some(details) = details {
                object.insert("details".to_owned(), details);
            }
        }

        FailureRecord::from_value(Value::Object(object))
    }
}

impl PeerRelationship {
    /// Create an unknown relationship for a peer id.
    pub fn new(peer_id: PeerId) -> Self {
        Self {
            peer_id,
            state: PeerRelationshipState::Unknown,
            learned_from: None,
            dialable: None,
            connected: false,
            transport_paths: Vec::new(),
            authorized: false,
            verified_wallet_public_key: None,
            selected_lifecycle_version: None,
            accepted_served_domains: Vec::new(),
            rejected_domains: Vec::new(),
            offer_catalog_state: OfferCatalogLoadState::Unavailable,
            loaded_offers: Vec::new(),
            paths: Vec::new(),
            authority_deadline: None,
            last_failures: Vec::new(),
        }
    }

    /// Mark a peer as discovered from a source.
    pub fn discovered(&mut self, learned_from: impl Into<String>, dialable: bool) {
        self.state = PeerRelationshipState::Discovered;
        self.learned_from = Some(learned_from.into());
        self.dialable = Some(dialable);
    }

    /// Mark a peer as explicitly configured.
    pub fn configured(&mut self) {
        self.state = PeerRelationshipState::Configured;
        self.learned_from = Some("configured".to_owned());
        self.dialable = Some(true);
    }

    /// Mark the peer as currently being dialed.
    pub fn dialing(&mut self) {
        self.state = PeerRelationshipState::Dialing;
    }

    /// Mark transport as connected.
    pub fn connected(&mut self) {
        self.state = PeerRelationshipState::Connected;
        self.connected = true;
    }

    /// Mark transport as connected and store observed active paths.
    pub fn connected_with_paths(&mut self, paths: Vec<AukiConnectionPath>) {
        self.connected();
        self.transport_paths = paths;
    }

    /// Replace observed active transport paths without changing lifecycle state.
    pub fn set_transport_paths(&mut self, paths: Vec<AukiConnectionPath>) {
        self.transport_paths = paths;
    }

    /// Apply an accepted handshake-policy result.
    pub fn handshake_accepted(&mut self, result: HandshakeValidationResult) {
        self.peer_id = result.authenticated_peer_id;
        self.state = PeerRelationshipState::Authorized;
        self.connected = true;
        self.authorized = true;
        self.verified_wallet_public_key =
            Some(base64url::encode(&result.verified_peer.wallet_public_key.0));
        self.selected_lifecycle_version = Some(result.selected_lifecycle_version);
        self.accepted_served_domains = result
            .accepted_served_domains
            .into_iter()
            .map(|domain| domain.domain_id)
            .collect();
        self.rejected_domains = result
            .rejected_declared_domains
            .into_iter()
            .map(|domain| RelationshipRejectedDomain {
                domain_id: domain.domain_id,
                code: domain.failure_code.to_owned(),
                message: Some(format!("{:?}", domain.reason)),
            })
            .chain(result.policy_rejected_domains.into_iter().map(|domain| {
                RelationshipRejectedDomain {
                    domain_id: Some(domain.domain_id),
                    code: domain.failure_code.to_owned(),
                    message: Some("domain rejected by local policy".to_owned()),
                }
            }))
            .collect();
        self.offer_catalog_state = if result.offer_catalog.is_some() {
            OfferCatalogLoadState::Available
        } else {
            OfferCatalogLoadState::Unavailable
        };
        self.authority_deadline = result.authority_deadline;
    }

    /// Record a fatal handshake failure and move the relationship to degraded.
    pub fn handshake_failed(
        &mut self,
        error: &HandshakePolicyError,
        at: impl Into<String>,
        cap: usize,
    ) {
        self.authorized = false;
        self.state = PeerRelationshipState::Degraded;
        let mut failure = RelationshipFailureRecord::new(
            error.failure_code(),
            at,
            RelationshipFailureScope::Peer,
        );
        failure.peer_id = Some(self.peer_id);
        failure.message = Some(error.to_string());
        self.record_failure(failure, cap);
    }

    /// Mark offer loading as started.
    pub fn loading_offers(&mut self) {
        self.state = PeerRelationshipState::LoadingOffers;
        self.offer_catalog_state = OfferCatalogLoadState::Loading;
    }

    /// Mark offers as loaded and the peer as ready.
    pub fn ready(&mut self) {
        self.state = PeerRelationshipState::Ready;
        self.offer_catalog_state = OfferCatalogLoadState::Loaded;
    }

    /// Mark the relationship as degraded and retain a failure.
    pub fn degraded(&mut self, failure: RelationshipFailureRecord, cap: usize) {
        self.state = PeerRelationshipState::Degraded;
        self.record_failure(failure, cap);
    }

    /// Mark transport as lost.
    pub fn lost(&mut self, at: impl Into<String>, cap: usize) {
        self.state = PeerRelationshipState::Lost;
        self.connected = false;
        self.transport_paths.clear();
        let mut failure = RelationshipFailureRecord::new(
            auki_protocol::v1::error::TRANSPORT_FAILED,
            at,
            RelationshipFailureScope::Transport,
        );
        failure.peer_id = Some(self.peer_id);
        failure.message = Some("transport connection lost".to_owned());
        self.record_failure(failure, cap);
    }

    /// Record a bounded, aggregated failure.
    pub fn record_failure(&mut self, mut failure: RelationshipFailureRecord, cap: usize) {
        if failure.peer_id.is_none() {
            failure.peer_id = Some(self.peer_id);
        }

        if let Some(last) = self.last_failures.last_mut()
            && last.aggregation_key_eq(&failure)
        {
            last.absorb(failure);
            return;
        }

        self.last_failures.push(failure);
        if cap == 0 {
            self.last_failures.clear();
        } else if self.last_failures.len() > cap {
            let excess = self.last_failures.len() - cap;
            self.last_failures.drain(0..excess);
        }
    }

    /// Retain an active or completed path with history cap enforcement.
    pub fn retain_path(&mut self, path: RelationshipPathStatus, cap: usize) {
        self.paths.push(path);
        if cap == 0 {
            self.paths.clear();
        } else if self.paths.len() > cap {
            let excess = self.paths.len() - cap;
            self.paths.drain(0..excess);
        }
    }

    /// Insert or replace a path by `path_id` with history cap enforcement.
    pub fn upsert_path(&mut self, path: RelationshipPathStatus, cap: usize) {
        if let Some(path_id) = &path.path_id
            && let Some(existing) = self
                .paths
                .iter_mut()
                .find(|existing| existing.path_id.as_ref() == Some(path_id))
        {
            *existing = path;
            return;
        }
        self.retain_path(path, cap);
    }

    /// Project this relationship into an RFC status object.
    pub fn to_remote_peer_status(
        &self,
        options: RelationshipStatusOptions,
    ) -> Result<RemotePeerStatus, RelationshipStatusBuildError> {
        let value = self
            .remote_peer_status_value(options)
            .map_err(RelationshipStatusBuildError::Status)?;
        RemotePeerStatus::from_value(value).map_err(RelationshipStatusBuildError::Status)
    }

    fn remote_peer_status_value(
        &self,
        options: RelationshipStatusOptions,
    ) -> Result<Value, StatusError> {
        let mut object = Map::new();
        object.insert(
            "peer_id".to_owned(),
            Value::String(self.peer_id.to_string()),
        );
        if let Some(learned_from) = &self.learned_from
            && !options.privacy.redact_labels
        {
            object.insert(
                "learned_from".to_owned(),
                Value::String(learned_from.clone()),
            );
        }
        if let Some(dialable) = self.dialable {
            object.insert("dialable".to_owned(), Value::Bool(dialable));
        }
        object.insert("connected".to_owned(), Value::Bool(self.connected));
        if !self.transport_paths.is_empty() {
            object.insert(
                "relay_involved".to_owned(),
                Value::Bool(self.transport_paths.iter().any(|path| path.relay_involved)),
            );
            object.insert(
                "transport_paths".to_owned(),
                Value::Array(
                    self.transport_paths
                        .iter()
                        .map(|path| path.to_status_value(!options.privacy.redact_addresses))
                        .collect(),
                ),
            );
        }
        object.insert(
            "lifecycle_state".to_owned(),
            Value::String(self.state.as_str().to_owned()),
        );
        if let Some(version) = &self.selected_lifecycle_version {
            object.insert(
                "selected_protocol_version".to_owned(),
                Value::String(version.clone()),
            );
        }
        object.insert("authorized".to_owned(), Value::Bool(self.authorized));
        if let Some(wallet_public_key) = &self.verified_wallet_public_key {
            object.insert(
                "verified_wallet_public_key".to_owned(),
                Value::String(wallet_public_key.clone()),
            );
        }
        object.insert(
            "accepted_served_domains".to_owned(),
            Value::Array(
                self.accepted_served_domains
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
        object.insert(
            "rejected_domains".to_owned(),
            Value::Array(
                self.rejected_domains
                    .iter()
                    .map(|domain| rejected_domain_value(domain, options.privacy))
                    .collect(),
            ),
        );
        object.insert(
            "offer_catalog_status".to_owned(),
            offer_catalog_status_value(self, options.privacy)?,
        );
        object.insert(
            "loaded_offers".to_owned(),
            Value::Array(
                self.loaded_offers
                    .iter()
                    .map(|offer| loaded_offer_value(self.peer_id, offer))
                    .collect(),
            ),
        );
        if let Some(failure) = self.last_failures.last() {
            let status = failure.to_status(options.privacy)?;
            object.insert("last_failure".to_owned(), status.into_value());
        }

        Ok(Value::Object(object))
    }
}

impl RelationshipPathStatus {
    fn to_status_value(
        &self,
        peer_id: PeerId,
        privacy: StatusPrivacyConfig,
    ) -> Result<Value, StatusError> {
        let mut object = Map::new();
        if let Some(path_id) = &self.path_id {
            object.insert("path_id".to_owned(), Value::String(path_id.clone()));
        }
        if let Some(path_type) = &self.path_type {
            object.insert("path_type".to_owned(), Value::String(path_type.clone()));
        }
        object.insert("peer_id".to_owned(), Value::String(peer_id.to_string()));
        if let Some(domain_id) = &self.domain_id {
            object.insert("domain_id".to_owned(), Value::String(domain_id.clone()));
        }
        if let Some(offer_id) = &self.offer_id {
            object.insert("offer_id".to_owned(), Value::String(offer_id.clone()));
        }
        if let Some(state) = &self.state {
            object.insert("state".to_owned(), Value::String(state.clone()));
        }
        if let Some(started_at) = &self.started_at {
            object.insert("started_at".to_owned(), Value::String(started_at.clone()));
        }
        if let Some(last_message_at) = &self.last_message_at {
            object.insert(
                "last_message_at".to_owned(),
                Value::String(last_message_at.clone()),
            );
        }
        if let Some(payload_type) = &self.payload_type {
            object.insert(
                "payload_type".to_owned(),
                Value::String(payload_type.clone()),
            );
        }
        if let Some(last_sequence) = self.last_sequence {
            object.insert(
                "last_sequence".to_owned(),
                Value::String(last_sequence.to_string()),
            );
        }
        object.insert(
            "sequence_gap_count".to_owned(),
            Value::Number(self.sequence_gap_count.into()),
        );
        if let Some(failure) = &self.last_envelope_failure {
            object.insert(
                "last_envelope_failure".to_owned(),
                failure.to_status(privacy)?.into_value(),
            );
        }
        if let Some(failure) = &self.last_payload_failure {
            object.insert(
                "last_payload_failure".to_owned(),
                failure.to_status(privacy)?.into_value(),
            );
        }
        if let Some(failure) = &self.last_failure {
            object.insert(
                "last_failure".to_owned(),
                failure.to_status(privacy)?.into_value(),
            );
        }
        Ok(Value::Object(object))
    }
}

impl fmt::Display for RelationshipStatusBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(error) => write!(f, "status projection failed: {error}"),
        }
    }
}

impl std::error::Error for RelationshipStatusBuildError {}

/// Build a status snapshot from relationship state.
pub fn build_relationship_status_snapshot(
    generated_at: &str,
    local_peer: LocalPeerStatus,
    relationships: &[PeerRelationship],
    options: RelationshipStatusOptions,
) -> Result<StatusSnapshot, RelationshipStatusBuildError> {
    let remote_peers = relationships
        .iter()
        .map(|relationship| relationship.to_remote_peer_status(options))
        .collect::<Result<Vec<_>, _>>()?;
    let active_paths = relationships
        .iter()
        .flat_map(|relationship| {
            relationship
                .paths
                .iter()
                .rev()
                .map(|path| path.to_status_value(relationship.peer_id, options.privacy))
        })
        .take(options.completed_path_history)
        .map(|value| value.and_then(PathStatus::from_value))
        .collect::<Result<Vec<_>, _>>()
        .map_err(RelationshipStatusBuildError::Status)?;
    let last_failures = relationships
        .iter()
        .flat_map(|relationship| relationship.last_failures.iter().rev())
        .take(options.retained_status_failures)
        .map(|failure| failure.to_status(options.privacy))
        .collect::<Result<Vec<_>, _>>()
        .map_err(RelationshipStatusBuildError::Status)?;

    StatusSnapshot::create(
        generated_at,
        local_peer,
        Vec::new(),
        remote_peers,
        active_paths,
        last_failures,
        None,
        None,
    )
    .map_err(RelationshipStatusBuildError::Status)
}

fn rejected_domain_value(
    domain: &RelationshipRejectedDomain,
    privacy: StatusPrivacyConfig,
) -> Value {
    let mut object = Map::new();
    if let Some(domain_id) = &domain.domain_id {
        object.insert("domain_id".to_owned(), Value::String(domain_id.clone()));
    }
    object.insert("code".to_owned(), Value::String(domain.code.clone()));
    if !privacy.redact_diagnostics
        && let Some(message) = &domain.message
    {
        object.insert("message".to_owned(), Value::String(message.clone()));
    }
    Value::Object(object)
}

fn offer_catalog_status_value(
    relationship: &PeerRelationship,
    privacy: StatusPrivacyConfig,
) -> Result<Value, StatusError> {
    let mut object = Map::new();
    object.insert(
        "path_available".to_owned(),
        Value::Bool(relationship.offer_catalog_state != OfferCatalogLoadState::Unavailable),
    );
    if relationship.offer_catalog_state == OfferCatalogLoadState::Failed
        && let Some(failure) = relationship.last_failures.last()
    {
        let status = failure.to_status(privacy)?;
        object.insert("last_failure".to_owned(), status.into_value());
    }
    Ok(Value::Object(object))
}

fn loaded_offer_value(peer_id: PeerId, offer: &RelationshipLoadedOffer) -> Value {
    let mut object = Map::new();
    object.insert("peer_id".to_owned(), Value::String(peer_id.to_string()));
    if let Some(domain_id) = &offer.domain_id {
        object.insert("domain_id".to_owned(), Value::String(domain_id.clone()));
    }
    if let Some(offer_id) = &offer.offer_id {
        object.insert("offer_id".to_owned(), Value::String(offer_id.clone()));
    }
    if let Some(kind) = &offer.kind {
        object.insert("kind".to_owned(), Value::String(kind.clone()));
    }
    if let Some(status) = &offer.status {
        object.insert("status".to_owned(), Value::String(status.clone()));
    }
    object.insert(
        "access_modes".to_owned(),
        Value::Array(
            offer
                .access_modes
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    if let Some(payload_type) = &offer.payload_type {
        object.insert(
            "payload_type".to_owned(),
            Value::String(payload_type.clone()),
        );
    }
    object.insert(
        "registry_refs".to_owned(),
        Value::Array(
            offer
                .registry_refs
                .iter()
                .map(registry_reference_status_value)
                .collect(),
        ),
    );
    if let Some(usable) = offer.usable {
        object.insert("usable".to_owned(), Value::Bool(usable));
    }
    if let Some(reason) = &offer.unusable_reason {
        object.insert("unusable_reason".to_owned(), Value::String(reason.clone()));
    }
    if let Some(updated_at) = &offer.updated_at {
        object.insert("updated_at".to_owned(), Value::String(updated_at.clone()));
    }
    if let Some(expires_at) = &offer.expires_at {
        object.insert("expires_at".to_owned(), Value::String(expires_at.clone()));
    }
    Value::Object(object)
}

fn registry_reference_status_value(reference: &RelationshipRegistryReferenceStatus) -> Value {
    let mut object = Map::new();
    object.insert(
        "registry".to_owned(),
        Value::String(reference.registry.clone()),
    );
    object.insert("role".to_owned(), Value::String(reference.role.clone()));
    object.insert("id".to_owned(), Value::String(reference.id.clone()));
    object.insert("hash".to_owned(), Value::String(reference.hash.clone()));
    Value::Object(object)
}

fn merged_details(details: Option<Value>, occurrences: u64) -> Option<Value> {
    if occurrences <= 1 {
        return details;
    }

    let mut object = match details {
        Some(Value::Object(object)) => object,
        Some(value) => {
            let mut object = Map::new();
            object.insert("detail".to_owned(), value);
            object
        }
        None => Map::new(),
    };
    object.insert("occurrences".to_owned(), Value::Number(occurrences.into()));
    Some(Value::Object(object))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AukiConnectionDirection, AukiP2pConfig, AukiTransportProtocol, LocalPeerIdentity,
        build_local_handshake, validate_remote_handshake,
    };
    use auki_identity::Wallet;
    use auki_protocol::v1::{
        authority::{DeclaredDomain, ServedDomainAuthority},
        domain::{DOMAIN_NONCE_LEN, DomainDeclaration},
        error,
    };

    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";
    const NOW: &str = "2026-05-26T12:00:00Z";
    const NONCE: [u8; DOMAIN_NONCE_LEN] = [11u8; DOMAIN_NONCE_LEN];

    fn identity(seed: u8) -> LocalPeerIdentity {
        let wallet = Wallet::from_seed(vec![seed; 32]).expect("32-byte seed");
        LocalPeerIdentity::from_wallet(wallet, ISSUED_AT, Some("relationship-test"))
            .expect("local peer identity")
    }

    fn direct_owner_result(identity: &LocalPeerIdentity) -> HandshakeValidationResult {
        let declaration =
            DomainDeclaration::create(identity.wallet(), &NONCE, Some("lab")).unwrap();
        let domain_id = declaration.domain_id().unwrap().to_owned();
        let declared = DeclaredDomain::new(domain_id, declaration, None);
        let handshake = build_local_handshake(identity, vec![declared]);
        validate_remote_handshake(crate::HandshakeValidationInput::new(
            &identity.peer_id(),
            &handshake,
            &AukiP2pConfig::development(),
            NOW,
        ))
        .expect("valid handshake")
    }

    fn transport_path(relay_involved: bool) -> AukiConnectionPath {
        AukiConnectionPath {
            direction: AukiConnectionDirection::Dialer,
            transport: AukiTransportProtocol::WebSocket,
            relay_involved,
            local_address: None,
            remote_address: "/ip4/127.0.0.1/tcp/4001/ws".parse().unwrap(),
        }
    }

    #[test]
    fn state_transitions_follow_relationship_lifecycle() {
        let peer = identity(61).peer_id();
        let mut relationship = PeerRelationship::new(peer);

        assert_eq!(relationship.state, PeerRelationshipState::Unknown);
        relationship.discovered("discovery", true);
        assert_eq!(relationship.state, PeerRelationshipState::Discovered);
        relationship.configured();
        assert_eq!(relationship.state, PeerRelationshipState::Configured);
        relationship.dialing();
        assert_eq!(relationship.state, PeerRelationshipState::Dialing);
        relationship.connected();
        assert_eq!(relationship.state, PeerRelationshipState::Connected);
        relationship.loading_offers();
        assert_eq!(relationship.state, PeerRelationshipState::LoadingOffers);
        relationship.ready();
        assert_eq!(relationship.state, PeerRelationshipState::Ready);
        relationship.lost("2026-05-26T12:01:00Z", 8);
        assert_eq!(relationship.state, PeerRelationshipState::Lost);
        assert!(!relationship.connected);
    }

    #[test]
    fn handshake_acceptance_populates_authorized_relationship() {
        let identity = identity(62);
        let result = direct_owner_result(&identity);
        let domain_id = result.accepted_served_domains[0].domain_id.clone();
        assert_eq!(
            result.accepted_served_domains[0].authority,
            ServedDomainAuthority::DirectOwner
        );
        let mut relationship = PeerRelationship::new(identity.peer_id());

        relationship.handshake_accepted(result);

        assert_eq!(relationship.state, PeerRelationshipState::Authorized);
        assert!(relationship.connected);
        assert!(relationship.authorized);
        assert_eq!(relationship.accepted_served_domains, vec![domain_id]);
        assert_eq!(
            relationship.offer_catalog_state,
            OfferCatalogLoadState::Unavailable
        );
        assert!(relationship.verified_wallet_public_key.is_some());
    }

    #[test]
    fn handshake_failure_records_bounded_status_failure() {
        let identity = identity(63);
        let mut relationship = PeerRelationship::new(identity.peer_id());
        let error = HandshakePolicyError::MissingAppPeerAuthorizationDecision;

        relationship.handshake_failed(&error, "2026-05-26T12:02:00Z", 2);

        assert_eq!(relationship.state, PeerRelationshipState::Degraded);
        assert_eq!(relationship.last_failures.len(), 1);
        assert_eq!(
            relationship.last_failures[0].code,
            error::AUTHORIZATION_PEER_REJECTED
        );
    }

    #[test]
    fn repeated_failures_are_aggregated_and_history_is_capped() {
        let identity = identity(64);
        let mut relationship = PeerRelationship::new(identity.peer_id());
        for index in 0..5 {
            let mut failure = RelationshipFailureRecord::new(
                error::MESSAGE_SEQUENCE_GAP,
                format!("2026-05-26T12:00:0{index}Z"),
                RelationshipFailureScope::Message,
            );
            failure.path_id = Some("path-a".to_owned());
            relationship.record_failure(failure, 4);
        }

        assert_eq!(relationship.last_failures.len(), 1);
        assert_eq!(relationship.last_failures[0].occurrences, 5);

        for index in 0..5 {
            relationship.record_failure(
                RelationshipFailureRecord::new(
                    format!("code.{index}"),
                    format!("2026-05-26T12:01:0{index}Z"),
                    RelationshipFailureScope::Peer,
                ),
                3,
            );
        }
        assert_eq!(relationship.last_failures.len(), 3);
        assert_eq!(relationship.last_failures[0].code, "code.2");
    }

    #[test]
    fn remote_peer_status_projects_relationship_without_transport_internals() {
        let identity = identity(65);
        let mut relationship = PeerRelationship::new(identity.peer_id());
        relationship.handshake_accepted(direct_owner_result(&identity));
        relationship.ready();

        let status = relationship
            .to_remote_peer_status(RelationshipStatusOptions::default())
            .expect("status projection");

        assert_eq!(
            status.peer_id.as_deref(),
            Some(identity.peer_id().to_string().as_str())
        );
        assert_eq!(status.lifecycle_state.as_deref(), Some("ready"));
        assert_eq!(status.authorized, Some(true));
        assert_eq!(status.accepted_served_domains.len(), 1);
        assert!(status.verified_wallet_public_key.is_some());
    }

    #[test]
    fn remote_peer_status_reports_transport_path_and_relay_involvement() {
        let identity = identity(95);
        let mut relationship = PeerRelationship::new(identity.peer_id());
        relationship.connected_with_paths(vec![transport_path(true)]);

        let status = relationship
            .to_remote_peer_status(RelationshipStatusOptions::default())
            .expect("status projection");
        let path = status
            .value()
            .get("transport_paths")
            .and_then(Value::as_array)
            .and_then(|paths| paths.first())
            .expect("transport path");

        assert_eq!(
            status
                .value()
                .get("relay_involved")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            path.get("direction").and_then(Value::as_str),
            Some("dialer")
        );
        assert_eq!(
            path.get("transport").and_then(Value::as_str),
            Some("websocket")
        );
        assert_eq!(
            path.get("relay_involved").and_then(Value::as_bool),
            Some(true)
        );
        assert!(path.get("remote_address").is_some());
        assert_eq!(status.authorized, Some(false));
    }

    #[test]
    fn transport_path_status_redacts_addresses_without_hiding_relay_state() {
        let identity = identity(96);
        let mut relationship = PeerRelationship::new(identity.peer_id());
        relationship.connected_with_paths(vec![transport_path(true)]);
        let options = RelationshipStatusOptions {
            privacy: StatusPrivacyConfig {
                redact_addresses: true,
                ..StatusPrivacyConfig::development()
            },
            ..RelationshipStatusOptions::default()
        };

        let status = relationship
            .to_remote_peer_status(options)
            .expect("status projection");
        let path = status
            .value()
            .get("transport_paths")
            .and_then(Value::as_array)
            .and_then(|paths| paths.first())
            .expect("transport path");

        assert_eq!(
            status
                .value()
                .get("relay_involved")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            path.get("transport").and_then(Value::as_str),
            Some("websocket")
        );
        assert_eq!(
            path.get("relay_involved").and_then(Value::as_bool),
            Some(true)
        );
        assert!(path.get("remote_address").is_none());
        assert!(path.get("local_address").is_none());
    }

    #[test]
    fn privacy_redaction_removes_diagnostic_messages() {
        let identity = identity(66);
        let mut relationship = PeerRelationship::new(identity.peer_id());
        let mut failure = RelationshipFailureRecord::new(
            error::TRANSPORT_FAILED,
            "2026-05-26T12:03:00Z",
            RelationshipFailureScope::Transport,
        );
        failure.message = Some("full diagnostic".to_owned());
        relationship.record_failure(failure, 8);
        relationship
            .rejected_domains
            .push(RelationshipRejectedDomain {
                domain_id: None,
                code: error::POLICY_DOMAIN_REJECTED.to_owned(),
                message: Some("domain diagnostic".to_owned()),
            });
        let options = RelationshipStatusOptions {
            privacy: StatusPrivacyConfig::production_recommended(),
            ..RelationshipStatusOptions::default()
        };

        let status = relationship
            .to_remote_peer_status(options)
            .expect("status projection");

        assert_eq!(status.last_failure.unwrap().message, None);
        assert_eq!(status.rejected_domains[0].message, None);
    }

    #[test]
    fn privacy_redaction_removes_relationship_source_labels() {
        let identity = identity(67);
        let mut relationship = PeerRelationship::new(identity.peer_id());
        relationship.discovered("peer_graph_hint", true);
        let options = RelationshipStatusOptions {
            privacy: StatusPrivacyConfig {
                redact_labels: true,
                redact_diagnostics: false,
                ..StatusPrivacyConfig::development()
            },
            ..RelationshipStatusOptions::default()
        };

        let status = relationship
            .to_remote_peer_status(options)
            .expect("status projection");

        assert_eq!(status.learned_from, None);
        assert_eq!(status.dialable, Some(true));
    }

    #[test]
    fn status_snapshot_builder_includes_relationships_paths_and_failures() {
        let identity = identity(68);
        let mut relationship = PeerRelationship::new(identity.peer_id());
        relationship.handshake_accepted(direct_owner_result(&identity));
        relationship.retain_path(
            RelationshipPathStatus {
                path_id: Some("path-1".to_owned()),
                path_type: Some("subscribe".to_owned()),
                domain_id: relationship.accepted_served_domains.first().cloned(),
                offer_id: Some("offer-1".to_owned()),
                state: Some("active".to_owned()),
                sequence_gap_count: 2,
                ..RelationshipPathStatus::default()
            },
            8,
        );
        relationship.record_failure(
            RelationshipFailureRecord::new(
                error::MESSAGE_SEQUENCE_GAP,
                "2026-05-26T12:04:00Z",
                RelationshipFailureScope::Message,
            ),
            8,
        );
        let local_peer = LocalPeerStatus::from_value(Value::Object(Map::new())).unwrap();

        let snapshot = build_relationship_status_snapshot(
            "2026-05-26T12:05:00Z",
            local_peer,
            &[relationship],
            RelationshipStatusOptions::default(),
        )
        .expect("status snapshot");

        assert_eq!(snapshot.remote_peers.len(), 1);
        assert_eq!(snapshot.active_paths.len(), 1);
        assert_eq!(snapshot.last_failures.len(), 1);
    }

    #[test]
    fn status_snapshot_builder_applies_global_history_caps() {
        let first = identity(69);
        let second = identity(70);
        let mut first_relationship = PeerRelationship::new(first.peer_id());
        let mut second_relationship = PeerRelationship::new(second.peer_id());
        for index in 0..2 {
            first_relationship.retain_path(
                RelationshipPathStatus {
                    path_id: Some(format!("first-path-{index}")),
                    path_type: Some("get".to_owned()),
                    domain_id: None,
                    offer_id: None,
                    state: Some("failed".to_owned()),
                    sequence_gap_count: 0,
                    ..RelationshipPathStatus::default()
                },
                8,
            );
            second_relationship.retain_path(
                RelationshipPathStatus {
                    path_id: Some(format!("second-path-{index}")),
                    path_type: Some("subscribe".to_owned()),
                    domain_id: None,
                    offer_id: None,
                    state: Some("ended".to_owned()),
                    sequence_gap_count: index,
                    ..RelationshipPathStatus::default()
                },
                8,
            );
            first_relationship.record_failure(
                RelationshipFailureRecord::new(
                    format!("first.failure.{index}"),
                    format!("2026-05-26T12:10:0{index}Z"),
                    RelationshipFailureScope::Get,
                ),
                8,
            );
            second_relationship.record_failure(
                RelationshipFailureRecord::new(
                    format!("second.failure.{index}"),
                    format!("2026-05-26T12:11:0{index}Z"),
                    RelationshipFailureScope::Subscribe,
                ),
                8,
            );
        }
        let local_peer = LocalPeerStatus::from_value(Value::Object(Map::new())).unwrap();
        let options = RelationshipStatusOptions {
            retained_status_failures: 2,
            completed_path_history: 2,
            ..RelationshipStatusOptions::default()
        };

        let snapshot = build_relationship_status_snapshot(
            "2026-05-26T12:12:00Z",
            local_peer,
            &[first_relationship, second_relationship],
            options,
        )
        .expect("status snapshot");

        assert_eq!(snapshot.active_paths.len(), 2);
        assert_eq!(snapshot.last_failures.len(), 2);
        assert_eq!(
            snapshot.last_failures[0].scope,
            RelationshipFailureScope::Get.as_str()
        );
    }

    #[test]
    fn invalid_failure_record_fails_status_projection() {
        let identity = identity(71);
        let mut relationship = PeerRelationship::new(identity.peer_id());
        relationship.record_failure(
            RelationshipFailureRecord::new(
                error::TRANSPORT_FAILED,
                "not-a-timestamp",
                RelationshipFailureScope::Transport,
            ),
            8,
        );

        let error = relationship
            .to_remote_peer_status(RelationshipStatusOptions::default())
            .expect_err("invalid failure timestamp should fail projection");

        assert!(matches!(error, RelationshipStatusBuildError::Status(_)));
    }

    #[test]
    fn app_policy_rejection_is_stored_as_rejected_domain_status() {
        let identity = identity(72);
        let mut result = direct_owner_result(&identity);
        let domain_id = result.accepted_served_domains[0].domain_id.clone();
        result
            .policy_rejected_domains
            .push(crate::PolicyRejectedDomain {
                domain_id: domain_id.clone(),
                failure_code: error::POLICY_DOMAIN_REJECTED,
            });
        result.accepted_served_domains.clear();
        let mut relationship = PeerRelationship::new(identity.peer_id());

        relationship.handshake_accepted(result);
        let status = relationship
            .to_remote_peer_status(RelationshipStatusOptions::default())
            .expect("status projection");

        assert_eq!(status.rejected_domains.len(), 1);
        assert_eq!(
            status.rejected_domains[0].domain_id.as_deref(),
            Some(domain_id.as_str())
        );
        assert_eq!(
            status.rejected_domains[0].code.as_deref(),
            Some(error::POLICY_DOMAIN_REJECTED)
        );
    }
}
