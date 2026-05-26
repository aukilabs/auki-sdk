//! Offer-catalog protocol helpers for v1.

use super::{
    base64url,
    domain::decode_domain_id,
    error,
    json::{JsonError, parse_json_object},
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, collections::HashSet, fmt, str::FromStr};

/// V1 offer-catalog fetch-path object type.
pub const OFFER_CATALOG_PATH_TYPE: &str = "auki.offer_catalog_path.v1";
/// V1 offer-catalog stream protocol id.
pub const OFFER_CATALOG_PROTOCOL_ID: &str = "/auki/offer-catalog/0.0.1";
/// V1 offer-catalog version string.
pub const OFFER_CATALOG_VERSION: &str = "auki.offer_catalog.v1";
/// V1 offer-catalog request object type.
pub const OFFER_CATALOG_REQUEST_TYPE: &str = "auki.offer_catalog_request.v1";
/// V1 offer-catalog response object type.
pub const OFFER_CATALOG_RESPONSE_TYPE: &str = "auki.offer_catalog_response.v1";

const FIELD_TYPE: &str = "type";
const FIELD_PROTOCOL_ID: &str = "protocol_id";
const FIELD_CATALOG_VERSION: &str = "catalog_version";
const FIELD_METADATA: &str = "metadata";
const FIELD_DOMAIN_IDS: &str = "domain_ids";
const FIELD_KINDS: &str = "kinds";
const FIELD_INCLUDE_INLINE_REGISTRY_ENTRIES: &str = "include_inline_registry_entries";
const FIELD_OFFERS: &str = "offers";
const FIELD_GENERATED_AT: &str = "generated_at";
const FIELD_DIAGNOSTICS: &str = "diagnostics";
const FIELD_OFFER_ID: &str = "offer_id";
const FIELD_DOMAIN_ID: &str = "domain_id";
const FIELD_KIND: &str = "kind";
const FIELD_STATUS: &str = "status";
const FIELD_ACCESS_MODES: &str = "access_modes";
const FIELD_PAYLOAD: &str = "payload";
const FIELD_REGISTRY_REFS: &str = "registry_refs";
const FIELD_DISPLAY_NAME: &str = "display_name";
const FIELD_UPDATED_AT: &str = "updated_at";
const FIELD_EXPIRES_AT: &str = "expires_at";
const FIELD_ENCODING: &str = "encoding";
const FIELD_SCHEMA_VERSION: &str = "schema_version";
const FIELD_MEDIA_TYPE: &str = "media_type";
const FIELD_REGISTRY: &str = "registry";
const FIELD_ROLE: &str = "role";
const FIELD_ID: &str = "id";
const FIELD_HASH: &str = "hash";
const FIELD_CANONICAL_JSON: &str = "canonical_json";
const FIELD_CODE: &str = "code";
const FIELD_MESSAGE: &str = "message";
const FIELD_RETRYABLE: &str = "retryable";
const FIELD_DETAILS: &str = "details";

/// Parsed v1 offer-catalog fetch path advertised in a peer handshake.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferCatalogPath {
    value: Value,
    /// Optional non-authoritative path metadata.
    pub metadata: Option<Value>,
}

/// Errors produced while creating or parsing an offer-catalog fetch path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferCatalogPathError {
    /// Path JSON value was not an object.
    NotObject,
    /// Required field was absent.
    MissingField {
        /// Field name.
        field: &'static str,
    },
    /// Field was present but had the wrong JSON type.
    InvalidFieldType {
        /// Field name.
        field: &'static str,
        /// Expected JSON type.
        expected: &'static str,
    },
    /// `type` was unsupported.
    UnsupportedType {
        /// Actual `type` value.
        actual: String,
    },
    /// `protocol_id` was unsupported.
    UnsupportedProtocolId {
        /// Actual `protocol_id` value.
        actual: String,
    },
    /// `catalog_version` was unsupported.
    UnsupportedCatalogVersion {
        /// Actual `catalog_version` value.
        actual: String,
    },
}

impl fmt::Display for OfferCatalogPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "offer catalog path is not a json object"),
            Self::MissingField { field } => write!(f, "offer catalog path missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "offer catalog path field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported offer catalog path type {actual}")
            }
            Self::UnsupportedProtocolId { actual } => {
                write!(f, "unsupported offer catalog protocol id {actual}")
            }
            Self::UnsupportedCatalogVersion { actual } => {
                write!(f, "unsupported offer catalog version {actual}")
            }
        }
    }
}

impl std::error::Error for OfferCatalogPathError {}

/// Parsed v1 offer-catalog request.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferCatalogRequest {
    value: Value,
    /// Domain filters requested by the consumer. Empty means all visible domains.
    pub domain_ids: Vec<String>,
    /// Offer-kind filters requested by the consumer. Empty means all kinds.
    pub kinds: Vec<String>,
    /// Whether inline canonical registry entries were requested.
    pub include_inline_registry_entries: bool,
}

/// Errors produced while creating or parsing an offer-catalog request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferCatalogRequestError {
    /// Request JSON value was not an object.
    NotObject,
    /// Required field was absent.
    MissingField {
        /// Field name.
        field: &'static str,
    },
    /// Field was present but had the wrong JSON type.
    InvalidFieldType {
        /// Field name.
        field: &'static str,
        /// Expected JSON type.
        expected: &'static str,
    },
    /// `type` was unsupported.
    UnsupportedType {
        /// Actual `type` value.
        actual: String,
    },
    /// A requested domain id was malformed.
    InvalidDomainId {
        /// Index in `domain_ids`.
        index: usize,
        /// Actual domain id string.
        domain_id: String,
        /// Error detail.
        error: String,
    },
}

impl OfferCatalogRequestError {
    /// Stable RFC failure code for this offer-catalog request error.
    pub fn failure_code(&self) -> &'static str {
        error::OFFER_INVALID_CATALOG_REQUEST
    }
}

impl fmt::Display for OfferCatalogRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "offer catalog request is not a json object"),
            Self::MissingField { field } => {
                write!(f, "offer catalog request missing field {field}")
            }
            Self::InvalidFieldType { field, expected } => {
                write!(f, "offer catalog request field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported offer catalog request type {actual}")
            }
            Self::InvalidDomainId {
                index,
                domain_id,
                error,
            } => write!(
                f,
                "invalid offer catalog request domain id at index {index} ({domain_id}): {error}"
            ),
        }
    }
}

impl std::error::Error for OfferCatalogRequestError {}

/// Parsed v1 offer-catalog response.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferCatalogResponse {
    value: Value,
    /// Offers visible to the requester for this complete snapshot.
    pub offers: Vec<Offer>,
    /// Optional response generation timestamp.
    pub generated_at: Option<String>,
    /// Diagnostic error objects returned with the response.
    pub diagnostics: Vec<Value>,
}

/// Parsed v1 offer object.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    value: Value,
    /// Producer-scoped offer id.
    pub offer_id: String,
    /// Domain id this offer is scoped to.
    pub domain_id: String,
    /// Open offer-kind string.
    pub kind: String,
    /// Producer-reported availability status.
    pub status: OfferStatus,
    /// Supported access modes.
    pub access_modes: Vec<OfferAccessMode>,
    /// Payload descriptor.
    pub payload: PayloadDescriptor,
    /// Registry references needed to interpret the offer.
    pub registry_refs: Vec<RegistryReference>,
    /// Optional display name.
    pub display_name: Option<String>,
    /// Optional update timestamp.
    pub updated_at: Option<String>,
    /// Optional expiry timestamp.
    pub expires_at: Option<String>,
    /// Optional non-authoritative metadata.
    pub metadata: Option<Value>,
}

/// V1 offer status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferStatus {
    /// The producer currently believes the offer can be used.
    Available,
    /// The offer is known but not currently usable.
    TemporarilyUnavailable,
}

impl OfferStatus {
    /// Return the RFC string value for this status.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::TemporarilyUnavailable => "temporarily_unavailable",
        }
    }
}

impl fmt::Display for OfferStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OfferStatus {
    type Err = OfferError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "available" => Ok(Self::Available),
            "temporarily_unavailable" => Ok(Self::TemporarilyUnavailable),
            _ => Err(OfferError::UnsupportedStatus {
                actual: value.to_owned(),
            }),
        }
    }
}

/// V1 offer access mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OfferAccessMode {
    /// One-shot Get access.
    Get,
    /// Ongoing Subscribe access.
    Subscribe,
}

impl OfferAccessMode {
    /// Return the RFC string value for this access mode.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Subscribe => "subscribe",
        }
    }
}

impl fmt::Display for OfferAccessMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OfferAccessMode {
    type Err = OfferError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "get" => Ok(Self::Get),
            "subscribe" => Ok(Self::Subscribe),
            _ => Err(OfferError::UnsupportedAccessMode {
                actual: value.to_owned(),
            }),
        }
    }
}

