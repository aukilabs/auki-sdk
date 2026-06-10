//! Pure peer-handshake policy validation.

use crate::{AukiP2pConfig, ConfigError, DomainAccessPolicy, LocalPeerIdentity};
use auki_protocol::v1::{
    authority::{
        AcceptedServedDomain, AuthorityChainError, PeerAuthorization, RejectedDeclaredDomain,
    },
    error,
    handshake::{CLUSTER_LIFECYCLE_V1, PeerHandshake},
    identity::{PeerBindingFreshnessError, VerifiedPeerBinding},
    offer::OfferCatalogPath,
};
use libp2p_identity::PeerId;
use serde_json::Value;
use std::fmt;

/// Application domain-access decision supplied to the pure policy kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppDomainAccess<'a> {
    /// No application domain-access decision was supplied.
    NotProvided,
    /// Application policy allows every authority-valid served domain.
    AllowAll,
    /// Application policy allows only these domain ids.
    AllowOnly(&'a [&'a str]),
}

/// Input for validating one decoded remote peer handshake.
#[derive(Debug, Clone, Copy)]
pub struct HandshakeValidationInput<'a> {
    /// Transport-authenticated libp2p peer id for this relationship.
    pub authenticated_peer_id: &'a PeerId,
    /// Decoded remote peer handshake.
    pub handshake: &'a PeerHandshake,
    /// Local runtime config and policy.
    pub config: &'a AukiP2pConfig,
    /// Local verification time as an RFC3339 UTC `Z` string.
    pub now: &'a str,
    /// Application peer-admission decision when config uses app-policy.
    pub app_peer_authorization: Option<PeerAuthorization>,
    /// Application domain-access decision when config uses app-policy.
    pub app_domain_access: AppDomainAccess<'a>,
    /// Authorization-material `type` values required by local policy.
    pub required_authorization_material_types: &'a [&'a str],
}

/// Result of accepting a remote peer handshake at the policy layer.
#[derive(Debug, Clone, PartialEq)]
pub struct HandshakeValidationResult {
    /// Transport-authenticated remote peer id.
    pub authenticated_peer_id: PeerId,
    /// Verified remote peer binding.
    pub verified_peer: VerifiedPeerBinding,
    /// Selected lifecycle version.
    pub selected_lifecycle_version: String,
    /// Initial lifecycle state after handshake policy succeeds.
    pub lifecycle_state: HandshakeLifecycleState,
    /// Authority-valid and locally accepted served domains.
    pub accepted_served_domains: Vec<AcceptedServedDomain>,
    /// Authority-invalid declared domains.
    pub rejected_declared_domains: Vec<RejectedDeclaredDomain>,
    /// Authority-valid domains rejected by local domain policy.
    pub policy_rejected_domains: Vec<PolicyRejectedDomain>,
    /// Remote offer-catalog path, if advertised.
    pub offer_catalog: Option<OfferCatalogPath>,
    /// Earliest relationship authority deadline from the handshake-time inputs.
    pub authority_deadline: Option<String>,
    /// Non-fatal diagnostics for rejected domains.
    pub failures: Vec<HandshakeFailureDiagnostic>,
}

/// Initial lifecycle state after the policy kernel accepts a handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeLifecycleState {
    /// Peer is connected and authorized; offers have not been loaded yet.
    Authorized,
}

/// Authority-valid domain rejected by local domain policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRejectedDomain {
    /// Rejected domain id.
    pub domain_id: String,
    /// Stable RFC failure code.
    pub failure_code: &'static str,
}

/// Non-fatal handshake diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeFailureDiagnostic {
    /// Stable RFC failure code.
    pub code: &'static str,
    /// Diagnostic scope.
    pub scope: HandshakeFailureScope,
    /// Peer id string, when relevant.
    pub peer_id: Option<String>,
    /// Domain id string, when relevant.
    pub domain_id: Option<String>,
    /// Human-readable diagnostic message.
    pub message: String,
}

/// Diagnostic scope for handshake policy failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeFailureScope {
    /// Peer-level failure.
    Peer,
    /// Domain authority failure.
    Domain,
    /// Local policy failure.
    Policy,
    /// Handshake shape or material failure.
    Handshake,
}

/// Metadata object being checked against local size limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeMetadataField {
    /// Top-level handshake `metadata`.
    Handshake,
    /// One declared-domain `metadata`.
    DeclaredDomain {
        /// Declared-domain index.
        index: usize,
    },
    /// One authorization-material `metadata`.
    AuthorizationMaterial {
        /// Authorization-material index.
        index: usize,
    },
    /// Offer-catalog path `metadata`.
    OfferCatalog,
}

