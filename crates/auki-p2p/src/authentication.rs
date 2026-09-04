use std::{future::Future, time::Duration};

use chrono::Utc;
use futures::{
    future::{select, Either},
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    pin_mut,
};
use futures_timer::Delay;
use libp2p_identity::PeerId;
use uuid::Uuid;

use crate::{
    token::{
        ensure_literal_expiry, ensure_token_peer, unix_time_now, DdsTokenVerifier, P2PAccessClaims,
        SignedApplicationMetadata, SignedP2pCredential, TokenStore, P2P_TOKEN_MAX_BYTES,
    },
    Error, Result,
};

const AUTHENTICATION_TIMEOUT: Duration = Duration::from_secs(10);
const AUTH_ACCEPTED: u8 = 1;
const AUTH_REJECTED: u8 = 0;

/// Domain and optional remote identity required by one authenticated stream.
#[derive(Clone, Debug)]
pub struct SessionRequirements {
    domain_id: Uuid,
    expected_remote_peer_id: Option<PeerId>,
}

impl SessionRequirements {
    pub fn new(domain_id: impl Into<String>) -> Result<Self> {
        let domain_id = domain_id.into();
        let parsed_domain_id = Uuid::parse_str(&domain_id)
            .map_err(|_| Error::InvalidToken("required Domain must be a UUID".into()))?;
        if parsed_domain_id.to_string() != domain_id {
            return Err(Error::InvalidToken(
                "required Domain must be a canonical UUID".into(),
            ));
        }
        Ok(Self {
            domain_id: parsed_domain_id,
            expected_remote_peer_id: None,
        })
    }

    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    pub fn expected_remote_peer_id(&self) -> Option<PeerId> {
        self.expected_remote_peer_id
    }

    pub fn with_expected_remote_peer_id(mut self, peer_id: PeerId) -> Self {
        self.expected_remote_peer_id = Some(peer_id);
        self
    }

    fn validate_remote(&self, claims: &P2PAccessClaims, noise_peer_id: PeerId) -> Result<()> {
        if let Some(expected) = self.expected_remote_peer_id {
            if expected != noise_peer_id {
                return Err(Error::UnexpectedRemotePeer {
                    expected: expected.to_string(),
                    actual: noise_peer_id.to_string(),
                });
            }
        }
        self.validate_domain(claims)
    }

    fn validate_domain(&self, claims: &P2PAccessClaims) -> Result<()> {
        if !claims
            .domain_ids
            .iter()
            .filter_map(|domain_id| Uuid::parse_str(domain_id).ok())
            .any(|domain_id| domain_id == self.domain_id)
        {
            return Err(Error::RemoteDomainMismatch(self.domain_id.to_string()));
        }
        Ok(())
    }
}

/// Verified identity and DDS authority of the peer at the far end of a stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedPeer {
    pub peer_id: PeerId,
    pub subject: Uuid,
    pub peer_type: Option<String>,
    pub domain_ids: Vec<Uuid>,
    pub scopes: Vec<String>,
    pub application: Option<SignedApplicationMetadata>,
    pub verified_until: chrono::DateTime<Utc>,
}

