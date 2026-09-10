//! Cross-platform Auki peer endpoint for Resource Catalog v3 and v4.
//!
//! Catalog v3 advertises sensor, pose, time-transform, detection, and live
//! message-channel resources. Catalog v4 advertises Map Logs. Version 0.2
//! remains a wire-only codec because v3 embeds its established log-row shape.

#![forbid(unsafe_code)]

use std::{fmt, future::Future, time::Duration};

use auki_sdk::{
    AukiPeerProtocols, AukiProtocolError, AukiProtocolRegistration, AukiProtocolSpec,
    AukiProtocolStream, AuthenticatedPeer, AuthenticatedRouteStream, Multiaddr, PeerId,
};
use futures::AsyncWriteExt;

use crate::endpoint_support::{Shared, clone_shared, deadline_after, prefer_primary, share};

use super::{v3, v4};

/// Maximum number of concurrently served streams for each Catalog version.
pub const CATALOG_MAX_CONCURRENCY: usize = 16;

/// Fixed deadline for opening, exchanging, or closing one Catalog stream.
pub const CATALOG_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

type ProviderHandle = Shared<dyn CatalogProvider>;

/// Application-owned source of currently available resources.
///
/// Both methods are sampled once per request and receive the mutually
/// authenticated requester. Their defaults advertise an empty catalog, so an
/// application opts into only the resource families it actually serves.
#[cfg(not(target_arch = "wasm32"))]
pub trait CatalogProvider: Send + Sync + 'static {
    /// Snapshot Catalog v3 resources visible to this authenticated requester.
    fn resources(
        &self,
        _requester: &AuthenticatedPeer,
        _request: &v3::ResourcesRequest,
    ) -> v3::ResourcesResponse {
        empty_resources()
    }

    /// Snapshot Catalog v4 Map Logs visible to this authenticated requester.
    fn maps(&self, _requester: &AuthenticatedPeer) -> v4::ResourcesResponse {
        empty_maps()
    }
}

/// Application-owned source of currently available resources.
///
/// Browser providers are local to the Wasm executor. Both methods are sampled
/// once per request and default to an empty catalog.
#[cfg(target_arch = "wasm32")]
pub trait CatalogProvider: 'static {
    /// Snapshot Catalog v3 resources visible to this authenticated requester.
    fn resources(
        &self,
        _requester: &AuthenticatedPeer,
        _request: &v3::ResourcesRequest,
    ) -> v3::ResourcesResponse {
        empty_resources()
    }

    /// Snapshot Catalog v4 Map Logs visible to this authenticated requester.
    fn maps(&self, _requester: &AuthenticatedPeer) -> v4::ResourcesResponse {
        empty_maps()
    }
}

impl CatalogProvider for () {}

/// Build the exact bounded Catalog v3 registration.
pub fn resources_protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(
        v3::ID,
        CATALOG_MAX_CONCURRENCY,
        v3::MAX_RESOURCES_FRAME_BYTES,
    )
}

/// Build the exact bounded Catalog v4 Map Log registration.
pub fn maps_protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(
        v4::ID,
        CATALOG_MAX_CONCURRENCY,
        v4::MAX_RESOURCES_FRAME_BYTES,
    )
}

/// Cloneable outbound client for Catalog v3 resources and v4 maps.
#[derive(Clone)]
pub struct CatalogClient {
    protocols: AukiPeerProtocols,
}

impl CatalogClient {
    /// Construct a client over one running peer's protocol surface.
    pub fn new(protocols: AukiPeerProtocols) -> Self {
        Self { protocols }
    }

    /// Fetch Catalog v3 resources using routes configured on the native peer.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn fetch_resources(
        &self,
        remote_peer_id: PeerId,
        request: v3::ResourcesRequest,
    ) -> Result<v3::ResourcesResponse, CatalogEndpointError> {
        request.validate()?;
        fetch_resources_opened(
            remote_peer_id,
            request,
            self.protocols.open(remote_peer_id, v3::ID),
        )
        .await
    }

    /// Fetch Catalog v3 resources through one exact advertised route.
    pub async fn fetch_resources_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        request: v3::ResourcesRequest,
    ) -> Result<v3::ResourcesResponse, CatalogEndpointError> {
        request.validate()?;
        fetch_resources_opened(
            remote_peer_id,
            request,
            self.protocols.open_exact(remote_peer_id, route, v3::ID),
        )
        .await
    }

    /// Fetch every Catalog v3 resource using configured native routes.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn fetch_all_resources(
        &self,
        remote_peer_id: PeerId,
    ) -> Result<v3::ResourcesResponse, CatalogEndpointError> {
        self.fetch_resources(remote_peer_id, v3::ResourcesRequest::all())
            .await
    }

    /// Fetch every Catalog v3 resource through one exact advertised route.
    pub async fn fetch_all_resources_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
    ) -> Result<v3::ResourcesResponse, CatalogEndpointError> {
        self.fetch_resources_exact(remote_peer_id, route, v3::ResourcesRequest::all())
            .await
    }

    /// Fetch Catalog v4 Map Logs using routes configured on the native peer.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn fetch_maps(
        &self,
        remote_peer_id: PeerId,
    ) -> Result<v4::ResourcesResponse, CatalogEndpointError> {
        fetch_maps_opened(self.protocols.open(remote_peer_id, v4::ID)).await
    }

    /// Fetch Catalog v4 Map Logs through one exact advertised route.
    pub async fn fetch_maps_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
    ) -> Result<v4::ResourcesResponse, CatalogEndpointError> {
        fetch_maps_opened(self.protocols.open_exact(remote_peer_id, route, v4::ID)).await
    }
}

