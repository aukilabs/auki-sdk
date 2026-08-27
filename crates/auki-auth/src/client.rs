use std::{collections::HashSet, fmt, net::IpAddr, sync::Arc, time::Duration};

use async_trait::async_trait;
use auki_p2p::{
    DDS_PREVIOUS_KEY_MIN_OVERLAP, DDS_VERIFICATION_KEY_MAX_BYTES, DdsTokenVerifier,
    DdsVerificationKeys, P2P_TOKEN_MAX_BYTES, P2PAccessClaims, PeerId, PeerIdentityProof,
    SignedP2pCredential,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use futures::StreamExt;
use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
use reqwest::{
    Client as HttpClient, RequestBuilder, Url,
    header::{ACCEPT, CONTENT_TYPE},
    redirect::Policy,
};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, MutexGuard};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    AppCredentials, AuthorityRenewal, AuthorityRenewalProvider, Credentials, DomainChoice,
    DomainDescriptor, DomainSelection, Error, PeerAuthorityProvider, PreparedPeer, PrincipalKind,
    RenewedAuthority, Result, SecretString, UserPassword,
    wire::{
        AccessibleDomain, AccessibleDomainsResponse, ApiTokenResponse, LoginRequest,
        PeerChallengeRequest, PeerChallengeResponse, PeerVerifyRequest, PeerVerifyResponse,
        ServiceTokenResponse, VerificationKeyStatus, VerificationKeysResponse,
    },
};

const API_LOGIN: &str = "API /user/login";
const API_REFRESH: &str = "API /user/refresh";
const API_SERVICE_TOKEN: &str = "API /service/domains-access-token";
const DDS_ACCESSIBLE_DOMAINS: &str = "DDS /api/v1/accessible-domains";
const DDS_P2P_CHALLENGE: &str = "DDS selected-Domain P2P challenge";
const DDS_P2P_VERIFY: &str = "DDS selected-Domain P2P verify";
const DDS_VERIFICATION_KEYS: &str = "DDS /service/p2p-verification-keys";

const MAX_EMAIL_BYTES: usize = 320;
const MAX_PASSWORD_BYTES: usize = 1024;
const MAX_APP_KEY_BYTES: usize = 256;
const MAX_APP_SECRET_BYTES: usize = 1024;
const ACCESSIBLE_DOMAINS_PAGE_SIZE: usize = 100;
const MAX_ACCESSIBLE_DOMAIN_RESULTS: usize = 1_024;
const MAX_DOMAIN_NAME_BYTES: usize = 256;
const MAX_DOMAIN_DESCRIPTION_BYTES: usize = 4 * 1024;
const MAX_CHALLENGE_ID_BYTES: usize = 256;
const CHALLENGE_BYTES: usize = 32;
const ED25519_SIGNATURE_BYTES: usize = 64;
const CHALLENGE_CLOCK_SKEW: ChronoDuration = ChronoDuration::seconds(60);
const CHALLENGE_TTL: ChronoDuration = ChronoDuration::seconds(60);
const MAX_KEY_ID_BYTES: usize = 64;
const MAX_CONFIGURED_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// API and DDS origins. Production origins must use HTTPS; plain HTTP is
/// accepted only for an actual loopback host.
#[derive(Clone, Debug)]
pub struct AuthEnvironment {
    api_base: Url,
    dds_base: Url,
}

impl AuthEnvironment {
    /// Zero-guess Auki development environment.
    pub fn dev() -> Self {
        Self {
            api_base: Url::parse("https://api.dev.aukiverse.com/")
                .expect("static development API URL is valid"),
            dds_base: Url::parse("https://dds.dev.aukiverse.com/")
                .expect("static development DDS URL is valid"),
        }
    }

    pub fn new(api_base: impl AsRef<str>, dds_base: impl AsRef<str>) -> Result<Self> {
        Ok(Self {
            api_base: parse_base_url(api_base.as_ref())?,
            dds_base: parse_base_url(dds_base.as_ref())?,
        })
    }

    pub fn api_base_url(&self) -> &str {
        self.api_base.as_str()
    }

    pub fn dds_base_url(&self) -> &str {
        self.dds_base.as_str()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AuthLimits {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_response_bytes: usize,
}

impl Default for AuthLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(15),
            max_response_bytes: 512 * 1024,
        }
    }
}

#[derive(Clone)]
pub struct AuthClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    environment: AuthEnvironment,
    http: HttpClient,
    limits: AuthLimits,
}

impl fmt::Debug for AuthClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthClient")
            .field("environment", &self.inner.environment)
            .field("limits", &self.inner.limits)
            .finish()
    }
}

#[derive(Clone)]
pub struct AuthSession {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    client: AuthClient,
    principal_kind: PrincipalKind,
    state: Mutex<SessionState>,
}

