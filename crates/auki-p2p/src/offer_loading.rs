//! Offer-catalog loading boundary for peer relationships.

use crate::{
    AukiP2pConfig, OfferCatalogLoadState, OfferPolicy, PeerRelationship, RelationshipFailureRecord,
    RelationshipFailureScope, RelationshipLoadedOffer, RelationshipRegistryReferenceStatus,
};
use auki_protocol::v1::{
    error,
    frame::{FrameError, decode_json_frame},
    offer::{
        Offer, OfferAccessMode, OfferCatalogResponse, OfferCatalogResponseError,
        OfferUsabilityInput, PolicyDecision, evaluate_offer_usability,
    },
};
use libp2p_identity::PeerId;
use serde_json::Value;
use std::fmt;

/// Internal boundary used by the runtime to fetch one complete offer-catalog
/// response frame from a remote peer.
pub trait OfferCatalogClient {
    /// Fetch one encoded v1 offer-catalog response frame.
    fn fetch_offer_catalog_frame(
        &mut self,
        peer_id: PeerId,
    ) -> Result<Vec<u8>, OfferCatalogClientError>;
}

/// Transport/client failure while fetching an offer catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferCatalogClientError {
    /// Stable failure code.
    pub code: &'static str,
    /// Diagnostic message.
    pub message: String,
    /// Retry hint for local status only.
    pub retryable: bool,
}

/// Application-selected offer allowed by local `OfferPolicy::AppPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppAllowedOffer<'a> {
    /// Offer domain id.
    pub domain_id: &'a str,
    /// Producer-scoped offer id.
    pub offer_id: &'a str,
}

/// Application offer-policy decision for this catalog load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppOfferPolicy<'a> {
    /// No application policy decision was supplied.
    NotProvided,
    /// Application policy allows every otherwise-usable offer.
    AllowAll,
    /// Application policy allows only these domain/offer tuples.
    AllowOnly {
        /// Allowed offer tuples.
        offers: &'a [AppAllowedOffer<'a>],
        /// Failure code to report for offers outside the allow-list.
        failure_code: &'static str,
    },
    /// Application policy rejects every offer.
    RejectAll {
        /// Failure code to report for rejected offers.
        failure_code: &'static str,
    },
}

/// Inputs for validating and loading one offer-catalog response.
#[derive(Debug, Clone, Copy)]
pub struct OfferLoadContext<'a> {
    /// Runtime config and limits.
    pub config: &'a AukiP2pConfig,
    /// Current UTC time used for status failure records and offer freshness.
    pub now: &'a str,
    /// Access mode the caller intends to use, if known.
    pub requested_access_mode: Option<OfferAccessMode>,
    /// Locally supported offer kinds. `None` defers this check.
    pub supported_kinds: Option<&'a [String]>,
    /// Locally supported payload types. `None` defers this check.
    pub supported_payload_types: Option<&'a [String]>,
    /// Application offer-policy decision when config uses app-policy.
    pub app_offer_policy: AppOfferPolicy<'a>,
}

/// Result of a successful catalog load.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferLoadReport {
    /// Remote peer whose catalog was loaded.
    pub peer_id: PeerId,
    /// Loaded offers with local usability state.
    pub offers: Vec<LoadedRemoteOffer>,
    /// Producer diagnostics carried by the catalog response.
    pub diagnostics: Vec<Value>,
    /// Response generation timestamp, when present.
    pub generated_at: Option<String>,
}

/// Runtime-owned loaded offer with local usability state.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedRemoteOffer {
    /// Parsed protocol offer.
    pub offer: Offer,
    /// Whether the offer is usable locally.
    pub usable: bool,
    /// Stable failure code when the offer is known but unusable.
    pub unusable_reason: Option<&'static str>,
}

/// Lookup failure for a loaded catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferLookupError {
    /// Domain id used for lookup.
    pub domain_id: String,
    /// Offer id used for lookup.
    pub offer_id: String,
}

