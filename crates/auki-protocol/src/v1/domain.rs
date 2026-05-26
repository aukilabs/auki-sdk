//! Domain authority helpers for v1.

use super::{
    base64url::{self, Base64UrlError},
    identity::WALLET_SIGNATURE_SCHEME_ED25519,
};
use auki_identity::{PublicKey as WalletPublicKey, Signature, VerifyError, Wallet, verify};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;

/// V1 domain-id hash input object type.
pub const DOMAIN_ID_TYPE: &str = "auki.domain_id.v1";
/// V1 domain declaration object type.
pub const DOMAIN_DECLARATION_TYPE: &str = "auki.domain_declaration.v1";
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

/// A v1 domain-owner signed domain declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainDeclaration {
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
    /// Declared domain id did not match the recomputed id.
    DomainIdMismatch {
        /// Domain id carried by the declaration.
        declared: String,
        /// Domain id recomputed from owner wallet public key and nonce.
        recomputed: String,
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
            Self::DomainIdMismatch {
                declared,
                recomputed,
            } => {
                write!(
                    f,
                    "domain id mismatch: declared {declared}, recomputed {recomputed}"
                )
            }
            Self::InvalidSignature => write!(f, "domain declaration signature does not verify"),
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
    base64url::decode_exact::<32>(domain_owner_public_key)
        .map(WalletPublicKey)
        .map_err(|error| DomainError::InvalidBase64Url {
            field: FIELD_DOMAIN_OWNER_PUBLIC_KEY,
            error,
        })
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

fn map_verify_error(_error: VerifyError) -> DomainError {
    DomainError::InvalidSignature
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

    fn wallet() -> std::sync::Arc<Wallet> {
        Wallet::from_seed(vec![3u8; 32]).expect("32-byte seed")
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
}
