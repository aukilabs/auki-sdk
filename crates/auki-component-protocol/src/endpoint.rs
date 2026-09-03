//! AukiPeer registration, authenticated dispatch, and portable clients.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
#[cfg(target_arch = "wasm32")]
use std::time::Instant;

use auki_components::{
    BufferLimits, ComponentReference, ComponentRuntime, Envelope, Exposure, FiniteObservations,
    Invocation, InvocationContext, InvocationError, Observation, Operable, ProductManifest,
    ProductReference, RetainedProduct,
};
#[cfg(not(target_arch = "wasm32"))]
use auki_components::{InvocationOptions, SharedDispatcher, SharedScheduler};
use auki_sdk::{
    AukiPeerProtocols, AukiProtocolError, AukiProtocolRegistration, AukiProtocolSpec,
    AukiProtocolStream, AuthenticatedRouteStream, Multiaddr, PeerId,
};
use futures::future::BoxFuture;
use futures::{AsyncWriteExt, FutureExt, pin_mut};
use futures_timer::Delay;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::wire::{
    CATALOG_PROTOCOL_ID, CatalogRequest, CatalogResponse, MAX_BATCH_OBSERVATIONS,
    MAX_CONTROL_FRAME_BYTES, MAX_OPERATION_DEADLINE_MS, MAX_PAYLOAD_FRAME_BYTES,
    OBSERVATIONS_PROTOCOL_ID, OPERATIONS_PROTOCOL_ID, ObservationBatchHeader,
    ObservationRecordHeader, ObservationRequest, ObservationSelection, OperationRequest,
    OperationResponse, RemoteOperationError, SourceGap, WireError, read_json, read_payload,
    write_json, write_payload,
};

const MAX_CONCURRENCY: usize = 32;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(target_arch = "wasm32"))]
const OPERATION_WORKERS: usize = 4;

#[derive(Clone)]
struct EncodedObservation {
    header: ObservationRecordHeader,
    payload: Vec<u8>,
}

struct EncodedObservationBatch {
    product: ProductManifest,
    product_manifest_hash: String,
    producer: auki_components::OutputManifest,
    observations: Vec<EncodedObservation>,
    gap: Option<SourceGap>,
}

type ProductHandler =
    dyn Fn(ObservationSelection) -> Result<EncodedObservationBatch, ServiceError> + Send + Sync;

struct EncodedOperationResult {
    invocation_id: String,
    payload: Vec<u8>,
}

type OperationHandler = dyn Fn(
        InvocationContext,
        Option<Duration>,
        Vec<u8>,
    ) -> BoxFuture<'static, Result<EncodedOperationResult, ServiceError>>
    + Send
    + Sync;

struct ServiceState {
    runtime: ComponentRuntime,
    catalog_gate: RwLock<()>,
    products: RwLock<BTreeMap<String, Arc<ProductHandler>>>,
    operations: RwLock<BTreeMap<(String, String), Arc<OperationHandler>>>,
    export_revision: AtomicU64,
    catalog_projection: Mutex<CatalogProjection>,
    #[cfg(not(target_arch = "wasm32"))]
    _scheduler: Arc<SharedScheduler>,
    #[cfg(not(target_arch = "wasm32"))]
    dispatcher: SharedDispatcher,
}

#[derive(Default)]
struct CatalogProjection {
    last_source_revisions: Option<(u64, u64)>,
    revision: u64,
}

/// Mounted Catalog, observation, and operation services for one Component runtime.
pub struct ComponentProtocolEndpoint {
    client: ComponentProtocolClient,
    state: Arc<ServiceState>,
    catalog_registration: AukiProtocolRegistration,
    observation_registration: AukiProtocolRegistration,
    operation_registration: AukiProtocolRegistration,
}

