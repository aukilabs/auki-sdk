//! Domain authority helpers for v1.

use super::{
    base64url::{self, Base64UrlError},
    identity::WALLET_SIGNATURE_SCHEME_ED25519,
};
use auki_identity::{PublicKey as WalletPublicKey, Signature, VerifyError, Wallet, verify};
use libp2p_identity::PeerId;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, fmt, str::FromStr};

/// V1 domain-id hash input object type.
pub const DOMAIN_ID_TYPE: &str = "auki.domain_id.v1";
/// V1 domain declaration object type.
pub const DOMAIN_DECLARATION_TYPE: &str = "auki.domain_declaration.v1";
/// V1 domain delegation object type.
pub const DOMAIN_DELEGATION_TYPE: &str = "auki.domain_delegation.v1";
/// Raw v1 domain id length in bytes.
pub const DOMAIN_ID_LEN: usize = 32;
/// Raw v1 domain nonce length in bytes.
pub const DOMAIN_NONCE_LEN: usize = 16;

const FIELD_TYPE: &str = "type";
const FIELD_WALLET_SIGNATURE_SCHEME: &str = "wallet_signature_scheme";
const FIELD_DOMAIN_OWNER_PUBLIC_KEY: &str = "domain_owner_public_key";
const FIELD_NONCE: &str = "nonce";
const FIELD_DOMAIN_ID: &str = "domain_id";
const FIELD_LABEL: &str = "label";
const FIELD_SIGNATURE: &str = "signature";
const FIELD_DELEGATE_WALLET_PUBLIC_KEY: &str = "delegate_wallet_public_key";
const FIELD_DELEGATE_PEER_ID: &str = "delegate_peer_id";
const FIELD_SCOPES: &str = "scopes";
const FIELD_VALID_FROM: &str = "valid_from";
const FIELD_EXPIRES_AT: &str = "expires_at";

/// V1 domain delegation scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DelegationScope {
    /// Peer may advertise the domain through reachability/discovery surfaces.
    Advertise,
    /// Peer may declare the domain during handshake and serve domain data.
    Serve,
}

impl DelegationScope {
    /// Return the RFC string value for this scope.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Advertise => "advertise",
            Self::Serve => "serve",
        }
    }
}

impl fmt::Display for DelegationScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DelegationScope {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "advertise" => Ok(Self::Advertise),
            "serve" => Ok(Self::Serve),
            _ => Err(DomainError::InvalidScope {
                scope: value.to_owned(),
            }),
        }
    }
}

/// A v1 domain-owner signed domain declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainDeclaration {
    value: Value,
}

/// A v1 domain-owner signed delegation to a peer wallet and peer id.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainDelegation {
    value: Value,
}

/// A successfully verified v1 domain declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedDomainDeclaration {
    /// Encoded v1 domain id.
    pub domain_id: String,
    /// Raw v1 domain id bytes.
    pub domain_id_bytes: [u8; DOMAIN_ID_LEN],
    /// Domain owner wallet public key.
    pub domain_owner_public_key: WalletPublicKey,
    /// Raw domain nonce bytes.
    pub nonce: [u8; DOMAIN_NONCE_LEN],
    /// Optional operator/application label.
    pub label: Option<String>,
}

/// A successfully verified v1 domain delegation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedDomainDelegation {
    /// Encoded delegated v1 domain id.
    pub domain_id: String,
    /// Raw delegated v1 domain id bytes.
    pub domain_id_bytes: [u8; DOMAIN_ID_LEN],
    /// Domain owner wallet public key that signed the delegation.
    pub domain_owner_public_key: WalletPublicKey,
    /// Delegate wallet public key from the remote peer binding.
    pub delegate_wallet_public_key: WalletPublicKey,
    /// Delegate libp2p peer id.
    pub delegate_peer_id: PeerId,
    /// Sorted, duplicate-free v1 delegation scopes.
    pub scopes: Vec<DelegationScope>,
    /// RFC3339 UTC timestamp at which the delegation starts.
    pub valid_from: String,
    /// RFC3339 UTC timestamp at which the delegation expires.
    pub expires_at: String,
    /// Optional operator/application label.
    pub label: Option<String>,
}

/// Errors produced by v1 domain helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// Domain JSON value was not an object.
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
    /// `wallet_signature_scheme` was unsupported.
    UnsupportedWalletSignatureScheme {
        /// Actual signature-scheme value.
        actual: String,
    },
    /// A base64url field was malformed or decoded to the wrong length.
    InvalidBase64Url {
        /// Field name.
        field: &'static str,
        /// Base64url decoding error.
        error: Base64UrlError,
    },
    /// `delegate_peer_id` could not be parsed by libp2p.
    InvalidPeerId {
        /// Actual peer id text.
        peer_id: String,
    },
    /// Delegation scopes array was empty.
    EmptyScopes,
    /// Delegation scope string is not a v1 scope.
    InvalidScope {
        /// Unsupported scope value.
        scope: String,
    },
    /// Delegation scopes array repeated a scope.
    DuplicateScope {
        /// Repeated scope.
        scope: DelegationScope,
    },
    /// Delegation scopes array was not in alphabetical string order.
    ScopesNotSorted {
        /// Scope that appeared before `current`.
        previous: DelegationScope,
        /// Scope that appeared out of order.
        current: DelegationScope,
    },
    /// Timestamp was not an RFC3339 UTC string with `Z` suffix.
    InvalidTimestamp {
        /// Field name.
        field: &'static str,
        /// Actual timestamp value.
        value: String,
    },
    /// Delegation validity window was empty or reversed.
    InvalidTimeWindow {
        /// `valid_from` timestamp.
        valid_from: String,
        /// `expires_at` timestamp.
        expires_at: String,
    },
    /// Declared domain id did not match the recomputed id.
    DomainIdMismatch {
        /// Domain id carried by the declaration.
        declared: String,
        /// Domain id recomputed from owner wallet public key and nonce.
        recomputed: String,
    },
    /// Delegation domain id did not match the domain being validated.
    DelegationDomainMismatch {
        /// Domain id carried by the delegation.
        delegated: String,
        /// Domain id being validated.
        expected: String,
    },
    /// Delegation owner public key did not match the verified declaration.
    DelegationOwnerMismatch {
        /// Owner public key carried by the delegation.
        delegated: WalletPublicKey,
        /// Owner public key from the verified declaration.
        expected: WalletPublicKey,
    },
    /// Delegation wallet public key did not match the verified peer binding.
    DelegateWalletMismatch {
        /// Delegate wallet public key carried by the delegation.
        delegated: WalletPublicKey,
        /// Wallet public key from the verified remote peer binding.
        expected: WalletPublicKey,
    },
    /// Delegation peer id did not match the transport-authenticated peer id.
    DelegatePeerIdMismatch {
        /// Peer id carried by the delegation.
        delegated: PeerId,
        /// Transport-authenticated peer id.
        expected: PeerId,
    },
    /// Delegation did not include a required scope.
    MissingScope {
        /// Required scope.
        scope: DelegationScope,
    },
    /// Delegation is not valid yet at the supplied current time.
    DelegationNotYetValid {
        /// `valid_from` timestamp.
        valid_from: String,
        /// Supplied current timestamp.
        now: String,
    },
    /// Delegation has expired at the supplied current time.
    DelegationExpired {
        /// `expires_at` timestamp.
        expires_at: String,
        /// Supplied current timestamp.
        now: String,
    },
    /// Signature verification failed.
    InvalidSignature,
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => write!(f, "domain object is not a json object"),
            Self::MissingField { field } => write!(f, "domain object missing field {field}"),
            Self::InvalidFieldType { field, expected } => {
                write!(f, "domain object field {field} is not {expected}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported domain object type {actual}")
            }
            Self::UnsupportedWalletSignatureScheme { actual } => {
                write!(f, "unsupported wallet signature scheme {actual}")
            }
            Self::InvalidBase64Url { field, error } => {
                write!(f, "invalid base64url in field {field}: {error}")
            }
            Self::InvalidPeerId { peer_id } => {
                write!(f, "invalid libp2p peer id {peer_id}")
            }
            Self::EmptyScopes => write!(f, "delegation scopes must be non-empty"),
            Self::InvalidScope { scope } => write!(f, "invalid delegation scope {scope}"),
            Self::DuplicateScope { scope } => {
                write!(f, "duplicate delegation scope {scope}")
            }
            Self::ScopesNotSorted { previous, current } => {
                write!(
                    f,
                    "delegation scopes are not sorted: {previous} before {current}"
                )
            }
            Self::InvalidTimestamp { field, value } => {
                write!(f, "invalid timestamp in field {field}: {value}")
            }
            Self::InvalidTimeWindow {
                valid_from,
                expires_at,
            } => {
                write!(
                    f,
                    "invalid delegation time window: valid_from {valid_from}, expires_at {expires_at}"
                )
            }
            Self::DomainIdMismatch {
                declared,
                recomputed,
            } => {
                write!(
                    f,
                    "domain id mismatch: declared {declared}, recomputed {recomputed}"
                )
            }
            Self::DelegationDomainMismatch {
                delegated,
                expected,
            } => {
                write!(
                    f,
                    "delegation domain id {delegated} does not match expected domain id {expected}"
                )
            }
            Self::DelegationOwnerMismatch { .. } => {
                write!(f, "delegation owner public key does not match declaration")
            }
            Self::DelegateWalletMismatch { .. } => {
                write!(
                    f,
                    "delegation wallet public key does not match peer binding"
                )
            }
            Self::DelegatePeerIdMismatch {
                delegated,
                expected,
            } => {
                write!(
                    f,
                    "delegation peer id {delegated} does not match authenticated peer id {expected}"
                )
            }
            Self::MissingScope { scope } => {
                write!(f, "delegation missing required scope {scope}")
            }
            Self::DelegationNotYetValid { valid_from, now } => {
                write!(f, "delegation valid_from {valid_from} is after now {now}")
            }
            Self::DelegationExpired { expires_at, now } => {
                write!(
                    f,
                    "delegation expires_at {expires_at} is at or before now {now}"
                )
            }
            Self::InvalidSignature => write!(f, "domain signature does not verify"),
        }
    }
}