/// Fatal policy-kernel errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakePolicyError {
    /// Local config is invalid.
    Config(ConfigError),
    /// Serialized handshake body exceeds the configured limit.
    HandshakeBodyTooLarge {
        /// Serialized body length.
        actual: u64,
        /// Local maximum body length.
        max: u64,
    },
    /// Too many declared domains were present.
    TooManyDeclaredDomains {
        /// Actual count.
        actual: usize,
        /// Local maximum.
        max: usize,
    },
    /// Too many authorization-material entries were present.
    TooManyAuthorizationMaterialEntries {
        /// Actual count.
        actual: usize,
        /// Local maximum.
        max: usize,
    },
    /// A metadata object exceeded the local size limit.
    MetadataTooLarge {
        /// Metadata object being checked.
        field: HandshakeMetadataField,
        /// Serialized metadata length.
        actual: u64,
        /// Local maximum.
        max: u64,
    },
    /// Authorization material was malformed.
    InvalidAuthorizationMaterial {
        /// Authorization-material index.
        index: usize,
        /// Error detail.
        reason: &'static str,
    },
    /// Local policy requires authorization material that was not present.
    MissingRequiredAuthorizationMaterial {
        /// Required authorization-material type.
        material_type: String,
    },
    /// Config uses app-policy peer admission but no app decision was supplied.
    MissingAppPeerAuthorizationDecision,
    /// Config uses app-policy domain access but no app decision was supplied.
    MissingAppDomainAccessDecision,
    /// Timestamp syntax was invalid in local or remote handshake inputs.
    InvalidTimestamp {
        /// Field that carried the invalid timestamp.
        field: &'static str,
        /// Invalid timestamp value.
        value: String,
    },
    /// Peer binding failed freshness policy.
    PeerBindingFreshness(PeerBindingFreshnessError),
    /// Authority-chain validation failed at peer level.
    Authority(AuthorityChainError),
}

impl Default for AppDomainAccess<'_> {
    fn default() -> Self {
        Self::NotProvided
    }
}

impl<'a> HandshakeValidationInput<'a> {
    /// Create validation input with only the mandatory fields.
    pub fn new(
        authenticated_peer_id: &'a PeerId,
        handshake: &'a PeerHandshake,
        config: &'a AukiP2pConfig,
        now: &'a str,
    ) -> Self {
        Self {
            authenticated_peer_id,
            handshake,
            config,
            now,
            app_peer_authorization: None,
            app_domain_access: AppDomainAccess::NotProvided,
            required_authorization_material_types: &[],
        }
    }
}

/// Build a local peer handshake from local identity and served-domain claims.
pub fn build_local_handshake(
    identity: &LocalPeerIdentity,
    declared_domains: Vec<auki_protocol::v1::authority::DeclaredDomain>,
) -> PeerHandshake {
    PeerHandshake::create(identity.peer_binding().clone(), declared_domains)
}

/// Validate one decoded remote handshake against local policy.
pub fn validate_remote_handshake(
    input: HandshakeValidationInput<'_>,
) -> Result<HandshakeValidationResult, HandshakePolicyError> {
    input
        .config
        .validate()
        .map_err(HandshakePolicyError::Config)?;
    enforce_handshake_body_limit(input.handshake, input.config)?;
    enforce_count_limits(input.handshake, input.config)?;
    enforce_metadata_limits(input.handshake, input.config)?;
    enforce_authorization_material_shape(input.handshake)?;
    enforce_required_authorization_material(
        input.handshake,
        input.required_authorization_material_types,
    )?;

    let verified_for_freshness = input
        .handshake
        .peer_binding
        .verify_for_peer_id(input.authenticated_peer_id)
        .map_err(|error| {
            HandshakePolicyError::Authority(AuthorityChainError::InvalidPeerBinding(Box::new(
                error,
            )))
        })?;
    verified_for_freshness
        .validate_freshness(
            input.now,
            input.config.peer_binding_freshness.as_protocol_policy(),
        )
        .map_err(HandshakePolicyError::PeerBindingFreshness)?;

    let peer_authorization_policy = input
        .config
        .peer_admission
        .as_protocol_policy(input.app_peer_authorization)
        .map_err(|error| match error {
            ConfigError::MissingAppPolicyDecision => {
                HandshakePolicyError::MissingAppPeerAuthorizationDecision
            }
            other => HandshakePolicyError::Config(other),
        })?;

    let authority = input
        .handshake
        .validate_authority_with_authorization_policy(
            input.authenticated_peer_id,
            peer_authorization_policy,
            input.now,
        )
        .map_err(HandshakePolicyError::Authority)?;

    let mut accepted_served_domains = Vec::new();
    let mut policy_rejected_domains = Vec::new();
    for accepted in authority.accepted_served_domains {
        if domain_policy_allows(input.config, input.app_domain_access, &accepted.domain_id)? {
            accepted_served_domains.push(accepted);
        } else {
            policy_rejected_domains.push(PolicyRejectedDomain {
                domain_id: accepted.domain_id,
                failure_code: error::POLICY_DOMAIN_REJECTED,
            });
        }
    }

    let mut failures = Vec::new();
    failures.extend(
        authority
            .rejected_declared_domains
            .iter()
            .map(rejected_domain_diagnostic),
    );
    failures.extend(
        policy_rejected_domains
            .iter()
            .map(policy_rejected_domain_diagnostic),
    );
    let authority_deadline =
        select_authority_deadline(input, &authority.peer, &accepted_served_domains)?;

    Ok(HandshakeValidationResult {
        authenticated_peer_id: *input.authenticated_peer_id,
        verified_peer: authority.peer,
        selected_lifecycle_version: CLUSTER_LIFECYCLE_V1.to_owned(),
        lifecycle_state: HandshakeLifecycleState::Authorized,
        accepted_served_domains,
        rejected_declared_domains: authority.rejected_declared_domains,
        policy_rejected_domains,
        offer_catalog: input.handshake.offer_catalog.clone(),
        authority_deadline,
        failures,
    })
}

