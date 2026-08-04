//! `Domain` — the network face of a peer + session.
//!
//! A [`Domain`] composes a `&Peer` (eternal identity + registries) and a
//! `&Session` (one timeline's logs) from `auki-session` and gives them a
//! presence on a cluster: it bootstraps a [`ClusterManager`] and serves the
//! resource catalog (`Peer.registries` + `Session.logs`) to remote peers that
//! ask via `/auki/resources/*`.
//!
//! `auki-session` has no network dependencies; everything network-facing lives
//! here. See #274 (D3).

use std::{collections::HashMap, sync::Arc};

use futures::{StreamExt, stream};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;

use auki_network::resources_protocol::{
    Available, DetectionManifestPointer, Head, PoseBlock, PoseManifestPointer, ResourceEntry,
    SensorBlock, SensorKind, SensorManifestPointer, TimeTransformManifestPointer, VariantContent,
};
use auki_network::stream_runtime::StreamProvider;
use auki_network::stream_runtime::{SourceStream, StreamDispatch, StreamItem, StreamSubscription};
use auki_network::swarm::Behaviour;
use auki_network::{
    MapCatalogProvider, MapLogResource, MessageChannelRegistration, MessageChannelResource,
    MessageChannelSender, OpenMessageChannelError, PeerIdentity, RegistrationError,
    ResourcesProtocolErrorV4, ResourcesResponseV4, SendMessageError, SessionHandle, Swarm,
    resources_v3_protocol::{
        ResourcesProtocolError as ResourcesProtocolErrorV3, ResourcesRequest as ResourcesRequestV3,
        ResourcesResponse as ResourcesResponseV3,
    },
    stream_protocol::{ReadFrom, StreamManifest, StreamRequest},
};

use auki_registry::SensorBody;
use auki_session::{
    DetectionLogHandle, HeadSpec, Peer, PeerRegistries, PoseLogHandle, SensorLogHandle, Session,
    SessionLogs, TimeTransformLogHandle,
};

use crate::cluster_manager::{
    BootstrapError, ClusterManager, ClusterTarget, DaemonInfo, DiscoveryClientError,
};

// ─── DomainConfig ─────────────────────────────────────────────────────────────

/// Everything [`Domain::join`] needs that the peer / session don't own:
/// the cluster bootstrap policy, the local libp2p identity, the dialable
/// addresses, the Discovery service URL, the already-built swarm and stream
/// provider, and the daemon identity fields.
pub struct DomainConfig {
    /// Which cluster to create or join.
    pub target: ClusterTarget,
    /// The local libp2p identity (ed25519 keypair + derived `PeerId`).
    pub local_identity: PeerIdentity,
    /// Dialable multiaddrs to advertise in Discovery.
    pub local_multiaddrs: Vec<Multiaddr>,
    /// HTTP base URL of the Hagall Discovery service.
    pub discovery_url: String,
    /// Pre-built libp2p swarm for this peer.
    pub swarm: Swarm<Behaviour>,
    /// Provider for stream substream handling.
    pub stream_provider: StreamProvider,
    /// Static daemon identity fields (app, name, session_id, etc.).
    pub daemon_info: DaemonInfo,
}

/// A receiver-owned message-channel declaration composed before the Domain
/// joins. Registrations become visible atomically with their bounded live
/// receiver before [`DomainBuilder::join`] returns.
pub struct DomainBuilder<'a> {
    peer: &'a Peer,
    session: &'a Session,
    config: DomainConfig,
    message_channels: Vec<(MessageChannelResource, usize)>,
}

impl<'a> DomainBuilder<'a> {
    /// Start composing a Domain join.
    pub fn new(peer: &'a Peer, session: &'a Session, config: DomainConfig) -> Self {
        Self {
            peer,
            session,
            config,
            message_channels: Vec::new(),
        }
    }