impl std::error::Error for DomainError {}

/// Derive a v1 domain id from raw owner wallet public-key bytes and nonce.
pub fn derive_domain_id(
    domain_owner_public_key: &WalletPublicKey,
    nonce: &[u8; DOMAIN_NONCE_LEN],
) -> String {
    let domain_owner_public_key = base64url::encode(&domain_owner_public_key.0);
    let nonce = base64url::encode(nonce);
    derive_domain_id_from_encoded(&domain_owner_public_key, &nonce)
        .expect("encoded public key and nonce have canonical fixed lengths")
}

/// Derive a v1 domain id from encoded public-key and nonce strings.
///
/// The input strings are validated as canonical base64url without padding and
/// used byte-for-byte in the RFC-0007 hash input.
pub fn derive_domain_id_from_encoded(
    domain_owner_public_key: &str,
    nonce: &str,
) -> Result<String, DomainError> {
    let canonical_json = domain_id_hash_input_canonical_json(domain_owner_public_key, nonce)?;
    let digest = Sha256::digest(&canonical_json);
    Ok(base64url::encode(&digest))
}

/// Return the RFC-0007 canonical JSON bytes that are hashed for a domain id.
pub fn domain_id_hash_input_canonical_json(
    domain_owner_public_key: &str,
    nonce: &str,
) -> Result<Vec<u8>, DomainError> {
    validate_domain_owner_public_key(domain_owner_public_key)?;
    validate_domain_nonce(nonce)?;
    Ok(auki_jcs::canonicalize(&domain_id_hash_input_value(
        domain_owner_public_key,
        nonce,
    )))
}

/// Decode a v1 domain id string into its raw 32-byte digest.
pub fn decode_domain_id(domain_id: &str) -> Result<[u8; DOMAIN_ID_LEN], DomainError> {
    base64url::decode_exact::<DOMAIN_ID_LEN>(domain_id).map_err(|error| {
        DomainError::InvalidBase64Url {
            field: FIELD_DOMAIN_ID,
            error,
        }
    })
}

/// Decode a v1 domain nonce string into its raw 16-byte value.
pub fn decode_domain_nonce(nonce: &str) -> Result<[u8; DOMAIN_NONCE_LEN], DomainError> {
    base64url::decode_exact::<DOMAIN_NONCE_LEN>(nonce).map_err(|error| {
        DomainError::InvalidBase64Url {
            field: FIELD_NONCE,
            error,
        }
    })
}

/// Decode a domain-owner wallet public key string into raw Ed25519 bytes.
pub fn decode_domain_owner_public_key(
    domain_owner_public_key: &str,
) -> Result<WalletPublicKey, DomainError> {
    decode_wallet_public_key_field(FIELD_DOMAIN_OWNER_PUBLIC_KEY, domain_owner_public_key)
}

/// Decode a delegate wallet public key string into raw Ed25519 bytes.
pub fn decode_delegate_wallet_public_key(
    delegate_wallet_public_key: &str,
) -> Result<WalletPublicKey, DomainError> {
    decode_wallet_public_key_field(FIELD_DELEGATE_WALLET_PUBLIC_KEY, delegate_wallet_public_key)
}

fn decode_wallet_public_key_field(
    field: &'static str,
    value: &str,
) -> Result<WalletPublicKey, DomainError> {
    base64url::decode_exact::<32>(value)
        .map(WalletPublicKey)
        .map_err(|error| DomainError::InvalidBase64Url { field, error })
}

impl DomainDeclaration {
    /// Create and sign a v1 domain declaration.
    pub fn create(
        domain_owner_wallet: &Wallet,
        nonce: &[u8; DOMAIN_NONCE_LEN],
        label: Option<&str>,
    ) -> Result<Self, DomainError> {
        let domain_owner_public_key = base64url::encode(&domain_owner_wallet.public_key().0);
        let nonce = base64url::encode(nonce);
        let domain_id = derive_domain_id_from_encoded(&domain_owner_public_key, &nonce)?;

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(DOMAIN_DECLARATION_TYPE.to_owned()),
        );
        object.insert(
            FIELD_WALLET_SIGNATURE_SCHEME.to_owned(),
            Value::String(WALLET_SIGNATURE_SCHEME_ED25519.to_owned()),
        );
        object.insert(FIELD_DOMAIN_ID.to_owned(), Value::String(domain_id));
        object.insert(
            FIELD_DOMAIN_OWNER_PUBLIC_KEY.to_owned(),
            Value::String(domain_owner_public_key),
        );
        object.insert(FIELD_NONCE.to_owned(), Value::String(nonce));
        if let Some(label) = label {
            object.insert(FIELD_LABEL.to_owned(), Value::String(label.to_owned()));
        }