/// Fatal offer-loading failure.
#[derive(Debug, Clone, PartialEq)]
pub enum OfferLoadError {
    /// Relationship has not completed authority validation.
    RelationshipNotAuthorized {
        /// Remote peer id.
        peer_id: PeerId,
    },
    /// Offer-catalog client failed before a catalog frame was available.
    Client(OfferCatalogClientError),
    /// Catalog frame validation failed.
    CatalogFrame(FrameError),
    /// More bytes were present after the first complete catalog frame.
    TrailingFrameBytes {
        /// Bytes consumed by the decoded frame.
        consumed: usize,
        /// Total bytes supplied.
        total: usize,
    },
    /// Protocol catalog parsing failed.
    CatalogResponse(OfferCatalogResponseError),
    /// Catalog contained too many offers.
    TooManyOffers {
        /// Actual offer count.
        actual: usize,
        /// Local maximum.
        max: usize,
    },
    /// One offer contained too many registry references.
    TooManyRegistryRefs {
        /// Offer domain id.
        domain_id: String,
        /// Offer id.
        offer_id: String,
        /// Actual registry-reference count.
        actual: usize,
        /// Local maximum.
        max: usize,
    },
    /// Inline canonical registry JSON exceeded the local byte limit.
    InlineRegistryJsonTooLarge {
        /// Offer domain id.
        domain_id: String,
        /// Offer id.
        offer_id: String,
        /// Registry namespace.
        registry: String,
        /// Registry entry id.
        id: String,
        /// Actual byte length.
        actual: u64,
        /// Local maximum.
        max: u64,
    },
    /// Offer metadata exceeded the local byte limit.
    MetadataTooLarge {
        /// Offer domain id.
        domain_id: String,
        /// Offer id.
        offer_id: String,
        /// Actual serialized byte length.
        actual: u64,
        /// Local maximum.
        max: u64,
    },
    /// Config requires app offer policy but no app decision was supplied.
    MissingAppOfferPolicyDecision,
}

impl OfferCatalogClientError {
    /// Create a client failure.
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }
}

impl Default for AppOfferPolicy<'_> {
    fn default() -> Self {
        Self::NotProvided
    }
}

impl<'a> OfferLoadContext<'a> {
    /// Create an offer-load context with only mandatory inputs.
    pub fn new(config: &'a AukiP2pConfig, now: &'a str) -> Self {
        Self {
            config,
            now,
            requested_access_mode: None,
            supported_kinds: None,
            supported_payload_types: None,
            app_offer_policy: AppOfferPolicy::NotProvided,
        }
    }
}

impl OfferLoadReport {
    /// Find one loaded offer by its producing domain and offer id.
    pub fn find_offer(
        &self,
        domain_id: &str,
        offer_id: &str,
    ) -> Result<&LoadedRemoteOffer, OfferLookupError> {
        self.offers
            .iter()
            .find(|loaded| loaded.offer.domain_id == domain_id && loaded.offer.offer_id == offer_id)
            .ok_or_else(|| OfferLookupError {
                domain_id: domain_id.to_owned(),
                offer_id: offer_id.to_owned(),
            })
    }
}

impl OfferLookupError {
    /// Stable RFC failure code.
    pub fn failure_code(&self) -> &'static str {
        error::OFFER_UNKNOWN_OFFER
    }
}

