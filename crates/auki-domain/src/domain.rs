//! Public authenticated Domain lifecycle and retained product operations.
//!
//! One [`Domain`] owns one `auki-p2p` node for one DDS Domain UUID. Hosts
//! supply identity, trust material, listeners, and explicit routes; this crate
//! performs no Discovery, DDS, DMS, election, or membership work.

use std::{path::PathBuf, sync::Arc};

use auki_p2p::{DdsVerificationKeys, Identity, Multiaddr, PeerId, SignedP2pCredential};
use auki_protocols::{
    catalog::{
        v2::{
            Available, DetectionManifestPointer, Head, PoseBlock, PoseManifestPointer,
            ResourceEntry, ResourcesRequest, ResourcesResponse, SensorBlock, SensorKind,
            SensorManifestPointer, TimeTransformManifestPointer, VariantContent,
        },
        v3::{
            MessageChannelResource, ResourcesRequest as ResourcesRequestV3,
            ResourcesResponse as ResourcesResponseV3,
        },
        v4::{MapLogResource, ResourcesResponse as ResourcesResponseV4},
    },
    info::v1::AuthenticatedParticipantInfo,
    registry::v3::{RegistryKind, RegistryListEntry},
    stream::v2::{ReadFrom, StreamManifest, StreamRequest, map::MapUpdate},
};
use auki_registry::{
    ClockRegistryEntry, DetectorRegistryEntry, DeviceModelRegistryEntry, FrameRegistryEntry,
    MapRegistryEntry, SensorBody, SensorRegistryEntry,
};
use auki_session::{
    DetectionLogHandle, HeadSpec, Peer, PeerRegistries, PoseLogHandle, SensorLogHandle, Session,
    SessionLogs, TimeTransformLogHandle,
};
use futures::{StreamExt, stream};
use prost::Message;
use tokio::sync::watch;
use uuid::Uuid;

use crate::{
    authenticated_runtime::{
        AuthenticatedDomain, AuthenticatedDomainConfig, AuthenticatedDomainError,
        AuthenticatedDomainServicesConfig,
        authority::DomainAuthority,
        blobs::BlobsV1Error,
        info_v1::{InfoV1Error, ParticipantInfoProvider},
        messages::{MessageChannelRegistration, OpenMessageChannelError, SendMessageError},
        peers::DomainPeers,
        protocols::DomainProtocols,
        registries::RegistriesError,
        resources_v2::ResourcesV2Error,
        resources_v3::ResourcesV3Error,
        resources_v4::ResourcesV4Error,
        routes::DomainRoutes,
        status::DomainStatus,
        streams::StreamsError,
    },
    resource_catalog::{MapCatalogProvider, ResourceCatalogProvider},
    served_protocols::ServedProtocols,
    stream_runtime::{
        SourceStream, StreamDispatch, StreamItem, StreamProvider, StreamSubscription,
        decline_all_streams,
    },
};

/// Inputs needed to create one Domain-owned authenticated P2P node.
#[derive(Clone)]
pub struct DomainConfig {
    inner: AuthenticatedDomainConfig,
}

impl DomainConfig {
    /// Start a configuration for one exact DDS Domain UUID and stable identity.
    pub fn new(domain_id: Uuid, identity: Identity) -> Self {
        Self {
            inner: AuthenticatedDomainConfig::new(domain_id, identity),
        }
    }

    /// Replace the local listener set. Zero listeners is valid.
    pub fn with_listen_addresses(
        mut self,
        addresses: impl IntoIterator<Item = Multiaddr>,
    ) -> Result<Self, DomainError> {
        self.inner = self.inner.with_listen_addresses(addresses)?;
        Ok(self)
    }

    /// Replace the initial route candidates for one expected peer.
    pub fn with_peer_routes(
        mut self,
        expected_peer: PeerId,
        candidates: impl IntoIterator<Item = Multiaddr>,
    ) -> Result<Self, DomainError> {
        self.inner = self.inner.with_peer_routes(expected_peer, candidates)?;
        Ok(self)
    }

    /// Exact DDS Domain UUID selected by this configuration.
    pub fn domain_id(&self) -> Uuid {
        self.inner.domain_id()
    }

    /// Peer ID derived from the configured stable identity.
    pub fn peer_id(&self) -> PeerId {
        self.inner.peer_id()
    }
}

/// Builder for protocol-specific inputs and initial Domain authority.
pub struct DomainBuilder<'a> {
    peer: &'a Peer,
    session: &'a Session,
    config: DomainConfig,
    authority: Option<(DdsVerificationKeys, SignedP2pCredential)>,
    participant_info: Option<Arc<dyn ParticipantInfoProvider>>,
    resource_catalog_provider: Option<Arc<dyn ResourceCatalogProvider>>,
    map_catalog_provider: Option<Arc<dyn MapCatalogProvider>>,
    registry_app_root: Option<PathBuf>,
    message_channels: Vec<(MessageChannelResource, usize)>,
    stream_provider: StreamProvider,
    served_protocols: ServedProtocols,
}

