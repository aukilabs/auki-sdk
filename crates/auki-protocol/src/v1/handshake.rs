//! V1 peer handshake message helpers.

use super::{
    authority::{
        AuthorityChain, AuthorityChainError, AuthorityChainInput, AuthorityChainPolicyInput,
        DeclaredDomain, PeerAuthorization, PeerAuthorizationPolicy, validate_authority_chain,
        validate_authority_chain_with_authorization_policy,
    },
    error,
    identity::PeerBinding,
    offer::OfferCatalogPath,
};
use libp2p_identity::PeerId;
use serde_json::{Map, Value};
use std::{collections::HashSet, fmt};

/// V1 peer-handshake object type.
pub const PEER_HANDSHAKE_TYPE: &str = "auki.peer_handshake.v1";
/// Baseline lifecycle version required in every v1 handshake.
pub const CLUSTER_LIFECYCLE_V1: &str = "auki.cluster_lifecycle.v1";

const FIELD_TYPE: &str = "type";
const FIELD_SUPPORTED_LIFECYCLE_VERSIONS: &str = "supported_lifecycle_versions";
const FIELD_PEER_BINDING: &str = "peer_binding";
const FIELD_DECLARED_DOMAINS: &str = "declared_domains";
const FIELD_AUTHORIZATION_MATERIAL: &str = "authorization_material";
const FIELD_OFFER_CATALOG: &str = "offer_catalog";
const FIELD_DIAGNOSTICS: &str = "diagnostics";
const FIELD_METADATA: &str = "metadata";

/// Parsed v1 peer handshake.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerHandshake {
    value: Value,
    /// Supported lifecycle versions from the remote peer.
    pub supported_lifecycle_versions: Vec<String>,
    /// Remote peer binding.
    pub peer_binding: PeerBinding,
    /// Declared domains.
    pub declared_domains: Vec<DeclaredDomain>,
    /// Optional authorization material for local peer policy.
    pub authorization_material: Option<Vec<Value>>,
    /// Optional offer-catalog fetch path.
    pub offer_catalog: Option<OfferCatalogPath>,
    /// Optional diagnostics object.
    pub diagnostics: Option<Value>,
    /// Optional metadata object.
    pub metadata: Option<Value>,
}

/// Errors produced while creating or parsing peer handshakes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    /// Handshake JSON value was not an object.
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
    /// `supported_lifecycle_versions` was empty.
    EmptyLifecycleVersions,
    /// A lifecycle version appeared more than once.
    DuplicateLifecycleVersion {
        /// Duplicated lifecycle version string.
        version: String,
    },
    /// Required baseline lifecycle version was absent.
    MissingRequiredLifecycleVersion,
    /// Peer binding object was malformed.
    InvalidPeerBinding(String),
    /// Declared-domain object was malformed.
    InvalidDeclaredDomain {
        /// Index in `declared_domains`.
        index: usize,
        /// Error detail.
        error: String,
    },
    /// Offer-catalog fetch path was malformed.
    InvalidOfferCatalog(String),
}

impl HandshakeError {
    /// Stable RFC failure code for this handshake parse error.
    pub fn failure_code(&self) -> &'static str {
        match self {
            Self::MissingRequiredLifecycleVersion => error::PROTOCOL_UNSUPPORTED_VERSION,
            _ => error::HANDSHAKE_INVALID_MESSAGE,
        }
    }
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "peer handshake is not a json object"),
            Self::MissingField { field } => write!(f, "peer handshake missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "peer handshake field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported peer handshake type {actual}")
            }
            Self::EmptyLifecycleVersions => {
                write!(f, "supported_lifecycle_versions must be non-empty")
            }
            Self::DuplicateLifecycleVersion { version } => {
                write!(f, "duplicate lifecycle version {version}")
            }
            Self::MissingRequiredLifecycleVersion => {
                write!(
                    f,
                    "supported_lifecycle_versions missing {CLUSTER_LIFECYCLE_V1}"
                )
            }
            Self::InvalidPeerBinding(error) => write!(f, "invalid peer binding: {error}"),
            Self::InvalidDeclaredDomain { index, error } => {
                write!(f, "invalid declared domain at index {index}: {error}")
            }
            Self::InvalidOfferCatalog(error) => write!(f, "invalid offer catalog path: {error}"),
        }
    }
}

