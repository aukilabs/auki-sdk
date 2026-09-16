use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use auki_auth::{
    SecretString,
    machine::{
        self,
        registration::{self, RegistrationAttemptKind},
        siwe,
        token_manager::{
            AccessAuthenticator, SystemClock, TokenManager, TokenManagerConfig, TokenProvider,
            TokenProviderResult,
        },
    },
};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{Result, TaskError};

/// Explicit credentials for an already-provisioned compute node.
/// Wallet and registration credentials are separate from the P2P identity.
#[derive(Debug)]
pub struct ComputeConfig {
    pub dds_url: Url,
    pub dms_url: Url,
    pub client_id: String,
    pub version: String,
    pub request_timeout: Duration,
    pub registration_interval: Duration,
    registration: SecretString,
    wallet_key: SecretString,
    pub peer_identity: Option<auki_p2p::PeerIdentityProof>,
}

pub(crate) fn endpoint(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| TaskError::Configuration("invalid endpoint"))?;
    let local = url.host_str().is_some_and(|h| {
        h == "localhost"
            || h == "[::1]"
            || h.parse::<std::net::IpAddr>().is_ok_and(|a| a.is_loopback())
    });
    if !(url.scheme() == "https" || url.scheme() == "http" && local)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(TaskError::Configuration(
            "use HTTPS endpoints (HTTP only for loopback fixtures)",
        ));
    }
    Ok(url)
}

impl ComputeConfig {
    pub fn new(
        dds_url: &str,
        dms_url: &str,
        registration: SecretString,
        wallet_key: SecretString,
        version: &str,
        client_id: &str,
    ) -> Result<Self> {
        if version.is_empty()
            || version.len() > 128
            || client_id.is_empty()
            || client_id.len() > 128
            || !client_id.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(TaskError::Configuration(
                "provide version and a stable client ID",
            ));
        }
        siwe::derive_eth_address(wallet_key.expose_secret())
            .map_err(|_| TaskError::Configuration("invalid wallet key"))?;
        if registration.expose_secret().is_empty() {
            return Err(TaskError::Configuration(
                "registration credential is required",
            ));
        }
        Ok(Self {
            dds_url: endpoint(dds_url)?,
            dms_url: endpoint(dms_url)?,
            registration,
            wallet_key,
            client_id: client_id.into(),
            version: version.into(),
            request_timeout: Duration::from_secs(30),
            registration_interval: Duration::from_secs(120),
            peer_identity: None,
        })
    }
}

struct ComputeAuthenticator(Arc<ComputeConfig>);

#[async_trait]
impl AccessAuthenticator for ComputeAuthenticator {
    async fn login(&self) -> std::result::Result<machine::AccessBundle, machine::SiweError> {
        let key = self.0.wallet_key.expose_secret();
        let address = siwe::derive_eth_address(key)
            .map_err(|_| machine::SiweError::MissingField("valid wallet key"))?;
        let operation = async {
            let meta = siwe::request_nonce(self.0.dds_url.as_str(), &address).await?;
            let message = siwe::compose_message(&meta, &address, None)?;
            let signature = siwe::sign_message(key, &message)?;
            let bundle =
                siwe::verify(self.0.dds_url.as_str(), &address, &message, &signature).await?;
            crate::machine::bind_peer(
                &self.0.dds_url,
                self.0.request_timeout,
                self.0.peer_identity.as_ref(),
                bundle,
            )
            .await
        };
        tokio::time::timeout(self.0.request_timeout, operation)
            .await
            .map_err(|_| {
                machine::SiweError::MissingField("authentication response before deadline")
            })?
    }
}

type Manager = TokenManager<ComputeAuthenticator, SystemClock>;

