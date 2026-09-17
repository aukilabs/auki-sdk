use crate::{AukiCancellation, AukiSdkError, AukiSession, wait_cleanup};
use auki_sdk_rs::{AukiFleet, DomainFleetClient, FleetError};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::{runtime::Handle, sync::watch};
use tokio_util::sync::CancellationToken;

fn error(error: FleetError) -> AukiSdkError {
    AukiSdkError::Fleet {
        kind: error.kind().into(),
        status: error.http_status(),
        code: error.code().into(),
        message: error.to_string(),
    }
}

struct FleetOwner {
    inner: DomainFleetClient,
    closed: CancellationToken,
    completion: Mutex<Option<watch::Sender<Option<crate::CleanupResult>>>>,
    runtime: Mutex<Option<Handle>>,
}

impl FleetOwner {
    fn remember_runtime(&self) -> Option<Handle> {
        let mut runtime = self.runtime.lock();
        if runtime.is_none() {
            *runtime = Handle::try_current().ok();
        }
        runtime.clone()
    }
    fn begin_close(&self) -> Option<watch::Receiver<Option<crate::CleanupResult>>> {
        self.closed.cancel();
        let mut completion = self.completion.lock();
        if let Some(sender) = completion.as_ref() {
            return Some(sender.subscribe());
        }
        let runtime = self.remember_runtime()?;
        let (sender, receiver) = watch::channel(None);
        *completion = Some(sender.clone());
        let inner = self.inner.clone();
        runtime.spawn(async move {
            inner.close().await;
            sender.send_replace(Some(Ok(())));
        });
        Some(receiver)
    }
    fn token(
        &self,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<CancellationToken, AukiSdkError> {
        if self.closed.is_cancelled() {
            return Err(error(FleetError::Closed));
        }
        self.remember_runtime();
        Ok(cancellation
            .map(|value| value.token.child_token())
            .unwrap_or_default())
    }
}
impl Drop for FleetOwner {
    fn drop(&mut self) {
        let _ = self.begin_close();
    }
}

#[derive(uniffi::Object)]
pub struct AukiDomainFleet {
    owner: FleetOwner,
}

impl AukiDomainFleet {
    fn new(inner: DomainFleetClient) -> Self {
        Self {
            owner: FleetOwner {
                inner,
                closed: CancellationToken::new(),
                completion: Mutex::new(None),
                runtime: Mutex::new(Handle::try_current().ok()),
            },
        }
    }
    fn decode<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, AukiSdkError> {
        serde_json::from_str(json)
            .map_err(|_| error(FleetError::InvalidInput("invalid fleet query JSON")))
    }
    fn encode(value: &impl serde::Serialize) -> Result<String, AukiSdkError> {
        serde_json::to_string(value)
            .map_err(|_| error(FleetError::InvalidResponse("cannot encode fleet snapshot")))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiDomainFleet {
    pub fn domain_id(&self) -> String {
        self.owner.inner.domain_id().to_string()
    }
    pub async fn close(&self) -> Result<(), AukiSdkError> {
        let completion = self
            .owner
            .begin_close()
            .expect("UniFFI fleet close runs on the retained Tokio runtime");
        wait_cleanup(completion)
            .await
            .map_err(|_| error(FleetError::Closed))
    }
    #[uniffi::method(default(cancellation=None))]
    pub async fn list_json(
        &self,
        query_json: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<String, AukiSdkError> {
        let token = self.owner.token(cancellation)?;
        let query = Self::decode(&query_json)?;
        Self::encode(
            &self
                .owner
                .inner
                .list_with_cancellation(&query, &token)
                .await
                .map_err(error)?,
        )
    }
    #[uniffi::method(default(cancellation=None))]
    pub async fn compute_pool_json(
        &self,
        query_json: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<String, AukiSdkError> {
        let token = self.owner.token(cancellation)?;
        let query = Self::decode(&query_json)?;
        Self::encode(
            &self
                .owner
                .inner
                .compute_pool_with_cancellation(&query, &token)
                .await
                .map_err(error)?,
        )
    }
}

#[uniffi::export]
impl AukiSession {
    pub fn fleet(&self, domain_id: String) -> Result<Arc<AukiDomainFleet>, AukiSdkError> {
        let domain_id = domain_id
            .parse()
            .map_err(|_| error(FleetError::InvalidInput("expected Domain UUID")))?;
        let config = self
            .peer_config
            .as_ref()
            .ok_or_else(|| error(FleetError::InvalidInput("DMS is not configured")))?;
        let fleet = AukiFleet::new(self.session.clone(), config.dms_base_url()).map_err(error)?;
        Ok(Arc::new(AukiDomainFleet::new(fleet.in_domain(domain_id))))
    }
}