/// Parsed v1 payload descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct PayloadDescriptor {
    value: Value,
    /// Open payload family or schema type.
    pub payload_type: String,
    /// Optional payload encoding string.
    pub encoding: Option<String>,
    /// Optional schema-version string.
    pub schema_version: Option<String>,
    /// Optional media type.
    pub media_type: Option<String>,
}

/// Parsed v1 registry reference.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryReference {
    value: Value,
    /// Registry namespace.
    pub registry: String,
    /// Role of the referenced entry.
    pub role: String,
    /// Registry-local entry id.
    pub id: String,
    /// `sha256:<base64url>` content hash.
    pub hash: String,
    /// Optional RFC 8785 canonical registry JSON.
    pub canonical_json: Option<String>,
}

/// Errors produced while creating or parsing an offer-catalog response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferCatalogResponseError {
    /// Response JSON value was not an object.
    NotObject,
    /// Required field was absent.
    MissingField {
        /// Field name.
        field: &'static str,
    },
    /// Field was present but had the wrong JSON type.
    InvalidFieldType {
        /// Field name.
        field: &'static str,
        /// Expected JSON type.
        expected: &'static str,
    },
    /// `type` was unsupported.
    UnsupportedType {
        /// Actual `type` value.
        actual: String,
    },
    /// Timestamp was not an RFC3339 UTC string with `Z` suffix.
    InvalidTimestamp {
        /// Field name.
        field: &'static str,
        /// Actual timestamp value.
        value: String,
    },
    /// A diagnostic object was malformed.
    InvalidDiagnostic {
        /// Index in `diagnostics`.
        index: usize,
        /// Error detail.
        error: String,
    },
    /// An offer object was malformed.
    InvalidOffer {
        /// Index in `offers`.
        index: usize,
        /// Error detail.
        error: OfferError,
    },
    /// A response repeated a `(domain_id, offer_id)` tuple.
    DuplicateOffer {
        /// Repeated domain id.
        domain_id: String,
        /// Repeated offer id.
        offer_id: String,
    },
}

impl OfferCatalogResponseError {
    /// Stable RFC failure code for this offer-catalog response error.
    pub fn failure_code(&self) -> &'static str {
        error::OFFER_INVALID_CATALOG_RESPONSE
    }
}

impl fmt::Display for OfferCatalogResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "offer catalog response is not a json object"),
            Self::MissingField { field } => {
                write!(f, "offer catalog response missing field {field}")
            }
            Self::InvalidFieldType { field, expected } => {
                write!(f, "offer catalog response field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported offer catalog response type {actual}")
            }
            Self::InvalidTimestamp { field, value } => {
                write!(f, "invalid timestamp in response field {field}: {value}")
            }
            Self::InvalidDiagnostic { index, error } => {
                write!(f, "invalid diagnostic at index {index}: {error}")
            }
            Self::InvalidOffer { index, error } => {
                write!(f, "invalid offer at index {index}: {error}")
            }
            Self::DuplicateOffer {
                domain_id,
                offer_id,
            } => write!(
                f,
                "duplicate offer tuple in catalog response: {domain_id}/{offer_id}"
            ),
        }
    }
}

impl std::error::Error for OfferCatalogResponseError {}

/// Errors produced while creating or parsing an offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferError {
    /// Offer JSON value was not an object.
    NotObject,
    /// Required field was absent.
    MissingField {
        /// Field name.
        field: &'static str,
    },
    /// Field was present but had the wrong JSON type.
    InvalidFieldType {
        /// Field name.
        field: &'static str,
        /// Expected JSON type.
        expected: &'static str,
    },
    /// `domain_id` was malformed.
    InvalidDomainId {
        /// Actual domain id string.
        domain_id: String,
        /// Error detail.
        error: String,
    },
    /// `status` was unsupported.
    UnsupportedStatus {
        /// Actual status string.
        actual: String,
    },
    /// `access_modes` was empty.
    EmptyAccessModes,
    /// An access-mode string was unsupported.
    UnsupportedAccessMode {
        /// Actual access-mode string.
        actual: String,
    },
    /// An access mode appeared more than once.
    DuplicateAccessMode {
        /// Duplicated access mode.
        access_mode: OfferAccessMode,
    },
    /// Payload descriptor was malformed.
    InvalidPayload(PayloadDescriptorError),
    /// Registry reference was malformed.
    InvalidRegistryReference {
        /// Index in `registry_refs`.
        index: usize,
        /// Error detail.
        error: RegistryReferenceError,
    },
    /// Timestamp was not an RFC3339 UTC string with `Z` suffix.
    InvalidTimestamp {
        /// Field name.
        field: &'static str,
        /// Actual timestamp value.
        value: String,
    },
}

impl OfferError {
    /// Stable RFC failure code for this offer error.
    pub fn failure_code(&self) -> &'static str {
        error::OFFER_INVALID_OFFER
    }
}

impl fmt::Display for OfferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "offer is not a json object"),
            Self::MissingField { field } => write!(f, "offer missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "offer field {field} is not {expected}")
            }
            Self::InvalidDomainId { domain_id, error } => {
                write!(f, "invalid offer domain id {domain_id}: {error}")
            }
            Self::UnsupportedStatus { actual } => write!(f, "unsupported offer status {actual}"),
            Self::EmptyAccessModes => write!(f, "offer access_modes must be non-empty"),
            Self::UnsupportedAccessMode { actual } => {
                write!(f, "unsupported offer access mode {actual}")
            }
            Self::DuplicateAccessMode { access_mode } => {
                write!(f, "duplicate offer access mode {access_mode}")
            }
            Self::InvalidPayload(error) => write!(f, "invalid offer payload: {error}"),
            Self::InvalidRegistryReference { index, error } => {
                write!(
                    f,
                    "invalid offer registry reference at index {index}: {error}"
                )
            }
            Self::InvalidTimestamp { field, value } => {
                write!(f, "invalid timestamp in offer field {field}: {value}")
            }
        }
    }
}

impl std::error::Error for OfferError {}

/// Errors produced while creating or parsing a payload descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadDescriptorError {
    /// Payload descriptor JSON value was not an object.
    NotObject,
    /// Required field was absent.
    MissingField {
        /// Field name.
        field: &'static str,
    },
    /// Field was present but had the wrong JSON type.
    InvalidFieldType {
        /// Field name.
        field: &'static str,
        /// Expected JSON type.
        expected: &'static str,
    },
}

impl fmt::Display for PayloadDescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "payload descriptor is not a json object"),
            Self::MissingField { field } => write!(f, "payload descriptor missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "payload descriptor field {field} is not {expected}")
            }
        }
    }
}

impl std::error::Error for PayloadDescriptorError {}

/// Errors produced while creating or parsing a registry reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryReferenceError {
    /// Registry reference JSON value was not an object.
    NotObject,
    /// Required field was absent.
    MissingField {
        /// Field name.
        field: &'static str,
    },
    /// Field was present but had the wrong JSON type.
    InvalidFieldType {
        /// Field name.
        field: &'static str,
        /// Expected JSON type.
        expected: &'static str,
    },
    /// `hash` did not have the required `sha256:` prefix.
    InvalidHashPrefix {
        /// Actual hash string.
        hash: String,
    },
    /// `hash` digest was not canonical base64url over 32 bytes.
    InvalidHashDigest {
        /// Error detail.
        error: String,
    },
    /// `canonical_json` could not be parsed with v1 JSON rules.
    InvalidCanonicalJson(JsonError),
    /// `canonical_json` was parseable JSON, but not RFC 8785 canonical text.
    NonCanonicalJson,
    /// `canonical_json` did not match `hash`.
    CanonicalJsonHashMismatch {
        /// Expected hash.
        expected: String,
        /// Actual hash.
        actual: String,
    },
}

impl fmt::Display for RegistryReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "registry reference is not a json object"),
            Self::MissingField { field } => write!(f, "registry reference missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "registry reference field {field} is not {expected}")
            }
            Self::InvalidHashPrefix { hash } => {
                write!(f, "registry reference hash missing sha256 prefix: {hash}")
            }
            Self::InvalidHashDigest { error } => {
                write!(f, "invalid registry reference hash digest: {error}")
            }
            Self::InvalidCanonicalJson(error) => {
                write!(f, "invalid registry reference canonical_json: {error}")
            }
            Self::NonCanonicalJson => {
                write!(f, "registry reference canonical_json is not canonical")
            }
            Self::CanonicalJsonHashMismatch { expected, actual } => write!(
                f,
                "registry reference canonical_json hash mismatch: expected {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for RegistryReferenceError {}

/// Local policy decision for offer usability evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Allow this policy layer.
    Allow,
    /// Reject this policy layer with the supplied stable failure code.
    Reject {
        /// Stable failure code to surface for this policy rejection.
        failure_code: &'static str,
    },
}

