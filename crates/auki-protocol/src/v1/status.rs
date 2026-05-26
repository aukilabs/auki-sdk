//! Status snapshot and diagnostic status helpers for v1.

use super::{authority::PeerAuthorizationMode, base64url, domain::decode_domain_id};
use libp2p_identity::PeerId;
use serde_json::{Map, Value};
use std::{fmt, str::FromStr};

/// V1 status snapshot object type.
pub const STATUS_SNAPSHOT_TYPE: &str = "auki.status_snapshot.v1";

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

const OBJECT_STATUS_SNAPSHOT: &str = "status snapshot";
const OBJECT_LOCAL_PEER_STATUS: &str = "local peer status";
const OBJECT_LOCAL_DOMAIN_STATUS: &str = "local domain status";
const OBJECT_DISCOVERY_STATUS: &str = "discovery status";
const OBJECT_REMOTE_PEER_STATUS: &str = "remote peer status";
const OBJECT_REJECTED_DOMAIN_STATUS: &str = "rejected domain status";
const OBJECT_OFFER_CATALOG_STATUS: &str = "offer catalog status";
const OBJECT_OFFER_STATUS: &str = "offer status";
const OBJECT_REGISTRY_REFERENCE_SUMMARY: &str = "registry reference summary";
const OBJECT_PATH_STATUS: &str = "path status";
const OBJECT_FAILURE_RECORD: &str = "failure record";

const FIELD_TYPE: &str = "type";
const FIELD_GENERATED_AT: &str = "generated_at";
const FIELD_LOCAL_PEER: &str = "local_peer";
const FIELD_LOCAL_DOMAINS: &str = "local_domains";
const FIELD_REMOTE_PEERS: &str = "remote_peers";
const FIELD_ACTIVE_PATHS: &str = "active_paths";
const FIELD_LAST_FAILURES: &str = "last_failures";
const FIELD_DISCOVERY: &str = "discovery";
const FIELD_METADATA: &str = "metadata";
const FIELD_PEER_ID: &str = "peer_id";
const FIELD_WALLET_PUBLIC_KEY: &str = "wallet_public_key";
const FIELD_PEER_BINDING_ISSUED_AT: &str = "peer_binding_issued_at";
const FIELD_PEER_BINDING_AGE_MS: &str = "peer_binding_age_ms";
const FIELD_PEER_BINDING_FRESH: &str = "peer_binding_fresh";
const FIELD_AUTHORIZATION_MODE: &str = "authorization_mode";
const FIELD_LISTEN_ADDRESSES: &str = "listen_addresses";
const FIELD_ADVERTISED_ADDRESSES: &str = "advertised_addresses";
const FIELD_DOMAIN_ID: &str = "domain_id";
const FIELD_ROLE: &str = "role";
const FIELD_DECLARATION_PRESENT: &str = "declaration_present";
const FIELD_DECLARATION_VALID: &str = "declaration_valid";
const FIELD_DELEGATION_PRESENT: &str = "delegation_present";
const FIELD_DELEGATION_VALID: &str = "delegation_valid";
const FIELD_DELEGATION_SCOPES: &str = "delegation_scopes";
const FIELD_DELEGATION_EXPIRES_AT: &str = "delegation_expires_at";
const FIELD_ADVERTISED: &str = "advertised";
const FIELD_SERVING_OFFERS: &str = "serving_offers";
const FIELD_LAST_FAILURE: &str = "last_failure";
const FIELD_ENABLED: &str = "enabled";
const FIELD_DISCOVERABLE: &str = "discoverable";
const FIELD_ADVERTISED_DOMAINS: &str = "advertised_domains";
const FIELD_LAST_REFRESH_AT: &str = "last_refresh_at";
const FIELD_EXPIRES_AT: &str = "expires_at";
const FIELD_DEGRADED: &str = "degraded";
const FIELD_LEARNED_FROM: &str = "learned_from";
const FIELD_DIALABLE: &str = "dialable";
const FIELD_CONNECTED: &str = "connected";
const FIELD_LIFECYCLE_STATE: &str = "lifecycle_state";
const FIELD_SELECTED_PROTOCOL_VERSION: &str = "selected_protocol_version";
const FIELD_AUTHORIZED: &str = "authorized";
const FIELD_VERIFIED_WALLET_PUBLIC_KEY: &str = "verified_wallet_public_key";
const FIELD_ACCEPTED_SERVED_DOMAINS: &str = "accepted_served_domains";
const FIELD_REJECTED_DOMAINS: &str = "rejected_domains";
const FIELD_OFFER_CATALOG_STATUS: &str = "offer_catalog_status";
const FIELD_LOADED_OFFERS: &str = "loaded_offers";
const FIELD_CODE: &str = "code";
const FIELD_MESSAGE: &str = "message";
const FIELD_PATH_AVAILABLE: &str = "path_available";
const FIELD_LAST_FETCH_AT: &str = "last_fetch_at";
const FIELD_LAST_SUCCESS_AT: &str = "last_success_at";
const FIELD_OFFER_ID: &str = "offer_id";
const FIELD_KIND: &str = "kind";
const FIELD_STATUS: &str = "status";
const FIELD_ACCESS_MODES: &str = "access_modes";
const FIELD_PAYLOAD_TYPE: &str = "payload_type";
const FIELD_REGISTRY_REFS: &str = "registry_refs";
const FIELD_USABLE: &str = "usable";
const FIELD_UNUSABLE_REASON: &str = "unusable_reason";
const FIELD_UPDATED_AT: &str = "updated_at";
const FIELD_REGISTRY: &str = "registry";
const FIELD_ID: &str = "id";
const FIELD_HASH: &str = "hash";
const FIELD_PATH_ID: &str = "path_id";
const FIELD_PATH_TYPE: &str = "path_type";
const FIELD_STATE: &str = "state";
const FIELD_STARTED_AT: &str = "started_at";
const FIELD_LAST_MESSAGE_AT: &str = "last_message_at";
const FIELD_LAST_SEQUENCE: &str = "last_sequence";
const FIELD_SEQUENCE_GAP_COUNT: &str = "sequence_gap_count";
const FIELD_LAST_ENVELOPE_FAILURE: &str = "last_envelope_failure";
const FIELD_LAST_PAYLOAD_FAILURE: &str = "last_payload_failure";
const FIELD_AT: &str = "at";
const FIELD_SCOPE: &str = "scope";
const FIELD_RETRYABLE: &str = "retryable";
const FIELD_DETAILS: &str = "details";

/// Parsed v1 status snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusSnapshot {
    value: Value,
    /// Snapshot generation timestamp.
    pub generated_at: String,
    /// Local peer diagnostic status.
    pub local_peer: LocalPeerStatus,
    /// Local domain diagnostic statuses.
    pub local_domains: Vec<LocalDomainStatus>,
    /// Remote peer diagnostic statuses.
    pub remote_peers: Vec<RemotePeerStatus>,
    /// Active or recently completed path statuses.
    pub active_paths: Vec<PathStatus>,
    /// Recent failure records.
    pub last_failures: Vec<FailureRecord>,
    /// Optional Discovery diagnostic status.
    pub discovery: Option<DiscoveryStatus>,
    /// Optional non-authoritative metadata.
    pub metadata: Option<Value>,
}