enum PrincipalState {
    User { refresh_token: SecretString },
    App(AppCredentials),
}

struct SessionState {
    principal: PrincipalState,
    dds_service_bearer: SecretString,
}

impl SessionState {
    fn gateway_mac(&self) -> Option<&str> {
        match &self.principal {
            PrincipalState::User { .. } => None,
            PrincipalState::App(credentials) => credentials.gateway_mac.as_deref(),
        }
    }
}

impl fmt::Debug for AuthSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthSession")
            .field("principal_kind", &self.inner.principal_kind)
            .field("authority", &"[redacted]")
            .finish()
    }
}

impl AuthClient {
    pub fn new(environment: AuthEnvironment) -> Result<Self> {
        Self::with_limits(environment, AuthLimits::default())
    }

    pub fn with_limits(environment: AuthEnvironment, limits: AuthLimits) -> Result<Self> {
        validate_limits(limits)?;
        let http = HttpClient::builder()
            .no_proxy()
            .connect_timeout(limits.connect_timeout)
            .timeout(limits.request_timeout)
            .redirect(Policy::none())
            .user_agent(concat!("auki-auth/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| Error::InvalidConfiguration("failed to construct bounded HTTP client"))?;
        Ok(Self {
            inner: Arc::new(ClientInner {
                environment,
                http,
                limits,
            }),
        })
    }

    pub async fn authenticate(&self, credentials: Credentials) -> Result<AuthSession> {
        let cancellation = CancellationToken::new();
        self.authenticate_with_cancellation(credentials, &cancellation)
            .await
    }

    pub async fn authenticate_with_cancellation(
        &self,
        credentials: Credentials,
        cancellation: &CancellationToken,
    ) -> Result<AuthSession> {
        let principal_kind = credentials.principal_kind();
        let state = match credentials {
            Credentials::UserPassword(credentials) => {
                validate_user_credentials(&credentials)?;
                let (access_token, refresh_token) =
                    self.login_user(&credentials, cancellation).await?;
                // The raw password is dropped here and is never retained by the session.
                drop(credentials);
                let dds_service_bearer = self
                    .exchange_user_service_token(&access_token, cancellation)
                    .await?;
                SessionState {
                    principal: PrincipalState::User { refresh_token },
                    dds_service_bearer,
                }
            }
            Credentials::AppCredentials(credentials) => {
                validate_app_credentials(&credentials)?;
                let dds_service_bearer = self
                    .exchange_app_service_token(&credentials, cancellation)
                    .await?;
                SessionState {
                    principal: PrincipalState::App(credentials),
                    dds_service_bearer,
                }
            }
        };
        Ok(AuthSession {
            inner: Arc::new(SessionInner {
                client: self.clone(),
                principal_kind,
                state: Mutex::new(state),
            }),
        })
    }

    async fn login_user(
        &self,
        credentials: &UserPassword,
        cancellation: &CancellationToken,
    ) -> Result<(SecretString, SecretString)> {
        let request = self
            .inner
            .http
            .post(self.api_url("user/login"))
            .header(ACCEPT, "application/json")
            .json(&LoginRequest {
                email: &credentials.email,
                password: credentials.password.expose(),
            });
        let response: ApiTokenResponse = self.send_json(request, API_LOGIN, cancellation).await?;
        Ok((
            validated_token(response.access_token, API_LOGIN)?,
            validated_token(response.refresh_token, API_LOGIN)?,
        ))
    }

    async fn refresh_user(
        &self,
        refresh_token: &SecretString,
        cancellation: &CancellationToken,
    ) -> Result<(SecretString, SecretString)> {
        let request = self
            .inner
            .http
            .post(self.api_url("user/refresh"))
            .header(ACCEPT, "application/json")
            .bearer_auth(refresh_token.expose());
        let response: ApiTokenResponse = self.send_json(request, API_REFRESH, cancellation).await?;
        Ok((
            validated_token(response.access_token, API_REFRESH)?,
            validated_token(response.refresh_token, API_REFRESH)?,
        ))
    }

    async fn exchange_user_service_token(
        &self,
        access_token: &SecretString,
        cancellation: &CancellationToken,
    ) -> Result<SecretString> {
        let request = self
            .inner
            .http
            .post(self.api_url("service/domains-access-token"))
            .header(ACCEPT, "application/json")
            .bearer_auth(access_token.expose());
        let response: ServiceTokenResponse = self
            .send_json(request, API_SERVICE_TOKEN, cancellation)
            .await?;
        validated_token(response.access_token, API_SERVICE_TOKEN)
    }

    async fn exchange_app_service_token(
        &self,
        credentials: &AppCredentials,
        cancellation: &CancellationToken,
    ) -> Result<SecretString> {
        let request = self
            .inner
            .http
            .post(self.api_url("service/domains-access-token"))
            .header(ACCEPT, "application/json")
            .basic_auth(&credentials.access_key, Some(credentials.secret.expose()));
        let response: ServiceTokenResponse = self
            .send_json(request, API_SERVICE_TOKEN, cancellation)
            .await?;
        validated_token(response.access_token, API_SERVICE_TOKEN)
    }

    async fn refresh_service_bearer(
        &self,
        state: &mut SessionState,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        match &state.principal {
            PrincipalState::User { refresh_token, .. } => {
                let (access_token, refresh_token) =
                    self.refresh_user(refresh_token, cancellation).await?;
                // `/user/refresh` rotates the server-side session. Commit its
                // replacement refresh token immediately; that external change
                // cannot be rolled back if the following service exchange fails.
                state.principal = PrincipalState::User { refresh_token };
                let dds_service_bearer = self
                    .exchange_user_service_token(&access_token, cancellation)
                    .await?;
                // Keep the former DDS bearer until its replacement is complete.
                state.dds_service_bearer = dds_service_bearer;
            }
            PrincipalState::App(credentials) => {
                let dds_service_bearer = self
                    .exchange_app_service_token(credentials, cancellation)
                    .await?;
                state.dds_service_bearer = dds_service_bearer;
            }
        }
        Ok(())
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        endpoint: &'static str,
        cancellation: &CancellationToken,
    ) -> Result<T> {
        let operation = async {
            let response = request.send().await.map_err(|error| {
                if error.is_timeout() {
                    Error::RequestTimedOut { endpoint }
                } else {
                    Error::Transport { endpoint }
                }
            })?;
            if response.status().as_u16() != 200 {
                return Err(Error::HttpStatus {
                    endpoint,
                    status: response.status().as_u16(),
                });
            }
            let is_json = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"));
            if !is_json {
                return Err(Error::invalid_response(
                    endpoint,
                    "Content-Type must be application/json",
                ));
            }
            if response
                .content_length()
                .is_some_and(|length| length > self.inner.limits.max_response_bytes as u64)
            {
                return Err(Error::ResponseTooLarge {
                    endpoint,
                    maximum: self.inner.limits.max_response_bytes,
                });
            }

            let mut body = Vec::new();
            let mut chunks = response.bytes_stream();
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk.map_err(|error| {
                    if error.is_timeout() {
                        Error::RequestTimedOut { endpoint }
                    } else {
                        Error::Transport { endpoint }
                    }
                })?;
                let next_len =
                    body.len()
                        .checked_add(chunk.len())
                        .ok_or(Error::ResponseTooLarge {
                            endpoint,
                            maximum: self.inner.limits.max_response_bytes,
                        })?;
                if next_len > self.inner.limits.max_response_bytes {
                    return Err(Error::ResponseTooLarge {
                        endpoint,
                        maximum: self.inner.limits.max_response_bytes,
                    });
                }
                body.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&body).map_err(|_| {
                Error::invalid_response(endpoint, "JSON does not match the V1 contract")
            })
        };

        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(Error::Cancelled { endpoint }),
            result = operation => result,
        }
    }

