//! Explicit all-in-one Swift bundle for the standard portable protocols.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::watch;

use crate::{
    AukiPeer, AukiPeerCard, AukiSdkError, CleanupResult, DetachedCleanup, operation_error,
    wait_cleanup,
};

use super::{
    blob::AukiBlobEndpoint, catalog::AukiCatalogEndpoint, info::AukiInfoEndpoint,
    message::AukiMessageEndpoint, registry::AukiRegistryEndpoint, stream::AukiStreamEndpoint,
};

const STANDARD_PROTOCOL_IDS: [&str; 7] = [
    auki_protocols::info::v1::ID,
    auki_protocols::catalog::v3::ID,
    auki_protocols::catalog::v4::ID,
    auki_protocols::registry::v3::ID,
    auki_protocols::blob::v1::ID,
    auki_protocols::message::v1::ID,
    auki_protocols::stream::v2::ID,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StandardEndpointKind {
    Stream,
    Message,
    Blob,
    Registry,
    Catalog,
    Info,
}

impl StandardEndpointKind {
    fn name(self) -> &'static str {
        match self {
            Self::Stream => "Stream",
            Self::Message => "Message",
            Self::Blob => "Blob",
            Self::Registry => "Registry",
            Self::Catalog => "Catalog",
            Self::Info => "Info",
        }
    }
}

const STANDARD_CLOSE_ORDER: [StandardEndpointKind; 6] = [
    StandardEndpointKind::Stream,
    StandardEndpointKind::Message,
    StandardEndpointKind::Blob,
    StandardEndpointKind::Registry,
    StandardEndpointKind::Catalog,
    StandardEndpointKind::Info,
];

#[derive(Default)]
struct PartialStandardEndpoints {
    info: Option<Arc<AukiInfoEndpoint>>,
    catalog: Option<Arc<AukiCatalogEndpoint>>,
    registry: Option<Arc<AukiRegistryEndpoint>>,
    blob: Option<Arc<AukiBlobEndpoint>>,
    message: Option<Arc<AukiMessageEndpoint>>,
    stream: Option<Arc<AukiStreamEndpoint>>,
}

impl PartialStandardEndpoints {
    fn complete(self) -> StandardEndpoints {
        StandardEndpoints {
            info: self
                .info
                .expect("standard mount completes only after Info is mounted"),
            catalog: self
                .catalog
                .expect("standard mount completes only after Catalog is mounted"),
            registry: self
                .registry
                .expect("standard mount completes only after Registry is mounted"),
            blob: self
                .blob
                .expect("standard mount completes only after Blob is mounted"),
            message: self
                .message
                .expect("standard mount completes only after Message is mounted"),
            stream: self
                .stream
                .expect("standard mount completes only after Stream is mounted"),
        }
    }