impl HandshakePolicyError {
    /// Stable RFC failure code for this fatal error.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::Config(_) => error::HANDSHAKE_INVALID_MESSAGE,
            Self::HandshakeBodyTooLarge { .. } => error::MESSAGE_PAYLOAD_TOO_LARGE,
            Self::TooManyDeclaredDomains { .. }
            | Self::TooManyAuthorizationMaterialEntries { .. }
            | Self::MetadataTooLarge { .. }
            | Self::InvalidAuthorizationMaterial { .. } => error::HANDSHAKE_INVALID_MESSAGE,
            Self::MissingRequiredAuthorizationMaterial { .. } => {
                error::HANDSHAKE_MISSING_REQUIRED_MATERIAL
            }
            Self::MissingAppPeerAuthorizationDecision => error::AUTHORIZATION_PEER_REJECTED,
            Self::MissingAppDomainAccessDecision => error::POLICY_DOMAIN_REJECTED,
            Self::InvalidTimestamp { .. } => error::HANDSHAKE_INVALID_MESSAGE,
            Self::PeerBindingFreshness(error) => error.failure_code(),
            Self::Authority(error) => error.failure_code(),
        }
    }
}

impl fmt::Display for HandshakePolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(f, "invalid handshake policy config: {error}"),
            Self::HandshakeBodyTooLarge { actual, max } => {
                write!(f, "handshake body too large: {actual} bytes (max {max})")
            }
            Self::TooManyDeclaredDomains { actual, max } => {
                write!(f, "too many declared domains: {actual} (max {max})")
            }
            Self::TooManyAuthorizationMaterialEntries { actual, max } => write!(
                f,
                "too many authorization-material entries: {actual} (max {max})"
            ),
            Self::MetadataTooLarge { field, actual, max } => {
                write!(
                    f,
                    "{field:?} metadata too large: {actual} bytes (max {max})"
                )
            }
            Self::InvalidAuthorizationMaterial { index, reason } => {
                write!(
                    f,
                    "invalid authorization material at index {index}: {reason}"
                )
            }
            Self::MissingRequiredAuthorizationMaterial { material_type } => {
                write!(f, "missing required authorization material {material_type}")
            }
            Self::MissingAppPeerAuthorizationDecision => {
                write!(f, "missing app peer-authorization decision")
            }
            Self::MissingAppDomainAccessDecision => {
                write!(f, "missing app domain-access decision")
            }
            Self::InvalidTimestamp { field, value } => {
                write!(f, "invalid timestamp in {field}: {value}")
            }
            Self::PeerBindingFreshness(error) => write!(f, "peer binding freshness: {error}"),
            Self::Authority(error) => write!(f, "authority validation: {error}"),
        }
    }
}

impl std::error::Error for HandshakePolicyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::PeerBindingFreshness(error) => Some(error),
            Self::Authority(error) => Some(error),
            _ => None,
        }
    }
}

fn enforce_handshake_body_limit(
    handshake: &PeerHandshake,
    config: &AukiP2pConfig,
) -> Result<(), HandshakePolicyError> {
    let actual = serialized_len(handshake.value());
    let max = config.limits.handshake_frame_body_bytes;
    if actual > max {
        return Err(HandshakePolicyError::HandshakeBodyTooLarge { actual, max });
    }
    Ok(())
}

fn enforce_count_limits(
    handshake: &PeerHandshake,
    config: &AukiP2pConfig,
) -> Result<(), HandshakePolicyError> {
    let declared_domains = handshake.declared_domains.len();
    if declared_domains > config.limits.declared_domains_per_handshake {
        return Err(HandshakePolicyError::TooManyDeclaredDomains {
            actual: declared_domains,
            max: config.limits.declared_domains_per_handshake,
        });
    }

    let authorization_material = handshake
        .authorization_material
        .as_ref()
        .map_or(0, Vec::len);
    if authorization_material > config.limits.authorization_material_entries {
        return Err(HandshakePolicyError::TooManyAuthorizationMaterialEntries {
            actual: authorization_material,
            max: config.limits.authorization_material_entries,
        });
    }

    Ok(())
}

fn enforce_metadata_limits(
    handshake: &PeerHandshake,
    config: &AukiP2pConfig,
) -> Result<(), HandshakePolicyError> {
    let max = config.limits.metadata_bytes;

    if let Some(metadata) = &handshake.metadata {
        enforce_one_metadata_limit(HandshakeMetadataField::Handshake, metadata, max)?;
    }

    for (index, declared_domain) in handshake.declared_domains.iter().enumerate() {
        if let Some(metadata) = &declared_domain.metadata {
            enforce_one_metadata_limit(
                HandshakeMetadataField::DeclaredDomain { index },
                metadata,
                max,
            )?;
        }
    }

    if let Some(authorization_material) = &handshake.authorization_material {
        for (index, material) in authorization_material.iter().enumerate() {
            if let Some(metadata) = material.get("metadata") {
                enforce_one_metadata_limit(
                    HandshakeMetadataField::AuthorizationMaterial { index },
                    metadata,
                    max,
                )?;
            }
        }
    }

    if let Some(offer_catalog) = &handshake.offer_catalog
        && let Some(metadata) = &offer_catalog.metadata
    {
        enforce_one_metadata_limit(HandshakeMetadataField::OfferCatalog, metadata, max)?;
    }

    Ok(())
}