impl ComponentProtocolEndpoint {
    /// Mount all three exact Component protocols on one running Auki peer.
    pub fn mount(
        protocols: AukiPeerProtocols,
        runtime: ComponentRuntime,
    ) -> Result<Self, ComponentProtocolError> {
        let protocol_peer_id = protocols.peer_id().to_string();
        if runtime.peer_id() != protocol_peer_id {
            return Err(ComponentProtocolError::RuntimePeerMismatch {
                runtime_peer_id: runtime.peer_id().to_owned(),
                protocol_peer_id,
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        let scheduler = Arc::new(
            SharedScheduler::new(OPERATION_WORKERS)
                .map_err(|error| ComponentProtocolError::Export(error.to_string()))?,
        );
        let state = Arc::new(ServiceState {
            runtime,
            catalog_gate: RwLock::new(()),
            products: RwLock::new(BTreeMap::new()),
            operations: RwLock::new(BTreeMap::new()),
            export_revision: AtomicU64::new(0),
            catalog_projection: Mutex::new(CatalogProjection::default()),
            #[cfg(not(target_arch = "wasm32"))]
            dispatcher: scheduler.dispatcher(),
            #[cfg(not(target_arch = "wasm32"))]
            _scheduler: scheduler,
        });

        let catalog_state = Arc::clone(&state);
        let catalog_registration = protocols.register(catalog_spec()?, move |mut stream| {
            let state = Arc::clone(&catalog_state);
            async move {
                let _ = deadline(
                    ComponentProtocolOperation::Exchange,
                    serve_catalog(&mut stream, &state),
                    NETWORK_TIMEOUT,
                )
                .await;
                let _ = stream.close().await;
            }
        })?;

        let observation_state = Arc::clone(&state);
        let observation_registration =
            protocols.register(observations_spec()?, move |mut stream| {
                let state = Arc::clone(&observation_state);
                async move {
                    let _ = deadline(
                        ComponentProtocolOperation::Exchange,
                        serve_observations(&mut stream, &state),
                        NETWORK_TIMEOUT,
                    )
                    .await;
                    let _ = stream.close().await;
                }
            })?;

        let operation_state = Arc::clone(&state);
        let operation_registration =
            protocols.register(operations_spec()?, move |mut stream| {
                let state = Arc::clone(&operation_state);
                async move {
                    let _ = deadline(
                        ComponentProtocolOperation::Exchange,
                        serve_operation(&mut stream, &state),
                        Duration::from_millis(MAX_OPERATION_DEADLINE_MS + 5_000),
                    )
                    .await;
                    let _ = stream.close().await;
                }
            })?;

        Ok(Self {
            client: ComponentProtocolClient { protocols },
            state,
            catalog_registration,
            observation_registration,
            operation_registration,
        })
    }

    pub fn client(&self) -> ComponentProtocolClient {
        self.client.clone()
    }

    /// Make one exact retained Buffer Product available to authenticated peers.
    pub fn export_product<T>(
        &self,
        product: &RetainedProduct<T>,
    ) -> Result<(), ComponentProtocolError>
    where
        T: Serialize + Send + Sync + 'static,
    {
        let reference = product.reference();
        self.validate_product_export(&reference)?;
        let exported = product.clone();
        let handler: Arc<ProductHandler> = Arc::new(move |selection| {
            let (observations, gap) = select_observations(&exported, selection)?;
            let observations = observations
                .into_iter()
                .map(encode_observation)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(EncodedObservationBatch {
                product: exported.manifest.clone(),
                product_manifest_hash: exported.manifest_hash.clone(),
                producer: exported.producer.clone(),
                observations,
                gap,
            })
        });
        let _catalog_guard = self.state.catalog_gate.write().unwrap();
        let mut products = self.state.products.write().unwrap();
        if products.contains_key(&reference.product_id) {
            return Err(ComponentProtocolError::DuplicateExport(format!(
                "Product {}",
                reference.product_id
            )));
        }
        products.insert(reference.product_id, handler);
        self.state.export_revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    pub fn unexport_product(&self, product_id: &str) -> bool {
        let _catalog_guard = self.state.catalog_gate.write().unwrap();
        let removed = self
            .state
            .products
            .write()
            .unwrap()
            .remove(product_id)
            .is_some();
        if removed {
            self.state.export_revision.fetch_add(1, Ordering::AcqRel);
        }
        removed
    }

    /// Make one typed Operable available through authenticated invocation.
    pub fn export_operable<I, R>(
        &self,
        operable: &Operable<I, R>,
    ) -> Result<(), ComponentProtocolError>
    where
        I: DeserializeOwned + Send + 'static,
        R: Clone + Serialize + Send + 'static,
    {
        self.validate_operation_export(operable.owner(), operable.name(), operable.exposure())?;
        let target = operable.clone();
        #[cfg(not(target_arch = "wasm32"))]
        let dispatcher = self.state.dispatcher.clone();
        #[cfg(not(target_arch = "wasm32"))]
        let handler: Arc<OperationHandler> = Arc::new(move |context, deadline_after, payload| {
            let target = target.clone();
            let dispatcher = dispatcher.clone();
            async move {
                let instruction = serde_json::from_slice::<I>(&payload)
                    .map_err(|error| ServiceError::new("invalid_instruction", error.to_string()))?;
                let handle = target
                    .invoke_async(
                        context,
                        instruction,
                        &dispatcher,
                        InvocationOptions { deadline_after },
                    )
                    .map_err(ServiceError::invocation)?;
                loop {
                    if let Some(outcome) = handle.wait_timeout(Duration::ZERO) {
                        let invocation = outcome.map_err(ServiceError::invocation)?;
                        let payload = serde_json::to_vec(&invocation.result).map_err(|error| {
                            ServiceError::new("result_encoding_failed", error.to_string())
                        })?;
                        ensure_payload_bound(payload.len())?;
                        return Ok(EncodedOperationResult {
                            invocation_id: invocation.invocation_id,
                            payload,
                        });
                    }
                    Delay::new(Duration::from_millis(1)).await;
                }
            }
            .boxed()
        });
        #[cfg(target_arch = "wasm32")]
        let handler: Arc<OperationHandler> = Arc::new(move |context, deadline_after, payload| {
            let target = target.clone();
            async move {
                let instruction = serde_json::from_slice::<I>(&payload)
                    .map_err(|error| ServiceError::new("invalid_instruction", error.to_string()))?;
                let started = Instant::now();
                let invocation = target
                    .invoke(context, instruction)
                    .map_err(ServiceError::invocation)?;
                if deadline_after.is_some_and(|deadline| started.elapsed() >= deadline) {
                    return Err(ServiceError::invocation(InvocationError::DeadlineExceeded));
                }
                let payload = serde_json::to_vec(&invocation.result).map_err(|error| {
                    ServiceError::new("result_encoding_failed", error.to_string())
                })?;
                ensure_payload_bound(payload.len())?;
                Ok(EncodedOperationResult {
                    invocation_id: invocation.invocation_id,
                    payload,
                })
            }
            .boxed()
        });
        let _catalog_guard = self.state.catalog_gate.write().unwrap();
        let key = (
            operable.owner().component_id.clone(),
            operable.name().to_owned(),
        );
        let mut operations = self.state.operations.write().unwrap();
        if operations.contains_key(&key) {
            return Err(ComponentProtocolError::DuplicateExport(format!(
                "Operable {}.{}",
                key.0, key.1
            )));
        }
        operations.insert(key, handler);
        self.state.export_revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    pub fn unexport_operable(&self, component_id: &str, operable: &str) -> bool {
        let _catalog_guard = self.state.catalog_gate.write().unwrap();
        let removed = self
            .state
            .operations
            .write()
            .unwrap()
            .remove(&(component_id.to_owned(), operable.to_owned()))
            .is_some();
        if removed {
            self.state.export_revision.fetch_add(1, Ordering::AcqRel);
        }
        removed
    }

    pub async fn close(self) -> Result<(), ComponentProtocolError> {
        self.catalog_registration.close().await?;
        self.observation_registration.close().await?;
        self.operation_registration.close().await?;
        Ok(())
    }

    fn validate_product_export(
        &self,
        reference: &ProductReference,
    ) -> Result<(), ComponentProtocolError> {
        if reference.peer_id != self.state.runtime.peer_id() {
            return Err(ComponentProtocolError::Export(format!(
                "Product {} belongs to peer {}, not {}",
                reference.product_id,
                reference.peer_id,
                self.state.runtime.peer_id()
            )));
        }
        let catalog = self
            .state
            .runtime
            .catalog()
            .product(&reference.product_id)
            .ok_or_else(|| {
                ComponentProtocolError::Export(format!(
                    "Product {} is not in the live Catalog",
                    reference.product_id
                ))
            })?;
        if catalog.manifest_hash != reference.manifest_hash {
            return Err(ComponentProtocolError::Export(format!(
                "Product {} Manifest is not the current Catalog entry",
                reference.product_id
            )));
        }
        Ok(())
    }

    fn validate_operation_export(
        &self,
        owner: &ComponentReference,
        name: &str,
        exposure: Exposure,
    ) -> Result<(), ComponentProtocolError> {
        if owner.peer_id != self.state.runtime.peer_id() || exposure != Exposure::Cluster {
            return Err(ComponentProtocolError::Export(format!(
                "Operable {}.{} is not cluster-exposed by this peer",
                owner.component_id, name
            )));
        }
        let component = self
            .state
            .runtime
            .catalog()
            .component(&owner.component_id)
            .ok_or_else(|| {
                ComponentProtocolError::Export(format!(
                    "Component {} is not in the live Catalog",
                    owner.component_id
                ))
            })?;
        if component.manifest_hash != owner.manifest_hash
            || !component
                .manifest
                .operables
                .iter()
                .any(|contract| contract.name == name && contract.exposure == Exposure::Cluster)
        {
            return Err(ComponentProtocolError::Export(format!(
                "Operable {}.{} does not match the live Component Manifest",
                owner.component_id, name
            )));
        }
        Ok(())
    }
}

/// Cloneable outbound client for the Component protocol family.
#[derive(Clone)]
pub struct ComponentProtocolClient {
    protocols: AukiPeerProtocols,
}

impl ComponentProtocolClient {
    /// Bind an outbound client to one running Auki peer without mounting any
    /// inbound Component protocols on that peer.
    pub fn new(protocols: AukiPeerProtocols) -> Self {
        Self { protocols }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn catalog(
        &self,
        remote_peer_id: PeerId,
        known_revision: Option<u64>,
    ) -> Result<CatalogResponse, ComponentProtocolError> {
        catalog_opened(
            CatalogRequest { known_revision },
            self.protocols.open(remote_peer_id, CATALOG_PROTOCOL_ID),
        )
        .await
    }

    pub async fn catalog_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        known_revision: Option<u64>,
    ) -> Result<CatalogResponse, ComponentProtocolError> {
        catalog_opened(
            CatalogRequest { known_revision },
            self.protocols
                .open_exact(remote_peer_id, route, CATALOG_PROTOCOL_ID),
        )
        .await
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn observations<T>(
        &self,
        remote_peer_id: PeerId,
        request: ObservationRequest,
    ) -> Result<RemoteObservations<T>, ComponentProtocolError>
    where
        T: DeserializeOwned,
    {
        observations_opened(
            request,
            self.protocols
                .open(remote_peer_id, OBSERVATIONS_PROTOCOL_ID),
        )
        .await
    }

    pub async fn observations_exact<T>(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        request: ObservationRequest,
    ) -> Result<RemoteObservations<T>, ComponentProtocolError>
    where
        T: DeserializeOwned,
    {
        observations_opened(
            request,
            self.protocols
                .open_exact(remote_peer_id, route, OBSERVATIONS_PROTOCOL_ID),
        )
        .await
    }

    /// Create a typed local mirror fed from a remote Buffer Product.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn mirror_product<T>(
        &self,
        remote_peer_id: PeerId,
        product: ProductReference,
        limits: BufferLimits,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<RemoteProductMirror<T>, ComponentProtocolError>
    where
        T: DeserializeOwned + Send + Sync + 'static,
    {
        let initial = self
            .observations(
                remote_peer_id,
                ObservationRequest {
                    product,
                    selection: ObservationSelection::FromSequence {
                        sequence: 0,
                        max_observations: 1,
                    },
                },
            )
            .await?;
        RemoteProductMirror::from_initial(
            self.clone(),
            RemoteRoute::Configured(remote_peer_id),
            initial,
            limits,
            retained_size,
        )
    }

    /// Create a portable typed mirror through one exact advertised route.
    pub async fn mirror_product_exact<T>(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        product: ProductReference,
        limits: BufferLimits,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<RemoteProductMirror<T>, ComponentProtocolError>
    where
        T: DeserializeOwned + Send + Sync + 'static,
    {
        let initial = self
            .observations_exact(
                remote_peer_id,
                route.clone(),
                ObservationRequest {
                    product,
                    selection: ObservationSelection::FromSequence {
                        sequence: 0,
                        max_observations: 1,
                    },
                },
            )
            .await?;
        RemoteProductMirror::from_initial(
            self.clone(),
            RemoteRoute::Exact {
                peer_id: remote_peer_id,
                route,
            },
            initial,
            limits,
            retained_size,
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn invoke<I, R>(
        &self,
        remote_peer_id: PeerId,
        target_component: ComponentReference,
        operable: impl Into<String>,
        caller_component_id: impl Into<String>,
        invocation_id: impl Into<String>,
        deadline_after: Option<Duration>,
        instruction: &I,
    ) -> Result<Invocation<R>, ComponentProtocolError>
    where
        I: Serialize,
        R: DeserializeOwned,
    {
        let prepared = prepare_operation(
            target_component,
            operable.into(),
            caller_component_id.into(),
            invocation_id.into(),
            deadline_after,
            instruction,
        )?;
        operation_opened(
            prepared,
            self.protocols.open(remote_peer_id, OPERATIONS_PROTOCOL_ID),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn invoke_exact<I, R>(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        target_component: ComponentReference,
        operable: impl Into<String>,
        caller_component_id: impl Into<String>,
        invocation_id: impl Into<String>,
        deadline_after: Option<Duration>,
        instruction: &I,
    ) -> Result<Invocation<R>, ComponentProtocolError>
    where
        I: Serialize,
        R: DeserializeOwned,
    {
        let prepared = prepare_operation(
            target_component,
            operable.into(),
            caller_component_id.into(),
            invocation_id.into(),
            deadline_after,
            instruction,
        )?;
        operation_opened(
            prepared,
            self.protocols
                .open_exact(remote_peer_id, route, OPERATIONS_PROTOCOL_ID),
        )
        .await
    }
}

#[derive(Clone, Debug)]
pub struct RemoteObservations<T> {
    pub product: ProductManifest,
    pub product_manifest_hash: String,
    pub producer: auki_components::OutputManifest,
    pub observations: Vec<Observation<T>>,
    pub gap: Option<SourceGap>,
}

#[derive(Clone)]
enum RemoteRoute {
    #[cfg(not(target_arch = "wasm32"))]
    Configured(PeerId),
    Exact {
        peer_id: PeerId,
        route: Multiaddr,
    },
}

/// A typed local Buffer whose observations retain their remote source identity.
///
/// Call [`RemoteProductMirror::sync_once`] from the host's normal async task.
/// A Component consumes `product()` through the same `configured_buffer_input`
/// API used for a local Product; only the producer and Product peer are remote.
pub struct RemoteProductMirror<T> {
    client: ComponentProtocolClient,
    route: RemoteRoute,
    product: RetainedProduct<T>,
    next_sequence: u64,
    batch_size: u32,
    last_gap: Option<SourceGap>,
}

impl<T> RemoteProductMirror<T>
where
    T: DeserializeOwned + Send + Sync + 'static,
{
    fn from_initial(
        client: ComponentProtocolClient,
        route: RemoteRoute,
        initial: RemoteObservations<T>,
        limits: BufferLimits,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<Self, ComponentProtocolError> {
        let product = RetainedProduct::imported_buffer(
            initial.product,
            initial.product_manifest_hash,
            initial.producer,
            limits,
            retained_size,
        )
        .map_err(|error| ComponentProtocolError::Import(error.to_string()))?;
        let mut mirror = Self {
            client,
            route,
            product,
            next_sequence: 0,
            batch_size: 256,
            last_gap: None,
        };
        mirror.ingest(initial.observations, initial.gap)?;
        Ok(mirror)
    }

    pub fn product(&self) -> &RetainedProduct<T> {
        &self.product
    }

    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    pub fn last_gap(&self) -> Option<SourceGap> {
        self.last_gap
    }

    pub fn with_batch_size(mut self, batch_size: u32) -> Result<Self, ComponentProtocolError> {
        if batch_size == 0 || batch_size > MAX_BATCH_OBSERVATIONS {
            return Err(ComponentProtocolError::InvalidRequest(format!(
                "mirror batch size must be within 1..={MAX_BATCH_OBSERVATIONS}"
            )));
        }
        self.batch_size = batch_size;
        Ok(self)
    }

    /// Fetch and append one bounded batch, preserving source sequence and time.
    pub async fn sync_once(&mut self) -> Result<RemoteProductSync, ComponentProtocolError> {
        let request = ObservationRequest {
            product: self.product.reference(),
            selection: ObservationSelection::FromSequence {
                sequence: self.next_sequence,
                max_observations: self.batch_size,
            },
        };
        let batch = match &self.route {
            #[cfg(not(target_arch = "wasm32"))]
            RemoteRoute::Configured(peer_id) => self.client.observations(*peer_id, request).await?,
            RemoteRoute::Exact { peer_id, route } => {
                self.client
                    .observations_exact(*peer_id, route.clone(), request)
                    .await?
            }
        };
        let gap = batch.gap;
        let accepted = batch.observations.len();
        self.ingest(batch.observations, gap)?;
        Ok(RemoteProductSync {
            accepted,
            gap,
            next_sequence: self.next_sequence,
        })
    }

    pub fn close(&self) {
        self.product.buffer().close();
    }

    fn ingest(
        &mut self,
        observations: Vec<Observation<T>>,
        gap: Option<SourceGap>,
    ) -> Result<(), ComponentProtocolError> {
        self.last_gap = gap;
        if let Some(gap) = gap {
            self.next_sequence = gap.available_from;
        }
        for observation in observations {
            self.next_sequence = observation.sequence.saturating_add(1);
            self.product
                .buffer()
                .append_shared(Arc::new(Envelope::new(
                    observation.sequence,
                    observation.timestamp_ns,
                    observation,
                )))
                .map_err(|error| ComponentProtocolError::Import(error.to_string()))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteProductSync {
    pub accepted: usize,
    pub gap: Option<SourceGap>,
    pub next_sequence: u64,
}

async fn serve_catalog<S>(stream: &mut S, state: &ServiceState) -> Result<(), ServiceError>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    let request: CatalogRequest = read_json(stream).await?;
    let snapshot = network_catalog_snapshot(state);
    let response = if request.known_revision == Some(snapshot.revision) {
        CatalogResponse::Unchanged {
            revision: snapshot.revision,
        }
    } else {
        CatalogResponse::Snapshot { snapshot }
    };
    write_json(stream, &response).await?;
    Ok(())
}

fn network_catalog_snapshot(state: &ServiceState) -> auki_components::CatalogSnapshot {
    let _catalog_guard = state.catalog_gate.read().unwrap();
    let mut snapshot = state.runtime.catalog().snapshot();
    let exported_products = state
        .products
        .read()
        .unwrap()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let exported_operations = state
        .operations
        .read()
        .unwrap()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    snapshot
        .products
        .retain(|product| exported_products.contains(&product.manifest.product_id));

    let mut exported_components = exported_operations
        .iter()
        .map(|(component_id, _)| component_id.clone())
        .collect::<BTreeSet<_>>();
    exported_components.extend(
        snapshot
            .products
            .iter()
            .map(|product| product.manifest.producer.component_id.clone()),
    );
    snapshot
        .components
        .retain(|component| exported_components.contains(&component.manifest.component_id));

    let export_revision = state.export_revision.load(Ordering::Acquire);
    let mut projection = state.catalog_projection.lock().unwrap();
    let source_revisions = (snapshot.revision, export_revision);
    if projection.last_source_revisions != Some(source_revisions) {
        projection.revision = projection.revision.saturating_add(1);
        projection.last_source_revisions = Some(source_revisions);
    }
    snapshot.revision = projection.revision;
    snapshot
}

async fn serve_observations<S>(stream: &mut S, state: &ServiceState) -> Result<(), ServiceError>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    let request: ObservationRequest = read_json(stream).await?;
    if request.product.peer_id != state.runtime.peer_id() {
        return write_observation_rejection(
            stream,
            "wrong_peer",
            "Product belongs to another peer",
        )
        .await;
    }
    let handler = state
        .products
        .read()
        .unwrap()
        .get(&request.product.product_id)
        .cloned();
    let Some(handler) = handler else {
        return write_observation_rejection(stream, "unknown_product", "Product is not exported")
            .await;
    };
    if state
        .runtime
        .catalog()
        .product(&request.product.product_id)
        .is_none_or(|entry| entry.manifest_hash != request.product.manifest_hash)
    {
        return write_observation_rejection(
            stream,
            "product_not_current",
            "Product is deleted or its Manifest is no longer current",
        )
        .await;
    }
    let batch = match handler(request.selection) {
        Ok(batch) => batch,
        Err(error) => {
            return write_observation_rejection(stream, &error.code, &error.message).await;
        }
    };
    if batch.product_manifest_hash != request.product.manifest_hash {
        return write_observation_rejection(
            stream,
            "manifest_mismatch",
            "requested Product Manifest is no longer current",
        )
        .await;
    }
    let count = u32::try_from(batch.observations.len())
        .map_err(|_| ServiceError::new("batch_too_large", "observation count exceeds u32"))?;
    write_json(
        stream,
        &ObservationBatchHeader::Accepted {
            product: Box::new(batch.product),
            product_manifest_hash: batch.product_manifest_hash,
            producer: Box::new(batch.producer),
            observations: count,
            gap: batch.gap,
        },
    )
    .await?;
    for observation in batch.observations {
        write_json(stream, &observation.header).await?;
        write_payload(stream, &observation.payload).await?;
    }
    Ok(())
}

async fn write_observation_rejection<S>(
    stream: &mut S,
    code: &str,
    message: &str,
) -> Result<(), ServiceError>
where
    S: futures::AsyncWrite + Unpin,
{
    write_json(
        stream,
        &ObservationBatchHeader::Rejected {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    )
    .await?;
    Ok(())
}

async fn serve_operation(
    stream: &mut AukiProtocolStream,
    state: &ServiceState,
) -> Result<(), ServiceError> {
    let request: OperationRequest = read_json(stream).await?;
    if let Err(error) = validate_operation_request(&request, state.runtime.peer_id()) {
        write_operation_failure(stream, &request.invocation_id, error).await?;
        return Ok(());
    }
    let instruction = read_payload(stream, request.instruction_bytes).await?;
    let key = (
        request.target_component.component_id.clone(),
        request.operable.clone(),
    );
    let handler = state.operations.read().unwrap().get(&key).cloned();
    let result = if let Some(handler) = handler {
        if state
            .runtime
            .catalog()
            .component(&request.target_component.component_id)
            .is_some_and(|entry| entry.manifest_hash == request.target_component.manifest_hash)
        {
            let context = InvocationContext {
                invocation_id: request.invocation_id.clone(),
                caller_peer_id: stream.remote_peer().peer_id.to_string(),
                caller_component_id: request.caller_component_id,
            };
            handler(
                context,
                request.deadline_ms.map(Duration::from_millis),
                instruction,
            )
            .await
        } else {
            Err(ServiceError::new(
                "manifest_mismatch",
                "target Component Manifest is no longer current",
            ))
        }
    } else {
        Err(ServiceError::new(
            "unknown_operable",
            "Operable is not exported",
        ))
    };

    match result {
        Ok(result) => {
            let bytes = u32::try_from(result.payload.len())
                .expect("payload bound is smaller than u32::MAX");
            write_json(
                stream,
                &OperationResponse::Completed {
                    invocation_id: result.invocation_id,
                    result_encoding: "application/json".to_owned(),
                    result_bytes: bytes,
                },
            )
            .await?;
            write_payload(stream, &result.payload).await?;
        }
        Err(error) => {
            write_json(
                stream,
                &OperationResponse::Failed {
                    invocation_id: request.invocation_id,
                    error: RemoteOperationError {
                        code: error.code,
                        message: error.message,
                    },
                },
            )
            .await?;
        }
    }
    Ok(())
}

async fn write_operation_failure<S>(
    stream: &mut S,
    invocation_id: &str,
    error: ServiceError,
) -> Result<(), ServiceError>
where
    S: futures::AsyncWrite + Unpin,
{
    write_json(
        stream,
        &OperationResponse::Failed {
            invocation_id: invocation_id.to_owned(),
            error: RemoteOperationError {
                code: error.code,
                message: error.message,
            },
        },
    )
    .await?;
    Ok(())
}

fn select_observations<T>(
    product: &RetainedProduct<T>,
    selection: ObservationSelection,
) -> Result<(Vec<Observation<T>>, Option<SourceGap>), ServiceError>
where
    T: Send + Sync + 'static,
{
    match selection {
        ObservationSelection::LatestExisting => Ok((
            product
                .latest_existing()
                .map_err(ServiceError::product)?
                .into_iter()
                .collect(),
            None,
        )),
        ObservationSelection::TimeRange { request } => {
            let FiniteObservations { observations } =
                product.time_range(request).map_err(ServiceError::product)?;
            if observations.len() > MAX_BATCH_OBSERVATIONS as usize {
                return Err(ServiceError::new(
                    "batch_too_large",
                    format!(
                        "query selected {} observations; maximum is {}",
                        observations.len(),
                        MAX_BATCH_OBSERVATIONS
                    ),
                ));
            }
            Ok((observations, None))
        }
        ObservationSelection::FromSequence {
            sequence,
            max_observations,
        } => {
            if max_observations == 0 || max_observations > MAX_BATCH_OBSERVATIONS {
                return Err(ServiceError::new(
                    "invalid_batch_limit",
                    format!("max_observations must be within 1..={MAX_BATCH_OBSERVATIONS}"),
                ));
            }
            let range = product.buffer().range();
            let Some(first) = range.first_sequence else {
                return Ok((Vec::new(), None));
            };
            let gap = (sequence < first).then_some(SourceGap {
                requested_sequence: sequence,
                available_from: first,
            });
            let start = sequence.max(first);
            let Some(last_available) = range.last_sequence else {
                return Ok((Vec::new(), gap));
            };
            if start > last_available {
                return Ok((Vec::new(), gap));
            }
            let last = start
                .saturating_add(u64::from(max_observations).saturating_sub(1))
                .min(last_available);
            let observations = product
                .buffer()
                .snapshot(start, last)
                .into_iter()
                .map(|envelope| envelope.payload.clone())
                .collect();
            Ok((observations, gap))
        }
    }
}

fn encode_observation<T: Serialize>(
    observation: Observation<T>,
) -> Result<EncodedObservation, ServiceError> {
    let payload = serde_json::to_vec(&*observation.payload)
        .map_err(|error| ServiceError::new("payload_encoding_failed", error.to_string()))?;
    ensure_payload_bound(payload.len())?;
    Ok(EncodedObservation {
        header: ObservationRecordHeader {
            output: observation.output,
            sequence: observation.sequence,
            timestamp_ns: observation.timestamp_ns,
            payload_encoding: "application/json".to_owned(),
            payload_bytes: u32::try_from(payload.len())
                .expect("payload bound is smaller than u32::MAX"),
        },
        payload,
    })
}

async fn catalog_opened<F>(
    request: CatalogRequest,
    opening: F,
) -> Result<CatalogResponse, ComponentProtocolError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
{
    let mut stream = deadline(ComponentProtocolOperation::Open, opening, NETWORK_TIMEOUT).await??;
    let exchange = deadline(
        ComponentProtocolOperation::Exchange,
        async {
            write_json(&mut stream, &request).await?;
            read_json(&mut stream).await
        },
        NETWORK_TIMEOUT,
    )
    .await
    .and_then(|result| result.map_err(ComponentProtocolError::from_wire));
    let cleanup = close_stream(&mut stream).await;
    prefer_primary(exchange, cleanup)
}

async fn observations_opened<F, T>(
    request: ObservationRequest,
    opening: F,
) -> Result<RemoteObservations<T>, ComponentProtocolError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
    T: DeserializeOwned,
{
    let mut stream = deadline(ComponentProtocolOperation::Open, opening, NETWORK_TIMEOUT).await??;
    let requested = request.product.clone();
    let exchange = deadline(
        ComponentProtocolOperation::Exchange,
        async {
            write_json(&mut stream, &request).await?;
            let header: ObservationBatchHeader = read_json(&mut stream).await?;
            let ObservationBatchHeader::Accepted {
                product,
                product_manifest_hash,
                producer,
                observations,
                gap,
            } = header
            else {
                let ObservationBatchHeader::Rejected { code, message } = header else {
                    unreachable!()
                };
                return Err(ComponentProtocolError::RemoteRejected { code, message });
            };
            let product = *product;
            let producer = *producer;
            if observations > MAX_BATCH_OBSERVATIONS
                || product.peer_id != requested.peer_id
                || product.product_id != requested.product_id
                || product_manifest_hash != requested.manifest_hash
                || product.hash() != product_manifest_hash
            {
                return Err(ComponentProtocolError::InvalidResponse(
                    "observation response does not match the requested Product".to_owned(),
                ));
            }
            let producer_reference = producer.reference();
            if product.producer != producer_reference {
                return Err(ComponentProtocolError::InvalidResponse(
                    "Product producer does not match the returned Output Manifest".to_owned(),
                ));
            }
            let mut decoded = Vec::with_capacity(observations as usize);
            for _ in 0..observations {
                let record: ObservationRecordHeader = read_json(&mut stream).await?;
                if record.output != producer_reference
                    || record.payload_encoding != "application/json"
                {
                    return Err(ComponentProtocolError::InvalidResponse(
                        "observation record violates the Product contract".to_owned(),
                    ));
                }
                let payload = read_payload(&mut stream, record.payload_bytes).await?;
                let payload = serde_json::from_slice(&payload)
                    .map_err(|error| ComponentProtocolError::Codec(error.to_string()))?;
                decoded.push(Observation {
                    output: record.output,
                    sequence: record.sequence,
                    timestamp_ns: record.timestamp_ns,
                    payload: Arc::new(payload),
                });
            }
            Ok(RemoteObservations {
                product,
                product_manifest_hash,
                producer,
                observations: decoded,
                gap,
            })
        },
        NETWORK_TIMEOUT,
    )
    .await
    .and_then(|result| result);
    let cleanup = close_stream(&mut stream).await;
    prefer_primary(exchange, cleanup)
}

struct PreparedOperation {
    request: OperationRequest,
    payload: Vec<u8>,
    timeout: Duration,
}

fn prepare_operation<I: Serialize>(
    target_component: ComponentReference,
    operable: String,
    caller_component_id: String,
    invocation_id: String,
    deadline_after: Option<Duration>,
    instruction: &I,
) -> Result<PreparedOperation, ComponentProtocolError> {
    let payload = serde_json::to_vec(instruction)
        .map_err(|error| ComponentProtocolError::Codec(error.to_string()))?;
    ensure_payload_bound(payload.len()).map_err(ComponentProtocolError::from_service)?;
    let deadline_ms = deadline_after
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .transpose_if(|milliseconds| {
            if milliseconds == 0 || milliseconds > MAX_OPERATION_DEADLINE_MS {
                Err(ComponentProtocolError::InvalidRequest(format!(
                    "operation deadline must be within 1..={MAX_OPERATION_DEADLINE_MS} ms"
                )))
            } else {
                Ok(milliseconds)
            }
        })?;
    let timeout = Duration::from_millis(deadline_ms.unwrap_or(10_000).saturating_add(5_000));
    Ok(PreparedOperation {
        request: OperationRequest {
            target_component,
            operable,
            invocation_id,
            caller_component_id,
            deadline_ms,
            instruction_encoding: "application/json".to_owned(),
            instruction_bytes: u32::try_from(payload.len())
                .expect("payload bound is smaller than u32::MAX"),
        },
        payload,
        timeout,
    })
}

async fn operation_opened<F, R>(
    prepared: PreparedOperation,
    opening: F,
) -> Result<Invocation<R>, ComponentProtocolError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
    R: DeserializeOwned,
{
    let mut stream = deadline(ComponentProtocolOperation::Open, opening, NETWORK_TIMEOUT).await??;
    let invocation_id = prepared.request.invocation_id.clone();
    let exchange = deadline(
        ComponentProtocolOperation::Exchange,
        async {
            write_json(&mut stream, &prepared.request).await?;
            write_payload(&mut stream, &prepared.payload).await?;
            match read_json::<_, OperationResponse>(&mut stream).await? {
                OperationResponse::Completed {
                    invocation_id: returned_id,
                    result_encoding,
                    result_bytes,
                } => {
                    if returned_id != invocation_id || result_encoding != "application/json" {
                        return Err(ComponentProtocolError::InvalidResponse(
                            "operation response identity or encoding mismatch".to_owned(),
                        ));
                    }
                    let payload = read_payload(&mut stream, result_bytes).await?;
                    let result = serde_json::from_slice(&payload)
                        .map_err(|error| ComponentProtocolError::Codec(error.to_string()))?;
                    Ok(Invocation {
                        invocation_id: returned_id,
                        result,
                    })
                }
                OperationResponse::Failed {
                    invocation_id: returned_id,
                    error,
                } => {
                    if returned_id != invocation_id {
                        return Err(ComponentProtocolError::InvalidResponse(
                            "operation failure identity mismatch".to_owned(),
                        ));
                    }
                    Err(ComponentProtocolError::RemoteOperation(error))
                }
            }
        },
        prepared.timeout,
    )
    .await
    .and_then(|result| result);
    let cleanup = close_stream(&mut stream).await;
    prefer_primary(exchange, cleanup)
}

fn validate_operation_request(
    request: &OperationRequest,
    local_peer_id: &str,
) -> Result<(), ServiceError> {
    if request.target_component.peer_id != local_peer_id {
        return Err(ServiceError::new(
            "wrong_peer",
            "target Component belongs to another peer",
        ));
    }
    if request.operable.is_empty()
        || request.caller_component_id.is_empty()
        || request.invocation_id.is_empty()
    {
        return Err(ServiceError::new(
            "invalid_request",
            "operation names and identities must not be empty",
        ));
    }
    if request.instruction_encoding != "application/json" {
        return Err(ServiceError::new(
            "unsupported_encoding",
            "only application/json instructions are currently supported",
        ));
    }
    if request.instruction_bytes as usize > MAX_PAYLOAD_FRAME_BYTES {
        return Err(ServiceError::new(
            "instruction_too_large",
            "instruction exceeds the protocol frame bound",
        ));
    }
    if request
        .deadline_ms
        .is_some_and(|milliseconds| milliseconds == 0 || milliseconds > MAX_OPERATION_DEADLINE_MS)
    {
        return Err(ServiceError::new(
            "invalid_deadline",
            format!("deadline must be within 1..={MAX_OPERATION_DEADLINE_MS} ms"),
        ));
    }
    Ok(())
}

fn ensure_payload_bound(length: usize) -> Result<(), ServiceError> {
    if length > MAX_PAYLOAD_FRAME_BYTES {
        return Err(ServiceError::new(
            "payload_too_large",
            format!("payload is {length} bytes; maximum is {MAX_PAYLOAD_FRAME_BYTES}"),
        ));
    }
    Ok(())
}

fn catalog_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(
        CATALOG_PROTOCOL_ID,
        MAX_CONCURRENCY,
        MAX_CONTROL_FRAME_BYTES as u32,
    )
}

fn observations_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(
        OBSERVATIONS_PROTOCOL_ID,
        MAX_CONCURRENCY,
        MAX_PAYLOAD_FRAME_BYTES as u32,
    )
}

fn operations_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(
        OPERATIONS_PROTOCOL_ID,
        MAX_CONCURRENCY,
        MAX_PAYLOAD_FRAME_BYTES as u32,
    )
}

async fn close_stream(stream: &mut AuthenticatedRouteStream) -> Result<(), ComponentProtocolError> {
    deadline(
        ComponentProtocolOperation::Close,
        AsyncWriteExt::close(stream),
        NETWORK_TIMEOUT,
    )
    .await?
    .map_err(|error| ComponentProtocolError::Close(error.to_string()))
}

async fn deadline<T>(
    operation: ComponentProtocolOperation,
    future: impl Future<Output = T>,
    duration: Duration,
) -> Result<T, ComponentProtocolError> {
    let work = future.fuse();
    let timer = Delay::new(duration).fuse();
    pin_mut!(work, timer);
    futures::select_biased! {
        result = work => Ok(result),
        () = timer => Err(ComponentProtocolError::Timeout(operation)),
    }
}

fn prefer_primary<T>(
    primary: Result<T, ComponentProtocolError>,
    cleanup: Result<(), ComponentProtocolError>,
) -> Result<T, ComponentProtocolError> {
    match primary {
        Err(error) => Err(error),
        Ok(value) => cleanup.map(|()| value),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentProtocolOperation {
    Open,
    Exchange,
    Close,
}

impl std::fmt::Display for ComponentProtocolOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Open => "open",
            Self::Exchange => "exchange",
            Self::Close => "close",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ComponentProtocolError {
    #[error("Auki protocol operation failed: {0}")]
    Sdk(#[from] AukiProtocolError),
    #[error("Component runtime peer {runtime_peer_id} does not match AukiPeer {protocol_peer_id}")]
    RuntimePeerMismatch {
        runtime_peer_id: String,
        protocol_peer_id: String,
    },
    #[error("duplicate protocol export: {0}")]
    DuplicateExport(String),
    #[error("cannot export component interface: {0}")]
    Export(String),
    #[error("cannot import remote Product: {0}")]
    Import(String),
    #[error("component protocol wire failure: {0}")]
    Wire(String),
    #[error("component protocol codec failure: {0}")]
    Codec(String),
    #[error("invalid component protocol request: {0}")]
    InvalidRequest(String),
    #[error("invalid component protocol response: {0}")]
    InvalidResponse(String),
    #[error("remote component request rejected ({code}): {message}")]
    RemoteRejected { code: String, message: String },
    #[error("remote Operable invocation failed ({}): {}", .0.code, .0.message)]
    RemoteOperation(RemoteOperationError),
    #[error("component protocol {0} timed out")]
    Timeout(ComponentProtocolOperation),
    #[error("closing authenticated component stream failed: {0}")]
    Close(String),
}

impl ComponentProtocolError {
    fn from_wire(error: WireError) -> Self {
        Self::Wire(error.to_string())
    }

    fn from_service(error: ServiceError) -> Self {
        Self::InvalidRequest(error.message)
    }
}

impl From<WireError> for ComponentProtocolError {
    fn from(error: WireError) -> Self {
        Self::from_wire(error)
    }
}

#[derive(Debug)]
struct ServiceError {
    code: String,
    message: String,
}

impl ServiceError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn product(error: impl std::fmt::Display) -> Self {
        Self::new("product_access_failed", error.to_string())
    }

    fn invocation(error: InvocationError) -> Self {
        let code = match error {
            InvocationError::NotExposed => "not_exposed",
            InvocationError::Unauthorized => "unauthorized",
            InvocationError::TargetUnavailable => "target_unavailable",
            InvocationError::Rejected(_) => "rejected",
            InvocationError::Cancelled => "cancelled",
            InvocationError::DeadlineExceeded => "deadline_exceeded",
            InvocationError::TargetPanicked(_) => "target_panicked",
            InvocationError::RuntimeUnavailable => "runtime_unavailable",
            InvocationError::Overloaded { .. } => "overloaded",
        };
        Self::new(code, error.to_string())
    }
}

impl From<WireError> for ServiceError {
    fn from(error: WireError) -> Self {
        Self::new("wire_error", error.to_string())
    }
}

trait OptionTransposeIf<T> {
    fn transpose_if<E>(self, validate: impl FnOnce(T) -> Result<T, E>) -> Result<Option<T>, E>;
}

impl<T> OptionTransposeIf<T> for Option<T> {
    fn transpose_if<E>(self, validate: impl FnOnce(T) -> Result<T, E>) -> Result<Option<T>, E> {
        self.map(validate).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_deadlines_are_bounded_before_opening_a_stream() {
        let target = ComponentReference {
            peer_id: "peer".to_owned(),
            component_id: "actuator".to_owned(),
            manifest_hash: "hash".to_owned(),
        };
        assert!(matches!(
            prepare_operation::<u64>(
                target,
                "set".to_owned(),
                "console".to_owned(),
                "invocation".to_owned(),
                Some(Duration::from_secs(61)),
                &1,
            ),
            Err(ComponentProtocolError::InvalidRequest(_))
        ));
    }
}
