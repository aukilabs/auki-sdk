//! Optional task networking. The DMS lifecycle supplies authority; AukiPeer owns transport.
use crate::{
    AukiDiscovery, AukiKnownPeers, AukiPeer, AukiPeerConfig, AukiPeerLifecycle,
    AukiPeerProtocolContext, ExternalAuthorityControl, ExternalAuthorityUpdate, Identity,
    Multiaddr, PeerId,
};
use async_trait::async_trait;
use auki_auth::machine::p2p::DdsP2pClient;
use auki_p2p::{DdsTokenVerifier, PeerIdentityProof};
use auki_tasks::{TaskContext, TaskError, TaskPeerFactory, TaskPeerGrant, TaskPeerSession};
use parking_lot::Mutex;
use std::{any::Any, ops::Deref, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// Configure one persistent identity for task-scoped peers. No I/O until a task runs.
#[derive(Clone)]
pub struct AukiTaskPeerConfig {
    identity: Identity,
    dds_url: reqwest::Url,
    dds: DdsP2pClient,
    config: AukiPeerConfig,
}
impl AukiTaskPeerConfig {
    pub fn new(
        identity: Identity,
        dds_url: &str,
        config: AukiPeerConfig,
    ) -> Result<Self, TaskError> {
        let dds_url = reqwest::Url::parse(dds_url)
            .map_err(|_| TaskError::Configuration("invalid task DDS URL"))?;
        if config.dds_tracker().is_some_and(|t| {
            t.base_url().trim_end_matches('/') != dds_url.as_str().trim_end_matches('/')
        }) {
            return Err(TaskError::Configuration(
                "task discovery must use the machine DDS",
            ));
        }
        let dds = DdsP2pClient::new(dds_url.clone(), Duration::from_secs(10))
            .map_err(|_| TaskError::Configuration("task DDS P2P client"))?;
        Ok(Self {
            identity,
            dds_url,
            dds,
            config,
        })
    }
    pub fn identity_proof(&self) -> PeerIdentityProof {
        self.identity.proof()
    }
}

async fn authority(
    dds: &DdsP2pClient,
    identity: &Identity,
    grant: &TaskPeerGrant,
) -> Result<ExternalAuthorityUpdate, TaskError> {
    let material = dds
        .authority_material(
            &identity.proof(),
            grant.domain_id,
            grant.token.expose_secret(),
            grant.expires_at,
        )
        .await
        .map_err(|_| TaskError::Authority("DDS task peer authority unavailable"))?;
    let verifier = DdsTokenVerifier::from_keys(material.verification_keys.clone())
        .map_err(|_| TaskError::Authority("invalid DDS verification keys"))?;
    let claims = verifier
        .verify_credential(&material.credential)
        .map_err(|_| TaskError::Authority("invalid signed task peer credential"))?;
    if claims.peer_type.as_deref() != Some(grant.peer_type)
        || !claims
            .scopes
            .iter()
            .any(|scope| scope == auki_p2p::P2P_TOKEN_SCOPE)
    {
        return Err(TaskError::Authority(
            "task peer lacks the expected machine type or required scope",
        ));
    }
    // AukiPeer additionally pins Domain/Peer ID, checks literal expiry, installs
    // keys monotonically and enforces the complete DDS P2P profile.
    Ok(ExternalAuthorityUpdate::new(
        material.domain_id,
        material.peer_id,
        material.verification_keys,
        material.credential,
        material.expires_at,
    ))
}

/// Handler view of task-owned networking. Retaining it does not retain authority.
#[derive(Clone)]
pub struct AukiTaskPeer {
    context: AukiPeerProtocolContext,
    lifecycle: AukiPeerLifecycle,
    known_peers: AukiKnownPeers,
    discovery: Option<AukiDiscovery>,
    listen_addresses: Vec<Multiaddr>,
}
impl Deref for AukiTaskPeer {
    type Target = AukiPeerProtocolContext;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
impl AukiTaskPeer {
    pub fn lifecycle(&self) -> AukiPeerLifecycle {
        self.lifecycle.clone()
    }
    pub fn known_peers(&self) -> AukiKnownPeers {
        self.known_peers.clone()
    }
    pub fn discovery(&self) -> Option<AukiDiscovery> {
        self.discovery.clone()
    }
    pub fn listen_addresses(&self) -> &[Multiaddr] {
        &self.listen_addresses
    }
}

/// Import this trait to call `task.peer()` from a Rust handler.
pub trait TaskPeerContext {
    fn peer(&self) -> Option<AukiTaskPeer>;
}
impl TaskPeerContext for TaskContext {
    fn peer(&self) -> Option<AukiTaskPeer> {
        self.peer_session()?
            .as_any()
            .downcast_ref::<Session>()
            .map(|s| s.view.clone())
    }
}

struct Session {
    peer: Mutex<Option<AukiPeer>>,
    closing: tokio::sync::Mutex<()>,
    view: AukiTaskPeer,
    control: ExternalAuthorityControl,
    identity: Identity,
    dds: DdsP2pClient,
    peer_type: &'static str,
}
#[async_trait]
impl TaskPeerFactory for AukiTaskPeerConfig {
    fn peer_id(&self) -> PeerId {
        self.identity.peer_id()
    }
    fn dds_url(&self) -> &reqwest::Url {
        &self.dds_url
    }
    fn dms_url(&self) -> &str {
        self.config.dms_base_url()
    }
    async fn start(
        &self,
        grant: TaskPeerGrant,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn TaskPeerSession>, TaskError> {
        let update = tokio::select! { biased;
            _ = cancellation.cancelled() => return Err(TaskError::Cancelled),
            result = authority(&self.dds, &self.identity, &grant) => result?,
        };
        // Await AukiPeer's bounded startup even if cancelled so a booking that
        // succeeds during cancellation can be explicitly released below.
        let (peer, control) =
            AukiPeer::start_external(self.identity.clone(), update, self.config.clone())
                .await
                .map_err(|_| TaskError::Authority("task peer startup failed"))?;
        if cancellation.is_cancelled() {
            peer.shutdown().await.map_err(|_| TaskError::PeerCleanup)?;
            return Err(TaskError::Cancelled);
        }
        let view = AukiTaskPeer {
            context: peer.protocol_context(),
            lifecycle: peer.lifecycle(),
            known_peers: peer.known_peers(),
            discovery: peer.discovery_handle().ok(),
            listen_addresses: peer.listen_addresses().to_vec(),
        };
        Ok(Arc::new(Session {
            peer: Mutex::new(Some(peer)),
            closing: tokio::sync::Mutex::new(()),
            view,
            control,
            identity: self.identity.clone(),
            dds: self.dds.clone(),
            peer_type: grant.peer_type,
        }))
    }
}
#[async_trait]
impl TaskPeerSession for Session {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn fence(&self) {
        self.view.context.fence();
    }
    async fn update(&self, grant: TaskPeerGrant) -> Result<(), TaskError> {
        if grant.peer_type != self.peer_type || grant.domain_id != self.view.domain_id() {
            return Err(TaskError::Authority("task peer principal changed"));
        }
        let update = authority(&self.dds, &self.identity, &grant).await?;
        self.control
            .replace(update)
            .await
            .map_err(|_| TaskError::Authority("task peer rejected authority rotation"))?;
        Ok(())
    }
    async fn refresh_requested(&self) {
        if self.control.next_refresh_request().await.is_none() {
            std::future::pending::<()>().await;
        }
    }
    async fn wait_stopped(&self) {
        self.view.lifecycle.wait_stopped().await;
    }
    async fn shutdown(&self) -> Result<(), TaskError> {
        self.fence();
        let _closing = self.closing.lock().await;
        let peer = self.peer.lock().take();
        if let Some(peer) = peer {
            peer.shutdown().await.map_err(|_| TaskError::PeerCleanup)?;
        }
        Ok(())
    }
}
