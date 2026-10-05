//! Isolated demo authority, not a replacement for DDS authentication.
//! Fresh issuer and peer keys on every run; listeners are loopback-only.

use anyhow::Result;
use auki_p2p::{
    P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_SCOPE, P2P_TOKEN_TYPE, P2PAccessClaims,
};
use auki_sdk::{
    AukiPeer, AukiPeerConfig, DdsVerificationKeys, ExternalAuthorityUpdate, Identity,
    SignedP2pCredential,
};
use chrono::{TimeZone, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use p256::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use uuid::Uuid;

pub struct LocalPair {
    pub a: AukiPeer,
    pub b: AukiPeer,
}

impl LocalPair {
    pub async fn start() -> Result<Self> {
        let issuer = p256::SecretKey::random(&mut rand::rngs::OsRng);
        let private_pem = issuer.to_pkcs8_pem(LineEnding::LF)?;
        let public_pem = issuer.public_key().to_public_key_pem(LineEnding::LF)?;
        let signing = EncodingKey::from_ec_pem(private_pem.as_bytes())?;
        let domain = Uuid::new_v4();
        let a = start_peer(domain, &signing, public_pem.as_bytes()).await?;
        let b = match start_peer(domain, &signing, public_pem.as_bytes()).await {
            Ok(peer) => peer,
            Err(error) => {
                a.shutdown().await?;
                return Err(error);
            }
        };
        Ok(Self { a, b })
    }

    pub async fn shutdown(self) -> Result<()> {
        let (a, b) = tokio::join!(self.a.shutdown(), self.b.shutdown());
        a?;
        b?;
        Ok(())
    }
}

async fn start_peer(domain: Uuid, signing: &EncodingKey, public_pem: &[u8]) -> Result<AukiPeer> {
    let identity = Identity::generate();
    let now = Utc::now().timestamp() as u64;
    let expires_at = now + 30 * 60;
    let claims = P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.into(),
        iss: P2P_TOKEN_ISSUER.into(),
        aud: vec![P2P_TOKEN_AUDIENCE.into()],
        sub: Uuid::new_v4().to_string(),
        organization_id: None,
        peer_type: Some("test".into()),
        peer_id: identity.peer_id().to_string(),
        domain_ids: vec![domain.to_string()],
        scopes: vec![P2P_TOKEN_SCOPE.into()],
        application: None,
        iat: now,
        nbf: None,
        exp: expires_at,
    };
    let update = ExternalAuthorityUpdate::new(
        domain,
        identity.peer_id(),
        DdsVerificationKeys::new(0, public_pem.to_vec(), None),
        SignedP2pCredential::new(encode(&Header::new(Algorithm::ES256), &claims, signing)?)?,
        Utc.timestamp_opt(expires_at as i64, 0).unwrap(),
    );
    // This URL is not contacted: relays and discovery are disabled. No login,
    // remote credentials, public listener, or shared service fallback exists.
    let config = AukiPeerConfig::new("http://127.0.0.1:9")?
        .direct_only()
        .with_listen_addresses(["/ip4/127.0.0.1/tcp/0".parse()?])?;
    let (peer, _authority) = AukiPeer::start_external(identity, update, config).await?;
    Ok(peer)
}
