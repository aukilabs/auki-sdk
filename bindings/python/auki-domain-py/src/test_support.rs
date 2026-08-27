//! Non-shipping Python helpers for authenticated Domain integration tests.
//!
//! This module is intended to be compiled only by the non-default
//! `test-support` feature. The fixed key pair is test material already used by
//! `auki-domain`'s Rust integration tests; it must never be used by production
//! code or enabled in a published wheel.

use std::time::{SystemTime, UNIX_EPOCH};

use auki_p2p::{
    P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE, P2PAccessClaims,
    SignedApplicationMetadata,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyModule};
use uuid::Uuid;

use crate::{
    runtime_error,
    values::{PyDdsVerificationKeys, PyIdentity, PySignedP2pCredential},
};

const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

const TEST_SUBJECT: &str = "00000000-0000-0000-0000-000000000001";

/// Mint host-supplied authority material for one exact test identity/Domain.
///
/// `expired=True` creates a correctly signed credential whose literal expiry
/// is already in the past. Supplying the returned credential with another
/// `Identity` or another Domain UUID creates the normal peer/domain mismatch
/// vectors without requiring separate helpers.
#[pyfunction(name = "_test_authority")]
#[pyo3(signature = (identity, domain_id, *, expired=false))]
fn test_authority(
    identity: &PyIdentity,
    domain_id: &str,
    expired: bool,
) -> PyResult<(PyDdsVerificationKeys, PySignedP2pCredential)> {
    let domain_id_text = domain_id;
    let domain_id = Uuid::parse_str(domain_id_text)
        .map_err(|error| PyValueError::new_err(format!("invalid Domain UUID: {error}")))?;
    if domain_id.to_string() != domain_id_text {
        return Err(PyValueError::new_err(
            "domain_id must be a canonical lowercase UUID",
        ));
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(runtime_error)?
        .as_secs();
    let issued_at = if expired {
        now.checked_sub(P2P_TOKEN_TTL.as_secs() + 1)
            .ok_or_else(|| runtime_error("system time is too close to the Unix epoch"))?
    } else {
        now
    };
    let expires_at = issued_at
        .checked_add(P2P_TOKEN_TTL.as_secs())
        .ok_or_else(|| runtime_error("test credential expiration overflowed"))?;

    let claims = P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.into(),
        iss: P2P_TOKEN_ISSUER.into(),
        aud: vec![P2P_TOKEN_AUDIENCE.into()],
        sub: TEST_SUBJECT.into(),
        peer_type: None,
        peer_id: identity.inner.peer_id().to_string(),
        domain_ids: vec![domain_id.to_string()],
        scopes: Vec::new(),
        application: Some(SignedApplicationMetadata {
            name: "public-domain-test".into(),
            version: "1.0.0".into(),
        }),
        iat: issued_at,
        nbf: None,
        exp: expires_at,
    };
    let compact = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(TEST_DDS_PRIVATE_KEY).map_err(runtime_error)?,
    )
    .map_err(runtime_error)?;

    Ok((
        PyDdsVerificationKeys {
            inner: auki_domain_rs::DdsVerificationKeys::new(0, TEST_DDS_PUBLIC_KEY.to_vec(), None),
        },
        PySignedP2pCredential {
            inner: auki_domain_rs::SignedP2pCredential::new(compact).map_err(runtime_error)?,
        },
    ))
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(test_authority, module)?)?;
    Ok(())
}