/// Input for evaluating whether one parsed offer is usable for a requested path.
pub struct OfferUsabilityInput<'a> {
    /// Parsed offer to evaluate.
    pub offer: &'a Offer,
    /// Accepted served-domain ids for the producing peer relationship.
    pub accepted_served_domain_ids: &'a [String],
    /// Access mode the caller intends to use, if one is already known.
    pub requested_access_mode: Option<OfferAccessMode>,
    /// Locally supported offer kinds. `None` means this check is deferred.
    pub supported_kinds: Option<&'a [String]>,
    /// Locally supported payload types. `None` means this check is deferred.
    pub supported_payload_types: Option<&'a [String]>,
    /// Current UTC time for freshness checks. `None` defers expiry checks.
    pub now: Option<&'a str>,
    /// Local domain access policy decision.
    pub domain_policy: PolicyDecision,
    /// Local offer policy decision.
    pub offer_policy: PolicyDecision,
}

/// A successfully usable offer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsableOffer<'a> {
    /// The offer that passed usability evaluation.
    pub offer: &'a Offer,
}

/// Errors produced while evaluating offer usability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferUsabilityError {
    /// Offer domain is not in the accepted served-domain set.
    DomainNotServed {
        /// Offer domain id.
        domain_id: String,
    },
    /// Local domain access policy rejected the offer.
    DomainPolicyRejected {
        /// Stable policy failure code.
        failure_code: &'static str,
    },
    /// Local offer policy rejected the offer.
    OfferPolicyRejected {
        /// Stable policy failure code.
        failure_code: &'static str,
    },
    /// Offer kind is not supported by the caller.
    UnsupportedKind {
        /// Offer kind.
        kind: String,
    },
    /// Requested access mode is not present in the offer.
    UnsupportedAccessMode {
        /// Requested access mode.
        access_mode: OfferAccessMode,
    },
    /// Payload type is not supported by the caller.
    UnsupportedPayloadType {
        /// Payload descriptor type.
        payload_type: String,
    },
    /// Offer status is temporarily unavailable.
    TemporarilyUnavailable,
    /// Offer expired before the supplied `now`.
    Stale {
        /// Offer expiry timestamp.
        expires_at: String,
        /// Current timestamp used for the check.
        now: String,
    },
    /// Supplied `now` was not an RFC3339 UTC string with `Z` suffix.
    InvalidNowTimestamp {
        /// Invalid current timestamp.
        now: String,
    },
}

impl OfferUsabilityError {
    /// Stable RFC failure code for this offer-usability error.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::DomainNotServed { .. } => error::OFFER_DOMAIN_NOT_SERVED,
            Self::DomainPolicyRejected { failure_code }
            | Self::OfferPolicyRejected { failure_code } => failure_code,
            Self::UnsupportedKind { .. } => error::OFFER_UNSUPPORTED_KIND,
            Self::UnsupportedAccessMode { .. } => error::OFFER_UNSUPPORTED_ACCESS_MODE,
            Self::UnsupportedPayloadType { .. } => error::OFFER_UNSUPPORTED_PAYLOAD_TYPE,
            Self::TemporarilyUnavailable => error::OFFER_TEMPORARILY_UNAVAILABLE,
            Self::Stale { .. } | Self::InvalidNowTimestamp { .. } => error::OFFER_STALE,
        }
    }
}

impl fmt::Display for OfferUsabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DomainNotServed { domain_id } => {
                write!(f, "offer domain is not served: {domain_id}")
            }
            Self::DomainPolicyRejected { failure_code } => {
                write!(f, "domain policy rejected offer with {failure_code}")
            }
            Self::OfferPolicyRejected { failure_code } => {
                write!(f, "offer policy rejected offer with {failure_code}")
            }
            Self::UnsupportedKind { kind } => write!(f, "unsupported offer kind {kind}"),
            Self::UnsupportedAccessMode { access_mode } => {
                write!(f, "unsupported offer access mode {access_mode}")
            }
            Self::UnsupportedPayloadType { payload_type } => {
                write!(f, "unsupported offer payload type {payload_type}")
            }
            Self::TemporarilyUnavailable => write!(f, "offer is temporarily unavailable"),
            Self::Stale { expires_at, now } => {
                write!(f, "offer expired at {expires_at} before {now}")
            }
            Self::InvalidNowTimestamp { now } => write!(f, "invalid now timestamp {now}"),
        }
    }
}

impl std::error::Error for OfferUsabilityError {}

impl OfferCatalogPath {
    /// Create a v1 offer-catalog fetch path.
    pub fn create(metadata: Option<Value>) -> Result<Self, OfferCatalogPathError> {
        if let Some(metadata) = &metadata
            && !metadata.is_object()
        {
            return Err(OfferCatalogPathError::InvalidFieldType {
                field: FIELD_METADATA,
                expected: "an object",
            });
        }

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(OFFER_CATALOG_PATH_TYPE.to_owned()),
        );
        object.insert(
            FIELD_PROTOCOL_ID.to_owned(),
            Value::String(OFFER_CATALOG_PROTOCOL_ID.to_owned()),
        );
        object.insert(
            FIELD_CATALOG_VERSION.to_owned(),
            Value::String(OFFER_CATALOG_VERSION.to_owned()),
        );
        if let Some(metadata) = &metadata {
            object.insert(FIELD_METADATA.to_owned(), metadata.clone());
        }