impl OfferLoadError {
    /// Stable RFC failure code.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::RelationshipNotAuthorized { .. } => error::OFFER_LOAD_FAILED,
            Self::Client(error) => error.code,
            Self::CatalogFrame(FrameError::BodyTooLarge { .. })
            | Self::InlineRegistryJsonTooLarge { .. }
            | Self::MetadataTooLarge { .. } => error::MESSAGE_PAYLOAD_TOO_LARGE,
            Self::CatalogFrame(_) | Self::TrailingFrameBytes { .. } => error::TRANSPORT_FAILED,
            Self::CatalogResponse(error) => error.failure_code(),
            Self::TooManyOffers { .. } | Self::TooManyRegistryRefs { .. } => {
                error::OFFER_INVALID_CATALOG_RESPONSE
            }
            Self::MissingAppOfferPolicyDecision => error::OFFER_LOAD_FAILED,
        }
    }

    fn relationship_failure_scope(&self) -> RelationshipFailureScope {
        match self {
            Self::MissingAppOfferPolicyDecision => RelationshipFailureScope::Policy,
            _ => RelationshipFailureScope::OfferCatalog,
        }
    }

    fn retryable(&self) -> Option<bool> {
        match self {
            Self::Client(error) => Some(error.retryable),
            Self::CatalogFrame(_) | Self::TrailingFrameBytes { .. } => Some(true),
            Self::RelationshipNotAuthorized { .. }
            | Self::CatalogResponse(_)
            | Self::TooManyOffers { .. }
            | Self::TooManyRegistryRefs { .. }
            | Self::InlineRegistryJsonTooLarge { .. }
            | Self::MetadataTooLarge { .. }
            | Self::MissingAppOfferPolicyDecision => Some(false),
        }
    }
}

impl fmt::Display for OfferLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown offer {}/{}", self.domain_id, self.offer_id)
    }
}

impl std::error::Error for OfferLookupError {}

impl fmt::Display for OfferLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelationshipNotAuthorized { peer_id } => {
                write!(f, "cannot load offers before peer is authorized: {peer_id}")
            }
            Self::Client(error) => write!(f, "offer catalog client failed: {}", error.message),
            Self::CatalogFrame(error) => write!(f, "offer catalog frame failed: {error}"),
            Self::TrailingFrameBytes { consumed, total } => write!(
                f,
                "offer catalog frame had trailing bytes: consumed {consumed}, total {total}"
            ),
            Self::CatalogResponse(error) => write!(f, "invalid offer catalog response: {error}"),
            Self::TooManyOffers { actual, max } => {
                write!(f, "too many offers in catalog: {actual} (max {max})")
            }
            Self::TooManyRegistryRefs {
                domain_id,
                offer_id,
                actual,
                max,
            } => write!(
                f,
                "too many registry refs in offer {domain_id}/{offer_id}: {actual} (max {max})"
            ),
            Self::InlineRegistryJsonTooLarge {
                domain_id,
                offer_id,
                registry,
                id,
                actual,
                max,
            } => write!(
                f,
                "inline registry json too large in offer {domain_id}/{offer_id} ref {registry}/{id}: {actual} bytes (max {max})"
            ),
            Self::MetadataTooLarge {
                domain_id,
                offer_id,
                actual,
                max,
            } => write!(
                f,
                "offer metadata too large in {domain_id}/{offer_id}: {actual} bytes (max {max})"
            ),
            Self::MissingAppOfferPolicyDecision => {
                write!(f, "missing app offer-policy decision")
            }
        }
    }
}

impl std::error::Error for OfferLoadError {}

/// Fetch and load a remote offer catalog through an internal client boundary.
pub fn load_remote_offers_with_client<C: OfferCatalogClient>(
    relationship: &mut PeerRelationship,
    client: &mut C,
    context: OfferLoadContext<'_>,
) -> Result<OfferLoadReport, OfferLoadError> {
    if !relationship.authorized {
        let error = OfferLoadError::RelationshipNotAuthorized {
            peer_id: relationship.peer_id,
        };
        mark_offer_load_failed(relationship, &error, context);
        return Err(error);
    }

    let frame = match client.fetch_offer_catalog_frame(relationship.peer_id) {
        Ok(frame) => frame,
        Err(error) => {
            let error = OfferLoadError::Client(error);
            mark_offer_load_failed(relationship, &error, context);
            return Err(error);
        }
    };
    load_remote_offers_from_frame(relationship, &frame, context)
}

