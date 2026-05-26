//! Offer-catalog protocol helpers for v1.

use super::{domain::decode_domain_id, error};
use serde_json::{Map, Value};
use std::fmt;

/// V1 offer-catalog fetch-path object type.
pub const OFFER_CATALOG_PATH_TYPE: &str = "auki.offer_catalog_path.v1";
/// V1 offer-catalog stream protocol id.
pub const OFFER_CATALOG_PROTOCOL_ID: &str = "/auki/offer-catalog/0.0.1";
/// V1 offer-catalog version string.
pub const OFFER_CATALOG_VERSION: &str = "auki.offer_catalog.v1";
/// V1 offer-catalog request object type.
pub const OFFER_CATALOG_REQUEST_TYPE: &str = "auki.offer_catalog_request.v1";

const FIELD_TYPE: &str = "type";
const FIELD_PROTOCOL_ID: &str = "protocol_id";
const FIELD_CATALOG_VERSION: &str = "catalog_version";
const FIELD_METADATA: &str = "metadata";
const FIELD_DOMAIN_IDS: &str = "domain_ids";
const FIELD_KINDS: &str = "kinds";
const FIELD_INCLUDE_INLINE_REGISTRY_ENTRIES: &str = "include_inline_registry_entries";

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
