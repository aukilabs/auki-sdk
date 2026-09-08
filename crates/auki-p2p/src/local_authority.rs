//! Runtime-neutral installation gate for complete DDS authority updates.

use std::sync::Arc;

use chrono::{Timelike, Utc};
use futures::lock::Mutex;
use uuid::Uuid;

use crate::{
    token::{ensure_token_domain, ensure_token_peer, TokenStore},
    DdsTokenVerifier, Error, P2PAccessClaims, PeerAuthorityUpdate, PeerId, Result,
};

#[derive(Clone)]
pub(crate) struct LocalAuthority {
    inner: Arc<LocalAuthorityInner>,
}

struct LocalAuthorityInner {
    peer_id: PeerId,
    domain_id: Uuid,
    verifier: DdsTokenVerifier,
    tokens: TokenStore,
    updates: Mutex<()>,
}

impl LocalAuthority {
    pub(crate) async fn start(peer_id: PeerId, update: PeerAuthorityUpdate) -> Result<Self> {
        validate_envelope(peer_id, update.domain_id, &update)?;
        let expected_expiration = exact_future_expiration(update.credential_expires_at)?;
        let verifier = DdsTokenVerifier::from_keys(update.verification_keys)?;
        validate_signed_authority(
            &verifier,
            &update.credential,
            peer_id,
            update.domain_id,
            expected_expiration,
        )?;
        let tokens = TokenStore::default();
        tokens
            .install(
                update.credential,
                &verifier,
                peer_id,
                Some(expected_expiration),
                Some(update.domain_id),
            )
            .await?;
        Ok(Self {
            inner: Arc::new(LocalAuthorityInner {
                peer_id,
                domain_id: update.domain_id,
                verifier,
                tokens,
                updates: Mutex::new(()),
            }),
        })
    }

    pub(crate) fn peer_id(&self) -> PeerId {
        self.inner.peer_id
    }

    pub(crate) fn domain_id(&self) -> Uuid {
        self.inner.domain_id
    }

    pub(crate) async fn replace(&self, update: &PeerAuthorityUpdate) -> Result<P2PAccessClaims> {
        let _update = self.inner.updates.lock().await;
        validate_envelope(self.inner.peer_id, self.inner.domain_id, update)?;
        let expected_expiration = exact_future_expiration(update.credential_expires_at)?;

        // Reject a malformed or mismatched complete bundle before changing the
        // live key ring. The live verifier then independently enforces key-set
        // lineage and overlap during the atomic replacement below.
        let candidate = DdsTokenVerifier::from_keys(update.verification_keys.clone())?;
        validate_signed_authority(
            &candidate,
            &update.credential,
            self.inner.peer_id,
            self.inner.domain_id,
            expected_expiration,
        )?;

        self.inner
            .verifier
            .replace_keys(update.verification_keys.clone())?;
        self.inner
            .tokens
            .install(
                update.credential.clone(),
                &self.inner.verifier,
                self.inner.peer_id,
                Some(expected_expiration),
                Some(self.inner.domain_id),
            )
            .await
    }

    pub(crate) async fn clear(&self) {
        let _update = self.inner.updates.lock().await;
        self.inner.tokens.clear().await;
    }

    pub(crate) fn verifier(&self) -> DdsTokenVerifier {
        self.inner.verifier.clone()
    }

    pub(crate) fn tokens(&self) -> TokenStore {
        self.inner.tokens.clone()
    }

    #[cfg(test)]
    async fn current_claims(&self) -> Option<P2PAccessClaims> {
        self.inner
            .tokens
            .snapshot_with_claims()
            .await
            .map(|(_, claims)| claims)
    }
}

fn validate_envelope(
    expected_peer_id: PeerId,
    expected_domain_id: Uuid,
    update: &PeerAuthorityUpdate,
) -> Result<()> {
    if update.peer_id != expected_peer_id {
        return Err(Error::AuthorityPeerMismatch {
            expected: expected_peer_id.to_string(),
            actual: update.peer_id.to_string(),
        });
    }
    if update.domain_id != expected_domain_id {
        return Err(Error::AuthorityDomainMismatch {
            expected: expected_domain_id.to_string(),
            actual: update.domain_id.to_string(),
        });
    }
    Ok(())
}

fn exact_future_expiration(expiration: chrono::DateTime<Utc>) -> Result<u64> {
    if expiration <= Utc::now() || expiration.nanosecond() != 0 {
        return Err(Error::InvalidAuthorityExpiration);
    }
    u64::try_from(expiration.timestamp()).map_err(|_| Error::InvalidAuthorityExpiration)
}