struct Owner {
    config: Arc<ComputeConfig>,
    manager: Arc<Manager>,
    closed: CancellationToken,
    startup: Mutex<Option<Vec<String>>>,
    registrar: Mutex<Option<JoinHandle<()>>>,
    runtime_attached: AtomicBool,
    failed: Arc<AtomicBool>,
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

/// Owns registration and serialized SIWE renewal. Constructing it does no I/O.
#[derive(Clone)]
pub struct AukiComputeCredential(Arc<Owner>);

impl AukiComputeCredential {
    pub fn new(config: ComputeConfig) -> Result<Self> {
        endpoint(config.dds_url.as_str())?;
        endpoint(config.dms_url.as_str())?;
        if config.request_timeout.is_zero()
            || config.request_timeout > Duration::from_secs(300)
            || config.registration_interval < Duration::from_millis(100)
            || config.registration_interval > Duration::from_secs(3600)
        {
            return Err(TaskError::Configuration(
                "invalid machine timeout/registration interval",
            ));
        }
        let config = Arc::new(config);
        let manager = Arc::new(TokenManager::new(
            Arc::new(ComputeAuthenticator(config.clone())),
            Arc::new(SystemClock),
            TokenManagerConfig::default(),
        ));
        Ok(Self(Arc::new(Owner {
            config,
            manager,
            closed: CancellationToken::new(),
            startup: Mutex::new(None),
            registrar: Mutex::new(None),
            runtime_attached: AtomicBool::new(false),
            failed: Arc::new(AtomicBool::new(false)),
            authentication: Arc::new(tokio::sync::RwLock::new(())),
        })))
    }

    pub fn config(&self) -> &ComputeConfig {
        &self.0.config
    }

    pub(crate) fn attach_runtime(&self) -> Result<()> {
        if self.0.runtime_attached.swap(true, Ordering::AcqRel) {
            return Err(TaskError::Configuration(
                "compute credential already has a task runtime",
            ));
        }
        Ok(())
    }

    pub async fn wait_closed(&self) {
        self.0.closed.cancelled().await;
    }

    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.0.closed.clone()
    }

    pub(crate) fn failed(&self) -> bool {
        self.0.failed.load(Ordering::Acquire)
    }

    pub(crate) async fn start(
        &self,
        capabilities: &[String],
        cancellation: &CancellationToken,
    ) -> Result<()> {
        let operation = async {
            let mut started = self.0.startup.lock().await;
            if let Some(previous) = started.as_ref() {
                return if previous == capabilities {
                    Ok(())
                } else {
                    Err(TaskError::Configuration(
                        "credential already registered with different capabilities",
                    ))
                };
            }
            register(&self.0.config, capabilities).await?;
            self.bearer().await.map_err(|_| TaskError::Authentication)?;
            let config = self.0.config.clone();
            let caps = capabilities.to_vec();
            let closed = self.0.closed.clone();
            let failed = self.0.failed.clone();
            let mut registrar = self.0.registrar.lock().await;
            // Store the handle before returning; close() can always await it.
            *registrar = Some(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = closed.cancelled() => return,
                        _ = tokio::time::sleep(config.registration_interval) => {}
                    }
                    tokio::select! {
                        biased;
                        _ = closed.cancelled() => return,
                        result = register(&config, &caps) => {
                            if result.is_err() { failed.store(true, Ordering::Release); closed.cancel(); return; }
                        }
                    }
                }
            }));
            *started = Some(capabilities.to_vec());
            Ok(())
        };
        tokio::select! {
            biased;
            _ = self.0.closed.cancelled() => Err(TaskError::Closed),
            _ = cancellation.cancelled() => Err(TaskError::Cancelled),
            result = operation => result,
        }
    }

    pub async fn close(&self) {
        self.0.closed.cancel();
        // Synchronize with startup, including cancellation during registration.
        let _startup = self.0.startup.lock().await;
        let mut registrar = self.0.registrar.lock().await;
        if let Some(task) = registrar.as_mut() {
            let _ = task.await;
        }
        registrar.take();
        self.0.manager.stop().await;
        let _authentication = self.0.authentication.write().await;
    }
}

#[async_trait]
impl TokenProvider for AukiComputeCredential {
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

async fn register(config: &ComputeConfig, capabilities: &[String]) -> Result<()> {
    let key = registration::crypto::load_secp256k1_privhex(config.wallet_key.expose_secret())
        .map_err(|_| TaskError::Authentication)?;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(config.request_timeout)
        .build()
        .map_err(|_| TaskError::Authentication)?;
    for attempt in 0..3 {
        let result = registration::register_once(
            config.dds_url.as_str(),
            &config.version,
            config.registration.expose_secret(),
            &key,
            &http,
            capabilities,
        )
        .await;
        match result.kind() {
            RegistrationAttemptKind::Registered => return Ok(()),
            RegistrationAttemptKind::RetryableFailure if attempt < 2 => {
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await
            }
            _ => return Err(TaskError::Authentication),
        }
    }
    Err(TaskError::Authentication)
}