fn enforce_one_metadata_limit(
    field: HandshakeMetadataField,
    value: &Value,
    max: u64,
) -> Result<(), HandshakePolicyError> {
    let actual = serialized_len(value);
    if actual > max {
        Err(HandshakePolicyError::MetadataTooLarge { field, actual, max })
    } else {
        Ok(())
    }
}

fn enforce_authorization_material_shape(
    handshake: &PeerHandshake,
) -> Result<(), HandshakePolicyError> {
    let Some(materials) = &handshake.authorization_material else {
        return Ok(());
    };

    for (index, material) in materials.iter().enumerate() {
        let Some(object) = material.as_object() else {
            return Err(HandshakePolicyError::InvalidAuthorizationMaterial {
                index,
                reason: "material is not an object",
            });
        };
        match object.get("type") {
            Some(Value::String(_)) => {}
            Some(_) => {
                return Err(HandshakePolicyError::InvalidAuthorizationMaterial {
                    index,
                    reason: "type is not a string",
                });
            }
            None => {
                return Err(HandshakePolicyError::InvalidAuthorizationMaterial {
                    index,
                    reason: "missing type",
                });
            }
        }
        if let Some(metadata) = object.get("metadata")
            && !metadata.is_object()
        {
            return Err(HandshakePolicyError::InvalidAuthorizationMaterial {
                index,
                reason: "metadata is not an object",
            });
        }
        if let Some(expires_at) = object.get("expires_at") {
            let Some(expires_at) = expires_at.as_str() else {
                return Err(HandshakePolicyError::InvalidAuthorizationMaterial {
                    index,
                    reason: "expires_at is not a string",
                });
            };
            if parse_rfc3339_z_timestamp_millis(expires_at).is_none() {
                return Err(HandshakePolicyError::InvalidAuthorizationMaterial {
                    index,
                    reason: "expires_at is not an RFC3339 Z timestamp",
                });
            }
        }
    }

    Ok(())
}

fn enforce_required_authorization_material(
    handshake: &PeerHandshake,
    required_types: &[&str],
) -> Result<(), HandshakePolicyError> {
    for required_type in required_types {
        if !has_authorization_material_type(handshake, required_type) {
            return Err(HandshakePolicyError::MissingRequiredAuthorizationMaterial {
                material_type: (*required_type).to_owned(),
            });
        }
    }
    Ok(())
}

fn has_authorization_material_type(handshake: &PeerHandshake, required_type: &str) -> bool {
    handshake
        .authorization_material
        .as_ref()
        .into_iter()
        .flatten()
        .any(|material| material.get("type").and_then(Value::as_str) == Some(required_type))
}

fn domain_policy_allows(
    config: &AukiP2pConfig,
    app_domain_access: AppDomainAccess<'_>,
    domain_id: &str,
) -> Result<bool, HandshakePolicyError> {
    match config.domain_access_policy {
        DomainAccessPolicy::AllowAll => Ok(true),
        DomainAccessPolicy::AppPolicy => match app_domain_access {
            AppDomainAccess::NotProvided => {
                Err(HandshakePolicyError::MissingAppDomainAccessDecision)
            }
            AppDomainAccess::AllowAll => Ok(true),
            AppDomainAccess::AllowOnly(allowed_domain_ids) => {
                Ok(allowed_domain_ids.contains(&domain_id))
            }
        },
    }
}

fn rejected_domain_diagnostic(domain: &RejectedDeclaredDomain) -> HandshakeFailureDiagnostic {
    HandshakeFailureDiagnostic {
        code: domain.failure_code,
        scope: HandshakeFailureScope::Domain,
        peer_id: None,
        domain_id: domain.domain_id.clone(),
        message: format!("{:?}", domain.reason),
    }
}

fn policy_rejected_domain_diagnostic(domain: &PolicyRejectedDomain) -> HandshakeFailureDiagnostic {
    HandshakeFailureDiagnostic {
        code: domain.failure_code,
        scope: HandshakeFailureScope::Policy,
        peer_id: None,
        domain_id: Some(domain.domain_id.clone()),
        message: "domain rejected by local policy".to_owned(),
    }
}

