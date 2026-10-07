use crate::{Result, TaskError, compute::endpoint};
use async_trait::async_trait;
use auki_auth::{
    DomainAccess, DomainAccessProvider, Error as AuthError, SecretString,
    machine::{
        AccessBundle, SiweError,
        p2p::DdsP2pError,
        robot::RobotAuthenticator,
        token_manager::{
            AccessAuthenticator, SystemClock, TokenManager, TokenManagerConfig, TokenProvider,
            TokenProviderResult,
        },
    },
};
use auki_domain_client::{AukiDomainData, DomainDataClient};
use auki_p2p::PeerIdentityProof;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::Deserialize;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use url::Url;
use uuid::Uuid;

/// Credentials and expected token audience for an already-provisioned robot.
#[derive(Debug)]
pub struct RobotConfig {
    pub dds_url: Url,
    pub dms_url: Url,
    pub client_id: String,
    pub version: String,
    pub audience: String,
    pub capabilities: Vec<String>,
    pub request_timeout: Duration,
    pub registration_interval: Duration,
    pub peer_identity: Option<PeerIdentityProof>,
    registration: SecretString,
}
impl RobotConfig {
    /// Use the configured audience, or the preset for an official Auki DDS URL.
    /// Custom DDS endpoints require an explicit audience. No network requests
    /// are made, and token audience validation remains exact.
    pub fn new(
        dds_url: &str,
        dms_url: &str,
        registration: SecretString,
        version: &str,
        client_id: &str,
        audience: Option<&str>,
        capabilities: Vec<String>,
    ) -> Result<Self> {
        let dds_url = endpoint(dds_url)?;
        let audience = match audience {
            Some(value) => value,
            None => match dds_url.as_str() {
                "https://dds.dev.aukiverse.com/" => "https://dds.dev.aukiverse.com/robots",
                "https://dds.staging.aukiverse.com/" => "https://dds.staging.aukiverse.com/robots",
                "https://dds.auki.network/" => "https://dds.auki.network/robots",
                _ => {
                    return Err(TaskError::Configuration(
                        "provide an explicit robot audience for custom DDS endpoints",
                    ));
                }
            },
        };
        if registration.expose_secret().trim().is_empty()
            || version.is_empty()
            || version.len() > 128
            || client_id.is_empty()
            || client_id.len() > 128
            || !client_id.bytes().all(|b| b.is_ascii_graphic())
            || audience.is_empty()
            || audience.len() > 512
            || audience.chars().any(char::is_whitespace)
        {
            return Err(TaskError::Configuration(
                "provide robot registration, version, client ID and exclusive audience",
            ));
        }
        crate::runtime::validate_capabilities(&capabilities)?;
        Ok(Self {
            dds_url,
            dms_url: endpoint(dms_url)?,
            registration,
            version: version.into(),
            client_id: client_id.into(),
            audience: audience.into(),
            capabilities,
            request_timeout: Duration::from_secs(30),
            registration_interval: Duration::from_secs(120),
            peer_identity: None,
        })
    }
}

#[cfg(test)]
#[path = "robot_tests.rs"]
mod tests;

#[derive(Clone, Deserialize)]
pub(crate) struct RobotClaims {
    pub assigned_domain_id: Option<Uuid>,
    node_id: Uuid,
    organization_id: Uuid,
    sub: Uuid,
    node_type: String,
    node_mode: String,
    iss: String,
    aud: Vec<String>,
    iat: i64,
    exp: i64,
    peer_id: Option<String>,
}

