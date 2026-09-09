//! Browser handle for the runtime-neutral local DDS authority gate.

use std::fmt;

use uuid::Uuid;

use crate::{
    local_authority::LocalAuthority, DdsTokenVerifier, P2PAccessClaims, PeerAuthorityUpdate,
    PeerId, Result,
};

/// Cloneable authority handle for one fixed browser Peer ID and Domain.
///
/// Renewal can replace a complete authority update while the browser swarm
/// continues to drive relay and application streams. DDS requests and renewal
/// scheduling remain outside `auki-p2p`.
#[derive(Clone)]
pub struct BrowserAuthority {
    inner: LocalAuthority,
}

impl BrowserAuthority {
    pub(crate) async fn start(peer_id: PeerId, update: PeerAuthorityUpdate) -> Result<Self> {
        Ok(Self {
            inner: LocalAuthority::start(peer_id, update).await?,
        })
    }

    pub fn peer_id(&self) -> PeerId {
        self.inner.peer_id()
    }

    pub fn domain_id(&self) -> Uuid {
        self.inner.domain_id()
    }

    /// Validate and install one complete authority update.
    ///
    /// The update is borrowed so a facade can retain and retry the exact value
    /// if local installation fails after its renewal provider has advanced.
    pub async fn replace(&self, update: &PeerAuthorityUpdate) -> Result<P2PAccessClaims> {
        self.inner.replace(update).await
    }

    pub async fn clear(&self) {
        self.inner.clear().await;
    }

    pub(crate) fn verifier(&self) -> DdsTokenVerifier {
        self.inner.verifier()
    }

    pub(crate) fn tokens(&self) -> crate::token::TokenStore {
        self.inner.tokens()
    }
}

impl fmt::Debug for BrowserAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserAuthority")
            .field("peer_id", &self.peer_id())
            .field("domain_id", &self.domain_id())
            .field("credential", &"[redacted]")
            .field("private_identity", &"[redacted]")
            .finish()
    }
}
