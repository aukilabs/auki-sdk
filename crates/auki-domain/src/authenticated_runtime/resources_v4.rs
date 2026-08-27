use std::{sync::Arc, time::Duration};

use auki_p2p::PeerId;
use auki_protocols::catalog::v4::{
    ID as RESOURCES_V0_4_0, MAX_RESOURCES_FRAME_BYTES, ResourcesProtocolError, ResourcesRequest,
    ResourcesResponse, read_resources_request, read_resources_response, write_resources_request,
    write_resources_response,
};
use parking_lot::Mutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::resource_catalog::MapCatalogProvider;

use super::protocols::{
    DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
    DomainProtocols,
};

const RESOURCES_V4_MAX_CONCURRENCY: usize = 16;
const RESOURCES_V4_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct ResourcesV4 {
    protocols: DomainProtocols,
    lifecycle: CancellationToken,
    provider: Arc<Mutex<Option<Arc<dyn MapCatalogProvider>>>>,
}

impl ResourcesV4 {
    pub(super) fn new(protocols: DomainProtocols, lifecycle: CancellationToken) -> Self {
        Self {
            protocols,
            lifecycle,
            provider: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn register(&self) -> Result<DomainProtocolRegistration, ResourcesV4Error> {
        let spec = DomainProtocolSpec::new(
            RESOURCES_V0_4_0,
            RESOURCES_V4_MAX_CONCURRENCY,
            MAX_RESOURCES_FRAME_BYTES,
        )?;
        let resources = self.clone();
        self.protocols
            .register(spec, move |stream| {
                let resources = resources.clone();
                async move {
                    if let Err(error) = resources.handle(stream).await {
                        tracing::warn!(%error, "authenticated resource catalog v0.4 request failed");
                    }
                }
            })
            .map_err(ResourcesV4Error::Protocol)
    }

    pub(crate) fn set_provider(
        &self,
        provider: Arc<dyn MapCatalogProvider>,
    ) -> Result<(), ResourcesV4Error> {
        self.ensure_running()?;
        let mut current = self.provider.lock();
        self.ensure_running()?;
        *current = Some(provider);
        Ok(())
    }

    pub(crate) fn local(&self) -> Result<ResourcesResponse, ResourcesV4Error> {
        self.ensure_running()?;
        let provider = {
            let current = self.provider.lock();
            self.ensure_running()?;
            current.clone()
        };
        let response = provider
            .map(|provider| provider.map_catalog())
            .unwrap_or_else(|| ResourcesResponse {
                resources: Vec::new(),
            });
        response.validate()?;
        Ok(response)
    }

    pub(crate) async fn fetch(
        &self,
        expected_peer: PeerId,
    ) -> Result<ResourcesResponse, ResourcesV4Error> {
        self.ensure_running()?;
        timeout(RESOURCES_V4_EXCHANGE_TIMEOUT, async {
            let mut stream = self.protocols.open(expected_peer, RESOURCES_V0_4_0).await?;
            write_resources_request(&mut stream, &ResourcesRequest::all()).await?;
            read_resources_response(&mut stream)
                .await
                .map_err(ResourcesV4Error::Codec)
        })
        .await
        .map_err(|_| ResourcesV4Error::Timeout(RESOURCES_V4_EXCHANGE_TIMEOUT))?
    }

    async fn handle(&self, mut stream: DomainProtocolStream) -> Result<(), ResourcesV4Error> {
        timeout(RESOURCES_V4_EXCHANGE_TIMEOUT, async {
            read_resources_request(&mut stream).await?;
            let response = self.local()?;
            write_resources_response(&mut stream, &response).await?;
            Ok(())
        })
        .await
        .map_err(|_| ResourcesV4Error::Timeout(RESOURCES_V4_EXCHANGE_TIMEOUT))?
    }

    fn ensure_running(&self) -> Result<(), ResourcesV4Error> {
        if self.lifecycle.is_cancelled() {
            Err(ResourcesV4Error::Stopped)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResourcesV4Error {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("resource catalog v0.4 protocol failed: {0}")]
    Protocol(#[from] DomainProtocolError),
    #[error("resource catalog v0.4 codec failed: {0}")]
    Codec(#[from] ResourcesProtocolError),
    #[error("resource catalog v0.4 exchange exceeded {0:?}")]
    Timeout(Duration),
}
