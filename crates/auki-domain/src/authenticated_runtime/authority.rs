use std::sync::Arc;

use auki_p2p::{DdsVerificationKeys, P2pCredentialError, SignedP2pCredential};
use chrono::{DateTime, Utc};

use super::{RuntimeAccess, RuntimeAccessError};

/// Domain-scoped host authority. It can install host-fetched trust material
/// and sign DDS challenges, but cannot open streams or control the node.
#[derive(Clone)]
pub(crate) struct DomainAuthority {
    access: Arc<RuntimeAccess>,
}

impl std::fmt::Debug for DomainAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DomainAuthority")
            .field("peer_id", &self.access.peer_id)
            .field("domain_id", &self.access.domain_id)
            .field("credential", &"[redacted]")
            .field("private_identity", &"[redacted]")
            .finish()
    }
}

impl DomainAuthority {
    pub(super) fn new(access: Arc<RuntimeAccess>) -> Self {
        Self { access }
    }

    pub(crate) fn peer_id(&self) -> auki_p2p::PeerId {
        self.access.peer_id
    }

    pub(crate) fn domain_id(&self) -> uuid::Uuid {
        self.access.domain_id
    }

    pub(crate) async fn install_verification_keys(
        &self,
        keys: DdsVerificationKeys,
    ) -> Result<(), DomainAuthorityError> {
        let node = self.access.node()?;
        let authority = node.authority();
        let refresh_started_at = tokio::time::Instant::now();
        tokio::select! {
            biased;
            _ = self.access.lifecycle.cancelled() => Err(DomainAuthorityError::Stopped),
            result = authority.install_verification_keys(keys) => {
                result.map_err(DomainAuthorityError::P2p)?;
                self.access
                    .status
                    .refresh_verification_keys(refresh_started_at);
                self.recompute_status(&node).await;
                Ok(())
            }
        }
    }

    pub(crate) async fn install_credential(
        &self,
        credential: SignedP2pCredential,
    ) -> Result<(), DomainAuthorityError> {
        let node = self.access.node()?;
        let domain_id = self.access.domain_id;
        let authority = node.authority();
        let claims = tokio::select! {
            biased;
            _ = self.access.lifecycle.cancelled() => return Err(DomainAuthorityError::Stopped),
            result = authority.install_credential_for_domain(credential, domain_id) => {
                result.map_err(DomainAuthorityError::Credential)?
            }
        };
        let deadline = claims_deadline(claims.exp)?;
        self.access.status.set_credential_deadline(Some(deadline));
        Ok(())
    }

    pub(crate) fn peer_public_key_protobuf(&self) -> Result<Vec<u8>, DomainAuthorityError> {
        let node = self.access.node()?;
        Ok(node.authority().peer_public_key_protobuf())
    }

    pub(crate) fn sign_peer_challenge(
        &self,
        challenge: &[u8],
    ) -> Result<Vec<u8>, DomainAuthorityError> {
        let node = self.access.node()?;
        node.authority()
            .sign_peer_challenge(challenge)
            .map_err(DomainAuthorityError::P2p)
    }

    async fn recompute_status(&self, node: &auki_p2p::Node) {
        let deadline = match node.authority().require(self.access.domain_id).await {
            Ok(claims) => claims_deadline(claims.exp).ok(),
            Err(_) => None,
        };
        self.access.status.set_credential_deadline(deadline);
    }
}

fn claims_deadline(expiration: u64) -> Result<DateTime<Utc>, DomainAuthorityError> {
    let expiration = i64::try_from(expiration).map_err(|_| DomainAuthorityError::InvalidExpiry)?;
    DateTime::from_timestamp(expiration, 0).ok_or(DomainAuthorityError::InvalidExpiry)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DomainAuthorityError {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("the signed credential has an invalid expiration")]
    InvalidExpiry,
    #[error("Domain credential installation failed: {0}")]
    Credential(#[source] P2pCredentialError),
    #[error("Domain authority operation failed: {0}")]
    P2p(#[source] auki_p2p::Error),
}

impl From<RuntimeAccessError> for DomainAuthorityError {
    fn from(_: RuntimeAccessError) -> Self {
        Self::Stopped
    }
}