pub(crate) fn decode_claims<T: serde::de::DeserializeOwned>(token: &str) -> Result<T> {
    if token.len() > 64 * 1024 {
        return Err(TaskError::Authority("oversized machine token"));
    }
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|s| s.is_empty()) {
        return Err(TaskError::Authority("malformed machine token"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| TaskError::Authority("malformed machine claims"))?;
    serde_json::from_slice(&bytes).map_err(|_| TaskError::Authority("invalid machine claims"))
}

struct Authenticator {
    config: Arc<RobotConfig>,
    base: RobotAuthenticator,
    principal: parking_lot::Mutex<Option<RobotClaims>>,
    /// Last token that passed profile checks. A blip during refresh keeps this
    /// one until it actually expires, instead of cancelling the credential.
    last: parking_lot::Mutex<Option<AccessBundle>>,
    /// Set only when this credential cancels itself. The claim loop copies it
    /// into `node.error` so a presence rejection and a token rejection stay distinct.
    stop: parking_lot::Mutex<Option<TaskError>>,
    closed: CancellationToken,
    failed: Arc<AtomicBool>,
}

/// A transport or upstream blip. A rejected credential (4xx other than 408/429)
/// and a token whose robot profile is wrong are permanent.
fn transient_auth_error(error: &SiweError) -> bool {
    match error {
        SiweError::Request(err) => err.status().is_none_or(transient_status),
        SiweError::UpstreamStatus(status) => transient_status(*status),
        SiweError::InvalidExpiration(_) => true,
        SiweError::MissingField(field) => !matches!(
            *field,
            "valid robot authority" | "valid bound robot authority"
        ),
        SiweError::PeerBinding(_) => true,
        SiweError::InvalidHex(_)
        | SiweError::InvalidPrivateKeyLength(_)
        | SiweError::InvalidSigningKey(_)
        | SiweError::Signing(_) => false,
    }
}

fn transient_status(status: StatusCode) -> bool {
    status.is_server_error() || matches!(status.as_u16(), 408 | 429)
}

/// A dropped connection or a retryable HTTP status. A 4xx rejection stops the peer.
fn transient_p2p(error: &DdsP2pError) -> bool {
    match error {
        DdsP2pError::Request(err) => err.status().is_none_or(transient_status),
        DdsP2pError::UpstreamStatus(status) => transient_status(*status),
        _ => false,
    }
}

fn peer_exchange_error(error: DdsP2pError) -> TaskError {
    if transient_p2p(&error) {
        warn!(
            error = %error,
            "robot peer token exchange failed; retrying while the grant is valid"
        );
        return TaskError::Authority("robot P2P exchange unavailable");
    }
    if let DdsP2pError::UpstreamStatus(status) = &error {
        warn!(
            error = %error,
            "robot peer token exchange was rejected; stopping the node"
        );
        return TaskError::PeerExchangeRejected {
            status: status.as_u16(),
        };
    }
    warn!(
        error = %error,
        "robot peer token exchange was rejected; stopping the node"
    );
    TaskError::Authority("robot P2P exchange failed")
}
impl Authenticator {
    // Routing and profile checks on an authenticated DDS response. DDS/DMS still
    // verify signatures; decoded claims are never a substitute for that check.
    fn validate(&self, bundle: &AccessBundle, bound: bool) -> Result<()> {
        let c: RobotClaims = decode_claims(bundle.token())?;
        if c.iss != "dds"
            || c.aud != [self.config.audience.clone()]
            || c.node_type != "robot"
            || c.node_mode != "dedicated"
            || c.sub != c.node_id
            || c.node_id.is_nil()
            || c.organization_id.is_nil()
            || c.assigned_domain_id.is_some_and(|id| id.is_nil())
            || c.exp <= Utc::now().timestamp()
            || c.iat > Utc::now().timestamp()
            || c.exp <= c.iat
            || bundle.expires_at().timestamp() != c.exp
            || bound
                && c.peer_id
                    != self
                        .config
                        .peer_identity
                        .as_ref()
                        .map(|p| p.peer_id().to_string())
        {
            return Err(TaskError::Authority("invalid robot token profile"));
        }
        let mut previous = self.principal.lock();
        if previous.as_ref().is_some_and(|p| {
            p.node_id != c.node_id
                || p.organization_id != c.organization_id
                || p.assigned_domain_id != c.assigned_domain_id
        }) {
            return Err(TaskError::Authority(
                "robot identity or assignment changed; restart after operator reconciliation",
            ));
        }
        *previous = Some(c);
        Ok(())
    }

    fn reject(&self, error: TaskError) {
        *self.stop.lock() = Some(error);
        self.failed.store(true, Ordering::Release);
        self.closed.cancel();
    }
}
#[async_trait]
impl AccessAuthenticator for Authenticator {
    async fn login(&self) -> std::result::Result<AccessBundle, SiweError> {
        let result = async {
            let bundle = self.base.login().await?;
            self.validate(&bundle, false)
                .map_err(|_| SiweError::MissingField("valid robot authority"))?;
            let bundle = crate::machine::bind_peer(
                &self.config.dds_url,
                self.config.request_timeout,
                self.config.peer_identity.as_ref(),
                bundle,
            )
            .await?;
            self.validate(&bundle, true)
                .map_err(|_| SiweError::MissingField("valid bound robot authority"))?;
            Ok(bundle)
        }
        .await;
        match &result {
            Ok(bundle) => {
                *self.last.lock() = Some(bundle.clone());
            }
            Err(error) if transient_auth_error(error) => {
                if let Some(bundle) = self.last.lock().clone()
                    && bundle.expires_at() > Utc::now()
                {
                    warn!(
                        error = %error,
                        "robot token refresh failed; keeping the current token until it expires"
                    );
                    return Ok(bundle);
                }
                warn!(error = %error, "robot token refresh failed; will retry");
            }
            Err(error) => {
                warn!(error = %error, "robot token refresh was rejected; stopping the node");
                self.reject(TaskError::authentication(
                    "token",
                    error.status_code().map(|status| status.as_u16()),
                ));
            }
        }
        result
    }
}
type Manager = TokenManager<Authenticator, SystemClock>;
struct Owner {
    config: Arc<RobotConfig>,
    auth: Arc<Authenticator>,
    manager: Arc<Manager>,
    closed: CancellationToken,
    failed: Arc<AtomicBool>,
    attached: AtomicBool,
    startup: Mutex<bool>,
    registrar: Mutex<Option<JoinHandle<()>>>,
    access: Mutex<Option<Arc<DomainAccess>>>,
    http: reqwest::Client,
    authentication: Arc<tokio::sync::RwLock<()>>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.closed.cancel();
        if let Some(task) = self.registrar.get_mut().take() {
            task.abort();
        }
    }
}

