//! Spatial message envelope and error-object helpers for v1 data paths.

use super::{
    base64url,
    domain::decode_domain_id,
    error,
    offer::{RegistryReference, RegistryReferenceError},
};
use serde_json::{Map, Value};
use std::fmt;

/// V1 spatial message envelope object type.
pub const SPATIAL_MESSAGE_TYPE: &str = "auki.spatial_message.v1";

const FIELD_TYPE: &str = "type";
const FIELD_DOMAIN_ID: &str = "domain_id";
const FIELD_OFFER_ID: &str = "offer_id";
const FIELD_PAYLOAD: &str = "payload";
const FIELD_SEQUENCE: &str = "sequence";
const FIELD_TIMESTAMP_NS: &str = "timestamp_ns";
const FIELD_CLOCK: &str = "clock";
const FIELD_REGISTRY_REFS: &str = "registry_refs";
const FIELD_GENERATED_AT: &str = "generated_at";
const FIELD_METADATA: &str = "metadata";
const FIELD_ENCODING: &str = "encoding";
const FIELD_SCHEMA_VERSION: &str = "schema_version";
const FIELD_MEDIA_TYPE: &str = "media_type";
const FIELD_BYTES: &str = "bytes";
const FIELD_JSON: &str = "json";
const FIELD_CODE: &str = "code";
const FIELD_MESSAGE: &str = "message";
const FIELD_KIND: &str = "kind";
const FIELD_RETRYABLE: &str = "retryable";
const FIELD_DETAILS: &str = "details";

/// Parsed v1 spatial message envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialMessage {
    value: Value,
    /// Domain id this message belongs to.
    pub domain_id: String,
    /// Offer id this message belongs to.
    pub offer_id: String,
    /// Message payload object.
    pub payload: PayloadObject,
    /// Optional sequence value.
    pub sequence: Option<u64>,
    /// Optional producer or domain timestamp in nanoseconds.
    pub timestamp_ns: Option<String>,
    /// Optional clock registry reference.
    pub clock: Option<RegistryReference>,
    /// Optional message-level registry references.
    pub registry_refs: Vec<RegistryReference>,
    /// Optional generation timestamp.
    pub generated_at: Option<String>,
    /// Optional non-authoritative metadata.
    pub metadata: Option<Value>,
}

/// Parsed v1 message payload object.
#[derive(Debug, Clone, PartialEq)]
pub struct PayloadObject {
    value: Value,
    /// Open payload family or schema type.
    pub payload_type: String,
    /// Optional payload encoding string.
    pub encoding: Option<String>,
    /// Optional schema-version string.
    pub schema_version: Option<String>,
    /// Optional media type.
    pub media_type: Option<String>,
    /// Optional decoded opaque bytes.
    pub bytes: Option<Vec<u8>>,
    /// Optional structured JSON payload.
    pub json: Option<Value>,
}

/// Parsed v1 protocol error object.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorObject {
    value: Value,
    /// Stable failure code.
    pub code: String,
    /// Optional diagnostic message.
    pub message: Option<String>,
    /// Optional domain id.
    pub domain_id: Option<String>,
    /// Optional offer id.
    pub offer_id: Option<String>,
    /// Optional offer kind.
    pub kind: Option<String>,
    /// Optional retry hint.
    pub retryable: Option<bool>,
    /// Optional structured diagnostic details.
    pub details: Option<Value>,
}

/// Errors produced while parsing or validating spatial message envelopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpatialMessageError {
    /// Message JSON value was not an object.
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
    /// Decimal integer string was malformed.
    InvalidDecimalInteger {
        /// Field name.
        field: &'static str,
        /// Actual value.
        value: String,
    },
    /// `sequence` did not fit in `u64`.
    SequenceOutOfRange {
        /// Actual sequence value.
        value: String,
    },
    /// Timestamp was not an RFC3339 UTC string with `Z` suffix.
    InvalidTimestamp {
        /// Field name.
        field: &'static str,
        /// Actual timestamp value.
        value: String,
    },
    /// Payload object was malformed.
    InvalidPayload(PayloadObjectError),
    /// Clock registry reference was malformed.
    InvalidClock(RegistryReferenceError),
    /// Message registry reference was malformed.
    InvalidRegistryReference {
        /// Index in `registry_refs`.
        index: usize,
        /// Error detail.
        error: RegistryReferenceError,
    },
    /// Message domain or offer did not match the requested path.
    OfferMismatch {
        /// Expected domain id.
        expected_domain_id: String,
        /// Actual domain id.
        actual_domain_id: String,
        /// Expected offer id.
        expected_offer_id: String,
        /// Actual offer id.
        actual_offer_id: String,
    },
    /// Message payload type did not match the selected payload type.
    PayloadTypeMismatch {
        /// Selected payload type.
        expected: String,
        /// Actual payload type.
        actual: String,
    },
}