/// Parsed local peer diagnostic status.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalPeerStatus {
    value: Value,
    /// Local libp2p peer id string, when known.
    pub peer_id: Option<String>,
    /// Wallet public key string, when available.
    pub wallet_public_key: Option<String>,
    /// Peer binding issuance timestamp.
    pub peer_binding_issued_at: Option<String>,
    /// Peer binding age in milliseconds.
    pub peer_binding_age_ms: Option<u64>,
    /// Whether the peer binding is fresh under local policy.
    pub peer_binding_fresh: Option<bool>,
    /// Configured peer authorization mode.
    pub authorization_mode: Option<PeerAuthorizationMode>,
    /// Local listen multiaddr strings.
    pub listen_addresses: Vec<String>,
    /// Advertised multiaddr strings.
    pub advertised_addresses: Vec<String>,
}

/// Parsed local domain diagnostic status.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalDomainStatus {
    value: Value,
    /// Domain id string, when known.
    pub domain_id: Option<String>,
    /// Local diagnostic role.
    pub role: Option<LocalDomainRole>,
    /// Whether a domain declaration is present.
    pub declaration_present: Option<bool>,
    /// Whether the declaration is valid, or absent when not evaluated.
    pub declaration_valid: Option<bool>,
    /// Whether a delegation is present.
    pub delegation_present: Option<bool>,
    /// Whether the delegation is valid, or absent when not required/evaluated.
    pub delegation_valid: Option<bool>,
    /// Delegation scope strings.
    pub delegation_scopes: Vec<String>,
    /// Delegation expiry timestamp.
    pub delegation_expires_at: Option<String>,
    /// Whether the domain is advertised.
    pub advertised: Option<bool>,
    /// Whether offers are being served.
    pub serving_offers: Option<bool>,
    /// Last domain-level failure.
    pub last_failure: Option<FailureRecord>,
}

/// Local domain diagnostic role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDomainRole {
    /// Local peer is the domain owner.
    Owner,
    /// Local peer is a domain delegate.
    Delegate,
    /// Local peer tracks the domain locally without claiming authority.
    Managed,
}

impl LocalDomainRole {
    /// Return the RFC string value for this role.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Delegate => "delegate",
            Self::Managed => "managed",
        }
    }
}

impl fmt::Display for LocalDomainRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LocalDomainRole {
    type Err = StatusError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "owner" => Ok(Self::Owner),
            "delegate" => Ok(Self::Delegate),
            "managed" => Ok(Self::Managed),
            _ => Err(StatusError::UnsupportedLocalDomainRole {
                actual: value.to_owned(),
            }),
        }
    }
}

/// Parsed Discovery diagnostic status.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveryStatus {
    value: Value,
    /// Whether Discovery integration is enabled.
    pub enabled: Option<bool>,
    /// Whether this peer is currently discoverable.
    pub discoverable: Option<bool>,
    /// Domain ids advertised through Discovery.
    pub advertised_domains: Vec<String>,
    /// Addresses advertised through Discovery.
    pub advertised_addresses: Vec<String>,
    /// Last successful or attempted refresh timestamp.
    pub last_refresh_at: Option<String>,
    /// Discovery advertisement expiry timestamp.
    pub expires_at: Option<String>,
    /// Whether Discovery is degraded.
    pub degraded: Option<bool>,
    /// Last Discovery failure.
    pub last_failure: Option<FailureRecord>,
}

/// Parsed remote peer diagnostic status.
#[derive(Debug, Clone, PartialEq)]
pub struct RemotePeerStatus {
    value: Value,
    /// Remote libp2p peer id string, when known.
    pub peer_id: Option<String>,
    /// Source that introduced this peer.
    pub learned_from: Option<String>,
    /// Whether the peer is dialable, or absent when unknown.
    pub dialable: Option<bool>,
    /// Whether a transport connection currently exists.
    pub connected: Option<bool>,
    /// Lifecycle state string.
    pub lifecycle_state: Option<String>,
    /// Selected protocol version, if any.
    pub selected_protocol_version: Option<String>,
    /// Whether authorization has completed successfully.
    pub authorized: Option<bool>,
    /// Verified wallet public key, if known.
    pub verified_wallet_public_key: Option<String>,
    /// Accepted served-domain ids.
    pub accepted_served_domains: Vec<String>,
    /// Rejected domain statuses.
    pub rejected_domains: Vec<RejectedDomainStatus>,
    /// Offer-catalog status.
    pub offer_catalog_status: Option<OfferCatalogStatus>,
    /// Loaded offer statuses.
    pub loaded_offers: Vec<OfferStatus>,
    /// Last peer-level failure.
    pub last_failure: Option<FailureRecord>,
}

/// Parsed rejected domain diagnostic status.
#[derive(Debug, Clone, PartialEq)]
pub struct RejectedDomainStatus {
    value: Value,
    /// Domain id that was rejected, when known.
    pub domain_id: Option<String>,
    /// Stable failure code, when available.
    pub code: Option<String>,
    /// Optional diagnostic message.
    pub message: Option<String>,
}

/// Parsed offer-catalog diagnostic status.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferCatalogStatus {
    value: Value,
    /// Whether an offer-catalog path is available.
    pub path_available: Option<bool>,
    /// Last offer-catalog fetch timestamp.
    pub last_fetch_at: Option<String>,
    /// Last successful offer-catalog fetch timestamp.
    pub last_success_at: Option<String>,
    /// Last offer-catalog failure.
    pub last_failure: Option<FailureRecord>,
}

/// Parsed offer diagnostic status.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferStatus {
    value: Value,
    /// Producing peer id, when known.
    pub peer_id: Option<String>,
    /// Offer domain id, when known.
    pub domain_id: Option<String>,
    /// Producer-scoped offer id, when known.
    pub offer_id: Option<String>,
    /// Offer kind string.
    pub kind: Option<String>,
    /// Offer availability status string.
    pub status: Option<String>,
    /// Supported access-mode strings.
    pub access_modes: Vec<String>,
    /// Payload type string, when known.
    pub payload_type: Option<String>,
    /// Registry-reference summaries.
    pub registry_refs: Vec<RegistryReferenceSummary>,
    /// Whether this offer is locally usable.
    pub usable: Option<bool>,
    /// Stable unusable reason failure code, when unavailable.
    pub unusable_reason: Option<String>,
    /// Offer update timestamp.
    pub updated_at: Option<String>,
    /// Offer expiry timestamp.
    pub expires_at: Option<String>,
    /// Last offer-level failure.
    pub last_failure: Option<FailureRecord>,
}

/// Parsed registry-reference summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryReferenceSummary {
    value: Value,
    /// Registry namespace.
    pub registry: String,
    /// Role of the referenced entry.
    pub role: String,
    /// Registry-local entry id.
    pub id: String,
    /// `sha256:<base64url>` content hash.
    pub hash: String,
}

/// Parsed active or recent path diagnostic status.
#[derive(Debug, Clone, PartialEq)]
pub struct PathStatus {
    value: Value,
    /// Implementation-defined path id.
    pub path_id: Option<String>,
    /// Get or Subscribe path type.
    pub path_type: Option<PathType>,
    /// Producing peer id, when known.
    pub peer_id: Option<String>,
    /// Path domain id, when known.
    pub domain_id: Option<String>,
    /// Path offer id, when known.
    pub offer_id: Option<String>,
    /// Path state string.
    pub state: Option<String>,
    /// Path start timestamp.
    pub started_at: Option<String>,
    /// Last data-message timestamp.
    pub last_message_at: Option<String>,
    /// Payload type string, when known.
    pub payload_type: Option<String>,
    /// Last observed sequence value.
    pub last_sequence: Option<u64>,
    /// Number of sequence gaps observed.
    pub sequence_gap_count: Option<u64>,
    /// Last envelope-level failure.
    pub last_envelope_failure: Option<FailureRecord>,
    /// Last payload-level failure.
    pub last_payload_failure: Option<FailureRecord>,
    /// Last path-level failure.
    pub last_failure: Option<FailureRecord>,
}

