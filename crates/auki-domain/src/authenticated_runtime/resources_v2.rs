use std::{sync::Arc, time::Duration};

use auki_network::{
    SessionHandle,
    protocol_ids::RESOURCES_V0_2_0,
    resources_protocol::{
        MAX_RESOURCES_FRAME_BYTES, ResourcesProtocolError, ResourcesRequest, ResourcesResponse,
        read_resources_request, read_resources_response, write_resources_request,
        write_resources_response,
    },
};
use auki_p2p::PeerId;
use parking_lot::Mutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::cluster_manager::ResourceCatalogProvider;

use super::protocols::{
    DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
    DomainProtocols,
};

const RESOURCES_V2_MAX_CONCURRENCY: usize = 16;
const RESOURCES_V2_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct CatalogSources {
    provider: Option<Arc<dyn ResourceCatalogProvider>>,
    session: Option<Arc<dyn SessionHandle>>,
}

/// Private adapter for the retained Resource Catalog 0.2.0 payload.
///
/// This owns no transport. It registers and opens only through the single
/// Domain-owned authenticated protocol surface.
#[derive(Clone)]
pub(crate) struct ResourcesV2 {
    protocols: DomainProtocols,
    lifecycle: CancellationToken,
    sources: Arc<Mutex<CatalogSources>>,
}

impl ResourcesV2 {
    pub(super) fn new(protocols: DomainProtocols, lifecycle: CancellationToken) -> Self {
        Self {
            protocols,
            lifecycle,
            sources: Arc::new(Mutex::new(CatalogSources::default())),
        }
    }

    pub(super) fn register(&self) -> Result<DomainProtocolRegistration, ResourcesV2Error> {
        let spec = DomainProtocolSpec::new(
            RESOURCES_V0_2_0,
            RESOURCES_V2_MAX_CONCURRENCY,
            MAX_RESOURCES_FRAME_BYTES,
        )?;
        let resources = self.clone();
        self.protocols
            .register(spec, move |stream| {
                let resources = resources.clone();
                async move {
                    if let Err(error) = resources.handle(stream).await {
                        tracing::warn!(%error, "authenticated resource catalog request failed");
                    }
                }
            })
            .map_err(ResourcesV2Error::Protocol)
    }

    pub(super) fn set_provider(
        &self,
        provider: Arc<dyn ResourceCatalogProvider>,
    ) -> Result<(), ResourcesV2Error> {
        self.ensure_running()?;
        let mut sources = self.sources.lock();
        self.ensure_running()?;
        sources.provider = Some(provider);
        Ok(())
    }

    pub(super) fn set_session(
        &self,
        session: Arc<dyn SessionHandle>,
    ) -> Result<(), ResourcesV2Error> {
        self.ensure_running()?;
        let mut sources = self.sources.lock();
        self.ensure_running()?;
        sources.session = Some(session);
        Ok(())
    }

    pub(super) fn local(
        &self,
        request: &ResourcesRequest,
    ) -> Result<ResourcesResponse, ResourcesV2Error> {
        self.ensure_running()?;
        let (provider, session) = {
            let sources = self.sources.lock();
            self.ensure_running()?;
            (sources.provider.clone(), sources.session.clone())
        };
        let resources = snapshot_catalog(provider, session, request);
        Ok(ResourcesResponse { resources })
    }

    pub(super) async fn fetch(
        &self,
        expected_peer: PeerId,
        request: ResourcesRequest,
    ) -> Result<ResourcesResponse, ResourcesV2Error> {
        self.ensure_running()?;
        timeout(RESOURCES_V2_EXCHANGE_TIMEOUT, async {
            let mut stream = self.protocols.open(expected_peer, RESOURCES_V0_2_0).await?;
            write_resources_request(&mut stream, &request).await?;
            read_resources_response(&mut stream)
                .await
                .map_err(ResourcesV2Error::Codec)
        })
        .await
        .map_err(|_| ResourcesV2Error::Timeout(RESOURCES_V2_EXCHANGE_TIMEOUT))?
    }

    async fn handle(&self, mut stream: DomainProtocolStream) -> Result<(), ResourcesV2Error> {
        timeout(RESOURCES_V2_EXCHANGE_TIMEOUT, async {
            let request = read_resources_request(&mut stream).await?;
            let response = self.local(&request)?;
            write_resources_response(&mut stream, &response).await?;
            Ok(())
        })
        .await
        .map_err(|_| ResourcesV2Error::Timeout(RESOURCES_V2_EXCHANGE_TIMEOUT))?
    }