    fn api_url(&self, relative: &str) -> Url {
        self.inner
            .environment
            .api_base
            .join(relative)
            .expect("validated base URL joins static API path")
    }

    fn dds_url(&self, relative: &str) -> Url {
        self.inner
            .environment
            .dds_base
            .join(relative)
            .expect("validated base URL joins bounded DDS path")
    }
}

impl AuthSession {
    pub fn principal_kind(&self) -> PrincipalKind {
        self.inner.principal_kind
    }

    pub async fn accessible_domains(&self) -> Result<Vec<DomainChoice>> {
        let cancellation = CancellationToken::new();
        self.accessible_domains_with_cancellation(&cancellation)
            .await
    }

    pub async fn accessible_domains_with_cancellation(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<DomainChoice>> {
        let mut state = self
            .lock_state(cancellation, DDS_ACCESSIBLE_DOMAINS)
            .await?;
        match self.fetch_accessible_domains(&state, cancellation).await {
            Ok(domains) => Ok(domains),
            Err(error) if error.is_unauthorized() => {
                self.inner
                    .client
                    .refresh_service_bearer(&mut state, cancellation)
                    .await?;
                self.fetch_accessible_domains(&state, cancellation).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn authorize_peer(
        &self,
        selection: DomainSelection,
        identity: &PeerIdentityProof,
    ) -> Result<PreparedPeer> {
        let cancellation = CancellationToken::new();
        self.authorize_peer_with_cancellation(selection, identity, &cancellation)
            .await
    }

    pub async fn authorize_peer_with_cancellation(
        &self,
        selection: DomainSelection,
        identity: &PeerIdentityProof,
        cancellation: &CancellationToken,
    ) -> Result<PreparedPeer> {
        let material = self
            .authorize_material(selection, identity, cancellation)
            .await?;
        let version = RenewalVersion {
            issued_at: material.issued_at,
            keys: material.verification_keys.clone(),
        };
        let renewal = AuthorityRenewal::new(SessionRenewal {
            session: self.clone(),
            selection,
            identity: identity.clone(),
            version: Mutex::new(version),
        });
        Ok(PreparedPeer {
            domain: material.domain,
            peer_id: material.peer_id,
            initial_credential: material.credential,
            verification_keys: material.verification_keys,
            credential_expires_at: material.expires_at,
            renew_at: material.renew_at,
            renewal,
        })
    }

    async fn authorize_material(
        &self,
        selection: DomainSelection,
        identity: &PeerIdentityProof,
        cancellation: &CancellationToken,
    ) -> Result<AuthorizedMaterial> {
        let mut state = self.lock_state(cancellation, DDS_P2P_CHALLENGE).await?;
        match self
            .authorize_attempt(&state, selection, identity, cancellation)
            .await
        {
            Ok(material) => Ok(material),
            Err(error) if error.is_unauthorized() => {
                // A challenge binds the exact bearer digest. If any step sees
                // 401, rotate the bearer and restart the whole one-time flow.
                self.inner
                    .client
                    .refresh_service_bearer(&mut state, cancellation)
                    .await?;
                self.authorize_attempt(&state, selection, identity, cancellation)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    async fn authorize_attempt(
        &self,
        state: &SessionState,
        selection: DomainSelection,
        identity: &PeerIdentityProof,
        cancellation: &CancellationToken,
    ) -> Result<AuthorizedMaterial> {
        let domains = self.fetch_accessible_domains(state, cancellation).await?;
        let domain = domains
            .into_iter()
            .find(|choice| choice.domain.id == selection.domain_id)
            .map(|choice| choice.domain)
            .ok_or(Error::DomainNotAccessible)?;

        let peer_id = identity.peer_id();
        let peer_id_text = peer_id.to_string();
        let public_key = URL_SAFE_NO_PAD.encode(identity.public_key_protobuf());
        let challenge_path = format!("api/v1/domains/{}/p2p/challenge", selection.domain_id);
        let request_body = PeerChallengeRequest {
            peer_id: &peer_id_text,
            public_key: &public_key,
        };
        let request = self.dds_authorized_request(
            self.inner
                .client
                .inner
                .http
                .post(self.inner.client.dds_url(&challenge_path))
                .header(ACCEPT, "application/json")
                .json(&request_body),
            state,
        );
        let challenge: PeerChallengeResponse = self
            .inner
            .client
            .send_json(request, DDS_P2P_CHALLENGE, cancellation)
            .await
            .map_err(normalize_domain_selection_race)?;
        let challenge_bytes = validate_challenge(&challenge)?;
        let signature = identity.sign_challenge(&challenge_bytes)?;
        if signature.len() != ED25519_SIGNATURE_BYTES {
            return Err(Error::invalid_response(
                DDS_P2P_CHALLENGE,
                "identity returned a non-Ed25519 signature",
            ));
        }
        let signature = URL_SAFE_NO_PAD.encode(signature);

        let verify_path = format!("api/v1/domains/{}/p2p/verify", selection.domain_id);
        let verify_body = PeerVerifyRequest {
            challenge_id: &challenge.challenge_id,
            signature: &signature,
        };
        let request = self.dds_authorized_request(
            self.inner
                .client
                .inner
                .http
                .post(self.inner.client.dds_url(&verify_path))
                .header(ACCEPT, "application/json")
                .json(&verify_body),
            state,
        );
        let verified: PeerVerifyResponse = self
            .inner
            .client
            .send_json(request, DDS_P2P_VERIFY, cancellation)
            .await
            .map_err(normalize_domain_selection_race)?;
        validate_verify_envelope(
            &verified,
            peer_id,
            selection.domain_id,
            self.inner.principal_kind,
        )?;

        let verification_keys = self.fetch_verification_keys(cancellation).await?;
        let credential = SignedP2pCredential::new(verified.p2p_access_token)?;
        let verifier = DdsTokenVerifier::from_keys(verification_keys.clone())?;
        let claims = verifier.verify_credential(&credential)?;
        validate_credential_claims(
            &claims,
            peer_id,
            selection.domain_id,
            self.inner.principal_kind,
            verified.p2p_access_expires_at,
        )?;
        let renew_at = renewal_time(&claims)?;

        Ok(AuthorizedMaterial {
            domain,
            peer_id,
            credential,
            verification_keys,
            expires_at: verified.p2p_access_expires_at,
            renew_at,
            issued_at: claims.iat,
        })
    }

    async fn fetch_accessible_domains(
        &self,
        state: &SessionState,
        cancellation: &CancellationToken,
    ) -> Result<Vec<DomainChoice>> {
        let mut domains = Vec::new();
        let mut domain_ids = HashSet::new();
        let mut expected_total = None;
        let mut offset = 0u32;

        loop {
            let mut url = self.inner.client.dds_url("api/v1/accessible-domains");
            url.query_pairs_mut()
                .append_pair("limit", &ACCESSIBLE_DOMAINS_PAGE_SIZE.to_string())
                .append_pair("offset", &offset.to_string());
            let request = self.dds_authorized_request(
                self.inner
                    .client
                    .inner
                    .http
                    .get(url)
                    .header(ACCEPT, "application/json"),
                state,
            );
            let response: AccessibleDomainsResponse = self
                .inner
                .client
                .send_json(request, DDS_ACCESSIBLE_DOMAINS, cancellation)
                .await?;
            let total = append_accessible_domain_page(
                response,
                offset,
                expected_total,
                &mut domains,
                &mut domain_ids,
            )?;
            expected_total = Some(total);
            if domains.len() == total as usize {
                return Ok(domains);
            }
            offset = offset
                .checked_add(ACCESSIBLE_DOMAINS_PAGE_SIZE as u32)
                .ok_or_else(|| {
                    Error::invalid_response(
                        DDS_ACCESSIBLE_DOMAINS,
                        "accessible Domain offset overflowed",
                    )
                })?;
        }
    }

    async fn fetch_verification_keys(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<DdsVerificationKeys> {
        let request = self
            .inner
            .client
            .inner
            .http
            .get(self.inner.client.dds_url("service/p2p-verification-keys"))
            .header(ACCEPT, "application/json");
        let response: VerificationKeysResponse = self
            .inner
            .client
            .send_json(request, DDS_VERIFICATION_KEYS, cancellation)
            .await?;
        convert_verification_keys(response)
    }

    fn dds_authorized_request(
        &self,
        request: RequestBuilder,
        state: &SessionState,
    ) -> RequestBuilder {
        let request = request.bearer_auth(state.dds_service_bearer.expose());
        match state.gateway_mac() {
            Some(gateway_mac) => request.header("Posemesh-Gateway-MAC", gateway_mac),
            None => request,
        }
    }

    async fn lock_state<'a>(
        &'a self,
        cancellation: &CancellationToken,
        endpoint: &'static str,
    ) -> Result<MutexGuard<'a, SessionState>> {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(Error::Cancelled { endpoint }),
            state = self.inner.state.lock() => Ok(state),
        }
    }
}

#[async_trait]
impl PeerAuthorityProvider for AuthSession {
    async fn accessible_domains(&self) -> Result<Vec<DomainChoice>> {
        AuthSession::accessible_domains(self).await
    }

    async fn authorize_peer(
        &self,
        selection: DomainSelection,
        identity: &PeerIdentityProof,
    ) -> Result<PreparedPeer> {
        AuthSession::authorize_peer(self, selection, identity).await
    }
}

struct AuthorizedMaterial {
    domain: DomainDescriptor,
    peer_id: PeerId,
    credential: SignedP2pCredential,
    verification_keys: DdsVerificationKeys,
    expires_at: DateTime<Utc>,
    renew_at: DateTime<Utc>,
    issued_at: u64,
}

struct RenewalVersion {
    issued_at: u64,
    keys: DdsVerificationKeys,
}

struct SessionRenewal {
    session: AuthSession,
    selection: DomainSelection,
    identity: PeerIdentityProof,
    version: Mutex<RenewalVersion>,
}

#[async_trait]
impl AuthorityRenewalProvider for SessionRenewal {
    async fn renew_authority(&self, cancellation: &CancellationToken) -> Result<RenewedAuthority> {
        let mut current = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(Error::Cancelled { endpoint: DDS_P2P_CHALLENGE }),
            current = self.version.lock() => current,
        };
        let material = self
            .session
            .authorize_material(self.selection, &self.identity, cancellation)
            .await?;
        if material.issued_at <= current.issued_at
            || material.verification_keys.generation() < current.keys.generation()
        {
            return Err(Error::StaleAuthority);
        }
        if material.verification_keys.generation() == current.keys.generation()
            && material.verification_keys != current.keys
        {
            return Err(Error::VerificationKeyGenerationConflict);
        }
        current
            .keys
            .validate_successor(&material.verification_keys)?;
        current.issued_at = material.issued_at;
        current.keys = material.verification_keys.clone();

        Ok(RenewedAuthority {
            domain: material.domain,
            peer_id: material.peer_id,
            credential: material.credential,
            verification_keys: material.verification_keys,
            credential_expires_at: material.expires_at,
            renew_at: material.renew_at,
        })
    }
}

fn parse_base_url(input: &str) -> Result<Url> {
    let mut url = Url::parse(input)
        .map_err(|_| Error::InvalidConfiguration("API and DDS base URLs must be absolute URLs"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidConfiguration(
            "base URLs cannot contain credentials, query parameters, or fragments",
        ));
    }
    let secure = url.scheme() == "https";
    let loopback_http = url.scheme() == "http" && url.host_str().is_some_and(is_loopback_host);
    if !secure && !loopback_http {
        return Err(Error::InvalidConfiguration(
            "base URLs must use HTTPS, except HTTP on loopback",
        ));
    }
    if !matches!(url.path(), "" | "/") {
        return Err(Error::InvalidConfiguration(
            "base URLs must not contain a path prefix",
        ));
    }
    url.set_path("/");
    Ok(url)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn validate_limits(limits: AuthLimits) -> Result<()> {
    if limits.connect_timeout.is_zero()
        || limits.request_timeout.is_zero()
        || limits.connect_timeout > limits.request_timeout
        || limits.connect_timeout > MAX_CONNECT_TIMEOUT
        || limits.request_timeout > MAX_REQUEST_TIMEOUT
    {
        return Err(Error::InvalidConfiguration(
            "timeouts must be positive, connect <= request, connect <= 30s, and request <= 60s",
        ));
    }
    if !(1..=MAX_CONFIGURED_RESPONSE_BYTES).contains(&limits.max_response_bytes) {
        return Err(Error::InvalidConfiguration(
            "max_response_bytes must be between 1 byte and 4 MiB",
        ));
    }
    Ok(())
}

fn validate_user_credentials(credentials: &UserPassword) -> Result<()> {
    if credentials.email.is_empty()
        || credentials.email.len() > MAX_EMAIL_BYTES
        || credentials.email.trim() != credentials.email
        || !credentials.email.contains('@')
    {
        return Err(Error::InvalidInput {
            field: "email",
            reason: "must be a bounded email address without surrounding whitespace",
        });
    }
    validate_secret(&credentials.password, MAX_PASSWORD_BYTES, "password")
}

fn validate_app_credentials(credentials: &AppCredentials) -> Result<()> {
    if credentials.access_key.is_empty()
        || credentials.access_key.len() > MAX_APP_KEY_BYTES
        || credentials.access_key.trim() != credentials.access_key
        || credentials.access_key.contains(':')
    {
        return Err(Error::InvalidInput {
            field: "access_key",
            reason: "must be bounded, contain no colon, and have no surrounding whitespace",
        });
    }
    validate_secret(&credentials.secret, MAX_APP_SECRET_BYTES, "app secret")
}

fn validate_secret(secret: &SecretString, maximum: usize, field: &'static str) -> Result<()> {
    if secret.is_empty() || secret.len() > maximum {
        return Err(Error::InvalidInput {
            field,
            reason: "must be non-empty and within the configured byte bound",
        });
    }
    Ok(())
}

fn validated_token(value: String, endpoint: &'static str) -> Result<SecretString> {
    if value.is_empty() || value.len() > P2P_TOKEN_MAX_BYTES || value.trim() != value {
        return Err(Error::invalid_response(
            endpoint,
            "token must be non-empty, bounded, and contain no surrounding whitespace",
        ));
    }
    Ok(SecretString::new(value))
}

fn append_accessible_domain_page(
    response: AccessibleDomainsResponse,
    expected_offset: u32,
    expected_total: Option<u64>,
    domains: &mut Vec<DomainChoice>,
    domain_ids: &mut HashSet<Uuid>,
) -> Result<u64> {
    if response.limit as usize != ACCESSIBLE_DOMAINS_PAGE_SIZE || response.offset != expected_offset
    {
        return Err(Error::invalid_response(
            DDS_ACCESSIBLE_DOMAINS,
            "pagination echo does not match the request",
        ));
    }
    if response.domains.len() > ACCESSIBLE_DOMAINS_PAGE_SIZE {
        return Err(Error::invalid_response(
            DDS_ACCESSIBLE_DOMAINS,
            "accessible Domain page exceeds the requested bound",
        ));
    }
    if response.total > MAX_ACCESSIBLE_DOMAIN_RESULTS as u64 {
        return Err(Error::AccessibleDomainsTruncated {
            total: response.total,
            returned: domains.len() + response.domains.len(),
        });
    }
    if expected_total.is_some_and(|total| total != response.total) {
        return Err(Error::invalid_response(
            DDS_ACCESSIBLE_DOMAINS,
            "accessible Domain total changed between pages",
        ));
    }

    let remaining = response
        .total
        .checked_sub(u64::from(expected_offset))
        .ok_or_else(|| {
            Error::invalid_response(
                DDS_ACCESSIBLE_DOMAINS,
                "pagination offset exceeds the accessible Domain total",
            )
        })?;
    let expected_page_len = remaining.min(ACCESSIBLE_DOMAINS_PAGE_SIZE as u64) as usize;
    if response.domains.len() != expected_page_len {
        return Err(Error::invalid_response(
            DDS_ACCESSIBLE_DOMAINS,
            "accessible Domain page length is inconsistent with total",
        ));
    }

    for domain in response.domains {
        let descriptor = convert_domain(domain)?;
        if !domain_ids.insert(descriptor.id) {
            return Err(Error::invalid_response(
                DDS_ACCESSIBLE_DOMAINS,
                "Domain IDs must be unique across pages",
            ));
        }
        domains.push(DomainChoice { domain: descriptor });
    }
    Ok(response.total)
}

fn convert_domain(domain: AccessibleDomain) -> Result<DomainDescriptor> {
    let id = canonical_uuid(&domain.id, DDS_ACCESSIBLE_DOMAINS)?;
    let organization_id = domain
        .organization_id
        .as_deref()
        .map(|value| canonical_uuid(value, DDS_ACCESSIBLE_DOMAINS))
        .transpose()?;
    if domain.name.is_empty()
        || domain.name.len() > MAX_DOMAIN_NAME_BYTES
        || domain.name.trim() != domain.name
    {
        return Err(Error::invalid_response(
            DDS_ACCESSIBLE_DOMAINS,
            "Domain name is empty, oversized, or padded with whitespace",
        ));
    }
    if domain.description.len() > MAX_DOMAIN_DESCRIPTION_BYTES {
        return Err(Error::invalid_response(
            DDS_ACCESSIBLE_DOMAINS,
            "Domain description is oversized",
        ));
    }
    Ok(DomainDescriptor {
        id,
        name: Some(domain.name),
        description: Some(domain.description),
        organization_id,
    })
}

fn validate_challenge(response: &PeerChallengeResponse) -> Result<Vec<u8>> {
    validate_challenge_at(response, Utc::now())
}

pub(crate) fn validate_challenge_at(
    response: &PeerChallengeResponse,
    now: DateTime<Utc>,
) -> Result<Vec<u8>> {
    if response.challenge_id.is_empty()
        || response.challenge_id.len() > MAX_CHALLENGE_ID_BYTES
        || response.challenge_id.trim() != response.challenge_id
    {
        return Err(Error::invalid_response(
            DDS_P2P_CHALLENGE,
            "challenge_id is empty, oversized, or padded with whitespace",
        ));
    }
    // A slow response from a server whose wall clock is behind can carry an
    // expiration slightly before the client's current clock while the
    // one-time Redis challenge is still live. DDS remains authoritative at
    // verify time, so tolerate only the same bounded skew used by authority.
    let earliest_expiration = now - CHALLENGE_CLOCK_SKEW;
    let latest_expiration = now + CHALLENGE_TTL + CHALLENGE_CLOCK_SKEW;
    if response.expires_at < earliest_expiration || response.expires_at > latest_expiration {
        return Err(Error::invalid_response(
            DDS_P2P_CHALLENGE,
            "challenge expiration is outside the bounded clock-skew window",
        ));
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(response.challenge.as_bytes())
        .map_err(|_| {
            Error::invalid_response(DDS_P2P_CHALLENGE, "challenge must use unpadded base64url")
        })?;
    if decoded.len() != CHALLENGE_BYTES || URL_SAFE_NO_PAD.encode(&decoded) != response.challenge {
        return Err(Error::invalid_response(
            DDS_P2P_CHALLENGE,
            "challenge must be canonical unpadded base64url for 32 bytes",
        ));
    }
    Ok(decoded)
}

fn validate_verify_envelope(
    response: &PeerVerifyResponse,
    peer_id: PeerId,
    domain_id: Uuid,
    principal_kind: PrincipalKind,
) -> Result<()> {
    if response.peer_id != peer_id.to_string()
        || response.domain_id != domain_id.to_string()
        || response.peer_type != principal_kind.as_str()
    {
        return Err(Error::invalid_response(
            DDS_P2P_VERIFY,
            "verification envelope does not match the requested principal, Domain, and Peer ID",
        ));
    }
    if response.p2p_access_token.is_empty()
        || response.p2p_access_token.len() > P2P_TOKEN_MAX_BYTES
        || response.p2p_access_token.trim() != response.p2p_access_token
    {
        return Err(Error::invalid_response(
            DDS_P2P_VERIFY,
            "P2P access token is empty, oversized, or padded with whitespace",
        ));
    }
    if response.p2p_access_expires_at <= Utc::now()
        || response.p2p_access_expires_at.timestamp_subsec_nanos() != 0
    {
        return Err(Error::invalid_response(
            DDS_P2P_VERIFY,
            "P2P access expiration is not a current whole second",
        ));
    }
    Ok(())
}

fn convert_verification_keys(response: VerificationKeysResponse) -> Result<DdsVerificationKeys> {
    if response.version != 1 || response.generation == 0 {
        return Err(Error::invalid_response(
            DDS_VERIFICATION_KEYS,
            "unsupported key-set version or zero generation",
        ));
    }
    if response.previous_key_overlap_seconds < DDS_PREVIOUS_KEY_MIN_OVERLAP.as_secs() {
        return Err(Error::invalid_response(
            DDS_VERIFICATION_KEYS,
            "previous-key overlap is below the SDK safety window",
        ));
    }
    if !(1..=2).contains(&response.keys.len())
        || response.keys[0].status != VerificationKeyStatus::Current
        || response
            .keys
            .get(1)
            .is_some_and(|key| key.status != VerificationKeyStatus::Previous)
    {
        return Err(Error::invalid_response(
            DDS_VERIFICATION_KEYS,
            "keys must contain current first and at most one previous key",
        ));
    }

    let mut parsed = Vec::with_capacity(response.keys.len());
    let mut ids = HashSet::with_capacity(response.keys.len());
    for key in response.keys {
        if key.id.len() != MAX_KEY_ID_BYTES
            || !key
                .id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !ids.insert(key.id.clone())
            || key.signing_method != "ES256"
            || key.public_key.is_empty()
            || key.public_key.len() > DDS_VERIFICATION_KEY_MAX_BYTES
        {
            return Err(Error::invalid_response(
                DDS_VERIFICATION_KEYS,
                "verification-key metadata or PEM is invalid",
            ));
        }
        if verification_key_id(&key.public_key)? != key.id {
            return Err(Error::invalid_response(
                DDS_VERIFICATION_KEYS,
                "verification-key id does not match its canonical PKIX key fingerprint",
            ));
        }
        parsed.push(key.public_key.into_bytes());
    }
    let current = parsed.remove(0);
    let previous = parsed.pop();
    Ok(DdsVerificationKeys::new(
        response.generation,
        current,
        previous,
    ))
}

pub(crate) fn verification_key_id(public_key_pem: &str) -> Result<String> {
    let public_key = p256::PublicKey::from_public_key_pem(public_key_pem).map_err(|_| {
        Error::invalid_response(
            DDS_VERIFICATION_KEYS,
            "verification key must contain a P-256 public key",
        )
    })?;
    let canonical_der = public_key.to_public_key_der().map_err(|_| {
        Error::invalid_response(
            DDS_VERIFICATION_KEYS,
            "verification key could not be encoded as canonical PKIX DER",
        )
    })?;
    Ok(hex::encode(Sha256::digest(canonical_der.as_bytes())))
}

fn validate_credential_claims(
    claims: &P2PAccessClaims,
    peer_id: PeerId,
    domain_id: Uuid,
    principal_kind: PrincipalKind,
    response_expiration: DateTime<Utc>,
) -> Result<()> {
    let expiration = u64::try_from(response_expiration.timestamp())
        .map_err(|_| Error::invalid_response(DDS_P2P_VERIFY, "P2P access expiration is invalid"))?;
    if claims.peer_id != peer_id.to_string()
        || claims.domain_ids != [domain_id.to_string()]
        || claims.peer_type.as_deref() != Some(principal_kind.as_str())
        || claims.exp != expiration
    {
        return Err(Error::invalid_response(
            DDS_P2P_VERIFY,
            "signed claims do not match the response and selected authority",
        ));
    }
    Ok(())
}

fn renewal_time(claims: &P2PAccessClaims) -> Result<DateTime<Utc>> {
    let lifetime = claims
        .exp
        .checked_sub(claims.iat)
        .ok_or_else(|| Error::invalid_response(DDS_P2P_VERIFY, "invalid P2P token lifetime"))?;
    let renew_at = claims
        .iat
        .checked_add(lifetime.saturating_mul(3) / 4)
        .and_then(|timestamp| i64::try_from(timestamp).ok())
        .and_then(|timestamp| Utc.timestamp_opt(timestamp, 0).single())
        .ok_or_else(|| Error::invalid_response(DDS_P2P_VERIFY, "invalid renewal timestamp"))?;
    Ok(renew_at)
}

fn canonical_uuid(value: &str, endpoint: &'static str) -> Result<Uuid> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| Error::invalid_response(endpoint, "UUID field is invalid"))?;
    if parsed.to_string() != value {
        return Err(Error::invalid_response(
            endpoint,
            "UUID field is not canonical",
        ));
    }
    Ok(parsed)
}

fn normalize_domain_selection_race(error: Error) -> Error {
    match error {
        Error::HttpStatus { status: 404, .. } => Error::DomainNotAccessible,
        error => error,
    }
}