impl<'a> DomainBuilder<'a> {
    /// Start composing a Domain join.
    pub fn new(peer: &'a Peer, session: &'a Session, config: DomainConfig) -> Self {
        Self {
            peer,
            session,
            config,
            authority: None,
            participant_info: None,
            resource_catalog_provider: None,
            map_catalog_provider: None,
            registry_app_root: None,
            message_channels: Vec::new(),
            stream_provider: decline_all_streams(),
            served_protocols: ServedProtocols::none(),
        }
    }

    /// Install the host-fetched initial verification keys and signed token.
    pub fn authority(
        mut self,
        verification_keys: DdsVerificationKeys,
        credential: SignedP2pCredential,
    ) -> Self {
        self.authority = Some((verification_keys, credential));
        self
    }

    /// Select the exact application protocol versions this Domain will serve.
    ///
    /// Client operations remain available regardless of this selection. When
    /// omitted, the Domain serves no application protocols.
    pub fn served_protocols(mut self, protocols: ServedProtocols) -> Self {
        self.served_protocols = protocols;
        self
    }

    /// Install a sampled provider for bounded diagnostic participant info.
    pub fn participant_info_provider(mut self, provider: Arc<dyn ParticipantInfoProvider>) -> Self {
        self.participant_info = Some(provider);
        self
    }

    /// Override the Session-backed Resource Catalog provider.
    pub fn resource_catalog_provider(mut self, provider: Arc<dyn ResourceCatalogProvider>) -> Self {
        self.resource_catalog_provider = Some(provider);
        self
    }

    /// Override the Session-backed Map Log catalog provider.
    pub fn map_catalog_provider(mut self, provider: Arc<dyn MapCatalogProvider>) -> Self {
        self.map_catalog_provider = Some(provider);
        self
    }

    /// Override the Peer storage root shared by Registry and blob serving.
    pub fn registry_app_root(mut self, app_root: impl Into<PathBuf>) -> Self {
        self.registry_app_root = Some(app_root.into());
        self
    }

    /// Install the application fallback for typed streams.
    pub fn stream_provider(mut self, provider: StreamProvider) -> Self {
        self.stream_provider = provider;
        self
    }

    /// Declare one bounded receiver-owned message channel.
    pub fn message_channel(
        mut self,
        resource: MessageChannelResource,
        receiver_capacity: usize,
    ) -> Result<Self, DomainBuilderError> {
        resource
            .validate()
            .map_err(DomainBuilderError::InvalidMessageChannel)?;
        let expected = self.config.peer_id();
        if resource.owner_peer_id != expected {
            return Err(DomainBuilderError::ChannelOwnerMismatch {
                expected: Box::new(expected),
                actual: Box::new(resource.owner_peer_id),
            });
        }
        if receiver_capacity == 0 {
            return Err(DomainBuilderError::ZeroReceiverCapacity);
        }
        if !self.session.contains_clock_ref(&resource.clock) {
            return Err(DomainBuilderError::UnregisteredChannelClock {
                peer_id: resource.clock.peer_id.clone(),
                clock_id: resource.clock.id.clone(),
                clock_hash: resource.clock.hash.clone(),
            });
        }
        if self.message_channels.iter().any(|(existing, _)| {
            existing.owner_peer_id == resource.owner_peer_id
                && existing.resource_id == resource.resource_id
        }) {
            return Err(DomainBuilderError::DuplicateMessageChannel {
                owner_peer_id: resource.owner_peer_id,
                resource_id: resource.resource_id,
            });
        }
        self.message_channels.push((resource, receiver_capacity));
        Ok(self)
    }

    /// Validate inputs, bind listeners and explicitly selected protocols, and return.
    pub async fn join(self) -> Result<Domain, DomainError> {
        let (verification_keys, credential) = self
            .authority
            .ok_or(DomainError::InitialAuthorityRequired)?;
        validate_identity_chain(self.peer, self.session, &self.config)?;

        let catalog = Arc::new(DomainCatalog {
            logs: self.session.logs(),
            registries: self.peer.registries(),
        });
        let resource_provider: Arc<dyn ResourceCatalogProvider> = self
            .resource_catalog_provider
            .unwrap_or_else(|| catalog.clone());
        let map_provider: Arc<dyn MapCatalogProvider> =
            self.map_catalog_provider.unwrap_or_else(|| catalog.clone());
        let registry_app_root = self
            .registry_app_root
            .unwrap_or_else(|| self.peer.storage_root());
        let logs = self.session.logs();
        let stream_provider = detection_stream_provider(
            map_stream_provider(self.stream_provider, logs.clone()),
            logs,
        );

        let mut services = AuthenticatedDomainServicesConfig::default()
            .with_served_protocols(self.served_protocols)
            .with_resource_catalog_provider(resource_provider)
            .with_map_catalog_provider(map_provider)
            .with_registry_app_root(registry_app_root)
            .with_stream_provider(stream_provider);
        if let Some(provider) = self.participant_info {
            services = services.with_participant_info_provider(provider);
        }
        for (resource, capacity) in self.message_channels {
            services = services.with_message_channel(resource, capacity);
        }

        let runtime = AuthenticatedDomain::join_with_services(
            self.config.inner,
            verification_keys,
            credential,
            services,
        )
        .await?;
        Ok(Domain { runtime })
    }
}

