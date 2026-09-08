//! Swift adapter for finite Catalog v3 resources and v4 maps.

use std::sync::Arc;

use auki_protocols::catalog::{CatalogClient, CatalogEndpoint, CatalogProvider, v3, v4};
use auki_sdk_rs::{AuthenticatedPeer, PeerId};
use parking_lot::RwLock;

use crate::{AukiPeer, AukiPeerTarget, AukiSdkError, operation_error, wait_cleanup};

use super::finite_support::{
    CloseFuture, EndpointOwner, bounded_json, exact_target, parse_bounded_json,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiCatalogResourceVariant {
    SensorLog,
    PoseLog,
    TimeTransformLog,
    DetectionLog,
    MessageChannel,
}

impl From<AukiCatalogResourceVariant> for v3::ResourceVariant {
    fn from(value: AukiCatalogResourceVariant) -> Self {
        match value {
            AukiCatalogResourceVariant::SensorLog => Self::SensorLog,
            AukiCatalogResourceVariant::PoseLog => Self::PoseLog,
            AukiCatalogResourceVariant::TimeTransformLog => Self::TimeTransformLog,
            AukiCatalogResourceVariant::DetectionLog => Self::DetectionLog,
            AukiCatalogResourceVariant::MessageChannel => Self::MessageChannel,
        }
    }
}

#[derive(Clone)]
struct CatalogSnapshots {
    resources: Arc<RwLock<v3::ResourcesResponse>>,
    maps: Arc<RwLock<v4::ResourcesResponse>>,
}

impl Default for CatalogSnapshots {
    fn default() -> Self {
        Self {
            resources: Arc::new(RwLock::new(v3::ResourcesResponse {
                resources: Vec::new(),
            })),
            maps: Arc::new(RwLock::new(v4::ResourcesResponse {
                resources: Vec::new(),
            })),
        }
    }
}

impl CatalogProvider for CatalogSnapshots {
    fn resources(
        &self,
        _requester: &AuthenticatedPeer,
        _request: &v3::ResourcesRequest,
    ) -> v3::ResourcesResponse {
        self.resources.read().clone()
    }

    fn maps(&self, _requester: &AuthenticatedPeer) -> v4::ResourcesResponse {
        self.maps.read().clone()
    }
}

fn validate_resource_owners(
    response: &v3::ResourcesResponse,
    expected_peer_id: PeerId,
) -> Result<(), AukiSdkError> {
    for resource in &response.resources {
        if let v3::ResourceEntry::MessageChannel(channel) = resource
            && channel.owner_peer_id != expected_peer_id
        {
            return Err(operation_error(
                "validate Catalog resources snapshot",
                format!(
                    "message-channel owner {} does not match local peer {expected_peer_id}",
                    channel.owner_peer_id
                ),
            ));
        }
    }
    Ok(())
}

fn read_resources(
    json: &str,
    expected_peer_id: PeerId,
) -> Result<v3::ResourcesResponse, AukiSdkError> {
    let response: v3::ResourcesResponse = parse_bounded_json(
        "read Catalog resources JSON",
        json,
        v3::MAX_RESOURCES_FRAME_BYTES as usize,
    )?;
    response
        .validate()
        .map_err(|error| operation_error("validate Catalog resources JSON", error))?;
    validate_resource_owners(&response, expected_peer_id)?;
    // Re-encoding here catches an input whose escaped representation exceeds
    // the actual protocol frame even if its source string happened to fit.
    write_resources(&response)?;
    Ok(response)
}

fn write_resources(response: &v3::ResourcesResponse) -> Result<String, AukiSdkError> {
    bounded_json(
        "write Catalog resources JSON",
        response,
        v3::MAX_RESOURCES_FRAME_BYTES as usize,
    )
}

fn read_maps(json: &str) -> Result<v4::ResourcesResponse, AukiSdkError> {
    let response: v4::ResourcesResponse = parse_bounded_json(
        "read Catalog maps JSON",
        json,
        v4::MAX_RESOURCES_FRAME_BYTES as usize,
    )?;
    response
        .validate()
        .map_err(|error| operation_error("validate Catalog maps JSON", error))?;
    write_maps(&response)?;
    Ok(response)
}

fn write_maps(response: &v4::ResourcesResponse) -> Result<String, AukiSdkError> {
    bounded_json(
        "write Catalog maps JSON",
        response,
        v4::MAX_RESOURCES_FRAME_BYTES as usize,
    )
}

fn close_endpoint(endpoint: CatalogEndpoint) -> CloseFuture {
    Box::pin(async move { endpoint.close().await.map_err(|error| error.to_string()) })
}

#[derive(uniffi::Object)]
pub struct AukiCatalogClient {
    inner: CatalogClient,
    domain_id: String,
}

impl AukiCatalogClient {
    fn from_inner(inner: CatalogClient, domain_id: String) -> Arc<Self> {
        Arc::new(Self { inner, domain_id })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiCatalogClient {
    #[uniffi::constructor]
    pub fn new(peer: Arc<AukiPeer>) -> Arc<Self> {
        Self::from_inner(CatalogClient::new(peer.rust_protocols()), peer.domain_id())
    }

    pub fn resources_protocol(&self) -> String {
        v3::ID.into()
    }

    pub fn maps_protocol(&self) -> String {
        v4::ID.into()
    }

    /// Fetch a filtered v3 snapshot as Rust-validated compact JSON.
    pub async fn fetch_resources_exact(
        &self,
        target: AukiPeerTarget,
        variants: Vec<AukiCatalogResourceVariant>,
    ) -> Result<String, AukiSdkError> {
        let (peer_id, route) = exact_target(&self.domain_id, target)?;
        let request = v3::ResourcesRequest {
            variants: variants.into_iter().map(Into::into).collect(),
        };
        request
            .validate()
            .map_err(|error| operation_error("validate Catalog resource filter", error))?;
        let response = self
            .inner
            .fetch_resources_exact(peer_id, route, request)
            .await
            .map_err(|error| operation_error("fetch Catalog resources", error))?;
        write_resources(&response)
    }

    /// Fetch the complete v4 Map Log snapshot as Rust-validated compact JSON.
    pub async fn fetch_maps_exact(&self, target: AukiPeerTarget) -> Result<String, AukiSdkError> {
        let (peer_id, route) = exact_target(&self.domain_id, target)?;
        let response = self
            .inner
            .fetch_maps_exact(peer_id, route)
            .await
            .map_err(|error| operation_error("fetch Catalog maps", error))?;
        write_maps(&response)
    }
}

/// Both Catalog versions backed by atomically replaceable Rust snapshots.
#[derive(uniffi::Object)]
pub struct AukiCatalogEndpoint {
    owner: EndpointOwner<CatalogEndpoint>,
    snapshots: CatalogSnapshots,
    client: Arc<AukiCatalogClient>,
    peer_id: PeerId,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiCatalogEndpoint {
    #[uniffi::constructor]
    pub async fn mount(peer: Arc<AukiPeer>) -> Result<Arc<Self>, AukiSdkError> {
        let snapshots = CatalogSnapshots::default();
        let endpoint = CatalogEndpoint::mount(peer.rust_protocols(), snapshots.clone())
            .map_err(|error| operation_error("mount Catalog endpoint", error))?;
        let client = AukiCatalogClient::from_inner(endpoint.client(), peer.domain_id());
        let peer_id = peer
            .peer_id()
            .parse()
            .map_err(|error| operation_error("parse local Catalog Peer ID", error))?;
        Ok(Arc::new(Self {
            owner: EndpointOwner::new(endpoint, close_endpoint),
            snapshots,
            client,
            peer_id,
        }))
    }

    pub fn resources_protocol(&self) -> String {
        v3::ID.into()
    }

    pub fn maps_protocol(&self) -> String {
        v4::ID.into()
    }

    pub fn client(&self) -> Arc<AukiCatalogClient> {
        Arc::clone(&self.client)
    }

    pub fn replace_resources_json(&self, json: String) -> Result<(), AukiSdkError> {
        self.owner
            .ensure_open("replace Catalog resources snapshot")?;
        let response = read_resources(&json, self.peer_id)?;
        *self.snapshots.resources.write() = response;
        Ok(())
    }

    pub fn replace_maps_json(&self, json: String) -> Result<(), AukiSdkError> {
        self.owner.ensure_open("replace Catalog maps snapshot")?;
        let response = read_maps(&json)?;
        *self.snapshots.maps.write() = response;
        Ok(())
    }

    pub fn resources_json(&self) -> Result<String, AukiSdkError> {
        write_resources(&self.snapshots.resources.read())
    }

    pub fn maps_json(&self) -> Result<String, AukiSdkError> {
        write_maps(&self.snapshots.maps.read())
    }

    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close Catalog endpoint", error))
    }
}

#[cfg(test)]
mod tests {
    use auki_registry::RegistryRef;
    use auki_sdk_rs::Identity;

    use super::*;

    #[test]
    fn catalog_snapshots_are_strict_validated_and_owner_bound() {
        let peer_id = Identity::generate().peer_id();
        let response = v3::ResourcesResponse {
            resources: vec![v3::ResourceEntry::MessageChannel(
                v3::MessageChannelResource {
                    owner_peer_id: peer_id,
                    resource_id: "playground/events".into(),
                    clock: RegistryRef {
                        peer_id: peer_id.to_string(),
                        id: "clock".into(),
                        hash: "hash".into(),
                    },
                },
            )],
        };
        let json = write_resources(&response).unwrap();
        assert_eq!(read_resources(&json, peer_id).unwrap(), response);

        let other = Identity::generate().peer_id();
        assert!(read_resources(&json, other).is_err());
        assert!(read_maps(r#"{"resources":[{"resource_id":"missing fields"}]}"#).is_err());
    }

    #[test]
    fn duplicate_resource_filters_are_rejected_before_transport() {
        let request = v3::ResourcesRequest {
            variants: vec![
                AukiCatalogResourceVariant::SensorLog.into(),
                AukiCatalogResourceVariant::SensorLog.into(),
            ],
        };
        assert!(request.validate().is_err());
    }
}
