// The public `Domain` facade owns this engine. Its modules stay private so the
// facade remains the product boundary while selected handle and error types are
// re-exported from the crate root.
#![allow(dead_code, missing_docs)]

pub(crate) mod authority;
pub(crate) mod blobs;
pub(crate) mod info_v1;
pub(crate) mod io_tasks;
pub(crate) mod messages;
#[cfg(test)]
mod p09_tests;
#[cfg(test)]
mod p10_tests;
#[cfg(test)]
mod p11_tests;
pub(crate) mod peers;
pub(crate) mod protocols;
pub(crate) mod registries;
pub(crate) mod relay_reservations;
pub(crate) mod resources_v2;
pub(crate) mod resources_v3;
pub(crate) mod resources_v4;
pub(crate) mod routes;
pub(crate) mod status;
pub(crate) mod storage;
pub(crate) mod streams;

use std::{
    collections::BTreeMap, panic::AssertUnwindSafe, path::PathBuf, sync::Arc, time::Duration,
};

use auki_p2p::{
    DdsTokenVerifier, DdsVerificationKeys, Identity, Multiaddr, Node, NodeObservationEvent,
    NodeObservationStatus, PeerId, SignedP2pCredential,
};
use auki_protocols::catalog::v3::MessageChannelResource;
use futures::FutureExt;
use parking_lot::Mutex;
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
    time::{Instant, timeout, timeout_at},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use authority::{DomainAuthority, DomainAuthorityError};
use blobs::{BlobsV1, BlobsV1Error};
use info_v1::{InfoV1, InfoV1Error, ParticipantInfoProvider};
use io_tasks::DomainIoTasks;
use messages::{
    MessageChannelRegistration, MessageChannelRegistrationError, MessagesV1, MessagesV1Error,
};
use peers::DomainPeers;
use protocols::{DomainProtocolError, DomainProtocols};
use registries::{Registries, RegistriesError};
use resources_v2::{ResourcesV2, ResourcesV2Error};
use resources_v3::{ResourcesV3, ResourcesV3Error};
use resources_v4::{ResourcesV4, ResourcesV4Error};
use routes::{DomainRoutes, DomainRoutesError};
use status::{DomainFailure, DomainStatus};
use storage::{RegistryBlobStorage, StorageError};
use streams::{Streams, StreamsError};

use crate::{
    resource_catalog::{MapCatalogProvider, ResourceCatalogProvider},
    served_protocols::ServedProtocols,
    stream_runtime::{StreamProvider, decline_all_streams},
};

const DOMAIN_LISTENER_LIMIT: usize = 16;
const DOMAIN_LISTEN_ADDRESS_MAX_BYTES: usize = 1_024;
const DOMAIN_LISTENER_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const DOMAIN_LEAVE_TIMEOUT: Duration = Duration::from_secs(30);

/// Private configuration composed by the public [`crate::DomainBuilder`] for
/// one Domain-owned authenticated node.
#[derive(Clone)]
pub(crate) struct AuthenticatedDomainConfig {
    domain_id: Uuid,
    identity: Identity,
    listen_addresses: Vec<Multiaddr>,
    initial_routes: BTreeMap<PeerId, Vec<Multiaddr>>,
}

impl AuthenticatedDomainConfig {
    pub(crate) fn new(domain_id: Uuid, identity: Identity) -> Self {
        Self {
            domain_id,
            identity,
            listen_addresses: Vec::new(),
            initial_routes: BTreeMap::new(),
        }
    }

    pub(crate) fn with_listen_addresses(
        mut self,
        listen_addresses: impl IntoIterator<Item = Multiaddr>,
    ) -> Result<Self, AuthenticatedDomainError> {
        let mut bounded = Vec::with_capacity(DOMAIN_LISTENER_LIMIT);
        for address in listen_addresses {
            if bounded.len() == DOMAIN_LISTENER_LIMIT {
                return Err(AuthenticatedDomainError::ListenerLimit {
                    count: DOMAIN_LISTENER_LIMIT + 1,
                    maximum: DOMAIN_LISTENER_LIMIT,
                });
            }
            bounded.push(address);
        }
        if let Some(address) = bounded
            .iter()
            .find(|address| address.len() > DOMAIN_LISTEN_ADDRESS_MAX_BYTES)
        {
            return Err(AuthenticatedDomainError::ListenAddressTooLong {
                encoded_bytes: address.len(),
                maximum: DOMAIN_LISTEN_ADDRESS_MAX_BYTES,
            });
        }
        bounded.sort_unstable_by_key(ToString::to_string);
        bounded.dedup();
        self.listen_addresses = bounded;
        Ok(self)
    }

    pub(crate) fn with_peer_routes(
        mut self,
        expected_peer: PeerId,
        candidates: impl IntoIterator<Item = Multiaddr>,
    ) -> Result<Self, AuthenticatedDomainError> {
        let candidates = routes::canonicalize_candidates(expected_peer, candidates)?;
        let replacing = self.initial_routes.contains_key(&expected_peer);
        let peer_count =
            self.initial_routes.len() + usize::from(!replacing && !candidates.is_empty());
        if peer_count > routes::MAX_DOMAIN_ROUTE_PEERS {
            return Err(DomainRoutesError::PeerLimitExceeded {
                peer_count,
                maximum: routes::MAX_DOMAIN_ROUTE_PEERS,
            }
            .into());
        }
        let prior_count = self.initial_routes.get(&expected_peer).map_or(0, Vec::len);
        let candidate_count = self
            .initial_routes
            .values()
            .map(Vec::len)
            .sum::<usize>()
            .saturating_sub(prior_count)
            .saturating_add(candidates.len());
        if candidate_count > routes::MAX_DOMAIN_ROUTE_CANDIDATES {
            return Err(DomainRoutesError::CandidateLimitExceeded {
                candidate_count,
                maximum: routes::MAX_DOMAIN_ROUTE_CANDIDATES,
            }
            .into());
        }
        if candidates.is_empty() {
            self.initial_routes.remove(&expected_peer);
        } else {
            self.initial_routes.insert(expected_peer, candidates);
        }
        Ok(self)
    }

    pub(crate) fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    pub(crate) fn peer_id(&self) -> PeerId {
        self.identity.peer_id()
    }
}

/// Protocol-specific inputs composed by `DomainBuilder` alongside the narrow
/// D05 node configuration.
///
/// Keeping these values separate from [`AuthenticatedDomainConfig`] preserves
/// the locked public shape: listener/identity/routes configure the node, while
/// receiver declarations and application providers configure their services.
#[derive(Clone)]
pub(crate) struct AuthenticatedDomainServicesConfig {
    message_channels: Vec<(MessageChannelResource, usize)>,
    stream_provider: StreamProvider,
    participant_info_provider: Option<Arc<dyn ParticipantInfoProvider>>,
    resource_catalog_provider: Option<Arc<dyn ResourceCatalogProvider>>,
    map_catalog_provider: Option<Arc<dyn MapCatalogProvider>>,
    registry_app_root: Option<PathBuf>,
    served_protocols: ServedProtocols,
}

impl Default for AuthenticatedDomainServicesConfig {
    fn default() -> Self {
        Self {
            message_channels: Vec::new(),
            stream_provider: decline_all_streams(),
            participant_info_provider: None,
            resource_catalog_provider: None,
            map_catalog_provider: None,
            registry_app_root: None,
            served_protocols: ServedProtocols::none(),
        }
    }
}

impl AuthenticatedDomainServicesConfig {
    pub(crate) fn with_message_channel(
        mut self,
        resource: MessageChannelResource,
        receiver_capacity: usize,
    ) -> Self {
        self.message_channels.push((resource, receiver_capacity));
        self
    }

    pub(crate) fn with_stream_provider(mut self, provider: StreamProvider) -> Self {
        self.stream_provider = provider;
        self
    }

    pub(crate) fn with_participant_info_provider(
        mut self,
        provider: Arc<dyn ParticipantInfoProvider>,
    ) -> Self {
        self.participant_info_provider = Some(provider);
        self
    }