/// Apply the shared mutual DDS authentication conversation to any async byte stream.
///
/// Native and browser transports own stream creation. This function owns the
/// bounded authentication wire exchange and returns the unchanged stream only
/// after both peers accepted each other's current authority.
pub(crate) async fn authenticate_duplex<S>(
    mut stream: S,
    local_peer_id: PeerId,
    remote_peer_id: PeerId,
    tokens: &TokenStore,
    verifier: &DdsTokenVerifier,
    requirements: &SessionRequirements,
) -> Result<(S, AuthenticatedPeer)>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let local_credential = tokens.snapshot().await;
    let local_token = local_credential
        .as_ref()
        .map(SignedP2pCredential::as_str)
        .unwrap_or_default();
    authentication_phase(
        AUTHENTICATION_TIMEOUT,
        write_token_frame(&mut stream, local_token.as_bytes()),
    )
    .await?;
    let remote_token =
        authentication_phase(AUTHENTICATION_TIMEOUT, read_token_frame(&mut stream)).await;

    let local_result = if local_token.is_empty() {
        Err(Error::MissingToken)
    } else {
        verifier.verify(local_token).and_then(|claims| {
            ensure_token_peer(&claims, local_peer_id)?;
            requirements.validate_domain(&claims)?;
            Ok(claims)
        })
    };
    let remote_result = remote_token.and_then(|token| {
        let token = String::from_utf8(token).map_err(|_| Error::InvalidTokenEncoding)?;
        let credential = SignedP2pCredential::new(token)?;
        let claims = verifier.verify(credential.as_str())?;
        ensure_token_peer(&claims, remote_peer_id)?;
        requirements.validate_remote(&claims, remote_peer_id)?;
        Ok(credential)
    });

    let local_accepts = local_result.is_ok() && remote_result.is_ok();
    authentication_phase(AUTHENTICATION_TIMEOUT, async {
        stream
            .write_all(&[if local_accepts {
                AUTH_ACCEPTED
            } else {
                AUTH_REJECTED
            }])
            .await?;
        stream.flush().await?;
        Ok(())
    })
    .await?;

    let mut remote_status = [AUTH_REJECTED];
    authentication_phase(AUTHENTICATION_TIMEOUT, async {
        stream
            .read_exact(&mut remote_status)
            .await
            .map(|_| ())
            .map_err(Error::Io)
    })
    .await?;
    local_result?;
    let remote_credential = remote_result?;
    if remote_status[0] != AUTH_ACCEPTED {
        return Err(Error::RemoteRejected);
    }

    // Token verification happened before the mutual status exchange. Each
    // bounded I/O phase may consume time, so literal `exp` must win once more
    // at the exact boundary where an application stream would be exposed.
    validate_current_local_authority(
        tokens,
        verifier,
        local_peer_id,
        requirements,
        unix_time_now(),
    )
    .await?;
    let remote = finalize_authenticated_peer(
        &remote_credential,
        verifier,
        requirements,
        remote_peer_id,
        unix_time_now(),
    )?;
    Ok((stream, remote))
}

async fn validate_current_local_authority(
    tokens: &TokenStore,
    verifier: &DdsTokenVerifier,
    local_peer_id: PeerId,
    requirements: &SessionRequirements,
    now: u64,
) -> Result<()> {
    let credential = tokens.snapshot().await.ok_or(Error::MissingToken)?;
    let claims = verifier.verify(credential.as_str())?;
    ensure_token_peer(&claims, local_peer_id)?;
    requirements.validate_domain(&claims)?;
    ensure_literal_expiry(&claims, now)
}

fn finalize_authenticated_peer(
    remote_credential: &SignedP2pCredential,
    verifier: &DdsTokenVerifier,
    requirements: &SessionRequirements,
    remote_peer_id: PeerId,
    now: u64,
) -> Result<AuthenticatedPeer> {
    let remote_claims = verifier.verify(remote_credential.as_str())?;
    ensure_token_peer(&remote_claims, remote_peer_id)?;
    requirements.validate_remote(&remote_claims, remote_peer_id)?;
    ensure_literal_expiry(&remote_claims, now)?;
    authenticated_peer_from_claims(remote_claims, remote_peer_id)
}

fn authenticated_peer_from_claims(
    remote_claims: P2PAccessClaims,
    remote_peer_id: PeerId,
) -> Result<AuthenticatedPeer> {
    let subject = Uuid::parse_str(&remote_claims.sub)
        .map_err(|_| Error::InvalidToken("subject must be a canonical UUID".into()))?;
    let domain_ids = remote_claims
        .domain_ids
        .iter()
        .map(|domain_id| {
            Uuid::parse_str(domain_id)
                .map_err(|_| Error::InvalidToken("domain_ids must contain canonical UUIDs".into()))
        })
        .collect::<Result<Vec<_>>>()?;
    let expiration = i64::try_from(remote_claims.exp)
        .ok()
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .ok_or_else(|| Error::InvalidToken("expiration is outside the supported range".into()))?;
    Ok(AuthenticatedPeer {
        peer_id: remote_peer_id,
        subject,
        peer_type: remote_claims.peer_type,
        domain_ids,
        scopes: remote_claims.scopes,
        application: remote_claims.application,
        verified_until: expiration,
    })
}

async fn authentication_phase<T>(
    timeout: Duration,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    let delay = Delay::new(timeout);
    pin_mut!(future, delay);
    match select(future, delay).await {
        Either::Left((result, _)) => result,
        Either::Right(((), _)) => Err(Error::AuthenticationTimeout),
    }
}

