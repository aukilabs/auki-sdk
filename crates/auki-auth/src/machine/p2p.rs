use std::{collections::HashSet, sync::Arc, time::Duration};

use auki_p2p::{
    DDS_PREVIOUS_KEY_MIN_OVERLAP, DDS_VERIFICATION_KEY_MAX_BYTES, DdsTokenVerifier,
    DdsVerificationKeys, PeerId, PeerIdentityProof, SignedP2pCredential,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
use reqwest::Url;
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;
use uuid::Uuid;

use super::{AccessBundle, token_manager::TokenProviderError};

const CHALLENGE_PATH: &str = "/internal/v1/auth/p2p/challenge";
const VERIFY_PATH: &str = "/internal/v1/auth/p2p/verify";
const ROBOT_TOKEN_PATH: &str = "/internal/v1/auth/robot/p2p-token";
const VERIFICATION_KEYS_PATH: &str = "/service/p2p-verification-keys";

#[derive(Debug, thiserror::Error)]
pub enum DdsP2pError {
    #[error("failed to build DDS P2P HTTP client")]
    Client(#[source] reqwest::Error),
    #[error("DDS P2P request failed")]
    Request(#[source] reqwest::Error),
    #[error("DDS P2P endpoint returned status {0}")]
    UpstreamStatus(StatusCode),
    #[error("DDS P2P response missing field '{0}'")]
    MissingField(&'static str),
    #[error("DDS P2P challenge encoding is invalid")]
    InvalidChallengeEncoding(#[source] base64::DecodeError),
    #[error("DDS P2P identity proof failed")]
    IdentityProof(#[source] auki_p2p::Error),
    #[error("DDS P2P response returned a different Peer ID")]
    PeerIdMismatch,
    #[error("DDS P2P verification public key is invalid")]
    InvalidPublicKey(#[source] auki_p2p::Error),
    #[error("DDS P2P verification public key response is too large")]
    VerificationKeyResponseTooLarge,
    #[error("DDS P2P verification-key response is invalid: {0}")]
    InvalidVerificationKeyResponse(&'static str),
    #[error("machine token is malformed")]
    MalformedMachineToken,
    #[error("Robot machine token has no assigned Domain")]
    MissingRobotAssignment,
    #[error("Robot machine token is unavailable")]
    MachineToken(#[source] TokenProviderError),
    #[error("DDS P2P access token is invalid")]
    InvalidAccessToken(#[source] auki_p2p::Error),
    #[error("DDS P2P access-token expiration is invalid")]
    InvalidExpiration,
    #[error("the current DDS P2P credential does not authorize the required Domain")]
    CredentialDomainMismatch,
}

pub type Result<T> = std::result::Result<T, DdsP2pError>;

#[derive(Clone)]
pub struct DdsP2pClient {
    base_url: Url,
    http: Client,
    initial_verification: Arc<OnceCell<InitialVerification>>,
}

struct InitialVerification {
    verifier: DdsTokenVerifier,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerificationKeysResponse {
    version: u8,
    generation: u64,
    previous_key_overlap_seconds: u64,
    keys: Vec<VerificationKeyResponse>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum VerificationKeyStatus {
    Current,
    Previous,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerificationKeyResponse {
    id: String,
    status: VerificationKeyStatus,
    signing_method: String,
    public_key: String,
}

fn convert_verification_keys(response: VerificationKeysResponse) -> Result<DdsVerificationKeys> {
    if response.version != 1 || response.generation == 0 {
        return Err(DdsP2pError::InvalidVerificationKeyResponse(
            "unsupported key-set version or zero generation",
        ));
    }
    if response.previous_key_overlap_seconds < DDS_PREVIOUS_KEY_MIN_OVERLAP.as_secs() {
        return Err(DdsP2pError::InvalidVerificationKeyResponse(
            "previous-key overlap is below the SDK safety window",
        ));
    }
    if !(1..=2).contains(&response.keys.len())
        || response.keys[0].status != VerificationKeyStatus::Current
        || response
            .keys
            .get(1)
            .is_some_and(|key| key.status != VerificationKeyStatus::Previous)
    {
        return Err(DdsP2pError::InvalidVerificationKeyResponse(
            "keys must contain current first and at most one previous key",
        ));
    }

    let mut parsed = Vec::with_capacity(response.keys.len());
    let mut ids = HashSet::with_capacity(response.keys.len());
    for key in response.keys {
        if key.id.len() != 64
            || !key
                .id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !ids.insert(key.id.clone())
            || key.signing_method != "ES256"
            || key.public_key.is_empty()
            || key.public_key.len() > DDS_VERIFICATION_KEY_MAX_BYTES
        {
            return Err(DdsP2pError::InvalidVerificationKeyResponse(
                "verification-key metadata or PEM is invalid",
            ));
        }
        if verification_key_id(&key.public_key)? != key.id {
            return Err(DdsP2pError::InvalidVerificationKeyResponse(
                "verification-key id does not match its canonical PKIX fingerprint",
            ));
        }
        parsed.push(key.public_key.into_bytes());
    }

    let current = parsed.remove(0);
    let previous = parsed.pop();
    Ok(DdsVerificationKeys::new(
        response.generation,
        current,
        previous,
    ))
}

fn verification_key_id(public_key_pem: &str) -> Result<String> {
    let public_key = p256::PublicKey::from_public_key_pem(public_key_pem).map_err(|_| {
        DdsP2pError::InvalidVerificationKeyResponse(
            "verification key must contain a P-256 public key",
        )
    })?;
    let canonical_der = public_key.to_public_key_der().map_err(|_| {
        DdsP2pError::InvalidVerificationKeyResponse(
            "verification key could not be encoded as canonical PKIX DER",
        )
    })?;
    Ok(hex::encode(Sha256::digest(canonical_der.as_bytes())))
}

impl DdsP2pClient {
    pub fn new(base_url: Url, timeout: Duration) -> Result<Self> {
        let http = Client::builder()
            .use_rustls_tls()
            .no_proxy()
            .redirect(Policy::none())
            .timeout(timeout)
            .build()
            .map_err(DdsP2pError::Client)?;
        Ok(Self {
            base_url,
            http,
            initial_verification: Arc::new(OnceCell::new()),
        })
    }

    pub async fn token_verifier(&self) -> Result<DdsTokenVerifier> {
        Ok(self.initial_verification().await?.verifier.clone())
    }

    async fn initial_verification(&self) -> Result<&InitialVerification> {
        self.initial_verification
            .get_or_try_init(|| async {
                let keys = self.fetch_verification_keys().await?;
                let verifier =
                    DdsTokenVerifier::from_keys(keys).map_err(DdsP2pError::InvalidPublicKey)?;
                Ok(InitialVerification { verifier })
            })
            .await
    }

    async fn fetch_verification_keys(&self) -> Result<DdsVerificationKeys> {
        let response = self
            .http
            .get(self.endpoint(VERIFICATION_KEYS_PATH))
            .header(ACCEPT, "application/json")
            .header(CACHE_CONTROL, "no-cache")
            .send()
            .await
            .map_err(DdsP2pError::Request)?;
        ensure_success(&response)?;
        let is_json = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"));
        if !is_json {
            return Err(DdsP2pError::InvalidVerificationKeyResponse(
                "Content-Type must be application/json",
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > DDS_VERIFICATION_KEY_MAX_BYTES as u64)
        {
            return Err(DdsP2pError::VerificationKeyResponseTooLarge);
        }

        let mut encoded = Vec::new();
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(DdsP2pError::Request)?;
            if encoded.len().saturating_add(chunk.len()) > DDS_VERIFICATION_KEY_MAX_BYTES {
                return Err(DdsP2pError::VerificationKeyResponseTooLarge);
            }
            encoded.extend_from_slice(&chunk);
        }
        let response: VerificationKeysResponse =
            serde_json::from_slice(&encoded).map_err(|_| {
                DdsP2pError::InvalidVerificationKeyResponse("JSON does not match the V1 contract")
            })?;
        convert_verification_keys(response)
    }

    async fn create_challenge(
        &self,
        machine_token: &str,
        identity: &PeerIdentityProof,
    ) -> Result<ChallengeResponse> {
        let request = ChallengeRequest {
            peer_id: identity.peer_id().to_string(),
            public_key: URL_SAFE_NO_PAD.encode(identity.public_key_protobuf()),
        };
        let response = self
            .http
            .post(self.endpoint(CHALLENGE_PATH))
            .bearer_auth(machine_token)
            .json(&request)
            .send()
            .await
            .map_err(DdsP2pError::Request)?;
        ensure_success(&response)?;
        let response: ChallengeResponse = response.json().await.map_err(DdsP2pError::Request)?;
        if response.challenge_id.trim().is_empty() {
            return Err(DdsP2pError::MissingField("challenge_id"));
        }
        if response.challenge.trim().is_empty() {
            return Err(DdsP2pError::MissingField("challenge"));
        }
        if response.expires_at <= Utc::now() {
            return Err(DdsP2pError::InvalidExpiration);
        }
        Ok(response)
    }

    async fn verify_challenge(
        &self,
        machine_token: &str,
        identity: &PeerIdentityProof,
        challenge: ChallengeResponse,
    ) -> Result<AccessBundle> {
        let challenge_bytes = URL_SAFE_NO_PAD
            .decode(challenge.challenge)
            .map_err(DdsP2pError::InvalidChallengeEncoding)?;
        let signature = identity
            .sign_challenge(&challenge_bytes)
            .map_err(DdsP2pError::IdentityProof)?;
        let request = VerifyRequest {
            challenge_id: challenge.challenge_id,
            signature: URL_SAFE_NO_PAD.encode(signature),
        };
        let response = self
            .http
            .post(self.endpoint(VERIFY_PATH))
            .bearer_auth(machine_token)
            .json(&request)
            .send()
            .await
            .map_err(DdsP2pError::Request)?;
        ensure_success(&response)?;
        let response: VerifyResponse = response.json().await.map_err(DdsP2pError::Request)?;
        if response.peer_id != identity.peer_id().to_string() {
            return Err(DdsP2pError::PeerIdMismatch);
        }
        if response.access_token.is_empty() {
            return Err(DdsP2pError::MissingField("access_token"));
        }
        if response.access_expires_at <= Utc::now() {
            return Err(DdsP2pError::InvalidExpiration);
        }
        Ok(AccessBundle::new(
            response.access_token,
            response.access_expires_at,
        ))
    }

    /// Prepare credential and key material for the host runtime to install.
    /// The runtime verifies the signed credential during installation.
    pub async fn authority_material(
        &self,
        identity: &PeerIdentityProof,
        domain_id: Uuid,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<PeerAuthorityMaterial> {
        if expires_at <= Utc::now() {
            return Err(DdsP2pError::InvalidExpiration);
        }
        let verification_keys = self.fetch_verification_keys().await?;
        let credential =
            SignedP2pCredential::new(token.to_owned()).map_err(DdsP2pError::InvalidAccessToken)?;
        Ok(PeerAuthorityMaterial {
            domain_id,
            peer_id: identity.peer_id(),
            verification_keys,
            credential,
            expires_at,
        })
    }

    pub async fn robot_p2p_token(&self, machine_token: &str) -> Result<RobotP2pToken> {
        let domain_id = robot_assignment_hint(machine_token)?;
        let response = self
            .http
            .post(self.endpoint(ROBOT_TOKEN_PATH))
            .bearer_auth(machine_token)
            .json(&RobotP2pTokenRequest { domain_id })
            .send()
            .await
            .map_err(DdsP2pError::Request)?;
        ensure_success(&response)?;
        let response: RobotP2pTokenResponse =
            response.json().await.map_err(DdsP2pError::Request)?;
        if response.p2p_access_token.is_empty() {
            return Err(DdsP2pError::MissingField("p2p_access_token"));
        }
        if response.p2p_access_expires_at <= Utc::now() {
            return Err(DdsP2pError::InvalidExpiration);
        }
        Ok(RobotP2pToken {
            domain_id,
            token: response.p2p_access_token,
            expires_at: response.p2p_access_expires_at,
        })
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut endpoint = self.base_url.clone();
        endpoint.set_path(path);
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        endpoint
    }
}

#[derive(Clone)]
pub struct PeerBindingClient {
    dds: DdsP2pClient,
    identity: PeerIdentityProof,
}

impl PeerBindingClient {
    pub fn new(dds: DdsP2pClient, identity: PeerIdentityProof) -> Self {
        Self { dds, identity }
    }

    pub fn peer_id(&self) -> PeerId {
        self.identity.peer_id()
    }

    pub async fn bind(&self, base: &AccessBundle) -> Result<AccessBundle> {
        let challenge = self
            .dds
            .create_challenge(base.token(), &self.identity)
            .await?;
        self.dds
            .verify_challenge(base.token(), &self.identity, challenge)
            .await
    }
}

fn ensure_success(response: &reqwest::Response) -> Result<()> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(DdsP2pError::UpstreamStatus(response.status()))
    }
}

fn robot_assignment_hint(token: &str) -> Result<Uuid> {
    // This decoded claim is only a request hint. DDS verifies the signed,
    // peer-bound Robot Bearer token and its persisted assignment before it
    // issues a P2P token.
    let mut segments = token.split('.');
    let _header = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or(DdsP2pError::MalformedMachineToken)?;
    let payload = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or(DdsP2pError::MalformedMachineToken)?;
    let _signature = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or(DdsP2pError::MalformedMachineToken)?;
    if segments.next().is_some() {
        return Err(DdsP2pError::MalformedMachineToken);
    }
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| DdsP2pError::MalformedMachineToken)?;
    let claims: RobotAssignmentClaims =
        serde_json::from_slice(&payload).map_err(|_| DdsP2pError::MalformedMachineToken)?;
    claims
        .assigned_domain_id
        .ok_or(DdsP2pError::MissingRobotAssignment)
}

#[derive(Serialize)]
struct ChallengeRequest {
    peer_id: String,
    public_key: String,
}

#[derive(Deserialize)]
struct ChallengeResponse {
    challenge_id: String,
    challenge: String,
    expires_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct VerifyRequest {
    challenge_id: String,
    signature: String,
}

#[derive(Deserialize)]
struct VerifyResponse {
    peer_id: String,
    access_token: String,
    access_expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct RobotAssignmentClaims {
    assigned_domain_id: Option<Uuid>,
}

#[derive(Serialize)]
struct RobotP2pTokenRequest {
    domain_id: Uuid,
}

#[derive(Deserialize)]
struct RobotP2pTokenResponse {
    p2p_access_token: String,
    p2p_access_expires_at: DateTime<Utc>,
}

/// Existing assignment-bound Robot token exchange response.
pub struct RobotP2pToken {
    pub domain_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Material for constructing a host runtime authority update, without owning it.
pub struct PeerAuthorityMaterial {
    pub domain_id: Uuid,
    pub peer_id: PeerId,
    pub verification_keys: DdsVerificationKeys,
    pub credential: SignedP2pCredential,
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DDS_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

    const SECOND_DDS_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEAxcARQLozLIqu/CFm6ub89EElhHX
O+4eTRPLA8IA+ibNtrfWbavOIYZEtwGneJvRTovHr5OUGFu3n/gXNqGbKw==
-----END PUBLIC KEY-----"#;

    fn response(
        generation: u64,
        current: &str,
        previous: Option<&str>,
    ) -> VerificationKeysResponse {
        let mut keys = vec![VerificationKeyResponse {
            id: verification_key_id(current).unwrap(),
            status: VerificationKeyStatus::Current,
            signing_method: "ES256".into(),
            public_key: current.into(),
        }];
        if let Some(previous) = previous {
            keys.push(VerificationKeyResponse {
                id: verification_key_id(previous).unwrap(),
                status: VerificationKeyStatus::Previous,
                signing_method: "ES256".into(),
                public_key: previous.into(),
            });
        }
        VerificationKeysResponse {
            version: 1,
            generation,
            previous_key_overlap_seconds: DDS_PREVIOUS_KEY_MIN_OVERLAP.as_secs(),
            keys,
        }
    }

    #[test]
    fn dds_generations_drive_rotation_restart_and_retirement_shape() {
        let initial = convert_verification_keys(response(41, TEST_DDS_PUBLIC_KEY, None)).unwrap();
        let rotated = convert_verification_keys(response(
            42,
            SECOND_DDS_PUBLIC_KEY,
            Some(TEST_DDS_PUBLIC_KEY),
        ))
        .unwrap();
        let retired = convert_verification_keys(response(43, SECOND_DDS_PUBLIC_KEY, None)).unwrap();

        initial.validate_successor(&rotated).unwrap();
        rotated.validate_successor(&retired).unwrap();

        let initial_verifier = DdsTokenVerifier::from_keys(initial).unwrap();
        assert_eq!(initial_verifier.generation(), 41);
        let restarted = DdsTokenVerifier::from_keys(rotated).unwrap();
        assert_eq!(restarted.generation(), 42);
    }

    #[test]
    fn malformed_key_set_metadata_is_rejected() {
        let mut invalid = response(1, TEST_DDS_PUBLIC_KEY, None);
        invalid.previous_key_overlap_seconds = DDS_PREVIOUS_KEY_MIN_OVERLAP.as_secs() - 1;
        assert!(matches!(
            convert_verification_keys(invalid),
            Err(DdsP2pError::InvalidVerificationKeyResponse(_))
        ));

        let mut invalid = response(1, TEST_DDS_PUBLIC_KEY, None);
        invalid.keys[0].id = "a".repeat(64);
        assert!(matches!(
            convert_verification_keys(invalid),
            Err(DdsP2pError::InvalidVerificationKeyResponse(_))
        ));

        let mut invalid = response(1, TEST_DDS_PUBLIC_KEY, None);
        invalid.keys[0].signing_method = "RS256".into();
        assert!(matches!(
            convert_verification_keys(invalid),
            Err(DdsP2pError::InvalidVerificationKeyResponse(_))
        ));
    }
}