/// Get or Subscribe path type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathType {
    /// One-shot Get path.
    Get,
    /// Ongoing Subscribe path.
    Subscribe,
}

impl PathType {
    /// Return the RFC string value for this path type.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Subscribe => "subscribe",
        }
    }
}

impl fmt::Display for PathType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PathType {
    type Err = StatusError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "get" => Ok(Self::Get),
            "subscribe" => Ok(Self::Subscribe),
            _ => Err(StatusError::UnsupportedPathType {
                actual: value.to_owned(),
            }),
        }
    }
}

/// Parsed diagnostic failure record.
#[derive(Debug, Clone, PartialEq)]
pub struct FailureRecord {
    value: Value,
    /// Stable failure code.
    pub code: String,
    /// Failure timestamp.
    pub at: String,
    /// Diagnostic failure scope.
    pub scope: String,
    /// Peer id string, when relevant.
    pub peer_id: Option<String>,
    /// Domain id string, when relevant.
    pub domain_id: Option<String>,
    /// Offer id string, when relevant.
    pub offer_id: Option<String>,
    /// Path id string, when relevant.
    pub path_id: Option<String>,
    /// Retry hint.
    pub retryable: Option<bool>,
    /// Optional diagnostic message.
    pub message: Option<String>,
    /// Optional structured diagnostic details.
    pub details: Option<Value>,
}

/// Errors produced while creating or parsing status objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusError {
    /// JSON value was not an object.
    NotObject {
        /// Object name.
        object: &'static str,
    },
    /// Required field was absent.
    MissingField {
        /// Object name.
        object: &'static str,
        /// Field name.
        field: &'static str,
    },
    /// Field was present but had the wrong JSON type.
    InvalidFieldType {
        /// Object name.
        object: &'static str,
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
    /// `peer_id` could not be parsed by libp2p.
    InvalidPeerId {
        /// Field name.
        field: &'static str,
        /// Actual peer id text.
        value: String,
    },
    /// `domain_id` was malformed.
    InvalidDomainId {
        /// Field name.
        field: &'static str,
        /// Actual domain id string.
        value: String,
        /// Error detail.
        error: String,
    },
    /// Wallet public key was malformed.
    InvalidWalletPublicKey {
        /// Field name.
        field: &'static str,
        /// Actual wallet public key string.
        value: String,
        /// Error detail.
        error: String,
    },
    /// Registry hash was malformed.
    InvalidRegistryHash {
        /// Field name.
        field: &'static str,
        /// Actual hash string.
        value: String,
        /// Error detail.
        error: String,
    },
    /// Decimal integer string was malformed.
    InvalidDecimalInteger {
        /// Field name.
        field: &'static str,
        /// Actual value.
        value: String,
    },
    /// Integer exceeded JSON safe integer range.
    IntegerTooLarge {
        /// Field name.
        field: &'static str,
        /// Actual value.
        value: u64,
    },
    /// Local domain role string was unsupported.
    UnsupportedLocalDomainRole {
        /// Actual role string.
        actual: String,
    },
    /// Peer authorization mode string was unsupported.
    UnsupportedAuthorizationMode {
        /// Actual authorization mode string.
        actual: String,
    },
    /// Path type string was unsupported.
    UnsupportedPathType {
        /// Actual path type string.
        actual: String,
    },
    /// An item in an array field was malformed.
    InvalidArrayItem {
        /// Array field name.
        field: &'static str,
        /// Item index.
        index: usize,
        /// Error detail.
        error: String,
    },
}

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject { object } => write!(f, "{object} is not a json object"),
            Self::MissingField { object, field } => {
                write!(f, "{object} missing field {field}")
            }
            Self::InvalidFieldType {
                object,
                field,
                expected,
            } => write!(f, "{object} field {field} is not {expected}"),
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported status snapshot type {actual}")
            }
            Self::InvalidTimestamp { field, value } => {
                write!(f, "invalid timestamp in field {field}: {value}")
            }
            Self::InvalidPeerId { field, value } => {
                write!(f, "invalid peer id in field {field}: {value}")
            }
            Self::InvalidDomainId {
                field,
                value,
                error,
            } => write!(f, "invalid domain id in field {field} ({value}): {error}"),
            Self::InvalidWalletPublicKey {
                field,
                value,
                error,
            } => write!(
                f,
                "invalid wallet public key in field {field} ({value}): {error}"
            ),
            Self::InvalidRegistryHash {
                field,
                value,
                error,
            } => write!(
                f,
                "invalid registry hash in field {field} ({value}): {error}"
            ),
            Self::InvalidDecimalInteger { field, value } => {
                write!(f, "invalid decimal integer in field {field}: {value}")
            }
            Self::IntegerTooLarge { field, value } => {
                write!(f, "integer in field {field} exceeds safe range: {value}")
            }
            Self::UnsupportedLocalDomainRole { actual } => {
                write!(f, "unsupported local domain role {actual}")
            }
            Self::UnsupportedAuthorizationMode { actual } => {
                write!(f, "unsupported authorization mode {actual}")
            }
            Self::UnsupportedPathType { actual } => {
                write!(f, "unsupported path type {actual}")
            }
            Self::InvalidArrayItem {
                field,
                index,
                error,
            } => write!(f, "invalid item at {field}[{index}]: {error}"),
        }
    }
}

impl std::error::Error for StatusError {}

impl StatusSnapshot {
    /// Create a v1 status snapshot from parsed child status objects.
    pub fn create(
        generated_at: impl Into<String>,
        local_peer: LocalPeerStatus,
        local_domains: Vec<LocalDomainStatus>,
        remote_peers: Vec<RemotePeerStatus>,
        active_paths: Vec<PathStatus>,
        last_failures: Vec<FailureRecord>,
        discovery: Option<DiscoveryStatus>,
        metadata: Option<Value>,
    ) -> Result<Self, StatusError> {
        let generated_at = generated_at.into();
        validate_timestamp(FIELD_GENERATED_AT, &generated_at)?;
        if let Some(metadata) = &metadata {
            validate_object_value(OBJECT_STATUS_SNAPSHOT, FIELD_METADATA, metadata)?;
        }

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(STATUS_SNAPSHOT_TYPE.to_owned()),
        );
        object.insert(
            FIELD_GENERATED_AT.to_owned(),
            Value::String(generated_at.clone()),
        );
        object.insert(FIELD_LOCAL_PEER.to_owned(), local_peer.value().clone());
        object.insert(
            FIELD_LOCAL_DOMAINS.to_owned(),
            Value::Array(
                local_domains
                    .iter()
                    .map(|status| status.value().clone())
                    .collect(),
            ),
        );
        object.insert(
            FIELD_REMOTE_PEERS.to_owned(),
            Value::Array(
                remote_peers
                    .iter()
                    .map(|status| status.value().clone())
                    .collect(),
            ),
        );
        object.insert(
            FIELD_ACTIVE_PATHS.to_owned(),
            Value::Array(
                active_paths
                    .iter()
                    .map(|status| status.value().clone())
                    .collect(),
            ),
        );
        object.insert(
            FIELD_LAST_FAILURES.to_owned(),
            Value::Array(
                last_failures
                    .iter()
                    .map(|failure| failure.value().clone())
                    .collect(),
            ),
        );
        if let Some(discovery) = &discovery {
            object.insert(FIELD_DISCOVERY.to_owned(), discovery.value().clone());
        }
        if let Some(metadata) = &metadata {
            object.insert(FIELD_METADATA.to_owned(), metadata.clone());
        }

