use crate::{Result, TaskError};
use async_trait::async_trait;
use auki_auth::SecretString;
use auki_p2p::PeerId;
use chrono::{DateTime, Utc};
use std::{any::Any, sync::Arc};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

/// Separate signed transport authority; never usable as Domain data authority.
#[derive(Clone, Debug)]
pub struct TaskPeerGrant {
    pub domain_id: Uuid,
    pub token: Arc<SecretString>,
    pub peer_type: &'static str,
    pub expires_at: DateTime<Utc>,
}
impl TaskPeerGrant {
    pub(crate) fn from_wire(
        domain_id: Uuid,
        token: Option<&str>,
        expiry: Option<DateTime<Utc>>,
    ) -> Result<Option<Self>> {
        match (token, expiry) {
            (None, None) => Ok(None),
            (Some(token), Some(expires_at)) if !token.is_empty() && expires_at > Utc::now() => {
                Ok(Some(Self {
                    domain_id,
                    token: Arc::new(SecretString::new(token)),
                    expires_at,
                    peer_type: "compute",
                }))
            }
            _ => Err(TaskError::Authority("incomplete or expired peer grant")),
        }
    }
}

/// SDK adapter boundary. The SDK owns transport startup, authorization and shutdown.
#[async_trait]
pub trait TaskPeerFactory: Send + Sync {
    fn peer_id(&self) -> PeerId;
    fn dds_url(&self) -> &Url;
    fn dms_url(&self) -> &str;
    /// Must finish cleanup when cancelled; callers await this operation.
    async fn start(
        &self,
        grant: TaskPeerGrant,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn TaskPeerSession>>;
}

/// One task's optional transport. Heartbeats supply all replacement credentials.
#[async_trait]
pub trait TaskPeerSession: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    /// Fence application operations synchronously before handler cleanup.
    fn fence(&self);
    async fn update(&self, grant: TaskPeerGrant) -> Result<()>;
    async fn refresh_requested(&self);
    async fn wait_stopped(&self);
    async fn shutdown(&self) -> Result<()>;
}