impl std::error::Error for HandshakeError {}

impl PeerHandshake {
    /// Create a v1 peer handshake from parsed authority objects.
    pub fn create(peer_binding: PeerBinding, declared_domains: Vec<DeclaredDomain>) -> Self {
        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(PEER_HANDSHAKE_TYPE.to_owned()),
        );
        object.insert(
            FIELD_SUPPORTED_LIFECYCLE_VERSIONS.to_owned(),
            Value::Array(vec![Value::String(CLUSTER_LIFECYCLE_V1.to_owned())]),
        );
        object.insert(FIELD_PEER_BINDING.to_owned(), peer_binding.value().clone());
        object.insert(
            FIELD_DECLARED_DOMAINS.to_owned(),
            Value::Array(declared_domains.iter().map(declared_domain_value).collect()),
        );

        Self {
            value: Value::Object(object),
            supported_lifecycle_versions: vec![CLUSTER_LIFECYCLE_V1.to_owned()],
            peer_binding,
            declared_domains,
            authorization_material: None,
            offer_catalog: None,
            diagnostics: None,
            metadata: None,
        }
    }

    /// Parse a v1 peer handshake from a JSON value.
    pub fn from_value(value: Value) -> Result<Self, HandshakeError> {
        let object = value.as_object().ok_or(HandshakeError::NotObject)?;

        let type_value = required_string(object, FIELD_TYPE)?;
        if type_value != PEER_HANDSHAKE_TYPE {
            return Err(HandshakeError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let supported_lifecycle_versions = parse_lifecycle_versions(object)?;
        let peer_binding = object
            .get(FIELD_PEER_BINDING)
            .ok_or(HandshakeError::MissingField {
                field: FIELD_PEER_BINDING,
            })
            .and_then(|value| {
                PeerBinding::from_value(value.clone())
                    .map_err(|error| HandshakeError::InvalidPeerBinding(error.to_string()))
            })?;
        let declared_domains = parse_declared_domains(object)?;
        let authorization_material =
            parse_optional_object_array(object, FIELD_AUTHORIZATION_MATERIAL)?;
        let offer_catalog = object
            .get(FIELD_OFFER_CATALOG)
            .map(|value| {
                OfferCatalogPath::from_value(value.clone())
                    .map_err(|error| HandshakeError::InvalidOfferCatalog(error.to_string()))
            })
            .transpose()?;
        let diagnostics = optional_object(object, FIELD_DIAGNOSTICS)?;
        let metadata = optional_object(object, FIELD_METADATA)?;

        Ok(Self {
            value,
            supported_lifecycle_versions,
            peer_binding,
            declared_domains,
            authorization_material,
            offer_catalog,
            diagnostics,
            metadata,
        })
    }

    /// Borrow the original handshake JSON object.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this handshake and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Validate this handshake's authority chain for one remote peer.
    pub fn validate_authority(
        &self,
        authenticated_peer_id: &PeerId,
        peer_authorization: PeerAuthorization,
        now: &str,
    ) -> Result<AuthorityChain, AuthorityChainError> {
        validate_authority_chain(AuthorityChainInput {
            peer_binding: Some(&self.peer_binding),
            authenticated_peer_id,
            peer_authorization,
            declared_domains: &self.declared_domains,
            now,
        })
    }

    /// Validate this handshake's authority chain and evaluate peer authorization from policy.
    pub fn validate_authority_with_authorization_policy(
        &self,
        authenticated_peer_id: &PeerId,
        peer_authorization_policy: PeerAuthorizationPolicy<'_>,
        now: &str,
    ) -> Result<AuthorityChain, AuthorityChainError> {
        validate_authority_chain_with_authorization_policy(AuthorityChainPolicyInput {
            peer_binding: Some(&self.peer_binding),
            authenticated_peer_id,
            peer_authorization_policy,
            declared_domains: &self.declared_domains,
            now,
        })
    }
}

fn declared_domain_value(declared_domain: &DeclaredDomain) -> Value {
    let mut object = Map::new();
    object.insert(
        "domain_id".to_owned(),
        Value::String(declared_domain.domain_id.clone()),
    );
    object.insert(
        "domain_declaration".to_owned(),
        declared_domain.domain_declaration.value().clone(),
    );
    if let Some(delegation) = &declared_domain.delegation {
        object.insert("delegation".to_owned(), delegation.value().clone());
    }
    if let Some(metadata) = &declared_domain.metadata {
        object.insert("metadata".to_owned(), metadata.clone());
    }
    Value::Object(object)
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, HandshakeError> {
    object
        .get(field)
        .ok_or(HandshakeError::MissingField { field })?
        .as_str()
        .ok_or(HandshakeError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn parse_lifecycle_versions(object: &Map<String, Value>) -> Result<Vec<String>, HandshakeError> {
    let values = object
        .get(FIELD_SUPPORTED_LIFECYCLE_VERSIONS)
        .ok_or(HandshakeError::MissingField {
            field: FIELD_SUPPORTED_LIFECYCLE_VERSIONS,
        })?
        .as_array()
        .ok_or(HandshakeError::InvalidFieldType {
            field: FIELD_SUPPORTED_LIFECYCLE_VERSIONS,
            expected: "an array",
        })?;

    if values.is_empty() {
        return Err(HandshakeError::EmptyLifecycleVersions);
    }

    let mut seen = HashSet::with_capacity(values.len());
    let mut versions = Vec::with_capacity(values.len());
    for value in values {
        let version = value.as_str().ok_or(HandshakeError::InvalidFieldType {
            field: FIELD_SUPPORTED_LIFECYCLE_VERSIONS,
            expected: "an array of strings",
        })?;
        if !seen.insert(version.to_owned()) {
            return Err(HandshakeError::DuplicateLifecycleVersion {
                version: version.to_owned(),
            });
        }
        versions.push(version.to_owned());
    }

    if !seen.contains(CLUSTER_LIFECYCLE_V1) {
        return Err(HandshakeError::MissingRequiredLifecycleVersion);
    }

    Ok(versions)
}

fn parse_declared_domains(
    object: &Map<String, Value>,
) -> Result<Vec<DeclaredDomain>, HandshakeError> {
    let values = object
        .get(FIELD_DECLARED_DOMAINS)
        .ok_or(HandshakeError::MissingField {
            field: FIELD_DECLARED_DOMAINS,
        })?
        .as_array()
        .ok_or(HandshakeError::InvalidFieldType {
            field: FIELD_DECLARED_DOMAINS,
            expected: "an array",
        })?;

    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            DeclaredDomain::from_value(value.clone())
                .map_err(|error| HandshakeError::InvalidDeclaredDomain { index, error })
        })
        .collect()
}

fn parse_optional_object_array(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Vec<Value>>, HandshakeError> {
    object
        .get(field)
        .map(|value| {
            let values = value.as_array().ok_or(HandshakeError::InvalidFieldType {
                field,
                expected: "an array",
            })?;
            values
                .iter()
                .map(|value| {
                    if value.is_object() {
                        Ok(value.clone())
                    } else {
                        Err(HandshakeError::InvalidFieldType {
                            field,
                            expected: "an array of objects",
                        })
                    }
                })
                .collect()
        })
        .transpose()
}

fn optional_object(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<Value>, HandshakeError> {
    object
        .get(field)
        .map(|value| {
            if value.is_object() {
                Ok(value.clone())
            } else {
                Err(HandshakeError::InvalidFieldType {
                    field,
                    expected: "an object",
                })
            }
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::{
        domain::{DelegationScope, DomainDeclaration, DomainDelegation, derive_domain_id},
        error,
        offer::{OFFER_CATALOG_PATH_TYPE, OFFER_CATALOG_PROTOCOL_ID, OFFER_CATALOG_VERSION},
    };
    use auki_identity::Wallet;
    use serde_json::json;
    use std::str::FromStr;

    const PEER_ID: &str = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";
    const NOW: &str = "2026-05-26T12:00:00Z";
    const NONCE: [u8; 16] = [7u8; 16];

    fn owner_wallet() -> std::sync::Arc<Wallet> {
        Wallet::from_seed(vec![3u8; 32]).expect("32-byte seed")
    }

    fn delegate_wallet() -> std::sync::Arc<Wallet> {
        Wallet::from_seed(vec![4u8; 32]).expect("32-byte seed")
    }

    fn peer_id() -> PeerId {
        PeerId::from_str(PEER_ID).expect("valid peer id")
    }

    fn peer_binding(wallet: &Wallet) -> PeerBinding {
        PeerBinding::create(wallet, &peer_id(), ISSUED_AT, None).unwrap()
    }

    fn direct_owner_declared_domain() -> DeclaredDomain {
        let declaration = DomainDeclaration::create(&owner_wallet(), &NONCE, None).unwrap();
        DeclaredDomain::new(
            derive_domain_id(&owner_wallet().public_key(), &NONCE),
            declaration,
            None,
        )
    }

    fn delegated_declared_domain() -> DeclaredDomain {
        let domain_id = derive_domain_id(&owner_wallet().public_key(), &NONCE);
        let declaration = DomainDeclaration::create(&owner_wallet(), &NONCE, None).unwrap();
        let delegation = DomainDelegation::create(
            &owner_wallet(),
            &domain_id,
            &delegate_wallet().public_key(),
            &peer_id(),
            &[DelegationScope::Serve],
            "2026-05-26T11:00:00Z",
            "2026-05-26T13:00:00Z",
            None,
        )
        .unwrap();
        DeclaredDomain::new(domain_id, declaration, Some(delegation))
    }

    #[test]
    fn create_and_parse_empty_declared_domains_handshake() {
        let binding = peer_binding(&owner_wallet());
        let handshake = PeerHandshake::create(binding.clone(), vec![]);

        let parsed = PeerHandshake::from_value(handshake.value().clone()).unwrap();

        assert_eq!(
            parsed.supported_lifecycle_versions,
            vec![CLUSTER_LIFECYCLE_V1.to_owned()]
        );
        assert_eq!(parsed.peer_binding.value(), binding.value());
        assert!(parsed.declared_domains.is_empty());
    }

    #[test]
    fn parse_and_validate_direct_owner_handshake_authority() {
        let binding = peer_binding(&owner_wallet());
        let declared = direct_owner_declared_domain();
        let handshake = PeerHandshake::create(binding, vec![declared]);
        let parsed = PeerHandshake::from_value(handshake.into_value()).unwrap();

        let authority = parsed
            .validate_authority(&peer_id(), PeerAuthorization::Authorized, NOW)
            .unwrap();

        assert_eq!(authority.accepted_served_domains.len(), 1);
        assert_eq!(authority.rejected_declared_domains, vec![]);

        let policy_authority = parsed
            .validate_authority_with_authorization_policy(
                &peer_id(),
                PeerAuthorizationPolicy::all(),
                NOW,
            )
            .unwrap();
        assert_eq!(policy_authority, authority);
    }

    #[test]
    fn parse_and_validate_delegated_handshake_authority() {
        let binding = peer_binding(&delegate_wallet());
        let declared = delegated_declared_domain();
        let handshake = PeerHandshake::create(binding, vec![declared]);
        let parsed = PeerHandshake::from_value(handshake.into_value()).unwrap();

        let authority = parsed
            .validate_authority(&peer_id(), PeerAuthorization::Authorized, NOW)
            .unwrap();

        assert_eq!(authority.accepted_served_domains.len(), 1);
        assert_eq!(authority.rejected_declared_domains, vec![]);
    }

    #[test]
    fn parse_preserves_optional_handshake_material() {
        let mut value = PeerHandshake::create(peer_binding(&owner_wallet()), vec![]).into_value();
        let object = value.as_object_mut().unwrap();
        object.insert(
            FIELD_AUTHORIZATION_MATERIAL.to_owned(),
            json!([{"type": "local.test", "value": true}]),
        );
        let catalog_path = json!({
            "catalog_version": OFFER_CATALOG_VERSION,
            "protocol_id": OFFER_CATALOG_PROTOCOL_ID,
            "type": OFFER_CATALOG_PATH_TYPE,
        });
        object.insert(FIELD_OFFER_CATALOG.to_owned(), catalog_path.clone());
        object.insert(FIELD_DIAGNOSTICS.to_owned(), json!({"ok": true}));
        object.insert(FIELD_METADATA.to_owned(), json!({"label": "node"}));

        let parsed = PeerHandshake::from_value(value).unwrap();

        assert_eq!(
            parsed.authorization_material,
            Some(vec![json!({"type": "local.test", "value": true})])
        );
        assert_eq!(
            parsed.offer_catalog.as_ref().map(|path| path.value()),
            Some(&catalog_path)
        );
        assert_eq!(parsed.diagnostics, Some(json!({"ok": true})));
        assert_eq!(parsed.metadata, Some(json!({"label": "node"})));
    }

    #[test]
    fn parse_rejects_duplicate_lifecycle_versions() {
        let mut value = PeerHandshake::create(peer_binding(&owner_wallet()), vec![]).into_value();
        value.as_object_mut().unwrap().insert(
            FIELD_SUPPORTED_LIFECYCLE_VERSIONS.to_owned(),
            json!([CLUSTER_LIFECYCLE_V1, CLUSTER_LIFECYCLE_V1]),
        );

        assert_eq!(
            PeerHandshake::from_value(value),
            Err(HandshakeError::DuplicateLifecycleVersion {
                version: CLUSTER_LIFECYCLE_V1.to_owned()
            })
        );
    }

    #[test]
    fn parse_rejects_missing_baseline_lifecycle_version() {
        let mut value = PeerHandshake::create(peer_binding(&owner_wallet()), vec![]).into_value();
        value.as_object_mut().unwrap().insert(
            FIELD_SUPPORTED_LIFECYCLE_VERSIONS.to_owned(),
            json!(["future.version"]),
        );

        let error = PeerHandshake::from_value(value).unwrap_err();
        assert_eq!(error, HandshakeError::MissingRequiredLifecycleVersion);
        assert_eq!(error.failure_code(), error::PROTOCOL_UNSUPPORTED_VERSION);
    }

    #[test]
    fn parse_rejects_malformed_peer_binding() {
        let mut value = PeerHandshake::create(peer_binding(&owner_wallet()), vec![]).into_value();
        value
            .as_object_mut()
            .unwrap()
            .insert(FIELD_PEER_BINDING.to_owned(), json!({}));

        assert!(matches!(
            PeerHandshake::from_value(value),
            Err(HandshakeError::InvalidPeerBinding(_))
        ));
    }

    #[test]
    fn parse_rejects_malformed_declared_domain() {
        let mut value = PeerHandshake::create(peer_binding(&owner_wallet()), vec![]).into_value();
        value
            .as_object_mut()
            .unwrap()
            .insert(FIELD_DECLARED_DOMAINS.to_owned(), json!([{}]));

        assert!(matches!(
            PeerHandshake::from_value(value),
            Err(HandshakeError::InvalidDeclaredDomain { index: 0, .. })
        ));
    }

    #[test]
    fn parse_rejects_malformed_offer_catalog_path() {
        let mut value = PeerHandshake::create(peer_binding(&owner_wallet()), vec![]).into_value();
        value
            .as_object_mut()
            .unwrap()
            .insert(FIELD_OFFER_CATALOG.to_owned(), json!({"type": "path"}));

        let error = PeerHandshake::from_value(value).unwrap_err();

        assert!(matches!(error, HandshakeError::InvalidOfferCatalog(_)));
        assert_eq!(error.failure_code(), error::HANDSHAKE_INVALID_MESSAGE);
    }
}