impl SpatialMessageError {
    /// Stable RFC failure code for this message error.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::InvalidPayload(_) | Self::PayloadTypeMismatch { .. } => {
                error::MESSAGE_INVALID_PAYLOAD
            }
            _ => error::MESSAGE_INVALID_ENVELOPE,
        }
    }
}

impl fmt::Display for SpatialMessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "spatial message is not a json object"),
            Self::MissingField { field } => write!(f, "spatial message missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "spatial message field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported spatial message type {actual}")
            }
            Self::InvalidDomainId { domain_id, error } => {
                write!(f, "invalid spatial message domain id {domain_id}: {error}")
            }
            Self::InvalidDecimalInteger { field, value } => {
                write!(f, "invalid decimal integer in field {field}: {value}")
            }
            Self::SequenceOutOfRange { value } => {
                write!(f, "spatial message sequence is out of range: {value}")
            }
            Self::InvalidTimestamp { field, value } => {
                write!(f, "invalid timestamp in field {field}: {value}")
            }
            Self::InvalidPayload(error) => write!(f, "invalid spatial message payload: {error}"),
            Self::InvalidClock(error) => write!(f, "invalid spatial message clock: {error}"),
            Self::InvalidRegistryReference { index, error } => {
                write!(
                    f,
                    "invalid spatial message registry reference at index {index}: {error}"
                )
            }
            Self::OfferMismatch {
                expected_domain_id,
                actual_domain_id,
                expected_offer_id,
                actual_offer_id,
            } => write!(
                f,
                "spatial message offer mismatch: expected {expected_domain_id}/{expected_offer_id}, got {actual_domain_id}/{actual_offer_id}"
            ),
            Self::PayloadTypeMismatch { expected, actual } => {
                write!(
                    f,
                    "spatial message payload type mismatch: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for SpatialMessageError {}

/// Errors produced while parsing or validating message payload objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadObjectError {
    /// Payload JSON value was not an object.
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
    /// `bytes` was malformed base64url.
    InvalidBytes(String),
}

impl fmt::Display for PayloadObjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "payload is not a json object"),
            Self::MissingField { field } => write!(f, "payload missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "payload field {field} is not {expected}")
            }
            Self::InvalidBytes(error) => write!(f, "invalid payload bytes: {error}"),
        }
    }
}

impl std::error::Error for PayloadObjectError {}

/// Errors produced while parsing or validating v1 error objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorObjectError {
    /// Error JSON value was not an object.
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
    /// Optional `domain_id` was malformed.
    InvalidDomainId {
        /// Actual domain id string.
        domain_id: String,
        /// Error detail.
        error: String,
    },
}

impl fmt::Display for ErrorObjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "error object is not a json object"),
            Self::MissingField { field } => write!(f, "error object missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "error object field {field} is not {expected}")
            }
            Self::InvalidDomainId { domain_id, error } => {
                write!(f, "invalid error object domain id {domain_id}: {error}")
            }
        }
    }
}

impl std::error::Error for ErrorObjectError {}

impl SpatialMessage {
    /// Parse a v1 spatial message envelope from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, SpatialMessageError> {
        let object = value.as_object().ok_or(SpatialMessageError::NotObject)?;