        Ok(Self {
            value: Value::Object(object),
            generated_at,
            local_peer,
            local_domains,
            remote_peers,
            active_paths,
            last_failures,
            discovery,
            metadata,
        })
    }

    /// Parse a v1 status snapshot from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_STATUS_SNAPSHOT)?;
        let type_value = required_string(object, OBJECT_STATUS_SNAPSHOT, FIELD_TYPE)?;
        if type_value != STATUS_SNAPSHOT_TYPE {
            return Err(StatusError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let generated_at = required_timestamp(object, FIELD_GENERATED_AT)?;
        let local_peer = LocalPeerStatus::from_value(
            required_value(object, OBJECT_STATUS_SNAPSHOT, FIELD_LOCAL_PEER)?.clone(),
        )?;
        let local_domains =
            required_array_of(object, FIELD_LOCAL_DOMAINS, LocalDomainStatus::from_value)?;
        let remote_peers =
            required_array_of(object, FIELD_REMOTE_PEERS, RemotePeerStatus::from_value)?;
        let active_paths = required_array_of(object, FIELD_ACTIVE_PATHS, PathStatus::from_value)?;
        let last_failures =
            required_array_of(object, FIELD_LAST_FAILURES, FailureRecord::from_value)?;
        let discovery = optional_child(object, FIELD_DISCOVERY, DiscoveryStatus::from_value)?;
        let metadata = optional_object_value(object, OBJECT_STATUS_SNAPSHOT, FIELD_METADATA)?;

        Ok(Self {
            value,
            generated_at,
            local_peer,
            local_domains,
            remote_peers,
            active_paths,
            last_failures,
            discovery,
            metadata,
        })
    }

    /// Borrow the original status snapshot JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this snapshot and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl LocalPeerStatus {
    /// Parse a local peer status object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_LOCAL_PEER_STATUS)?;
        let peer_id = optional_peer_id(object, FIELD_PEER_ID)?;
        let wallet_public_key = optional_wallet_public_key(object, FIELD_WALLET_PUBLIC_KEY)?;
        let peer_binding_issued_at = optional_timestamp(object, FIELD_PEER_BINDING_ISSUED_AT)?;
        let peer_binding_age_ms = optional_safe_u64(object, FIELD_PEER_BINDING_AGE_MS)?;
        let peer_binding_fresh =
            optional_bool(object, OBJECT_LOCAL_PEER_STATUS, FIELD_PEER_BINDING_FRESH)?;
        let authorization_mode = optional_authorization_mode(object)?;
        let listen_addresses =
            optional_string_array(object, OBJECT_LOCAL_PEER_STATUS, FIELD_LISTEN_ADDRESSES)?;
        let advertised_addresses =
            optional_string_array(object, OBJECT_LOCAL_PEER_STATUS, FIELD_ADVERTISED_ADDRESSES)?;

        Ok(Self {
            value,
            peer_id,
            wallet_public_key,
            peer_binding_issued_at,
            peer_binding_age_ms,
            peer_binding_fresh,
            authorization_mode,
            listen_addresses,
            advertised_addresses,
        })
    }

    /// Borrow the original local peer status JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this status and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl LocalDomainStatus {
    /// Parse a local domain status object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_LOCAL_DOMAIN_STATUS)?;
        let domain_id = optional_domain_id(object, FIELD_DOMAIN_ID)?;
        let role = optional_string(object, OBJECT_LOCAL_DOMAIN_STATUS, FIELD_ROLE)?
            .map(|role| role.parse())
            .transpose()?;
        let declaration_present = optional_bool(
            object,
            OBJECT_LOCAL_DOMAIN_STATUS,
            FIELD_DECLARATION_PRESENT,
        )?;
        let declaration_valid =
            optional_bool(object, OBJECT_LOCAL_DOMAIN_STATUS, FIELD_DECLARATION_VALID)?;
        let delegation_present =
            optional_bool(object, OBJECT_LOCAL_DOMAIN_STATUS, FIELD_DELEGATION_PRESENT)?;
        let delegation_valid =
            optional_bool(object, OBJECT_LOCAL_DOMAIN_STATUS, FIELD_DELEGATION_VALID)?;
        let delegation_scopes =
            optional_string_array(object, OBJECT_LOCAL_DOMAIN_STATUS, FIELD_DELEGATION_SCOPES)?;
        let delegation_expires_at = optional_timestamp(object, FIELD_DELEGATION_EXPIRES_AT)?;
        let advertised = optional_bool(object, OBJECT_LOCAL_DOMAIN_STATUS, FIELD_ADVERTISED)?;
        let serving_offers =
            optional_bool(object, OBJECT_LOCAL_DOMAIN_STATUS, FIELD_SERVING_OFFERS)?;
        let last_failure = optional_child(object, FIELD_LAST_FAILURE, FailureRecord::from_value)?;

        Ok(Self {
            value,
            domain_id,
            role,
            declaration_present,
            declaration_valid,
            delegation_present,
            delegation_valid,
            delegation_scopes,
            delegation_expires_at,
            advertised,
            serving_offers,
            last_failure,
        })
    }

    /// Borrow the original local domain status JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this status and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl DiscoveryStatus {
    /// Parse a Discovery status object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_DISCOVERY_STATUS)?;
        let enabled = optional_bool(object, OBJECT_DISCOVERY_STATUS, FIELD_ENABLED)?;
        let discoverable = optional_bool(object, OBJECT_DISCOVERY_STATUS, FIELD_DISCOVERABLE)?;
        let advertised_domains = optional_domain_id_array(object, FIELD_ADVERTISED_DOMAINS)?;
        let advertised_addresses =
            optional_string_array(object, OBJECT_DISCOVERY_STATUS, FIELD_ADVERTISED_ADDRESSES)?;
        let last_refresh_at = optional_timestamp(object, FIELD_LAST_REFRESH_AT)?;
        let expires_at = optional_timestamp(object, FIELD_EXPIRES_AT)?;
        let degraded = optional_bool(object, OBJECT_DISCOVERY_STATUS, FIELD_DEGRADED)?;
        let last_failure = optional_child(object, FIELD_LAST_FAILURE, FailureRecord::from_value)?;

        Ok(Self {
            value,
            enabled,
            discoverable,
            advertised_domains,
            advertised_addresses,
            last_refresh_at,
            expires_at,
            degraded,
            last_failure,
        })
    }

    /// Borrow the original Discovery status JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this status and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl RemotePeerStatus {
    /// Parse a remote peer status object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_REMOTE_PEER_STATUS)?;
        let peer_id = optional_peer_id(object, FIELD_PEER_ID)?;
        let learned_from = optional_string(object, OBJECT_REMOTE_PEER_STATUS, FIELD_LEARNED_FROM)?;
        let dialable = optional_bool(object, OBJECT_REMOTE_PEER_STATUS, FIELD_DIALABLE)?;
        let connected = optional_bool(object, OBJECT_REMOTE_PEER_STATUS, FIELD_CONNECTED)?;
        let lifecycle_state =
            optional_string(object, OBJECT_REMOTE_PEER_STATUS, FIELD_LIFECYCLE_STATE)?;
        let selected_protocol_version = optional_string(
            object,
            OBJECT_REMOTE_PEER_STATUS,
            FIELD_SELECTED_PROTOCOL_VERSION,
        )?;
        let authorized = optional_bool(object, OBJECT_REMOTE_PEER_STATUS, FIELD_AUTHORIZED)?;
        let verified_wallet_public_key =
            optional_wallet_public_key(object, FIELD_VERIFIED_WALLET_PUBLIC_KEY)?;
        let accepted_served_domains =
            optional_domain_id_array(object, FIELD_ACCEPTED_SERVED_DOMAINS)?;
        let rejected_domains = optional_array_of(
            object,
            FIELD_REJECTED_DOMAINS,
            RejectedDomainStatus::from_value,
        )?;
        let offer_catalog_status = optional_child(
            object,
            FIELD_OFFER_CATALOG_STATUS,
            OfferCatalogStatus::from_value,
        )?;
        let loaded_offers =
            optional_array_of(object, FIELD_LOADED_OFFERS, OfferStatus::from_value)?;
        let last_failure = optional_child(object, FIELD_LAST_FAILURE, FailureRecord::from_value)?;

        Ok(Self {
            value,
            peer_id,
            learned_from,
            dialable,
            connected,
            lifecycle_state,
            selected_protocol_version,
            authorized,
            verified_wallet_public_key,
            accepted_served_domains,
            rejected_domains,
            offer_catalog_status,
            loaded_offers,
            last_failure,
        })
    }

    /// Borrow the original remote peer status JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this status and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl RejectedDomainStatus {
    /// Parse a rejected domain status object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_REJECTED_DOMAIN_STATUS)?;
        let domain_id = optional_domain_id(object, FIELD_DOMAIN_ID)?;
        let code = optional_string(object, OBJECT_REJECTED_DOMAIN_STATUS, FIELD_CODE)?;
        let message = optional_string(object, OBJECT_REJECTED_DOMAIN_STATUS, FIELD_MESSAGE)?;

        Ok(Self {
            value,
            domain_id,
            code,
            message,
        })
    }

    /// Borrow the original rejected domain status JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this status and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl OfferCatalogStatus {
    /// Parse an offer-catalog status object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_OFFER_CATALOG_STATUS)?;
        let path_available =
            optional_bool(object, OBJECT_OFFER_CATALOG_STATUS, FIELD_PATH_AVAILABLE)?;
        let last_fetch_at = optional_timestamp(object, FIELD_LAST_FETCH_AT)?;
        let last_success_at = optional_timestamp(object, FIELD_LAST_SUCCESS_AT)?;
        let last_failure = optional_child(object, FIELD_LAST_FAILURE, FailureRecord::from_value)?;

        Ok(Self {
            value,
            path_available,
            last_fetch_at,
            last_success_at,
            last_failure,
        })
    }

    /// Borrow the original offer-catalog status JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this status and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl OfferStatus {
    /// Parse an offer status object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_OFFER_STATUS)?;
        let peer_id = optional_peer_id(object, FIELD_PEER_ID)?;
        let domain_id = optional_domain_id(object, FIELD_DOMAIN_ID)?;
        let offer_id = optional_string(object, OBJECT_OFFER_STATUS, FIELD_OFFER_ID)?;
        let kind = optional_string(object, OBJECT_OFFER_STATUS, FIELD_KIND)?;
        let status = optional_string(object, OBJECT_OFFER_STATUS, FIELD_STATUS)?;
        let access_modes = optional_string_array(object, OBJECT_OFFER_STATUS, FIELD_ACCESS_MODES)?;
        let payload_type = optional_string(object, OBJECT_OFFER_STATUS, FIELD_PAYLOAD_TYPE)?;
        let registry_refs = optional_array_of(
            object,
            FIELD_REGISTRY_REFS,
            RegistryReferenceSummary::from_value,
        )?;
        let usable = optional_bool(object, OBJECT_OFFER_STATUS, FIELD_USABLE)?;
        let unusable_reason = optional_string(object, OBJECT_OFFER_STATUS, FIELD_UNUSABLE_REASON)?;
        let updated_at = optional_timestamp(object, FIELD_UPDATED_AT)?;
        let expires_at = optional_timestamp(object, FIELD_EXPIRES_AT)?;
        let last_failure = optional_child(object, FIELD_LAST_FAILURE, FailureRecord::from_value)?;

        Ok(Self {
            value,
            peer_id,
            domain_id,
            offer_id,
            kind,
            status,
            access_modes,
            payload_type,
            registry_refs,
            usable,
            unusable_reason,
            updated_at,
            expires_at,
            last_failure,
        })
    }

    /// Borrow the original offer status JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this status and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl RegistryReferenceSummary {
    /// Parse a registry-reference summary from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_REGISTRY_REFERENCE_SUMMARY)?;
        let registry =
            required_string(object, OBJECT_REGISTRY_REFERENCE_SUMMARY, FIELD_REGISTRY)?.to_owned();
        let role =
            required_string(object, OBJECT_REGISTRY_REFERENCE_SUMMARY, FIELD_ROLE)?.to_owned();
        let id = required_string(object, OBJECT_REGISTRY_REFERENCE_SUMMARY, FIELD_ID)?.to_owned();
        let hash =
            required_string(object, OBJECT_REGISTRY_REFERENCE_SUMMARY, FIELD_HASH)?.to_owned();
        validate_registry_hash(FIELD_HASH, &hash)?;

        Ok(Self {
            value,
            registry,
            role,
            id,
            hash,
        })
    }

    /// Borrow the original registry-reference summary JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this summary and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl PathStatus {
    /// Parse a path status object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_PATH_STATUS)?;
        let path_id = optional_string(object, OBJECT_PATH_STATUS, FIELD_PATH_ID)?;
        let path_type = optional_string(object, OBJECT_PATH_STATUS, FIELD_PATH_TYPE)?
            .map(|path_type| path_type.parse())
            .transpose()?;
        let peer_id = optional_peer_id(object, FIELD_PEER_ID)?;
        let domain_id = optional_domain_id(object, FIELD_DOMAIN_ID)?;
        let offer_id = optional_string(object, OBJECT_PATH_STATUS, FIELD_OFFER_ID)?;
        let state = optional_string(object, OBJECT_PATH_STATUS, FIELD_STATE)?;
        let started_at = optional_timestamp(object, FIELD_STARTED_AT)?;
        let last_message_at = optional_timestamp(object, FIELD_LAST_MESSAGE_AT)?;
        let payload_type = optional_string(object, OBJECT_PATH_STATUS, FIELD_PAYLOAD_TYPE)?;
        let last_sequence = optional_decimal_u64(object, FIELD_LAST_SEQUENCE)?;
        let sequence_gap_count = optional_safe_u64(object, FIELD_SEQUENCE_GAP_COUNT)?;
        let last_envelope_failure = optional_child(
            object,
            FIELD_LAST_ENVELOPE_FAILURE,
            FailureRecord::from_value,
        )?;
        let last_payload_failure = optional_child(
            object,
            FIELD_LAST_PAYLOAD_FAILURE,
            FailureRecord::from_value,
        )?;
        let last_failure = optional_child(object, FIELD_LAST_FAILURE, FailureRecord::from_value)?;

        Ok(Self {
            value,
            path_id,
            path_type,
            peer_id,
            domain_id,
            offer_id,
            state,
            started_at,
            last_message_at,
            payload_type,
            last_sequence,
            sequence_gap_count,
            last_envelope_failure,
            last_payload_failure,
            last_failure,
        })
    }

    /// Borrow the original path status JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this status and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl FailureRecord {
    /// Create a failure record with required fields.
    pub fn create(
        code: impl Into<String>,
        at: impl Into<String>,
        scope: impl Into<String>,
    ) -> Result<Self, StatusError> {
        let code = code.into();
        let at = at.into();
        let scope = scope.into();
        validate_timestamp(FIELD_AT, &at)?;

        let mut object = Map::new();
        object.insert(FIELD_CODE.to_owned(), Value::String(code.clone()));
        object.insert(FIELD_AT.to_owned(), Value::String(at.clone()));
        object.insert(FIELD_SCOPE.to_owned(), Value::String(scope.clone()));

        Ok(Self {
            value: Value::Object(object),
            code,
            at,
            scope,
            peer_id: None,
            domain_id: None,
            offer_id: None,
            path_id: None,
            retryable: None,
            message: None,
            details: None,
        })
    }

    /// Parse a failure record from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, StatusError> {
        let object = object(&value, OBJECT_FAILURE_RECORD)?;
        let code = required_string(object, OBJECT_FAILURE_RECORD, FIELD_CODE)?.to_owned();
        let at = required_timestamp(object, FIELD_AT)?;
        let scope = required_string(object, OBJECT_FAILURE_RECORD, FIELD_SCOPE)?.to_owned();
        let peer_id = optional_peer_id(object, FIELD_PEER_ID)?;
        let domain_id = optional_domain_id(object, FIELD_DOMAIN_ID)?;
        let offer_id = optional_string(object, OBJECT_FAILURE_RECORD, FIELD_OFFER_ID)?;
        let path_id = optional_string(object, OBJECT_FAILURE_RECORD, FIELD_PATH_ID)?;
        let retryable = optional_bool(object, OBJECT_FAILURE_RECORD, FIELD_RETRYABLE)?;
        let message = optional_string(object, OBJECT_FAILURE_RECORD, FIELD_MESSAGE)?;
        let details = optional_object_value(object, OBJECT_FAILURE_RECORD, FIELD_DETAILS)?;

        Ok(Self {
            value,
            code,
            at,
            scope,
            peer_id,
            domain_id,
            offer_id,
            path_id,
            retryable,
            message,
            details,
        })
    }

    /// Borrow the original failure record JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this failure record and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

