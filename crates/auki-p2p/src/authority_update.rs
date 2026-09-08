//! Complete peer-bound DDS authority material shared by platform facades.

use std::fmt;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{DdsVerificationKeys, PeerId, SignedP2pCredential};

/// One complete, peer- and Domain-pinned DDS authority replacement.
///
/// The credential and verification-key bytes are intentionally omitted from
/// debug output and public getters. Authentication adapters transfer complete
/// updates into a runtime instead of exposing partial live mutation.
pub struct PeerAuthorityUpdate {
    pub(crate) domain_id: Uuid,
    pub(crate) peer_id: PeerId,
    pub(crate) verification_keys: DdsVerificationKeys,
    pub(crate) credential: SignedP2pCredential,
    pub(crate) credential_expires_at: DateTime<Utc>,
}

impl PeerAuthorityUpdate {
    pub fn new(
        domain_id: Uuid,
        peer_id: PeerId,
        verification_keys: DdsVerificationKeys,
        credential: SignedP2pCredential,
        credential_expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            domain_id,
            peer_id,
            verification_keys,
            credential,
            credential_expires_at,
        }
    }

    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn verification_key_generation(&self) -> u64 {
        self.verification_keys.generation()
    }

    pub fn credential_expires_at(&self) -> DateTime<Utc> {
        self.credential_expires_at
    }
}

impl fmt::Debug for PeerAuthorityUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerAuthorityUpdate")
            .field("domain_id", &self.domain_id)
            .field("peer_id", &self.peer_id)
            .field(
                "verification_key_generation",
                &self.verification_keys.generation(),
            )
            .field("credential", &"[redacted]")
            .field("verification_keys", &"[redacted]")
            .field("credential_expires_at", &self.credential_expires_at)
            .finish()
    }
}