        let type_value = required_message_string(object, FIELD_TYPE)?;
        if type_value != SPATIAL_MESSAGE_TYPE {
            return Err(SpatialMessageError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let domain_id = required_message_string(object, FIELD_DOMAIN_ID)?.to_owned();
        validate_message_domain_id(&domain_id)?;
        let offer_id = required_message_string(object, FIELD_OFFER_ID)?.to_owned();
        let payload = object
            .get(FIELD_PAYLOAD)
            .ok_or(SpatialMessageError::MissingField {
                field: FIELD_PAYLOAD,
            })
            .and_then(|value| {
                PayloadObject::from_value(value.clone())
                    .map_err(SpatialMessageError::InvalidPayload)
            })?;
        let sequence = optional_sequence(object)?;
        let timestamp_ns = optional_decimal_string(object, FIELD_TIMESTAMP_NS)?;
        let clock = object
            .get(FIELD_CLOCK)
            .map(|value| {
                RegistryReference::from_value(value.clone())
                    .map_err(SpatialMessageError::InvalidClock)
            })
            .transpose()?;
        let registry_refs = parse_registry_refs(object)?;
        let generated_at = optional_message_timestamp(object, FIELD_GENERATED_AT)?;
        let metadata = optional_message_object(object, FIELD_METADATA)?;

        Ok(Self {
            value,
            domain_id,
            offer_id,
            payload,
            sequence,
            timestamp_ns,
            clock,
            registry_refs,
            generated_at,
            metadata,
        })
    }

    /// Borrow the original spatial message JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this message and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Return the raw payload byte length defined by RFC-0028.
    pub fn raw_payload_len(&self) -> usize {
        self.payload.raw_payload_len()
    }

    /// Validate the message against a request path and selected payload type.
    pub fn validate_for_offer(
        &self,
        domain_id: &str,
        offer_id: &str,
        selected_payload_type: &str,
    ) -> Result<(), SpatialMessageError> {
        if self.domain_id != domain_id || self.offer_id != offer_id {
            return Err(SpatialMessageError::OfferMismatch {
                expected_domain_id: domain_id.to_owned(),
                actual_domain_id: self.domain_id.clone(),
                expected_offer_id: offer_id.to_owned(),
                actual_offer_id: self.offer_id.clone(),
            });
        }
        if self.payload.payload_type != selected_payload_type {
            return Err(SpatialMessageError::PayloadTypeMismatch {
                expected: selected_payload_type.to_owned(),
                actual: self.payload.payload_type.clone(),
            });
        }
        Ok(())
    }
}

impl PayloadObject {
    /// Parse a v1 payload object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, PayloadObjectError> {
        let object = value.as_object().ok_or(PayloadObjectError::NotObject)?;
        let payload_type = required_payload_string(object, FIELD_TYPE)?.to_owned();
        let encoding = optional_payload_string(object, FIELD_ENCODING)?;
        let schema_version = optional_payload_string(object, FIELD_SCHEMA_VERSION)?;
        let media_type = optional_payload_string(object, FIELD_MEDIA_TYPE)?;
        let bytes = object
            .get(FIELD_BYTES)
            .map(|value| {
                let encoded = value.as_str().ok_or(PayloadObjectError::InvalidFieldType {
                    field: FIELD_BYTES,
                    expected: "a string",
                })?;
                base64url::decode(encoded)
                    .map_err(|error| PayloadObjectError::InvalidBytes(error.to_string()))
            })
            .transpose()?;
        let json = object.get(FIELD_JSON).cloned();

        Ok(Self {
            value,
            payload_type,
            encoding,
            schema_version,
            media_type,
            bytes,
            json,
        })
    }

    /// Borrow the original payload JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this payload and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Return the raw payload byte length defined by RFC-0028.
    pub fn raw_payload_len(&self) -> usize {
        let bytes_len = self.bytes.as_ref().map_or(0, Vec::len);
        let json_len = self.json.as_ref().map_or(0, |value| {
            serde_json::to_vec(value)
                .expect("serde_json::Value serializes")
                .len()
        });
        bytes_len + json_len
    }
}

impl ErrorObject {
    /// Create a minimal v1 error object.
    pub fn create(code: impl Into<String>) -> Self {
        let code = code.into();
        let mut object = Map::new();
        object.insert(FIELD_CODE.to_owned(), Value::String(code.clone()));
        Self {
            value: Value::Object(object),
            code,
            message: None,
            domain_id: None,
            offer_id: None,
            kind: None,
            retryable: None,
            details: None,
        }
    }

    /// Parse a v1 error object from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, ErrorObjectError> {
        let object = value.as_object().ok_or(ErrorObjectError::NotObject)?;
        let code = required_error_string(object, FIELD_CODE)?.to_owned();
        let message = optional_error_string(object, FIELD_MESSAGE)?;
        let domain_id = optional_error_domain_id(object)?;
        let offer_id = optional_error_string(object, FIELD_OFFER_ID)?;
        let kind = optional_error_string(object, FIELD_KIND)?;
        let retryable = object
            .get(FIELD_RETRYABLE)
            .map(|value| {
                value.as_bool().ok_or(ErrorObjectError::InvalidFieldType {
                    field: FIELD_RETRYABLE,
                    expected: "a boolean",
                })
            })
            .transpose()?;
        let details = optional_error_object(object, FIELD_DETAILS)?;