    fn ensure_running(&self) -> Result<(), ResourcesV2Error> {
        if self.lifecycle.is_cancelled() {
            Err(ResourcesV2Error::Stopped)
        } else {
            Ok(())
        }
    }
}

fn filter_resources(
    resources: Vec<auki_network::resources_protocol::ResourceEntry>,
    request: &ResourcesRequest,
) -> Vec<auki_network::resources_protocol::ResourceEntry> {
    if request.variants.is_empty() {
        return resources;
    }
    use auki_network::resources_protocol::{Variant, VariantContent};
    resources
        .into_iter()
        .filter(|resource| {
            let variant = match &resource.variant_content {
                VariantContent::SensorLog { .. } => Variant::SensorLog,
                VariantContent::PoseLog { .. } => Variant::PoseLog,
                VariantContent::TimeTransformLog { .. } => Variant::TimeTransformLog,
                VariantContent::DetectionLog { .. } => Variant::DetectionLog,
            };
            request.variants.contains(&variant)
        })
        .collect()
}

fn snapshot_catalog(
    provider: Option<Arc<dyn ResourceCatalogProvider>>,
    session: Option<Arc<dyn SessionHandle>>,
    request: &ResourcesRequest,
) -> Vec<auki_network::resources_protocol::ResourceEntry> {
    if let Some(provider) = provider {
        provider.snapshot_for_request(request, None)
    } else if let Some(session) = session {
        filter_resources(session.catalog(), request)
    } else {
        Vec::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ResourcesV2Error {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("resource catalog protocol failed: {0}")]
    Protocol(#[from] DomainProtocolError),
    #[error("resource catalog codec failed: {0}")]
    Codec(#[from] ResourcesProtocolError),
    #[error("resource catalog exchange exceeded {0:?}")]
    Timeout(Duration),
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use auki_network::resources_protocol::{
        Available, Head, ResourceEntry, SensorBlock, SensorKind, SensorManifestPointer, Variant,
        VariantContent,
    };
    use auki_registry::RegistryRef;

    use super::*;

    struct Provider {
        resources: Vec<ResourceEntry>,
        calls: Arc<AtomicUsize>,
    }

    impl ResourceCatalogProvider for Provider {
        fn snapshot(&self) -> Vec<ResourceEntry> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.resources.clone()
        }
    }

    struct Session(Vec<ResourceEntry>);

    impl SessionHandle for Session {
        fn catalog(&self) -> Vec<ResourceEntry> {
            self.0.clone()
        }
    }

    fn sensor(resource_id: &str) -> ResourceEntry {
        ResourceEntry {
            source_peer_id: "source".into(),
            writer_peer_id: "writer".into(),
            resource_id: resource_id.into(),
            state: "live".into(),
            head: Some(Head::Rolling { retention_ns: 1 }),
            extent: None,
            available: Available {
                bytes: 1,
                entries: 1,
                duration_ns: 1,
            },
            sensor: Some(SensorBlock {
                kind: SensorKind::Camera,
                r#type: "rgb".into(),
                sensor_id: resource_id.into(),
                sensor_hash: "sensor-hash".into(),
            }),
            pose: None,
            variant_content: VariantContent::SensorLog {
                manifest: SensorManifestPointer {
                    clock: RegistryRef {
                        peer_id: "source".into(),
                        id: "clock".into(),
                        hash: "clock-hash".into(),
                    },
                    frame: None,
                },
            },
        }
    }

    #[test]
    fn provider_precedes_session_and_is_sampled_for_each_filtered_request() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn ResourceCatalogProvider> = Arc::new(Provider {
            resources: vec![sensor("provider")],
            calls: Arc::clone(&calls),
        });
        let session: Arc<dyn SessionHandle> = Arc::new(Session(vec![sensor("session")]));
        let request = ResourcesRequest {
            variants: vec![Variant::SensorLog],
        };

        for _ in 0..2 {
            let resources = snapshot_catalog(
                Some(Arc::clone(&provider)),
                Some(Arc::clone(&session)),
                &request,
            );
            assert_eq!(resources, vec![sensor("provider")]);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            snapshot_catalog(None, Some(session), &request),
            vec![sensor("session")]
        );
        assert!(snapshot_catalog(None, None, &request).is_empty());
    }
}
