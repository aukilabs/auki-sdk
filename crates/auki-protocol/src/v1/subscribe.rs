//! Subscribe protocol helpers for v1.

use super::{
    domain::decode_domain_id,
    error,
    message::{ErrorObject, ErrorObjectError, SpatialMessage, SpatialMessageError},
    offer::{PayloadDescriptor, PayloadDescriptorError, RegistryReference, RegistryReferenceError},
};
use serde_json::{Map, Number, Value};
use std::fmt;

/// V1 Subscribe stream protocol id.
pub const SUBSCRIBE_PROTOCOL_ID: &str = "/auki/subscribe/0.0.1";
/// V1 Subscribe request object type.
pub const SUBSCRIBE_REQUEST_TYPE: &str = "auki.subscribe_request.v1";
/// V1 Subscribe accept object type.
pub const SUBSCRIBE_ACCEPT_TYPE: &str = "auki.subscribe_accept.v1";
/// V1 Subscribe reject object type.
pub const SUBSCRIBE_REJECT_TYPE: &str = "auki.subscribe_reject.v1";
/// V1 Subscribe end object type.
pub const SUBSCRIBE_END_TYPE: &str = "auki.subscribe_end.v1";

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

const FIELD_TYPE: &str = "type";
const FIELD_DOMAIN_ID: &str = "domain_id";
const FIELD_OFFER_ID: &str = "offer_id";
const FIELD_PARAMS: &str = "params";
const FIELD_ACCEPTED_PAYLOAD_TYPES: &str = "accepted_payload_types";
const FIELD_MAX_MESSAGE_BYTES: &str = "max_message_bytes";
const FIELD_PAYLOAD: &str = "payload";
const FIELD_REGISTRY_REFS: &str = "registry_refs";
const FIELD_INITIAL_SEQUENCE: &str = "initial_sequence";
const FIELD_GENERATED_AT: &str = "generated_at";
const FIELD_METADATA: &str = "metadata";
const FIELD_ERROR: &str = "error";
const FIELD_REASON: &str = "reason";
const FIELD_RETRYABLE: &str = "retryable";
const FIELD_DETAILS: &str = "details";

/// Parsed v1 Subscribe request.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeRequest {
    value: Value,
    /// Domain id targeted by this request.
    pub domain_id: String,
    /// Offer id targeted by this request.
    pub offer_id: String,
    /// Optional offer-kind-specific params.
    pub params: Option<Value>,
    /// Accepted payload types. Empty means any locally supported selected type.
    pub accepted_payload_types: Vec<String>,
    /// Optional positive serialized spatial-message envelope byte limit.
    pub max_message_bytes: Option<u64>,
}

/// Parsed first Subscribe stream result.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeStartResult {
    value: Value,
    /// Accepted or rejected subscription start result.
    pub body: SubscribeStartResultBody,
}

/// V1 Subscribe start result body.
#[derive(Debug, Clone, PartialEq)]
pub enum SubscribeStartResultBody {
    /// Producer accepted the subscription.
    Accept(SubscribeAccept),
    /// Producer rejected the subscription.
    Reject(SubscribeReject),
}

/// Parsed v1 Subscribe accept object.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeAccept {
    value: Value,
    /// Accepted domain id.
    pub domain_id: String,
    /// Accepted offer id.
    pub offer_id: String,
    /// Selected payload descriptor for all subsequent data messages.
    pub payload: PayloadDescriptor,
    /// Optional registry references needed to interpret the stream.
    pub registry_refs: Vec<RegistryReference>,
    /// Optional first sequence value for the accepted stream.
    pub initial_sequence: Option<u64>,
    /// Optional accept generation timestamp.
    pub generated_at: Option<String>,
    /// Optional non-authoritative metadata.
    pub metadata: Option<Value>,
}

/// Parsed v1 Subscribe reject object.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeReject {
    value: Value,
    /// Structured rejection reason.
    pub error: ErrorObject,
}

/// Parsed v1 Subscribe end object.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscribeEnd {
    value: Value,
    /// Domain id for the ended subscription.
    pub domain_id: String,
    /// Offer id for the ended subscription.
    pub offer_id: String,
    /// Reason the stream ended.
    pub reason: SubscribeEndReason,
    /// Optional structured end error.
    pub error: Option<ErrorObject>,
    /// Optional retry hint.
    pub retryable: Option<bool>,
    /// Optional structured diagnostic details.
    pub details: Option<Value>,
}

/// V1 Subscribe end reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeEndReason {
    /// Stream completed normally.
    Complete,
    /// Stream was cancelled.
    Cancelled,
    /// Offer was withdrawn.
    OfferWithdrawn,
    /// Consumer is not authorized.
    NotAuthorized,
    /// Producer shut down.
    ProducerShutdown,
    /// Stream ended due to an error.
    Error,
}

impl SubscribeEndReason {
    /// Return the wire-format reason string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::OfferWithdrawn => "offer_withdrawn",
            Self::NotAuthorized => "not_authorized",
            Self::ProducerShutdown => "producer_shutdown",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for SubscribeEndReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Errors produced while creating or parsing Subscribe requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeRequestError {
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
    /// `domain_id` was malformed.
    InvalidDomainId {
        /// Actual domain id string.
        domain_id: String,
        /// Error detail.
        error: String,
    },
    /// `max_message_bytes` was zero.
    MaxMessageBytesZero,
    /// `max_message_bytes` exceeded JSON safe integer range.
    MaxMessageBytesTooLarge {
        /// Actual value.
        value: u64,
    },
}

impl SubscribeRequestError {
    /// Stable RFC failure code for this Subscribe request error.
    pub fn failure_code(&self) -> &'static str {
        error::SUBSCRIBE_INVALID_REQUEST
    }
}

