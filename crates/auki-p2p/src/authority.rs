use chrono::{DateTime, Timelike, Utc};
use uuid::Uuid;

use crate::{
    DdsVerificationKeys, Error, Node, P2PAccessClaims, PeerId, Result, SignedP2pCredential,
    transport::CurrentCredentialStatus,
};

/// Fail-closed errors for the process-local authenticated P2P authority.
#[derive(Debug, thiserror::Error)]
pub enum P2pCredentialError {
    #[error("DDS P2P access token is invalid")]
    InvalidAccessToken(#[source] Error),
    #[error("DDS P2P access-token expiration is invalid or inconsistent")]
    InvalidExpiration,
    #[error("no current DDS P2P credential is installed")]
    MissingCredential,
    #[error("the current DDS P2P credential has expired")]
    ExpiredCredential,
    #[error("the current DDS P2P credential does not authorize the required Domain")]
    CredentialDomainMismatch,
}

pub type P2pCredentialResult<T> = std::result::Result<T, P2pCredentialError>;

/// The single narrow host authority for every application protocol on one Node.
///
/// Credential and verification-key acquisition remain the host application's
/// responsibility. This handle can only install that material, inspect
/// validated local diagnostics, and produce the public proof material required
/// by DDS's Peer-ID challenge. It cannot open streams, control the swarm, or
/// expose private identity bytes.
#[derive(Clone)]
pub struct DomainAuthority {
    node: Node,
}

impl std::fmt::Debug for DomainAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DomainAuthority")
            .field("peer_id", &self.node.peer_id())
            .field("credential", &"[redacted]")
            .field("private_identity", &"[redacted]")
            .finish()
    }
}

impl DomainAuthority {
    pub(crate) fn new(node: Node) -> Self {
        Self { node }
    }

    pub fn peer_id(&self) -> PeerId {
        self.node.peer_id()
    }

    /// Verify and atomically install newer local DDS authority.
    ///
    /// All authority handles constructed from clones of the same [`Node`] observe the
    /// same validated state. Older replacements and same-issued conflicting
    /// claims are rejected without disturbing the current credential.
    pub async fn install_verification_keys(&self, keys: DdsVerificationKeys) -> Result<()> {
        self.node.install_verification_keys(keys).await
    }

    pub async fn install_credential(
        &self,
        credential: SignedP2pCredential,
    ) -> P2pCredentialResult<P2PAccessClaims> {
        self.node
            .install_credential(credential)
            .await
            .map_err(P2pCredentialError::InvalidAccessToken)
    }

    /// Verify and install a credential only when it authorizes one exact
    /// Domain. The Domain check and monotonic credential replacement share the
    /// Node's authority-update critical section, so a valid token for another
    /// Domain cannot transiently replace the current credential.
    pub async fn install_credential_for_domain(
        &self,
        credential: SignedP2pCredential,
        domain_id: Uuid,
    ) -> P2pCredentialResult<P2PAccessClaims> {
        self.node
            .install_credential_for_domain(credential, domain_id)
            .await
            .map_err(|error| match error {
                Error::LocalDomainMismatch(_) => P2pCredentialError::CredentialDomainMismatch,
                error => P2pCredentialError::InvalidAccessToken(error),
            })
    }

    /// Install a credential only when its verified signed `exp` exactly
    /// matches the expiration returned alongside it by the host's DDS adapter.
    /// The comparison happens before the same atomic monotonic replacement as
    /// [`Self::install_credential`], so a mismatched DTO cannot clear or
    /// replace authority.
    pub async fn install_credential_checked(
        &self,
        credential: SignedP2pCredential,
        expected_expires_at: DateTime<Utc>,
    ) -> P2pCredentialResult<P2PAccessClaims> {
        if expected_expires_at <= Utc::now() || expected_expires_at.nanosecond() != 0 {
            return Err(P2pCredentialError::InvalidExpiration);
        }
        let expected_expiration = u64::try_from(expected_expires_at.timestamp())
            .map_err(|_| P2pCredentialError::InvalidExpiration)?;
        self.node
            .install_credential_checked(credential, expected_expiration)
            .await
            .map_err(|error| match error {
                Error::CredentialExpirationMismatch { .. } => P2pCredentialError::InvalidExpiration,
                error => P2pCredentialError::InvalidAccessToken(error),
            })
    }

    /// Public identity material required when requesting a DDS challenge.
    pub fn peer_public_key_protobuf(&self) -> Vec<u8> {
        self.node.peer_public_key_protobuf()
    }

    /// Sign the exact DDS challenge without exposing the Node's private key.
    pub fn sign_peer_challenge(&self, challenge: &[u8]) -> Result<Vec<u8>> {
        self.node.sign_peer_challenge(challenge)
    }

    pub async fn require(&self, domain_id: Uuid) -> P2pCredentialResult<P2PAccessClaims> {
        let claims = match self
            .node
            .current_credential_status()
            .await
            .map_err(P2pCredentialError::InvalidAccessToken)?
        {
            CurrentCredentialStatus::Missing => return Err(P2pCredentialError::MissingCredential),
            CurrentCredentialStatus::Expired => return Err(P2pCredentialError::ExpiredCredential),
            CurrentCredentialStatus::Verified(claims) => *claims,
        };
        if !claims
            .domain_ids
            .iter()
            .filter_map(|candidate| Uuid::parse_str(candidate).ok())
            .any(|candidate| candidate == domain_id)
        {
            return Err(P2pCredentialError::CredentialDomainMismatch);
        }
        Ok(claims)
    }

    pub async fn current_claims(&self) -> Option<P2PAccessClaims> {
        match self.node.current_credential_status().await {
            Ok(CurrentCredentialStatus::Verified(claims)) => Some(*claims),
            Ok(CurrentCredentialStatus::Missing | CurrentCredentialStatus::Expired) | Err(_) => {
                None
            }
        }
    }

    pub async fn clear_credential(&self) {
        self.node.clear_credential().await;
    }
}