    pub(crate) fn with_resource_catalog_provider(
        mut self,
        provider: Arc<dyn ResourceCatalogProvider>,
    ) -> Self {
        self.resource_catalog_provider = Some(provider);
        self
    }

    pub(crate) fn with_map_catalog_provider(
        mut self,
        provider: Arc<dyn MapCatalogProvider>,
    ) -> Self {
        self.map_catalog_provider = Some(provider);
        self
    }

    pub(crate) fn with_registry_app_root(mut self, app_root: PathBuf) -> Self {
        self.registry_app_root = Some(app_root);
        self
    }

    pub(crate) fn with_served_protocols(mut self, protocols: ServedProtocols) -> Self {
        self.served_protocols = protocols;
        self
    }
}

/// The private authenticated Domain engine used by retained protocol adapters.
pub(crate) struct AuthenticatedDomain {
    access: Arc<RuntimeAccess>,
    authority: DomainAuthority,
    routes: DomainRoutes,
    peers: DomainPeers,
    protocols: DomainProtocols,
    info_v1: InfoV1,
    resources_v2: ResourcesV2,
    resources_v3: ResourcesV3,
    resources_v4: ResourcesV4,
    registries: Registries,
    blobs: BlobsV1,
    messages: MessagesV1,
    streams: Streams,
    storage: RegistryBlobStorage,
    message_channels: BTreeMap<String, MessageChannelRegistration>,
    protocol_registrations: Vec<protocols::DomainProtocolRegistration>,
    served_protocol_ids: Vec<&'static str>,
    listen_addresses: Vec<Multiaddr>,
    supervisor: Option<JoinHandle<()>>,
    authority_expiry: Option<JoinHandle<()>>,
    io_task_host: Option<JoinHandle<()>>,
    cleanup_complete: bool,
}

impl AuthenticatedDomain {
    pub(crate) async fn join(
        config: AuthenticatedDomainConfig,
        verification_keys: DdsVerificationKeys,
        credential: SignedP2pCredential,
    ) -> Result<Self, AuthenticatedDomainError> {
        Self::join_with_services(
            config,
            verification_keys,
            credential,
            AuthenticatedDomainServicesConfig::default(),
        )
        .await
    }

    pub(crate) async fn join_with_services(
        config: AuthenticatedDomainConfig,
        verification_keys: DdsVerificationKeys,
        credential: SignedP2pCredential,
        services: AuthenticatedDomainServicesConfig,
    ) -> Result<Self, AuthenticatedDomainError> {
        let served_protocols = services.served_protocols;
        let lifecycle = CancellationToken::new();
        let status = status::DomainStatusController::credential_unavailable();
        let routes = DomainRoutes::new(lifecycle.clone());
        for (peer_id, candidates) in &config.initial_routes {
            routes.replace(*peer_id, candidates.clone())?;
        }

        let verification_refresh_started_at = Instant::now();
        let verifier = DdsTokenVerifier::from_keys(verification_keys)
            .map_err(AuthenticatedDomainError::P2p)?;
        let node = Node::start(config.identity, verifier, config.listen_addresses.clone())
            .map_err(AuthenticatedDomainError::P2p)?;
        let observations = node.observations();
        let access = Arc::new(RuntimeAccess {
            domain_id: config.domain_id,
            peer_id: node.peer_id(),
            lifecycle: lifecycle.clone(),
            status: status.clone(),
            node: Mutex::new(Some(node)),
        });
        let authority = DomainAuthority::new(Arc::clone(&access));
        status.refresh_verification_keys(verification_refresh_started_at);

        if let Err(error) = authority.install_credential(credential).await {
            return Err(
                rollback_join_error(&access, AuthenticatedDomainError::Authority(error)).await,
            );
        }

        let mut listen_addresses = match timeout(
            DOMAIN_LISTENER_STARTUP_TIMEOUT,
            access.node()?.wait_for_listeners(),
        )
        .await
        {
            Ok(Ok(addresses)) => addresses,
            Ok(Err(error)) => {
                return Err(
                    rollback_join_error(&access, AuthenticatedDomainError::P2p(error)).await,
                );
            }
            Err(_) => {
                return Err(rollback_join_error(
                    &access,
                    AuthenticatedDomainError::ListenerStartupTimeout,
                )
                .await);
            }
        };
        listen_addresses.sort_unstable_by_key(ToString::to_string);

        let peers = DomainPeers::new(config.domain_id, observations.clone(), lifecycle.clone());
        let (fatal_sender, fatal_receiver) = mpsc::unbounded_channel();
        let (io_tasks, io_task_host) = DomainIoTasks::new(lifecycle.clone(), fatal_sender.clone());
        let protocols = DomainProtocols::new(Arc::clone(&access), routes.clone(), fatal_sender);
        let storage = RegistryBlobStorage::new(access.peer_id, lifecycle.clone());
        let info_v1 = InfoV1::new(
            access.peer_id,
            protocols.clone(),
            peers.clone(),
            lifecycle.clone(),
        );
        let resources_v2 = ResourcesV2::new(protocols.clone(), lifecycle.clone());
        let resources_v3 = ResourcesV3::new(
            access.peer_id,
            protocols.clone(),
            resources_v2.clone(),
            lifecycle.clone(),
        );
        let resources_v4 = ResourcesV4::new(protocols.clone(), lifecycle.clone());
        let registries = Registries::new(protocols.clone(), storage.clone(), lifecycle.clone());
        let blobs = BlobsV1::new(protocols.clone(), storage.clone());
        let messages = MessagesV1::new(
            access.peer_id,
            protocols.clone(),
            lifecycle.clone(),
            io_tasks.clone(),
        );
        let streams = Streams::new(protocols.clone(), io_tasks, lifecycle.clone());
        if let Some(provider) = services.participant_info_provider
            && let Err(error) = info_v1.set_provider(provider)
        {
            return Err(
                rollback_join_error(&access, AuthenticatedDomainError::InfoV1(error)).await,
            );
        }
        if let Some(provider) = services.resource_catalog_provider
            && let Err(error) = resources_v2.set_provider(provider)
        {
            return Err(
                rollback_join_error(&access, AuthenticatedDomainError::ResourcesV2(error)).await,
            );
        }
        if let Some(provider) = services.map_catalog_provider
            && let Err(error) = resources_v4.set_provider(provider)
        {
            return Err(
                rollback_join_error(&access, AuthenticatedDomainError::ResourcesV4(error)).await,
            );
        }
        if let Some(app_root) = services.registry_app_root
            && let Err(error) = storage.set_app_root(app_root)
        {
            return Err(
                rollback_join_error(&access, AuthenticatedDomainError::Storage(error)).await,
            );
        }
        if let Err(error) = streams.set_provider(services.stream_provider) {
            return Err(
                rollback_join_error(&access, AuthenticatedDomainError::Streams(error)).await,
            );
        }
        let mut message_channels = BTreeMap::new();
        for (resource, receiver_capacity) in services.message_channels {
            let resource_id = resource.resource_id.clone();
            let registration = match messages.declare(resource, receiver_capacity) {
                Ok(registration) => registration,
                Err(error) => {
                    return Err(rollback_join_error(
                        &access,
                        AuthenticatedDomainError::MessageChannelRegistration(error),
                    )
                    .await);
                }
            };
            message_channels.insert(resource_id, registration);
        }
        if let Err(error) = resources_v3.set_message_channel_provider(Arc::new(messages.clone())) {
            return Err(
                rollback_join_error(&access, AuthenticatedDomainError::ResourcesV3(error)).await,
            );
        }

        let mut protocol_registrations = Vec::with_capacity(9);
        if served_protocols.serves_info_v1() {
            register_domain_protocol(
                &access,
                &protocols,
                &mut protocol_registrations,
                info_v1.register().map_err(AuthenticatedDomainError::InfoV1),
            )
            .await?;
        }
        if served_protocols.serves_resources_v2() {
            register_domain_protocol(
                &access,
                &protocols,
                &mut protocol_registrations,
                resources_v2
                    .register()
                    .map_err(AuthenticatedDomainError::ResourcesV2),
            )
            .await?;
        }
        if served_protocols.serves_resources_v3() {
            register_domain_protocol(
                &access,
                &protocols,
                &mut protocol_registrations,
                resources_v3
                    .register()
                    .map_err(AuthenticatedDomainError::ResourcesV3),
            )
            .await?;
        }
        if served_protocols.serves_resources_v4() {
            register_domain_protocol(
                &access,
                &protocols,
                &mut protocol_registrations,
                resources_v4
                    .register()
                    .map_err(AuthenticatedDomainError::ResourcesV4),
            )
            .await?;
        }
        if served_protocols.serves_registries_v2() {
            register_domain_protocol(
                &access,
                &protocols,
                &mut protocol_registrations,
                registries
                    .register_v2()
                    .map_err(AuthenticatedDomainError::Registries),
            )
            .await?;
        }
        if served_protocols.serves_registries_v3() {
            register_domain_protocol(
                &access,
                &protocols,
                &mut protocol_registrations,
                registries
                    .register_v3()
                    .map_err(AuthenticatedDomainError::Registries),
            )
            .await?;
        }
        if served_protocols.serves_blobs_v1() {
            register_domain_protocol(
                &access,
                &protocols,
                &mut protocol_registrations,
                blobs.register().map_err(AuthenticatedDomainError::Blobs),
            )
            .await?;
        }
        if served_protocols.serves_messages_v1() {
            register_domain_protocol(
                &access,
                &protocols,
                &mut protocol_registrations,
                messages
                    .register()
                    .map_err(AuthenticatedDomainError::Messages),
            )
            .await?;
        }
        if served_protocols.serves_streams_v2() {
            register_domain_protocol(
                &access,
                &protocols,
                &mut protocol_registrations,
                streams
                    .register()
                    .map_err(AuthenticatedDomainError::Streams),
            )
            .await?;
        }
        let served_protocol_ids = served_protocols.protocol_ids();

        let io_task_host = tokio::spawn(io_task_host.run());

        let supervisor_access = Arc::clone(&access);
        let supervisor = tokio::spawn(async move {
            let outcome = AssertUnwindSafe(supervise_runtime(
                Arc::clone(&supervisor_access),
                observations,
                fatal_receiver,
            ))
            .catch_unwind()
            .await;
            if outcome.is_err() && !supervisor_access.lifecycle.is_cancelled() {
                fail_and_shutdown(&supervisor_access, DomainFailure::SupervisorStopped).await;
            }
        });

        let expiry_access = Arc::clone(&access);
        let expiry_status = status.clone();
        let expiry_lifecycle = lifecycle.clone();
        let authority_expiry = tokio::spawn(async move {
            let outcome =
                AssertUnwindSafe(expiry_status.drive_authority_expiry(expiry_lifecycle.clone()))
                    .catch_unwind()
                    .await;
            if (outcome.is_err()
                || (!expiry_lifecycle.is_cancelled() && !expiry_access.status.is_terminal()))
                && !expiry_access.lifecycle.is_cancelled()
            {
                fail_and_shutdown(&expiry_access, DomainFailure::SupervisorStopped).await;
            }
        });

        Ok(Self {
            access,
            authority,
            routes,
            peers,
            protocols,
            info_v1,
            resources_v2,
            resources_v3,
            resources_v4,
            registries,
            blobs,
            messages,
            streams,
            storage,
            message_channels,
            protocol_registrations,
            served_protocol_ids,
            listen_addresses,
            supervisor: Some(supervisor),
            authority_expiry: Some(authority_expiry),
            io_task_host: Some(io_task_host),
            cleanup_complete: false,
        })
    }