impl fmt::Display for SubscribeRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "subscribe request is not a json object"),
            Self::MissingField { field } => {
                write!(f, "subscribe request missing field {field}")
            }
            Self::InvalidFieldType { field, expected } => {
                write!(f, "subscribe request field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported subscribe request type {actual}")
            }
            Self::InvalidDomainId { domain_id, error } => {
                write!(
                    f,
                    "invalid subscribe request domain id {domain_id}: {error}"
                )
            }
            Self::MaxMessageBytesZero => {
                write!(f, "subscribe request max_message_bytes must be positive")
            }
            Self::MaxMessageBytesTooLarge { value } => write!(
                f,
                "subscribe request max_message_bytes exceeds safe integer range: {value}"
            ),
        }
    }
}

impl std::error::Error for SubscribeRequestError {}

/// Errors produced while parsing Subscribe start results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeStartResultError {
    /// Start result JSON value was not an object.
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
    /// Accept object was malformed.
    InvalidAccept(SubscribeAcceptError),
    /// Reject object was malformed.
    InvalidReject(SubscribeRejectError),
}

impl SubscribeStartResultError {
    /// Stable RFC failure code for this Subscribe start-result error.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::InvalidAccept(error) => error.failure_code(),
            Self::InvalidReject(error) => error.failure_code(),
            _ => error::MESSAGE_INVALID_ENVELOPE,
        }
    }
}

impl fmt::Display for SubscribeStartResultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "subscribe start result is not a json object"),
            Self::MissingField { field } => {
                write!(f, "subscribe start result missing field {field}")
            }
            Self::InvalidFieldType { field, expected } => {
                write!(f, "subscribe start result field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported subscribe start result type {actual}")
            }
            Self::InvalidAccept(error) => write!(f, "invalid subscribe accept: {error}"),
            Self::InvalidReject(error) => write!(f, "invalid subscribe reject: {error}"),
        }
    }
}

impl std::error::Error for SubscribeStartResultError {}

/// Errors produced while creating, parsing, or validating Subscribe accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeAcceptError {
    /// Accept JSON value was not an object.
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
    /// `domain_id` was malformed.
    InvalidDomainId {
        /// Actual domain id string.
        domain_id: String,
        /// Error detail.
        error: String,
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
    /// Decimal integer string was malformed.
    InvalidDecimalInteger {
        /// Field name.
        field: &'static str,
        /// Actual value.
        value: String,
    },
    /// `initial_sequence` did not fit in `u64`.
    InitialSequenceOutOfRange {
        /// Actual initial sequence value.
        value: String,
    },
    /// Timestamp was not an RFC3339 UTC string with `Z` suffix.
    InvalidTimestamp {
        /// Field name.
        field: &'static str,
        /// Actual timestamp value.
        value: String,
    },
    /// Accept domain or offer did not match the request.
    RequestMismatch {
        /// Expected domain id.
        expected_domain_id: String,
        /// Actual domain id.
        actual_domain_id: String,
        /// Expected offer id.
        expected_offer_id: String,
        /// Actual offer id.
        actual_offer_id: String,
    },
    /// Accepted payload type was not requested.
    PayloadTypeNotAccepted {
        /// Actual selected payload type.
        payload_type: String,
    },
}

impl SubscribeAcceptError {
    /// Stable RFC failure code for this Subscribe accept error.
    pub fn failure_code(&self) -> &'static str {
        error::MESSAGE_INVALID_ENVELOPE
    }
}

impl fmt::Display for SubscribeAcceptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "subscribe accept is not a json object"),
            Self::MissingField { field } => {
                write!(f, "subscribe accept missing field {field}")
            }
            Self::InvalidFieldType { field, expected } => {
                write!(f, "subscribe accept field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported subscribe accept type {actual}")
            }
            Self::InvalidDomainId { domain_id, error } => {
                write!(f, "invalid subscribe accept domain id {domain_id}: {error}")
            }
            Self::InvalidPayload(error) => {
                write!(f, "invalid subscribe accept payload descriptor: {error}")
            }
            Self::InvalidRegistryReference { index, error } => write!(
                f,
                "invalid subscribe accept registry reference at index {index}: {error}"
            ),
            Self::InvalidDecimalInteger { field, value } => {
                write!(f, "invalid decimal integer in field {field}: {value}")
            }
            Self::InitialSequenceOutOfRange { value } => {
                write!(
                    f,
                    "subscribe accept initial_sequence is out of range: {value}"
                )
            }
            Self::InvalidTimestamp { field, value } => {
                write!(
                    f,
                    "invalid timestamp in subscribe accept field {field}: {value}"
                )
            }
            Self::RequestMismatch {
                expected_domain_id,
                actual_domain_id,
                expected_offer_id,
                actual_offer_id,
            } => write!(
                f,
                "subscribe accept request mismatch: expected {expected_domain_id}/{expected_offer_id}, got {actual_domain_id}/{actual_offer_id}"
            ),
            Self::PayloadTypeNotAccepted { payload_type } => {
                write!(
                    f,
                    "subscribe accept payload type was not requested: {payload_type}"
                )
            }
        }
    }
}

impl std::error::Error for SubscribeAcceptError {}

/// Errors produced while creating or parsing Subscribe rejects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeRejectError {
    /// Reject JSON value was not an object.
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
    /// Error object was malformed.
    InvalidError(ErrorObjectError),
}

impl SubscribeRejectError {
    /// Stable RFC failure code for this Subscribe reject error.
    pub fn failure_code(&self) -> &'static str {
        error::MESSAGE_INVALID_ENVELOPE
    }
}

impl fmt::Display for SubscribeRejectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "subscribe reject is not a json object"),
            Self::MissingField { field } => {
                write!(f, "subscribe reject missing field {field}")
            }
            Self::InvalidFieldType { field, expected } => {
                write!(f, "subscribe reject field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported subscribe reject type {actual}")
            }
            Self::InvalidError(error) => write!(f, "invalid subscribe reject error: {error}"),
        }
    }
}

impl std::error::Error for SubscribeRejectError {}