    /// Add a bounded, live-only receiver-owned message channel.
    ///
    /// The owner must equal `peer.peer_id()`, the clock reference must exactly
    /// match a clock registered in the supplied Session, the resource must be
    /// valid, the capacity must be nonzero, and the owner/resource id pair must
    /// be unique within this builder.
    pub fn message_channel(
        mut self,
        resource: MessageChannelResource,
        receiver_capacity: usize,
    ) -> Result<Self, DomainBuilderError> {
        resource
            .validate()
            .map_err(DomainBuilderError::InvalidMessageChannel)?;
        if resource.owner_peer_id.to_string() != self.peer.peer_id() {
            return Err(DomainBuilderError::ChannelOwnerMismatch {
                expected: self.peer.peer_id(),
                actual: resource.owner_peer_id.to_string(),
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
                owner_peer_id: resource.owner_peer_id.to_string(),
                resource_id: resource.resource_id,
            });
        }
        self.message_channels.push((resource, receiver_capacity));
        Ok(self)
    }

    /// Join the configured cluster and bind every declared message channel.
    pub async fn join(self) -> Result<Domain, DomainError> {
        Domain::join_with_message_channels(
            self.peer,
            self.session,
            self.config,
            self.message_channels,
        )
        .await
    }
}

/// Invalid message-channel composition detected before Domain bootstrap.
#[derive(Debug, thiserror::Error)]
pub enum DomainBuilderError {
    /// The channel owner differs from the Peer that will join.
    #[error("message channel owner {actual} does not match Domain peer {expected}")]
    ChannelOwnerMismatch {
        /// Peer identity the Domain will use.
        expected: String,
        /// Owner declared by the channel row.
        actual: String,
    },
    /// The same owner/resource id was declared twice.
    #[error("duplicate message channel {owner_peer_id}/{resource_id}")]
    DuplicateMessageChannel {
        /// Receiver owner.
        owner_peer_id: String,
        /// Owner-scoped resource id.
        resource_id: String,
    },
    /// A bounded receiver cannot have zero capacity.
    #[error("message channel receiver capacity must be greater than zero")]
    ZeroReceiverCapacity,
    /// The channel clock does not exactly identify a clock in this Session.
    #[error(
        "message channel clock is not registered in Session: {peer_id}/{clock_id}@{clock_hash}"
    )]
    UnregisteredChannelClock {
        /// Peer identity carried by the clock reference.
        peer_id: String,
        /// Clock registry id.
        clock_id: String,
        /// Clock declaration hash.
        clock_hash: String,
    },
    /// The v0.3 resource row is malformed.
    #[error("invalid message channel resource: {0}")]
    InvalidMessageChannel(#[source] ResourcesProtocolErrorV3),
}

// ─── DomainError ──────────────────────────────────────────────────────────────

/// Errors returned by [`Domain::join`] / [`Domain::leave`].
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    /// The supplied Session was started by a different Peer.
    #[error("session peer id {session:?} != Domain peer id {peer:?}")]
    SessionIdentityMismatch {
        /// Identity of the Peer supplied to the Domain.
        peer: String,
        /// Identity retained by the supplied Session.
        session: String,
    },
    /// The peer's id is not the local network identity. The session's
    /// registered clock is keyed by `peer.peer_id()`, while the cluster's
    /// runtime clock is keyed by `local_identity.peer_id()`; if they differ
    /// the two clocks silently diverge. The peer must be constructed with the
    /// libp2p peer id as its `peer_id` (see the SDK identity convention).
    #[error("peer id {peer:?} != local network identity {identity:?}")]
    IdentityMismatch {
        /// The `peer.peer_id()` the session's clock is registered under.
        peer: String,
        /// The local libp2p identity the cluster's runtime clock would use.
        identity: String,
    },
    /// The pre-built swarm uses a different identity than `local_identity`.
    #[error("swarm local peer id {swarm:?} != configured local identity {identity:?}")]
    SwarmIdentityMismatch {
        /// Peer id derived from `DomainConfig::local_identity`.
        identity: String,
        /// Local peer id of `DomainConfig::swarm`.
        swarm: String,
    },
    /// Cluster bootstrap failed (Discovery unreachable, name collision, join
    /// rejection, etc.).
    #[error("domain bootstrap: {0}")]
    Bootstrap(#[from] BootstrapError),
    /// A predeclared message channel could not be bound to the runtime.
    #[error("domain message channel registration: {0}")]
    MessageChannelRegistration(#[from] RegistrationError),
    /// Discovery deregistration failed while leaving (the local peer was the
    /// last Manager and the HTTP DELETE failed). The manager is dropped
    /// regardless.
    #[error("domain shutdown: {0}")]
    Shutdown(DiscoveryClientError),
}

/// One live inbound application event.
///
/// All fields are opaque to the SDK except the authenticated sender and channel
/// routing identity. Applications own interpretation, freshness, scheduling,
/// and action policy.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageEvent {
    /// Receiver-owned Resource Catalog row that accepted the message.
    pub channel: MessageChannelResource,
    /// Noise-authenticated cluster peer that sent the message.
    pub sender: PeerId,
    /// Opaque application type string.
    pub r#type: String,
    /// Application timestamp expressed in the channel's declared clock.
    pub timestamp_ns: i64,
    /// Opaque application bytes.
    pub payload: Vec<u8>,
}