fn object<'a>(
    value: &'a Value,
    object: &'static str,
) -> Result<&'a Map<String, Value>, StatusError> {
    value.as_object().ok_or(StatusError::NotObject { object })
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    object_name: &'static str,
    field: &'static str,
) -> Result<&'a Value, StatusError> {
    object.get(field).ok_or(StatusError::MissingField {
        object: object_name,
        field,
    })
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    object_name: &'static str,
    field: &'static str,
) -> Result<&'a str, StatusError> {
    required_value(object, object_name, field)?
        .as_str()
        .ok_or(StatusError::InvalidFieldType {
            object: object_name,
            field,
            expected: "a string",
        })
}

fn optional_string(
    object: &Map<String, Value>,
    object_name: &'static str,
    field: &'static str,
) -> Result<Option<String>, StatusError> {
    object
        .get(field)
        .map(|value| {
            if value.is_null() {
                Ok(None)
            } else {
                value.as_str().map(|value| Some(value.to_owned())).ok_or(
                    StatusError::InvalidFieldType {
                        object: object_name,
                        field,
                        expected: "a string or null",
                    },
                )
            }
        })
        .transpose()
        .map(Option::flatten)
}

fn optional_bool(
    object: &Map<String, Value>,
    object_name: &'static str,
    field: &'static str,
) -> Result<Option<bool>, StatusError> {
    object
        .get(field)
        .map(|value| {
            if value.is_null() {
                Ok(None)
            } else {
                value
                    .as_bool()
                    .map(Some)
                    .ok_or(StatusError::InvalidFieldType {
                        object: object_name,
                        field,
                        expected: "a boolean or null",
                    })
            }
        })
        .transpose()
        .map(Option::flatten)
}