/// Errors produced while creating, parsing, or validating Subscribe end messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeEndError {
    /// End JSON value was not an object.
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
    /// `domain_id` was malformed.
    InvalidDomainId {
        /// Actual domain id string.
        domain_id: String,
        /// Error detail.
        error: String,
    },
    /// `reason` was unsupported.
    UnsupportedReason {
        /// Actual reason string.
        actual: String,
    },
    /// Error object was malformed.
    InvalidError(ErrorObjectError),
    /// End domain or offer did not match the subscription path.
    PathMismatch {
        /// Expected domain id.
        expected_domain_id: String,
        /// Actual domain id.
        actual_domain_id: String,
        /// Expected offer id.
        expected_offer_id: String,
        /// Actual offer id.
        actual_offer_id: String,
    },
}

impl SubscribeEndError {
    /// Stable RFC failure code for this Subscribe end error.
    pub fn failure_code(&self) -> &'static str {
        error::MESSAGE_INVALID_ENVELOPE
    }
}

impl fmt::Display for SubscribeEndError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "subscribe end is not a json object"),
            Self::MissingField { field } => write!(f, "subscribe end missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "subscribe end field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported subscribe end type {actual}")
            }
            Self::InvalidDomainId { domain_id, error } => {
                write!(f, "invalid subscribe end domain id {domain_id}: {error}")
            }
            Self::UnsupportedReason { actual } => {
                write!(f, "unsupported subscribe end reason {actual}")
            }
            Self::InvalidError(error) => write!(f, "invalid subscribe end error: {error}"),
            Self::PathMismatch {
                expected_domain_id,
                actual_domain_id,
                expected_offer_id,
                actual_offer_id,
            } => write!(
                f,
                "subscribe end path mismatch: expected {expected_domain_id}/{expected_offer_id}, got {actual_domain_id}/{actual_offer_id}"
            ),
        }
    }
}

impl std::error::Error for SubscribeEndError {}

/// Errors produced while validating Subscribe data messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeDataError {
    /// Spatial message envelope was malformed or mismatched.
    InvalidMessage(SpatialMessageError),
    /// Spatial-message JSON body bytes exceeded a request limit.
    MessageTooLarge {
        /// Actual spatial-message JSON body byte count.
        actual: usize,
        /// Maximum allowed spatial-message JSON body byte count.
        max: u64,
    },
}

impl SubscribeDataError {
    /// Stable RFC failure code for this Subscribe data-message error.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::InvalidMessage(error) => error.failure_code(),
            Self::MessageTooLarge { .. } => error::MESSAGE_PAYLOAD_TOO_LARGE,
        }
    }
}

impl fmt::Display for SubscribeDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMessage(error) => write!(f, "invalid subscribe data message: {error}"),
            Self::MessageTooLarge { actual, max } => {
                write!(f, "subscribe data message too large: {actual} > {max}")
            }
        }
    }
}

impl std::error::Error for SubscribeDataError {}