/// Mounted Catalog v3 and v4 services plus their outbound client.
pub struct CatalogEndpoint {
    client: CatalogClient,
    resources_registration: AukiProtocolRegistration,
    maps_registration: AukiProtocolRegistration,
}

impl CatalogEndpoint {
    /// Mount Catalog v3 resources and v4 maps on one running Auki peer.
    pub fn mount<P>(protocols: AukiPeerProtocols, provider: P) -> Result<Self, CatalogEndpointError>
    where
        P: CatalogProvider,
    {
        let local_peer_id = protocols.peer_id();
        let provider: ProviderHandle = share(provider);

        let resources_provider = clone_shared(&provider);
        let resources_registration =
            protocols.register(resources_protocol_spec()?, move |mut stream| {
                let provider = clone_shared(&resources_provider);
                async move {
                    let _ = serve_resources(&mut stream, provider.as_ref(), local_peer_id).await;
                }
            })?;

        let maps_registration = protocols.register(maps_protocol_spec()?, move |mut stream| {
            let provider = clone_shared(&provider);
            async move {
                let _ = serve_maps(&mut stream, provider.as_ref()).await;
            }
        })?;

        Ok(Self {
            client: CatalogClient::new(protocols),
            resources_registration,
            maps_registration,
        })
    }

    /// Clone the outbound client without cloning registration ownership.
    pub fn client(&self) -> CatalogClient {
        self.client.clone()
    }

    /// Stop accepting Catalog streams and await handlers admitted by both versions.
    pub async fn close(self) -> Result<(), CatalogEndpointError> {
        let resources = self
            .resources_registration
            .close()
            .await
            .map_err(CatalogEndpointError::Sdk);
        let maps = self
            .maps_registration
            .close()
            .await
            .map_err(CatalogEndpointError::Sdk);
        prefer_primary(resources, maps)
    }
}

async fn fetch_resources_opened<F>(
    remote_peer_id: PeerId,
    request: v3::ResourcesRequest,
    opening: F,
) -> Result<v3::ResourcesResponse, CatalogEndpointError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
{
    let mut stream = deadline(CatalogOperation::Open, opening)
        .await?
        .map_err(CatalogEndpointError::Sdk)?;
    let exchange = deadline(CatalogOperation::Exchange, async {
        v3::write_resources_request(&mut stream, &request).await?;
        let response = v3::read_resources_response(&mut stream).await?;
        validate_received_resources(remote_peer_id, &request, &response)?;
        Ok::<_, CatalogEndpointError>(response)
    })
    .await
    .and_then(|result| result);
    let cleanup = close_stream(&mut stream).await;
    prefer_primary(exchange, cleanup)
}

async fn fetch_maps_opened<F>(opening: F) -> Result<v4::ResourcesResponse, CatalogEndpointError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
{
    let mut stream = deadline(CatalogOperation::Open, opening)
        .await?
        .map_err(CatalogEndpointError::Sdk)?;
    let exchange = deadline(CatalogOperation::Exchange, async {
        v4::write_resources_request(&mut stream, &v4::ResourcesRequest::all()).await?;
        let response = v4::read_resources_response(&mut stream).await?;
        validate_maps_response(&response)?;
        Ok::<_, CatalogEndpointError>(response)
    })
    .await
    .and_then(|result| result);
    let cleanup = close_stream(&mut stream).await;
    prefer_primary(exchange, cleanup)
}

async fn serve_resources<P>(
    stream: &mut AukiProtocolStream,
    provider: &P,
    local_peer_id: PeerId,
) -> Result<(), CatalogEndpointError>
where
    P: CatalogProvider + ?Sized,
{
    let requester = stream.remote_peer().clone();
    let exchange = deadline(CatalogOperation::Exchange, async {
        let request = v3::read_resources_request(stream).await?;
        let response = provider.resources(&requester, &request);
        let response = prepare_resources_response(local_peer_id, &request, response)?;
        v3::write_resources_response(stream, &response).await?;
        Ok::<_, CatalogEndpointError>(())
    })
    .await
    .and_then(|result| result);
    let cleanup = close_stream(stream).await;
    prefer_primary(exchange, cleanup)
}