        Ok(Self {
            value: Value::Object(object),
            metadata,
        })
    }

    /// Parse a v1 offer-catalog fetch path from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, OfferCatalogPathError> {
        let object = value.as_object().ok_or(OfferCatalogPathError::NotObject)?;

        let type_value = required_string(object, FIELD_TYPE)?;
        if type_value != OFFER_CATALOG_PATH_TYPE {
            return Err(OfferCatalogPathError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let protocol_id = required_string(object, FIELD_PROTOCOL_ID)?;
        if protocol_id != OFFER_CATALOG_PROTOCOL_ID {
            return Err(OfferCatalogPathError::UnsupportedProtocolId {
                actual: protocol_id.to_owned(),
            });
        }

        let catalog_version = required_string(object, FIELD_CATALOG_VERSION)?;
        if catalog_version != OFFER_CATALOG_VERSION {
            return Err(OfferCatalogPathError::UnsupportedCatalogVersion {
                actual: catalog_version.to_owned(),
            });
        }

        let metadata = object
            .get(FIELD_METADATA)
            .map(|value| {
                if value.is_object() {
                    Ok(value.clone())
                } else {
                    Err(OfferCatalogPathError::InvalidFieldType {
                        field: FIELD_METADATA,
                        expected: "an object",
                    })
                }
            })
            .transpose()?;

        Ok(Self { value, metadata })
    }

    /// Borrow the original offer-catalog path JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this path and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl OfferCatalogRequest {
    /// Create a v1 offer-catalog request.
    pub fn create(
        domain_ids: Vec<String>,
        kinds: Vec<String>,
        include_inline_registry_entries: bool,
    ) -> Result<Self, OfferCatalogRequestError> {
        validate_domain_ids(&domain_ids)?;

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(OFFER_CATALOG_REQUEST_TYPE.to_owned()),
        );
        if !domain_ids.is_empty() {
            object.insert(
                FIELD_DOMAIN_IDS.to_owned(),
                Value::Array(domain_ids.iter().cloned().map(Value::String).collect()),
            );
        }
        if !kinds.is_empty() {
            object.insert(
                FIELD_KINDS.to_owned(),
                Value::Array(kinds.iter().cloned().map(Value::String).collect()),
            );
        }
        if include_inline_registry_entries {
            object.insert(
                FIELD_INCLUDE_INLINE_REGISTRY_ENTRIES.to_owned(),
                Value::Bool(true),
            );
        }

        Ok(Self {
            value: Value::Object(object),
            domain_ids,
            kinds,
            include_inline_registry_entries,
        })
    }

    /// Parse a v1 offer-catalog request from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, OfferCatalogRequestError> {
        let object = value
            .as_object()
            .ok_or(OfferCatalogRequestError::NotObject)?;

        let type_value = required_request_string(object, FIELD_TYPE)?;
        if type_value != OFFER_CATALOG_REQUEST_TYPE {
            return Err(OfferCatalogRequestError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let domain_ids = optional_domain_ids(object)?;
        let kinds = optional_request_string_array(object, FIELD_KINDS)?;
        let include_inline_registry_entries = object
            .get(FIELD_INCLUDE_INLINE_REGISTRY_ENTRIES)
            .map(|value| {
                value
                    .as_bool()
                    .ok_or(OfferCatalogRequestError::InvalidFieldType {
                        field: FIELD_INCLUDE_INLINE_REGISTRY_ENTRIES,
                        expected: "a boolean",
                    })
            })
            .transpose()?
            .unwrap_or(false);

        Ok(Self {
            value,
            domain_ids,
            kinds,
            include_inline_registry_entries,
        })
    }

    /// Borrow the original offer-catalog request JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this request and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl OfferCatalogResponse {
    /// Create a v1 offer-catalog response.
    pub fn create(
        offers: Vec<Offer>,
        generated_at: Option<&str>,
        diagnostics: Vec<Value>,
    ) -> Result<Self, OfferCatalogResponseError> {
        if let Some(generated_at) = generated_at {
            validate_response_timestamp(FIELD_GENERATED_AT, generated_at)?;
        }
        validate_diagnostics(&diagnostics)?;
        validate_offer_tuples(&offers)?;

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(OFFER_CATALOG_RESPONSE_TYPE.to_owned()),
        );
        object.insert(
            FIELD_OFFERS.to_owned(),
            Value::Array(offers.iter().map(|offer| offer.value().clone()).collect()),
        );
        if let Some(generated_at) = generated_at {
            object.insert(
                FIELD_GENERATED_AT.to_owned(),
                Value::String(generated_at.to_owned()),
            );
        }
        if !diagnostics.is_empty() {
            object.insert(
                FIELD_DIAGNOSTICS.to_owned(),
                Value::Array(diagnostics.clone()),
            );
        }

        Ok(Self {
            value: Value::Object(object),
            offers,
            generated_at: generated_at.map(ToOwned::to_owned),
            diagnostics,
        })
    }

    /// Parse a v1 offer-catalog response from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, OfferCatalogResponseError> {
        let object = value
            .as_object()
            .ok_or(OfferCatalogResponseError::NotObject)?;

        let type_value = required_response_string(object, FIELD_TYPE)?;
        if type_value != OFFER_CATALOG_RESPONSE_TYPE {
            return Err(OfferCatalogResponseError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let offers = parse_response_offers(object)?;
        validate_offer_tuples(&offers)?;
        let generated_at = optional_response_timestamp(object, FIELD_GENERATED_AT)?;
        let diagnostics = parse_diagnostics(object)?;

        Ok(Self {
            value,
            offers,
            generated_at,
            diagnostics,
        })
    }

    /// Borrow the original offer-catalog response JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this response and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl Offer {
    /// Create a v1 offer object from parsed child objects.
    pub fn create(
        offer_id: impl Into<String>,
        domain_id: impl Into<String>,
        kind: impl Into<String>,
        status: OfferStatus,
        access_modes: Vec<OfferAccessMode>,
        payload: PayloadDescriptor,
        registry_refs: Vec<RegistryReference>,
    ) -> Result<Self, OfferError> {
        let offer_id = offer_id.into();
        let domain_id = domain_id.into();
        let kind = kind.into();
        validate_offer_domain_id(&domain_id)?;
        validate_access_modes(&access_modes)?;

        let mut object = Map::new();
        object.insert(FIELD_OFFER_ID.to_owned(), Value::String(offer_id.clone()));
        object.insert(FIELD_DOMAIN_ID.to_owned(), Value::String(domain_id.clone()));
        object.insert(FIELD_KIND.to_owned(), Value::String(kind.clone()));
        object.insert(
            FIELD_STATUS.to_owned(),
            Value::String(status.as_str().to_owned()),
        );
        object.insert(
            FIELD_ACCESS_MODES.to_owned(),
            Value::Array(
                access_modes
                    .iter()
                    .map(|mode| Value::String(mode.as_str().to_owned()))
                    .collect(),
            ),
        );
        object.insert(FIELD_PAYLOAD.to_owned(), payload.value().clone());
        object.insert(
            FIELD_REGISTRY_REFS.to_owned(),
            Value::Array(
                registry_refs
                    .iter()
                    .map(|reference| reference.value().clone())
                    .collect(),
            ),
        );

        Ok(Self {
            value: Value::Object(object),
            offer_id,
            domain_id,
            kind,
            status,
            access_modes,
            payload,
            registry_refs,
            display_name: None,
            updated_at: None,
            expires_at: None,
            metadata: None,
        })
    }

    /// Parse a v1 offer from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, OfferError> {
        let object = value.as_object().ok_or(OfferError::NotObject)?;

        let offer_id = required_offer_string(object, FIELD_OFFER_ID)?.to_owned();
        let domain_id = required_offer_string(object, FIELD_DOMAIN_ID)?.to_owned();
        validate_offer_domain_id(&domain_id)?;
        let kind = required_offer_string(object, FIELD_KIND)?.to_owned();
        let status = required_offer_string(object, FIELD_STATUS)?.parse()?;
        let access_modes = parse_access_modes(object)?;
        let payload = object
            .get(FIELD_PAYLOAD)
            .ok_or(OfferError::MissingField {
                field: FIELD_PAYLOAD,
            })
            .and_then(|value| {
                PayloadDescriptor::from_value(value.clone()).map_err(OfferError::InvalidPayload)
            })?;
        let registry_refs = parse_registry_refs(object)?;
        let display_name = optional_offer_string(object, FIELD_DISPLAY_NAME)?;
        let updated_at = optional_offer_timestamp(object, FIELD_UPDATED_AT)?;
        let expires_at = optional_offer_timestamp(object, FIELD_EXPIRES_AT)?;
        let metadata = optional_offer_object(object, FIELD_METADATA)?;

        Ok(Self {
            value,
            offer_id,
            domain_id,
            kind,
            status,
            access_modes,
            payload,
            registry_refs,
            display_name,
            updated_at,
            expires_at,
            metadata,
        })
    }

    /// Borrow the original offer JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this offer and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl PayloadDescriptor {
    /// Create a v1 payload descriptor.
    pub fn create(payload_type: impl Into<String>) -> Self {
        let payload_type = payload_type.into();
        let mut object = Map::new();
        object.insert(FIELD_TYPE.to_owned(), Value::String(payload_type.clone()));
        Self {
            value: Value::Object(object),
            payload_type,
            encoding: None,
            schema_version: None,
            media_type: None,
        }
    }

    /// Parse a v1 payload descriptor from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, PayloadDescriptorError> {
        let object = value.as_object().ok_or(PayloadDescriptorError::NotObject)?;

        let payload_type = required_payload_string(object, FIELD_TYPE)?.to_owned();
        let encoding = optional_payload_string(object, FIELD_ENCODING)?;
        let schema_version = optional_payload_string(object, FIELD_SCHEMA_VERSION)?;
        let media_type = optional_payload_string(object, FIELD_MEDIA_TYPE)?;

        Ok(Self {
            value,
            payload_type,
            encoding,
            schema_version,
            media_type,
        })
    }

    /// Borrow the original payload descriptor JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this descriptor and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl RegistryReference {
    /// Create a v1 registry reference.
    pub fn create(
        registry: impl Into<String>,
        role: impl Into<String>,
        id: impl Into<String>,
        hash: impl Into<String>,
        canonical_json: Option<String>,
    ) -> Result<Self, RegistryReferenceError> {
        let registry = registry.into();
        let role = role.into();
        let id = id.into();
        let hash = hash.into();
        validate_registry_hash(&hash, canonical_json.as_deref())?;

        let mut object = Map::new();
        object.insert(FIELD_REGISTRY.to_owned(), Value::String(registry.clone()));
        object.insert(FIELD_ROLE.to_owned(), Value::String(role.clone()));
        object.insert(FIELD_ID.to_owned(), Value::String(id.clone()));
        object.insert(FIELD_HASH.to_owned(), Value::String(hash.clone()));
        if let Some(canonical_json) = &canonical_json {
            object.insert(
                FIELD_CANONICAL_JSON.to_owned(),
                Value::String(canonical_json.clone()),
            );
        }

        Ok(Self {
            value: Value::Object(object),
            registry,
            role,
            id,
            hash,
            canonical_json,
        })
    }

    /// Parse a v1 registry reference from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, RegistryReferenceError> {
        let object = value.as_object().ok_or(RegistryReferenceError::NotObject)?;

        let registry = required_registry_string(object, FIELD_REGISTRY)?.to_owned();
        let role = required_registry_string(object, FIELD_ROLE)?.to_owned();
        let id = required_registry_string(object, FIELD_ID)?.to_owned();
        let hash = required_registry_string(object, FIELD_HASH)?.to_owned();
        let canonical_json = optional_registry_string(object, FIELD_CANONICAL_JSON)?;
        validate_registry_hash(&hash, canonical_json.as_deref())?;

        Ok(Self {
            value,
            registry,
            role,
            id,
            hash,
            canonical_json,
        })
    }

    /// Borrow the original registry reference JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this reference and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