fn optional_safe_u64(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<u64>, StatusError> {
    object
        .get(field)
        .map(|value| {
            if value.is_null() {
                Ok(None)
            } else {
                let integer = value.as_u64().ok_or(StatusError::InvalidFieldType {
                    object: OBJECT_STATUS_SNAPSHOT,
                    field,
                    expected: "a non-negative safe integer or null",
                })?;
                validate_safe_integer(field, integer)?;
                Ok(Some(integer))
            }
        })
        .transpose()
        .map(Option::flatten)
}

fn required_timestamp(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<String, StatusError> {
    let timestamp = required_string(object, OBJECT_STATUS_SNAPSHOT, field)?;
    validate_timestamp(field, timestamp)?;
    Ok(timestamp.to_owned())
}

fn optional_timestamp(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, StatusError> {
    object
        .get(field)
        .map(|value| {
            if value.is_null() {
                Ok(None)
            } else {
                let timestamp = value.as_str().ok_or(StatusError::InvalidFieldType {
                    object: OBJECT_STATUS_SNAPSHOT,
                    field,
                    expected: "a timestamp string or null",
                })?;
                validate_timestamp(field, timestamp)?;
                Ok(Some(timestamp.to_owned()))
            }
        })
        .transpose()
        .map(Option::flatten)
}

fn optional_peer_id(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, StatusError> {
    let Some(peer_id) = optional_string(object, OBJECT_STATUS_SNAPSHOT, field)? else {
        return Ok(None);
    };
    PeerId::from_str(&peer_id).map_err(|_| StatusError::InvalidPeerId {
        field,
        value: peer_id.clone(),
    })?;
    Ok(Some(peer_id))
}

fn optional_domain_id(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, StatusError> {
    let Some(domain_id) = optional_string(object, OBJECT_STATUS_SNAPSHOT, field)? else {
        return Ok(None);
    };
    validate_domain_id(field, &domain_id)?;
    Ok(Some(domain_id))
}

fn optional_wallet_public_key(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, StatusError> {
    let Some(wallet_public_key) = optional_string(object, OBJECT_STATUS_SNAPSHOT, field)? else {
        return Ok(None);
    };
    base64url::decode_exact::<32>(&wallet_public_key).map_err(|error| {
        StatusError::InvalidWalletPublicKey {
            field,
            value: wallet_public_key.clone(),
            error: error.to_string(),
        }
    })?;
    Ok(Some(wallet_public_key))
}

fn optional_authorization_mode(
    object: &Map<String, Value>,
) -> Result<Option<PeerAuthorizationMode>, StatusError> {
    optional_string(object, OBJECT_LOCAL_PEER_STATUS, FIELD_AUTHORIZATION_MODE)?
        .map(|mode| {
            mode.parse::<PeerAuthorizationMode>()
                .map_err(|_| StatusError::UnsupportedAuthorizationMode { actual: mode })
        })
        .transpose()
}

fn optional_decimal_u64(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<u64>, StatusError> {
    let Some(decimal) = optional_string(object, OBJECT_PATH_STATUS, field)? else {
        return Ok(None);
    };
    validate_decimal_integer(field, &decimal)?;
    decimal
        .parse::<u64>()
        .map(Some)
        .map_err(|_| StatusError::InvalidDecimalInteger {
            field,
            value: decimal,
        })
}

fn optional_string_array(
    object: &Map<String, Value>,
    object_name: &'static str,
    field: &'static str,
) -> Result<Vec<String>, StatusError> {
    object
        .get(field)
        .map(|value| {
            let values = value.as_array().ok_or(StatusError::InvalidFieldType {
                object: object_name,
                field,
                expected: "an array",
            })?;
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        StatusError::InvalidArrayItem {
                            field,
                            index,
                            error: "item is not a string".to_owned(),
                        }
                    })
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn optional_domain_id_array(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Vec<String>, StatusError> {
    let values = optional_string_array(object, OBJECT_STATUS_SNAPSHOT, field)?;
    for (index, domain_id) in values.iter().enumerate() {
        validate_domain_id(field, domain_id).map_err(|error| StatusError::InvalidArrayItem {
            field,
            index,
            error: error.to_string(),
        })?;
    }
    Ok(values)
}

fn required_array_of<T, F>(
    object: &Map<String, Value>,
    field: &'static str,
    parse: F,
) -> Result<Vec<T>, StatusError>
where
    F: Fn(Value) -> Result<T, StatusError>,
{
    let values = required_value(object, OBJECT_STATUS_SNAPSHOT, field)?
        .as_array()
        .ok_or(StatusError::InvalidFieldType {
            object: OBJECT_STATUS_SNAPSHOT,
            field,
            expected: "an array",
        })?;
    parse_array(values, field, parse)
}

fn optional_array_of<T, F>(
    object: &Map<String, Value>,
    field: &'static str,
    parse: F,
) -> Result<Vec<T>, StatusError>
where
    F: Fn(Value) -> Result<T, StatusError>,
{
    object
        .get(field)
        .map(|value| {
            let values = value.as_array().ok_or(StatusError::InvalidFieldType {
                object: OBJECT_STATUS_SNAPSHOT,
                field,
                expected: "an array",
            })?;
            parse_array(values, field, parse)
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn parse_array<T, F>(values: &[Value], field: &'static str, parse: F) -> Result<Vec<T>, StatusError>
where
    F: Fn(Value) -> Result<T, StatusError>,
{
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            parse(value.clone()).map_err(|error| StatusError::InvalidArrayItem {
                field,
                index,
                error: error.to_string(),
            })
        })
        .collect()
}

fn optional_child<T, F>(
    object: &Map<String, Value>,
    field: &'static str,
    parse: F,
) -> Result<Option<T>, StatusError>
where
    F: Fn(Value) -> Result<T, StatusError>,
{
    object
        .get(field)
        .map(|value| {
            if value.is_null() {
                Ok(None)
            } else {
                parse(value.clone()).map(Some)
            }
        })
        .transpose()
        .map(Option::flatten)
}

fn optional_object_value(
    object: &Map<String, Value>,
    object_name: &'static str,
    field: &'static str,
) -> Result<Option<Value>, StatusError> {
    object
        .get(field)
        .map(|value| {
            if value.is_null() {
                Ok(None)
            } else {
                validate_object_value(object_name, field, value)?;
                Ok(Some(value.clone()))
            }
        })
        .transpose()
        .map(Option::flatten)
}

fn validate_object_value(
    object_name: &'static str,
    field: &'static str,
    value: &Value,
) -> Result<(), StatusError> {
    if value.is_object() {
        Ok(())
    } else {
        Err(StatusError::InvalidFieldType {
            object: object_name,
            field,
            expected: "an object",
        })
    }
}

fn validate_domain_id(field: &'static str, domain_id: &str) -> Result<(), StatusError> {
    decode_domain_id(domain_id).map_err(|error| StatusError::InvalidDomainId {
        field,
        value: domain_id.to_owned(),
        error: error.to_string(),
    })?;
    Ok(())
}

fn validate_registry_hash(field: &'static str, hash: &str) -> Result<(), StatusError> {
    let digest = hash
        .strip_prefix("sha256:")
        .ok_or_else(|| StatusError::InvalidRegistryHash {
            field,
            value: hash.to_owned(),
            error: "missing sha256 prefix".to_owned(),
        })?;
    base64url::decode_exact::<32>(digest).map_err(|error| StatusError::InvalidRegistryHash {
        field,
        value: hash.to_owned(),
        error: error.to_string(),
    })?;
    Ok(())
}

fn validate_safe_integer(field: &'static str, value: u64) -> Result<(), StatusError> {
    if value > MAX_SAFE_INTEGER {
        Err(StatusError::IntegerTooLarge { field, value })
    } else {
        Ok(())
    }
}

fn validate_decimal_integer(field: &'static str, value: &str) -> Result<(), StatusError> {
    let valid = value == "0"
        || value
            .strip_prefix(|byte: char| ('1'..='9').contains(&byte))
            .is_some_and(|rest| rest.bytes().all(|byte| byte.is_ascii_digit()));
    if valid {
        Ok(())
    } else {
        Err(StatusError::InvalidDecimalInteger {
            field,
            value: value.to_owned(),
        })
    }
}

fn validate_timestamp(field: &'static str, value: &str) -> Result<(), StatusError> {
    if is_rfc3339_z_timestamp(value) {
        Ok(())
    } else {
        Err(StatusError::InvalidTimestamp {
            field,
            value: value.to_owned(),
        })
    }
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

    let Some((hour, minute, second)) = parse_time(time) else {
        return false;
    };
    hour <= 23 && minute <= 59 && second <= 60
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

fn parse_time(value: &str) -> Option<(u32, u32, u32)> {
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
    let _ = fraction;
    Some((hour, minute, second))
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
    use crate::v1::{base64url, error};
    use serde_json::json;

    const DOMAIN_ID: &str = "noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs";
    const PEER_ID: &str = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
    const GENERATED_AT: &str = "2026-05-26T12:00:00Z";

    fn wallet_public_key() -> String {
        base64url::encode(&[1u8; 32])
    }

    fn local_peer_value() -> Value {
        json!({
            "peer_id": PEER_ID,
            "wallet_public_key": wallet_public_key(),
            "peer_binding_issued_at": GENERATED_AT,
            "peer_binding_age_ms": 500,
            "peer_binding_fresh": true,
            "authorization_mode": "whitelisted-only",
            "listen_addresses": ["/ip4/0.0.0.0/tcp/0"],
            "advertised_addresses": ["/ip4/203.0.113.1/tcp/4001"],
        })
    }

    fn failure_value() -> Value {
        json!({
            "code": error::OFFER_UNKNOWN_OFFER,
            "at": GENERATED_AT,
            "scope": "offer",
            "peer_id": PEER_ID,
            "domain_id": DOMAIN_ID,
            "offer_id": "camera-main",
            "path_id": "path-1",
            "retryable": false,
            "message": "unknown offer",
            "details": {"missing": true},
        })
    }

    fn status_snapshot_value() -> Value {
        json!({
            "type": STATUS_SNAPSHOT_TYPE,
            "generated_at": GENERATED_AT,
            "local_peer": local_peer_value(),
            "local_domains": [{
                "domain_id": DOMAIN_ID,
                "role": "owner",
                "declaration_present": true,
                "declaration_valid": true,
                "delegation_present": false,
                "delegation_valid": null,
                "delegation_scopes": ["serve"],
                "delegation_expires_at": null,
                "advertised": true,
                "serving_offers": true,
                "last_failure": null,
            }],
            "discovery": {
                "enabled": true,
                "discoverable": true,
                "advertised_domains": [DOMAIN_ID],
                "advertised_addresses": ["/ip4/203.0.113.1/tcp/4001"],
                "last_refresh_at": GENERATED_AT,
                "expires_at": "2026-05-26T13:00:00Z",
                "degraded": false,
                "last_failure": null,
            },
            "remote_peers": [{
                "peer_id": PEER_ID,
                "learned_from": "configured",
                "dialable": true,
                "connected": true,
                "lifecycle_state": "ready",
                "selected_protocol_version": "2026-05-baseline",
                "authorized": true,
                "verified_wallet_public_key": wallet_public_key(),
                "accepted_served_domains": [DOMAIN_ID],
                "rejected_domains": [{
                    "domain_id": DOMAIN_ID,
                    "code": error::DOMAIN_MISSING_DELEGATION,
                    "message": "missing delegation",
                }],
                "offer_catalog_status": {
                    "path_available": true,
                    "last_fetch_at": GENERATED_AT,
                    "last_success_at": GENERATED_AT,
                    "last_failure": null,
                },
                "loaded_offers": [{
                    "peer_id": PEER_ID,
                    "domain_id": DOMAIN_ID,
                    "offer_id": "camera-main",
                    "kind": "sensor.frame",
                    "status": "available",
                    "access_modes": ["get", "subscribe"],
                    "payload_type": "auki.frame",
                    "registry_refs": [{
                        "registry": "sensor",
                        "role": "primary",
                        "id": "camera-main",
                        "hash": "sha256:47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU",
                    }],
                    "usable": true,
                    "unusable_reason": null,
                    "updated_at": GENERATED_AT,
                    "expires_at": null,
                    "last_failure": null,
                }],
                "last_failure": null,
            }],
            "active_paths": [{
                "path_id": "path-1",
                "path_type": "subscribe",
                "peer_id": PEER_ID,
                "domain_id": DOMAIN_ID,
                "offer_id": "camera-main",
                "state": "active",
                "started_at": GENERATED_AT,
                "last_message_at": GENERATED_AT,
                "payload_type": "auki.frame",
                "last_sequence": "7",
                "sequence_gap_count": 0,
                "last_envelope_failure": null,
                "last_payload_failure": null,
                "last_failure": null,
            }],
            "last_failures": [failure_value()],
            "metadata": {"fixture": true},
        })
    }

    #[test]
    fn create_and_parse_empty_status_snapshot() {
        let local_peer = LocalPeerStatus::from_value(local_peer_value()).unwrap();
        let snapshot = StatusSnapshot::create(
            GENERATED_AT,
            local_peer,
            vec![],
            vec![],
            vec![],
            vec![],
            None,
            Some(json!({"empty": true})),
        )
        .unwrap();
        let parsed = StatusSnapshot::from_value(snapshot.value().clone()).unwrap();

        assert_eq!(parsed.value(), snapshot.value());
        assert_eq!(parsed.generated_at, GENERATED_AT);
        assert_eq!(parsed.local_peer.peer_id.as_deref(), Some(PEER_ID));
        assert!(parsed.local_domains.is_empty());
        assert!(parsed.remote_peers.is_empty());
        assert!(parsed.active_paths.is_empty());
        assert!(parsed.last_failures.is_empty());
        assert_eq!(parsed.metadata, Some(json!({"empty": true})));
    }

    #[test]
    fn parses_full_status_snapshot() {
        let snapshot = StatusSnapshot::from_value(status_snapshot_value()).unwrap();

        assert_eq!(snapshot.generated_at, GENERATED_AT);
        assert_eq!(
            snapshot.local_peer.authorization_mode,
            Some(PeerAuthorizationMode::WhitelistedOnly)
        );
        assert_eq!(snapshot.local_domains[0].role, Some(LocalDomainRole::Owner));
        assert_eq!(
            snapshot.discovery.as_ref().unwrap().advertised_domains,
            vec![DOMAIN_ID]
        );
        assert_eq!(
            snapshot.remote_peers[0].loaded_offers[0].registry_refs[0].hash,
            "sha256:47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU"
        );
        assert_eq!(
            snapshot.active_paths[0].path_type,
            Some(PathType::Subscribe)
        );
        assert_eq!(snapshot.active_paths[0].last_sequence, Some(7));
        assert_eq!(snapshot.last_failures[0].code, error::OFFER_UNKNOWN_OFFER);
    }

    #[test]
    fn rejects_missing_required_snapshot_fields_and_wrong_type() {
        assert_eq!(
            StatusSnapshot::from_value(json!({
                "type": "auki.status.v1",
                "generated_at": GENERATED_AT,
                "local_peer": {},
                "local_domains": [],
                "remote_peers": [],
                "active_paths": [],
                "last_failures": [],
            })),
            Err(StatusError::UnsupportedType {
                actual: "auki.status.v1".to_owned(),
            })
        );

        assert_eq!(
            StatusSnapshot::from_value(json!({
                "type": STATUS_SNAPSHOT_TYPE,
                "generated_at": GENERATED_AT,
                "local_peer": {},
                "remote_peers": [],
                "active_paths": [],
                "last_failures": [],
            })),
            Err(StatusError::MissingField {
                object: OBJECT_STATUS_SNAPSHOT,
                field: FIELD_LOCAL_DOMAINS,
            })
        );
    }

    #[test]
    fn rejects_invalid_nested_status_fields() {
        let mut invalid_domain = status_snapshot_value();
        invalid_domain["local_domains"][0]["domain_id"] = json!("bad");
        assert!(matches!(
            StatusSnapshot::from_value(invalid_domain),
            Err(StatusError::InvalidArrayItem {
                field: FIELD_LOCAL_DOMAINS,
                ..
            })
        ));

        let mut invalid_sequence = status_snapshot_value();
        invalid_sequence["active_paths"][0]["last_sequence"] = json!("01");
        assert!(matches!(
            StatusSnapshot::from_value(invalid_sequence),
            Err(StatusError::InvalidArrayItem {
                field: FIELD_ACTIVE_PATHS,
                ..
            })
        ));

        let mut invalid_path_type = status_snapshot_value();
        invalid_path_type["active_paths"][0]["path_type"] = json!("stream");
        assert!(matches!(
            StatusSnapshot::from_value(invalid_path_type),
            Err(StatusError::InvalidArrayItem {
                field: FIELD_ACTIVE_PATHS,
                ..
            })
        ));
    }

    #[test]
    fn rejects_invalid_failure_record_and_registry_summary() {
        assert_eq!(
            FailureRecord::from_value(json!({
                "at": GENERATED_AT,
                "scope": "peer",
            })),
            Err(StatusError::MissingField {
                object: OBJECT_FAILURE_RECORD,
                field: FIELD_CODE,
            })
        );

        assert!(matches!(
            RegistryReferenceSummary::from_value(json!({
                "registry": "sensor",
                "role": "primary",
                "id": "camera-main",
                "hash": "bad",
            })),
            Err(StatusError::InvalidRegistryHash { .. })
        ));
    }
}