    async fn close_ordered(&self) -> Result<(), String> {
        let mut first_error = None;
        for kind in STANDARD_CLOSE_ORDER {
            let result = match kind {
                StandardEndpointKind::Stream => match self.stream.as_ref() {
                    Some(endpoint) => endpoint.close().await,
                    None => Ok(()),
                },
                StandardEndpointKind::Message => match self.message.as_ref() {
                    Some(endpoint) => endpoint.close().await,
                    None => Ok(()),
                },
                StandardEndpointKind::Blob => match self.blob.as_ref() {
                    Some(endpoint) => endpoint.close().await,
                    None => Ok(()),
                },
                StandardEndpointKind::Registry => match self.registry.as_ref() {
                    Some(endpoint) => endpoint.close().await,
                    None => Ok(()),
                },
                StandardEndpointKind::Catalog => match self.catalog.as_ref() {
                    Some(endpoint) => endpoint.close().await,
                    None => Ok(()),
                },
                StandardEndpointKind::Info => match self.info.as_ref() {
                    Some(endpoint) => endpoint.close().await,
                    None => Ok(()),
                },
            };
            record_close_error(&mut first_error, kind, result);
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn record_close_error(
    first_error: &mut Option<String>,
    kind: StandardEndpointKind,
    result: Result<(), AukiSdkError>,
) {
    if first_error.is_none()
        && let Err(error) = result
    {
        *first_error = Some(format!("close {} endpoint: {error}", kind.name()));
    }
}

#[derive(Clone)]
struct StandardEndpoints {
    info: Arc<AukiInfoEndpoint>,
    catalog: Arc<AukiCatalogEndpoint>,
    registry: Arc<AukiRegistryEndpoint>,
    blob: Arc<AukiBlobEndpoint>,
    message: Arc<AukiMessageEndpoint>,
    stream: Arc<AukiStreamEndpoint>,
}

impl StandardEndpoints {
    fn partial(&self) -> PartialStandardEndpoints {
        PartialStandardEndpoints {
            info: Some(Arc::clone(&self.info)),
            catalog: Some(Arc::clone(&self.catalog)),
            registry: Some(Arc::clone(&self.registry)),
            blob: Some(Arc::clone(&self.blob)),
            message: Some(Arc::clone(&self.message)),
            stream: Some(Arc::clone(&self.stream)),
        }
    }
}

struct StandardProtocolsOwner {
    closing: AtomicBool,
    cleanup: DetachedCleanup,
}

impl StandardProtocolsOwner {
    fn new() -> Self {
        Self {
            closing: AtomicBool::new(false),
            cleanup: DetachedCleanup::new(),
        }
    }

    fn ensure_open(&self) -> Result<(), AukiSdkError> {
        if self.closing.load(Ordering::SeqCst) {
            Err(operation_error(
                "build standard protocol peer card",
                "standard protocol bundle is closing",
            ))
        } else {
            Ok(())
        }
    }

    fn begin_close(&self, endpoints: &StandardEndpoints) -> watch::Receiver<Option<CleanupResult>> {
        self.closing.store(true, Ordering::SeqCst);
        self.cleanup.get_or_start(|| {
            let endpoints = endpoints.partial();
            async move { endpoints.close_ordered().await }
        })
    }
}

/// Explicit opt-in bundle mounting every standard Auki protocol on one peer.
///
/// Applications may still mount and use each endpoint independently. This
/// convenience owner only removes repetitive setup, card construction, and
/// shutdown ordering for applications that want the complete standard set.
#[derive(uniffi::Object)]
pub struct AukiStandardProtocols {
    peer: Arc<AukiPeer>,
    endpoints: StandardEndpoints,
    owner: StandardProtocolsOwner,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiStandardProtocols {
    /// Mount Info, Catalog, Registry, Blob, Message, and Stream as one bundle.
    #[uniffi::constructor]
    pub async fn mount(peer: Arc<AukiPeer>) -> Result<Arc<Self>, AukiSdkError> {
        let mut mounted = PartialStandardEndpoints::default();
        let mounting: Result<(), AukiSdkError> = async {
            mounted.info = Some(AukiInfoEndpoint::mount(Arc::clone(&peer)).await?);
            mounted.catalog = Some(AukiCatalogEndpoint::mount(Arc::clone(&peer)).await?);
            mounted.registry = Some(AukiRegistryEndpoint::mount(Arc::clone(&peer)).await?);
            mounted.blob = Some(AukiBlobEndpoint::mount(Arc::clone(&peer)).await?);
            mounted.message = Some(AukiMessageEndpoint::mount(Arc::clone(&peer)).await?);
            mounted.stream = Some(AukiStreamEndpoint::mount(Arc::clone(&peer)).await?);
            Ok(())
        }
        .await;

        if let Err(error) = mounting {
            return match mounted.close_ordered().await {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(operation_error(
                    "mount standard protocols",
                    format!("{error}; ordered rollback also failed: {cleanup_error}"),
                )),
            };
        }

        Ok(Arc::new(Self {
            peer,
            endpoints: mounted.complete(),
            owner: StandardProtocolsOwner::new(),
        }))
    }

    pub fn info(&self) -> Arc<AukiInfoEndpoint> {
        Arc::clone(&self.endpoints.info)
    }

    pub fn catalog(&self) -> Arc<AukiCatalogEndpoint> {
        Arc::clone(&self.endpoints.catalog)
    }

    pub fn registry(&self) -> Arc<AukiRegistryEndpoint> {
        Arc::clone(&self.endpoints.registry)
    }

    pub fn blob(&self) -> Arc<AukiBlobEndpoint> {
        Arc::clone(&self.endpoints.blob)
    }

    pub fn message(&self) -> Arc<AukiMessageEndpoint> {
        Arc::clone(&self.endpoints.message)
    }

    pub fn stream(&self) -> Arc<AukiStreamEndpoint> {
        Arc::clone(&self.endpoints.stream)
    }

    /// Every protocol ID mounted by this bundle, including both Catalog IDs.
    pub fn protocols(&self) -> Vec<String> {
        STANDARD_PROTOCOL_IDS
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// Build the generic relay-backed card for exactly the mounted protocols.
    pub fn card(&self) -> Result<AukiPeerCard, AukiSdkError> {
        self.owner.ensure_open()?;
        self.peer.card(self.protocols())
    }

    /// Fence card creation, then close in dependency-safe reverse mount order.
    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.owner.begin_close(&self.endpoints))
            .await
            .map_err(|error| operation_error("close standard protocols", error))
    }
}

impl Drop for AukiStandardProtocols {
    fn drop(&mut self) {
        let _ = self.owner.begin_close(&self.endpoints);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn standard_protocol_ids_are_complete_ordered_and_unique() {
        assert_eq!(
            STANDARD_PROTOCOL_IDS,
            [
                "/auki/auth/1/info/1.0.0",
                "/auki/auth/1/resources/0.3.0",
                "/auki/auth/1/resources/0.4.0",
                "/auki/auth/1/registries/0.3.0",
                "/auki/auth/1/blobs/0.1.0",
                "/auki/auth/1/message/0.1.0",
                "/auki/auth/1/stream/0.2.0",
            ]
        );
        assert_eq!(
            STANDARD_PROTOCOL_IDS
                .into_iter()
                .collect::<BTreeSet<_>>()
                .len(),
            STANDARD_PROTOCOL_IDS.len()
        );
    }

    #[test]
    fn standard_close_order_is_reverse_mount_order() {
        assert_eq!(
            STANDARD_CLOSE_ORDER,
            [
                StandardEndpointKind::Stream,
                StandardEndpointKind::Message,
                StandardEndpointKind::Blob,
                StandardEndpointKind::Registry,
                StandardEndpointKind::Catalog,
                StandardEndpointKind::Info,
            ]
        );
    }

    #[test]
    fn ordered_close_keeps_the_first_failure() {
        let mut first = None;
        record_close_error(
            &mut first,
            StandardEndpointKind::Stream,
            Err(operation_error("test Stream", "first")),
        );
        record_close_error(
            &mut first,
            StandardEndpointKind::Message,
            Err(operation_error("test Message", "second")),
        );
        assert_eq!(
            first.as_deref(),
            Some("close Stream endpoint: test Stream: first")
        );
    }
}