fn validate_signed_authority(
    verifier: &DdsTokenVerifier,
    credential: &crate::SignedP2pCredential,
    peer_id: PeerId,
    domain_id: Uuid,
    expected_expiration: u64,
) -> Result<()> {
    let claims = verifier.verify_credential(credential)?;
    ensure_token_peer(&claims, peer_id)?;
    ensure_token_domain(&claims, domain_id)?;
    if claims.exp != expected_expiration {
        return Err(Error::CredentialExpirationMismatch {
            credential_expiration: claims.exp,
            expected_expiration,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    use crate::{
        token::unix_time_now, DdsVerificationKeys, Identity, P2PAccessClaims, PeerAuthorityUpdate,
        SignedP2pCredential, P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE,
    };

    use super::*;

    const PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

    const PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

    #[tokio::test]
    async fn complete_updates_advance_one_shared_authority() {
        let identity = Identity::generate();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time_now();
        let initial = update(identity.peer_id(), domain_id, issued_at);
        let authority = LocalAuthority::start(identity.peer_id(), initial)
            .await
            .unwrap();
        let clone = authority.clone();

        let replacement = update(identity.peer_id(), domain_id, issued_at + 1);
        let claims = clone.replace(&replacement).await.unwrap();

        assert_eq!(claims.iat, issued_at + 1);
        assert_eq!(authority.current_claims().await.unwrap(), claims);
        assert_eq!(clone.peer_id(), identity.peer_id());
        assert_eq!(clone.domain_id(), domain_id);
    }

    #[tokio::test]
    async fn rejected_update_preserves_the_installed_credential() {
        let identity = Identity::generate();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time_now();
        let authority = LocalAuthority::start(
            identity.peer_id(),
            update(identity.peer_id(), domain_id, issued_at),
        )
        .await
        .unwrap();

        let mut mismatched = update(identity.peer_id(), domain_id, issued_at + 1);
        mismatched.credential_expires_at += chrono::Duration::seconds(1);
        assert!(matches!(
            authority.replace(&mismatched).await,
            Err(Error::CredentialExpirationMismatch { .. })
        ));
        assert_eq!(authority.current_claims().await.unwrap().iat, issued_at);

        authority.clear().await;
        assert!(authority.current_claims().await.is_none());
    }

    #[tokio::test]
    async fn envelope_and_signed_identity_are_pinned_without_debug_leaks() {
        let identity = Identity::generate();
        let other = Identity::generate();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time_now();

        let wrong_envelope = update(other.peer_id(), domain_id, issued_at);
        assert!(matches!(
            LocalAuthority::start(identity.peer_id(), wrong_envelope).await,
            Err(Error::AuthorityPeerMismatch { .. })
        ));

        let mut wrong_signed_peer = update(other.peer_id(), domain_id, issued_at);
        wrong_signed_peer.peer_id = identity.peer_id();
        let rendered = format!("{wrong_signed_peer:?}");
        assert!(!rendered.contains("PRIVATE KEY"));
        assert!(!rendered.contains("PUBLIC KEY"));
        assert!(!rendered.contains("eyJ"));
        assert!(matches!(
            LocalAuthority::start(identity.peer_id(), wrong_signed_peer).await,
            Err(Error::PeerIdMismatch { .. })
        ));
    }

    fn update(peer_id: PeerId, domain_id: Uuid, issued_at: u64) -> PeerAuthorityUpdate {
        let expiration = issued_at + P2P_TOKEN_TTL.as_secs();
        let claims = P2PAccessClaims {
            token_type: P2P_TOKEN_TYPE.into(),
            iss: P2P_TOKEN_ISSUER.into(),
            aud: vec![P2P_TOKEN_AUDIENCE.into()],
            sub: Uuid::new_v4().to_string(),
            organization_id: None,
            peer_type: Some("browser".into()),
            peer_id: peer_id.to_string(),
            domain_ids: vec![domain_id.to_string()],
            scopes: Vec::new(),
            application: None,
            iat: issued_at,
            nbf: None,
            exp: expiration,
        };
        let compact = encode(
            &Header::new(Algorithm::ES256),
            &claims,
            &EncodingKey::from_ec_pem(PRIVATE_KEY).unwrap(),
        )
        .unwrap();
        PeerAuthorityUpdate::new(
            domain_id,
            peer_id,
            DdsVerificationKeys::new(1, PUBLIC_KEY.to_vec(), None),
            SignedP2pCredential::new(compact).unwrap(),
            Utc.timestamp_opt(expiration as i64, 0).unwrap(),
        )
    }
}
