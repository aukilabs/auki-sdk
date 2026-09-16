use async_trait::async_trait;
use auki_auth::machine::{
    AccessBundle, SiweError,
    p2p::{DdsP2pClient, PeerBindingClient},
    token_manager::{TokenProvider, TokenProviderResult},
};
use auki_p2p::{PeerId, PeerIdentityProof};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{AukiComputeCredential, AukiRobotCredential, Result};

/// Keep a serialized token-manager refresh alive if its caller is cancelled.
/// Shutdown cancels it and awaits the active guard before releasing credentials.
pub(crate) async fn owned_bearer<P: TokenProvider + 'static>(
    manager: std::sync::Arc<P>,
    closed: CancellationToken,
    active: std::sync::Arc<tokio::sync::RwLock<()>>,
) -> TokenProviderResult<String> {
    let guard = active.read_owned().await;
    if closed.is_cancelled() {
        return Err(
            auki_auth::machine::token_manager::TokenProviderError::Message(
                "machine credential closed".into(),
            ),
        );
    }
    tokio::spawn(async move {
        let _guard = guard;
        tokio::select! { biased;
            _ = closed.cancelled() => Err(auki_auth::machine::token_manager::TokenProviderError::Message("machine credential closed".into())),
            result = manager.bearer() => result,
        }
    }).await.map_err(|_| auki_auth::machine::token_manager::TokenProviderError::Message("machine authentication stopped".into()))?
}

/// Distinct machine identities sharing the same task execution lifecycle.
#[derive(Clone)]
pub enum MachineCredential {
    Compute(AukiComputeCredential),
    Robot(AukiRobotCredential),
}
impl From<AukiComputeCredential> for MachineCredential {
    fn from(value: AukiComputeCredential) -> Self {
        Self::Compute(value)
    }
}
impl From<AukiRobotCredential> for MachineCredential {
    fn from(value: AukiRobotCredential) -> Self {
        Self::Robot(value)
    }
}
impl MachineCredential {
    pub fn dds_url(&self) -> &Url {
        match self {
            Self::Compute(c) => &c.config().dds_url,
            Self::Robot(r) => &r.config().dds_url,
        }
    }
    pub fn dms_url(&self) -> &Url {
        match self {
            Self::Compute(c) => &c.config().dms_url,
            Self::Robot(r) => &r.config().dms_url,
        }
    }
    pub fn client_id(&self) -> &str {
        match self {
            Self::Compute(c) => &c.config().client_id,
            Self::Robot(r) => &r.config().client_id,
        }
    }
    pub fn peer_id(&self) -> Option<PeerId> {
        match self {
            Self::Compute(c) => c.config().peer_identity.as_ref(),
            Self::Robot(r) => r.config().peer_identity.as_ref(),
        }
        .map(PeerIdentityProof::peer_id)
    }
    pub(crate) fn attach_runtime(&self, capabilities: &[String]) -> Result<()> {
        match self {
            Self::Compute(c) => c.attach_runtime(),
            Self::Robot(r) => r.attach_runtime(capabilities),
        }
    }
    pub(crate) async fn start(
        &self,
        capabilities: &[String],
        cancellation: &CancellationToken,
    ) -> Result<()> {
        match self {
            Self::Compute(c) => c.start(capabilities, cancellation).await,
            Self::Robot(r) => r.start(cancellation).await,
        }
    }
    pub(crate) fn cancellation(&self) -> CancellationToken {
        match self {
            Self::Compute(c) => c.cancellation(),
            Self::Robot(r) => r.cancellation(),
        }
    }
    pub(crate) async fn wait_closed(&self) {
        self.cancellation().cancelled().await;
    }
    pub(crate) fn failed(&self) -> bool {
        match self {
            Self::Compute(c) => c.failed(),
            Self::Robot(r) => r.failed(),
        }
    }
    pub async fn close(&self) {
        match self {
            Self::Compute(c) => c.close().await,
            Self::Robot(r) => r.close().await,
        }
    }
}
#[async_trait]
impl TokenProvider for MachineCredential {
    async fn bearer(&self) -> TokenProviderResult<String> {
        match self {
            Self::Compute(c) => c.bearer().await,
            Self::Robot(r) => r.bearer().await,
        }
    }
    async fn on_unauthorized(&self) {
        match self {
            Self::Compute(c) => c.on_unauthorized().await,
            Self::Robot(r) => r.on_unauthorized().await,
        }
    }
}

pub(crate) async fn bind_peer(
    dds: &Url,
    timeout: std::time::Duration,
    identity: Option<&PeerIdentityProof>,
    bundle: AccessBundle,
) -> std::result::Result<AccessBundle, SiweError> {
    let Some(identity) = identity else {
        return Ok(bundle);
    };
    let failure = |_| SiweError::MissingField("valid DDS peer binding");
    let client = DdsP2pClient::new(dds.clone(), timeout).map_err(failure)?;
    PeerBindingClient::new(client, identity.clone())
        .bind(&bundle)
        .await
        .map_err(failure)
}