impl SubscribeRequest {
    /// Create a v1 Subscribe request.
    pub fn create(
        domain_id: impl Into<String>,
        offer_id: impl Into<String>,
        params: Option<Value>,
        accepted_payload_types: Vec<String>,
        max_message_bytes: Option<u64>,
    ) -> Result<Self, SubscribeRequestError> {
        let domain_id = domain_id.into();
        let offer_id = offer_id.into();
        validate_request_domain_id(&domain_id)?;
        if let Some(params) = &params
            && !params.is_object()
        {
            return Err(SubscribeRequestError::InvalidFieldType {
                field: FIELD_PARAMS,
                expected: "an object",
            });
        }
        validate_max_message_bytes(max_message_bytes)?;

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(SUBSCRIBE_REQUEST_TYPE.to_owned()),
        );
        object.insert(FIELD_DOMAIN_ID.to_owned(), Value::String(domain_id.clone()));
        object.insert(FIELD_OFFER_ID.to_owned(), Value::String(offer_id.clone()));
        if let Some(params) = &params {
            object.insert(FIELD_PARAMS.to_owned(), params.clone());
        }
        if !accepted_payload_types.is_empty() {
            object.insert(
                FIELD_ACCEPTED_PAYLOAD_TYPES.to_owned(),
                Value::Array(
                    accepted_payload_types
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if let Some(max_message_bytes) = max_message_bytes {
            object.insert(
                FIELD_MAX_MESSAGE_BYTES.to_owned(),
                Value::Number(Number::from(max_message_bytes)),
            );
        }

        Ok(Self {
            value: Value::Object(object),
            domain_id,
            offer_id,
            params,
            accepted_payload_types,
            max_message_bytes,
        })
    }

    /// Parse a v1 Subscribe request from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, SubscribeRequestError> {
        let object = value.as_object().ok_or(SubscribeRequestError::NotObject)?;
        let type_value = required_request_string(object, FIELD_TYPE)?;
        if type_value != SUBSCRIBE_REQUEST_TYPE {
            return Err(SubscribeRequestError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let domain_id = required_request_string(object, FIELD_DOMAIN_ID)?.to_owned();
        validate_request_domain_id(&domain_id)?;
        let offer_id = required_request_string(object, FIELD_OFFER_ID)?.to_owned();
        let params = optional_request_object(object, FIELD_PARAMS)?;
        let accepted_payload_types =
            optional_request_string_array(object, FIELD_ACCEPTED_PAYLOAD_TYPES)?;
        let max_message_bytes = optional_max_message_bytes(object)?;

        Ok(Self {
            value,
            domain_id,
            offer_id,
            params,
            accepted_payload_types,
            max_message_bytes,
        })
    }

    /// Borrow the original Subscribe request JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this request and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Return whether this request accepts a selected payload type.
    pub fn accepts_payload_type(&self, payload_type: &str) -> bool {
        self.accepted_payload_types.is_empty()
            || self
                .accepted_payload_types
                .iter()
                .any(|accepted| accepted == payload_type)
    }
}

impl SubscribeStartResult {
    /// Create an accepted v1 Subscribe start result.
    pub fn accept(accept: SubscribeAccept) -> Self {
        Self {
            value: accept.value().clone(),
            body: SubscribeStartResultBody::Accept(accept),
        }
    }

    /// Create a rejected v1 Subscribe start result.
    pub fn reject(reject: SubscribeReject) -> Self {
        Self {
            value: reject.value().clone(),
            body: SubscribeStartResultBody::Reject(reject),
        }
    }

    /// Parse a v1 Subscribe start result from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, SubscribeStartResultError> {
        let object = value
            .as_object()
            .ok_or(SubscribeStartResultError::NotObject)?;
        let type_value = required_start_string(object, FIELD_TYPE)?;

        match type_value {
            SUBSCRIBE_ACCEPT_TYPE => {
                let accept = SubscribeAccept::from_value(value)
                    .map_err(SubscribeStartResultError::InvalidAccept)?;
                Ok(Self::accept(accept))
            }
            SUBSCRIBE_REJECT_TYPE => {
                let reject = SubscribeReject::from_value(value)
                    .map_err(SubscribeStartResultError::InvalidReject)?;
                Ok(Self::reject(reject))
            }
            actual => Err(SubscribeStartResultError::UnsupportedType {
                actual: actual.to_owned(),
            }),
        }
    }

    /// Borrow the original Subscribe start result JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this result and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Return the accepted subscription, if this is an accept result.
    pub fn accept_body(&self) -> Option<&SubscribeAccept> {
        match &self.body {
            SubscribeStartResultBody::Accept(accept) => Some(accept),
            SubscribeStartResultBody::Reject(_) => None,
        }
    }

    /// Return the rejection, if this is a reject result.
    pub fn reject_body(&self) -> Option<&SubscribeReject> {
        match &self.body {
            SubscribeStartResultBody::Accept(_) => None,
            SubscribeStartResultBody::Reject(reject) => Some(reject),
        }
    }
}

impl SubscribeAccept {
    /// Create a v1 Subscribe accept.
    pub fn create(
        domain_id: impl Into<String>,
        offer_id: impl Into<String>,
        payload: PayloadDescriptor,
        registry_refs: Vec<RegistryReference>,
        initial_sequence: Option<u64>,
        generated_at: Option<String>,
        metadata: Option<Value>,
    ) -> Result<Self, SubscribeAcceptError> {
        let domain_id = domain_id.into();
        let offer_id = offer_id.into();
        validate_accept_domain_id(&domain_id)?;
        if let Some(generated_at) = &generated_at {
            validate_accept_timestamp(FIELD_GENERATED_AT, generated_at)?;
        }
        if let Some(metadata) = &metadata
            && !metadata.is_object()
        {
            return Err(SubscribeAcceptError::InvalidFieldType {
                field: FIELD_METADATA,
                expected: "an object",
            });
        }

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(SUBSCRIBE_ACCEPT_TYPE.to_owned()),
        );
        object.insert(FIELD_DOMAIN_ID.to_owned(), Value::String(domain_id.clone()));
        object.insert(FIELD_OFFER_ID.to_owned(), Value::String(offer_id.clone()));
        object.insert(FIELD_PAYLOAD.to_owned(), payload.value().clone());
        if !registry_refs.is_empty() {
            object.insert(
                FIELD_REGISTRY_REFS.to_owned(),
                Value::Array(
                    registry_refs
                        .iter()
                        .map(|reference| reference.value().clone())
                        .collect(),
                ),
            );
        }
        if let Some(initial_sequence) = initial_sequence {
            object.insert(
                FIELD_INITIAL_SEQUENCE.to_owned(),
                Value::String(initial_sequence.to_string()),
            );
        }
        if let Some(generated_at) = &generated_at {
            object.insert(
                FIELD_GENERATED_AT.to_owned(),
                Value::String(generated_at.clone()),
            );
        }
        if let Some(metadata) = &metadata {
            object.insert(FIELD_METADATA.to_owned(), metadata.clone());
        }

        Ok(Self {
            value: Value::Object(object),
            domain_id,
            offer_id,
            payload,
            registry_refs,
            initial_sequence,
            generated_at,
            metadata,
        })
    }

    /// Parse a v1 Subscribe accept from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, SubscribeAcceptError> {
        let object = value.as_object().ok_or(SubscribeAcceptError::NotObject)?;
        let type_value = required_accept_string(object, FIELD_TYPE)?;
        if type_value != SUBSCRIBE_ACCEPT_TYPE {
            return Err(SubscribeAcceptError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let domain_id = required_accept_string(object, FIELD_DOMAIN_ID)?.to_owned();
        validate_accept_domain_id(&domain_id)?;
        let offer_id = required_accept_string(object, FIELD_OFFER_ID)?.to_owned();
        let payload = object
            .get(FIELD_PAYLOAD)
            .ok_or(SubscribeAcceptError::MissingField {
                field: FIELD_PAYLOAD,
            })
            .and_then(|value| {
                PayloadDescriptor::from_value(value.clone())
                    .map_err(SubscribeAcceptError::InvalidPayload)
            })?;
        let registry_refs = parse_accept_registry_refs(object)?;
        let initial_sequence = optional_initial_sequence(object)?;
        let generated_at = optional_accept_timestamp(object, FIELD_GENERATED_AT)?;
        let metadata = optional_accept_object(object, FIELD_METADATA)?;

        Ok(Self {
            value,
            domain_id,
            offer_id,
            payload,
            registry_refs,
            initial_sequence,
            generated_at,
            metadata,
        })
    }

    /// Borrow the original Subscribe accept JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this accept and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Validate this accept against the original Subscribe request.
    pub fn validate_for_request(
        &self,
        request: &SubscribeRequest,
    ) -> Result<(), SubscribeAcceptError> {
        if self.domain_id != request.domain_id || self.offer_id != request.offer_id {
            return Err(SubscribeAcceptError::RequestMismatch {
                expected_domain_id: request.domain_id.clone(),
                actual_domain_id: self.domain_id.clone(),
                expected_offer_id: request.offer_id.clone(),
                actual_offer_id: self.offer_id.clone(),
            });
        }
        if !request.accepts_payload_type(&self.payload.payload_type) {
            return Err(SubscribeAcceptError::PayloadTypeNotAccepted {
                payload_type: self.payload.payload_type.clone(),
            });
        }
        Ok(())
    }

    /// Validate a subsequent Subscribe data message against this accepted stream.
    ///
    /// This helper reserializes the parsed message to measure its compact JSON
    /// size. Runtime stream receivers should prefer
    /// [`Self::validate_data_message_with_body_len`] so they can enforce the
    /// exact received frame body byte length.
    pub fn validate_data_message(
        &self,
        message: &SpatialMessage,
        max_message_bytes: Option<u64>,
    ) -> Result<(), SubscribeDataError> {
        self.validate_data_message_with_body_len(
            message,
            serialized_message_len(message),
            max_message_bytes,
        )
    }

    /// Validate a Subscribe data message using the actual received frame body length.
    pub fn validate_data_message_with_body_len(
        &self,
        message: &SpatialMessage,
        actual_body_len: usize,
        max_message_bytes: Option<u64>,
    ) -> Result<(), SubscribeDataError> {
        message
            .validate_for_offer(&self.domain_id, &self.offer_id, &self.payload.payload_type)
            .map_err(SubscribeDataError::InvalidMessage)?;

        if let Some(max) = max_message_bytes {
            if actual_body_len as u64 > max {
                return Err(SubscribeDataError::MessageTooLarge {
                    actual: actual_body_len,
                    max,
                });
            }
        }

        Ok(())
    }
}

impl SubscribeReject {
    /// Create a v1 Subscribe reject.
    pub fn create(error: ErrorObject) -> Self {
        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(SUBSCRIBE_REJECT_TYPE.to_owned()),
        );
        object.insert(FIELD_ERROR.to_owned(), error.value().clone());
        Self {
            value: Value::Object(object),
            error,
        }
    }

    /// Parse a v1 Subscribe reject from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, SubscribeRejectError> {
        let object = value.as_object().ok_or(SubscribeRejectError::NotObject)?;
        let type_value = required_reject_string(object, FIELD_TYPE)?;
        if type_value != SUBSCRIBE_REJECT_TYPE {
            return Err(SubscribeRejectError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let error = object
            .get(FIELD_ERROR)
            .ok_or(SubscribeRejectError::MissingField { field: FIELD_ERROR })
            .and_then(|value| {
                ErrorObject::from_value(value.clone()).map_err(SubscribeRejectError::InvalidError)
            })?;

        Ok(Self { value, error })
    }

    /// Borrow the original Subscribe reject JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this reject and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

impl SubscribeEnd {
    /// Create a v1 Subscribe end message.
    pub fn create(
        domain_id: impl Into<String>,
        offer_id: impl Into<String>,
        reason: SubscribeEndReason,
        error: Option<ErrorObject>,
        retryable: Option<bool>,
        details: Option<Value>,
    ) -> Result<Self, SubscribeEndError> {
        let domain_id = domain_id.into();
        let offer_id = offer_id.into();
        validate_end_domain_id(&domain_id)?;
        if let Some(details) = &details
            && !details.is_object()
        {
            return Err(SubscribeEndError::InvalidFieldType {
                field: FIELD_DETAILS,
                expected: "an object",
            });
        }

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(SUBSCRIBE_END_TYPE.to_owned()),
        );
        object.insert(FIELD_DOMAIN_ID.to_owned(), Value::String(domain_id.clone()));
        object.insert(FIELD_OFFER_ID.to_owned(), Value::String(offer_id.clone()));
        object.insert(
            FIELD_REASON.to_owned(),
            Value::String(reason.as_str().to_owned()),
        );
        if let Some(error) = &error {
            object.insert(FIELD_ERROR.to_owned(), error.value().clone());
        }
        if let Some(retryable) = retryable {
            object.insert(FIELD_RETRYABLE.to_owned(), Value::Bool(retryable));
        }
        if let Some(details) = &details {
            object.insert(FIELD_DETAILS.to_owned(), details.clone());
        }

        Ok(Self {
            value: Value::Object(object),
            domain_id,
            offer_id,
            reason,
            error,
            retryable,
            details,
        })
    }

    /// Parse a v1 Subscribe end message from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, SubscribeEndError> {
        let object = value.as_object().ok_or(SubscribeEndError::NotObject)?;
        let type_value = required_end_string(object, FIELD_TYPE)?;
        if type_value != SUBSCRIBE_END_TYPE {
            return Err(SubscribeEndError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let domain_id = required_end_string(object, FIELD_DOMAIN_ID)?.to_owned();
        validate_end_domain_id(&domain_id)?;
        let offer_id = required_end_string(object, FIELD_OFFER_ID)?.to_owned();
        let reason = parse_end_reason(required_end_string(object, FIELD_REASON)?)?;
        let error = object
            .get(FIELD_ERROR)
            .map(|value| {
                ErrorObject::from_value(value.clone()).map_err(SubscribeEndError::InvalidError)
            })
            .transpose()?;
        let retryable = optional_end_bool(object, FIELD_RETRYABLE)?;
        let details = optional_end_object(object, FIELD_DETAILS)?;

        Ok(Self {
            value,
            domain_id,
            offer_id,
            reason,
            error,
            retryable,
            details,
        })
    }

    /// Borrow the original Subscribe end JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this end message and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Validate this end message against the accepted subscription path.
    pub fn validate_for_offer(
        &self,
        domain_id: &str,
        offer_id: &str,
    ) -> Result<(), SubscribeEndError> {
        if self.domain_id != domain_id || self.offer_id != offer_id {
            return Err(SubscribeEndError::PathMismatch {
                expected_domain_id: domain_id.to_owned(),
                actual_domain_id: self.domain_id.clone(),
                expected_offer_id: offer_id.to_owned(),
                actual_offer_id: self.offer_id.clone(),
            });
        }
        Ok(())
    }
}

fn required_request_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, SubscribeRequestError> {
    object
        .get(field)
        .ok_or(SubscribeRequestError::MissingField { field })?
        .as_str()
        .ok_or(SubscribeRequestError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn validate_request_domain_id(domain_id: &str) -> Result<(), SubscribeRequestError> {
    decode_domain_id(domain_id).map_err(|error| SubscribeRequestError::InvalidDomainId {
        domain_id: domain_id.to_owned(),
        error: error.to_string(),
    })?;
    Ok(())
}

fn optional_request_object(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Value>, SubscribeRequestError> {
    object
        .get(field)
        .map(|value| {
            if value.is_object() {
                Ok(value.clone())
            } else {
                Err(SubscribeRequestError::InvalidFieldType {
                    field,
                    expected: "an object",
                })
            }
        })
        .transpose()
}

fn optional_request_string_array(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Vec<String>, SubscribeRequestError> {
    object
        .get(field)
        .map(|value| {
            let values = value
                .as_array()
                .ok_or(SubscribeRequestError::InvalidFieldType {
                    field,
                    expected: "an array",
                })?;
            values
                .iter()
                .map(|value| {
                    value.as_str().map(ToOwned::to_owned).ok_or(
                        SubscribeRequestError::InvalidFieldType {
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

fn optional_max_message_bytes(
    object: &Map<String, Value>,
) -> Result<Option<u64>, SubscribeRequestError> {
    object
        .get(FIELD_MAX_MESSAGE_BYTES)
        .map(|value| {
            value
                .as_u64()
                .ok_or(SubscribeRequestError::InvalidFieldType {
                    field: FIELD_MAX_MESSAGE_BYTES,
                    expected: "a positive safe integer",
                })
                .and_then(|max| {
                    validate_max_message_bytes(Some(max))?;
                    Ok(max)
                })
        })
        .transpose()
}

fn validate_max_message_bytes(max_message_bytes: Option<u64>) -> Result<(), SubscribeRequestError> {
    let Some(max_message_bytes) = max_message_bytes else {
        return Ok(());
    };
    if max_message_bytes == 0 {
        Err(SubscribeRequestError::MaxMessageBytesZero)
    } else if max_message_bytes > MAX_SAFE_INTEGER {
        Err(SubscribeRequestError::MaxMessageBytesTooLarge {
            value: max_message_bytes,
        })
    } else {
        Ok(())
    }
}

fn required_start_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, SubscribeStartResultError> {
    object
        .get(field)
        .ok_or(SubscribeStartResultError::MissingField { field })?
        .as_str()
        .ok_or(SubscribeStartResultError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn required_accept_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, SubscribeAcceptError> {
    object
        .get(field)
        .ok_or(SubscribeAcceptError::MissingField { field })?
        .as_str()
        .ok_or(SubscribeAcceptError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn validate_accept_domain_id(domain_id: &str) -> Result<(), SubscribeAcceptError> {
    decode_domain_id(domain_id).map_err(|error| SubscribeAcceptError::InvalidDomainId {
        domain_id: domain_id.to_owned(),
        error: error.to_string(),
    })?;
    Ok(())
}

fn parse_accept_registry_refs(
    object: &Map<String, Value>,
) -> Result<Vec<RegistryReference>, SubscribeAcceptError> {
    object
        .get(FIELD_REGISTRY_REFS)
        .map(|value| {
            let values = value
                .as_array()
                .ok_or(SubscribeAcceptError::InvalidFieldType {
                    field: FIELD_REGISTRY_REFS,
                    expected: "an array",
                })?;
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    RegistryReference::from_value(value.clone()).map_err(|error| {
                        SubscribeAcceptError::InvalidRegistryReference { index, error }
                    })
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn optional_initial_sequence(
    object: &Map<String, Value>,
) -> Result<Option<u64>, SubscribeAcceptError> {
    let Some(sequence) = optional_accept_decimal_string(object, FIELD_INITIAL_SEQUENCE)? else {
        return Ok(None);
    };
    sequence
        .parse::<u64>()
        .map(Some)
        .map_err(|_| SubscribeAcceptError::InitialSequenceOutOfRange { value: sequence })
}

fn optional_accept_decimal_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, SubscribeAcceptError> {
    object
        .get(field)
        .map(|value| {
            let decimal = value
                .as_str()
                .ok_or(SubscribeAcceptError::InvalidFieldType {
                    field,
                    expected: "a string",
                })?;
            validate_accept_decimal_integer(field, decimal)?;
            Ok(decimal.to_owned())
        })
        .transpose()
}

fn validate_accept_decimal_integer(
    field: &'static str,
    value: &str,
) -> Result<(), SubscribeAcceptError> {
    let valid = value == "0"
        || value
            .strip_prefix(|byte: char| ('1'..='9').contains(&byte))
            .is_some_and(|rest| rest.bytes().all(|byte| byte.is_ascii_digit()));
    if valid {
        Ok(())
    } else {
        Err(SubscribeAcceptError::InvalidDecimalInteger {
            field,
            value: value.to_owned(),
        })
    }
}

fn optional_accept_timestamp(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, SubscribeAcceptError> {
    object
        .get(field)
        .map(|value| {
            let timestamp = value
                .as_str()
                .ok_or(SubscribeAcceptError::InvalidFieldType {
                    field,
                    expected: "a string",
                })?;
            validate_accept_timestamp(field, timestamp)?;
            Ok(timestamp.to_owned())
        })
        .transpose()
}

fn validate_accept_timestamp(field: &'static str, value: &str) -> Result<(), SubscribeAcceptError> {
    if is_rfc3339_z_timestamp(value) {
        Ok(())
    } else {
        Err(SubscribeAcceptError::InvalidTimestamp {
            field,
            value: value.to_owned(),
        })
    }
}

fn optional_accept_object(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Value>, SubscribeAcceptError> {
    object
        .get(field)
        .map(|value| {
            if value.is_object() {
                Ok(value.clone())
            } else {
                Err(SubscribeAcceptError::InvalidFieldType {
                    field,
                    expected: "an object",
                })
            }
        })
        .transpose()
}

fn required_reject_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, SubscribeRejectError> {
    object
        .get(field)
        .ok_or(SubscribeRejectError::MissingField { field })?
        .as_str()
        .ok_or(SubscribeRejectError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn required_end_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, SubscribeEndError> {
    object
        .get(field)
        .ok_or(SubscribeEndError::MissingField { field })?
        .as_str()
        .ok_or(SubscribeEndError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn validate_end_domain_id(domain_id: &str) -> Result<(), SubscribeEndError> {
    decode_domain_id(domain_id).map_err(|error| SubscribeEndError::InvalidDomainId {
        domain_id: domain_id.to_owned(),
        error: error.to_string(),
    })?;
    Ok(())
}

fn parse_end_reason(value: &str) -> Result<SubscribeEndReason, SubscribeEndError> {
    match value {
        "complete" => Ok(SubscribeEndReason::Complete),
        "cancelled" => Ok(SubscribeEndReason::Cancelled),
        "offer_withdrawn" => Ok(SubscribeEndReason::OfferWithdrawn),
        "not_authorized" => Ok(SubscribeEndReason::NotAuthorized),
        "producer_shutdown" => Ok(SubscribeEndReason::ProducerShutdown),
        "error" => Ok(SubscribeEndReason::Error),
        actual => Err(SubscribeEndError::UnsupportedReason {
            actual: actual.to_owned(),
        }),
    }
}

fn optional_end_bool(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<bool>, SubscribeEndError> {
    object
        .get(field)
        .map(|value| {
            value.as_bool().ok_or(SubscribeEndError::InvalidFieldType {
                field,
                expected: "a boolean",
            })
        })
        .transpose()
}

fn optional_end_object(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Value>, SubscribeEndError> {
    object
        .get(field)
        .map(|value| {
            if value.is_object() {
                Ok(value.clone())
            } else {
                Err(SubscribeEndError::InvalidFieldType {
                    field,
                    expected: "an object",
                })
            }
        })
        .transpose()
}

fn serialized_message_len(message: &SpatialMessage) -> usize {
    serde_json::to_vec(message.value())
        .expect("serde_json::Value serializes")
        .len()
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
    use crate::v1::{
        message::SPATIAL_MESSAGE_TYPE,
        offer::{PayloadDescriptor, RegistryReference},
    };
    use serde_json::json;

    const DOMAIN_ID: &str = "noEv5Zu7UvR7qx9ooyAHd407PWcp8nUQLNRxrnd1ZRs";

    fn message_value() -> Value {
        json!({
            "type": SPATIAL_MESSAGE_TYPE,
            "domain_id": DOMAIN_ID,
            "offer_id": "camera-main",
            "payload": {
                "type": "auki.frame",
                "bytes": "AQID",
            },
            "sequence": "1",
        })
    }

    fn registry_reference() -> RegistryReference {
        RegistryReference::create(
            "sensor",
            "primary",
            "camera-main",
            "sha256:RBNvo1WzZ4oRRq0W9-hknpT7T8If536DEMBg9hyq_4o",
            Some("{}".to_owned()),
        )
        .unwrap()
    }

    #[test]
    fn create_and_parse_subscribe_request() {
        let request = SubscribeRequest::create(
            DOMAIN_ID,
            "camera-main",
            Some(json!({"fps": 30})),
            vec!["auki.frame".to_owned()],
            Some(4096),
        )
        .unwrap();
        let parsed = SubscribeRequest::from_value(request.value().clone()).unwrap();

        assert_eq!(parsed.value(), request.value());
        assert_eq!(parsed.domain_id, DOMAIN_ID);
        assert_eq!(parsed.offer_id, "camera-main");
        assert_eq!(parsed.params, Some(json!({"fps": 30})));
        assert_eq!(parsed.accepted_payload_types, vec!["auki.frame"]);
        assert_eq!(parsed.max_message_bytes, Some(4096));
        assert!(parsed.accepts_payload_type("auki.frame"));
        assert!(!parsed.accepts_payload_type("other"));
    }

    #[test]
    fn subscribe_request_defaults_optional_fields() {
        let request = SubscribeRequest::from_value(json!({
            "type": SUBSCRIBE_REQUEST_TYPE,
            "domain_id": DOMAIN_ID,
            "offer_id": "camera-main",
        }))
        .unwrap();

        assert_eq!(request.params, None);
        assert!(request.accepted_payload_types.is_empty());
        assert!(request.accepts_payload_type("any.payload"));
        assert_eq!(request.max_message_bytes, None);
    }

    #[test]
    fn subscribe_request_rejects_invalid_domain_and_size() {
        assert!(matches!(
            SubscribeRequest::from_value(json!({
                "type": SUBSCRIBE_REQUEST_TYPE,
                "domain_id": "bad",
                "offer_id": "camera-main",
            })),
            Err(SubscribeRequestError::InvalidDomainId { .. })
        ));

        let error = SubscribeRequest::from_value(json!({
            "type": SUBSCRIBE_REQUEST_TYPE,
            "domain_id": DOMAIN_ID,
            "offer_id": "camera-main",
            "max_message_bytes": 0,
        }))
        .unwrap_err();
        assert_eq!(error, SubscribeRequestError::MaxMessageBytesZero);
        assert_eq!(error.failure_code(), error::SUBSCRIBE_INVALID_REQUEST);
    }

    #[test]
    fn subscribe_accept_parses_and_validates_request() {
        let request = SubscribeRequest::create(
            DOMAIN_ID,
            "camera-main",
            None,
            vec!["auki.frame".to_owned()],
            Some(4096),
        )
        .unwrap();
        let accept = SubscribeAccept::create(
            DOMAIN_ID,
            "camera-main",
            PayloadDescriptor::create("auki.frame"),
            vec![registry_reference()],
            Some(7),
            Some("2026-05-26T12:00:00Z".to_owned()),
            Some(json!({"accepted": true})),
        )
        .unwrap();
        let parsed = SubscribeAccept::from_value(accept.value().clone()).unwrap();

        assert_eq!(parsed.value(), accept.value());
        assert_eq!(parsed.domain_id, DOMAIN_ID);
        assert_eq!(parsed.offer_id, "camera-main");
        assert_eq!(parsed.payload.payload_type, "auki.frame");
        assert_eq!(parsed.registry_refs.len(), 1);
        assert_eq!(parsed.initial_sequence, Some(7));
        assert_eq!(parsed.generated_at, Some("2026-05-26T12:00:00Z".to_owned()));
        parsed.validate_for_request(&request).unwrap();
    }

    #[test]
    fn subscribe_accept_rejects_request_mismatch_payload_and_sequence() {
        let request = SubscribeRequest::create(
            DOMAIN_ID,
            "camera-main",
            None,
            vec!["auki.frame".to_owned()],
            None,
        )
        .unwrap();
        let other_offer = SubscribeAccept::create(
            DOMAIN_ID,
            "other",
            PayloadDescriptor::create("auki.frame"),
            vec![],
            None,
            None,
            None,
        )
        .unwrap();
        assert!(matches!(
            other_offer.validate_for_request(&request),
            Err(SubscribeAcceptError::RequestMismatch { .. })
        ));

        let other_payload = SubscribeAccept::create(
            DOMAIN_ID,
            "camera-main",
            PayloadDescriptor::create("other.payload"),
            vec![],
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            other_payload.validate_for_request(&request),
            Err(SubscribeAcceptError::PayloadTypeNotAccepted {
                payload_type: "other.payload".to_owned(),
            })
        );

        assert!(matches!(
            SubscribeAccept::from_value(json!({
                "type": SUBSCRIBE_ACCEPT_TYPE,
                "domain_id": DOMAIN_ID,
                "offer_id": "camera-main",
                "payload": {"type": "auki.frame"},
                "initial_sequence": "01",
            })),
            Err(SubscribeAcceptError::InvalidDecimalInteger {
                field: FIELD_INITIAL_SEQUENCE,
                ..
            })
        ));
    }

    #[test]
    fn subscribe_start_result_parses_accept_and_reject() {
        let accept = SubscribeAccept::create(
            DOMAIN_ID,
            "camera-main",
            PayloadDescriptor::create("auki.frame"),
            vec![],
            None,
            None,
            None,
        )
        .unwrap();
        let start = SubscribeStartResult::from_value(accept.value().clone()).unwrap();
        assert!(matches!(start.body, SubscribeStartResultBody::Accept(_)));

        let reject = SubscribeReject::create(ErrorObject::create(error::OFFER_UNKNOWN_OFFER));
        let start = SubscribeStartResult::from_value(reject.value().clone()).unwrap();
        let SubscribeStartResultBody::Reject(reject) = start.body else {
            panic!("expected reject");
        };
        assert_eq!(reject.error.code, error::OFFER_UNKNOWN_OFFER);
    }

    #[test]
    fn subscribe_data_validation_uses_envelope_size_and_payload_type() {
        let message = SpatialMessage::from_value(message_value()).unwrap();
        let accept = SubscribeAccept::create(
            DOMAIN_ID,
            "camera-main",
            PayloadDescriptor::create("auki.frame"),
            vec![],
            None,
            None,
            None,
        )
        .unwrap();

        accept.validate_data_message(&message, None).unwrap();

        let payload_mismatch = SubscribeAccept::create(
            DOMAIN_ID,
            "camera-main",
            PayloadDescriptor::create("other.payload"),
            vec![],
            None,
            None,
            None,
        )
        .unwrap()
        .validate_data_message(&message, None)
        .unwrap_err();
        assert_eq!(
            payload_mismatch.failure_code(),
            error::MESSAGE_INVALID_PAYLOAD
        );

        let too_large = accept.validate_data_message(&message, Some(1)).unwrap_err();
        assert_eq!(too_large.failure_code(), error::MESSAGE_PAYLOAD_TOO_LARGE);

        let raw_body_too_large = accept
            .validate_data_message_with_body_len(&message, 512, Some(128))
            .unwrap_err();
        assert_eq!(
            raw_body_too_large,
            SubscribeDataError::MessageTooLarge {
                actual: 512,
                max: 128,
            }
        );
        assert_eq!(
            raw_body_too_large.failure_code(),
            error::MESSAGE_PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn subscribe_end_parses_and_validates_path() {
        let end = SubscribeEnd::create(
            DOMAIN_ID,
            "camera-main",
            SubscribeEndReason::OfferWithdrawn,
            Some(ErrorObject::create(error::OFFER_TEMPORARILY_UNAVAILABLE)),
            Some(true),
            Some(json!({"retry_after_ms": 1000})),
        )
        .unwrap();
        let parsed = SubscribeEnd::from_value(end.value().clone()).unwrap();

        assert_eq!(parsed.value(), end.value());
        assert_eq!(parsed.reason, SubscribeEndReason::OfferWithdrawn);
        assert_eq!(parsed.retryable, Some(true));
        assert_eq!(
            parsed.error.as_ref().map(|error| error.code.as_str()),
            Some(error::OFFER_TEMPORARILY_UNAVAILABLE)
        );
        parsed.validate_for_offer(DOMAIN_ID, "camera-main").unwrap();
        assert!(matches!(
            parsed.validate_for_offer(DOMAIN_ID, "other"),
            Err(SubscribeEndError::PathMismatch { .. })
        ));
    }

    #[test]
    fn subscribe_end_rejects_unknown_reason() {
        let error = SubscribeEnd::from_value(json!({
            "type": SUBSCRIBE_END_TYPE,
            "domain_id": DOMAIN_ID,
            "offer_id": "camera-main",
            "reason": "unknown",
        }))
        .unwrap_err();
        assert_eq!(
            error,
            SubscribeEndError::UnsupportedReason {
                actual: "unknown".to_owned(),
            }
        );
        assert_eq!(error.failure_code(), error::MESSAGE_INVALID_ENVELOPE);
    }
}
