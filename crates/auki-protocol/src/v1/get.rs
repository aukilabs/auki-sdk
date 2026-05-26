//! Get protocol helpers for v1.

use super::{
    domain::decode_domain_id,
    error,
    message::{ErrorObject, ErrorObjectError, SpatialMessage, SpatialMessageError},
};
use serde_json::{Map, Number, Value};
use std::fmt;

/// V1 Get stream protocol id.
pub const GET_PROTOCOL_ID: &str = "/auki/get/0.0.1";
/// V1 Get request object type.
pub const GET_REQUEST_TYPE: &str = "auki.get_request.v1";
/// V1 Get response object type.
pub const GET_RESPONSE_TYPE: &str = "auki.get_response.v1";

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

const FIELD_TYPE: &str = "type";
const FIELD_DOMAIN_ID: &str = "domain_id";
const FIELD_OFFER_ID: &str = "offer_id";
const FIELD_PARAMS: &str = "params";
const FIELD_ACCEPTED_PAYLOAD_TYPES: &str = "accepted_payload_types";
const FIELD_MAX_PAYLOAD_BYTES: &str = "max_payload_bytes";
const FIELD_MESSAGE: &str = "message";
const FIELD_ERROR: &str = "error";

/// Parsed v1 Get request.
#[derive(Debug, Clone, PartialEq)]
pub struct GetRequest {
    value: Value,
    /// Domain id targeted by this request.
    pub domain_id: String,
    /// Offer id targeted by this request.
    pub offer_id: String,
    /// Optional offer-kind-specific params.
    pub params: Option<Value>,
    /// Accepted payload types. Empty means any locally supported selected type.
    pub accepted_payload_types: Vec<String>,
    /// Optional positive raw-payload byte limit.
    pub max_payload_bytes: Option<u64>,
}

/// Parsed v1 Get response.
#[derive(Debug, Clone, PartialEq)]
pub struct GetResponse {
    value: Value,
    /// Successful spatial message or failed error object.
    pub body: GetResponseBody,
}

/// V1 Get response body.
#[derive(Debug, Clone, PartialEq)]
pub enum GetResponseBody {
    /// Successful Get response.
    Message(SpatialMessage),
    /// Failed Get response.
    Error(ErrorObject),
}

/// Errors produced while creating or parsing Get requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetRequestError {
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
    /// `max_payload_bytes` was zero.
    MaxPayloadBytesZero,
    /// `max_payload_bytes` exceeded JSON safe integer range.
    MaxPayloadBytesTooLarge {
        /// Actual value.
        value: u64,
    },
}

impl GetRequestError {
    /// Stable RFC failure code for this Get request error.
    pub fn failure_code(&self) -> &'static str {
        error::GET_INVALID_REQUEST
    }
}

impl fmt::Display for GetRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "get request is not a json object"),
            Self::MissingField { field } => write!(f, "get request missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "get request field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => write!(f, "unsupported get request type {actual}"),
            Self::InvalidDomainId { domain_id, error } => {
                write!(f, "invalid get request domain id {domain_id}: {error}")
            }
            Self::MaxPayloadBytesZero => {
                write!(f, "get request max_payload_bytes must be positive")
            }
            Self::MaxPayloadBytesTooLarge { value } => {
                write!(
                    f,
                    "get request max_payload_bytes exceeds safe integer range: {value}"
                )
            }
        }
    }
}

impl std::error::Error for GetRequestError {}

/// Errors produced while creating, parsing, or validating Get responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetResponseError {
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
    /// Neither `message` nor `error` was present.
    MissingBody,
    /// Both `message` and `error` were present.
    MultipleBodies,
    /// Successful message body was malformed.
    InvalidMessage(SpatialMessageError),
    /// Failed error body was malformed.
    InvalidError(ErrorObjectError),
    /// A success-only validation helper was used on a failed response.
    ErrorResponse {
        /// Stable error code from the failed response.
        code: String,
    },
    /// Selected response payload type was not accepted by the request.
    PayloadTypeNotAccepted {
        /// Actual selected payload type.
        payload_type: String,
    },
    /// Message raw payload bytes exceeded a request limit.
    PayloadTooLarge {
        /// Actual raw payload byte count.
        actual: usize,
        /// Maximum allowed raw payload byte count.
        max: u64,
    },
}

impl GetResponseError {
    /// Stable RFC failure code for this Get response error.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::InvalidMessage(error) => error.failure_code(),
            Self::PayloadTooLarge { .. } => error::MESSAGE_PAYLOAD_TOO_LARGE,
            _ => error::MESSAGE_INVALID_ENVELOPE,
        }
    }
}