/// Load remote offers from one complete v1 offer-catalog response frame.
pub fn load_remote_offers_from_frame(
    relationship: &mut PeerRelationship,
    frame: &[u8],
    context: OfferLoadContext<'_>,
) -> Result<OfferLoadReport, OfferLoadError> {
    if !relationship.authorized {
        let error = OfferLoadError::RelationshipNotAuthorized {
            peer_id: relationship.peer_id,
        };
        mark_offer_load_failed(relationship, &error, context);
        return Err(error);
    }

    relationship.loading_offers();
    match load_remote_offers_from_frame_inner(relationship.peer_id, frame, relationship, context) {
        Ok(report) => {
            relationship.loaded_offers = report
                .offers
                .iter()
                .map(relationship_loaded_offer)
                .collect();
            relationship.ready();
            Ok(report)
        }
        Err(error) => {
            mark_offer_load_failed(relationship, &error, context);
            Err(error)
        }
    }
}

fn load_remote_offers_from_frame_inner(
    peer_id: PeerId,
    frame: &[u8],
    relationship: &PeerRelationship,
    context: OfferLoadContext<'_>,
) -> Result<OfferLoadReport, OfferLoadError> {
    let (value, consumed) = decode_json_frame(
        frame,
        context.config.limits.catalog_response_frame_body_bytes,
    )
    .map_err(OfferLoadError::CatalogFrame)?;
    if consumed != frame.len() {
        return Err(OfferLoadError::TrailingFrameBytes {
            consumed,
            total: frame.len(),
        });
    }

    let response =
        OfferCatalogResponse::from_value(value).map_err(OfferLoadError::CatalogResponse)?;
    enforce_catalog_limits(&response, context.config)?;
    let offers = response
        .offers
        .iter()
        .map(|offer| evaluate_loaded_offer(offer, relationship, context))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(OfferLoadReport {
        peer_id,
        offers,
        diagnostics: response.diagnostics,
        generated_at: response.generated_at,
    })
}

fn evaluate_loaded_offer(
    offer: &Offer,
    relationship: &PeerRelationship,
    context: OfferLoadContext<'_>,
) -> Result<LoadedRemoteOffer, OfferLoadError> {
    let offer_policy = offer_policy_decision(context.config.offer_policy, context, offer)?;
    let result = evaluate_offer_usability(OfferUsabilityInput {
        offer,
        accepted_served_domain_ids: &relationship.accepted_served_domains,
        requested_access_mode: context.requested_access_mode,
        supported_kinds: context.supported_kinds,
        supported_payload_types: context.supported_payload_types,
        now: Some(context.now),
        domain_policy: PolicyDecision::Allow,
        offer_policy,
    });

    Ok(match result {
        Ok(_) => LoadedRemoteOffer {
            offer: offer.clone(),
            usable: true,
            unusable_reason: None,
        },
        Err(error) => LoadedRemoteOffer {
            offer: offer.clone(),
            usable: false,
            unusable_reason: Some(error.failure_code()),
        },
    })
}

fn offer_policy_decision(
    offer_policy: OfferPolicy,
    context: OfferLoadContext<'_>,
    offer: &Offer,
) -> Result<PolicyDecision, OfferLoadError> {
    match offer_policy {
        OfferPolicy::AllowAll => Ok(PolicyDecision::Allow),
        OfferPolicy::AppPolicy => match context.app_offer_policy {
            AppOfferPolicy::NotProvided => Err(OfferLoadError::MissingAppOfferPolicyDecision),
            AppOfferPolicy::AllowAll => Ok(PolicyDecision::Allow),
            AppOfferPolicy::AllowOnly {
                offers,
                failure_code,
            } => {
                if offers.iter().any(|allowed| {
                    allowed.domain_id == offer.domain_id && allowed.offer_id == offer.offer_id
                }) {
                    Ok(PolicyDecision::Allow)
                } else {
                    Ok(PolicyDecision::Reject { failure_code })
                }
            }
            AppOfferPolicy::RejectAll { failure_code } => {
                Ok(PolicyDecision::Reject { failure_code })
            }
        },
    }
}

