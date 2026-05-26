//! Domain authority helpers for v1.

use super::{
    base64url::{self, Base64UrlError},
    identity::WALLET_SIGNATURE_SCHEME_ED25519,
};
use auki_identity::PublicKey as WalletPublicKey;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;

/// V1 domain-id hash input object type.
pub const DOMAIN_ID_TYPE: &str = "auki.domain_id.v1";
/// Raw v1 domain id length in bytes.
pub const DOMAIN_ID_LEN: usize = 32;
/// Raw v1 domain nonce length in bytes.
pub const DOMAIN_NONCE_LEN: usize = 16;

const FIELD_TYPE: &str = "type";
const FIELD_WALLET_SIGNATURE_SCHEME: &str = "wallet_signature_scheme";
const FIELD_DOMAIN_OWNER_PUBLIC_KEY: &str = "domain_owner_public_key";
const FIELD_NONCE: &str = "nonce";
const FIELD_DOMAIN_ID: &str = "domain_id";

/// Errors produced by v1 domain helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// A base64url field was malformed or decoded to the wrong length.
    InvalidBase64Url {
        /// Field name.
        field: &'static str,
        /// Base64url decoding error.
        error: Base64UrlError,
    },
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBase64Url { field, error } => {
                write!(f, "invalid base64url in field {field}: {error}")
            }
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
}