impl fmt::Display for GetResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "get response is not a json object"),
            Self::MissingField { field } => write!(f, "get response missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "get response field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => write!(f, "unsupported get response type {actual}"),
            Self::MissingBody => write!(f, "get response missing message or error body"),
            Self::MultipleBodies => write!(f, "get response includes both message and error"),
            Self::InvalidMessage(error) => write!(f, "invalid get response message: {error}"),
            Self::InvalidError(error) => write!(f, "invalid get response error object: {error}"),
            Self::ErrorResponse { code } => write!(f, "get response failed with {code}"),
            Self::PayloadTypeNotAccepted { payload_type } => {
                write!(
                    f,
                    "get response payload type was not requested: {payload_type}"
                )
            }
            Self::PayloadTooLarge { actual, max } => {
                write!(f, "get response payload too large: {actual} > {max}")
            }
        }
    }
}

impl std::error::Error for GetResponseError {}

impl GetRequest {
    /// Create a v1 Get request.
    pub fn create(
        domain_id: impl Into<String>,
        offer_id: impl Into<String>,
        params: Option<Value>,
        accepted_payload_types: Vec<String>,
        max_payload_bytes: Option<u64>,
    ) -> Result<Self, GetRequestError> {
        let domain_id = domain_id.into();
        let offer_id = offer_id.into();
        validate_domain_id(&domain_id)?;
        if let Some(params) = &params
            && !params.is_object()
        {
            return Err(GetRequestError::InvalidFieldType {
                field: FIELD_PARAMS,
                expected: "an object",
            });
        }
        validate_max_payload_bytes(max_payload_bytes)?;

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(GET_REQUEST_TYPE.to_owned()),
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
        if let Some(max_payload_bytes) = max_payload_bytes {
            object.insert(
                FIELD_MAX_PAYLOAD_BYTES.to_owned(),
                Value::Number(Number::from(max_payload_bytes)),
            );
        }

        Ok(Self {
            value: Value::Object(object),
            domain_id,
            offer_id,
            params,
            accepted_payload_types,
            max_payload_bytes,
        })
    }

    /// Parse a v1 Get request from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, GetRequestError> {
        let object = value.as_object().ok_or(GetRequestError::NotObject)?;
        let type_value = required_request_string(object, FIELD_TYPE)?;
        if type_value != GET_REQUEST_TYPE {
            return Err(GetRequestError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let domain_id = required_request_string(object, FIELD_DOMAIN_ID)?.to_owned();
        validate_domain_id(&domain_id)?;
        let offer_id = required_request_string(object, FIELD_OFFER_ID)?.to_owned();
        let params = optional_request_object(object, FIELD_PARAMS)?;
        let accepted_payload_types =
            optional_request_string_array(object, FIELD_ACCEPTED_PAYLOAD_TYPES)?;
        let max_payload_bytes = optional_max_payload_bytes(object)?;

        Ok(Self {
            value,
            domain_id,
            offer_id,
            params,
            accepted_payload_types,
            max_payload_bytes,
        })
    }

    /// Borrow the original Get request JSON object.
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

impl GetResponse {
    /// Create a successful v1 Get response.
    pub fn success(message: SpatialMessage) -> Self {
        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(GET_RESPONSE_TYPE.to_owned()),
        );
        object.insert(FIELD_MESSAGE.to_owned(), message.value().clone());
        Self {
            value: Value::Object(object),
            body: GetResponseBody::Message(message),
        }
    }

    /// Create a failed v1 Get response.
    pub fn failure(error: ErrorObject) -> Self {
        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(GET_RESPONSE_TYPE.to_owned()),
        );
        object.insert(FIELD_ERROR.to_owned(), error.value().clone());
        Self {
            value: Value::Object(object),
            body: GetResponseBody::Error(error),
        }
    }