async fn write_token_frame<S>(stream: &mut S, token: &[u8]) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    if token.len() > P2P_TOKEN_MAX_BYTES {
        return Err(Error::TokenFrameTooLarge(P2P_TOKEN_MAX_BYTES));
    }
    stream
        .write_all(&(token.len() as u32).to_be_bytes())
        .await?;
    stream.write_all(token).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_token_frame<S>(stream: &mut S) -> Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length).await?;
    let length = u32::from_be_bytes(length) as usize;
    if length > P2P_TOKEN_MAX_BYTES {
        return Err(Error::TokenFrameTooLarge(P2P_TOKEN_MAX_BYTES));
    }
    let mut token = vec![0; length];
    stream.read_exact(&mut token).await?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use std::future::pending;

    use futures::io::Cursor;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    use crate::{Identity, P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE};

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
    async fn authentication_wire_keeps_bounded_big_endian_token_frame() {
        let mut writer = Cursor::new(Vec::new());
        write_token_frame(&mut writer, b"jwt").await.unwrap();
        assert_eq!(writer.into_inner(), b"\0\0\0\x03jwt");

        let mut oversized = Cursor::new(
            (u32::try_from(P2P_TOKEN_MAX_BYTES).unwrap() + 1)
                .to_be_bytes()
                .to_vec(),
        );
        assert!(matches!(
            read_token_frame(&mut oversized).await,
            Err(Error::TokenFrameTooLarge(P2P_TOKEN_MAX_BYTES))
        ));
    }

    #[tokio::test]
    async fn each_authentication_phase_has_its_own_timeout() {
        assert_eq!(
            authentication_phase(Duration::from_secs(1), async { Ok::<_, Error>(7_u8) })
                .await
                .unwrap(),
            7
        );
        assert!(matches!(
            authentication_phase(Duration::from_millis(1), pending::<Result<()>>()).await,
            Err(Error::AuthenticationTimeout)
        ));
    }

    #[tokio::test]
    async fn final_local_authority_boundary_rejects_a_cleared_store() {
        let identity = Identity::generate();
        let tokens = TokenStore::default();
        let verifier = DdsTokenVerifier::from_es256_pem(PUBLIC_KEY).unwrap();
        let requirements = SessionRequirements::new(Uuid::nil().to_string()).unwrap();
        assert!(matches!(
            validate_current_local_authority(
                &tokens,
                &verifier,
                identity.peer_id(),
                &requirements,
                1,
            )
            .await,
            Err(Error::MissingToken)
        ));
    }

    #[test]
    fn final_mutual_boundary_reverifies_remote_authority_and_literal_expiry() {
        const UNRELATED_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEAxcARQLozLIqu/CFm6ub89EElhHX
O+4eTRPLA8IA+ibNtrfWbavOIYZEtwGneJvRTovHr5OUGFu3n/gXNqGbKw==
-----END PUBLIC KEY-----"#;
        let peer_id = Identity::generate().peer_id();
        let domain_id = Uuid::new_v4();
        let now = unix_time_now();
        let claims = P2PAccessClaims {
            token_type: P2P_TOKEN_TYPE.into(),
            iss: P2P_TOKEN_ISSUER.into(),
            aud: vec![P2P_TOKEN_AUDIENCE.into()],
            sub: Uuid::nil().to_string(),
            organization_id: None,
            peer_type: None,
            peer_id: peer_id.to_string(),
            domain_ids: vec![domain_id.to_string()],
            scopes: Vec::new(),
            application: None,
            iat: now,
            nbf: None,
            exp: now + P2P_TOKEN_TTL.as_secs(),
        };
        let signed = SignedP2pCredential::new(
            encode(
                &Header::new(Algorithm::ES256),
                &claims,
                &EncodingKey::from_ec_pem(PRIVATE_KEY).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let requirements = SessionRequirements::new(domain_id.to_string()).unwrap();
        let initial_verifier = DdsTokenVerifier::from_es256_pem(PUBLIC_KEY).unwrap();
        assert_eq!(
            finalize_authenticated_peer(&signed, &initial_verifier, &requirements, peer_id, now)
                .unwrap()
                .verified_until
                .timestamp(),
            claims.exp as i64
        );

        let changed_verifier = DdsTokenVerifier::from_es256_pem(UNRELATED_PUBLIC_KEY).unwrap();
        assert!(matches!(
            finalize_authenticated_peer(&signed, &changed_verifier, &requirements, peer_id, now),
            Err(Error::TokenVerification(_))
        ));
        assert!(matches!(
            finalize_authenticated_peer(
                &signed,
                &initial_verifier,
                &requirements,
                peer_id,
                claims.exp,
            ),
            Err(Error::InvalidToken(message)) if message.contains("literally expired")
        ));
    }
}
