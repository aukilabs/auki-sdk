use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(crate) struct LoginRequest<'a> {
    pub email: &'a str,
    pub password: &'a str,
}

#[derive(Deserialize)]
pub(crate) struct ApiTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Deserialize)]
pub(crate) struct ServiceTokenResponse {
    pub access_token: String,
}

#[derive(Deserialize)]
pub(crate) struct AccessibleDomainsResponse {
    pub domains: Vec<AccessibleDomain>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Deserialize)]
pub(crate) struct AccessibleDomain {
    pub id: String,
    pub name: String,
    pub description: String,
    pub organization_id: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct PeerChallengeRequest<'a> {
    pub peer_id: &'a str,
    pub public_key: &'a str,
}

#[derive(Deserialize)]
pub(crate) struct PeerChallengeResponse {
    pub challenge_id: String,
    pub challenge: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub(crate) struct PeerVerifyRequest<'a> {
    pub challenge_id: &'a str,
    pub signature: &'a str,
}

#[derive(Deserialize)]
pub(crate) struct PeerVerifyResponse {
    pub peer_id: String,
    pub domain_id: String,
    pub peer_type: String,
    pub p2p_access_token: String,
    pub p2p_access_expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub(crate) struct VerificationKeysResponse {
    pub version: u8,
    pub generation: u64,
    pub previous_key_overlap_seconds: u64,
    pub keys: Vec<VerificationKey>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VerificationKeyStatus {
    Current,
    Previous,
}

#[derive(Deserialize)]
pub(crate) struct VerificationKey {
    pub id: String,
    pub status: VerificationKeyStatus,
    pub signing_method: String,
    pub public_key: String,
}
