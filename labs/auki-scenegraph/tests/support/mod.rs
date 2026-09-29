// Isolated test-only signing fixture, copied from the Component protocol tests.
use auki_p2p::{
    P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_SCOPE, P2P_TOKEN_TYPE, P2PAccessClaims,
};
use auki_sdk::{
    AukiPeerConfig, DdsVerificationKeys, ExternalAuthorityUpdate, Identity, Multiaddr,
    SignedP2pCredential,
};
use chrono::{TimeZone, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use std::str::FromStr;
use std::time::SystemTime;
use uuid::Uuid;
const TEST_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

const TEST_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

pub fn authority(identity: &Identity, domain_id: Uuid) -> ExternalAuthorityUpdate {
    authority_at(identity, domain_id, Uuid::new_v4(), unix_now() - 60)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn authority_at(
    identity: &Identity,
    domain_id: Uuid,
    subject: Uuid,
    issued_at: u64,
) -> ExternalAuthorityUpdate {
    let expires_at = issued_at + 30 * 60;
    let claims = P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.to_owned(),
        iss: P2P_TOKEN_ISSUER.to_owned(),
        aud: vec![P2P_TOKEN_AUDIENCE.to_owned()],
        sub: subject.to_string(),
        organization_id: None,
        peer_type: Some("test".to_owned()),
        peer_id: identity.peer_id().to_string(),
        domain_ids: vec![domain_id.to_string()],
        scopes: vec![P2P_TOKEN_SCOPE.to_owned()],
        application: None,
        iat: issued_at,
        nbf: None,
        exp: expires_at,
    };
    let compact = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY).unwrap(),
    )
    .unwrap();
    ExternalAuthorityUpdate::new(
        domain_id,
        identity.peer_id(),
        DdsVerificationKeys::new(0, TEST_PUBLIC_KEY.to_vec(), None),
        SignedP2pCredential::new(compact).unwrap(),
        Utc.timestamp_opt(expires_at as i64, 0).unwrap(),
    )
}

pub fn direct_config() -> AukiPeerConfig {
    AukiPeerConfig::new("http://127.0.0.1:9")
        .unwrap()
        .direct_only()
        .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
        .unwrap()
}
