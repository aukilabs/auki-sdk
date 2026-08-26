use std::{sync::Arc, time::Duration};

use auki_network::{
    protocol_ids::RESOURCES_V0_3_0,
    resources_protocol::{ResourcesRequest as ResourcesRequestV2, Variant as VariantV2},
    resources_v3_protocol::{
        MAX_RESOURCES_FRAME_BYTES, MessageChannelResource, ResourceEntry, ResourceVariant,
        ResourcesProtocolError, ResourcesRequest, ResourcesResponse, read_resources_request,
        read_resources_response, write_resources_request, write_resources_response,
    },
};
use auki_p2p::PeerId;
use parking_lot::Mutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::{
    protocols::{
        DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
        DomainProtocols,
    },
    resources_v2::{ResourcesV2, ResourcesV2Error},
};

const RESOURCES_V3_MAX_CONCURRENCY: usize = 16;
const RESOURCES_V3_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) trait MessageChannelCatalogProvider: Send + Sync + 'static {
    fn message_channel_catalog(&self) -> Vec<MessageChannelResource>;
}

#[derive(Clone)]
pub(crate) struct ResourcesV3 {
    local_peer_id: PeerId,
    protocols: DomainProtocols,
    resources_v2: ResourcesV2,
    lifecycle: CancellationToken,
    message_channels: Arc<Mutex<Option<Arc<dyn MessageChannelCatalogProvider>>>>,
}

impl ResourcesV3 {
    pub(super) fn new(
        local_peer_id: PeerId,
        protocols: DomainProtocols,
        resources_v2: ResourcesV2,
        lifecycle: CancellationToken,
    ) -> Self {
        Self {
            local_peer_id,
            protocols,
            resources_v2,
            lifecycle,
            message_channels: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn register(&self) -> Result<DomainProtocolRegistration, ResourcesV3Error> {
        let spec = DomainProtocolSpec::new(
            RESOURCES_V0_3_0,
            RESOURCES_V3_MAX_CONCURRENCY,
            MAX_RESOURCES_FRAME_BYTES,
        )?;
        let resources = self.clone();
        self.protocols
            .register(spec, move |stream| {
                let resources = resources.clone();
                async move {
                    if let Err(error) = resources.handle(stream).await {
                        tracing::warn!(%error, "authenticated resource catalog v0.3 request failed");
                    }
                }
            })
            .map_err(ResourcesV3Error::Protocol)
    }

    pub(crate) fn set_message_channel_provider(
        &self,
        provider: Arc<dyn MessageChannelCatalogProvider>,
    ) -> Result<(), ResourcesV3Error> {
        self.ensure_running()?;
        let mut current = self.message_channels.lock();
        self.ensure_running()?;
        *current = Some(provider);
        Ok(())
    }

    pub(crate) fn local(
        &self,
        request: &ResourcesRequest,
    ) -> Result<ResourcesResponse, ResourcesV3Error> {
        self.ensure_running()?;
        request.validate()?;

        let wants_v2 = request.variants.is_empty()
            || request
                .variants
                .iter()
                .any(|variant| *variant != ResourceVariant::MessageChannel);
        let mut resources = if wants_v2 {
            let variants = request
                .variants
                .iter()
                .filter_map(|variant| match variant {
                    ResourceVariant::SensorLog => Some(VariantV2::SensorLog),
                    ResourceVariant::PoseLog => Some(VariantV2::PoseLog),
                    ResourceVariant::TimeTransformLog => Some(VariantV2::TimeTransformLog),
                    ResourceVariant::DetectionLog => Some(VariantV2::DetectionLog),
                    ResourceVariant::MessageChannel => None,
                })
                .collect();
            self.resources_v2
                .local(&ResourcesRequestV2 { variants })?
                .resources
                .into_iter()
                .map(Box::new)
                .map(ResourceEntry::V2)
                .collect()
        } else {
            Vec::new()
        };

        if request.includes(ResourceVariant::MessageChannel) {
            let provider = {
                let current = self.message_channels.lock();
                self.ensure_running()?;
                current.clone()
            };
            if let Some(provider) = provider {
                for channel in provider.message_channel_catalog() {
                    if channel.owner_peer_id != self.local_peer_id {
                        return Err(ResourcesV3Error::MessageChannelOwnerMismatch {
                            expected: Box::new(self.local_peer_id),
                            actual: Box::new(channel.owner_peer_id),
                        });
                    }
                    resources.push(ResourceEntry::MessageChannel(channel));
                }
            }
        }
        let response = ResourcesResponse { resources }.filtered(request);
        response.validate()?;
        Ok(response)
    }

    pub(crate) async fn fetch(
        &self,
        expected_peer: PeerId,
        request: ResourcesRequest,
    ) -> Result<ResourcesResponse, ResourcesV3Error> {
        self.ensure_running()?;
        timeout(RESOURCES_V3_EXCHANGE_TIMEOUT, async {
            let mut stream = self.protocols.open(expected_peer, RESOURCES_V0_3_0).await?;
            write_resources_request(&mut stream, &request).await?;
            read_resources_response(&mut stream)
                .await
                .map_err(ResourcesV3Error::Codec)
        })
        .await
        .map_err(|_| ResourcesV3Error::Timeout(RESOURCES_V3_EXCHANGE_TIMEOUT))?
    }

    async fn handle(&self, mut stream: DomainProtocolStream) -> Result<(), ResourcesV3Error> {
        timeout(RESOURCES_V3_EXCHANGE_TIMEOUT, async {
            let request = read_resources_request(&mut stream).await?;
            let response = self.local(&request)?;
            write_resources_response(&mut stream, &response).await?;
            Ok(())
        })
        .await
        .map_err(|_| ResourcesV3Error::Timeout(RESOURCES_V3_EXCHANGE_TIMEOUT))?
    }

    fn ensure_running(&self) -> Result<(), ResourcesV3Error> {
        if self.lifecycle.is_cancelled() {
            Err(ResourcesV3Error::Stopped)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ResourcesV3Error {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("resource catalog v0.3 protocol failed: {0}")]
    Protocol(#[from] DomainProtocolError),
    #[error("resource catalog v0.2 source failed: {0}")]
    ResourcesV2(#[source] Box<ResourcesV2Error>),
    #[error("resource catalog v0.3 codec failed: {0}")]
    Codec(#[from] ResourcesProtocolError),
    #[error("message-channel owner {actual} does not match local peer {expected}")]
    MessageChannelOwnerMismatch {
        expected: Box<PeerId>,
        actual: Box<PeerId>,
    },
    #[error("resource catalog v0.3 exchange exceeded {0:?}")]
    Timeout(Duration),
}

impl From<ResourcesV2Error> for ResourcesV3Error {
    fn from(error: ResourcesV2Error) -> Self {
        Self::ResourcesV2(Box::new(error))
    }
}