    /// Parse a v1 Get response from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, GetResponseError> {
        let object = value.as_object().ok_or(GetResponseError::NotObject)?;
        let type_value = required_response_string(object, FIELD_TYPE)?;
        if type_value != GET_RESPONSE_TYPE {
            return Err(GetResponseError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let message = object.get(FIELD_MESSAGE).cloned();
        let error = object.get(FIELD_ERROR).cloned();

        match (message, error) {
            (Some(_), Some(_)) => Err(GetResponseError::MultipleBodies),
            (None, None) => Err(GetResponseError::MissingBody),
            (Some(message), None) => Ok(Self {
                value,
                body: GetResponseBody::Message(
                    SpatialMessage::from_value(message)
                        .map_err(GetResponseError::InvalidMessage)?,
                ),
            }),
            (None, Some(error)) => Ok(Self {
                value,
                body: GetResponseBody::Error(
                    ErrorObject::from_value(error).map_err(GetResponseError::InvalidError)?,
                ),
            }),
        }
    }

    /// Borrow the original Get response JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this response and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Return the successful message, if this is a successful response.
    pub fn message(&self) -> Option<&SpatialMessage> {
        match &self.body {
            GetResponseBody::Message(message) => Some(message),
            GetResponseBody::Error(_) => None,
        }
    }

    /// Validate a successful response against its request and selected payload type.
    pub fn validate_success_for_request(
        &self,
        request: &GetRequest,
        selected_payload_type: &str,
    ) -> Result<&SpatialMessage, GetResponseError> {
        let message = match &self.body {
            GetResponseBody::Message(message) => message,
            GetResponseBody::Error(error) => {
                return Err(GetResponseError::ErrorResponse {
                    code: error.code.clone(),
                });
            }
        };

        if !request.accepts_payload_type(selected_payload_type) {
            return Err(GetResponseError::PayloadTypeNotAccepted {
                payload_type: selected_payload_type.to_owned(),
            });
        }

        message
            .validate_for_offer(&request.domain_id, &request.offer_id, selected_payload_type)
            .map_err(GetResponseError::InvalidMessage)?;

        if let Some(max) = request.max_payload_bytes {
            let actual = message.raw_payload_len();
            if actual as u64 > max {
                return Err(GetResponseError::PayloadTooLarge { actual, max });
            }
        }

        Ok(message)
    }
}

fn required_request_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, GetRequestError> {
    object
        .get(field)
        .ok_or(GetRequestError::MissingField { field })?
        .as_str()
        .ok_or(GetRequestError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn validate_domain_id(domain_id: &str) -> Result<(), GetRequestError> {
    decode_domain_id(domain_id).map_err(|error| GetRequestError::InvalidDomainId {
        domain_id: domain_id.to_owned(),
        error: error.to_string(),
    })?;
    Ok(())
}

fn optional_request_object(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Value>, GetRequestError> {
    object
        .get(field)
        .map(|value| {
            if value.is_object() {
                Ok(value.clone())
            } else {
                Err(GetRequestError::InvalidFieldType {
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
) -> Result<Vec<String>, GetRequestError> {
    object
        .get(field)
        .map(|value| {
            let values = value.as_array().ok_or(GetRequestError::InvalidFieldType {
                field,
                expected: "an array",
            })?;
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .ok_or(GetRequestError::InvalidFieldType {
                            field,
                            expected: "an array of strings",
                        })
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn optional_max_payload_bytes(object: &Map<String, Value>) -> Result<Option<u64>, GetRequestError> {
    object
        .get(FIELD_MAX_PAYLOAD_BYTES)
        .map(|value| {
            value
                .as_u64()
                .ok_or(GetRequestError::InvalidFieldType {
                    field: FIELD_MAX_PAYLOAD_BYTES,
                    expected: "a positive safe integer",
                })
                .and_then(|max| {
                    validate_max_payload_bytes(Some(max))?;
                    Ok(max)
                })
        })
        .transpose()
}

fn validate_max_payload_bytes(max_payload_bytes: Option<u64>) -> Result<(), GetRequestError> {
    let Some(max_payload_bytes) = max_payload_bytes else {
        return Ok(());
    };
    if max_payload_bytes == 0 {
        Err(GetRequestError::MaxPayloadBytesZero)
    } else if max_payload_bytes > MAX_SAFE_INTEGER {
        Err(GetRequestError::MaxPayloadBytesTooLarge {
            value: max_payload_bytes,
        })
    } else {
        Ok(())
    }
}

fn required_response_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, GetResponseError> {
    object
        .get(field)
        .ok_or(GetResponseError::MissingField { field })?
        .as_str()
        .ok_or(GetResponseError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::message::{SPATIAL_MESSAGE_TYPE, SpatialMessage};
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
        })
    }

    #[test]
    fn create_and_parse_get_request() {
        let request = GetRequest::create(
            DOMAIN_ID,
            "camera-main",
            Some(json!({"frame": "latest"})),
            vec!["auki.frame".to_owned()],
            Some(1024),
        )
        .unwrap();
        let parsed = GetRequest::from_value(request.value().clone()).unwrap();

        assert_eq!(parsed.value(), request.value());
        assert_eq!(parsed.domain_id, DOMAIN_ID);
        assert_eq!(parsed.offer_id, "camera-main");
        assert_eq!(parsed.params, Some(json!({"frame": "latest"})));
        assert_eq!(parsed.accepted_payload_types, vec!["auki.frame"]);
        assert_eq!(parsed.max_payload_bytes, Some(1024));
        assert!(parsed.accepts_payload_type("auki.frame"));
        assert!(!parsed.accepts_payload_type("other"));
    }

    #[test]
    fn get_request_defaults_optional_fields() {
        let request = GetRequest::from_value(json!({
            "type": GET_REQUEST_TYPE,
            "domain_id": DOMAIN_ID,
            "offer_id": "camera-main",
        }))
        .unwrap();

        assert_eq!(request.params, None);
        assert!(request.accepted_payload_types.is_empty());
        assert!(request.accepts_payload_type("any.payload"));
        assert_eq!(request.max_payload_bytes, None);
    }

    #[test]
    fn get_request_rejects_invalid_domain_and_size() {
        assert!(matches!(
            GetRequest::from_value(json!({
                "type": GET_REQUEST_TYPE,
                "domain_id": "bad",
                "offer_id": "camera-main",
            })),
            Err(GetRequestError::InvalidDomainId { .. })
        ));

        assert_eq!(
            GetRequest::from_value(json!({
                "type": GET_REQUEST_TYPE,
                "domain_id": DOMAIN_ID,
                "offer_id": "camera-main",
                "max_payload_bytes": 0,
            })),
            Err(GetRequestError::MaxPayloadBytesZero)
        );
    }

    #[test]
    fn get_response_parses_success_and_validates_request_binding() {
        let request = GetRequest::create(
            DOMAIN_ID,
            "camera-main",
            None,
            vec!["auki.frame".to_owned()],
            Some(8),
        )
        .unwrap();
        let response = GetResponse::from_value(json!({
            "type": GET_RESPONSE_TYPE,
            "message": message_value(),
        }))
        .unwrap();

        let message = response
            .validate_success_for_request(&request, "auki.frame")
            .unwrap();

        assert_eq!(message.domain_id, DOMAIN_ID);
        assert_eq!(message.raw_payload_len(), 3);
    }

    #[test]
    fn get_response_parses_failure() {
        let response = GetResponse::from_value(json!({
            "type": GET_RESPONSE_TYPE,
            "error": {
                "code": error::OFFER_UNKNOWN_OFFER,
                "domain_id": DOMAIN_ID,
                "offer_id": "camera-main",
            },
        }))
        .unwrap();

        assert!(matches!(response.body, GetResponseBody::Error(_)));
    }

    #[test]
    fn get_response_rejects_missing_or_multiple_bodies() {
        assert_eq!(
            GetResponse::from_value(json!({"type": GET_RESPONSE_TYPE})),
            Err(GetResponseError::MissingBody)
        );
        assert_eq!(
            GetResponse::from_value(json!({
                "type": GET_RESPONSE_TYPE,
                "message": message_value(),
                "error": {"code": error::OFFER_UNKNOWN_OFFER},
            })),
            Err(GetResponseError::MultipleBodies)
        );
    }

    #[test]
    fn get_response_validation_rejects_mismatch_payload_type_and_size() {
        let response = GetResponse::success(SpatialMessage::from_value(message_value()).unwrap());
        let request = GetRequest::create(DOMAIN_ID, "other", None, vec![], Some(8)).unwrap();

        let mismatch = response
            .validate_success_for_request(&request, "auki.frame")
            .unwrap_err();
        assert_eq!(mismatch.failure_code(), error::MESSAGE_INVALID_ENVELOPE);

        let request = GetRequest::create(DOMAIN_ID, "camera-main", None, vec![], Some(8)).unwrap();
        let payload_mismatch = response
            .validate_success_for_request(&request, "other.payload")
            .unwrap_err();
        assert_eq!(
            payload_mismatch.failure_code(),
            error::MESSAGE_INVALID_PAYLOAD
        );

        let request = GetRequest::create(
            DOMAIN_ID,
            "camera-main",
            None,
            vec!["other.payload".to_owned()],
            Some(8),
        )
        .unwrap();
        let payload_not_accepted = response
            .validate_success_for_request(&request, "auki.frame")
            .unwrap_err();
        assert_eq!(
            payload_not_accepted,
            GetResponseError::PayloadTypeNotAccepted {
                payload_type: "auki.frame".to_owned(),
            }
        );
        assert_eq!(
            payload_not_accepted.failure_code(),
            error::MESSAGE_INVALID_ENVELOPE
        );

        let tiny_request =
            GetRequest::create(DOMAIN_ID, "camera-main", None, vec![], Some(2)).unwrap();
        let too_large = response
            .validate_success_for_request(&tiny_request, "auki.frame")
            .unwrap_err();
        assert_eq!(too_large.failure_code(), error::MESSAGE_PAYLOAD_TOO_LARGE);
    }
}