    pub(crate) fn peer_id(&self) -> PeerId {
        self.access.peer_id
    }

    pub(crate) fn domain_id(&self) -> Uuid {
        self.access.domain_id
    }

    pub(crate) fn listen_addresses(&self) -> &[Multiaddr] {
        &self.listen_addresses
    }

    pub(crate) fn authority(&self) -> DomainAuthority {
        self.authority.clone()
    }

    pub(crate) fn routes(&self) -> DomainRoutes {
        self.routes.clone()
    }

    pub(crate) fn peers(&self) -> DomainPeers {
        self.peers.clone()
    }

    pub(crate) fn protocols(&self) -> DomainProtocols {
        self.protocols.clone()
    }

    pub(crate) fn relay_reservations(&self) -> relay_reservations::DomainRelayReservations {
        relay_reservations::DomainRelayReservations::new(Arc::clone(&self.access))
    }

    pub(crate) fn served_protocol_ids(&self) -> &[&'static str] {
        &self.served_protocol_ids
    }

    pub(crate) fn info_v1(&self) -> InfoV1 {
        self.info_v1.clone()
    }

    pub(crate) fn resources_v2(&self) -> ResourcesV2 {
        self.resources_v2.clone()
    }

    pub(crate) fn resources_v3(&self) -> ResourcesV3 {
        self.resources_v3.clone()
    }

    pub(crate) fn resources_v4(&self) -> ResourcesV4 {
        self.resources_v4.clone()
    }

    pub(crate) fn registries(&self) -> Registries {
        self.registries.clone()
    }

    pub(crate) fn blobs(&self) -> BlobsV1 {
        self.blobs.clone()
    }

    pub(crate) fn messages(&self) -> MessagesV1 {
        self.messages.clone()
    }

    pub(crate) fn streams(&self) -> Streams {
        self.streams.clone()
    }

    pub(crate) fn take_message_channel(
        &mut self,
        resource_id: &str,
    ) -> Option<MessageChannelRegistration> {
        self.message_channels.remove(resource_id)
    }

    pub(crate) fn set_registry_app_root(
        &self,
        app_root: impl Into<std::path::PathBuf>,
    ) -> Result<(), AuthenticatedDomainError> {
        self.storage.set_app_root(app_root)?;
        Ok(())
    }

    pub(crate) fn set_resource_catalog_provider(
        &self,
        provider: Arc<dyn ResourceCatalogProvider>,
    ) -> Result<(), ResourcesV2Error> {
        self.resources_v2.set_provider(provider)
    }

    pub(crate) fn set_map_catalog_provider(
        &self,
        provider: Arc<dyn MapCatalogProvider>,
    ) -> Result<(), ResourcesV4Error> {
        self.resources_v4.set_provider(provider)
    }

    pub(crate) fn status(&self) -> DomainStatus {
        self.access.status.status()
    }

    pub(crate) fn subscribe_status(&self) -> watch::Receiver<DomainStatus> {
        self.access.status.subscribe()
    }

    pub(crate) async fn leave(mut self) -> Result<(), AuthenticatedDomainError> {
        self.leave_inner().await
    }

    async fn leave_inner(&mut self) -> Result<(), AuthenticatedDomainError> {
        self.leave_until(Instant::now() + DOMAIN_LEAVE_TIMEOUT)
            .await
    }

    async fn leave_until(&mut self, deadline: Instant) -> Result<(), AuthenticatedDomainError> {
        if self.cleanup_complete {
            return Ok(());
        }
        self.access.lifecycle.cancel();
        self.peers.clear_participant_info();
        self.messages.shutdown();

        let mut first_error = self
            .protocols
            .shutdown_all(deadline)
            .await
            .err()
            .map(AuthenticatedDomainError::Protocol);
        if let Err(error) = await_task_until(&mut self.authority_expiry, deadline).await {
            first_error.get_or_insert(error);
        }
        if let Err(error) = await_task_until(&mut self.io_task_host, deadline).await {
            first_error.get_or_insert(error);
        }
        if let Err(error) = await_task_until(&mut self.supervisor, deadline).await {
            first_error.get_or_insert(error);
        }

        if let Some(node) = self.access.take_node() {
            match timeout_at(deadline, node.shutdown()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    first_error.get_or_insert(AuthenticatedDomainError::P2p(error));
                    node.shutdown_now().await;
                }
                Err(_) => {
                    first_error.get_or_insert(AuthenticatedDomainError::CleanupTimeout);
                    node.shutdown_now().await;
                }
            }
        }