fn select_authority_deadline(
    input: HandshakeValidationInput<'_>,
    verified_peer: &VerifiedPeerBinding,
    accepted_served_domains: &[AcceptedServedDomain],
) -> Result<Option<String>, HandshakePolicyError> {
    let mut selector = DeadlineSelector::default();

    if input
        .config
        .authority_deadline
        .include_peer_binding_freshness
        && input.config.peer_binding_freshness.enforce
    {
        let issued_at_ms =
            required_timestamp_millis("peer_binding.issued_at", &verified_peer.issued_at)?;
        let deadline_ms =
            issued_at_ms.saturating_add(i128::from(input.config.peer_binding_freshness.max_age_ms));
        selector.consider(deadline_ms, format_rfc3339_z_timestamp_millis(deadline_ms));
    }

    if input.config.authority_deadline.include_delegation_expiry {
        for accepted in accepted_served_domains {
            if let Some(delegation) = &accepted.delegation {
                selector.consider_timestamp("delegation.expires_at", &delegation.expires_at)?;
            }
        }
    }

    if input
        .config
        .authority_deadline
        .include_authorization_material_expiry
        && let Some(materials) = &input.handshake.authorization_material
    {
        for material in materials {
            if let Some(expires_at) = material.get("expires_at").and_then(Value::as_str) {
                selector.consider_timestamp("authorization_material.expires_at", expires_at)?;
            }
        }
    }

    if let Some(local_session_lifetime_ms) =
        input.config.authority_deadline.local_session_lifetime_ms
    {
        let now_ms = required_timestamp_millis("now", input.now)?;
        let deadline_ms = now_ms.saturating_add(i128::from(local_session_lifetime_ms));
        selector.consider(deadline_ms, format_rfc3339_z_timestamp_millis(deadline_ms));
    }

    Ok(selector.finish())
}

#[derive(Default)]
struct DeadlineSelector {
    earliest: Option<(i128, String)>,
}

impl DeadlineSelector {
    fn consider(&mut self, timestamp_ms: i128, timestamp: String) {
        if self
            .earliest
            .as_ref()
            .is_none_or(|(current_ms, _)| timestamp_ms < *current_ms)
        {
            self.earliest = Some((timestamp_ms, timestamp));
        }
    }

    fn consider_timestamp(
        &mut self,
        field: &'static str,
        timestamp: &str,
    ) -> Result<(), HandshakePolicyError> {
        let timestamp_ms = required_timestamp_millis(field, timestamp)?;
        self.consider(timestamp_ms, timestamp.to_owned());
        Ok(())
    }

    fn finish(self) -> Option<String> {
        self.earliest.map(|(_, timestamp)| timestamp)
    }
}

fn required_timestamp_millis(
    field: &'static str,
    value: &str,
) -> Result<i128, HandshakePolicyError> {
    parse_rfc3339_z_timestamp_millis(value).ok_or_else(|| HandshakePolicyError::InvalidTimestamp {
        field,
        value: value.to_owned(),
    })
}

fn parse_rfc3339_z_timestamp_millis(value: &str) -> Option<i128> {
    let value = value.strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;
    let (year, month, day) = parse_date(date)?;
    if year > 9999 || month == 0 || month > 12 {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }

    let (hour, minute, second, fraction_ms) = parse_time_millis(time)?;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds =
        days * 86_400 + i128::from(hour) * 3_600 + i128::from(minute) * 60 + i128::from(second);
    Some(seconds * 1000 + i128::from(fraction_ms))
}

fn format_rfc3339_z_timestamp_millis(timestamp_ms: i128) -> String {
    let total_seconds = timestamp_ms.div_euclid(1000);
    let millis = timestamp_ms.rem_euclid(1000);
    let days = total_seconds.div_euclid(86_400);
    let second_of_day = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;

    if millis == 0 {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    } else {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
    }
}

fn parse_time_millis(value: &str) -> Option<(u32, u32, u32, u32)> {
    let (base, fraction) = match value.split_once('.') {
        Some((base, fraction)) => {
            if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            (base, Some(fraction))
        }
        None => (value, None),
    };

    if base.len() != 8 {
        return None;
    }
    if base.as_bytes().get(2) != Some(&b':') || base.as_bytes().get(5) != Some(&b':') {
        return None;
    }
    let hour = parse_fixed_digits(&base[0..2])?;
    let minute = parse_fixed_digits(&base[3..5])?;
    let second = parse_fixed_digits(&base[6..8])?;
    Some((
        hour,
        minute,
        second,
        fraction_millis(fraction.unwrap_or("")),
    ))
}

fn fraction_millis(fraction: &str) -> u32 {
    let mut millis = 0;
    for index in 0..3 {
        millis *= 10;
        if let Some(byte) = fraction.as_bytes().get(index) {
            millis += u32::from(byte - b'0');
        }
    }
    millis
}

fn parse_date(value: &str) -> Option<(u32, u32, u32)> {
    if value.len() != 10 {
        return None;
    }
    if value.as_bytes().get(4) != Some(&b'-') || value.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let year = parse_fixed_digits(&value[0..4])?;
    let month = parse_fixed_digits(&value[5..7])?;
    let day = parse_fixed_digits(&value[8..10])?;
    Some((year, month, day))
}

fn parse_fixed_digits(value: &str) -> Option<u32> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