/// Evaluate whether an offer is usable for one requested path.
pub fn evaluate_offer_usability(
    input: OfferUsabilityInput<'_>,
) -> Result<UsableOffer<'_>, OfferUsabilityError> {
    if !input
        .accepted_served_domain_ids
        .iter()
        .any(|domain_id| domain_id == &input.offer.domain_id)
    {
        return Err(OfferUsabilityError::DomainNotServed {
            domain_id: input.offer.domain_id.clone(),
        });
    }

    if let PolicyDecision::Reject { failure_code } = input.domain_policy {
        return Err(OfferUsabilityError::DomainPolicyRejected { failure_code });
    }

    if let PolicyDecision::Reject { failure_code } = input.offer_policy {
        return Err(OfferUsabilityError::OfferPolicyRejected { failure_code });
    }

    if input.offer.status == OfferStatus::TemporarilyUnavailable {
        return Err(OfferUsabilityError::TemporarilyUnavailable);
    }

    if let Some(requested_access_mode) = input.requested_access_mode
        && !input.offer.access_modes.contains(&requested_access_mode)
    {
        return Err(OfferUsabilityError::UnsupportedAccessMode {
            access_mode: requested_access_mode,
        });
    }

    if let Some(supported_kinds) = input.supported_kinds
        && !supported_kinds.iter().any(|kind| kind == &input.offer.kind)
    {
        return Err(OfferUsabilityError::UnsupportedKind {
            kind: input.offer.kind.clone(),
        });
    }

    if let Some(supported_payload_types) = input.supported_payload_types
        && !supported_payload_types
            .iter()
            .any(|payload_type| payload_type == &input.offer.payload.payload_type)
    {
        return Err(OfferUsabilityError::UnsupportedPayloadType {
            payload_type: input.offer.payload.payload_type.clone(),
        });
    }

    if let (Some(expires_at), Some(now)) = (&input.offer.expires_at, input.now) {
        let ordering = compare_rfc3339_z_timestamps(now, expires_at).ok_or_else(|| {
            OfferUsabilityError::InvalidNowTimestamp {
                now: now.to_owned(),
            }
        })?;
        if ordering != Ordering::Less {
            return Err(OfferUsabilityError::Stale {
                expires_at: expires_at.clone(),
                now: now.to_owned(),
            });
        }
    }

    Ok(UsableOffer { offer: input.offer })
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, OfferCatalogPathError> {
    object
        .get(field)
        .ok_or(OfferCatalogPathError::MissingField { field })?
        .as_str()
        .ok_or(OfferCatalogPathError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn required_request_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, OfferCatalogRequestError> {
    object
        .get(field)
        .ok_or(OfferCatalogRequestError::MissingField { field })?
        .as_str()
        .ok_or(OfferCatalogRequestError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn optional_domain_ids(
    object: &Map<String, Value>,
) -> Result<Vec<String>, OfferCatalogRequestError> {
    let domain_ids = optional_request_string_array(object, FIELD_DOMAIN_IDS)?;
    validate_domain_ids(&domain_ids)?;
    Ok(domain_ids)
}

fn optional_request_string_array(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Vec<String>, OfferCatalogRequestError> {
    object
        .get(field)
        .map(|value| {
            let values = value
                .as_array()
                .ok_or(OfferCatalogRequestError::InvalidFieldType {
                    field,
                    expected: "an array",
                })?;
            values
                .iter()
                .map(|value| {
                    value.as_str().map(ToOwned::to_owned).ok_or(
                        OfferCatalogRequestError::InvalidFieldType {
                            field,
                            expected: "an array of strings",
                        },
                    )
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn validate_domain_ids(domain_ids: &[String]) -> Result<(), OfferCatalogRequestError> {
    for (index, domain_id) in domain_ids.iter().enumerate() {
        decode_domain_id(domain_id).map_err(|error| OfferCatalogRequestError::InvalidDomainId {
            index,
            domain_id: domain_id.clone(),
            error: error.to_string(),
        })?;
    }
    Ok(())
}

fn required_response_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, OfferCatalogResponseError> {
    object
        .get(field)
        .ok_or(OfferCatalogResponseError::MissingField { field })?
        .as_str()
        .ok_or(OfferCatalogResponseError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn parse_response_offers(
    object: &Map<String, Value>,
) -> Result<Vec<Offer>, OfferCatalogResponseError> {
    let values = object
        .get(FIELD_OFFERS)
        .ok_or(OfferCatalogResponseError::MissingField {
            field: FIELD_OFFERS,
        })?
        .as_array()
        .ok_or(OfferCatalogResponseError::InvalidFieldType {
            field: FIELD_OFFERS,
            expected: "an array",
        })?;

    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Offer::from_value(value.clone())
                .map_err(|error| OfferCatalogResponseError::InvalidOffer { index, error })
        })
        .collect()
}

fn validate_offer_tuples(offers: &[Offer]) -> Result<(), OfferCatalogResponseError> {
    let mut seen = HashSet::with_capacity(offers.len());
    for offer in offers {
        let key = (offer.domain_id.clone(), offer.offer_id.clone());
        if !seen.insert(key.clone()) {
            return Err(OfferCatalogResponseError::DuplicateOffer {
                domain_id: key.0,
                offer_id: key.1,
            });
        }
    }
    Ok(())
}

fn optional_response_timestamp(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, OfferCatalogResponseError> {
    object
        .get(field)
        .map(|value| {
            let timestamp = value
                .as_str()
                .ok_or(OfferCatalogResponseError::InvalidFieldType {
                    field,
                    expected: "a string",
                })?;
            validate_response_timestamp(field, timestamp)?;
            Ok(timestamp.to_owned())
        })
        .transpose()
}

fn validate_response_timestamp(
    field: &'static str,
    value: &str,
) -> Result<(), OfferCatalogResponseError> {
    if is_rfc3339_z_timestamp(value) {
        Ok(())
    } else {
        Err(OfferCatalogResponseError::InvalidTimestamp {
            field,
            value: value.to_owned(),
        })
    }
}

fn parse_diagnostics(object: &Map<String, Value>) -> Result<Vec<Value>, OfferCatalogResponseError> {
    object
        .get(FIELD_DIAGNOSTICS)
        .map(|value| {
            let values = value
                .as_array()
                .ok_or(OfferCatalogResponseError::InvalidFieldType {
                    field: FIELD_DIAGNOSTICS,
                    expected: "an array",
                })?;
            let diagnostics = values.clone();
            validate_diagnostics(&diagnostics)?;
            Ok(diagnostics)
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn validate_diagnostics(diagnostics: &[Value]) -> Result<(), OfferCatalogResponseError> {
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        validate_error_object(diagnostic)
            .map_err(|error| OfferCatalogResponseError::InvalidDiagnostic { index, error })?;
    }
    Ok(())
}

fn validate_error_object(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "diagnostic is not a json object".to_owned())?;
    required_diagnostic_string(object, FIELD_CODE)?;
    optional_diagnostic_string(object, FIELD_MESSAGE)?;
    if let Some(domain_id) = optional_diagnostic_string(object, FIELD_DOMAIN_ID)? {
        decode_domain_id(&domain_id).map_err(|error| error.to_string())?;
    }
    optional_diagnostic_string(object, FIELD_OFFER_ID)?;
    optional_diagnostic_string(object, FIELD_KIND)?;
    optional_diagnostic_bool(object, FIELD_RETRYABLE)?;
    optional_diagnostic_object(object, FIELD_DETAILS)?;
    Ok(())
}

fn required_diagnostic_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<String, String> {
    object
        .get(field)
        .ok_or_else(|| format!("diagnostic missing field {field}"))?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("diagnostic field {field} is not a string"))
}

fn optional_diagnostic_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, String> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| format!("diagnostic field {field} is not a string"))
        })
        .transpose()
}

fn optional_diagnostic_bool(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<bool>, String> {
    object
        .get(field)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("diagnostic field {field} is not a boolean"))
        })
        .transpose()
}

fn optional_diagnostic_object(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Value>, String> {
    object
        .get(field)
        .map(|value| {
            if value.is_object() {
                Ok(value.clone())
            } else {
                Err(format!("diagnostic field {field} is not an object"))
            }
        })
        .transpose()
}

fn required_offer_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, OfferError> {
    object
        .get(field)
        .ok_or(OfferError::MissingField { field })?
        .as_str()
        .ok_or(OfferError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn optional_offer_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, OfferError> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or(OfferError::InvalidFieldType {
                    field,
                    expected: "a string",
                })
        })
        .transpose()
}

fn optional_offer_object(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Value>, OfferError> {
    object
        .get(field)
        .map(|value| {
            if value.is_object() {
                Ok(value.clone())
            } else {
                Err(OfferError::InvalidFieldType {
                    field,
                    expected: "an object",
                })
            }
        })
        .transpose()
}

fn validate_offer_domain_id(domain_id: &str) -> Result<(), OfferError> {
    decode_domain_id(domain_id).map_err(|error| OfferError::InvalidDomainId {
        domain_id: domain_id.to_owned(),
        error: error.to_string(),
    })?;
    Ok(())
}

fn parse_access_modes(object: &Map<String, Value>) -> Result<Vec<OfferAccessMode>, OfferError> {
    let values = object
        .get(FIELD_ACCESS_MODES)
        .ok_or(OfferError::MissingField {
            field: FIELD_ACCESS_MODES,
        })?
        .as_array()
        .ok_or(OfferError::InvalidFieldType {
            field: FIELD_ACCESS_MODES,
            expected: "an array",
        })?;

    if values.is_empty() {
        return Err(OfferError::EmptyAccessModes);
    }

    let mut access_modes = Vec::with_capacity(values.len());
    for value in values {
        let mode = value
            .as_str()
            .ok_or(OfferError::InvalidFieldType {
                field: FIELD_ACCESS_MODES,
                expected: "an array of strings",
            })?
            .parse()?;
        access_modes.push(mode);
    }

    validate_access_modes(&access_modes)?;
    Ok(access_modes)
}

fn validate_access_modes(access_modes: &[OfferAccessMode]) -> Result<(), OfferError> {
    if access_modes.is_empty() {
        return Err(OfferError::EmptyAccessModes);
    }

    let mut seen = HashSet::with_capacity(access_modes.len());
    for access_mode in access_modes {
        if !seen.insert(*access_mode) {
            return Err(OfferError::DuplicateAccessMode {
                access_mode: *access_mode,
            });
        }
    }
    Ok(())
}

fn parse_registry_refs(object: &Map<String, Value>) -> Result<Vec<RegistryReference>, OfferError> {
    let values = object
        .get(FIELD_REGISTRY_REFS)
        .ok_or(OfferError::MissingField {
            field: FIELD_REGISTRY_REFS,
        })?
        .as_array()
        .ok_or(OfferError::InvalidFieldType {
            field: FIELD_REGISTRY_REFS,
            expected: "an array",
        })?;

    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            RegistryReference::from_value(value.clone())
                .map_err(|error| OfferError::InvalidRegistryReference { index, error })
        })
        .collect()
}

fn optional_offer_timestamp(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, OfferError> {
    object
        .get(field)
        .map(|value| {
            let timestamp = value.as_str().ok_or(OfferError::InvalidFieldType {
                field,
                expected: "a string",
            })?;
            validate_offer_timestamp(field, timestamp)?;
            Ok(timestamp.to_owned())
        })
        .transpose()
}

fn validate_offer_timestamp(field: &'static str, value: &str) -> Result<(), OfferError> {
    if is_rfc3339_z_timestamp(value) {
        Ok(())
    } else {
        Err(OfferError::InvalidTimestamp {
            field,
            value: value.to_owned(),
        })
    }
}

fn required_payload_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, PayloadDescriptorError> {
    object
        .get(field)
        .ok_or(PayloadDescriptorError::MissingField { field })?
        .as_str()
        .ok_or(PayloadDescriptorError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn optional_payload_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, PayloadDescriptorError> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or(PayloadDescriptorError::InvalidFieldType {
                    field,
                    expected: "a string",
                })
        })
        .transpose()
}

fn required_registry_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, RegistryReferenceError> {
    object
        .get(field)
        .ok_or(RegistryReferenceError::MissingField { field })?
        .as_str()
        .ok_or(RegistryReferenceError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn optional_registry_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, RegistryReferenceError> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or(RegistryReferenceError::InvalidFieldType {
                    field,
                    expected: "a string",
                })
        })
        .transpose()
}

fn validate_registry_hash(
    hash: &str,
    canonical_json: Option<&str>,
) -> Result<(), RegistryReferenceError> {
    let digest =
        hash.strip_prefix("sha256:")
            .ok_or_else(|| RegistryReferenceError::InvalidHashPrefix {
                hash: hash.to_owned(),
            })?;
    base64url::decode_exact::<32>(digest).map_err(|error| {
        RegistryReferenceError::InvalidHashDigest {
            error: error.to_string(),
        }
    })?;

    if let Some(canonical_json) = canonical_json {
        let parsed = parse_json_object(canonical_json)
            .map_err(RegistryReferenceError::InvalidCanonicalJson)?;
        let canonical =
            String::from_utf8(auki_jcs::canonicalize(&parsed)).expect("JCS output is valid UTF-8");
        if canonical != canonical_json {
            return Err(RegistryReferenceError::NonCanonicalJson);
        }

        let expected = format!(
            "sha256:{}",
            base64url::encode(&Sha256::digest(canonical_json.as_bytes()))
        );
        if expected != hash {
            return Err(RegistryReferenceError::CanonicalJsonHashMismatch {
                expected,
                actual: hash.to_owned(),
            });
        }
    }

    Ok(())
}

fn is_rfc3339_z_timestamp(value: &str) -> bool {
    let Some(value) = value.strip_suffix('Z') else {
        return false;
    };
    let Some((date, time)) = value.split_once('T') else {
        return false;
    };

    let Some((year, month, day)) = parse_date(date) else {
        return false;
    };
    if year > 9999 || month == 0 || month > 12 {
        return false;
    }
    if day == 0 || day > days_in_month(year, month) {
        return false;
    }

    let Some((hour, minute, second, _fraction)) = parse_time(time) else {
        return false;
    };
    hour <= 23 && minute <= 59 && second <= 60
}

fn compare_rfc3339_z_timestamps(left: &str, right: &str) -> Option<Ordering> {
    let left = parse_rfc3339_z_timestamp(left)?;
    let right = parse_rfc3339_z_timestamp(right)?;

    Some(
        left.year
            .cmp(&right.year)
            .then_with(|| left.month.cmp(&right.month))
            .then_with(|| left.day.cmp(&right.day))
            .then_with(|| left.hour.cmp(&right.hour))
            .then_with(|| left.minute.cmp(&right.minute))
            .then_with(|| left.second.cmp(&right.second))
            .then_with(|| compare_fractional_seconds(left.fraction, right.fraction)),
    )
}

struct Rfc3339ZTimestamp<'a> {
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    fraction: &'a str,
}

fn parse_rfc3339_z_timestamp(value: &str) -> Option<Rfc3339ZTimestamp<'_>> {
    let value = value.strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;
    let (year, month, day) = parse_date(date)?;
    if year > 9999 || month == 0 || month > 12 {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let (hour, minute, second, fraction) = parse_time(time)?;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    Some(Rfc3339ZTimestamp {
        year,
        month,
        day,
        hour,
        minute,
        second,
        fraction,
    })
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

fn parse_time(value: &str) -> Option<(u32, u32, u32, &str)> {
    let (base, fraction) = match value.split_once('.') {
        Some((base, fraction)) => {
            if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            (base, fraction)
        }
        None => (value, ""),
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
    Some((hour, minute, second, fraction))
}

fn compare_fractional_seconds(left: &str, right: &str) -> Ordering {
    let left = left.trim_end_matches('0');
    let right = right.trim_end_matches('0');
    for (left_digit, right_digit) in left.bytes().zip(right.bytes()) {
        match left_digit.cmp(&right_digit) {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    left.len().cmp(&right.len())
}

fn parse_fixed_digits(value: &str) -> Option<u32> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::error;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    const DOMAIN_ID: &str = "noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs";

    fn canonical_hash(canonical_json: &str) -> String {
        format!(
            "sha256:{}",
            base64url::encode(&Sha256::digest(canonical_json.as_bytes()))
        )
    }

    fn registry_reference_value() -> Value {
        let canonical_json = r#"{"id":"clock-main","rate":30}"#;
        json!({
            "registry": "clock",
            "role": "clock",
            "id": "clock-main",
            "hash": canonical_hash(canonical_json),
            "canonical_json": canonical_json,
        })
    }

    fn offer_value() -> Value {
        json!({
            "offer_id": "camera-main",
            "domain_id": DOMAIN_ID,
            "kind": "sensor.frame",
            "status": "available",
            "access_modes": ["get", "subscribe"],
            "payload": {
                "type": "auki.frame",
                "encoding": "json",
                "schema_version": "1",
                "media_type": "application/json",
            },
            "registry_refs": [registry_reference_value()],
            "display_name": "Main Camera",
            "updated_at": "2026-05-26T12:00:00Z",
            "expires_at": "2026-05-26T13:00:00Z",
            "metadata": {"fixture": true},
        })
    }

    #[test]
    fn create_and_parse_offer_catalog_path() {
        let path = OfferCatalogPath::create(Some(json!({"label": "catalog"}))).unwrap();
        let parsed = OfferCatalogPath::from_value(path.value().clone()).unwrap();

        assert_eq!(parsed.value(), path.value());
        assert_eq!(parsed.metadata, Some(json!({"label": "catalog"})));
        assert_eq!(parsed.value()["type"], OFFER_CATALOG_PATH_TYPE);
        assert_eq!(parsed.value()["protocol_id"], OFFER_CATALOG_PROTOCOL_ID);
        assert_eq!(parsed.value()["catalog_version"], OFFER_CATALOG_VERSION);
    }

    #[test]
    fn create_rejects_non_object_metadata() {
        assert_eq!(
            OfferCatalogPath::create(Some(json!("bad"))),
            Err(OfferCatalogPathError::InvalidFieldType {
                field: FIELD_METADATA,
                expected: "an object",
            })
        );
    }

    #[test]
    fn from_value_rejects_missing_required_field() {
        assert_eq!(
            OfferCatalogPath::from_value(json!({
                "type": OFFER_CATALOG_PATH_TYPE,
                "protocol_id": OFFER_CATALOG_PROTOCOL_ID,
            })),
            Err(OfferCatalogPathError::MissingField {
                field: FIELD_CATALOG_VERSION,
            })
        );
    }

    #[test]
    fn from_value_rejects_unsupported_type() {
        assert_eq!(
            OfferCatalogPath::from_value(json!({
                "type": "future.path",
                "protocol_id": OFFER_CATALOG_PROTOCOL_ID,
                "catalog_version": OFFER_CATALOG_VERSION,
            })),
            Err(OfferCatalogPathError::UnsupportedType {
                actual: "future.path".to_owned(),
            })
        );
    }

    #[test]
    fn from_value_rejects_unsupported_protocol_id() {
        assert_eq!(
            OfferCatalogPath::from_value(json!({
                "type": OFFER_CATALOG_PATH_TYPE,
                "protocol_id": "/future/catalog/1.0.0",
                "catalog_version": OFFER_CATALOG_VERSION,
            })),
            Err(OfferCatalogPathError::UnsupportedProtocolId {
                actual: "/future/catalog/1.0.0".to_owned(),
            })
        );
    }

    #[test]
    fn from_value_rejects_unsupported_catalog_version() {
        assert_eq!(
            OfferCatalogPath::from_value(json!({
                "type": OFFER_CATALOG_PATH_TYPE,
                "protocol_id": OFFER_CATALOG_PROTOCOL_ID,
                "catalog_version": "future.catalog.v2",
            })),
            Err(OfferCatalogPathError::UnsupportedCatalogVersion {
                actual: "future.catalog.v2".to_owned(),
            })
        );
    }

    #[test]
    fn create_and_parse_offer_catalog_request() {
        let request = OfferCatalogRequest::create(
            vec!["noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs".to_owned()],
            vec!["sensor.frame".to_owned()],
            true,
        )
        .unwrap();
        let parsed = OfferCatalogRequest::from_value(request.value().clone()).unwrap();

        assert_eq!(parsed.value(), request.value());
        assert_eq!(
            parsed.domain_ids,
            vec!["noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs"]
        );
        assert_eq!(parsed.kinds, vec!["sensor.frame"]);
        assert!(parsed.include_inline_registry_entries);
    }

    #[test]
    fn create_omits_empty_request_filters_and_false_inline_flag() {
        let request = OfferCatalogRequest::create(vec![], vec![], false).unwrap();

        assert_eq!(
            request.value(),
            &json!({"type": OFFER_CATALOG_REQUEST_TYPE})
        );
        assert!(request.domain_ids.is_empty());
        assert!(request.kinds.is_empty());
        assert!(!request.include_inline_registry_entries);
    }

    #[test]
    fn request_from_value_defaults_omitted_optional_fields() {
        let request =
            OfferCatalogRequest::from_value(json!({"type": OFFER_CATALOG_REQUEST_TYPE})).unwrap();

        assert!(request.domain_ids.is_empty());
        assert!(request.kinds.is_empty());
        assert!(!request.include_inline_registry_entries);
    }

    #[test]
    fn request_from_value_accepts_empty_filter_arrays() {
        let request = OfferCatalogRequest::from_value(json!({
            "type": OFFER_CATALOG_REQUEST_TYPE,
            "domain_ids": [],
            "kinds": [],
        }))
        .unwrap();

        assert!(request.domain_ids.is_empty());
        assert!(request.kinds.is_empty());
    }

    #[test]
    fn request_from_value_rejects_unsupported_type() {
        let error = OfferCatalogRequest::from_value(json!({"type": "future.request"})).unwrap_err();

        assert_eq!(
            error,
            OfferCatalogRequestError::UnsupportedType {
                actual: "future.request".to_owned(),
            }
        );
        assert_eq!(error.failure_code(), error::OFFER_INVALID_CATALOG_REQUEST);
    }

    #[test]
    fn request_from_value_rejects_malformed_domain_id() {
        assert!(matches!(
            OfferCatalogRequest::from_value(json!({
                "type": OFFER_CATALOG_REQUEST_TYPE,
                "domain_ids": ["bad"],
            })),
            Err(OfferCatalogRequestError::InvalidDomainId { index: 0, .. })
        ));
    }

    #[test]
    fn request_from_value_rejects_non_string_kind() {
        assert_eq!(
            OfferCatalogRequest::from_value(json!({
                "type": OFFER_CATALOG_REQUEST_TYPE,
                "kinds": [42],
            })),
            Err(OfferCatalogRequestError::InvalidFieldType {
                field: FIELD_KINDS,
                expected: "an array of strings",
            })
        );
    }

    #[test]
    fn create_and_parse_registry_reference_with_canonical_json() {
        let canonical_json = r#"{"id":"clock-main","rate":30}"#;
        let hash = canonical_hash(canonical_json);
        let reference = RegistryReference::create(
            "clock",
            "clock",
            "clock-main",
            hash.clone(),
            Some(canonical_json.to_owned()),
        )
        .unwrap();
        let parsed = RegistryReference::from_value(reference.value().clone()).unwrap();

        assert_eq!(parsed.registry, "clock");
        assert_eq!(parsed.role, "clock");
        assert_eq!(parsed.id, "clock-main");
        assert_eq!(parsed.hash, hash);
        assert_eq!(parsed.canonical_json, Some(canonical_json.to_owned()));
    }

    #[test]
    fn registry_reference_rejects_canonical_json_hash_mismatch() {
        let error = RegistryReference::from_value(json!({
            "registry": "clock",
            "role": "clock",
            "id": "clock-main",
            "hash": "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "canonical_json": r#"{"id":"clock-main","rate":30}"#,
        }))
        .unwrap_err();

        assert!(matches!(
            error,
            RegistryReferenceError::CanonicalJsonHashMismatch { .. }
        ));
    }

    #[test]
    fn registry_reference_rejects_non_canonical_json() {
        assert_eq!(
            RegistryReference::from_value(json!({
                "registry": "clock",
                "role": "clock",
                "id": "clock-main",
                "hash": canonical_hash(r#"{"id":"clock-main","rate":30}"#),
                "canonical_json": r#"{"rate":30,"id":"clock-main"}"#,
            })),
            Err(RegistryReferenceError::NonCanonicalJson)
        );
    }

    #[test]
    fn create_and_parse_offer_catalog_response() {
        let response = OfferCatalogResponse::from_value(json!({
            "type": OFFER_CATALOG_RESPONSE_TYPE,
            "offers": [offer_value()],
            "generated_at": "2026-05-26T12:00:01Z",
            "diagnostics": [{
                "code": error::OFFER_DOMAIN_NOT_SERVED,
                "domain_id": DOMAIN_ID,
                "message": "not served for requester",
                "retryable": false,
                "details": {"requested": true},
            }],
        }))
        .unwrap();

        assert_eq!(
            response.generated_at,
            Some("2026-05-26T12:00:01Z".to_owned())
        );
        assert_eq!(response.diagnostics.len(), 1);
        assert_eq!(response.offers.len(), 1);

        let offer = &response.offers[0];
        assert_eq!(offer.offer_id, "camera-main");
        assert_eq!(offer.domain_id, DOMAIN_ID);
        assert_eq!(offer.kind, "sensor.frame");
        assert_eq!(offer.status, OfferStatus::Available);
        assert_eq!(
            offer.access_modes,
            vec![OfferAccessMode::Get, OfferAccessMode::Subscribe]
        );
        assert_eq!(offer.payload.payload_type, "auki.frame");
        assert_eq!(offer.registry_refs.len(), 1);
        assert_eq!(offer.display_name, Some("Main Camera".to_owned()));
    }

    #[test]
    fn create_offer_and_catalog_response_round_trip() {
        let payload = PayloadDescriptor::create("auki.frame");
        let reference = RegistryReference::from_value(registry_reference_value()).unwrap();
        let offer = Offer::create(
            "camera-main",
            DOMAIN_ID,
            "sensor.frame",
            OfferStatus::Available,
            vec![OfferAccessMode::Get],
            payload,
            vec![reference],
        )
        .unwrap();
        let response = OfferCatalogResponse::create(vec![offer], None, vec![]).unwrap();
        let parsed = OfferCatalogResponse::from_value(response.value().clone()).unwrap();

        assert_eq!(parsed.value(), response.value());
        assert_eq!(parsed.offers.len(), 1);
        assert!(parsed.generated_at.is_none());
        assert!(parsed.diagnostics.is_empty());
    }

    #[test]
    fn response_rejects_duplicate_domain_offer_tuple() {
        let error = OfferCatalogResponse::from_value(json!({
            "type": OFFER_CATALOG_RESPONSE_TYPE,
            "offers": [offer_value(), offer_value()],
        }))
        .unwrap_err();

        assert_eq!(
            error,
            OfferCatalogResponseError::DuplicateOffer {
                domain_id: DOMAIN_ID.to_owned(),
                offer_id: "camera-main".to_owned(),
            }
        );
        assert_eq!(error.failure_code(), error::OFFER_INVALID_CATALOG_RESPONSE);
    }

    #[test]
    fn response_rejects_malformed_diagnostic() {
        assert!(matches!(
            OfferCatalogResponse::from_value(json!({
                "type": OFFER_CATALOG_RESPONSE_TYPE,
                "offers": [],
                "diagnostics": [{"code": 42}],
            })),
            Err(OfferCatalogResponseError::InvalidDiagnostic { index: 0, .. })
        ));
    }

    #[test]
    fn offer_rejects_empty_access_modes() {
        let mut offer = offer_value();
        offer
            .as_object_mut()
            .unwrap()
            .insert(FIELD_ACCESS_MODES.to_owned(), json!([]));

        assert_eq!(Offer::from_value(offer), Err(OfferError::EmptyAccessModes));
    }

    #[test]
    fn offer_rejects_duplicate_access_modes() {
        let mut offer = offer_value();
        offer
            .as_object_mut()
            .unwrap()
            .insert(FIELD_ACCESS_MODES.to_owned(), json!(["get", "get"]));

        assert_eq!(
            Offer::from_value(offer),
            Err(OfferError::DuplicateAccessMode {
                access_mode: OfferAccessMode::Get,
            })
        );
    }

    #[test]
    fn offer_rejects_malformed_payload_descriptor() {
        let mut offer = offer_value();
        offer
            .as_object_mut()
            .unwrap()
            .insert(FIELD_PAYLOAD.to_owned(), json!({}));

        assert_eq!(
            Offer::from_value(offer),
            Err(OfferError::InvalidPayload(
                PayloadDescriptorError::MissingField { field: FIELD_TYPE }
            ))
        );
    }

    #[test]
    fn offer_usability_accepts_supported_available_offer() {
        let offer = Offer::from_value(offer_value()).unwrap();
        let accepted_domains = vec![DOMAIN_ID.to_owned()];
        let supported_kinds = vec!["sensor.frame".to_owned()];
        let supported_payloads = vec!["auki.frame".to_owned()];

        let usable = evaluate_offer_usability(OfferUsabilityInput {
            offer: &offer,
            accepted_served_domain_ids: &accepted_domains,
            requested_access_mode: Some(OfferAccessMode::Subscribe),
            supported_kinds: Some(&supported_kinds),
            supported_payload_types: Some(&supported_payloads),
            now: Some("2026-05-26T12:30:00Z"),
            domain_policy: PolicyDecision::Allow,
            offer_policy: PolicyDecision::Allow,
        })
        .unwrap();

        assert_eq!(usable.offer.offer_id, "camera-main");
    }

    #[test]
    fn offer_usability_rejects_unserved_domain() {
        let offer = Offer::from_value(offer_value()).unwrap();
        let accepted_domains = vec![];

        let error = evaluate_offer_usability(OfferUsabilityInput {
            offer: &offer,
            accepted_served_domain_ids: &accepted_domains,
            requested_access_mode: None,
            supported_kinds: None,
            supported_payload_types: None,
            now: None,
            domain_policy: PolicyDecision::Allow,
            offer_policy: PolicyDecision::Allow,
        })
        .unwrap_err();

        assert_eq!(
            error,
            OfferUsabilityError::DomainNotServed {
                domain_id: DOMAIN_ID.to_owned()
            }
        );
        assert_eq!(error.failure_code(), error::OFFER_DOMAIN_NOT_SERVED);
    }

    #[test]
    fn offer_usability_rejects_policy_denial() {
        let offer = Offer::from_value(offer_value()).unwrap();
        let accepted_domains = vec![DOMAIN_ID.to_owned()];

        let error = evaluate_offer_usability(OfferUsabilityInput {
            offer: &offer,
            accepted_served_domain_ids: &accepted_domains,
            requested_access_mode: None,
            supported_kinds: None,
            supported_payload_types: None,
            now: None,
            domain_policy: PolicyDecision::Reject {
                failure_code: error::POLICY_DOMAIN_REJECTED,
            },
            offer_policy: PolicyDecision::Allow,
        })
        .unwrap_err();

        assert_eq!(
            error,
            OfferUsabilityError::DomainPolicyRejected {
                failure_code: error::POLICY_DOMAIN_REJECTED,
            }
        );
        assert_eq!(error.failure_code(), error::POLICY_DOMAIN_REJECTED);
    }

    #[test]
    fn offer_usability_rejects_unsupported_access_kind_and_payload() {
        let mut offer_value = offer_value();
        offer_value
            .as_object_mut()
            .unwrap()
            .insert(FIELD_ACCESS_MODES.to_owned(), json!(["get"]));
        let offer = Offer::from_value(offer_value).unwrap();
        let accepted_domains = vec![DOMAIN_ID.to_owned()];
        let supported_kinds = vec!["other.kind".to_owned()];
        let supported_payloads = vec!["other.payload".to_owned()];

        let access_error = evaluate_offer_usability(OfferUsabilityInput {
            offer: &offer,
            accepted_served_domain_ids: &accepted_domains,
            requested_access_mode: Some(OfferAccessMode::Subscribe),
            supported_kinds: None,
            supported_payload_types: None,
            now: None,
            domain_policy: PolicyDecision::Allow,
            offer_policy: PolicyDecision::Allow,
        })
        .unwrap_err();
        assert_eq!(
            access_error,
            OfferUsabilityError::UnsupportedAccessMode {
                access_mode: OfferAccessMode::Subscribe,
            }
        );
        assert_eq!(
            access_error.failure_code(),
            error::OFFER_UNSUPPORTED_ACCESS_MODE
        );

        let kind_error = evaluate_offer_usability(OfferUsabilityInput {
            offer: &offer,
            accepted_served_domain_ids: &accepted_domains,
            requested_access_mode: Some(OfferAccessMode::Get),
            supported_kinds: Some(&supported_kinds),
            supported_payload_types: None,
            now: None,
            domain_policy: PolicyDecision::Allow,
            offer_policy: PolicyDecision::Allow,
        })
        .unwrap_err();
        assert_eq!(kind_error.failure_code(), error::OFFER_UNSUPPORTED_KIND);

        let payload_error = evaluate_offer_usability(OfferUsabilityInput {
            offer: &offer,
            accepted_served_domain_ids: &accepted_domains,
            requested_access_mode: Some(OfferAccessMode::Get),
            supported_kinds: None,
            supported_payload_types: Some(&supported_payloads),
            now: None,
            domain_policy: PolicyDecision::Allow,
            offer_policy: PolicyDecision::Allow,
        })
        .unwrap_err();
        assert_eq!(
            payload_error.failure_code(),
            error::OFFER_UNSUPPORTED_PAYLOAD_TYPE
        );
    }

    #[test]
    fn offer_usability_rejects_unavailable_and_stale_offers() {
        let mut unavailable_value = offer_value();
        unavailable_value
            .as_object_mut()
            .unwrap()
            .insert(FIELD_STATUS.to_owned(), json!("temporarily_unavailable"));
        let unavailable = Offer::from_value(unavailable_value).unwrap();
        let accepted_domains = vec![DOMAIN_ID.to_owned()];

        let unavailable_error = evaluate_offer_usability(OfferUsabilityInput {
            offer: &unavailable,
            accepted_served_domain_ids: &accepted_domains,
            requested_access_mode: None,
            supported_kinds: None,
            supported_payload_types: None,
            now: None,
            domain_policy: PolicyDecision::Allow,
            offer_policy: PolicyDecision::Allow,
        })
        .unwrap_err();
        assert_eq!(
            unavailable_error.failure_code(),
            error::OFFER_TEMPORARILY_UNAVAILABLE
        );

        let stale = Offer::from_value(offer_value()).unwrap();
        let stale_error = evaluate_offer_usability(OfferUsabilityInput {
            offer: &stale,
            accepted_served_domain_ids: &accepted_domains,
            requested_access_mode: None,
            supported_kinds: None,
            supported_payload_types: None,
            now: Some("2026-05-26T13:00:00Z"),
            domain_policy: PolicyDecision::Allow,
            offer_policy: PolicyDecision::Allow,
        })
        .unwrap_err();
        assert_eq!(
            stale_error,
            OfferUsabilityError::Stale {
                expires_at: "2026-05-26T13:00:00Z".to_owned(),
                now: "2026-05-26T13:00:00Z".to_owned(),
            }
        );
        assert_eq!(stale_error.failure_code(), error::OFFER_STALE);
    }
}