/// Wallet-free robot registration, shared renewal and assigned-Domain read access.
#[derive(Clone)]
pub struct AukiRobotCredential(Arc<Owner>);
impl AukiRobotCredential {
    pub fn new(config: RobotConfig) -> Result<Self> {
        endpoint(config.dds_url.as_str())?;
        endpoint(config.dms_url.as_str())?;
        crate::runtime::validate_capabilities(&config.capabilities)?;
        if config.request_timeout.is_zero()
            || config.request_timeout > Duration::from_secs(300)
            || config.registration_interval < Duration::from_millis(100)
            || config.registration_interval > Duration::from_secs(3600)
        {
            return Err(TaskError::Configuration("invalid robot timing"));
        }
        let base = RobotAuthenticator::new(
            config.dds_url.clone(),
            config.registration.expose_secret().into(),
            config.version.clone(),
            config.capabilities.clone(),
            config.request_timeout,
        )
        .map_err(|_| TaskError::Configuration("invalid robot configuration"))?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(config.request_timeout)
            .build()
            .map_err(|_| TaskError::Configuration("robot HTTP client"))?;
        let config = Arc::new(config);
        let closed = CancellationToken::new();
        let failed = Arc::new(AtomicBool::new(false));
        let auth = Arc::new(Authenticator {
            config: config.clone(),
            base,
            principal: parking_lot::Mutex::new(None),
            last: parking_lot::Mutex::new(None),
            stop: parking_lot::Mutex::new(None),
            closed: closed.clone(),
            failed: failed.clone(),
        });
        let manager = Arc::new(TokenManager::new(
            auth.clone(),
            Arc::new(SystemClock),
            TokenManagerConfig {
                max_retries: 0,
                ..Default::default()
            },
        ));
        Ok(Self(Arc::new(Owner {
            config,
            auth,
            manager,
            closed,
            failed,
            attached: AtomicBool::new(false),
            startup: Mutex::new(false),
            registrar: Mutex::new(None),
            access: Mutex::new(None),
            http,
            authentication: Arc::new(tokio::sync::RwLock::new(())),
        })))
    }
    pub fn config(&self) -> &RobotConfig {
        &self.0.config
    }
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.0.closed.clone()
    }
    pub async fn wait_closed(&self) {
        self.0.closed.cancelled().await;
    }
    pub(crate) fn failed(&self) -> bool {
        self.0.failed.load(Ordering::Acquire)
    }

    /// The rejection recorded when this credential cancelled itself.
    /// A bearer failure before that record exists stays the generic authentication error.
    pub(crate) fn authentication_error(&self) -> TaskError {
        self.0
            .auth
            .stop
            .lock()
            .unwrap_or_else(|| TaskError::authentication("", None))
    }
    pub(crate) fn attach_runtime(&self, caps: &[String]) -> Result<()> {
        let mut configured = self.config().capabilities.clone();
        configured.sort();
        if configured != caps {
            return Err(TaskError::Configuration(
                "robot handlers must match registered capabilities",
            ));
        }
        if self.0.attached.swap(true, Ordering::AcqRel) {
            return Err(TaskError::Configuration(
                "robot credential already has a task runtime",
            ));
        }
        Ok(())
    }
    pub async fn start(&self, cancellation: &CancellationToken) -> Result<()> {
        let operation = async {
            let mut started = self.0.startup.lock().await;
            if *started {
                return Ok(());
            }
            self.bearer()
                .await
                .map_err(|_| self.authentication_error())?;
            // Renew the access token before it expires, including while no task
            // is running. A failed refresh retries; it does not cancel this
            // credential unless DDS rejects the robot.
            self.0.manager.start_bg().await;
            let auth = self.0.auth.clone();
            let closed = self.0.closed.clone();
            *self.0.registrar.lock().await = Some(tokio::spawn(async move {
                loop {
                    tokio::select! { biased; _ = closed.cancelled() => return, _ = tokio::time::sleep(auth.config.registration_interval) => {} }
                    let result = tokio::select! { biased; _ = closed.cancelled() => return,
                    result = auth.base.register_presence() => result };
                    match result {
                        Ok(bundle) => {
                            if let Err(error) = auth.validate(&bundle, false) {
                                warn!(
                                    error = %error,
                                    "robot presence token was rejected; stopping the node"
                                );
                                auth.reject(error);
                                return;
                            }
                        }
                        Err(error) if transient_auth_error(&error) => {
                            warn!(
                                error = %error,
                                "robot presence refresh failed; retrying on the next interval"
                            );
                        }
                        Err(error) => {
                            warn!(
                                error = %error,
                                "robot presence refresh was rejected; stopping the node"
                            );
                            auth.reject(TaskError::authentication(
                                "presence",
                                error.status_code().map(|status| status.as_u16()),
                            ));
                            return;
                        }
                    }
                }
            }));
            *started = true;
            Ok(())
        };
        tokio::select! { biased;
        _ = self.wait_closed() => Err(if self.failed() { self.authentication_error() } else { TaskError::Closed }),
        _ = cancellation.cancelled() => Err(TaskError::Cancelled), result = operation => result }
    }
    pub async fn assigned_domain_id(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<Uuid>> {
        self.start(cancellation).await?;
        self.bearer()
            .await
            .map_err(|_| self.authentication_error())?;
        Ok(self
            .0
            .auth
            .principal
            .lock()
            .as_ref()
            .and_then(|c| c.assigned_domain_id))
    }
    /// Select an idle read client. DDS verifies the robot's persisted assignment.
    pub fn data(&self, domain: Uuid) -> Result<DomainDataClient> {
        Ok(AukiDomainData::new(self.clone())
            .map_err(|_| TaskError::Data)?
            .in_domain(domain))
    }
    pub(crate) async fn peer_grant(&self, domain: Uuid) -> Result<crate::TaskPeerGrant> {
        let dds = auki_auth::machine::p2p::DdsP2pClient::new(
            self.config().dds_url.clone(),
            self.config().request_timeout,
        )
        .map_err(|_| TaskError::Authority("robot P2P client"))?;
        for attempt in 0..2 {
            let bearer = self
                .bearer()
                .await
                .map_err(|_| self.authentication_error())?;
            match dds.robot_p2p_token(&bearer).await {
                Ok(grant) => {
                    if grant.domain_id != domain {
                        return Err(TaskError::Authority("robot P2P Domain mismatch"));
                    }
                    return Ok(crate::TaskPeerGrant {
                        domain_id: domain,
                        token: Arc::new(SecretString::new(grant.token)),
                        expires_at: grant.expires_at,
                        peer_type: "robot",
                    });
                }
                Err(DdsP2pError::UpstreamStatus(status))
                    if status.as_u16() == 401 && attempt == 0 =>
                {
                    self.on_unauthorized().await
                }
                Err(error) => return Err(peer_exchange_error(error)),
            }
        }
        Err(TaskError::PeerExchangeRejected { status: 401 })
    }
    pub async fn close(&self) {
        self.0.closed.cancel();
        self.0.manager.stop().await;
        let _authentication = self.0.authentication.write().await;
        let _startup = self.0.startup.lock().await;
        let mut registrar = self.0.registrar.lock().await;
        if let Some(task) = registrar.as_mut() {
            let _ = task.await;
        }
        registrar.take();
        self.0.access.lock().await.take();
    }
}
#[async_trait]
impl TokenProvider for AukiRobotCredential {
    async fn bearer(&self) -> TokenProviderResult<String> {
        crate::machine::owned_bearer(
            self.0.manager.clone(),
            self.0.closed.clone(),
            self.0.authentication.clone(),
        )
        .await
    }
    async fn on_unauthorized(&self) {
        self.0.manager.on_unauthorized().await;
    }
}