fn days_from_civil(year: u32, month: u32, day: u32) -> i128 {
    let year = i128::from(year) - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i128::from(month);
    let day = i128::from(day);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i128) -> (i128, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month as u32, day as u32)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn serialized_len(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .expect("serde_json::Value serialization is infallible")
        .len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalPeerIdentity, PeerAdmissionConfig};
    use auki_identity::Wallet;
    use auki_protocol::v1::{
        authority::{PeerAuthorizationMode, ServedDomainAuthority},
        domain::{
            DOMAIN_NONCE_LEN, DelegationScope, DomainDeclaration, DomainDelegation,
            DomainDelegationParams,
        },
    };
    use serde_json::{Value, json};

    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";
    const OLD_ISSUED_AT: &str = "2026-05-26T10:00:00Z";
    const FUTURE_ISSUED_AT: &str = "2026-05-26T12:10:01Z";
    const VALID_FROM: &str = "2026-05-26T11:00:00Z";
    const EXPIRES_AT: &str = "2026-05-26T13:00:00Z";
    const NOW: &str = "2026-05-26T12:00:00Z";
    const NONCE: [u8; DOMAIN_NONCE_LEN] = [9u8; DOMAIN_NONCE_LEN];

    fn wallet(seed: u8) -> std::sync::Arc<Wallet> {
        Wallet::from_seed(vec![seed; 32]).expect("32-byte seed")
    }

    fn identity(seed: u8, issued_at: &str) -> LocalPeerIdentity {
        LocalPeerIdentity::from_wallet(wallet(seed), issued_at, Some("policy-test"))
            .expect("local peer identity")
    }

    fn declaration(owner: &Wallet) -> DomainDeclaration {
        DomainDeclaration::create(owner, &NONCE, Some("warehouse")).unwrap()
    }

    fn direct_owner_handshake(identity: &LocalPeerIdentity) -> PeerHandshake {
        let owner = identity.wallet();
        let declaration = declaration(owner);
        let domain_id = declaration.domain_id().unwrap().to_owned();
        let declared =
            auki_protocol::v1::authority::DeclaredDomain::new(domain_id, declaration, None);
        build_local_handshake(identity, vec![declared])
    }

    fn delegated_handshake(
        owner: &Wallet,
        delegate: &LocalPeerIdentity,
        expires_at: &str,
    ) -> PeerHandshake {
        let declaration = declaration(owner);
        let domain_id = declaration.domain_id().unwrap().to_owned();
        let delegation = DomainDelegation::create(
            owner,
            DomainDelegationParams {
                domain_id: &domain_id,
                delegate_wallet_public_key: &delegate.wallet_public_key(),
                delegate_peer_id: &delegate.peer_id(),
                scopes: &[DelegationScope::Serve],
                valid_from: VALID_FROM,
                expires_at,
                label: Some("delegate"),
            },
        )
        .unwrap();
        let declared = auki_protocol::v1::authority::DeclaredDomain::new(
            domain_id,
            declaration,
            Some(delegation),
        );
        build_local_handshake(delegate, vec![declared])
    }

    fn with_handshake_field(handshake: PeerHandshake, field: &str, value: Value) -> PeerHandshake {
        let mut value_object = handshake.into_value();
        value_object
            .as_object_mut()
            .expect("handshake object")
            .insert(field.to_owned(), value);
        PeerHandshake::from_value(value_object).expect("mutated handshake remains valid")
    }

    fn validate<'a>(
        peer_id: &'a PeerId,
        handshake: &'a PeerHandshake,
        config: &'a AukiP2pConfig,
    ) -> Result<HandshakeValidationResult, HandshakePolicyError> {
        validate_remote_handshake(HandshakeValidationInput::new(
            peer_id, handshake, config, NOW,
        ))
    }

    #[test]
    fn validates_direct_owner_handshake() {
        let identity = identity(41, ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);

        let result = validate(
            &identity.peer_id(),
            &handshake,
            &AukiP2pConfig::development(),
        )
        .expect("valid direct owner");

        assert_eq!(result.verified_peer.peer_id, identity.peer_id());
        assert_eq!(result.selected_lifecycle_version, CLUSTER_LIFECYCLE_V1);
        assert_eq!(result.lifecycle_state, HandshakeLifecycleState::Authorized);
        assert_eq!(result.accepted_served_domains.len(), 1);
        assert_eq!(
            result.accepted_served_domains[0].authority,
            ServedDomainAuthority::DirectOwner
        );
        assert_eq!(result.authority_deadline, None);
        assert!(result.rejected_declared_domains.is_empty());
        assert!(result.failures.is_empty());
    }

    #[test]
    fn validates_delegated_peer_handshake() {
        let owner = wallet(42);
        let delegate = identity(43, ISSUED_AT);
        let handshake = delegated_handshake(&owner, &delegate, EXPIRES_AT);

        let result = validate(
            &delegate.peer_id(),
            &handshake,
            &AukiP2pConfig::development(),
        )
        .expect("valid delegated peer");

        assert_eq!(result.accepted_served_domains.len(), 1);
        assert_eq!(
            result.accepted_served_domains[0].authority,
            ServedDomainAuthority::Delegated
        );
        assert!(result.accepted_served_domains[0].delegation.is_some());
        assert_eq!(result.authority_deadline.as_deref(), Some(EXPIRES_AT));
    }

    #[test]
    fn selects_peer_binding_freshness_deadline_when_enforced() {
        let identity = identity(55, ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);
        let config = AukiP2pConfig {
            peer_binding_freshness: crate::PeerBindingFreshnessConfig::production_recommended(),
            ..AukiP2pConfig::development()
        };

        let result = validate(&identity.peer_id(), &handshake, &config).expect("valid peer");

        assert_eq!(
            result.authority_deadline.as_deref(),
            Some("2026-05-26T13:00:00Z")
        );
    }

    #[test]
    fn selects_authorization_material_expiry_deadline() {
        let identity = identity(56, ISSUED_AT);
        let handshake = with_handshake_field(
            direct_owner_handshake(&identity),
            "authorization_material",
            json!([{ "type": "auki.local.capability", "expires_at": "2026-05-26T12:30:00Z" }]),
        );

        let result = validate(
            &identity.peer_id(),
            &handshake,
            &AukiP2pConfig::development(),
        )
        .expect("valid peer");

        assert_eq!(
            result.authority_deadline.as_deref(),
            Some("2026-05-26T12:30:00Z")
        );
    }

    #[test]
    fn selects_local_session_lifetime_deadline() {
        let identity = identity(57, ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);
        let mut config = AukiP2pConfig::development();
        config.authority_deadline.local_session_lifetime_ms = Some(30 * 60 * 1000);

        let result = validate(&identity.peer_id(), &handshake, &config).expect("valid peer");

        assert_eq!(
            result.authority_deadline.as_deref(),
            Some("2026-05-26T12:30:00Z")
        );
    }

    #[test]
    fn authority_deadline_selects_earliest_relevant_source() {
        let owner = wallet(58);
        let delegate = identity(59, ISSUED_AT);
        let handshake = with_handshake_field(
            delegated_handshake(&owner, &delegate, EXPIRES_AT),
            "authorization_material",
            json!([{ "type": "auki.local.capability", "expires_at": "2026-05-26T12:45:00Z" }]),
        );
        let mut config = AukiP2pConfig {
            peer_binding_freshness: crate::PeerBindingFreshnessConfig {
                enforce: true,
                max_age_ms: 20 * 60 * 1000,
                future_tolerance_ms: 5 * 60 * 1000,
            },
            ..AukiP2pConfig::development()
        };
        config.authority_deadline.local_session_lifetime_ms = Some(30 * 60 * 1000);

        let result = validate(&delegate.peer_id(), &handshake, &config).expect("valid peer");

        assert_eq!(
            result.authority_deadline.as_deref(),
            Some("2026-05-26T12:20:00Z")
        );
    }

    #[test]
    fn rejects_invalid_authorization_material_expiry() {
        let identity = identity(60, ISSUED_AT);
        let handshake = with_handshake_field(
            direct_owner_handshake(&identity),
            "authorization_material",
            json!([{ "type": "auki.local.capability", "expires_at": "not-a-timestamp" }]),
        );

        let error = validate(
            &identity.peer_id(),
            &handshake,
            &AukiP2pConfig::development(),
        )
        .unwrap_err();

        assert_eq!(
            error,
            HandshakePolicyError::InvalidAuthorizationMaterial {
                index: 0,
                reason: "expires_at is not an RFC3339 Z timestamp",
            }
        );
        assert_eq!(error.failure_code(), error::HANDSHAKE_INVALID_MESSAGE);
    }

    #[test]
    fn rejects_peer_not_allowed_by_whitelist() {
        let identity = identity(44, ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);
        let config = AukiP2pConfig {
            peer_admission: PeerAdmissionConfig::WhitelistedOnly {
                peer_ids: vec![],
                wallet_public_keys: vec![],
            },
            ..AukiP2pConfig::development()
        };

        let error = validate(&identity.peer_id(), &handshake, &config).unwrap_err();

        assert_eq!(
            error,
            HandshakePolicyError::Authority(AuthorityChainError::PeerRejected)
        );
        assert_eq!(error.failure_code(), error::AUTHORIZATION_PEER_REJECTED);
    }

    #[test]
    fn rejects_old_peer_binding_when_freshness_is_enforced() {
        let identity = identity(45, OLD_ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);
        let config = AukiP2pConfig {
            peer_binding_freshness: crate::PeerBindingFreshnessConfig::production_recommended(),
            ..AukiP2pConfig::development()
        };

        let error = validate(&identity.peer_id(), &handshake, &config).unwrap_err();

        assert!(matches!(
            error,
            HandshakePolicyError::PeerBindingFreshness(
                PeerBindingFreshnessError::BindingTooOld { .. }
            )
        ));
        assert_eq!(error.failure_code(), error::IDENTITY_BINDING_TOO_OLD);
    }

    #[test]
    fn rejects_future_peer_binding_when_freshness_is_enforced() {
        let identity = identity(46, FUTURE_ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);
        let config = AukiP2pConfig {
            peer_binding_freshness: crate::PeerBindingFreshnessConfig::production_recommended(),
            ..AukiP2pConfig::development()
        };

        let error = validate(&identity.peer_id(), &handshake, &config).unwrap_err();

        assert!(matches!(
            error,
            HandshakePolicyError::PeerBindingFreshness(
                PeerBindingFreshnessError::BindingFromFuture { .. }
            )
        ));
        assert_eq!(error.failure_code(), error::IDENTITY_BINDING_FROM_FUTURE);
    }

    #[test]
    fn keeps_domain_authority_rejections_as_non_fatal_diagnostics() {
        let identity = identity(47, ISSUED_AT);
        let other_owner = wallet(48);
        let declaration = declaration(&other_owner);
        let domain_id = declaration.domain_id().unwrap().to_owned();
        let declared =
            auki_protocol::v1::authority::DeclaredDomain::new(domain_id.clone(), declaration, None);
        let handshake = build_local_handshake(&identity, vec![declared]);

        let result = validate(
            &identity.peer_id(),
            &handshake,
            &AukiP2pConfig::development(),
        )
        .expect("peer remains authorized");

        assert!(result.accepted_served_domains.is_empty());
        assert_eq!(result.rejected_declared_domains.len(), 1);
        assert_eq!(
            result.rejected_declared_domains[0].failure_code,
            error::DOMAIN_MISSING_DELEGATION
        );
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].scope, HandshakeFailureScope::Domain);
        assert_eq!(
            result.failures[0].domain_id.as_deref(),
            Some(domain_id.as_str())
        );
    }

    #[test]
    fn rejects_missing_required_authorization_material() {
        let identity = identity(49, ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);
        let peer_id = identity.peer_id();
        let config = AukiP2pConfig::development();
        let mut input = HandshakeValidationInput::new(&peer_id, &handshake, &config, NOW);
        input.required_authorization_material_types = &["auki.local.capability"];

        let error = validate_remote_handshake(input).unwrap_err();

        assert_eq!(
            error,
            HandshakePolicyError::MissingRequiredAuthorizationMaterial {
                material_type: "auki.local.capability".to_owned(),
            }
        );
        assert_eq!(
            error.failure_code(),
            error::HANDSHAKE_MISSING_REQUIRED_MATERIAL
        );
    }

    #[test]
    fn accepts_present_required_authorization_material() {
        let identity = identity(50, ISSUED_AT);
        let handshake = with_handshake_field(
            direct_owner_handshake(&identity),
            "authorization_material",
            json!([{ "type": "auki.local.capability" }]),
        );
        let peer_id = identity.peer_id();
        let config = AukiP2pConfig::development();
        let mut input = HandshakeValidationInput::new(&peer_id, &handshake, &config, NOW);
        input.required_authorization_material_types = &["auki.local.capability"];

        let result = validate_remote_handshake(input).expect("required material present");

        assert_eq!(result.accepted_served_domains.len(), 1);
    }

    #[test]
    fn rejects_domain_with_local_app_policy() {
        let identity = identity(51, ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);
        let config = AukiP2pConfig {
            domain_access_policy: DomainAccessPolicy::AppPolicy,
            ..AukiP2pConfig::development()
        };
        let peer_id = identity.peer_id();
        let mut input = HandshakeValidationInput::new(&peer_id, &handshake, &config, NOW);
        input.app_domain_access = AppDomainAccess::AllowOnly(&[]);

        let result = validate_remote_handshake(input).expect("peer remains authorized");

        assert!(result.accepted_served_domains.is_empty());
        assert_eq!(result.policy_rejected_domains.len(), 1);
        assert_eq!(
            result.policy_rejected_domains[0].failure_code,
            error::POLICY_DOMAIN_REJECTED
        );
        assert_eq!(result.failures[0].scope, HandshakeFailureScope::Policy);
    }

    #[test]
    fn rejects_missing_app_peer_decision_for_app_policy() {
        let identity = identity(52, ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);
        let config = AukiP2pConfig {
            peer_admission: PeerAdmissionConfig::AppPolicy,
            ..AukiP2pConfig::development()
        };

        let error = validate(&identity.peer_id(), &handshake, &config).unwrap_err();

        assert_eq!(
            error,
            HandshakePolicyError::MissingAppPeerAuthorizationDecision
        );
        assert_eq!(error.failure_code(), error::AUTHORIZATION_PEER_REJECTED);
        assert_eq!(
            config.peer_admission.mode(),
            PeerAuthorizationMode::AppPolicy
        );
    }

    #[test]
    fn rejects_metadata_that_exceeds_limit_before_authority_validation() {
        let identity = identity(53, ISSUED_AT);
        let handshake = with_handshake_field(
            direct_owner_handshake(&identity),
            "metadata",
            json!({ "large": "xxxxxxxxxxxxxxxx" }),
        );
        let mut config = AukiP2pConfig::development();
        config.limits.metadata_bytes = 8;

        let error = validate(&identity.peer_id(), &handshake, &config).unwrap_err();

        assert!(matches!(
            error,
            HandshakePolicyError::MetadataTooLarge {
                field: HandshakeMetadataField::Handshake,
                ..
            }
        ));
        assert_eq!(error.failure_code(), error::HANDSHAKE_INVALID_MESSAGE);
    }

    #[test]
    fn rejects_too_many_declared_domains_before_authority_validation() {
        let identity = identity(54, ISSUED_AT);
        let handshake = direct_owner_handshake(&identity);
        let mut config = AukiP2pConfig::development();
        config.limits.declared_domains_per_handshake = 0;

        let error = validate(&identity.peer_id(), &handshake, &config).unwrap_err();

        assert_eq!(
            error,
            HandshakePolicyError::Config(ConfigError::ZeroLimit {
                field: "declared_domains_per_handshake"
            })
        );

        config.limits.declared_domains_per_handshake = 1;
        let mut many_domains = handshake.into_value();
        let first = many_domains["declared_domains"][0].clone();
        many_domains["declared_domains"] = Value::Array(vec![first.clone(), first]);
        let many_domains = PeerHandshake::from_value(many_domains).unwrap();
        let error = validate(&identity.peer_id(), &many_domains, &config).unwrap_err();

        assert_eq!(
            error,
            HandshakePolicyError::TooManyDeclaredDomains { actual: 2, max: 1 }
        );
    }
}