/// Invalid builder composition detected before the node starts.
#[derive(Debug, thiserror::Error)]
pub enum DomainBuilderError {
    /// The channel owner differs from the configured stable identity.
    #[error("message channel owner {actual} does not match Domain peer {expected}")]
    ChannelOwnerMismatch {
        /// Configured local Domain peer.
        expected: Box<PeerId>,
        /// Owner encoded by the channel resource.
        actual: Box<PeerId>,
    },
    /// The same receiver-owned channel was declared twice.
    #[error("duplicate message channel {owner_peer_id}/{resource_id}")]
    DuplicateMessageChannel {
        /// Receiver peer encoded by the duplicate declarations.
        owner_peer_id: PeerId,
        /// Duplicated application resource identifier.
        resource_id: String,
    },
    /// A bounded receiver cannot have zero capacity.
    #[error("message channel receiver capacity must be greater than zero")]
    ZeroReceiverCapacity,
    /// The channel clock does not identify a clock in this Session.
    #[error("message channel clock is not registered: {peer_id}/{clock_id}@{clock_hash}")]
    UnregisteredChannelClock {
        /// Peer component of the missing clock reference.
        peer_id: String,
        /// Identifier component of the missing clock reference.
        clock_id: String,
        /// Content hash component of the missing clock reference.
        clock_hash: String,
    },
    /// The v0.3 catalog row is malformed.
    #[error("invalid message channel resource: {0}")]
    InvalidMessageChannel(#[source] auki_protocols::catalog::v3::ResourcesProtocolError),
}

/// Domain join and ordered-leave failures.
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    /// The builder did not receive the initial signed authority material.
    #[error("initial DDS verification keys and signed P2P credential are required")]
    InitialAuthorityRequired,
    /// The Session belongs to a different Peer.
    #[error("session peer id {session:?} does not match Peer id {peer:?}")]
    SessionIdentityMismatch {
        /// Peer object identity.
        peer: String,
        /// Session owner identity.
        session: String,
    },
    /// The SDK Peer identity differs from the configured P2P identity.
    #[error("Peer id {peer:?} does not match P2P identity {identity}")]
    IdentityMismatch {
        /// SDK Peer identity.
        peer: String,
        /// Canonical P2P identity selected by the configuration.
        identity: PeerId,
    },
    /// The authenticated runtime failed.
    #[error(transparent)]
    Runtime(Box<AuthenticatedDomainError>),
}

impl From<AuthenticatedDomainError> for DomainError {
    fn from(error: AuthenticatedDomainError) -> Self {
        Self::Runtime(Box::new(error))
    }
}

/// One live inbound application message.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageEvent {
    /// Receiver-owned channel that accepted the message.
    pub channel: MessageChannelResource,
    /// Mutually authenticated sender.
    pub sender: PeerId,
    /// Opaque application type.
    pub r#type: String,
    /// Application timestamp in the channel's declared clock.
    pub timestamp_ns: i64,
    /// Opaque application payload.
    pub payload: Vec<u8>,
}

/// Bounded receiver for one live-only message channel.
pub struct MessageChannelReceiver {
    registration: MessageChannelRegistration,
}

impl MessageChannelReceiver {
    /// Exact resource row bound to this receiver.
    pub fn resource(&self) -> &MessageChannelResource {
        self.registration.resource()
    }

    /// Receive the next live event, or `None` after closure.
    pub async fn recv(&mut self) -> Option<MessageEvent> {
        let channel = self.registration.resource().clone();
        let inbound = self.registration.recv().await?;
        Some(MessageEvent {
            channel,
            sender: inbound.sender,
            r#type: inbound.message.r#type,
            timestamp_ns: inbound.message.timestamp_ns,
            payload: inbound.message.payload,
        })
    }
}

/// Cloneable persistent sender for one receiver-owned channel.
#[derive(Clone)]
pub struct MessageChannelSender {
    inner: crate::authenticated_runtime::messages::MessageChannelSender,
}