        let signed_value = Value::Object(object.clone());
        let signed_bytes = auki_jcs::canonicalize(&signed_value);
        let signature = domain_owner_wallet.sign(&signed_bytes);
        object.insert(
            FIELD_SIGNATURE.to_owned(),
            Value::String(base64url::encode(&signature.0)),
        );

        Self::from_value(Value::Object(object))
    }

    /// Parse a v1 domain declaration from a JSON value and validate its shape.
    pub fn from_value(value: Value) -> Result<Self, DomainError> {
        let declaration = Self { value };
        declaration.validate_shape()?;
        Ok(declaration)
    }

    /// Borrow the original JSON object, including fields unknown to this crate.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this declaration and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Return the encoded v1 domain id.
    pub fn domain_id(&self) -> Result<&str, DomainError> {
        required_string(self.object()?, FIELD_DOMAIN_ID)
    }

    /// Return the encoded domain-owner wallet public key.
    pub fn domain_owner_public_key(&self) -> Result<&str, DomainError> {
        required_string(self.object()?, FIELD_DOMAIN_OWNER_PUBLIC_KEY)
    }

    /// Return the encoded domain nonce.
    pub fn nonce(&self) -> Result<&str, DomainError> {
        required_string(self.object()?, FIELD_NONCE)
    }

    /// Return the optional label string.
    pub fn label(&self) -> Result<Option<&str>, DomainError> {
        optional_string(self.object()?, FIELD_LABEL)
    }

    /// Recompute signed bytes for this declaration using RFC-0003 rules.
    pub fn signed_bytes(&self) -> Result<Vec<u8>, DomainError> {
        Ok(auki_jcs::canonicalize(&self.signed_value()?))
    }

    /// Verify this domain declaration.
    pub fn verify(&self) -> Result<VerifiedDomainDeclaration, DomainError> {
        self.validate_shape()?;

        let object = self.object()?;
        let domain_id = required_string(object, FIELD_DOMAIN_ID)?;
        let domain_id_bytes = decode_domain_id(domain_id)?;
        let domain_owner_public_key = decode_domain_owner_public_key(required_string(
            object,
            FIELD_DOMAIN_OWNER_PUBLIC_KEY,
        )?)?;
        let nonce = decode_domain_nonce(required_string(object, FIELD_NONCE)?)?;
        let signature = decode_signature(object)?;
        let signed_bytes = self.signed_bytes()?;

        verify(&domain_owner_public_key, &signed_bytes, &signature).map_err(map_verify_error)?;

        Ok(VerifiedDomainDeclaration {
            domain_id: domain_id.to_owned(),
            domain_id_bytes,
            domain_owner_public_key,
            nonce,
            label: optional_string(object, FIELD_LABEL)?.map(ToOwned::to_owned),
        })
    }

    fn validate_shape(&self) -> Result<(), DomainError> {
        let object = self.object()?;

        let type_value = required_string(object, FIELD_TYPE)?;
        if type_value != DOMAIN_DECLARATION_TYPE {
            return Err(DomainError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let scheme = required_string(object, FIELD_WALLET_SIGNATURE_SCHEME)?;
        if scheme != WALLET_SIGNATURE_SCHEME_ED25519 {
            return Err(DomainError::UnsupportedWalletSignatureScheme {
                actual: scheme.to_owned(),
            });
        }

        let domain_id = required_string(object, FIELD_DOMAIN_ID)?;
        decode_domain_id(domain_id)?;
        let domain_owner_public_key = required_string(object, FIELD_DOMAIN_OWNER_PUBLIC_KEY)?;
        decode_domain_owner_public_key(domain_owner_public_key)?;
        let nonce = required_string(object, FIELD_NONCE)?;
        decode_domain_nonce(nonce)?;
        let recomputed = derive_domain_id_from_encoded(domain_owner_public_key, nonce)?;
        if recomputed != domain_id {
            return Err(DomainError::DomainIdMismatch {
                declared: domain_id.to_owned(),
                recomputed,
            });
        }
        optional_string(object, FIELD_LABEL)?;
        decode_signature(object)?;

        Ok(())
    }

    fn object(&self) -> Result<&Map<String, Value>, DomainError> {
        self.value.as_object().ok_or(DomainError::NotObject)
    }

    fn signed_value(&self) -> Result<Value, DomainError> {
        let mut object = self.object()?.clone();
        object
            .remove(FIELD_SIGNATURE)
            .ok_or(DomainError::MissingField {
                field: FIELD_SIGNATURE,
            })?;
        Ok(Value::Object(object))
    }
}

impl DomainDelegation {
    /// Create and sign a v1 domain delegation.
    pub fn create(
        domain_owner_wallet: &Wallet,
        domain_id: &str,
        delegate_wallet_public_key: &WalletPublicKey,
        delegate_peer_id: &PeerId,
        scopes: &[DelegationScope],
        valid_from: &str,
        expires_at: &str,
        label: Option<&str>,
    ) -> Result<Self, DomainError> {
        decode_domain_id(domain_id)?;
        let scopes = sorted_unique_scopes_for_signing(scopes)?;
        validate_time_window(valid_from, expires_at)?;

        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(DOMAIN_DELEGATION_TYPE.to_owned()),
        );
        object.insert(
            FIELD_WALLET_SIGNATURE_SCHEME.to_owned(),
            Value::String(WALLET_SIGNATURE_SCHEME_ED25519.to_owned()),
        );
        object.insert(
            FIELD_DOMAIN_ID.to_owned(),
            Value::String(domain_id.to_owned()),
        );
        object.insert(
            FIELD_DOMAIN_OWNER_PUBLIC_KEY.to_owned(),
            Value::String(base64url::encode(&domain_owner_wallet.public_key().0)),
        );
        object.insert(
            FIELD_DELEGATE_WALLET_PUBLIC_KEY.to_owned(),
            Value::String(base64url::encode(&delegate_wallet_public_key.0)),
        );
        object.insert(
            FIELD_DELEGATE_PEER_ID.to_owned(),
            Value::String(delegate_peer_id.to_string()),
        );
        object.insert(
            FIELD_SCOPES.to_owned(),
            Value::Array(
                scopes
                    .iter()
                    .map(|scope| Value::String(scope.as_str().to_owned()))
                    .collect(),
            ),
        );
        object.insert(
            FIELD_VALID_FROM.to_owned(),
            Value::String(valid_from.to_owned()),
        );
        object.insert(
            FIELD_EXPIRES_AT.to_owned(),
            Value::String(expires_at.to_owned()),
        );
        if let Some(label) = label {
            object.insert(FIELD_LABEL.to_owned(), Value::String(label.to_owned()));
        }

        let signed_value = Value::Object(object.clone());
        let signed_bytes = auki_jcs::canonicalize(&signed_value);
        let signature = domain_owner_wallet.sign(&signed_bytes);
        object.insert(
            FIELD_SIGNATURE.to_owned(),
            Value::String(base64url::encode(&signature.0)),
        );

        Self::from_value(Value::Object(object))
    }

    /// Parse a v1 domain delegation from a JSON value and validate its shape.
    pub fn from_value(value: Value) -> Result<Self, DomainError> {
        let delegation = Self { value };
        delegation.validate_shape()?;
        Ok(delegation)
    }

    /// Borrow the original JSON object, including fields unknown to this crate.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Consume this delegation and return the original JSON object.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Return the encoded delegated v1 domain id.
    pub fn domain_id(&self) -> Result<&str, DomainError> {
        required_string(self.object()?, FIELD_DOMAIN_ID)
    }

    /// Return the encoded domain-owner wallet public key.
    pub fn domain_owner_public_key(&self) -> Result<&str, DomainError> {
        required_string(self.object()?, FIELD_DOMAIN_OWNER_PUBLIC_KEY)
    }

    /// Return the encoded delegate wallet public key.
    pub fn delegate_wallet_public_key(&self) -> Result<&str, DomainError> {
        required_string(self.object()?, FIELD_DELEGATE_WALLET_PUBLIC_KEY)
    }

    /// Return the delegate peer id string.
    pub fn delegate_peer_id(&self) -> Result<&str, DomainError> {
        required_string(self.object()?, FIELD_DELEGATE_PEER_ID)
    }

    /// Return the delegation `valid_from` timestamp.
    pub fn valid_from(&self) -> Result<&str, DomainError> {
        required_string(self.object()?, FIELD_VALID_FROM)
    }

    /// Return the delegation `expires_at` timestamp.
    pub fn expires_at(&self) -> Result<&str, DomainError> {
        required_string(self.object()?, FIELD_EXPIRES_AT)
    }

    /// Return the optional label string.
    pub fn label(&self) -> Result<Option<&str>, DomainError> {
        optional_string(self.object()?, FIELD_LABEL)
    }

    /// Recompute signed bytes for this delegation using RFC-0003 rules.
    pub fn signed_bytes(&self) -> Result<Vec<u8>, DomainError> {
        Ok(auki_jcs::canonicalize(&self.signed_value()?))
    }

    /// Verify this delegation's shape, domain-owner signature, and time-window shape.
    pub fn verify(&self) -> Result<VerifiedDomainDelegation, DomainError> {
        self.validate_shape()?;

        let object = self.object()?;
        let domain_id = required_string(object, FIELD_DOMAIN_ID)?;
        let domain_id_bytes = decode_domain_id(domain_id)?;
        let domain_owner_public_key = decode_domain_owner_public_key(required_string(
            object,
            FIELD_DOMAIN_OWNER_PUBLIC_KEY,
        )?)?;
        let delegate_wallet_public_key = decode_delegate_wallet_public_key(required_string(
            object,
            FIELD_DELEGATE_WALLET_PUBLIC_KEY,
        )?)?;
        let delegate_peer_id = parse_peer_id(required_string(object, FIELD_DELEGATE_PEER_ID)?)?;
        let scopes = parse_scopes(object)?;
        let valid_from = required_string(object, FIELD_VALID_FROM)?;
        let expires_at = required_string(object, FIELD_EXPIRES_AT)?;
        let signature = decode_signature(object)?;
        let signed_bytes = self.signed_bytes()?;

        verify(&domain_owner_public_key, &signed_bytes, &signature).map_err(map_verify_error)?;

        Ok(VerifiedDomainDelegation {
            domain_id: domain_id.to_owned(),
            domain_id_bytes,
            domain_owner_public_key,
            delegate_wallet_public_key,
            delegate_peer_id,
            scopes,
            valid_from: valid_from.to_owned(),
            expires_at: expires_at.to_owned(),
            label: optional_string(object, FIELD_LABEL)?.map(ToOwned::to_owned),
        })
    }

    /// Verify this delegation against an authority-chain context and current time.
    pub fn verify_for_authority(
        &self,
        expected_domain_id: &str,
        expected_domain_owner_public_key: &WalletPublicKey,
        expected_delegate_wallet_public_key: &WalletPublicKey,
        expected_delegate_peer_id: &PeerId,
        required_scope: DelegationScope,
        now: &str,
    ) -> Result<VerifiedDomainDelegation, DomainError> {
        let verified = self.verify()?;
        validate_rfc3339_z_timestamp(FIELD_VALID_FROM, now)?;

        if verified.domain_id != expected_domain_id {
            return Err(DomainError::DelegationDomainMismatch {
                delegated: verified.domain_id,
                expected: expected_domain_id.to_owned(),
            });
        }
        if verified.domain_owner_public_key != *expected_domain_owner_public_key {
            return Err(DomainError::DelegationOwnerMismatch {
                delegated: verified.domain_owner_public_key,
                expected: *expected_domain_owner_public_key,
            });
        }
        if verified.delegate_wallet_public_key != *expected_delegate_wallet_public_key {
            return Err(DomainError::DelegateWalletMismatch {
                delegated: verified.delegate_wallet_public_key,
                expected: *expected_delegate_wallet_public_key,
            });
        }
        if verified.delegate_peer_id != *expected_delegate_peer_id {
            return Err(DomainError::DelegatePeerIdMismatch {
                delegated: verified.delegate_peer_id,
                expected: *expected_delegate_peer_id,
            });
        }
        if !verified.scopes.contains(&required_scope) {
            return Err(DomainError::MissingScope {
                scope: required_scope,
            });
        }
        if compare_rfc3339_z_timestamps(now, &verified.valid_from)? == Ordering::Less {
            return Err(DomainError::DelegationNotYetValid {
                valid_from: verified.valid_from,
                now: now.to_owned(),
            });
        }
        if compare_rfc3339_z_timestamps(now, &verified.expires_at)? != Ordering::Less {
            return Err(DomainError::DelegationExpired {
                expires_at: verified.expires_at,
                now: now.to_owned(),
            });
        }

        Ok(verified)
    }

    fn validate_shape(&self) -> Result<(), DomainError> {
        let object = self.object()?;

        let type_value = required_string(object, FIELD_TYPE)?;
        if type_value != DOMAIN_DELEGATION_TYPE {
            return Err(DomainError::UnsupportedType {
                actual: type_value.to_owned(),
            });
        }

        let scheme = required_string(object, FIELD_WALLET_SIGNATURE_SCHEME)?;
        if scheme != WALLET_SIGNATURE_SCHEME_ED25519 {
            return Err(DomainError::UnsupportedWalletSignatureScheme {
                actual: scheme.to_owned(),
            });
        }

        decode_domain_id(required_string(object, FIELD_DOMAIN_ID)?)?;
        decode_domain_owner_public_key(required_string(object, FIELD_DOMAIN_OWNER_PUBLIC_KEY)?)?;
        decode_delegate_wallet_public_key(required_string(
            object,
            FIELD_DELEGATE_WALLET_PUBLIC_KEY,
        )?)?;
        parse_peer_id(required_string(object, FIELD_DELEGATE_PEER_ID)?)?;
        parse_scopes(object)?;
        validate_time_window(
            required_string(object, FIELD_VALID_FROM)?,
            required_string(object, FIELD_EXPIRES_AT)?,
        )?;
        optional_string(object, FIELD_LABEL)?;
        decode_signature(object)?;

        Ok(())
    }

    fn object(&self) -> Result<&Map<String, Value>, DomainError> {
        self.value.as_object().ok_or(DomainError::NotObject)
    }

    fn signed_value(&self) -> Result<Value, DomainError> {
        let mut object = self.object()?.clone();
        object
            .remove(FIELD_SIGNATURE)
            .ok_or(DomainError::MissingField {
                field: FIELD_SIGNATURE,
            })?;
        Ok(Value::Object(object))
    }
}