#[derive(Deserialize)]
struct DomainGrant {
    domain_id: Uuid,
    domain_server_url: String,
    access_token: String,
    access_expires_at: DateTime<Utc>,
}
#[async_trait]
impl DomainAccessProvider for AukiRobotCredential {
    fn client_id(&self) -> &str {
        &self.config().client_id
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn wait_closed(&self) {
        self.wait_closed().await;
    }
    async fn domain_access(
        &self,
        domain_id: Uuid,
        rejected: Option<&DomainAccess>,
        cancellation: &CancellationToken,
    ) -> auki_auth::Result<Arc<DomainAccess>> {
        const ENDPOINT: &str = "DDS robot Domain read access";
        let operation = async {
            self.start(cancellation)
                .await
                .map_err(|_| AuthError::AuthenticationRequired)?;
            let mut cache = self.0.access.lock().await;
            let token = self
                .bearer()
                .await
                .map_err(|_| AuthError::AuthenticationRequired)?;
            if self
                .0
                .auth
                .principal
                .lock()
                .as_ref()
                .and_then(|c| c.assigned_domain_id)
                != Some(domain_id)
            {
                return Err(AuthError::DomainNotAccessible);
            }
            if let Some(grant) = cache.as_ref()
                && grant.domain_id() == domain_id
                && grant.expires_at() > Utc::now() + chrono::Duration::seconds(30)
                && rejected.is_none_or(|r| r.bearer() != grant.bearer())
            {
                return Ok(grant.clone());
            }
            let url = format!(
                "{}/internal/v1/auth/robot/domain-token",
                self.config().dds_url.as_str().trim_end_matches('/')
            );
            let mut token = token;
            for attempt in 0..2 {
                let mut response = self
                    .0
                    .http
                    .post(&url)
                    .bearer_auth(&token)
                    .json(&serde_json::json!({"domain_id": domain_id}))
                    .send()
                    .await
                    .map_err(|_| AuthError::Transport { endpoint: ENDPOINT })?;
                let status = response.status().as_u16();
                if status == 401 && attempt == 0 {
                    self.on_unauthorized().await;
                    token = self
                        .bearer()
                        .await
                        .map_err(|_| AuthError::AuthenticationRequired)?;
                    continue;
                }
                if !(200..300).contains(&status) {
                    return Err(AuthError::HttpStatus {
                        endpoint: ENDPOINT,
                        status,
                    });
                }
                let mut body = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| AuthError::Transport { endpoint: ENDPOINT })?
                {
                    if body.len() + chunk.len() > 128 * 1024 {
                        return Err(AuthError::ResponseTooLarge {
                            endpoint: ENDPOINT,
                            maximum: 128 * 1024,
                        });
                    }
                    body.extend_from_slice(&chunk);
                }
                let invalid = || AuthError::InvalidResponse {
                    endpoint: ENDPOINT,
                    reason: "invalid robot Domain read grant",
                };
                let response: DomainGrant = serde_json::from_slice(&body).map_err(|_| invalid())?;
                if response.domain_id != domain_id {
                    return Err(invalid());
                }
                let claims: serde_json::Value =
                    decode_claims(&response.access_token).map_err(|_| invalid())?;
                if claims["scopes"] != serde_json::json!(["domain:r"]) || claims["type"] != "robot"
                {
                    return Err(invalid());
                }
                let grant = Arc::new(DomainAccess::from_issued_grant(
                    domain_id,
                    &response.domain_server_url,
                    SecretString::new(response.access_token),
                    response.access_expires_at,
                )?);
                *cache = Some(grant.clone());
                return Ok(grant);
            }
            unreachable!("bounded retry returns")
        };
        tokio::select! { biased; _ = self.wait_closed() => Err(AuthError::SessionClosed),
        _ = cancellation.cancelled() => Err(AuthError::Cancelled { endpoint: ENDPOINT }), result = operation => result }
    }
}