async fn serve_maps<P>(
    stream: &mut AukiProtocolStream,
    provider: &P,
) -> Result<(), CatalogEndpointError>
where
    P: CatalogProvider + ?Sized,
{
    let requester = stream.remote_peer().clone();
    let exchange = deadline(CatalogOperation::Exchange, async {
        v4::read_resources_request(stream).await?;
        let response = provider.maps(&requester);
        validate_maps_response(&response)?;
        v4::write_resources_response(stream, &response).await?;
        Ok::<_, CatalogEndpointError>(())
    })
    .await
    .and_then(|result| result);
    let cleanup = close_stream(stream).await;
    prefer_primary(exchange, cleanup)
}

fn prepare_resources_response(
    local_peer_id: PeerId,
    request: &v3::ResourcesRequest,
    response: v3::ResourcesResponse,
) -> Result<v3::ResourcesResponse, CatalogEndpointError> {
    let response = response.filtered(request);
    response.validate()?;
    validate_message_channel_owners(local_peer_id, &response)?;
    Ok(response)
}

fn validate_received_resources(
    remote_peer_id: PeerId,
    request: &v3::ResourcesRequest,
    response: &v3::ResourcesResponse,
) -> Result<(), CatalogEndpointError> {
    response.validate()?;
    validate_message_channel_owners(remote_peer_id, response)?;
    if let Some(resource) = response
        .resources
        .iter()
        .find(|resource| !request.includes(resource.variant()))
    {
        return Err(CatalogEndpointError::UnexpectedResourceVariant {
            variant: resource.variant(),
        });
    }
    Ok(())
}

fn validate_message_channel_owners(
    expected_peer_id: PeerId,
    response: &v3::ResourcesResponse,
) -> Result<(), CatalogEndpointError> {
    for resource in &response.resources {
        if let v3::ResourceEntry::MessageChannel(channel) = resource
            && channel.owner_peer_id != expected_peer_id
        {
            return Err(CatalogEndpointError::MessageChannelOwnerMismatch {
                expected: Box::new(expected_peer_id),
                actual: Box::new(channel.owner_peer_id),
            });
        }
    }
    Ok(())
}

fn validate_maps_response(response: &v4::ResourcesResponse) -> Result<(), CatalogEndpointError> {
    response.validate().map_err(CatalogEndpointError::MapsCodec)
}

fn empty_resources() -> v3::ResourcesResponse {
    v3::ResourcesResponse {
        resources: Vec::new(),
    }
}

fn empty_maps() -> v4::ResourcesResponse {
    v4::ResourcesResponse {
        resources: Vec::new(),
    }
}

async fn close_stream<S>(stream: &mut S) -> Result<(), CatalogEndpointError>
where
    S: AsyncWriteExt + Unpin,
{
    deadline(CatalogOperation::Close, stream.close())
        .await?
        .map_err(|error| CatalogEndpointError::Close(error.to_string()))
}

async fn deadline<T>(
    operation: CatalogOperation,
    future: impl Future<Output = T>,
) -> Result<T, CatalogEndpointError> {
    deadline_after(CATALOG_OPERATION_TIMEOUT, future, || {
        CatalogEndpointError::Timeout(operation)
    })
    .await
}

/// One bounded Catalog endpoint operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogOperation {
    /// Open and authenticate a Catalog stream.
    Open,
    /// Exchange one bounded request and response.
    Exchange,
    /// Close one authenticated Catalog stream.
    Close,
}

impl fmt::Display for CatalogOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Open => "open",
            Self::Exchange => "exchange",
            Self::Close => "close",
        })
    }
}

