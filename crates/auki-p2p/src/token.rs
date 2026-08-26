use std::{
    collections::HashSet,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use libp2p::PeerId;
use p256::pkcs8::DecodePublicKey;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as AsyncRwLock;
use uuid::Uuid;

use crate::{Error, Result};

pub const P2P_TOKEN_TYPE: &str = "p2p-access";
pub const P2P_TOKEN_ISSUER: &str = "dds";
pub const P2P_TOKEN_AUDIENCE: &str = "auki-p2p";
/// Legacy signed metadata retained for Posemesh compatibility. It is not
/// consulted by the generic authenticated transport.
pub const P2P_TOKEN_SCOPE: &str = "domain-data:r";
pub const P2P_TOKEN_TTL: Duration = Duration::from_secs(30 * 60);
pub const P2P_TOKEN_CLOCK_SKEW: Duration = Duration::from_secs(60);
pub const P2P_TOKEN_MAX_BYTES: usize = 64 * 1024;
pub const DOMAIN_SERVER_MAX_DOMAINS: usize = 25;
pub const P2P_TOKEN_MAX_SCOPES: usize = 32;
pub const P2P_TOKEN_MAX_SCOPE_BYTES: usize = 128;
pub const P2P_TOKEN_MAX_PEER_TYPE_BYTES: usize = 64;
pub const P2P_TOKEN_MAX_APPLICATION_NAME_BYTES: usize = 64;
pub const P2P_TOKEN_MAX_APPLICATION_VERSION_BYTES: usize = 64;
pub const DDS_VERIFICATION_KEY_MAX_BYTES: usize = 64 * 1024;
pub const DDS_PREVIOUS_KEY_MIN_OVERLAP: Duration = Duration::from_secs(31 * 60);
pub const DDS_VERIFICATION_KEYS_MAX_STALENESS: Duration = Duration::from_secs(60 * 60);

/// Well-known diagnostic values used by Posemesh protocols.
///
/// The base `auki-p2p` authorization rule never branches on this enum. Tokens
/// may carry other bounded `peer_type` strings and still authenticate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerRole {
    Robot,
    Compute,
    DomainServer,
}

impl PeerRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Robot => "robot",
            Self::Compute => "compute",
            Self::DomainServer => "domain_server",
        }
    }
}

impl std::fmt::Display for PeerRole {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Bounded signed application diagnostics carried without granting authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignedApplicationMetadata {
    pub name: String,
    pub version: String,
}

/// The currently supported bounded DDS claim schema.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct P2PAccessClaims {
    #[serde(rename = "type")]
    pub token_type: String,
    pub iss: String,
    pub aud: Vec<String>,
    pub sub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_type: Option<String>,
    pub peer_id: String,
    pub domain_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<SignedApplicationMetadata>,
    pub iat: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,
    pub exp: u64,
}

/// An owned compact DDS credential whose secret token bytes are never printed.
///
/// Construction applies the encoded-size boundary only. Installation still
/// verifies the signature, complete claim profile, literal expiry, and local
/// Noise Peer ID before this credential can become current authority.
#[derive(Clone, Eq, PartialEq)]
pub struct SignedP2pCredential {
    compact: String,
}

impl SignedP2pCredential {
    pub fn new(compact: impl Into<String>) -> Result<Self> {
        let compact = compact.into();
        if compact.is_empty() || compact.len() > P2P_TOKEN_MAX_BYTES {
            return Err(Error::InvalidToken(format!(
                "encoded token must contain between 1 and {P2P_TOKEN_MAX_BYTES} bytes"
            )));
        }
        Ok(Self { compact })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.compact
    }
}

impl std::fmt::Debug for SignedP2pCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SignedP2pCredential")
            .field("compact", &"[redacted]")
            .finish()
    }
}

/// One bounded, host-supplied DDS verification-key generation.
///
/// Debug deliberately reveals neither PEM. Parsing and rotation validation are
/// performed by [`DdsTokenVerifier`], before the live key ring is replaced.
#[derive(Clone, PartialEq, Eq)]
pub struct DdsVerificationKeys {
    generation: u64,
    current_es256_pem: Vec<u8>,
    previous_es256_pem: Option<Vec<u8>>,
}