fn decode_signature(object: &Map<String, Value>) -> Result<Signature, DomainError> {
    let value = required_string(object, FIELD_SIGNATURE)?;
    base64url::decode_exact::<64>(value)
        .map(Signature)
        .map_err(|error| DomainError::InvalidBase64Url {
            field: FIELD_SIGNATURE,
            error,
        })
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, DomainError> {
    object
        .get(field)
        .ok_or(DomainError::MissingField { field })?
        .as_str()
        .ok_or(DomainError::InvalidFieldType {
            field,
            expected: "a string",
        })
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<Option<&'a str>, DomainError> {
    object
        .get(field)
        .map(|value| {
            value.as_str().ok_or(DomainError::InvalidFieldType {
                field,
                expected: "a string",
            })
        })
        .transpose()
}

fn parse_peer_id(peer_id: &str) -> Result<PeerId, DomainError> {
    PeerId::from_str(peer_id).map_err(|_| DomainError::InvalidPeerId {
        peer_id: peer_id.to_owned(),
    })
}

fn parse_scopes(object: &Map<String, Value>) -> Result<Vec<DelegationScope>, DomainError> {
    let values = object
        .get(FIELD_SCOPES)
        .ok_or(DomainError::MissingField {
            field: FIELD_SCOPES,
        })?
        .as_array()
        .ok_or(DomainError::InvalidFieldType {
            field: FIELD_SCOPES,
            expected: "an array",
        })?;

    if values.is_empty() {
        return Err(DomainError::EmptyScopes);
    }

    let mut scopes: Vec<DelegationScope> = Vec::with_capacity(values.len());
    for value in values {
        let scope = value.as_str().ok_or(DomainError::InvalidFieldType {
            field: FIELD_SCOPES,
            expected: "an array of strings",
        })?;
        let scope = DelegationScope::from_str(scope)?;

        if let Some(previous) = scopes.last().copied() {
            match previous.as_str().cmp(scope.as_str()) {
                Ordering::Equal => return Err(DomainError::DuplicateScope { scope }),
                Ordering::Greater => {
                    return Err(DomainError::ScopesNotSorted {
                        previous,
                        current: scope,
                    });
                }
                Ordering::Less => {}
            }
        }

        scopes.push(scope);
    }

    Ok(scopes)
}

fn sorted_unique_scopes_for_signing(
    scopes: &[DelegationScope],
) -> Result<Vec<DelegationScope>, DomainError> {
    if scopes.is_empty() {
        return Err(DomainError::EmptyScopes);
    }

    let mut scopes = scopes.to_vec();
    scopes.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    for pair in scopes.windows(2) {
        if pair[0] == pair[1] {
            return Err(DomainError::DuplicateScope { scope: pair[0] });
        }
    }
    Ok(scopes)
}

fn map_verify_error(_error: VerifyError) -> DomainError {
    DomainError::InvalidSignature
}

fn validate_time_window(valid_from: &str, expires_at: &str) -> Result<(), DomainError> {
    validate_rfc3339_z_timestamp(FIELD_VALID_FROM, valid_from)?;
    validate_rfc3339_z_timestamp(FIELD_EXPIRES_AT, expires_at)?;
    if compare_rfc3339_z_timestamps(valid_from, expires_at)? == Ordering::Less {
        Ok(())
    } else {
        Err(DomainError::InvalidTimeWindow {
            valid_from: valid_from.to_owned(),
            expires_at: expires_at.to_owned(),
        })
    }
}

fn validate_rfc3339_z_timestamp(field: &'static str, value: &str) -> Result<(), DomainError> {
    parse_rfc3339_z_timestamp(value)
        .map(|_| ())
        .ok_or_else(|| DomainError::InvalidTimestamp {
            field,
            value: value.to_owned(),
        })
}

fn compare_rfc3339_z_timestamps(left: &str, right: &str) -> Result<Ordering, DomainError> {
    let left_parsed =
        parse_rfc3339_z_timestamp(left).ok_or_else(|| DomainError::InvalidTimestamp {
            field: FIELD_VALID_FROM,
            value: left.to_owned(),
        })?;
    let right_parsed =
        parse_rfc3339_z_timestamp(right).ok_or_else(|| DomainError::InvalidTimestamp {
            field: FIELD_EXPIRES_AT,
            value: right.to_owned(),
        })?;
    Ok(left_parsed.cmp(&right_parsed))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rfc3339ZTimestamp<'a> {
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    fraction: &'a str,
}

impl Ord for Rfc3339ZTimestamp<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
        )
            .cmp(&(
                other.year,
                other.month,
                other.day,
                other.hour,
                other.minute,
                other.second,
            ))
            .then_with(|| compare_fractional_seconds(self.fraction, other.fraction))
    }
}

