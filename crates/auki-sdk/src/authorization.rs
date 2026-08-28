use std::sync::Arc;

use auki_p2p::P2PAccessClaims;
use chrono::{DateTime, Utc};

use crate::context::ContextLifecycle;

/// Non-secret, readiness-fenced view of the current local DDS authority.
///
/// Signed claims are safe to inspect but cannot be used to mutate authority or
/// render the bearer credential. A protocol adapter should request a fresh
/// snapshot for every operation whose capability policy depends on these
/// claims; retained snapshots are diagnostics, not proof of current readiness.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AukiPeerAuthorizationSnapshot {
    credential_revision: u64,
    expires_at: DateTime<Utc>,
    claims: P2PAccessClaims,
}

impl AukiPeerAuthorizationSnapshot {
    #[allow(dead_code)] // Produced by the AuthoritySupervisor adapter in the next slice.
    pub(crate) fn new(
        credential_revision: u64,
        expires_at: DateTime<Utc>,
        claims: P2PAccessClaims,
    ) -> Self {
        Self {
            credential_revision,
            expires_at,
            claims,
        }
    }

    /// Process-local monotonic revision of the accepted signed credential.
    pub fn credential_revision(&self) -> u64 {
        self.credential_revision
    }

    /// Literal signed credential expiration.
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Verified non-secret DDS P2P claims.
    pub fn claims(&self) -> &P2PAccessClaims {
        &self.claims
    }

    /// Convenience accessor for the current bounded `peer_type` claim.
    pub fn peer_type(&self) -> Option<&str> {
        self.claims.peer_type.as_deref()
    }

    /// Current bounded application capability scopes.
    pub fn scopes(&self) -> &[String] {
        &self.claims.scopes
    }
}

pub(crate) trait AuthorizationSnapshotSource: Send + Sync {
    fn current(&self) -> Result<AukiPeerAuthorizationSnapshot, AukiPeerAuthorizationError>;
}

/// Cloneable read capability for current local signed authorization metadata.
///
/// This handle never exposes verification-key installation, bearer material,
/// or external refresh control.
#[derive(Clone)]
pub struct AukiPeerAuthorization {
    source: Arc<dyn AuthorizationSnapshotSource>,
    lifecycle: ContextLifecycle,
}

impl AukiPeerAuthorization {
    pub(crate) fn new(
        source: Arc<dyn AuthorizationSnapshotSource>,
        lifecycle: ContextLifecycle,
    ) -> Self {
        Self { source, lifecycle }
    }

    /// Read the exact current authorization snapshot.
    ///
    /// The call fails when authority is starting, unavailable, expired, or
    /// stopped. Expiration is checked again against wall time after reading the
    /// source so an operation never accepts a snapshot past its literal fence.
    pub fn current(&self) -> Result<AukiPeerAuthorizationSnapshot, AukiPeerAuthorizationError> {
        let _running = self
            .lifecycle
            .enter()
            .ok_or(AukiPeerAuthorizationError::Stopped)?;
        let snapshot = self.source.current()?;
        if snapshot.expires_at <= Utc::now() {
            Err(AukiPeerAuthorizationError::Expired)
        } else {
            Ok(snapshot)
        }
    }
}

/// Why no current local authorization snapshot can be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AukiPeerAuthorizationError {
    /// Authority is starting or temporarily unavailable.
    #[error("local peer authorization is unavailable")]
    Unavailable,
    /// The current signed credential reached its literal expiration.
    #[error("local peer authorization is expired")]
    Expired,
    /// The owning peer runtime has stopped.
    #[error("local peer authorization is stopped")]
    Stopped,
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::Duration;
    use uuid::Uuid;

    use super::*;

    struct FakeSource(Mutex<Result<AukiPeerAuthorizationSnapshot, AukiPeerAuthorizationError>>);

    impl AuthorizationSnapshotSource for FakeSource {
        fn current(&self) -> Result<AukiPeerAuthorizationSnapshot, AukiPeerAuthorizationError> {
            self.0.lock().unwrap().clone()
        }
    }

    fn claims(peer_type: Option<&str>) -> P2PAccessClaims {
        P2PAccessClaims {
            token_type: "p2p-access".into(),
            iss: "dds".into(),
            aud: vec!["auki-p2p".into()],
            sub: "principal".into(),
            organization_id: Some("organization".into()),
            peer_type: peer_type.map(str::to_owned),
            peer_id: "peer".into(),
            domain_ids: vec![Uuid::nil().to_string()],
            scopes: vec!["dataset:read".into()],
            application: None,
            iat: 1,
            nbf: None,
            exp: u64::MAX,
        }
    }

    #[test]
    fn current_exposes_only_non_secret_claim_metadata() {
        let expires_at = Utc::now() + Duration::minutes(5);
        let source = Arc::new(FakeSource(Mutex::new(Ok(
            AukiPeerAuthorizationSnapshot::new(7, expires_at, claims(Some("robot"))),
        ))));
        let authorization = AukiPeerAuthorization::new(source, ContextLifecycle::new());
        let current = authorization.current().unwrap();
        assert_eq!(current.credential_revision(), 7);
        assert_eq!(current.expires_at(), expires_at);
        assert_eq!(current.peer_type(), Some("robot"));
        assert_eq!(current.scopes(), ["dataset:read"]);
    }

    #[test]
    fn literal_expiration_is_rechecked_by_the_public_handle() {
        let source = Arc::new(FakeSource(Mutex::new(Ok(
            AukiPeerAuthorizationSnapshot::new(
                1,
                Utc::now() - Duration::seconds(1),
                claims(Some("compute")),
            ),
        ))));
        let authorization = AukiPeerAuthorization::new(source, ContextLifecycle::new());
        assert_eq!(
            authorization.current().unwrap_err(),
            AukiPeerAuthorizationError::Expired
        );
    }

    #[test]
    fn source_unavailability_is_preserved() {
        let source = Arc::new(FakeSource(Mutex::new(Err(
            AukiPeerAuthorizationError::Unavailable,
        ))));
        let authorization = AukiPeerAuthorization::new(source, ContextLifecycle::new());
        assert_eq!(
            authorization.current().unwrap_err(),
            AukiPeerAuthorizationError::Unavailable
        );
    }

    #[test]
    fn context_fence_precedes_underlying_authority_shutdown() {
        let expires_at = Utc::now() + Duration::minutes(5);
        let source = Arc::new(FakeSource(Mutex::new(Ok(
            AukiPeerAuthorizationSnapshot::new(1, expires_at, claims(Some("robot"))),
        ))));
        let lifecycle = ContextLifecycle::new();
        let authorization = AukiPeerAuthorization::new(source, lifecycle.clone());
        assert!(authorization.current().is_ok());

        lifecycle.fence();
        assert_eq!(
            authorization.current().unwrap_err(),
            AukiPeerAuthorizationError::Stopped
        );
    }
}