impl DdsVerificationKeys {
    pub fn new(
        generation: u64,
        current_es256_pem: impl Into<Vec<u8>>,
        previous_es256_pem: Option<Vec<u8>>,
    ) -> Self {
        Self {
            generation,
            current_es256_pem: current_es256_pem.into(),
            previous_es256_pem,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl std::fmt::Debug for DdsVerificationKeys {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DdsVerificationKeys")
            .field("generation", &self.generation)
            .field("current_es256_pem", &"[redacted]")
            .field(
                "previous_es256_pem",
                &self.previous_es256_pem.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct DdsTokenVerifier {
    keys: Arc<RwLock<VerificationKeyRing>>,
}

struct VerificationKeyRing {
    generation: u64,
    current: VerificationKey,
    previous: Option<VerificationKey>,
    previous_protected_until: Option<Instant>,
    last_refreshed_at: Instant,
}

struct VerificationKey {
    pem: Vec<u8>,
    canonical_public_key: Box<[u8]>,
    decoding: DecodingKey,
}

impl DdsTokenVerifier {
    /// Compatibility constructor for a single initial DDS ES256 key.
    /// Hosts that rotate keys should install them through [`crate::DomainAuthority`].
    pub fn from_es256_pem(public_key_pem: &[u8]) -> Result<Self> {
        Self::from_keys(DdsVerificationKeys::new(0, public_key_pem.to_vec(), None))
    }

    pub fn from_keys(keys: DdsVerificationKeys) -> Result<Self> {
        let now = Instant::now();
        let (current, previous) = parse_key_set(&keys)?;
        Ok(Self {
            keys: Arc::new(RwLock::new(VerificationKeyRing {
                generation: keys.generation,
                current,
                previous,
                previous_protected_until: keys
                    .previous_es256_pem
                    .as_ref()
                    .map(|_| now + DDS_PREVIOUS_KEY_MIN_OVERLAP),
                last_refreshed_at: now,
            })),
        })
    }

    /// Atomically install a newer key generation, or refresh the exact current
    /// generation. A rotated-out current key must remain as `previous` for the
    /// complete overlap window.
    pub(crate) fn replace_keys(&self, keys: DdsVerificationKeys) -> Result<()> {
        self.replace_keys_at(keys, Instant::now())
    }

    pub fn generation(&self) -> u64 {
        self.keys.read().generation
    }

    pub fn verify(&self, token: &str) -> Result<P2PAccessClaims> {
        self.verify_at(token, unix_time_now(), Instant::now())
    }

    #[cfg(test)]
    pub(crate) fn make_stale_for_test(&self) {
        self.keys.write().last_refreshed_at =
            Instant::now() - DDS_VERIFICATION_KEYS_MAX_STALENESS - Duration::from_nanos(1);
    }

    fn replace_keys_at(&self, keys: DdsVerificationKeys, now: Instant) -> Result<()> {
        // Parse every candidate before taking the write lock. A failed update
        // therefore leaves the live generation untouched.
        let (current, previous) = parse_key_set(&keys)?;
        let mut live = self.keys.write();
        if keys.generation < live.generation {
            return Err(Error::StaleVerificationKeyGeneration {
                current: live.generation,
                proposed: keys.generation,
            });
        }

        // Same-generation refresh is intentionally byte-exact per the frozen
        // host contract. Canonical key identity below is used separately for
        // duplicate detection and rotation lineage.
        let same_current_bytes = current.pem == live.current.pem;
        let same_previous_bytes =
            previous.as_ref().map(|key| &key.pem) == live.previous.as_ref().map(|key| &key.pem);
        if keys.generation == live.generation {
            if !same_current_bytes || !same_previous_bytes {
                return Err(Error::VerificationKeyGenerationConflict(keys.generation));
            }
            live.last_refreshed_at = now;
            return Ok(());
        }

        let same_current_key = current.canonical_public_key == live.current.canonical_public_key;
        let same_previous_key = previous.as_ref().map(|key| &key.canonical_public_key)
            == live.previous.as_ref().map(|key| &key.canonical_public_key);
        if !same_current_key {
            if live
                .previous_protected_until
                .is_some_and(|deadline| now < deadline)
            {
                return Err(Error::VerificationKeyOverlapActive);
            }
            if previous.as_ref().map(|key| &key.canonical_public_key)
                != Some(&live.current.canonical_public_key)
            {
                return Err(Error::VerificationKeyRotationMissingPrevious);
            }
            live.previous_protected_until = Some(now + DDS_PREVIOUS_KEY_MIN_OVERLAP);
        } else if !same_previous_key {
            if live
                .previous_protected_until
                .is_some_and(|deadline| now < deadline)
            {
                return Err(Error::VerificationKeyOverlapActive);
            }
            // With an unchanged current key the only meaningful set change is
            // retiring the previous key after its overlap. Adding or swapping
            // an unrelated previous key creates an ambiguous trust history.
            if previous.is_some() {
                return Err(Error::VerificationKeyRotationMissingPrevious);
            }
            live.previous_protected_until = None;
        }

        live.generation = keys.generation;
        live.current = current;
        live.previous = previous;
        live.last_refreshed_at = now;
        Ok(())
    }

    fn verify_at(
        &self,
        token: &str,
        wall_time_seconds: u64,
        monotonic_now: Instant,
    ) -> Result<P2PAccessClaims> {
        if token.is_empty() || token.len() > P2P_TOKEN_MAX_BYTES {
            return Err(Error::InvalidToken(format!(
                "encoded token must contain between 1 and {P2P_TOKEN_MAX_BYTES} bytes"
            )));
        }

        let keys = self.keys.read();
        if monotonic_now.saturating_duration_since(keys.last_refreshed_at)
            > DDS_VERIFICATION_KEYS_MAX_STALENESS
        {
            return Err(Error::VerificationKeysStale);
        }

        let current_result = decode_claims(token, &keys.current.decoding);
        let claims = match current_result {
            Ok(claims) => claims,
            Err(current_error) => match &keys.previous {
                Some(previous) => {
                    decode_claims(token, &previous.decoding).map_err(|_| current_error)?
                }
                None => return Err(current_error),
            },
        };
        validate_profile(&claims, wall_time_seconds)?;
        Ok(claims)
    }
}

fn parse_key_set(keys: &DdsVerificationKeys) -> Result<(VerificationKey, Option<VerificationKey>)> {
    let current = parse_key(&keys.current_es256_pem)?;
    let previous = keys
        .previous_es256_pem
        .as_deref()
        .map(parse_key)
        .transpose()?;
    if previous
        .as_ref()
        .is_some_and(|previous| previous.canonical_public_key == current.canonical_public_key)
    {
        return Err(Error::InvalidVerificationKeySet(
            "current and previous keys must be different".into(),
        ));
    }
    Ok((current, previous))
}

fn parse_key(pem: &[u8]) -> Result<VerificationKey> {
    if pem.is_empty() || pem.len() > DDS_VERIFICATION_KEY_MAX_BYTES {
        return Err(Error::InvalidVerificationKeySet(format!(
            "each verification key must contain between 1 and {DDS_VERIFICATION_KEY_MAX_BYTES} bytes"
        )));
    }
    let pem_text = std::str::from_utf8(pem).map_err(|_| {
        Error::InvalidVerificationKeySet("verification key PEM must be UTF-8".into())
    })?;
    let public_key = p256::PublicKey::from_public_key_pem(pem_text).map_err(|_| {
        Error::InvalidVerificationKeySet("verification key must contain a P-256 public key".into())
    })?;
    Ok(VerificationKey {
        pem: pem.to_vec(),
        canonical_public_key: public_key.to_sec1_bytes(),
        decoding: DecodingKey::from_ec_pem(pem)?,
    })
}

fn decode_claims(token: &str, key: &DecodingKey) -> Result<P2PAccessClaims> {
    let mut validation = Validation::new(Algorithm::ES256);
    validation.leeway = P2P_TOKEN_CLOCK_SKEW.as_secs();
    // `exp` and `iat` are checked below so literal expiry can fail closed even
    // while bounded future-clock skew remains accepted.
    validation.validate_exp = false;
    validation.set_audience(&[P2P_TOKEN_AUDIENCE]);
    validation.set_issuer(&[P2P_TOKEN_ISSUER]);
    validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub"]);
    decode::<P2PAccessClaims>(token, key, &validation)
        .map(|decoded| decoded.claims)
        .map_err(Error::TokenVerification)
}

fn validate_profile(claims: &P2PAccessClaims, now: u64) -> Result<()> {
    if claims.token_type != P2P_TOKEN_TYPE {
        return Err(Error::InvalidToken("unexpected token type".into()));
    }
    if claims.iss != P2P_TOKEN_ISSUER {
        return Err(Error::InvalidToken("unexpected issuer".into()));
    }
    if claims.aud != [P2P_TOKEN_AUDIENCE] {
        return Err(Error::InvalidToken(
            "audience must be exactly [auki-p2p]".into(),
        ));
    }
    validate_canonical_uuid(&claims.sub, "subject must be a canonical UUID")?;
    let peer_id = PeerId::from_str(&claims.peer_id)
        .map_err(|_| Error::InvalidToken("peer_id must be a libp2p Peer ID".into()))?;
    if peer_id.to_string() != claims.peer_id {
        return Err(Error::InvalidToken(
            "peer_id must use its canonical representation".into(),
        ));
    }
    if claims.exp.checked_sub(claims.iat) != Some(P2P_TOKEN_TTL.as_secs()) {
        return Err(Error::InvalidToken(
            "expiration must be exactly 30 minutes after issued-at".into(),
        ));
    }
    ensure_literal_expiry(claims, now)?;
    if claims.iat > now.saturating_add(P2P_TOKEN_CLOCK_SKEW.as_secs()) {
        return Err(Error::InvalidToken(
            "issued-at is beyond the allowed clock skew".into(),
        ));
    }
    if claims
        .nbf
        .is_some_and(|not_before| not_before > now.saturating_add(P2P_TOKEN_CLOCK_SKEW.as_secs()))
    {
        return Err(Error::InvalidToken(
            "not-before is beyond the allowed clock skew".into(),
        ));
    }

    if !(1..=DOMAIN_SERVER_MAX_DOMAINS).contains(&claims.domain_ids.len()) {
        return Err(Error::InvalidToken(
            "domain_ids must contain between 1 and 25 Domains".into(),
        ));
    }
    let mut unique_domains = HashSet::with_capacity(claims.domain_ids.len());
    for domain_id in &claims.domain_ids {
        let parsed = validate_canonical_uuid(domain_id, "domain_ids must contain canonical UUIDs")?;
        if !unique_domains.insert(parsed) {
            return Err(Error::InvalidToken("domain_ids must be unique".into()));
        }
    }

    if let Some(peer_type) = &claims.peer_type {
        validate_visible_ascii(peer_type, P2P_TOKEN_MAX_PEER_TYPE_BYTES, "peer_type")?;
    }
    if claims.scopes.len() > P2P_TOKEN_MAX_SCOPES {
        return Err(Error::InvalidToken(format!(
            "scopes must contain at most {P2P_TOKEN_MAX_SCOPES} entries"
        )));
    }
    let mut unique_scopes = HashSet::with_capacity(claims.scopes.len());
    for scope in &claims.scopes {
        validate_visible_ascii(scope, P2P_TOKEN_MAX_SCOPE_BYTES, "scope")?;
        if !unique_scopes.insert(scope) {
            return Err(Error::InvalidToken("scopes must be unique".into()));
        }
    }
    if let Some(application) = &claims.application {
        validate_visible_ascii(
            &application.name,
            P2P_TOKEN_MAX_APPLICATION_NAME_BYTES,
            "application.name",
        )?;
        validate_visible_ascii(
            &application.version,
            P2P_TOKEN_MAX_APPLICATION_VERSION_BYTES,
            "application.version",
        )?;
    }
    Ok(())
}

fn validate_canonical_uuid(value: &str, message: &'static str) -> Result<Uuid> {
    let parsed = Uuid::parse_str(value).map_err(|_| Error::InvalidToken(message.into()))?;
    if parsed.to_string() != value {
        return Err(Error::InvalidToken(message.into()));
    }
    Ok(parsed)
}

fn validate_visible_ascii(value: &str, maximum: usize, field: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > maximum
        || !value.bytes().all(|byte| (b'!'..=b'~').contains(&byte))
    {
        return Err(Error::InvalidToken(format!(
            "{field} must contain 1..={maximum} visible ASCII bytes"
        )));
    }
    Ok(())
}

pub(crate) fn ensure_literal_expiry(claims: &P2PAccessClaims, now: u64) -> Result<()> {
    if claims.exp <= now {
        return Err(Error::InvalidToken("token has literally expired".into()));
    }
    Ok(())
}

pub(crate) fn unix_time_now() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
}

#[derive(Clone)]
struct StoredCredential {
    signed: SignedP2pCredential,
    claims: P2PAccessClaims,
}

#[derive(Clone, Default)]
pub(crate) struct TokenStore {
    current: Arc<AsyncRwLock<Option<StoredCredential>>>,
}

impl TokenStore {
    pub async fn install(
        &self,
        signed: SignedP2pCredential,
        verifier: &DdsTokenVerifier,
        local_peer_id: PeerId,
        expected_expiration: Option<u64>,
        required_domain_id: Option<Uuid>,
    ) -> Result<P2PAccessClaims> {
        // Node's authority-update gate prevents verification keys from changing
        // between this verification and the write. The comparison and replacement
        // below remain one atomic credential critical section, so a slow stale
        // credential verification can never overwrite newer authority.
        let claims = verifier.verify(signed.as_str())?;
        ensure_token_peer(&claims, local_peer_id)?;
        if let Some(required_domain_id) = required_domain_id {
            ensure_token_domain(&claims, required_domain_id)?;
        }
        if let Some(expected_expiration) = expected_expiration {
            if claims.exp != expected_expiration {
                return Err(Error::CredentialExpirationMismatch {
                    credential_expiration: claims.exp,
                    expected_expiration,
                });
            }
        }
        let mut current = self.current.write().await;
        if let Some(installed) = current.as_ref() {
            if claims.iat < installed.claims.iat {
                return Err(Error::StaleCredential {
                    current_issued_at: installed.claims.iat,
                    proposed_issued_at: claims.iat,
                });
            }
            if claims.iat == installed.claims.iat {
                if claims != installed.claims {
                    return Err(Error::CredentialIssuedAtConflict(claims.iat));
                }
                // Re-signing identical claims (for example during key rotation)
                // is semantically idempotent. Keep the already-installed
                // compact token so concurrent equal updates cannot oscillate.
                return Ok(installed.claims.clone());
            }
        }
        *current = Some(StoredCredential {
            signed,
            claims: claims.clone(),
        });
        Ok(claims)
    }

    pub async fn clear(&self) {
        *self.current.write().await = None;
    }

    pub async fn snapshot(&self) -> Option<SignedP2pCredential> {
        self.current
            .read()
            .await
            .as_ref()
            .map(|current| current.signed.clone())
    }

    pub async fn snapshot_with_claims(&self) -> Option<(SignedP2pCredential, P2PAccessClaims)> {
        self.current
            .read()
            .await
            .as_ref()
            .map(|current| (current.signed.clone(), current.claims.clone()))
    }
}

pub(crate) fn ensure_token_peer(claims: &P2PAccessClaims, noise_peer_id: PeerId) -> Result<()> {
    let token_peer_id = PeerId::from_str(&claims.peer_id)
        .map_err(|_| Error::InvalidToken("peer_id must be a libp2p Peer ID".into()))?;
    if token_peer_id != noise_peer_id {
        return Err(Error::PeerIdMismatch {
            token_peer_id: token_peer_id.to_string(),
            noise_peer_id: noise_peer_id.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn ensure_token_domain(
    claims: &P2PAccessClaims,
    required_domain_id: Uuid,
) -> Result<()> {
    if claims
        .domain_ids
        .iter()
        .filter_map(|domain_id| Uuid::parse_str(domain_id).ok())
        .any(|domain_id| domain_id == required_domain_id)
    {
        Ok(())
    } else {
        Err(Error::LocalDomainMismatch(required_domain_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

    const SECOND_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEAxcARQLozLIqu/CFm6ub89EElhHX
O+4eTRPLA8IA+ibNtrfWbavOIYZEtwGneJvRTovHr5OUGFu3n/gXNqGbKw==
-----END PUBLIC KEY-----"#;

    #[test]
    fn previous_key_cannot_retire_before_the_full_overlap() {
        let base = Instant::now();
        let verifier = DdsTokenVerifier::from_keys(DdsVerificationKeys::new(
            10,
            FIRST_PUBLIC_KEY.to_vec(),
            None,
        ))
        .unwrap();
        verifier
            .replace_keys_at(
                DdsVerificationKeys::new(
                    11,
                    SECOND_PUBLIC_KEY.to_vec(),
                    Some(FIRST_PUBLIC_KEY.to_vec()),
                ),
                base,
            )
            .unwrap();

        assert!(matches!(
            verifier.replace_keys_at(
                DdsVerificationKeys::new(12, SECOND_PUBLIC_KEY.to_vec(), None),
                base + DDS_PREVIOUS_KEY_MIN_OVERLAP - Duration::from_nanos(1),
            ),
            Err(Error::VerificationKeyOverlapActive)
        ));
        verifier
            .replace_keys_at(
                DdsVerificationKeys::new(12, SECOND_PUBLIC_KEY.to_vec(), None),
                base + DDS_PREVIOUS_KEY_MIN_OVERLAP,
            )
            .unwrap();
    }

    #[test]
    fn stale_key_ring_fails_before_token_parsing() {
        let verifier = DdsTokenVerifier::from_es256_pem(FIRST_PUBLIC_KEY).unwrap();
        verifier.keys.write().last_refreshed_at =
            Instant::now() - DDS_VERIFICATION_KEYS_MAX_STALENESS - Duration::from_nanos(1);
        assert!(matches!(
            verifier.verify("not-even-a-jwt"),
            Err(Error::VerificationKeysStale)
        ));
    }

    #[test]
    fn malformed_duplicate_and_oversized_key_sets_are_rejected() {
        assert!(
            DdsTokenVerifier::from_keys(DdsVerificationKeys::new(
                1,
                FIRST_PUBLIC_KEY.to_vec(),
                Some(FIRST_PUBLIC_KEY.to_vec()),
            ))
            .is_err()
        );
        assert!(
            DdsTokenVerifier::from_keys(DdsVerificationKeys::new(
                1,
                vec![b'x'; DDS_VERIFICATION_KEY_MAX_BYTES + 1],
                None,
            ))
            .is_err()
        );
    }

    #[test]
    fn semantic_duplicate_keys_are_rejected_even_with_different_pem_bytes() {
        let mut equivalent_encoding = FIRST_PUBLIC_KEY.to_vec();
        equivalent_encoding.push(b'\n');
        assert_ne!(equivalent_encoding, FIRST_PUBLIC_KEY);
        assert!(matches!(
            DdsTokenVerifier::from_keys(DdsVerificationKeys::new(
                1,
                FIRST_PUBLIC_KEY.to_vec(),
                Some(equivalent_encoding),
            )),
            Err(Error::InvalidVerificationKeySet(_))
        ));
    }

    #[test]
    fn exact_generation_refreshes_but_failed_replacement_preserves_live_state() {
        let initial = Instant::now();
        let verifier = DdsTokenVerifier::from_keys(DdsVerificationKeys::new(
            4,
            FIRST_PUBLIC_KEY.to_vec(),
            None,
        ))
        .unwrap();
        verifier.keys.write().last_refreshed_at = initial;
        let refreshed = initial + Duration::from_secs(7);
        verifier
            .replace_keys_at(
                DdsVerificationKeys::new(4, FIRST_PUBLIC_KEY.to_vec(), None),
                refreshed,
            )
            .unwrap();
        assert_eq!(verifier.keys.read().last_refreshed_at, refreshed);

        let mut equivalent_but_not_identical = FIRST_PUBLIC_KEY.to_vec();
        equivalent_but_not_identical.push(b'\n');
        assert!(matches!(
            verifier.replace_keys_at(
                DdsVerificationKeys::new(4, equivalent_but_not_identical, None),
                refreshed + Duration::from_secs(1),
            ),
            Err(Error::VerificationKeyGenerationConflict(4))
        ));
        let live = verifier.keys.read();
        assert_eq!(live.generation, 4);
        assert_eq!(live.current.pem, FIRST_PUBLIC_KEY);
        assert_eq!(live.last_refreshed_at, refreshed);
    }
}