        Ok(Self {
            value,
            code,
            message,
            domain_id,
            offer_id,
            kind,
            retryable,
            details,
        })
    }

    /// Borrow the original error JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this error object and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }
}

fn required_message_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, SpatialMessageError> {
    object
        .get(field)
        .ok_or(SpatialMessageError::MissingField { field })?
        .as_str()
        .ok_or(SpatialMessageError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn validate_message_domain_id(domain_id: &str) -> Result<(), SpatialMessageError> {
    decode_domain_id(domain_id).map_err(|error| SpatialMessageError::InvalidDomainId {
        domain_id: domain_id.to_owned(),
        error: error.to_string(),
    })?;
    Ok(())
}

fn optional_sequence(object: &Map<String, Value>) -> Result<Option<u64>, SpatialMessageError> {
    let Some(sequence) = optional_decimal_string(object, FIELD_SEQUENCE)? else {
        return Ok(None);
    };
    sequence
        .parse::<u64>()
        .map(Some)
        .map_err(|_| SpatialMessageError::SequenceOutOfRange { value: sequence })
}

fn optional_decimal_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, SpatialMessageError> {
    object
        .get(field)
        .map(|value| {
            let decimal = value
                .as_str()
                .ok_or(SpatialMessageError::InvalidFieldType {
                    field,
                    expected: "a string",
                })?;
            validate_decimal_integer(field, decimal)?;
            Ok(decimal.to_owned())
        })
        .transpose()
}

fn validate_decimal_integer(field: &'static str, value: &str) -> Result<(), SpatialMessageError> {
    let valid = value == "0"
        || value
            .strip_prefix(|byte: char| ('1'..='9').contains(&byte))
            .is_some_and(|rest| rest.bytes().all(|byte| byte.is_ascii_digit()));
    if valid {
        Ok(())
    } else {
        Err(SpatialMessageError::InvalidDecimalInteger {
            field,
            value: value.to_owned(),
        })
    }
}

fn parse_registry_refs(
    object: &Map<String, Value>,
) -> Result<Vec<RegistryReference>, SpatialMessageError> {
    object
        .get(FIELD_REGISTRY_REFS)
        .map(|value| {
            let values = value
                .as_array()
                .ok_or(SpatialMessageError::InvalidFieldType {
                    field: FIELD_REGISTRY_REFS,
                    expected: "an array",
                })?;
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    RegistryReference::from_value(value.clone()).map_err(|error| {
                        SpatialMessageError::InvalidRegistryReference { index, error }
                    })
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn optional_message_timestamp(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, SpatialMessageError> {
    object
        .get(field)
        .map(|value| {
            let timestamp = value
                .as_str()
                .ok_or(SpatialMessageError::InvalidFieldType {
                    field,
                    expected: "a string",
                })?;
            validate_message_timestamp(field, timestamp)?;
            Ok(timestamp.to_owned())
        })
        .transpose()
}

fn validate_message_timestamp(field: &'static str, value: &str) -> Result<(), SpatialMessageError> {
    if is_rfc3339_z_timestamp(value) {
        Ok(())
    } else {
        Err(SpatialMessageError::InvalidTimestamp {
            field,
            value: value.to_owned(),
        })
    }
}

fn optional_message_object(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Value>, SpatialMessageError> {
    object
        .get(field)
        .map(|value| {
            if value.is_object() {
                Ok(value.clone())
            } else {
                Err(SpatialMessageError::InvalidFieldType {
                    field,
                    expected: "an object",
                })
            }
        })
        .transpose()
}

fn required_payload_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, PayloadObjectError> {
    object
        .get(field)
        .ok_or(PayloadObjectError::MissingField { field })?
        .as_str()
        .ok_or(PayloadObjectError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn optional_payload_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, PayloadObjectError> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or(PayloadObjectError::InvalidFieldType {
                    field,
                    expected: "a string",
                })
        })
        .transpose()
}

fn required_error_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, ErrorObjectError> {
    object
        .get(field)
        .ok_or(ErrorObjectError::MissingField { field })?
        .as_str()
        .ok_or(ErrorObjectError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn optional_error_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, ErrorObjectError> {
    object
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or(ErrorObjectError::InvalidFieldType {
                    field,
                    expected: "a string",
                })
        })
        .transpose()
}

fn optional_error_domain_id(
    object: &Map<String, Value>,
) -> Result<Option<String>, ErrorObjectError> {
    let Some(domain_id) = optional_error_string(object, FIELD_DOMAIN_ID)? else {
        return Ok(None);
    };
    decode_domain_id(&domain_id).map_err(|error| ErrorObjectError::InvalidDomainId {
        domain_id: domain_id.clone(),
        error: error.to_string(),
    })?;
    Ok(Some(domain_id))
}

fn optional_error_object(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Value>, ErrorObjectError> {
    object
        .get(field)
        .map(|value| {
            if value.is_object() {
                Ok(value.clone())
            } else {
                Err(ErrorObjectError::InvalidFieldType {
                    field,
                    expected: "an object",
                })
            }
        })
        .transpose()
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
                "json": {"ok": true},
            },
            "sequence": "1",
            "timestamp_ns": "123456789",
            "generated_at": "2026-05-26T12:00:00Z",
            "metadata": {"fixture": true},
        })
    }

    #[test]
    fn parses_spatial_message_and_validates_offer_binding() {
        let message = SpatialMessage::from_value(message_value()).unwrap();

        assert_eq!(message.domain_id, DOMAIN_ID);
        assert_eq!(message.offer_id, "camera-main");
        assert_eq!(message.payload.payload_type, "auki.frame");
        assert_eq!(message.payload.bytes, Some(vec![1, 2, 3]));
        assert_eq!(message.payload.json, Some(json!({"ok": true})));
        assert_eq!(message.sequence, Some(1));
        assert_eq!(message.timestamp_ns, Some("123456789".to_owned()));
        assert_eq!(message.raw_payload_len(), 14);
        message
            .validate_for_offer(DOMAIN_ID, "camera-main", "auki.frame")
            .unwrap();
    }

    #[test]
    fn rejects_message_offer_and_payload_mismatch() {
        let message = SpatialMessage::from_value(message_value()).unwrap();

        assert_eq!(
            message.validate_for_offer(DOMAIN_ID, "other", "auki.frame"),
            Err(SpatialMessageError::OfferMismatch {
                expected_domain_id: DOMAIN_ID.to_owned(),
                actual_domain_id: DOMAIN_ID.to_owned(),
                expected_offer_id: "other".to_owned(),
                actual_offer_id: "camera-main".to_owned(),
            })
        );

        let error = message
            .validate_for_offer(DOMAIN_ID, "camera-main", "other.payload")
            .unwrap_err();
        assert_eq!(error.failure_code(), error::MESSAGE_INVALID_PAYLOAD);
    }

    #[test]
    fn rejects_malformed_sequence_and_payload_bytes() {
        let mut bad_sequence = message_value();
        bad_sequence
            .as_object_mut()
            .unwrap()
            .insert(FIELD_SEQUENCE.to_owned(), json!("01"));
        assert!(matches!(
            SpatialMessage::from_value(bad_sequence),
            Err(SpatialMessageError::InvalidDecimalInteger {
                field: FIELD_SEQUENCE,
                ..
            })
        ));

        let mut bad_bytes = message_value();
        bad_bytes["payload"]
            .as_object_mut()
            .unwrap()
            .insert(FIELD_BYTES.to_owned(), json!("AQID="));
        assert!(matches!(
            SpatialMessage::from_value(bad_bytes),
            Err(SpatialMessageError::InvalidPayload(
                PayloadObjectError::InvalidBytes(_)
            ))
        ));
    }

    #[test]
    fn parses_error_object() {
        let error_object = ErrorObject::from_value(json!({
            "code": error::OFFER_UNKNOWN_OFFER,
            "message": "unknown offer",
            "domain_id": DOMAIN_ID,
            "offer_id": "camera-main",
            "kind": "sensor.frame",
            "retryable": false,
            "details": {"missing": true},
        }))
        .unwrap();

        assert_eq!(error_object.code, error::OFFER_UNKNOWN_OFFER);
        assert_eq!(error_object.domain_id, Some(DOMAIN_ID.to_owned()));
        assert_eq!(error_object.offer_id, Some("camera-main".to_owned()));
        assert_eq!(error_object.retryable, Some(false));
    }
}