        self.cleanup_complete = true;
        match first_error {
            Some(error) => {
                let failure = if cleanup_timed_out(&error) {
                    DomainFailure::CleanupTimeout
                } else {
                    DomainFailure::CleanupFailed
                };
                self.access.status.fail(failure);
                Err(error)
            }
            None => {
                self.access.status.stop();
                Ok(())
            }
        }
    }

    #[cfg(test)]
    pub(super) fn fail_protocol_host_for_test(&self) {
        self.protocols.fail_host_for_test();
    }
}

impl Drop for AuthenticatedDomain {
    fn drop(&mut self) {
        if self.cleanup_complete {
            return;
        }
        self.access.lifecycle.cancel();
        self.peers.clear_participant_info();
        self.messages.shutdown();
        // A Stopped observation is a public teardown barrier: fence every
        // cloned runtime surface before waking status subscribers.
        self.access.status.stop();
        self.protocols.abort_all();
        abort_task(&mut self.authority_expiry);
        abort_task(&mut self.io_task_host);
        abort_task(&mut self.supervisor);
        if let Some(node) = self.access.take_node() {
            spawn_best_effort_shutdown(node);
        }
    }
}

struct RuntimeAccess {
    domain_id: Uuid,
    peer_id: PeerId,
    lifecycle: CancellationToken,
    status: status::DomainStatusController,
    node: Mutex<Option<Node>>,
}

impl RuntimeAccess {
    fn node(&self) -> Result<Node, RuntimeAccessError> {
        if self.lifecycle.is_cancelled() {
            return Err(RuntimeAccessError::Stopped);
        }
        let node = self
            .node
            .lock()
            .as_ref()
            .cloned()
            .ok_or(RuntimeAccessError::Stopped)?;
        if self.lifecycle.is_cancelled() {
            return Err(RuntimeAccessError::Stopped);
        }
        Ok(node)
    }

    fn take_node(&self) -> Option<Node> {
        self.node.lock().take()
    }