/// Failure from the portable Catalog endpoint.
#[derive(Debug, thiserror::Error)]
pub enum CatalogEndpointError {
    /// The SDK rejected protocol registration or stream opening.
    #[error("Auki protocol operation failed: {0}")]
    Sdk(#[from] AukiProtocolError),
    /// Catalog v3 framing or validation failed.
    #[error("Catalog v3 protocol failed: {0}")]
    ResourcesCodec(#[from] v3::ResourcesProtocolError),
    /// Catalog v4 framing or validation failed.
    #[error("Catalog v4 protocol failed: {0}")]
    MapsCodec(#[from] v4::ResourcesProtocolError),
    /// A message channel was not owned by the authenticated serving peer.
    #[error("message-channel owner {actual} does not match authenticated peer {expected}")]
    MessageChannelOwnerMismatch {
        /// Expected local or remote authenticated peer.
        expected: Box<PeerId>,
        /// Owner repeated by the catalog row.
        actual: Box<PeerId>,
    },
    /// A peer returned a resource excluded by the request filter.
    #[error("Catalog peer returned excluded resource variant {variant:?}")]
    UnexpectedResourceVariant {
        /// Unexpected row variant.
        variant: v3::ResourceVariant,
    },
    /// One endpoint phase exceeded its fixed deadline.
    #[error("Catalog {0} timed out after 5 seconds")]
    Timeout(CatalogOperation),
    /// Stream cleanup failed after the exchange.
    #[error("close Catalog stream: {0}")]
    Close(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_registry::RegistryRef;
    use auki_sdk::Identity;

    fn channel(owner_peer_id: PeerId, resource_id: &str) -> v3::ResourceEntry {
        v3::ResourceEntry::MessageChannel(v3::MessageChannelResource {
            owner_peer_id,
            resource_id: resource_id.into(),
            clock: RegistryRef {
                peer_id: owner_peer_id.to_string(),
                id: "session/monotonic".into(),
                hash: "clock-hash".into(),
            },
        })
    }

    #[test]
    fn specs_mount_only_catalog_v3_and_v4() {
        let resources = resources_protocol_spec().unwrap();
        assert_eq!(resources.protocol_id(), v3::ID);
        assert_eq!(resources.max_concurrency(), CATALOG_MAX_CONCURRENCY);
        assert_eq!(resources.max_frame_bytes(), v3::MAX_RESOURCES_FRAME_BYTES);

        let maps = maps_protocol_spec().unwrap();
        assert_eq!(maps.protocol_id(), v4::ID);
        assert_eq!(maps.max_concurrency(), CATALOG_MAX_CONCURRENCY);
        assert_eq!(maps.max_frame_bytes(), v4::MAX_RESOURCES_FRAME_BYTES);
    }

    #[test]
    fn default_catalog_snapshots_are_empty() {
        assert!(empty_resources().resources.is_empty());
        assert!(empty_maps().resources.is_empty());
    }

    #[test]
    fn server_filters_rows_and_binds_message_channels_to_its_peer() {
        let local_peer = Identity::generate().peer_id();
        let response = v3::ResourcesResponse {
            resources: vec![channel(local_peer, "commands")],
        };

        let excluded = prepare_resources_response(
            local_peer,
            &v3::ResourcesRequest {
                variants: vec![v3::ResourceVariant::SensorLog],
            },
            response.clone(),
        )
        .unwrap();
        assert!(excluded.resources.is_empty());

        let included =
            prepare_resources_response(local_peer, &v3::ResourcesRequest::all(), response).unwrap();
        assert_eq!(included.resources, vec![channel(local_peer, "commands")]);

        let other_peer = Identity::generate().peer_id();
        assert!(matches!(
            prepare_resources_response(
                local_peer,
                &v3::ResourcesRequest::all(),
                v3::ResourcesResponse {
                    resources: vec![channel(other_peer, "commands")],
                },
            ),
            Err(CatalogEndpointError::MessageChannelOwnerMismatch { .. })
        ));
    }

    #[test]
    fn client_rejects_wrong_owner_and_excluded_variants() {
        let remote_peer = Identity::generate().peer_id();
        let other_peer = Identity::generate().peer_id();
        let all = v3::ResourcesRequest::all();

        assert!(matches!(
            validate_received_resources(
                remote_peer,
                &all,
                &v3::ResourcesResponse {
                    resources: vec![channel(other_peer, "events")],
                },
            ),
            Err(CatalogEndpointError::MessageChannelOwnerMismatch { .. })
        ));

        assert!(matches!(
            validate_received_resources(
                remote_peer,
                &v3::ResourcesRequest {
                    variants: vec![v3::ResourceVariant::SensorLog],
                },
                &v3::ResourcesResponse {
                    resources: vec![channel(remote_peer, "events")],
                },
            ),
            Err(CatalogEndpointError::UnexpectedResourceVariant {
                variant: v3::ResourceVariant::MessageChannel
            })
        ));
    }

    #[test]
    fn map_catalog_keeps_empty_valid_and_rejects_empty_identity_fields() {
        assert!(validate_maps_response(&empty_maps()).is_ok());

        let invalid = v4::ResourcesResponse {
            resources: vec![v4::MapLogResource {
                source_peer_id: String::new(),
                writer_peer_id: "writer".into(),
                resource_id: "occupancy".into(),
                map: RegistryRef {
                    peer_id: "writer".into(),
                    id: "occupancy".into(),
                    hash: "map-hash".into(),
                },
                clock: RegistryRef {
                    peer_id: "writer".into(),
                    id: "clock".into(),
                    hash: "clock-hash".into(),
                },
            }],
        };
        assert!(matches!(
            validate_maps_response(&invalid),
            Err(CatalogEndpointError::MapsCodec(
                v4::ResourcesProtocolError::Validation(_)
            ))
        ));
    }
}