impl PartialOrd for Rfc3339ZTimestamp<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
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

fn parse_fixed_digits(value: &str) -> Option<u32> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

fn compare_fractional_seconds(left: &str, right: &str) -> Ordering {
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_digit = left.as_bytes().get(index).copied().unwrap_or(b'0');
        let right_digit = right.as_bytes().get(index).copied().unwrap_or(b'0');
        match left_digit.cmp(&right_digit) {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    Ordering::Equal
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

fn validate_domain_owner_public_key(domain_owner_public_key: &str) -> Result<(), DomainError> {
    decode_domain_owner_public_key(domain_owner_public_key).map(|_| ())
}

fn validate_domain_nonce(nonce: &str) -> Result<(), DomainError> {
    decode_domain_nonce(nonce).map(|_| ())
}

fn domain_id_hash_input_value(domain_owner_public_key: &str, nonce: &str) -> Value {
    let mut object = Map::new();
    object.insert(
        FIELD_TYPE.to_owned(),
        Value::String(DOMAIN_ID_TYPE.to_owned()),
    );
    object.insert(
        FIELD_WALLET_SIGNATURE_SCHEME.to_owned(),
        Value::String(WALLET_SIGNATURE_SCHEME_ED25519.to_owned()),
    );
    object.insert(
        FIELD_DOMAIN_OWNER_PUBLIC_KEY.to_owned(),
        Value::String(domain_owner_public_key.to_owned()),
    );
    object.insert(FIELD_NONCE.to_owned(), Value::String(nonce.to_owned()));
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DOMAIN_OWNER_PUBLIC_KEY: [u8; 32] = [
        0x10, 0x80, 0x63, 0x3b, 0xcb, 0x57, 0xba, 0xc0, 0x66, 0xcf, 0x84, 0x46, 0xe2, 0xb7, 0xae,
        0x71, 0x15, 0x71, 0xcb, 0x04, 0xbe, 0x0b, 0x46, 0xbd, 0xaf, 0x03, 0x14, 0x63, 0x17, 0xbf,
        0xe7, 0x07,
    ];
    const DOMAIN_OWNER_PUBLIC_KEY_B64: &str = "EIBjO8tXusBmz4RG4reucRVxywS-C0a9rwMUYxe_5wc";
    const NONCE: [u8; DOMAIN_NONCE_LEN] = [7u8; DOMAIN_NONCE_LEN];
    const NONCE_B64: &str = "BwcHBwcHBwcHBwcHBwcHBw";
    const DOMAIN_ID: &str = "B5ahQnTLt-VeaGP7pn1c7B_5TFiF5u5t1IuokqQpUYU";
    const CANONICAL_HASH_INPUT: &[u8] = br#"{"domain_owner_public_key":"EIBjO8tXusBmz4RG4reucRVxywS-C0a9rwMUYxe_5wc","nonce":"BwcHBwcHBwcHBwcHBwcHBw","type":"auki.domain_id.v1","wallet_signature_scheme":"ed25519"}"#;
    const DELEGATE_PEER_ID: &str = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
    const OTHER_PEER_ID: &str = "12D3KooWFU1bqozGMWdqN2Ckh2YHNbr9n5Lypw6uJrNkbm2ptVbF";
    const VALID_FROM: &str = "2026-05-26T12:00:00Z";
    const EXPIRES_AT: &str = "2026-05-26T13:00:00Z";
    const NOW: &str = "2026-05-26T12:30:00Z";

    fn wallet() -> std::sync::Arc<Wallet> {
        Wallet::from_seed(vec![3u8; 32]).expect("32-byte seed")
    }

    fn delegate_wallet() -> std::sync::Arc<Wallet> {
        Wallet::from_seed(vec![4u8; 32]).expect("32-byte seed")
    }

    fn delegate_peer_id() -> PeerId {
        PeerId::from_str(DELEGATE_PEER_ID).expect("valid peer id")
    }

    fn other_peer_id() -> PeerId {
        PeerId::from_str(OTHER_PEER_ID).expect("valid peer id")
    }

    fn domain_id_for_wallet() -> String {
        derive_domain_id(&wallet().public_key(), &NONCE)
    }

    fn signed_declaration_value(extra: Option<(&str, Value)>) -> Value {
        let wallet = wallet();
        let nonce = base64url::encode(&NONCE);
        let domain_owner_public_key = base64url::encode(&wallet.public_key().0);
        let domain_id = derive_domain_id_from_encoded(&domain_owner_public_key, &nonce).unwrap();
        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(DOMAIN_DECLARATION_TYPE.to_owned()),
        );
        object.insert(
            FIELD_WALLET_SIGNATURE_SCHEME.to_owned(),
            Value::String(WALLET_SIGNATURE_SCHEME_ED25519.to_owned()),
        );
        object.insert(FIELD_DOMAIN_ID.to_owned(), Value::String(domain_id));
        object.insert(
            FIELD_DOMAIN_OWNER_PUBLIC_KEY.to_owned(),
            Value::String(domain_owner_public_key),
        );
        object.insert(FIELD_NONCE.to_owned(), Value::String(nonce));
        if let Some((field, value)) = extra {
            object.insert(field.to_owned(), value);
        }

        let signed_value = Value::Object(object.clone());
        let signed_bytes = auki_jcs::canonicalize(&signed_value);
        object.insert(
            FIELD_SIGNATURE.to_owned(),
            Value::String(base64url::encode(&wallet.sign(&signed_bytes).0)),
        );
        Value::Object(object)
    }

    fn signed_delegation_value(
        scopes: &[&str],
        valid_from: &str,
        expires_at: &str,
        extra: Option<(&str, Value)>,
    ) -> Value {
        let owner = wallet();
        let delegate = delegate_wallet();
        let mut object = Map::new();
        object.insert(
            FIELD_TYPE.to_owned(),
            Value::String(DOMAIN_DELEGATION_TYPE.to_owned()),
        );
        object.insert(
            FIELD_WALLET_SIGNATURE_SCHEME.to_owned(),
            Value::String(WALLET_SIGNATURE_SCHEME_ED25519.to_owned()),
        );
        object.insert(
            FIELD_DOMAIN_ID.to_owned(),
            Value::String(domain_id_for_wallet()),
        );
        object.insert(
            FIELD_DOMAIN_OWNER_PUBLIC_KEY.to_owned(),
            Value::String(base64url::encode(&owner.public_key().0)),
        );
        object.insert(
            FIELD_DELEGATE_WALLET_PUBLIC_KEY.to_owned(),
            Value::String(base64url::encode(&delegate.public_key().0)),
        );
        object.insert(
            FIELD_DELEGATE_PEER_ID.to_owned(),
            Value::String(DELEGATE_PEER_ID.to_owned()),
        );
        object.insert(
            FIELD_SCOPES.to_owned(),
            Value::Array(
                scopes
                    .iter()
                    .map(|scope| Value::String((*scope).to_owned()))
                    .collect(),
            ),
        );
        object.insert(
            FIELD_VALID_FROM.to_owned(),
            Value::String(valid_from.to_owned()),
        );
        object.insert(
            FIELD_EXPIRES_AT.to_owned(),
            Value::String(expires_at.to_owned()),
        );
        if let Some((field, value)) = extra {
            object.insert(field.to_owned(), value);
        }

        let signed_value = Value::Object(object.clone());
        let signed_bytes = auki_jcs::canonicalize(&signed_value);
        object.insert(
            FIELD_SIGNATURE.to_owned(),
            Value::String(base64url::encode(&owner.sign(&signed_bytes).0)),
        );
        Value::Object(object)
    }

    #[test]
    fn locked_domain_id_derivation_vector() {
        let public_key = WalletPublicKey(DOMAIN_OWNER_PUBLIC_KEY);

        assert_eq!(
            base64url::encode(&public_key.0),
            DOMAIN_OWNER_PUBLIC_KEY_B64
        );
        assert_eq!(base64url::encode(&NONCE), NONCE_B64);
        assert_eq!(derive_domain_id(&public_key, &NONCE), DOMAIN_ID);
        assert_eq!(
            domain_id_hash_input_canonical_json(DOMAIN_OWNER_PUBLIC_KEY_B64, NONCE_B64).unwrap(),
            CANONICAL_HASH_INPUT
        );
    }

    #[test]
    fn derive_domain_id_from_encoded_matches_raw_derivation() {
        let public_key = WalletPublicKey(DOMAIN_OWNER_PUBLIC_KEY);

        assert_eq!(
            derive_domain_id_from_encoded(DOMAIN_OWNER_PUBLIC_KEY_B64, NONCE_B64).unwrap(),
            derive_domain_id(&public_key, &NONCE)
        );
    }

    #[test]
    fn decode_domain_id_requires_32_bytes() {
        assert_eq!(
            decode_domain_id("Zm9v"),
            Err(DomainError::InvalidBase64Url {
                field: FIELD_DOMAIN_ID,
                error: Base64UrlError::DecodedLengthMismatch {
                    expected: DOMAIN_ID_LEN,
                    actual: 3
                }
            })
        );
    }

    #[test]
    fn domain_id_input_rejects_non_canonical_public_key() {
        assert!(matches!(
            domain_id_hash_input_canonical_json("Zg=", NONCE_B64),
            Err(DomainError::InvalidBase64Url {
                field: FIELD_DOMAIN_OWNER_PUBLIC_KEY,
                ..
            })
        ));
    }

    #[test]
    fn domain_id_input_rejects_wrong_nonce_length() {
        assert_eq!(
            derive_domain_id_from_encoded(DOMAIN_OWNER_PUBLIC_KEY_B64, "Zm9v"),
            Err(DomainError::InvalidBase64Url {
                field: FIELD_NONCE,
                error: Base64UrlError::DecodedLengthMismatch {
                    expected: DOMAIN_NONCE_LEN,
                    actual: 3
                }
            })
        );
    }

    #[test]
    fn create_and_verify_domain_declaration() {
        let wallet = wallet();
        let declaration =
            DomainDeclaration::create(&wallet, &NONCE, Some("warehouse-main")).unwrap();
        let verified = declaration.verify().unwrap();

        assert_eq!(verified.domain_owner_public_key, wallet.public_key());
        assert_eq!(verified.nonce, NONCE);
        assert_eq!(verified.label.as_deref(), Some("warehouse-main"));
        assert_eq!(
            verified.domain_id,
            derive_domain_id(&wallet.public_key(), &NONCE)
        );
    }

    #[test]
    fn verify_accepts_unknown_fields_when_they_were_signed() {
        let declaration = DomainDeclaration::from_value(signed_declaration_value(Some((
            "unknown_extension",
            json!({"kept": true}),
        ))))
        .unwrap();

        assert!(declaration.verify().is_ok());
    }

    #[test]
    fn verify_rejects_unknown_field_added_after_signing() {
        let mut declaration = DomainDeclaration::create(&wallet(), &NONCE, None)
            .unwrap()
            .into_value();
        declaration
            .as_object_mut()
            .unwrap()
            .insert("unknown_extension".to_owned(), json!({"tampered": true}));
        let declaration = DomainDeclaration::from_value(declaration).unwrap();

        assert_eq!(declaration.verify(), Err(DomainError::InvalidSignature));
    }

    #[test]
    fn from_value_rejects_domain_id_mismatch() {
        let mut value = signed_declaration_value(None);
        value.as_object_mut().unwrap().insert(
            FIELD_DOMAIN_ID.to_owned(),
            Value::String(base64url::encode(&[0u8; DOMAIN_ID_LEN])),
        );

        assert!(matches!(
            DomainDeclaration::from_value(value),
            Err(DomainError::DomainIdMismatch { .. })
        ));
    }

    #[test]
    fn from_value_rejects_missing_required_field() {
        assert_eq!(
            DomainDeclaration::from_value(json!({})),
            Err(DomainError::MissingField { field: FIELD_TYPE })
        );
    }

    #[test]
    fn from_value_rejects_unsupported_type() {
        let mut value = signed_declaration_value(None);
        value.as_object_mut().unwrap().insert(
            FIELD_TYPE.to_owned(),
            Value::String("auki.other_declaration.v1".to_owned()),
        );

        assert_eq!(
            DomainDeclaration::from_value(value),
            Err(DomainError::UnsupportedType {
                actual: "auki.other_declaration.v1".to_owned()
            })
        );
    }

    #[test]
    fn from_value_rejects_malformed_nonce() {
        let mut value = signed_declaration_value(None);
        value
            .as_object_mut()
            .unwrap()
            .insert(FIELD_NONCE.to_owned(), Value::String("Zm9v".to_owned()));

        assert_eq!(
            DomainDeclaration::from_value(value),
            Err(DomainError::InvalidBase64Url {
                field: FIELD_NONCE,
                error: Base64UrlError::DecodedLengthMismatch {
                    expected: DOMAIN_NONCE_LEN,
                    actual: 3
                }
            })
        );
    }

    #[test]
    fn verify_rejects_tampered_signature_field() {
        let mut value = signed_declaration_value(None);
        value.as_object_mut().unwrap().insert(
            FIELD_SIGNATURE.to_owned(),
            Value::String(base64url::encode(&[0u8; 64])),
        );
        let declaration = DomainDeclaration::from_value(value).unwrap();

        assert_eq!(declaration.verify(), Err(DomainError::InvalidSignature));
    }

    #[test]
    fn signed_bytes_remove_only_signature_field() {
        let declaration =
            DomainDeclaration::from_value(signed_declaration_value(Some(("z_extra", json!(7)))))
                .unwrap();
        let signed: Value = serde_json::from_slice(&declaration.signed_bytes().unwrap()).unwrap();

        assert!(signed.get(FIELD_SIGNATURE).is_none());
        assert_eq!(signed["z_extra"], json!(7));
    }

    #[test]
    fn create_sorts_and_verifies_domain_delegation() {
        let owner = wallet();
        let delegate = delegate_wallet();
        let domain_id = domain_id_for_wallet();
        let delegation = DomainDelegation::create(
            &owner,
            &domain_id,
            &delegate.public_key(),
            &delegate_peer_id(),
            &[DelegationScope::Serve, DelegationScope::Advertise],
            VALID_FROM,
            EXPIRES_AT,
            Some("ops-peer"),
        )
        .unwrap();
        let verified = delegation
            .verify_for_authority(
                &domain_id,
                &owner.public_key(),
                &delegate.public_key(),
                &delegate_peer_id(),
                DelegationScope::Serve,
                NOW,
            )
            .unwrap();

        assert_eq!(verified.domain_id, domain_id);
        assert_eq!(verified.domain_owner_public_key, owner.public_key());
        assert_eq!(verified.delegate_wallet_public_key, delegate.public_key());
        assert_eq!(verified.delegate_peer_id, delegate_peer_id());
        assert_eq!(
            verified.scopes,
            vec![DelegationScope::Advertise, DelegationScope::Serve]
        );
        assert_eq!(verified.label.as_deref(), Some("ops-peer"));
        assert_eq!(
            delegation.value()[FIELD_SCOPES],
            json!(["advertise", "serve"])
        );
    }

    #[test]
    fn verify_accepts_unknown_delegation_fields_when_they_were_signed() {
        let delegation = DomainDelegation::from_value(signed_delegation_value(
            &["advertise", "serve"],
            VALID_FROM,
            EXPIRES_AT,
            Some(("unknown_extension", json!({"kept": true}))),
        ))
        .unwrap();

        assert!(delegation.verify().is_ok());
    }

    #[test]
    fn verify_rejects_unknown_delegation_field_added_after_signing() {
        let mut delegation = DomainDelegation::create(
            &wallet(),
            &domain_id_for_wallet(),
            &delegate_wallet().public_key(),
            &delegate_peer_id(),
            &[DelegationScope::Serve],
            VALID_FROM,
            EXPIRES_AT,
            None,
        )
        .unwrap()
        .into_value();
        delegation
            .as_object_mut()
            .unwrap()
            .insert("unknown_extension".to_owned(), json!({"tampered": true}));
        let delegation = DomainDelegation::from_value(delegation).unwrap();

        assert_eq!(delegation.verify(), Err(DomainError::InvalidSignature));
    }

    #[test]
    fn create_rejects_duplicate_delegation_scopes() {
        assert_eq!(
            DomainDelegation::create(
                &wallet(),
                &domain_id_for_wallet(),
                &delegate_wallet().public_key(),
                &delegate_peer_id(),
                &[DelegationScope::Serve, DelegationScope::Serve],
                VALID_FROM,
                EXPIRES_AT,
                None,
            ),
            Err(DomainError::DuplicateScope {
                scope: DelegationScope::Serve
            })
        );
    }

    #[test]
    fn from_value_rejects_unsorted_delegation_scopes() {
        assert_eq!(
            DomainDelegation::from_value(signed_delegation_value(
                &["serve", "advertise"],
                VALID_FROM,
                EXPIRES_AT,
                None,
            )),
            Err(DomainError::ScopesNotSorted {
                previous: DelegationScope::Serve,
                current: DelegationScope::Advertise
            })
        );
    }

    #[test]
    fn from_value_rejects_invalid_delegation_scope() {
        assert_eq!(
            DomainDelegation::from_value(signed_delegation_value(
                &["admin"],
                VALID_FROM,
                EXPIRES_AT,
                None,
            )),
            Err(DomainError::InvalidScope {
                scope: "admin".to_owned()
            })
        );
    }

    #[test]
    fn from_value_rejects_invalid_delegation_time_window() {
        assert_eq!(
            DomainDelegation::from_value(signed_delegation_value(
                &["serve"],
                EXPIRES_AT,
                VALID_FROM,
                None,
            )),
            Err(DomainError::InvalidTimeWindow {
                valid_from: EXPIRES_AT.to_owned(),
                expires_at: VALID_FROM.to_owned()
            })
        );
    }

    #[test]
    fn verify_for_authority_rejects_wrong_domain() {
        let delegation = DomainDelegation::from_value(signed_delegation_value(
            &["serve"],
            VALID_FROM,
            EXPIRES_AT,
            None,
        ))
        .unwrap();

        assert_eq!(
            delegation.verify_for_authority(
                DOMAIN_ID,
                &wallet().public_key(),
                &delegate_wallet().public_key(),
                &delegate_peer_id(),
                DelegationScope::Serve,
                NOW,
            ),
            Err(DomainError::DelegationDomainMismatch {
                delegated: domain_id_for_wallet(),
                expected: DOMAIN_ID.to_owned()
            })
        );
    }

    #[test]
    fn verify_for_authority_rejects_wrong_delegate_wallet() {
        let delegation = DomainDelegation::from_value(signed_delegation_value(
            &["serve"],
            VALID_FROM,
            EXPIRES_AT,
            None,
        ))
        .unwrap();
        let wrong_wallet = Wallet::from_seed(vec![9u8; 32]).expect("32-byte seed");

        assert_eq!(
            delegation.verify_for_authority(
                &domain_id_for_wallet(),
                &wallet().public_key(),
                &wrong_wallet.public_key(),
                &delegate_peer_id(),
                DelegationScope::Serve,
                NOW,
            ),
            Err(DomainError::DelegateWalletMismatch {
                delegated: delegate_wallet().public_key(),
                expected: wrong_wallet.public_key()
            })
        );
    }

    #[test]
    fn verify_for_authority_rejects_wrong_delegate_peer_id() {
        let delegation = DomainDelegation::from_value(signed_delegation_value(
            &["serve"],
            VALID_FROM,
            EXPIRES_AT,
            None,
        ))
        .unwrap();

        assert_eq!(
            delegation.verify_for_authority(
                &domain_id_for_wallet(),
                &wallet().public_key(),
                &delegate_wallet().public_key(),
                &other_peer_id(),
                DelegationScope::Serve,
                NOW,
            ),
            Err(DomainError::DelegatePeerIdMismatch {
                delegated: delegate_peer_id(),
                expected: other_peer_id()
            })
        );
    }

    #[test]
    fn verify_for_authority_rejects_missing_scope() {
        let delegation = DomainDelegation::from_value(signed_delegation_value(
            &["advertise"],
            VALID_FROM,
            EXPIRES_AT,
            None,
        ))
        .unwrap();

        assert_eq!(
            delegation.verify_for_authority(
                &domain_id_for_wallet(),
                &wallet().public_key(),
                &delegate_wallet().public_key(),
                &delegate_peer_id(),
                DelegationScope::Serve,
                NOW,
            ),
            Err(DomainError::MissingScope {
                scope: DelegationScope::Serve
            })
        );
    }

    #[test]
    fn verify_for_authority_rejects_not_yet_valid_and_expired() {
        let delegation = DomainDelegation::from_value(signed_delegation_value(
            &["serve"],
            VALID_FROM,
            EXPIRES_AT,
            None,
        ))
        .unwrap();

        assert_eq!(
            delegation.verify_for_authority(
                &domain_id_for_wallet(),
                &wallet().public_key(),
                &delegate_wallet().public_key(),
                &delegate_peer_id(),
                DelegationScope::Serve,
                "2026-05-26T11:59:59Z",
            ),
            Err(DomainError::DelegationNotYetValid {
                valid_from: VALID_FROM.to_owned(),
                now: "2026-05-26T11:59:59Z".to_owned()
            })
        );

        assert_eq!(
            delegation.verify_for_authority(
                &domain_id_for_wallet(),
                &wallet().public_key(),
                &delegate_wallet().public_key(),
                &delegate_peer_id(),
                DelegationScope::Serve,
                EXPIRES_AT,
            ),
            Err(DomainError::DelegationExpired {
                expires_at: EXPIRES_AT.to_owned(),
                now: EXPIRES_AT.to_owned()
            })
        );
    }

    #[test]
    fn verify_rejects_tampered_delegation_signature_field() {
        let mut value = signed_delegation_value(&["serve"], VALID_FROM, EXPIRES_AT, None);
        value.as_object_mut().unwrap().insert(
            FIELD_SIGNATURE.to_owned(),
            Value::String(base64url::encode(&[0u8; 64])),
        );
        let delegation = DomainDelegation::from_value(value).unwrap();

        assert_eq!(delegation.verify(), Err(DomainError::InvalidSignature));
    }

    #[test]
    fn delegation_signed_bytes_remove_only_signature_field() {
        let delegation = DomainDelegation::from_value(signed_delegation_value(
            &["serve"],
            VALID_FROM,
            EXPIRES_AT,
            Some(("z_extra", json!(7))),
        ))
        .unwrap();
        let signed: Value = serde_json::from_slice(&delegation.signed_bytes().unwrap()).unwrap();

        assert!(signed.get(FIELD_SIGNATURE).is_none());
        assert_eq!(signed["z_extra"], json!(7));
    }

    #[test]
    fn timestamp_comparison_treats_trailing_fractional_zeroes_as_equal() {
        assert_eq!(
            compare_rfc3339_z_timestamps("2026-05-26T12:00:00.1Z", "2026-05-26T12:00:00.10Z")
                .unwrap(),
            Ordering::Equal
        );
    }
}