    fn clone_node_for_shutdown(&self) -> Option<Node> {
        self.node.lock().as_ref().cloned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeFailureSignal {
    ProtocolHostStopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum RuntimeAccessError {
    #[error("the Domain runtime is stopped")]
    Stopped,
}

async fn supervise_runtime(
    access: Arc<RuntimeAccess>,
    observations: auki_p2p::NodeObservations,
    mut fatal: mpsc::UnboundedReceiver<RuntimeFailureSignal>,
) {
    let mut events = observations.subscribe();
    if apply_node_status(&access, observations.snapshot().status()).await {
        return;
    }
    loop {
        tokio::select! {
            biased;
            _ = access.lifecycle.cancelled() => return,
            signal = fatal.recv() => {
                match signal {
                    Some(RuntimeFailureSignal::ProtocolHostStopped) => {
                        fail_and_shutdown(&access, DomainFailure::ProtocolHostStopped).await;
                    }
                    None => {
                        fail_and_shutdown(&access, DomainFailure::SupervisorStopped).await;
                    }
                }
                return;
            }
            event = events.recv() => {
                match event {
                    Ok(NodeObservationEvent::StatusChanged(node_status)) => {
                        if apply_node_status(&access, node_status).await {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if apply_node_status(&access, observations.snapshot().status()).await {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        fail_and_shutdown(&access, DomainFailure::SupervisorStopped).await;
                        return;
                    }
                }
            }
        }
    }
}

async fn apply_node_status(access: &Arc<RuntimeAccess>, status: NodeObservationStatus) -> bool {
    match status {
        NodeObservationStatus::Running => false,
        NodeObservationStatus::Stopped if access.lifecycle.is_cancelled() => true,
        NodeObservationStatus::Stopped => {
            fail_and_shutdown(access, DomainFailure::NodeStoppedUnexpectedly).await;
            true
        }
        NodeObservationStatus::Failed(failure) => {
            fail_and_shutdown(access, DomainFailure::Node(failure)).await;
            true
        }
    }
}

async fn fail_and_shutdown(access: &Arc<RuntimeAccess>, failure: DomainFailure) {
    // Retain the RuntimeAccess-owned handle until the shutdown future reaches
    // its join barrier. If this supervisor is itself canceled by bounded
    // explicit leave, that original handle remains available for shutdown_now.
    let node = access.clone_node_for_shutdown();
    access.lifecycle.cancel();
    // Publish the terminal status only after every cloned runtime surface is
    // synchronously fenced by lifecycle cancellation.
    access.status.fail(failure);
    if let Some(node) = node {
        let _ = node.shutdown().await;
        access.take_node();
    }
}

async fn register_domain_protocol(
    access: &Arc<RuntimeAccess>,
    protocols: &DomainProtocols,
    registrations: &mut Vec<protocols::DomainProtocolRegistration>,
    registration: Result<protocols::DomainProtocolRegistration, AuthenticatedDomainError>,
) -> Result<(), AuthenticatedDomainError> {
    match registration {
        Ok(registration) => {
            registrations.push(registration);
            Ok(())
        }
        Err(join) => {
            access.lifecycle.cancel();
            let join = match protocols
                .shutdown_all(Instant::now() + DOMAIN_LEAVE_TIMEOUT)
                .await
            {
                Ok(()) => join,
                Err(cleanup) => AuthenticatedDomainError::ProtocolRegistrationRollback {
                    join: Box::new(join),
                    cleanup: Box::new(cleanup),
                },
            };
            Err(rollback_join_error(access, join).await)
        }
    }
}

async fn rollback_join_error(
    access: &Arc<RuntimeAccess>,
    join: AuthenticatedDomainError,
) -> AuthenticatedDomainError {
    match rollback_node(access).await {
        Ok(()) => join,
        Err(rollback) => AuthenticatedDomainError::JoinRollback {
            join: Box::new(join),
            rollback: Box::new(rollback),
        },
    }
}

async fn rollback_node(access: &Arc<RuntimeAccess>) -> Result<(), DomainRollbackError> {
    access.lifecycle.cancel();
    if let Some(node) = access.take_node() {
        match timeout(DOMAIN_LEAVE_TIMEOUT, node.shutdown()).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => {
                node.shutdown_now().await;
                return Err(DomainRollbackError::P2p(error));
            }
            Err(_) => {
                node.shutdown_now().await;
                return Err(DomainRollbackError::Timeout);
            }
        }
    }
    Ok(())
}

fn cleanup_timed_out(error: &AuthenticatedDomainError) -> bool {
    matches!(
        error,
        AuthenticatedDomainError::CleanupTimeout
            | AuthenticatedDomainError::Protocol(DomainProtocolError::CleanupTimeout)
    )
}

async fn await_task_until(
    task: &mut Option<JoinHandle<()>>,
    deadline: Instant,
) -> Result<(), AuthenticatedDomainError> {
    let Some(owned) = task.as_mut() else {
        return Ok(());
    };
    let result = match timeout_at(deadline, &mut *owned).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(AuthenticatedDomainError::Task(error)),
        Err(_) => {
            owned.abort();
            let _ = owned.await;
            Err(AuthenticatedDomainError::CleanupTimeout)
        }
    };
    task.take();
    result
}

fn abort_task(task: &mut Option<JoinHandle<()>>) {
    if let Some(task) = task.take() {
        task.abort();
    }
}

fn spawn_best_effort_shutdown(node: Node) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            let _ = node.shutdown().await;
        });
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthenticatedDomainError {
    #[error("Domain configuration has {count} listeners; maximum is {maximum}")]
    ListenerLimit { count: usize, maximum: usize },
    #[error("Domain listener is {encoded_bytes} encoded bytes; maximum is {maximum}")]
    ListenAddressTooLong {
        encoded_bytes: usize,
        maximum: usize,
    },
    #[error("Domain listeners did not all bind before the startup deadline")]
    ListenerStartupTimeout,
    #[error("Domain cleanup exceeded its 30-second deadline")]
    CleanupTimeout,
    #[error("Domain join failed ({join}) and rollback also failed ({rollback})")]
    JoinRollback {
        join: Box<AuthenticatedDomainError>,
        rollback: Box<DomainRollbackError>,
    },
    #[error("Domain protocol registration failed ({join}) and cleanup also failed ({cleanup})")]
    ProtocolRegistrationRollback {
        join: Box<AuthenticatedDomainError>,
        cleanup: Box<DomainProtocolError>,
    },
    #[error("Domain authority failed: {0}")]
    Authority(#[from] DomainAuthorityError),
    #[error(transparent)]
    Routes(#[from] DomainRoutesError),
    #[error("Domain protocol runtime failed: {0}")]
    Protocol(#[from] DomainProtocolError),
    #[error("Domain resource catalog runtime failed: {0}")]
    ResourcesV2(#[from] ResourcesV2Error),
    #[error("Domain participant info runtime failed: {0}")]
    InfoV1(#[from] InfoV1Error),
    #[error("Domain resource catalog v0.3 runtime failed: {0}")]
    ResourcesV3(#[from] ResourcesV3Error),
    #[error("Domain resource catalog v0.4 runtime failed: {0}")]
    ResourcesV4(#[from] ResourcesV4Error),
    #[error("Domain Registry runtime failed: {0}")]
    Registries(#[from] RegistriesError),
    #[error("Domain blob runtime failed: {0}")]
    Blobs(#[from] BlobsV1Error),
    #[error("Domain message runtime failed: {0}")]
    Messages(#[from] MessagesV1Error),
    #[error("Domain message-channel declaration failed: {0}")]
    MessageChannelRegistration(#[from] MessageChannelRegistrationError),
    #[error("Domain typed-stream runtime failed: {0}")]
    Streams(#[from] StreamsError),
    #[error("Domain registry/blob storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("authenticated P2P runtime failed: {0}")]
    P2p(#[source] auki_p2p::Error),
    #[error("Domain-owned task failed: {0}")]
    Task(#[source] tokio::task::JoinError),
    #[error("the Domain runtime is stopped")]
    Stopped,
}

#[derive(Debug, thiserror::Error)]
pub enum DomainRollbackError {
    #[error("rollback exceeded its 30-second cleanup deadline")]
    Timeout,
    #[error("rollback node shutdown failed: {0}")]
    P2p(#[source] auki_p2p::Error),
}

impl From<RuntimeAccessError> for AuthenticatedDomainError {
    fn from(_: RuntimeAccessError) -> Self {
        Self::Stopped
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::pending,
        str::FromStr,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use auki_p2p::{
        P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE, P2PAccessClaims,
        SignedApplicationMetadata,
    };
    use futures::io::{AsyncReadExt, AsyncWriteExt};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde::Serialize;
    use tokio::sync::oneshot;

    use super::*;
    use crate::authenticated_runtime::protocols::{DomainProtocolError, DomainProtocolSpec};
    use crate::resource_catalog::ResourceCatalogProvider;
    use auki_protocols::catalog::v2::{
        Available, Head, ID as RESOURCES_V0_2_0, ResourceEntry, ResourcesRequest, SensorBlock,
        SensorKind, SensorManifestPointer, VariantContent,
    };
    use auki_registry::RegistryRef;

    const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

    const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

    const ROTATED_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgwRbuxaM6rEI3vYEl
vRmIEsc1QtC3uPMWvXo1xXt+CcOhRANCAAQDFwBFAujMsiq78IWbq5vz0QSWEdc7
7h5NE8sDwgD6Js22t9Ztq84hhkS3Aad4m9FOi8evk5QYW7ef+Bc2oZsr
-----END PRIVATE KEY-----"#;

    const ROTATED_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEAxcARQLozLIqu/CFm6ub89EElhHX
O+4eTRPLA8IA+ibNtrfWbavOIYZEtwGneJvRTovHr5OUGFu3n/gXNqGbKw==
-----END PUBLIC KEY-----"#;

    const TEST_PROTOCOL: &str = "/auki/auth/1/runtime-test/1.0.0";

    fn identity(seed: u8) -> Identity {
        Identity::from_ed25519_seed(&[seed; 32])
    }

    fn keys() -> DdsVerificationKeys {
        DdsVerificationKeys::new(0, TEST_DDS_PUBLIC_KEY.to_vec(), None)
    }

    fn unix_time() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must follow the Unix epoch")
            .as_secs()
    }

    fn credential(peer_id: PeerId, domain_id: Uuid, issued_at: u64) -> SignedP2pCredential {
        credential_with_key(peer_id, domain_id, issued_at, TEST_DDS_PRIVATE_KEY)
    }

    fn credential_with_key(
        peer_id: PeerId,
        domain_id: Uuid,
        issued_at: u64,
        private_key: &[u8],
    ) -> SignedP2pCredential {
        let claims = P2PAccessClaims {
            token_type: P2P_TOKEN_TYPE.into(),
            iss: P2P_TOKEN_ISSUER.into(),
            aud: vec![P2P_TOKEN_AUDIENCE.into()],
            sub: Uuid::new_v4().to_string(),
            organization_id: None,
            peer_type: None,
            peer_id: peer_id.to_string(),
            domain_ids: vec![domain_id.to_string()],
            scopes: Vec::new(),
            application: Some(SignedApplicationMetadata {
                name: "runtime-test".into(),
                version: "1.0.0".into(),
            }),
            iat: issued_at,
            nbf: None,
            exp: issued_at + P2P_TOKEN_TTL.as_secs(),
        };
        SignedP2pCredential::new(sign(&claims, private_key)).unwrap()
    }

    fn sign(claims: &impl Serialize, private_key: &[u8]) -> String {
        encode(
            &Header::new(Algorithm::ES256),
            claims,
            &EncodingKey::from_ec_pem(private_key).unwrap(),
        )
        .unwrap()
    }

    struct CountingResourceProvider {
        resources: Vec<ResourceEntry>,
        calls: Arc<AtomicUsize>,
    }

    impl ResourceCatalogProvider for CountingResourceProvider {
        fn snapshot(&self) -> Vec<ResourceEntry> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.resources.clone()
        }
    }

    fn resource(
        source_peer_id: impl Into<String>,
        writer_peer_id: impl Into<String>,
        resource_id: impl Into<String>,
    ) -> ResourceEntry {
        let source_peer_id = source_peer_id.into();
        let resource_id = resource_id.into();
        ResourceEntry {
            source_peer_id: source_peer_id.clone(),
            writer_peer_id: writer_peer_id.into(),
            resource_id: resource_id.clone(),
            state: "live".into(),
            head: Some(Head::Rolling {
                retention_ns: 5_000_000_000,
            }),
            extent: None,
            available: Available {
                bytes: 1_024,
                entries: 10,
                duration_ns: 5_000_000_000,
            },
            sensor: Some(SensorBlock {
                kind: SensorKind::Camera,
                r#type: "rgb".into(),
                sensor_id: resource_id,
                sensor_hash: "sensor-hash".into(),
            }),
            pose: None,
            variant_content: VariantContent::SensorLog {
                manifest: SensorManifestPointer {
                    clock: RegistryRef {
                        peer_id: source_peer_id,
                        id: "clock".into(),
                        hash: "clock-hash".into(),
                    },
                    frame: None,
                },
            },
        }
    }

    fn tcp_port(address: &Multiaddr) -> u16 {
        address
            .iter()
            .find_map(|protocol| match protocol {
                auki_p2p::Protocol::Tcp(port) => Some(port),
                _ => None,
            })
            .expect("test listener must contain a TCP port")
    }

    async fn join_domain(config: AuthenticatedDomainConfig, issued_at: u64) -> AuthenticatedDomain {
        let credential = credential(config.peer_id(), config.domain_id(), issued_at);
        let services = AuthenticatedDomainServicesConfig::default()
            .with_served_protocols(ServedProtocols::none().with_resources_v2());
        AuthenticatedDomain::join_with_services(config, keys(), credential, services)
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zero_listener_and_zero_route_join_is_ready_and_leave_fences_clones() {
        let domain_id = Uuid::new_v4();
        let domain = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(1)),
            unix_time(),
        )
        .await;
        assert_eq!(domain.domain_id(), domain_id);
        assert_eq!(domain.status(), DomainStatus::Ready);
        assert!(domain.listen_addresses().is_empty());
        assert!(domain.routes().snapshot().unwrap().peers.is_empty());
        assert_eq!(domain.peers().peer_count(), 0);

        let authority = domain.authority();
        let routes = domain.routes();
        let protocols = domain.protocols();
        let peers = domain.peers();
        let mut status = domain.subscribe_status();
        tokio::time::timeout(Duration::from_secs(5), domain.leave())
            .await
            .expect("leave must remain bounded")
            .unwrap();

        status.changed().await.unwrap();
        assert_eq!(*status.borrow(), DomainStatus::Stopped);
        assert!(matches!(routes.snapshot(), Err(DomainRoutesError::Stopped)));
        assert_eq!(peers.peer_count(), 0);
        assert!(matches!(
            authority.peer_public_key_protobuf(),
            Err(DomainAuthorityError::Stopped)
        ));
        assert!(matches!(
            protocols.open(identity(9).peer_id(), TEST_PROTOCOL).await,
            Err(DomainProtocolError::Stopped
                | DomainProtocolError::Routes(DomainRoutesError::Stopped))
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn default_serves_nothing_and_omitted_exact_id_is_unsupported() {
        let domain_id = Uuid::new_v4();
        let server_identity = identity(20);
        let server_peer = server_identity.peer_id();
        let server_config = AuthenticatedDomainConfig::new(domain_id, server_identity)
            .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
            .unwrap();
        let server_credential = credential(server_peer, domain_id, unix_time());
        let server = AuthenticatedDomain::join(server_config, keys(), server_credential)
            .await
            .unwrap();
        assert!(server.served_protocol_ids().is_empty());

        let client_identity = identity(21);
        let client_peer = client_identity.peer_id();
        let client_config = AuthenticatedDomainConfig::new(domain_id, client_identity)
            .with_peer_routes(server_peer, [server.listen_addresses()[0].clone()])
            .unwrap();
        let client_credential = credential(client_peer, domain_id, unix_time());
        let client = AuthenticatedDomain::join(client_config, keys(), client_credential)
            .await
            .unwrap();
        assert!(client.served_protocol_ids().is_empty());

        let error = client
            .resources_v2()
            .fetch(server_peer, ResourcesRequest::all())
            .await
            .unwrap_err();
        match error {
            ResourcesV2Error::Protocol(DomainProtocolError::AllRoutesFailed {
                protocol_id,
                attempts,
                ..
            }) => {
                assert_eq!(protocol_id, RESOURCES_V0_2_0);
                assert!(!attempts.is_empty());
                assert!(attempts.iter().all(|attempt| attempt.unsupported_protocol));
            }
            other => panic!("omitted exact ID must negotiate as unsupported: {other}"),
        }

        client.leave().await.unwrap();
        server.leave().await.unwrap();
    }

    #[test]
    fn listener_configuration_is_bounded_before_node_start() {
        let config = AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(11));
        let addresses = (0..=DOMAIN_LISTENER_LIMIT)
            .map(|port| Multiaddr::from_str(&format!("/ip4/127.0.0.1/tcp/{}", port + 1)).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            config.with_listen_addresses(addresses),
            Err(AuthenticatedDomainError::ListenerLimit {
                count,
                maximum: DOMAIN_LISTENER_LIMIT,
            }) if count == DOMAIN_LISTENER_LIMIT + 1
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicate_registration_fails_and_raii_close_releases_the_protocol() {
        let domain = join_domain(
            AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(2)),
            unix_time(),
        )
        .await;
        let protocols = domain.protocols();
        let spec = DomainProtocolSpec::new(TEST_PROTOCOL, 1, 1_024).unwrap();
        let first = protocols
            .register(spec.clone(), |_| async {})
            .expect("first registration must succeed");
        assert!(matches!(
            protocols.register(spec.clone(), |_| async {}),
            Err(DomainProtocolError::DuplicateProtocol(protocol)) if protocol == TEST_PROTOCOL
        ));

        first.close().await;
        let replacement = protocols
            .register(spec, |_| async {})
            .expect("closed registration must release its protocol ID");
        replacement.close().await;
        domain.leave().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exact_route_mutual_auth_observes_both_peers_and_leave_aborts_active_handlers() {
        let domain_id = Uuid::new_v4();
        let server_identity = identity(3);
        let server_peer = server_identity.peer_id();
        let server = join_domain(
            AuthenticatedDomainConfig::new(domain_id, server_identity)
                .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
                .unwrap(),
            unix_time(),
        )
        .await;
        let server_route = server.listen_addresses()[0].clone();

        let (handler_started, handler_started_rx) = oneshot::channel();
        let handler_started = Arc::new(Mutex::new(Some(handler_started)));
        let registration = server
            .protocols()
            .register(
                DomainProtocolSpec::new(TEST_PROTOCOL, 2, 1_024).unwrap(),
                move |mut stream| {
                    let handler_started = Arc::clone(&handler_started);
                    async move {
                        let mut request = [0_u8; 4];
                        stream.read_exact(&mut request).await.unwrap();
                        assert_eq!(&request, b"ping");
                        stream.write_all(b"pong").await.unwrap();
                        stream.flush().await.unwrap();
                        if let Some(started) = handler_started.lock().take() {
                            let _ = started.send(());
                        }
                        pending::<()>().await;
                    }
                },
            )
            .unwrap();

        let client_identity = identity(4);
        let client_peer = client_identity.peer_id();
        let client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, client_identity)
                .with_peer_routes(server_peer, [server_route])
                .unwrap(),
            unix_time(),
        )
        .await;

        let mut stream = client
            .protocols()
            .open(server_peer, TEST_PROTOCOL)
            .await
            .unwrap();
        assert_eq!(stream.remote_peer().peer_id, server_peer);
        stream.write_all(b"ping").await.unwrap();
        stream.flush().await.unwrap();
        let mut response = [0_u8; 4];
        stream.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
        handler_started_rx.await.unwrap();

        assert_eq!(client.peers().snapshot().peers()[0].peer_id(), server_peer);
        assert_eq!(server.peers().snapshot().peers()[0].peer_id(), client_peer);
        assert_eq!(
            client.peers().snapshot().peers()[0]
                .application()
                .unwrap()
                .name,
            "runtime-test"
        );

        tokio::time::timeout(Duration::from_secs(5), server.leave())
            .await
            .expect("leave must cancel an active handler")
            .unwrap();
        drop(registration);
        drop(stream);
        client.leave().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resources_v2_bidirectional_same_domain_observes_and_leaves_cleanly() {
        tokio::time::timeout(Duration::from_secs(20), async {
            let domain_id = Uuid::new_v4();
            let issued_at = unix_time();
            let a_identity = identity(21);
            let a_peer = a_identity.peer_id();
            let a = join_domain(
                AuthenticatedDomainConfig::new(domain_id, a_identity)
                    .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
                    .unwrap(),
                issued_at,
            )
            .await;
            assert_eq!(a.status(), DomainStatus::Ready);
            assert!(a.routes().snapshot().unwrap().peers.is_empty());
            let a_address = a.listen_addresses()[0].clone();
            let a_port = tcp_port(&a_address);

            let b_identity = identity(22);
            let b_peer = b_identity.peer_id();
            let b = join_domain(
                AuthenticatedDomainConfig::new(domain_id, b_identity)
                    .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
                    .unwrap(),
                issued_at,
            )
            .await;
            let b_address = b.listen_addresses()[0].clone();
            let b_port = tcp_port(&b_address);
            a.routes().replace(b_peer, [b_address]).unwrap();
            b.routes().replace(a_peer, [a_address]).unwrap();

            let a_calls = Arc::new(AtomicUsize::new(0));
            let b_calls = Arc::new(AtomicUsize::new(0));
            let a_row = resource("materialized-source", a_peer.to_string(), "a-camera");
            let b_row = resource(b_peer.to_string(), b_peer.to_string(), "b-camera");
            let a_resources = a.resources_v2();
            let b_resources = b.resources_v2();
            a_resources
                .set_provider(Arc::new(CountingResourceProvider {
                    resources: vec![a_row.clone()],
                    calls: Arc::clone(&a_calls),
                }))
                .unwrap();
            b_resources
                .set_provider(Arc::new(CountingResourceProvider {
                    resources: vec![b_row.clone()],
                    calls: Arc::clone(&b_calls),
                }))
                .unwrap();

            let mut a_events = a.peers().subscribe();
            let mut b_events = b.peers().subscribe();
            let from_b = a_resources
                .fetch(b_peer, ResourcesRequest::all())
                .await
                .unwrap();
            assert_eq!(from_b.resources, vec![b_row]);
            assert!(matches!(
                a_events.recv().await.unwrap(),
                peers::KnownPeerEvent::Appeared(ref peer) if peer.peer_id() == b_peer
            ));
            assert!(matches!(
                b_events.recv().await.unwrap(),
                peers::KnownPeerEvent::Appeared(ref peer) if peer.peer_id() == a_peer
            ));

            let from_a = b_resources
                .fetch(a_peer, ResourcesRequest::all())
                .await
                .unwrap();
            assert_eq!(from_a.resources, vec![a_row]);
            assert_ne!(
                from_a.resources[0].source_peer_id, from_a.resources[0].writer_peer_id,
                "materialized resource ownership must remain byte-for-byte unchanged"
            );
            assert_eq!(a_calls.load(Ordering::SeqCst), 1);
            assert_eq!(b_calls.load(Ordering::SeqCst), 1);
            assert_eq!(a.peers().snapshot().peers()[0].peer_id(), b_peer);
            assert_eq!(b.peers().snapshot().peers()[0].peer_id(), a_peer);

            let a_routes = a.routes();
            let b_routes = b.routes();
            b.leave().await.unwrap();
            a.leave().await.unwrap();
            assert!(matches!(
                a_resources.set_provider(Arc::new(CountingResourceProvider {
                    resources: Vec::new(),
                    calls: Arc::new(AtomicUsize::new(0)),
                })),
                Err(resources_v2::ResourcesV2Error::Stopped)
            ));
            assert!(matches!(
                a_routes.snapshot(),
                Err(DomainRoutesError::Stopped)
            ));
            assert!(matches!(
                b_routes.snapshot(),
                Err(DomainRoutesError::Stopped)
            ));
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, a_port)).unwrap();
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, b_port)).unwrap();
        })
        .await
        .expect("bidirectional authenticated resource exchange must remain bounded");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resources_v2_authority_matrix_exposes_zero_catalog() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let domain_id = Uuid::new_v4();
            let issued_at = unix_time();
            let server_identity = identity(31);
            let server_peer = server_identity.peer_id();
            let server = join_domain(
                AuthenticatedDomainConfig::new(domain_id, server_identity)
                    .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
                    .unwrap(),
                issued_at,
            )
            .await;
            let server_address = server.listen_addresses()[0].clone();
            let calls = Arc::new(AtomicUsize::new(0));
            server
                .resources_v2()
                .set_provider(Arc::new(CountingResourceProvider {
                    resources: vec![resource(
                        server_peer.to_string(),
                        server_peer.to_string(),
                        "server-camera",
                    )],
                    calls: Arc::clone(&calls),
                }))
                .unwrap();

            let client = join_domain(
                AuthenticatedDomainConfig::new(domain_id, identity(32)),
                issued_at,
            )
            .await;
            let wrong_peer = identity(33).peer_id();
            client
                .routes()
                .replace(wrong_peer, [server_address.clone()])
                .unwrap();
            assert!(
                client
                    .resources_v2()
                    .fetch(wrong_peer, ResourcesRequest::all())
                    .await
                    .is_err()
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);

            let wrong_domain = join_domain(
                AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(34))
                    .with_peer_routes(server_peer, [server_address.clone()])
                    .unwrap(),
                issued_at,
            )
            .await;
            assert!(
                wrong_domain
                    .resources_v2()
                    .fetch(server_peer, ResourcesRequest::all())
                    .await
                    .is_err()
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);

            let expiring_identity = identity(35);
            let expiring = join_domain(
                AuthenticatedDomainConfig::new(domain_id, expiring_identity)
                    .with_peer_routes(server_peer, [server_address.clone()])
                    .unwrap(),
                issued_at - P2P_TOKEN_TTL.as_secs() + 2,
            )
            .await;
            let mut expiring_status = expiring.subscribe_status();
            while *expiring_status.borrow_and_update() != DomainStatus::CredentialUnavailable {
                expiring_status.changed().await.unwrap();
            }
            assert!(
                expiring
                    .resources_v2()
                    .fetch(server_peer, ResourcesRequest::all())
                    .await
                    .is_err()
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);

            let anonymous = Node::start(
                identity(36),
                DdsTokenVerifier::from_keys(keys()).unwrap(),
                [],
            )
            .unwrap();
            let anonymous_result = anonymous
                .open(
                    server_peer,
                    vec![server_address.clone()],
                    auki_p2p::ApplicationProtocol::new(RESOURCES_V0_2_0).unwrap(),
                    auki_p2p::SessionRequirements::new(domain_id.to_string())
                        .unwrap()
                        .with_expected_remote_peer_id(server_peer),
                )
                .await;
            assert!(anonymous_result.is_err());
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert_eq!(server.peers().peer_count(), 0);

            client
                .routes()
                .replace(server_peer, [server_address])
                .unwrap();
            let valid = client
                .resources_v2()
                .fetch(server_peer, ResourcesRequest::all())
                .await
                .unwrap();
            assert_eq!(valid.resources.len(), 1);
            assert_eq!(calls.load(Ordering::SeqCst), 1);

            anonymous.shutdown().await.unwrap();
            expiring.leave().await.unwrap();
            wrong_domain.leave().await.unwrap();
            client.leave().await.unwrap();
            server.leave().await.unwrap();
        })
        .await
        .expect("resource authorization matrix must remain bounded");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn legacy_resources_id_never_reaches_catalog() {
        use futures::StreamExt;
        use libp2p::{StreamProtocol, SwarmBuilder, noise, swarm::SwarmEvent, tcp, yamux};
        use libp2p_stream::{Behaviour as StreamBehaviour, OpenStreamError};

        tokio::time::timeout(Duration::from_secs(15), async {
            let domain_id = Uuid::new_v4();
            let server_identity = identity(37);
            let server_peer = server_identity.peer_id();
            let server = join_domain(
                AuthenticatedDomainConfig::new(domain_id, server_identity)
                    .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
                    .unwrap(),
                unix_time(),
            )
            .await;
            let calls = Arc::new(AtomicUsize::new(0));
            server
                .resources_v2()
                .set_provider(Arc::new(CountingResourceProvider {
                    resources: Vec::new(),
                    calls: Arc::clone(&calls),
                }))
                .unwrap();

            let streams = StreamBehaviour::new();
            let mut control = streams.new_control();
            let mut attacker = SwarmBuilder::with_new_identity()
                .with_tokio()
                .with_tcp(
                    tcp::Config::default(),
                    noise::Config::new,
                    yamux::Config::default,
                )
                .unwrap()
                .with_behaviour(|_| streams)
                .unwrap()
                .build();
            let address = server.listen_addresses()[0]
                .clone()
                .with(auki_p2p::Protocol::P2p(server_peer));
            attacker.dial(address).unwrap();
            let (connected_tx, connected_rx) = oneshot::channel();
            let driver = tokio::spawn(async move {
                let mut connected_tx = Some(connected_tx);
                while let Some(event) = attacker.next().await {
                    if matches!(
                        event,
                        SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == server_peer
                    ) && let Some(connected_tx) = connected_tx.take()
                    {
                        let _ = connected_tx.send(());
                    }
                }
            });
            connected_rx.await.unwrap();

            let legacy = "/auki/resources/0.2.0";
            let error = control
                .open_stream(server_peer, StreamProtocol::new(legacy))
                .await
                .expect_err("the authenticated Domain must not negotiate the legacy resource ID");
            assert!(
                matches!(error, OpenStreamError::UnsupportedProtocol(ref protocol) if protocol.as_ref() == legacy)
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert_eq!(server.peers().peer_count(), 0);

            driver.abort();
            let _ = driver.await;
            server.leave().await.unwrap();
        })
        .await
        .expect("legacy resource negotiation proof must remain bounded");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wrong_domain_transport_exposes_no_handler_or_known_peer() {
        let server_domain_id = Uuid::new_v4();
        let client_domain_id = Uuid::new_v4();
        let server_identity = identity(5);
        let server_peer = server_identity.peer_id();
        let server = join_domain(
            AuthenticatedDomainConfig::new(server_domain_id, server_identity)
                .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
                .unwrap(),
            unix_time(),
        )
        .await;
        let server_route = server.listen_addresses()[0].clone();
        let handler_ran = Arc::new(AtomicBool::new(false));
        let handler_flag = Arc::clone(&handler_ran);
        let registration = server
            .protocols()
            .register(
                DomainProtocolSpec::new(TEST_PROTOCOL, 1, 1_024).unwrap(),
                move |_| {
                    handler_flag.store(true, Ordering::SeqCst);
                    async {}
                },
            )
            .unwrap();

        let client_identity = identity(6);
        let client = join_domain(
            AuthenticatedDomainConfig::new(client_domain_id, client_identity)
                .with_peer_routes(server_peer, [server_route])
                .unwrap(),
            unix_time(),
        )
        .await;
        assert!(matches!(
            client.protocols().open(server_peer, TEST_PROTOCOL).await,
            Err(DomainProtocolError::AllRoutesFailed { .. })
        ));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!handler_ran.load(Ordering::SeqCst));
        assert_eq!(server.peers().peer_count(), 0);
        assert_eq!(client.peers().peer_count(), 0);

        registration.close().await;
        client.leave().await.unwrap();
        server.leave().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn authority_expiry_and_reinstallation_drive_local_readiness() {
        let now = unix_time();
        let domain_id = Uuid::new_v4();
        let local_identity = identity(7);
        let peer_id = local_identity.peer_id();
        let expiring_issued_at = now - P2P_TOKEN_TTL.as_secs() + 2;
        let domain = join_domain(
            AuthenticatedDomainConfig::new(domain_id, local_identity),
            expiring_issued_at,
        )
        .await;
        assert_eq!(domain.status(), DomainStatus::Ready);
        let mut status = domain.subscribe_status();
        tokio::time::timeout(Duration::from_secs(5), async {
            while *status.borrow_and_update() != DomainStatus::CredentialUnavailable {
                status.changed().await.unwrap();
            }
        })
        .await
        .expect("literal credential expiry must update readiness");

        let authority = domain.authority();
        authority
            .install_verification_keys(DdsVerificationKeys::new(
                1,
                ROTATED_DDS_PUBLIC_KEY.to_vec(),
                Some(TEST_DDS_PUBLIC_KEY.to_vec()),
            ))
            .await
            .unwrap();
        authority
            .install_credential(credential_with_key(
                peer_id,
                domain_id,
                now + 1,
                ROTATED_DDS_PRIVATE_KEY,
            ))
            .await
            .unwrap();
        assert_eq!(domain.status(), DomainStatus::Ready);
        domain.leave().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fatal_protocol_host_failure_is_supervised_and_fail_closed() {
        let domain = join_domain(
            AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(8)),
            unix_time(),
        )
        .await;
        let mut status = domain.subscribe_status();
        domain.fail_protocol_host_for_test();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if *status.borrow_and_update()
                    == DomainStatus::Failed(DomainFailure::ProtocolHostStopped)
                {
                    break;
                }
                status.changed().await.unwrap();
            }
        })
        .await
        .expect("fatal child failure must reach the Domain status");
        assert!(matches!(
            domain.routes().snapshot(),
            Err(DomainRoutesError::Stopped)
        ));
        domain.leave().await.unwrap();
        assert_eq!(
            *status.borrow(),
            DomainStatus::Failed(DomainFailure::ProtocolHostStopped)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cleanup_deadline_aborts_and_joins_every_owner_before_returning_error() {
        let domain = join_domain(
            AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(9))
                .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
                .unwrap(),
            unix_time(),
        )
        .await;
        let listen_port = domain.listen_addresses()[0]
            .iter()
            .find_map(|protocol| match protocol {
                auki_p2p::Protocol::Tcp(port) => Some(port),
                _ => None,
            })
            .unwrap();
        let routes = domain.routes();
        let authority = domain.authority();
        let status = domain.subscribe_status();
        let registration = domain.protocols().insert_stubborn_host_for_test();
        let mut domain = domain;
        let error = domain.leave_until(Instant::now()).await.unwrap_err();
        assert!(matches!(
            error,
            AuthenticatedDomainError::Protocol(DomainProtocolError::CleanupTimeout)
        ));
        assert_eq!(
            domain.status(),
            DomainStatus::Failed(DomainFailure::CleanupTimeout)
        );
        assert!(status.has_changed().unwrap());
        assert!(matches!(routes.snapshot(), Err(DomainRoutesError::Stopped)));
        assert!(matches!(
            authority.peer_public_key_protobuf(),
            Err(DomainAuthorityError::Stopped)
        ));
        tokio::time::timeout(Duration::from_secs(1), registration.close())
            .await
            .expect("forced cleanup must resolve stale registrations");
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, listen_port))
            .expect("forced cleanup must drop the swarm and release every listener before return");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn canceling_leave_preserves_task_ownership_for_drop_fallback() {
        let domain = join_domain(
            AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(11)),
            unix_time(),
        )
        .await;
        let authority = domain.authority();
        let mut status = domain.subscribe_status();
        let registration = domain.protocols().insert_stubborn_host_for_test();
        let mut leave = Box::pin(domain.leave());

        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut leave)
                .await
                .is_err(),
            "the stubborn host must keep graceful leave pending"
        );
        drop(leave);

        tokio::time::timeout(Duration::from_secs(1), status.changed())
            .await
            .expect("canceled leave must run the Domain Drop fence")
            .unwrap();
        assert_eq!(*status.borrow(), DomainStatus::Stopped);
        assert!(matches!(
            authority.peer_public_key_protobuf(),
            Err(DomainAuthorityError::Stopped)
        ));
        tokio::time::timeout(Duration::from_secs(1), registration.close())
            .await
            .expect("Drop must still own and abort the in-flight protocol host");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_domain_before_protocol_task_first_poll_completes_registration() {
        let domain = join_domain(
            AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(12)),
            unix_time(),
        )
        .await;
        let registration = domain
            .protocols()
            .register(
                DomainProtocolSpec::new(TEST_PROTOCOL, 1, 1_024).unwrap(),
                |_| async {},
            )
            .unwrap();

        // This current-thread test performs no await between spawn and Drop,
        // so the protocol host cannot have been polled yet.
        drop(domain);
        tokio::time::timeout(Duration::from_secs(1), registration.close())
            .await
            .expect("a pre-poll task abort must complete stale registrations");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drop_transitions_status_and_fences_cloned_handles_immediately() {
        let domain = join_domain(
            AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(10)),
            unix_time(),
        )
        .await;
        let authority = domain.authority();
        let mut status = domain.subscribe_status();

        drop(domain);

        tokio::time::timeout(Duration::from_secs(1), status.changed())
            .await
            .expect("Drop must publish a terminal status")
            .unwrap();
        assert_eq!(*status.borrow(), DomainStatus::Stopped);
        assert!(matches!(
            authority.peer_public_key_protobuf(),
            Err(DomainAuthorityError::Stopped)
        ));
    }
}