/// Bounded async receiver for one live-only message channel.
pub struct MessageChannelReceiver {
    registration: MessageChannelRegistration,
}

impl MessageChannelReceiver {
    /// Exact Resource Catalog row bound to this receiver.
    pub fn resource(&self) -> &MessageChannelResource {
        self.registration.resource()
    }

    /// Receive the next live event, or `None` after runtime closure.
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

/// Failure to open a discovered receiver-owned message channel.
#[derive(Debug, thiserror::Error)]
pub enum DomainOpenMessageChannelError {
    /// The catalog row owner is not the authenticated peer being opened.
    #[error("message channel owner {owner} does not match target peer {target}")]
    OwnerMismatch {
        /// Authenticated serving peer being dialed.
        target: PeerId,
        /// Owner in the discovered catalog row.
        owner: PeerId,
    },
    /// The discovered row is invalid.
    #[error("invalid message channel resource: {0}")]
    InvalidResource(#[source] ResourcesProtocolErrorV3),
    /// Network open or protocol negotiation failed.
    #[error("open message channel: {0}")]
    Open(#[from] OpenMessageChannelError),
}

/// Failure to open an exact discovered Map Log stream.
#[derive(Debug, thiserror::Error)]
pub enum DomainOpenMapStreamError {
    /// The catalog row is malformed.
    #[error("invalid map log resource: {0}")]
    InvalidResource(#[source] ResourcesProtocolErrorV4),
    /// The authenticated peer being dialed is not the row's writer.
    #[error("map log writer {writer_peer_id} does not match target peer {target}")]
    WriterMismatch {
        /// Authenticated serving peer being dialed.
        target: PeerId,
        /// Writer declared in the discovered row.
        writer_peer_id: String,
    },
    /// The producer accepted but did not bind the stream to the exact map,
    /// clock, payload, and resource identity from discovery.
    #[error("accepted map stream manifest does not match the discovered resource")]
    ManifestMismatch,
    /// Network open or stream protocol negotiation failed.
    #[error("open map stream: {0}")]
    Open(#[from] auki_network::stream_runtime::OpenStreamError),
}

/// Failure from [`Domain::send_message`].
#[derive(Debug, thiserror::Error)]
pub enum DomainSendMessageError {
    /// Opening the one-shot sender failed.
    #[error(transparent)]
    Open(#[from] DomainOpenMessageChannelError),
    /// Sending or receiving the transport ACK failed.
    #[error("send message: {0}")]
    Send(#[from] SendMessageError),
}

// ─── Domain ────────────────────────────────────────────────────────────────────

/// Network presence for a peer + its current session.
pub struct Domain {
    manager: ClusterManager,
    catalog: Arc<DomainCatalog>,
    message_channels: HashMap<String, MessageChannelRegistration>,
}

impl Domain {
    /// Join or create a cluster as described by `config.target`, bootstrap the
    /// [`ClusterManager`], and wire it a [`SessionHandle`] so inbound
    /// `/auki/resources/*` requests return the catalog built from `peer`'s
    /// registries and `session`'s logs.
    ///
    /// Returns `Err(DomainError::Bootstrap(_))` if the cluster bootstrap fails.
    pub async fn join(
        peer: &Peer,
        session: &Session,
        config: DomainConfig,
    ) -> Result<Domain, DomainError> {
        DomainBuilder::new(peer, session, config).join().await
    }

    async fn join_with_message_channels(
        peer: &Peer,
        session: &Session,
        mut config: DomainConfig,
        message_channels: Vec<(MessageChannelResource, usize)>,
    ) -> Result<Domain, DomainError> {
        // Validate the complete identity chain before bootstrap can touch the
        // network or Discovery.
        let peer_id = peer.peer_id();
        let session_id = session.peer_id();
        if session_id != peer_id {
            return Err(DomainError::SessionIdentityMismatch {
                peer: peer_id,
                session: session_id,
            });
        }
        let local_id = config.local_identity.peer_id().to_string();
        if peer_id != local_id {
            return Err(DomainError::IdentityMismatch {
                peer: peer_id,
                identity: local_id,
            });
        }
        let swarm_id = config.swarm.local_peer_id().to_string();
        if swarm_id != local_id {
            return Err(DomainError::SwarmIdentityMismatch {
                identity: local_id,
                swarm: swarm_id,
            });
        }

        // Stamp the session's authoritative clock identity into DaemonInfo.
        // The cluster rebuilds a `SessionClock` from `daemon_info.session_id`
        // (+ the matching peer id), which reconstructs the identical
        // `ClockRegistryEntry` `start_session` registered — so the advertised
        // `(id, hash)` resolves to the registry entry. Replaces callers
        // hand-feeding these (and the `"compat"` placeholders).
        let mono = session.monotonic_clock();
        config.daemon_info.session_id = session.session_id();
        config.daemon_info.session_clock_id = mono.id;
        config.daemon_info.session_clock_hash = mono.hash;

        // Map Logs are SDK resources, so serving them must not depend on an
        // application callback. Compose them ahead of the existing provider;
        // all non-map requests continue to the application unchanged.
        config.stream_provider = map_stream_provider(config.stream_provider, session.logs());

        let manager = ClusterManager::bootstrap(
            config.target,
            config.local_identity,
            config.local_multiaddrs,
            config.discovery_url,
            config.swarm,
            config.stream_provider,
            config.daemon_info,
        )
        .await?;

        let catalog = Arc::new(DomainCatalog {
            logs: session.logs(),
            registries: peer.registries(),
        });
        let handle: Arc<dyn SessionHandle> = catalog.clone();
        manager.set_session_handle(handle);
        let map_catalog_provider: Arc<dyn MapCatalogProvider> = catalog.clone();
        manager.set_map_catalog_provider(map_catalog_provider);

        let mut registrations = HashMap::new();
        for (resource, capacity) in message_channels {
            let resource_id = resource.resource_id.clone();
            let registration = manager.register_message_channel(resource, capacity)?;
            registrations.insert(resource_id, registration);
        }

        Ok(Domain {
            manager,
            catalog,
            message_channels: registrations,
        })
    }

    /// The unchanged Resource Catalog v0.2 snapshot this Domain currently
    /// serves: one row per registered log in `/auki/resources/0.2.0` shape.
    ///
    /// Message channels are v0.3-only and are not returned here.
    pub fn catalog(&self) -> Vec<ResourceEntry> {
        self.catalog.catalog()
    }

    /// The active [`ClusterManager`].
    pub fn cluster_manager(&self) -> &ClusterManager {
        &self.manager
    }

    /// Remove and return the app receiver created by a builder declaration.
    ///
    /// The registration remains active and advertised while the returned
    /// receiver is alive. Dropping it deregisters the channel immediately.
    pub fn take_message_channel_receiver(
        &mut self,
        resource_id: &str,
    ) -> Option<MessageChannelReceiver> {
        self.message_channels
            .remove(resource_id)
            .map(|registration| MessageChannelReceiver { registration })
    }

    /// Fetch a peer's unchanged Resource Catalog v0.2.
    pub async fn fetch_resources_catalog(
        &self,
        peer_id: PeerId,
    ) -> Result<
        auki_network::resources_protocol::ResourcesResponse,
        crate::cluster_manager::FetchResourcesCatalogError,
    > {
        self.manager.fetch_resources_catalog(peer_id).await
    }

    /// Fetch a peer's Resource Catalog v0.3 explicitly.
    ///
    /// No fallback to v0.2 is attempted.
    pub async fn fetch_resources_catalog_v3(
        &self,
        peer_id: PeerId,
    ) -> Result<ResourcesResponseV3, crate::cluster_manager::FetchResourcesCatalogV3Error> {
        self.manager.fetch_resources_catalog_v3(peer_id).await
    }

    /// Fetch a filtered Resource Catalog v0.3 explicitly.
    ///
    /// No fallback to v0.2 is attempted.
    pub async fn fetch_resources_catalog_v3_with(
        &self,
        peer_id: PeerId,
        request: ResourcesRequestV3,
    ) -> Result<ResourcesResponseV3, crate::cluster_manager::FetchResourcesCatalogV3Error> {
        self.manager
            .fetch_resources_catalog_v3_with(peer_id, request)
            .await
    }

    /// Fetch a peer's Map Log catalog over Resource Catalog v0.4.
    pub async fn fetch_map_catalog(
        &self,
        peer_id: PeerId,
    ) -> Result<ResourcesResponseV4, crate::cluster_manager::FetchMapCatalogError> {
        self.manager.fetch_map_catalog(peer_id).await
    }

    /// Open an exact discovered Map Log and receive replay plus live
    /// [`auki_datatypes::map::MapUpdate`] entries through the SDK stream.
    pub async fn open_map_stream(
        &self,
        peer_id: PeerId,
        resource: &MapLogResource,
        from: ReadFrom,
    ) -> Result<
        StreamSubscription<auki_network::stream_protocol::map::MapUpdate>,
        DomainOpenMapStreamError,
    > {
        resource
            .validate()
            .map_err(DomainOpenMapStreamError::InvalidResource)?;
        if resource.writer_peer_id != peer_id.to_string() {
            return Err(DomainOpenMapStreamError::WriterMismatch {
                target: peer_id,
                writer_peer_id: resource.writer_peer_id.clone(),
            });
        }

        let subscription = self
            .manager
            .open_stream::<auki_network::stream_protocol::map::MapUpdate>(
                peer_id,
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

    /// Open a persistent sender for an exact discovered message-channel row.
    ///
    /// `target_peer_id` must equal the row owner. The complete row clock
    /// reference is bound into the open handshake so a stale discovered row is
    /// rejected if the receiver re-registers the same resource id with another
    /// clock.
    pub async fn open_message_channel(
        &self,
        target_peer_id: PeerId,
        channel: &MessageChannelResource,
    ) -> Result<MessageChannelSender, DomainOpenMessageChannelError> {
        channel
            .validate()
            .map_err(DomainOpenMessageChannelError::InvalidResource)?;
        if channel.owner_peer_id != target_peer_id {
            return Err(DomainOpenMessageChannelError::OwnerMismatch {
                target: target_peer_id,
                owner: channel.owner_peer_id,
            });
        }
        Ok(self
            .manager
            .open_message_channel(
                target_peer_id,
                channel.resource_id.clone(),
                channel.clock.clone(),
            )
            .await?)
    }

    /// Open, send one opaque live message, and drop the sender.
    ///
    /// Success means transport acceptance into the receiver runtime's bounded
    /// queue only. It does not mean application semantic acceptance. An error
    /// can be indeterminate if the receiver enqueued the event but its ACK was
    /// lost; callers must not automatically retry.
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

    /// Shut down the cluster manager and leave the domain.
    ///
    /// Returns `Err(DomainError::Shutdown(_))` only if the local peer was the
    /// last Manager in Discovery and the HTTP DELETE failed; the manager is
    /// dropped regardless.
    pub async fn leave(self) -> Result<(), DomainError> {
        self.manager
            .shutdown()
            .await
            .map_err(DomainError::Shutdown)?;
        Ok(())
    }
}

/// Build the resource catalog for a peer + session without bootstrapping a
/// cluster. Used by tests to assert wire-equivalence; production serving goes
/// through [`Domain::join`]'s installed [`SessionHandle`].
pub fn catalog_of(peer: &Peer, session: &Session) -> Vec<ResourceEntry> {
    DomainCatalog {
        logs: session.logs(),
        registries: peer.registries(),
    }
    .catalog()
}

/// Build the v0.4 Map Log catalog. Kept separate from [`catalog_of`] because
/// v0.2/v0.3 have closed row variants and must remain wire-compatible.
pub fn map_catalog_of(session: &Session) -> Vec<MapLogResource> {
    map_catalog_from_logs(&session.logs())
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

        let source = match map_log_source(&handle, request.from) {
            Ok(source) => source,
            Err(detail) => {
                return StreamDispatch::Decline {
                    reason: auki_network::stream_protocol::DeclineReason::other(detail),
                };
            }
        };
        StreamDispatch::AcceptMap {
            manifest: map_stream_manifest(&handle),
            source,
        }
    })
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

fn map_log_source(
    handle: &auki_session::MapLogHandle,
    from: ReadFrom,
) -> Result<SourceStream<auki_network::stream_protocol::map::MapUpdate>, String> {
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

fn validate_map_stream_manifest(
    manifest: &StreamManifest,
    resource: &MapLogResource,
) -> Result<(), DomainOpenMapStreamError> {
    let matches = manifest.resource_id == resource.resource_id
        && manifest.payload == "map_update"
        && manifest.map_peer_id == resource.map.peer_id
        && manifest.map_id == resource.map.id
        && manifest.map_hash == resource.map.hash
        && manifest.clock_peer_id == resource.clock.peer_id
        && manifest.clock_id == resource.clock.id
        && manifest.clock_hash == resource.clock.hash;
    if matches {
        Ok(())
    } else {
        Err(DomainOpenMapStreamError::ManifestMismatch)
    }
}

// ─── Catalog bridge ─────────────────────────────────────────────────────────

/// `SessionHandle` bridge: reads the session's live logs and the peer's
/// registries to build catalog rows on each inbound request.
struct DomainCatalog {
    logs: SessionLogs,
    registries: PeerRegistries,
}

impl DomainCatalog {
    fn catalog(&self) -> Vec<ResourceEntry> {
        let mut out = Vec::new();
        for handle in self.logs.sensor_logs() {
            out.push(sensor_log_row(&handle, &self.registries));
        }
        for handle in self.logs.pose_logs() {
            out.push(pose_log_row(&handle));
        }
        for handle in self.logs.time_logs() {
            out.push(time_transform_row(&handle));
        }
        for handle in self.logs.detection_logs() {
            out.push(detection_log_row(&handle));
        }
        out
    }
}

impl SessionHandle for DomainCatalog {
    fn catalog(&self) -> Vec<ResourceEntry> {
        DomainCatalog::catalog(self)
    }
}

impl MapCatalogProvider for DomainCatalog {
    fn map_catalog(&self) -> ResourcesResponseV4 {
        ResourcesResponseV4 {
            resources: map_catalog_from_logs(&self.logs),
        }
    }
}

// ─── Row builders ─────────────────────────────────────────────────────────────

fn head_from_spec(spec: &HeadSpec) -> Option<Head> {
    match spec {
        HeadSpec::Rolling { retention_ns } => Some(Head::Rolling {
            retention_ns: *retention_ns,
        }),
        HeadSpec::Fixed => Some(Head::Fixed { started_at_ns: 0 }), // stub; real timestamp when backing Log<T> is wired
    }
}

fn sensor_kind_and_type(body: &SensorBody) -> (SensorKind, String) {
    match body {
        SensorBody::Camera(b) => (SensorKind::Camera, b.r#type.clone()),
        SensorBody::Rangefinder(b) => (SensorKind::Rangefinder, b.r#type.clone()),
        SensorBody::Rf(b) => (SensorKind::Rf, b.r#type.clone()),
        SensorBody::Audio(b) => (SensorKind::Audio, b.r#type.clone()),
        SensorBody::JointEncoders(b) => (SensorKind::JointEncoders, b.r#type.clone()),
    }
}

fn sensor_log_row(handle: &SensorLogHandle, registries: &PeerRegistries) -> ResourceEntry {
    // Kind + type come from the peer's sensor registry (eternal; the log's
    // registration guarantees the sensor is present). Default only guards the
    // unreachable missing-entry case so the catalog handler never panics.
    let (kind, sensor_type) = registries
        .sensor(&handle.manifest.sensor.id)
        .map(|entry| sensor_kind_and_type(&entry.body))
        .unwrap_or((SensorKind::Camera, String::new()));

    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor: Some(SensorBlock {
            kind,
            r#type: sensor_type,
            sensor_id: handle.manifest.sensor.id.clone(),
            sensor_hash: handle.manifest.sensor.hash.clone(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: handle.manifest.clock.clone(),
                frame: handle.manifest.frame.clone(),
            },
        },
    }
}

fn pose_log_row(handle: &PoseLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor: None,
        pose: Some(PoseBlock {
            writer_mode: handle.writer_mode.clone(),
        }),
        variant_content: VariantContent::PoseLog {
            manifest: PoseManifestPointer {
                from_frame: handle.manifest.from_frame.clone(),
                to_frame: handle.manifest.to_frame.clone(),
                clock: handle.manifest.clock.clone(),
                source: handle.manifest.source.clone(),
                expected_rate_hz: handle.manifest.expected_rate_hz,
            },
        },
    }
}

fn time_transform_row(handle: &TimeTransformLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor: None,
        pose: None,
        variant_content: VariantContent::TimeTransformLog {
            manifest: TimeTransformManifestPointer {
                from_clock: handle.manifest.from_clock.clone(),
                to_clock: handle.manifest.to_clock.clone(),
                source: handle.manifest.source.clone(),
            },
        },
    }
}

fn detection_log_row(handle: &DetectionLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available {
            bytes: 0,
            entries: 0,
            duration_ns: 0,
        },
        sensor: None,
        pose: None,
        variant_content: VariantContent::DetectionLog {
            manifest: DetectionManifestPointer {
                instance_id: handle.manifest.instance_id.clone(),
                detector: handle.manifest.detector.clone(),
                input_log: handle.manifest.input_log.clone(),
                input_sensor: handle.manifest.input_sensor.clone(),
                clock: handle.manifest.clock.clone(),
                cadence: handle.manifest.cadence.clone(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_registry::{Camera, ClockBody, ClockMeta, Scope, SensorBody};
    use auki_session::{FrameDef, HeadSpec, Peer, SensorLogSpec};
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn catalog_of_returns_one_wire_row_per_sensor_log() {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let frame = peer
            .register_frame("head_left_camera_optical", FrameDef::ros_optical())
            .unwrap();
        let sensor = peer
            .register_sensor(
                "head_left_rgb",
                SensorBody::Camera(Camera {
                    r#type: "rgb".to_string(),
                    width: 1920,
                    height: 1200,
                    frame_rate_hz: 30,
                    image_encoding: "raw".to_string(),
                    pixel_format: "rgb8".to_string(),
                    row_stride_bytes: 1920 * 3,
                    color_space: "srgb".to_string(),
                    intrinsics_model: "pinhole".to_string(),
                    distortion_model: "brown_conrady".to_string(),
                    frame: frame.clone(),
                }),
            )
            .unwrap();
        let session = peer.start_session().unwrap();
        let clock = session
            .register_clock(
                "session/sdk_clock",
                ClockBody::MonotonicClock(ClockMeta {
                    unit: "ns".to_string(),
                    monotonic: true,
                    epoch: None,
                    scope: Scope::DeviceLocal,
                }),
            )
            .unwrap();
        session
            .register_sensor_log(SensorLogSpec {
                sensor,
                clock,
                frame: Some(frame),
                head: HeadSpec::Rolling {
                    retention_ns: 5_000_000_000,
                },
                segment_duration: Duration::from_secs(1),
                retention: Duration::from_secs(5),
            })
            .unwrap();

        let rows = catalog_of(&peer, &session);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.source_peer_id, "galbot");
        assert_eq!(row.writer_peer_id, "galbot");
        assert_eq!(row.resource_id, "head_left_rgb");
        assert_eq!(row.state, "live");
        assert!(matches!(
            row.head,
            Some(Head::Rolling {
                retention_ns: 5_000_000_000
            })
        ));
        // Kind + type were derived from the peer's sensor registry.
        let sensor_block = row.sensor.as_ref().unwrap();
        assert_eq!(sensor_block.kind, SensorKind::Camera);
        assert_eq!(sensor_block.r#type, "rgb");
        assert_eq!(sensor_block.sensor_id, "head_left_rgb");
        assert!(row.pose.is_none());
        assert!(matches!(
            row.variant_content,
            VariantContent::SensorLog { .. }
        ));
    }

    #[tokio::test]
    async fn map_catalog_and_stream_provider_follow_registered_map_log_live() {
        use auki_network::stream_runtime::StreamDispatch;
        use auki_registry::{FiniteF64, MapBody, VoxelMap, VoxelValueModel};
        use auki_session::MapLogSpec;
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let map = peer
            .register_map(
                "occupancy",
                MapBody::Voxel(VoxelMap {
                    frame,
                    voxel_size_m: FiniteF64(0.05),
                    chunk_dimension: 64,
                    value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                    semantic_classes: vec![],
                }),
            )
            .unwrap();
        let provider = DomainCatalog {
            logs: session.logs(),
            registries: peer.registries(),
        };
        assert!(
            MapCatalogProvider::map_catalog(&provider)
                .resources
                .is_empty()
        );
        let handle = session
            .register_map_log(MapLogSpec {
                map,
                clock: session.monotonic_clock(),
                head: HeadSpec::Fixed,
                segment_duration: Duration::from_secs(1),
                retention: Duration::ZERO,
            })
            .unwrap();
        let rows = map_catalog_of(&session);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].resource_id, "occupancy");

        let served = MapCatalogProvider::map_catalog(&provider);
        assert_eq!(served.resources, rows);

        let first = auki_network::stream_protocol::map::MapUpdate {
            voxel_chunks: vec![],
        };
        handle.append(100, &first).unwrap();
        let streams = map_stream_provider(
            auki_network::stream_runtime::decline_all_streams(),
            session.logs(),
        );
        let dispatch = streams(
            PeerId::random(),
            StreamRequest {
                source_peer_id: peer.peer_id().to_string(),
                resource_id: "occupancy".into(),
                from: ReadFrom::FromStart,
            },
        );
        let StreamDispatch::AcceptMap {
            manifest,
            mut source,
        } = dispatch
        else {
            panic!("registered Map Log must be served by the SDK provider")
        };
        assert_eq!(manifest.map_hash, rows[0].map.hash);
        validate_map_stream_manifest(&manifest, &rows[0]).unwrap();
        assert_eq!(source.next().await.unwrap().unwrap().timestamp_ns, 100);

        handle
            .append(
                200,
                &auki_network::stream_protocol::map::MapUpdate {
                    voxel_chunks: vec![],
                },
            )
            .unwrap();
        assert_eq!(source.next().await.unwrap().unwrap().timestamp_ns, 200);
    }

    #[test]
    fn catalog_of_is_empty_for_a_session_with_no_logs() {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("park", "vis").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        assert!(catalog_of(&peer, &session).is_empty());
    }
}