impl MessageChannelSender {
    /// Send once and wait for the receiver runtime's sequence ACK.
    pub async fn send(
        &self,
        r#type: impl Into<String>,
        timestamp_ns: i64,
        payload: impl Into<Vec<u8>>,
    ) -> Result<(), SendMessageError> {
        self.inner.send(r#type, timestamp_ns, payload).await
    }
}

/// Failure to open a validated Map Log stream.
#[derive(Debug, thiserror::Error)]
pub enum DomainOpenMapStreamError {
    /// The catalog row is malformed.
    #[error("invalid map resource: {0}")]
    InvalidResource(#[source] auki_protocols::catalog::v4::ResourcesProtocolError),
    /// The row's writer does not match the expected authenticated producer.
    #[error("map writer {writer_peer_id} does not match target {target}")]
    WriterMismatch {
        /// Expected mutually authenticated producer.
        target: PeerId,
        /// Writer encoded by the selected catalog row.
        writer_peer_id: String,
    },
    /// The authenticated typed-stream open failed.
    #[error(transparent)]
    Open(#[from] StreamsError),
    /// The producer returned a different manifest than the pinned catalog row.
    #[error("map stream manifest does not match the selected resource")]
    ManifestMismatch,
}

/// Failure from the one-shot message convenience operation.
#[derive(Debug, thiserror::Error)]
pub enum DomainSendMessageError {
    /// Opening the authenticated sender failed.
    #[error(transparent)]
    Open(#[from] OpenMessageChannelError),
    /// Sending or receiving the ACK failed.
    #[error(transparent)]
    Send(#[from] SendMessageError),
}

/// Network presence for one Peer and Session in one authenticated Domain.
pub struct Domain {
    runtime: AuthenticatedDomain,
}

impl Domain {
    /// Start a builder over the retained SDK Peer and Session objects.
    pub fn builder<'a>(
        peer: &'a Peer,
        session: &'a Session,
        config: DomainConfig,
    ) -> DomainBuilder<'a> {
        DomainBuilder::new(peer, session, config)
    }

    /// Exact DDS Domain UUID owned by this runtime.
    pub fn domain_id(&self) -> Uuid {
        self.runtime.domain_id()
    }

    /// Stable local P2P identity.
    pub fn peer_id(&self) -> PeerId {
        self.runtime.peer_id()
    }

    /// Addresses that successfully bound before join returned.
    pub fn listen_addresses(&self) -> &[Multiaddr] {
        self.runtime.listen_addresses()
    }

    /// Current local lifecycle/readiness snapshot.
    pub fn status(&self) -> DomainStatus {
        self.runtime.status()
    }

    /// Subscribe to lifecycle/readiness changes with snapshot recovery.
    pub fn subscribe_status(&self) -> watch::Receiver<DomainStatus> {
        self.runtime.subscribe_status()
    }

    /// Narrow host authority handle for key/token updates and challenge signing.
    pub fn authority(&self) -> DomainAuthority {
        self.runtime.authority()
    }

    /// Explicit bounded route catalog. Routes are dial hints, not authority.
    pub fn routes(&self) -> DomainRoutes {
        self.runtime.routes()
    }

    /// Authenticated, currently connected peers for this exact Domain.
    pub fn known_peers(&self) -> DomainPeers {
        self.runtime.peers()
    }

    /// Restricted authenticated protocol extension surface.
    pub fn protocols(&self) -> DomainProtocols {
        self.runtime.protocols()
    }

    /// Exact application protocol IDs currently served by this Domain.
    pub fn served_protocol_ids(&self) -> &[&'static str] {
        self.runtime.served_protocol_ids()
    }

    /// Current local Resource Catalog v0.2 rows from the active provider.
    pub fn catalog(&self) -> Result<Vec<ResourceEntry>, ResourcesV2Error> {
        self.runtime
            .resources_v2()
            .local(&ResourcesRequest::all())
            .map(|response| response.resources)
    }

    /// Fetch and cache a peer's bounded diagnostic participant info.
    pub async fn fetch_participant_info(
        &self,
        expected_peer: PeerId,
    ) -> Result<AuthenticatedParticipantInfo, InfoV1Error> {
        if expected_peer == self.peer_id() {
            self.runtime.info_v1().local()
        } else {
            self.runtime.info_v1().fetch(expected_peer).await
        }
    }

    /// Fetch all Resource Catalog v0.2 rows from one expected peer.
    pub async fn fetch_resources_catalog(
        &self,
        expected_peer: PeerId,
    ) -> Result<ResourcesResponse, ResourcesV2Error> {
        self.fetch_resources_catalog_with(expected_peer, ResourcesRequest::all())
            .await
    }

    /// Fetch filtered Resource Catalog v0.2 rows.
    pub async fn fetch_resources_catalog_with(
        &self,
        expected_peer: PeerId,
        request: ResourcesRequest,
    ) -> Result<ResourcesResponse, ResourcesV2Error> {
        if expected_peer == self.peer_id() {
            self.runtime.resources_v2().local(&request)
        } else {
            self.runtime
                .resources_v2()
                .fetch(expected_peer, request)
                .await
        }
    }

    /// Fetch all Resource Catalog v0.3 rows from one expected peer.
    pub async fn fetch_resources_catalog_v3(
        &self,
        expected_peer: PeerId,
    ) -> Result<ResourcesResponseV3, ResourcesV3Error> {
        self.fetch_resources_catalog_v3_with(expected_peer, ResourcesRequestV3::all())
            .await
    }

    /// Fetch filtered Resource Catalog v0.3 rows.
    pub async fn fetch_resources_catalog_v3_with(
        &self,
        expected_peer: PeerId,
        request: ResourcesRequestV3,
    ) -> Result<ResourcesResponseV3, ResourcesV3Error> {
        if expected_peer == self.peer_id() {
            self.runtime.resources_v3().local(&request)
        } else {
            self.runtime
                .resources_v3()
                .fetch(expected_peer, request)
                .await
        }
    }

    /// Fetch a peer's authenticated Map Log catalog.
    pub async fn fetch_map_catalog(
        &self,
        expected_peer: PeerId,
    ) -> Result<ResourcesResponseV4, ResourcesV4Error> {
        if expected_peer == self.peer_id() {
            self.runtime.resources_v4().local()
        } else {
            self.runtime.resources_v4().fetch(expected_peer).await
        }
    }

    /// Replace the shared Registry/blob application root.
    pub fn set_registry_app_root(&self, app_root: impl Into<PathBuf>) -> Result<(), DomainError> {
        self.runtime.set_registry_app_root(app_root)?;
        Ok(())
    }

    /// Replace the live Resource Catalog provider sampled by v0.2 and v0.3.
    pub fn set_resource_catalog_provider(
        &self,
        provider: Arc<dyn ResourceCatalogProvider>,
    ) -> Result<(), ResourcesV2Error> {
        self.runtime.set_resource_catalog_provider(provider)
    }

    /// Replace the live Map Log catalog provider sampled by v0.4.
    pub fn set_map_catalog_provider(
        &self,
        provider: Arc<dyn MapCatalogProvider>,
    ) -> Result<(), ResourcesV4Error> {
        self.runtime.set_map_catalog_provider(provider)
    }

    /// List content-pinned Registry rows. Device models are currently listable.
    pub async fn list_registry_entries(
        &self,
        expected_peer: PeerId,
        kind: RegistryKind,
    ) -> Result<Vec<RegistryListEntry>, RegistriesError> {
        self.runtime.registries().list(expected_peer, kind).await
    }

    /// Fetch and verify a Sensor Registry entry.
    pub async fn fetch_sensor_entry(
        &self,
        peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<SensorRegistryEntry, RegistriesError> {
        self.runtime.registries().fetch_sensor(peer, id, hash).await
    }

    /// Fetch and verify a Clock Registry entry.
    pub async fn fetch_clock_entry(
        &self,
        peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<ClockRegistryEntry, RegistriesError> {
        self.runtime.registries().fetch_clock(peer, id, hash).await
    }

    /// Fetch and verify a Frame Registry entry.
    pub async fn fetch_frame_entry(
        &self,
        peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<FrameRegistryEntry, RegistriesError> {
        self.runtime.registries().fetch_frame(peer, id, hash).await
    }

    /// Fetch and verify a Detector Registry entry.
    pub async fn fetch_detector_entry(
        &self,
        peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<DetectorRegistryEntry, RegistriesError> {
        self.runtime
            .registries()
            .fetch_detector(peer, id, hash)
            .await
    }

    /// Fetch and verify a Map Registry entry.
    pub async fn fetch_map_entry(
        &self,
        peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<MapRegistryEntry, RegistriesError> {
        self.runtime.registries().fetch_map(peer, id, hash).await
    }

    /// Fetch and verify a Device Model Registry entry.
    pub async fn fetch_device_model_entry(
        &self,
        peer: PeerId,
        id: impl Into<String>,
        hash: impl Into<String>,
    ) -> Result<DeviceModelRegistryEntry, RegistriesError> {
        self.runtime
            .registries()
            .fetch_device_model(peer, id, hash)
            .await
    }

    /// Fetch a complete content-addressed blob and return only verified bytes.
    pub async fn fetch_blob(
        &self,
        expected_peer: PeerId,
        sha256: impl AsRef<str>,
    ) -> Result<Vec<u8>, BlobsV1Error> {
        self.runtime
            .blobs()
            .fetch(expected_peer, sha256.as_ref())
            .await
    }

    /// Remove and return one receiver declared by the builder.
    pub fn take_message_channel_receiver(
        &mut self,
        resource_id: &str,
    ) -> Option<MessageChannelReceiver> {
        self.runtime
            .take_message_channel(resource_id)
            .map(|registration| MessageChannelReceiver { registration })
    }

    /// Open a persistent sender for an exact receiver-owned row.
    pub async fn open_message_channel(
        &self,
        expected_peer: PeerId,
        channel: &MessageChannelResource,
    ) -> Result<MessageChannelSender, OpenMessageChannelError> {
        let inner = self.runtime.messages().open(expected_peer, channel).await?;
        Ok(MessageChannelSender { inner })
    }

    /// Open, send once, and drop the sender. No automatic replay occurs.
    pub async fn send_message(
        &self,
        channel: &MessageChannelResource,
        r#type: impl Into<String>,
        timestamp_ns: i64,
        payload: impl Into<Vec<u8>>,
    ) -> Result<(), DomainSendMessageError> {
        self.open_message_channel(channel.owner_peer_id, channel)
            .await?
            .send(r#type, timestamp_ns, payload)
            .await?;
        Ok(())
    }

    /// Open one authenticated typed stream from an expected producer.
    pub async fn open_stream<T>(
        &self,
        expected_peer: PeerId,
        request: StreamRequest,
    ) -> Result<StreamSubscription<T>, StreamsError>
    where
        T: Message + Default + Send + 'static,
    {
        self.runtime.streams().open(expected_peer, request).await
    }

    /// Open and validate one exact discovered Map Log stream.
    pub async fn open_map_stream(
        &self,
        expected_peer: PeerId,
        resource: &MapLogResource,
        from: ReadFrom,
    ) -> Result<StreamSubscription<MapUpdate>, DomainOpenMapStreamError> {
        resource
            .validate()
            .map_err(DomainOpenMapStreamError::InvalidResource)?;
        if resource.writer_peer_id != expected_peer.to_string() {
            return Err(DomainOpenMapStreamError::WriterMismatch {
                target: expected_peer,
                writer_peer_id: resource.writer_peer_id.clone(),
            });
        }
        let subscription = self
            .open_stream(
                expected_peer,
                StreamRequest {
                    source_peer_id: resource.source_peer_id.clone(),
                    resource_id: resource.resource_id.clone(),
                    from,
                },
            )
            .await?;
        validate_map_stream_manifest(&subscription.manifest, resource)?;
        Ok(subscription)
    }

    /// Stop protocol services and owned I/O tasks before shutting down the node.
    pub async fn leave(self) -> Result<(), DomainError> {
        self.runtime.leave().await?;
        Ok(())
    }
}

fn validate_identity_chain(
    peer: &Peer,
    session: &Session,
    config: &DomainConfig,
) -> Result<(), DomainError> {
    let peer_id = peer.peer_id();
    let session_id = session.peer_id();
    if session_id != peer_id {
        return Err(DomainError::SessionIdentityMismatch {
            peer: peer_id,
            session: session_id,
        });
    }
    if peer_id != config.peer_id().to_string() {
        return Err(DomainError::IdentityMismatch {
            peer: peer_id,
            identity: config.peer_id(),
        });
    }
    Ok(())
}

/// Build the unchanged Resource Catalog v0.2 snapshot without joining.
pub fn catalog_of(peer: &Peer, session: &Session) -> Vec<ResourceEntry> {
    DomainCatalog {
        logs: session.logs(),
        registries: peer.registries(),
    }
    .catalog()
}

/// Build the unchanged Resource Catalog v0.4 Map Log snapshot without joining.
pub fn map_catalog_of(session: &Session) -> Vec<MapLogResource> {
    map_catalog_from_logs(&session.logs())
}

struct DomainCatalog {
    logs: SessionLogs,
    registries: PeerRegistries,
}

impl DomainCatalog {
    fn catalog(&self) -> Vec<ResourceEntry> {
        let mut out = Vec::new();
        out.extend(
            self.logs
                .sensor_logs()
                .iter()
                .map(|handle| sensor_log_row(handle, &self.registries)),
        );
        out.extend(
            self.logs
                .pose_logs()
                .iter()
                .map(|handle| pose_log_row(handle)),
        );
        out.extend(
            self.logs
                .time_logs()
                .iter()
                .map(|handle| time_transform_row(handle)),
        );
        out.extend(
            self.logs
                .detection_logs()
                .iter()
                .map(|handle| detection_log_row(handle)),
        );
        out
    }
}

impl ResourceCatalogProvider for DomainCatalog {
    fn snapshot(&self) -> Vec<ResourceEntry> {
        self.catalog()
    }
}

impl MapCatalogProvider for DomainCatalog {
    fn map_catalog(&self) -> ResourcesResponseV4 {
        ResourcesResponseV4 {
            resources: map_catalog_from_logs(&self.logs),
        }
    }
}

fn map_catalog_from_logs(logs: &SessionLogs) -> Vec<MapLogResource> {
    logs.map_logs()
        .into_iter()
        .map(|handle| MapLogResource {
            source_peer_id: handle.manifest.source_peer_id.clone(),
            writer_peer_id: handle.manifest.writer_peer_id.clone(),
            resource_id: handle.resource_id.clone(),
            map: handle.manifest.map.clone(),
            clock: handle.manifest.clock.clone(),
        })
        .collect()
}

fn map_stream_provider(fallback: StreamProvider, logs: SessionLogs) -> StreamProvider {
    Arc::new(move |peer, request| {
        let handle = logs.map_logs().into_iter().find(|handle| {
            handle.resource_id == request.resource_id
                && (request.source_peer_id.is_empty()
                    || handle.manifest.source_peer_id == request.source_peer_id)
        });
        let Some(handle) = handle else {
            return fallback(peer, request);
        };
        match map_log_source(&handle, request.from) {
            Ok(source) => StreamDispatch::AcceptMap {
                manifest: map_stream_manifest(&handle),
                source,
            },
            Err(detail) => StreamDispatch::Decline {
                reason: auki_protocols::stream::v2::DeclineReason::other(detail),
            },
        }
    })
}

fn detection_stream_provider(fallback: StreamProvider, logs: SessionLogs) -> StreamProvider {
    Arc::new(move |peer, request| {
        let handle = logs.detection_logs().into_iter().find(|handle| {
            handle.resource_id == request.resource_id
                && (request.source_peer_id.is_empty()
                    || handle.manifest.source_peer_id == request.source_peer_id)
        });
        let Some(handle) = handle else {
            return fallback(peer, request);
        };
        match detection_log_source(&handle, request.from) {
            Ok(source) => StreamDispatch::AcceptDetection {
                manifest: detection_stream_manifest(&handle),
                source,
            },
            Err(detail) => StreamDispatch::Decline {
                reason: auki_protocols::stream::v2::DeclineReason::other(detail),
            },
        }
    })
}

fn map_log_source(
    handle: &auki_session::MapLogHandle,
    from: ReadFrom,
) -> Result<SourceStream<MapUpdate>, String> {
    let (history, receiver) = match from {
        ReadFrom::Latest => (Vec::new(), handle.subscribe()),
        ReadFrom::FromStart | ReadFrom::FromTimestamp(_) => handle
            .snapshot_and_subscribe()
            .map_err(|error| error.to_string())?,
    };
    let history = history
        .into_iter()
        .filter(move |entry| match from {
            ReadFrom::FromTimestamp(start) => entry.timestamp_ns >= start,
            _ => true,
        })
        .map(|entry| {
            Ok(StreamItem {
                timestamp_ns: entry.timestamp_ns,
                payload: entry.payload,
            })
        });
    let live = stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok((timestamp_ns, payload)) => Some((
                Ok(StreamItem {
                    timestamp_ns,
                    payload,
                }),
                receiver,
            )),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => Some((
                Err(format!("map log subscriber lagged by {count} updates")),
                receiver,
            )),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    Ok(Box::pin(stream::iter(history).chain(live)))
}

fn detection_log_source(
    handle: &DetectionLogHandle,
    from: ReadFrom,
) -> Result<SourceStream<auki_datatypes::detection::DetectionFrame>, String> {
    let (history, receiver) = match from {
        ReadFrom::Latest => (Vec::new(), handle.subscribe()),
        ReadFrom::FromStart | ReadFrom::FromTimestamp(_) => handle
            .snapshot_and_subscribe()
            .map_err(|error| error.to_string())?,
    };
    let history = history
        .into_iter()
        .filter(move |entry| match from {
            ReadFrom::FromTimestamp(start) => entry.timestamp_ns >= start,
            _ => true,
        })
        .map(|entry| {
            Ok(StreamItem {
                timestamp_ns: entry.timestamp_ns,
                payload: entry.payload,
            })
        });
    let live = stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok((timestamp_ns, payload)) => Some((
                Ok(StreamItem {
                    timestamp_ns,
                    payload,
                }),
                receiver,
            )),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => Some((
                Err(format!(
                    "detection log subscriber lagged by {count} updates"
                )),
                receiver,
            )),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    Ok(Box::pin(stream::iter(history).chain(live)))
}

fn map_stream_manifest(handle: &auki_session::MapLogHandle) -> StreamManifest {
    StreamManifest {
        resource_id: handle.resource_id.clone(),
        payload: "map_update".into(),
        map_peer_id: handle.manifest.map.peer_id.clone(),
        map_id: handle.manifest.map.id.clone(),
        map_hash: handle.manifest.map.hash.clone(),
        clock_peer_id: handle.manifest.clock.peer_id.clone(),
        clock_id: handle.manifest.clock.id.clone(),
        clock_hash: handle.manifest.clock.hash.clone(),
        ..Default::default()
    }
}

fn detection_stream_manifest(handle: &DetectionLogHandle) -> StreamManifest {
    StreamManifest {
        resource_id: handle.resource_id.clone(),
        payload: "detection".into(),
        sensor_id: handle.manifest.input_sensor.id.clone(),
        sensor_hash: handle.manifest.input_sensor.hash.clone(),
        clock_peer_id: handle.manifest.clock.peer_id.clone(),
        clock_id: handle.manifest.clock.id.clone(),
        clock_hash: handle.manifest.clock.hash.clone(),
        ..Default::default()
    }
}

fn validate_map_stream_manifest(
    manifest: &StreamManifest,
    resource: &MapLogResource,
) -> Result<(), DomainOpenMapStreamError> {
    (manifest.resource_id == resource.resource_id
        && manifest.payload == "map_update"
        && manifest.map_peer_id == resource.map.peer_id
        && manifest.map_id == resource.map.id
        && manifest.map_hash == resource.map.hash
        && manifest.clock_peer_id == resource.clock.peer_id
        && manifest.clock_id == resource.clock.id
        && manifest.clock_hash == resource.clock.hash)
        .then_some(())
        .ok_or(DomainOpenMapStreamError::ManifestMismatch)
}

fn head_from_spec(spec: &HeadSpec) -> Option<Head> {
    match spec {
        HeadSpec::Rolling { retention_ns } => Some(Head::Rolling {
            retention_ns: *retention_ns,
        }),
        HeadSpec::Fixed => Some(Head::Fixed { started_at_ns: 0 }),
    }
}

fn sensor_kind_and_type(body: &SensorBody) -> (SensorKind, String) {
    match body {
        SensorBody::Camera(body) => (SensorKind::Camera, body.r#type.clone()),
        SensorBody::Rangefinder(body) => (SensorKind::Rangefinder, body.r#type.clone()),
        SensorBody::Rf(body) => (SensorKind::Rf, body.r#type.clone()),
        SensorBody::Audio(body) => (SensorKind::Audio, body.r#type.clone()),
        SensorBody::JointEncoders(body) => (SensorKind::JointEncoders, body.r#type.clone()),
        SensorBody::Scalar(body) => (SensorKind::Scalar, body.r#type.clone()),
    }
}

fn base_row(
    source_peer_id: String,
    writer_peer_id: String,
    resource_id: String,
    head: Option<Head>,
    sensor: Option<SensorBlock>,
    pose: Option<PoseBlock>,
    variant_content: VariantContent,
) -> ResourceEntry {
    ResourceEntry {
        source_peer_id,
        writer_peer_id,
        resource_id,
        state: "live".into(),
        head,
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor,
        pose,
        variant_content,
    }
}

fn sensor_log_row(handle: &SensorLogHandle, registries: &PeerRegistries) -> ResourceEntry {
    let (kind, r#type) = registries
        .sensor(&handle.manifest.sensor.id)
        .map(|entry| sensor_kind_and_type(&entry.body))
        .unwrap_or((SensorKind::Camera, String::new()));
    base_row(
        handle.manifest.source_peer_id.clone(),
        handle.manifest.writer_peer_id.clone(),
        handle.resource_id.clone(),
        head_from_spec(&handle.head_spec),
        Some(SensorBlock {
            kind,
            r#type,
            sensor_id: handle.manifest.sensor.id.clone(),
            sensor_hash: handle.manifest.sensor.hash.clone(),
        }),
        None,
        VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: handle.manifest.clock.clone(),
                frame: handle.manifest.frame.clone(),
            },
        },
    )
}

fn pose_log_row(handle: &PoseLogHandle) -> ResourceEntry {
    base_row(
        handle.manifest.source_peer_id.clone(),
        handle.manifest.writer_peer_id.clone(),
        handle.resource_id.clone(),
        head_from_spec(&handle.head_spec),
        None,
        Some(PoseBlock {
            writer_mode: handle.writer_mode,
        }),
        VariantContent::PoseLog {
            manifest: PoseManifestPointer {
                from_frame: handle.manifest.from_frame.clone(),
                to_frame: handle.manifest.to_frame.clone(),
                clock: handle.manifest.clock.clone(),
                source: handle.manifest.source.clone(),
                expected_rate_hz: handle.manifest.expected_rate_hz,
            },
        },
    )
}

fn time_transform_row(handle: &TimeTransformLogHandle) -> ResourceEntry {
    base_row(
        handle.manifest.source_peer_id.clone(),
        handle.manifest.writer_peer_id.clone(),
        handle.resource_id.clone(),
        head_from_spec(&handle.head_spec),
        None,
        None,
        VariantContent::TimeTransformLog {
            manifest: TimeTransformManifestPointer {
                from_clock: handle.manifest.from_clock.clone(),
                to_clock: handle.manifest.to_clock.clone(),
                source: handle.manifest.source.clone(),
            },
        },
    )
}

fn detection_log_row(handle: &DetectionLogHandle) -> ResourceEntry {
    base_row(
        handle.manifest.source_peer_id.clone(),
        handle.manifest.writer_peer_id.clone(),
        handle.resource_id.clone(),
        head_from_spec(&handle.head_spec),
        None,
        None,
        VariantContent::DetectionLog {
            manifest: DetectionManifestPointer {
                instance_id: handle.manifest.instance_id.clone(),
                detector: handle.manifest.detector.clone(),
                input_log: handle.manifest.input_log.clone(),
                input_sensor: handle.manifest.input_sensor.clone(),
                clock: handle.manifest.clock.clone(),
                cadence: handle.manifest.cadence,
            },
        },
    )
}