fn enforce_catalog_limits(
    response: &OfferCatalogResponse,
    config: &AukiP2pConfig,
) -> Result<(), OfferLoadError> {
    let limits = config.limits;
    if response.offers.len() > limits.offers_per_catalog {
        return Err(OfferLoadError::TooManyOffers {
            actual: response.offers.len(),
            max: limits.offers_per_catalog,
        });
    }

    for offer in &response.offers {
        if offer.registry_refs.len() > limits.registry_refs_per_offer {
            return Err(OfferLoadError::TooManyRegistryRefs {
                domain_id: offer.domain_id.clone(),
                offer_id: offer.offer_id.clone(),
                actual: offer.registry_refs.len(),
                max: limits.registry_refs_per_offer,
            });
        }

        for reference in &offer.registry_refs {
            if let Some(canonical_json) = &reference.canonical_json {
                let actual = canonical_json.len() as u64;
                if actual > limits.inline_registry_json_bytes {
                    return Err(OfferLoadError::InlineRegistryJsonTooLarge {
                        domain_id: offer.domain_id.clone(),
                        offer_id: offer.offer_id.clone(),
                        registry: reference.registry.clone(),
                        id: reference.id.clone(),
                        actual,
                        max: limits.inline_registry_json_bytes,
                    });
                }
            }
        }

        if let Some(metadata) = &offer.metadata {
            let actual = serialized_json_len(metadata);
            if actual > limits.metadata_bytes {
                return Err(OfferLoadError::MetadataTooLarge {
                    domain_id: offer.domain_id.clone(),
                    offer_id: offer.offer_id.clone(),
                    actual,
                    max: limits.metadata_bytes,
                });
            }
        }
    }

    Ok(())
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

fn mark_offer_load_failed(
    relationship: &mut PeerRelationship,
    error: &OfferLoadError,
    context: OfferLoadContext<'_>,
) {
    relationship.offer_catalog_state = OfferCatalogLoadState::Failed;
    let mut failure = RelationshipFailureRecord::new(
        error.failure_code(),
        context.now,
        error.relationship_failure_scope(),
    );
    failure.peer_id = Some(relationship.peer_id);
    failure.retryable = error.retryable();
    failure.message = Some(error.to_string());
    relationship.degraded(failure, context.config.limits.retained_status_failures);
}

fn serialized_json_len(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .expect("serde_json::Value serialization cannot fail")
        .len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalPeerIdentity, PeerRelationshipState};
    use auki_identity::Wallet;
    use auki_protocol::v1::{frame::encode_json_frame, offer::OFFER_CATALOG_RESPONSE_TYPE};
    use serde_json::json;

    const DOMAIN_ID: &str = "noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs";
    const OTHER_DOMAIN_ID: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const VALID_HASH: &str = "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const INLINE_CANONICAL_JSON: &str = r#"{"id":"clock-main","rate":30}"#;
    const INLINE_CANONICAL_HASH: &str = "sha256:S8HmmZgtphz6U43WrRdH4Mckm2EZx5_5WCKqOOkbVSU";
    const NOW: &str = "2026-05-26T12:30:00Z";

    struct StaticOfferCatalogClient {
        frame: Result<Vec<u8>, OfferCatalogClientError>,
        requested_peer: Option<PeerId>,
    }

    impl OfferCatalogClient for StaticOfferCatalogClient {
        fn fetch_offer_catalog_frame(
            &mut self,
            peer_id: PeerId,
        ) -> Result<Vec<u8>, OfferCatalogClientError> {
            self.requested_peer = Some(peer_id);
            self.frame.clone()
        }
    }

    fn test_config() -> AukiP2pConfig {
        AukiP2pConfig::development()
    }

    fn authorized_relationship() -> PeerRelationship {
        let wallet = Wallet::from_seed(vec![91; 32]).expect("32-byte seed");
        let identity =
            LocalPeerIdentity::from_wallet(wallet, "2026-05-26T12:00:00Z", Some("offer-test"))
                .expect("identity");
        let mut relationship = PeerRelationship::new(identity.peer_id());
        relationship.state = PeerRelationshipState::Authorized;
        relationship.connected = true;
        relationship.authorized = true;
        relationship.accepted_served_domains = vec![DOMAIN_ID.to_owned()];
        relationship.offer_catalog_state = OfferCatalogLoadState::Available;
        relationship
    }

    fn context<'a>(config: &'a AukiP2pConfig) -> OfferLoadContext<'a> {
        OfferLoadContext::new(config, NOW)
    }

    fn registry_reference() -> Value {
        json!({
            "registry": "clock",
            "role": "clock",
            "id": "clock-main",
            "hash": VALID_HASH,
        })
    }

    fn inline_registry_reference() -> Value {
        json!({
            "registry": "clock",
            "role": "clock",
            "id": "clock-main",
            "hash": INLINE_CANONICAL_HASH,
            "canonical_json": INLINE_CANONICAL_JSON,
        })
    }

    fn offer_value(domain_id: &str, offer_id: &str, payload_type: &str) -> Value {
        json!({
            "offer_id": offer_id,
            "domain_id": domain_id,
            "kind": "sensor.frame",
            "status": "available",
            "access_modes": ["get", "subscribe"],
            "payload": {
                "type": payload_type,
                "encoding": "json",
            },
            "registry_refs": [registry_reference()],
            "updated_at": "2026-05-26T12:00:00Z",
            "expires_at": "2026-05-26T13:00:00Z",
        })
    }

    fn response_value(offers: Vec<Value>) -> Value {
        json!({
            "type": OFFER_CATALOG_RESPONSE_TYPE,
            "offers": offers,
            "generated_at": "2026-05-26T12:00:01Z",
        })
    }

    fn response_frame(config: &AukiP2pConfig, offers: Vec<Value>) -> Vec<u8> {
        encode_json_frame(
            &response_value(offers),
            config.limits.catalog_response_frame_body_bytes,
        )
        .expect("catalog frame")
    }

    #[test]
    fn loads_usable_offer_and_updates_relationship_status() {
        let config = test_config();
        let mut relationship = authorized_relationship();
        let supported_kinds = vec!["sensor.frame".to_owned()];
        let supported_payloads = vec!["auki.frame".to_owned()];
        let context = OfferLoadContext {
            requested_access_mode: Some(OfferAccessMode::Subscribe),
            supported_kinds: Some(&supported_kinds),
            supported_payload_types: Some(&supported_payloads),
            ..context(&config)
        };

        let report = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(
                &config,
                vec![offer_value(DOMAIN_ID, "camera-main", "auki.frame")],
            ),
            context,
        )
        .expect("offer load");

        assert_eq!(report.offers.len(), 1);
        assert!(report.offers[0].usable);
        assert_eq!(relationship.state, PeerRelationshipState::Ready);
        assert_eq!(
            relationship.offer_catalog_state,
            OfferCatalogLoadState::Loaded
        );
        assert_eq!(relationship.loaded_offers.len(), 1);
        assert_eq!(
            relationship.loaded_offers[0].payload_type.as_deref(),
            Some("auki.frame")
        );
        assert_eq!(relationship.loaded_offers[0].registry_refs.len(), 1);
    }

    #[test]
    fn client_boundary_fetches_frame_for_relationship_peer() {
        let config = test_config();
        let mut relationship = authorized_relationship();
        let expected_peer = relationship.peer_id;
        let mut client = StaticOfferCatalogClient {
            frame: Ok(response_frame(
                &config,
                vec![offer_value(DOMAIN_ID, "camera-main", "auki.frame")],
            )),
            requested_peer: None,
        };

        load_remote_offers_with_client(&mut relationship, &mut client, context(&config))
            .expect("offer load");

        assert_eq!(client.requested_peer, Some(expected_peer));
    }

    #[test]
    fn refuses_to_load_before_relationship_is_authorized() {
        let config = test_config();
        let mut relationship = PeerRelationship::new(authorized_relationship().peer_id);

        let error = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(
                &config,
                vec![offer_value(DOMAIN_ID, "camera-main", "auki.frame")],
            ),
            context(&config),
        )
        .expect_err("unauthorized relationship");

        assert!(matches!(
            error,
            OfferLoadError::RelationshipNotAuthorized { .. }
        ));
        assert_eq!(relationship.state, PeerRelationshipState::Degraded);
        assert_eq!(
            relationship.offer_catalog_state,
            OfferCatalogLoadState::Failed
        );
    }

    #[test]
    fn rejects_catalog_frame_over_body_limit_before_parsing() {
        let mut config = test_config();
        let mut relationship = authorized_relationship();
        config.limits.catalog_response_frame_body_bytes = 8;
        let frame = encode_json_frame(
            &response_value(vec![offer_value(DOMAIN_ID, "camera-main", "auki.frame")]),
            2048,
        )
        .expect("catalog frame");

        let error = load_remote_offers_from_frame(&mut relationship, &frame, context(&config))
            .expect_err("frame limit");

        assert!(matches!(
            error,
            OfferLoadError::CatalogFrame(FrameError::BodyTooLarge { .. })
        ));
        assert_eq!(
            relationship.last_failures[0].code,
            error::MESSAGE_PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn enforces_offer_count_registry_ref_and_metadata_limits() {
        let mut config = test_config();
        let mut relationship = authorized_relationship();
        config.limits.offers_per_catalog = 1;

        let error = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(
                &config,
                vec![
                    offer_value(DOMAIN_ID, "camera-main", "auki.frame"),
                    offer_value(DOMAIN_ID, "camera-secondary", "auki.frame"),
                ],
            ),
            context(&config),
        )
        .expect_err("offer count limit");

        assert!(matches!(
            error,
            OfferLoadError::TooManyOffers { actual: 2, max: 1 }
        ));

        let mut config = test_config();
        let mut relationship = authorized_relationship();
        config.limits.registry_refs_per_offer = 1;
        let mut offer = offer_value(DOMAIN_ID, "camera-main", "auki.frame");
        offer.as_object_mut().unwrap().insert(
            "registry_refs".to_owned(),
            json!([registry_reference(), registry_reference()]),
        );
        let error = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(&config, vec![offer]),
            context(&config),
        )
        .expect_err("registry refs limit");
        assert!(matches!(
            error,
            OfferLoadError::TooManyRegistryRefs {
                actual: 2,
                max: 1,
                ..
            }
        ));

        let mut config = test_config();
        let mut relationship = authorized_relationship();
        config.limits.inline_registry_json_bytes = 8;
        let mut offer = offer_value(DOMAIN_ID, "camera-main", "auki.frame");
        offer.as_object_mut().unwrap().insert(
            "registry_refs".to_owned(),
            json!([inline_registry_reference()]),
        );
        let error = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(&config, vec![offer]),
            context(&config),
        )
        .expect_err("inline registry json limit");
        assert!(matches!(
            error,
            OfferLoadError::InlineRegistryJsonTooLarge { .. }
        ));

        let mut config = test_config();
        let mut relationship = authorized_relationship();
        config.limits.metadata_bytes = 4;
        let mut offer = offer_value(DOMAIN_ID, "camera-main", "auki.frame");
        offer
            .as_object_mut()
            .unwrap()
            .insert("metadata".to_owned(), json!({"long": true}));
        let error = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(&config, vec![offer]),
            context(&config),
        )
        .expect_err("metadata limit");
        assert!(matches!(error, OfferLoadError::MetadataTooLarge { .. }));
    }

    #[test]
    fn unknown_offer_lookup_returns_stable_failure_code() {
        let config = test_config();
        let mut relationship = authorized_relationship();
        let report = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(
                &config,
                vec![offer_value(DOMAIN_ID, "camera-main", "auki.frame")],
            ),
            context(&config),
        )
        .expect("offer load");

        let error = report
            .find_offer(DOMAIN_ID, "missing")
            .expect_err("unknown offer");

        assert_eq!(error.failure_code(), error::OFFER_UNKNOWN_OFFER);
    }

    #[test]
    fn unserved_domain_offer_is_loaded_but_unusable() {
        let config = test_config();
        let mut relationship = authorized_relationship();

        let report = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(
                &config,
                vec![offer_value(OTHER_DOMAIN_ID, "foreign", "auki.frame")],
            ),
            context(&config),
        )
        .expect("offer load");

        assert!(!report.offers[0].usable);
        assert_eq!(
            report.offers[0].unusable_reason,
            Some(error::OFFER_DOMAIN_NOT_SERVED)
        );
        assert_eq!(
            relationship.loaded_offers[0].unusable_reason.as_deref(),
            Some(error::OFFER_DOMAIN_NOT_SERVED)
        );
    }

    #[test]
    fn unsupported_payload_offer_is_loaded_but_unusable() {
        let config = test_config();
        let mut relationship = authorized_relationship();
        let supported_payloads = vec!["auki.frame".to_owned()];
        let context = OfferLoadContext {
            supported_payload_types: Some(&supported_payloads),
            ..context(&config)
        };

        let report = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(
                &config,
                vec![offer_value(DOMAIN_ID, "camera-main", "auki.pointcloud")],
            ),
            context,
        )
        .expect("offer load");

        assert!(!report.offers[0].usable);
        assert_eq!(
            report.offers[0].unusable_reason,
            Some(error::OFFER_UNSUPPORTED_PAYLOAD_TYPE)
        );
    }

    #[test]
    fn stale_offer_is_loaded_but_unusable() {
        let config = test_config();
        let mut relationship = authorized_relationship();
        let mut offer = offer_value(DOMAIN_ID, "camera-main", "auki.frame");
        offer
            .as_object_mut()
            .unwrap()
            .insert("expires_at".to_owned(), json!("2026-05-26T12:00:00Z"));

        let report = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(&config, vec![offer]),
            context(&config),
        )
        .expect("offer load");

        assert!(!report.offers[0].usable);
        assert_eq!(report.offers[0].unusable_reason, Some(error::OFFER_STALE));
    }

    #[test]
    fn registry_hash_mismatch_fails_catalog_validation() {
        let config = test_config();
        let mut relationship = authorized_relationship();
        let mut bad_reference = registry_reference();
        bad_reference
            .as_object_mut()
            .unwrap()
            .insert("canonical_json".to_owned(), json!(r#"{"id":"clock-main"}"#));
        let mut offer = offer_value(DOMAIN_ID, "camera-main", "auki.frame");
        offer
            .as_object_mut()
            .unwrap()
            .insert("registry_refs".to_owned(), json!([bad_reference]));

        let error = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(&config, vec![offer]),
            context(&config),
        )
        .expect_err("registry hash mismatch");

        assert!(matches!(error, OfferLoadError::CatalogResponse(_)));
        assert_eq!(error.failure_code(), error::OFFER_INVALID_CATALOG_RESPONSE);
    }

    #[test]
    fn app_offer_policy_can_make_loaded_offer_unusable() {
        let mut config = test_config();
        config.offer_policy = OfferPolicy::AppPolicy;
        let mut relationship = authorized_relationship();
        let allowed = [AppAllowedOffer {
            domain_id: DOMAIN_ID,
            offer_id: "other",
        }];
        let context = OfferLoadContext {
            app_offer_policy: AppOfferPolicy::AllowOnly {
                offers: &allowed,
                failure_code: error::OFFER_LOAD_FAILED,
            },
            ..context(&config)
        };

        let report = load_remote_offers_from_frame(
            &mut relationship,
            &response_frame(
                &config,
                vec![offer_value(DOMAIN_ID, "camera-main", "auki.frame")],
            ),
            context,
        )
        .expect("offer load");

        assert!(!report.offers[0].usable);
        assert_eq!(
            report.offers[0].unusable_reason,
            Some(error::OFFER_LOAD_FAILED)
        );
    }
}
