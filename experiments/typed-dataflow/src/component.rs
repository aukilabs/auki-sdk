use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ComponentError, Connection, ConnectionControl, ConnectionError, ConnectionOptions,
    ConnectionStats, Envelope, EveryFullPolicy, InputPort, OutputPort, PublishReport,
    SharedDelivery, SharedDispatcher, connect, connect_shared,
};

pub type ManifestHash = String;

/// Hashes the exact serialized experimental manifest.
///
/// The experiment uses structs and ordered collections, so their JSON encoding
/// is deterministic. A production format still needs an explicitly versioned
/// canonicalization rule.
pub fn manifest_hash(manifest: &impl Serialize) -> ManifestHash {
    let encoded = serde_json::to_vec(manifest).expect("experimental manifests must serialize");
    let digest = Sha256::digest(encoded);
    let mut encoded_digest = String::with_capacity(7 + digest.len() * 2);
    encoded_digest.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded_digest, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded_digest
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exposure {
    Local,
    Cluster,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationAccess {
    LatestExisting,
    FirstAvailable,
    AllAvailable,
    TimeRange,
    FollowNew,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservableContract {
    pub name: String,
    pub datatype: String,
    pub schema: String,
    pub access: Vec<ObservationAccess>,
    pub exposure: Exposure,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OperableContract {
    pub name: String,
    pub instruction: String,
    pub result: String,
    pub exposure: Exposure,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentManifest {
    pub schema: String,
    pub peer_id: String,
    pub component_id: String,
    pub observables: Vec<ObservableContract>,
    pub operables: Vec<OperableContract>,
}

impl ComponentManifest {
    pub fn hash(&self) -> ManifestHash {
        manifest_hash(self)
    }

    pub fn reference(&self) -> ComponentReference {
        ComponentReference {
            peer_id: self.peer_id.clone(),
            component_id: self.component_id.clone(),
            manifest_hash: self.hash(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ComponentReference {
    pub peer_id: String,
    pub component_id: String,
    pub manifest_hash: ManifestHash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PayloadContract {
    pub kind: String,
    pub datatype: String,
    pub schema: String,
    pub encoding: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub observes: String,
    pub unit: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OutputManifest {
    pub schema: String,
    pub peer_id: String,
    pub component_id: String,
    pub component_manifest_hash: ManifestHash,
    pub slot: String,
    pub output_id: String,
    pub clock_id: String,
    pub spatial_frame_id: Option<String>,
    pub payload: PayloadContract,
}

impl OutputManifest {
    pub fn hash(&self) -> ManifestHash {
        manifest_hash(self)
    }

    pub fn reference(&self) -> OutputReference {
        OutputReference {
            peer_id: self.peer_id.clone(),
            component_id: self.component_id.clone(),
            component_manifest_hash: self.component_manifest_hash.clone(),
            slot: self.slot.clone(),
            output_id: self.output_id.clone(),
            manifest_hash: self.hash(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct OutputReference {
    pub peer_id: String,
    pub component_id: String,
    pub component_manifest_hash: ManifestHash,
    pub slot: String,
    pub output_id: String,
    pub manifest_hash: ManifestHash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductForm {
    Buffer,
    Episode,
    Artifact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProductManifest {
    pub schema: String,
    pub peer_id: String,
    pub product_id: String,
    pub form: ProductForm,
    pub producer: OutputReference,
    pub access: Vec<ObservationAccess>,
}

impl ProductManifest {
    pub fn hash(&self) -> ManifestHash {
        manifest_hash(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogOutputEntry {
    pub manifest: OutputManifest,
    pub manifest_hash: ManifestHash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogComponentEntry {
    pub manifest: ComponentManifest,
    pub manifest_hash: ManifestHash,
    pub current_outputs: BTreeMap<String, CatalogOutputEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogProductEntry {
    pub manifest: ProductManifest,
    pub manifest_hash: ManifestHash,
}

#[derive(Default)]
struct CatalogState {
    components: BTreeMap<String, CatalogComponentEntry>,
    products: BTreeMap<String, CatalogProductEntry>,
}

/// A viewer-neutral Catalog projection for one experimental Peer.
///
/// Only Component interfaces deliberately included in a Component Manifest
/// and Products deliberately registered here are discoverable.
#[derive(Clone, Default)]
pub struct Catalog {
    inner: Arc<RwLock<CatalogState>>,
}

impl fmt::Debug for Catalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.read().unwrap();
        formatter
            .debug_struct("Catalog")
            .field("components", &state.components.keys())
            .field("products", &state.products.keys())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogError {
    UnknownComponent(String),
    ComponentManifestMismatch {
        component_id: String,
        expected: ManifestHash,
        actual: ManifestHash,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownComponent(component_id) => {
                write!(formatter, "Catalog has no Component {component_id}")
            }
            Self::ComponentManifestMismatch {
                component_id,
                expected,
                actual,
            } => write!(
                formatter,
                "Output for Component {component_id} references Component Manifest {actual}, \
                 but the Catalog has {expected}"
            ),
        }
    }
}

impl std::error::Error for CatalogError {}

impl Catalog {
    pub fn register_component(&self, manifest: ComponentManifest) {
        let manifest_hash = manifest.hash();
        self.inner.write().unwrap().components.insert(
            manifest.component_id.clone(),
            CatalogComponentEntry {
                manifest,
                manifest_hash,
                current_outputs: BTreeMap::new(),
            },
        );
    }

    pub fn set_current_output(&self, manifest: OutputManifest) -> Result<(), CatalogError> {
        let manifest_hash = manifest.hash();
        let mut state = self.inner.write().unwrap();
        let component = state
            .components
            .get_mut(&manifest.component_id)
            .ok_or_else(|| CatalogError::UnknownComponent(manifest.component_id.clone()))?;
        if manifest.component_manifest_hash != component.manifest_hash {
            return Err(CatalogError::ComponentManifestMismatch {
                component_id: manifest.component_id.clone(),
                expected: component.manifest_hash.clone(),
                actual: manifest.component_manifest_hash.clone(),
            });
        }
        component.current_outputs.insert(
            manifest.slot.clone(),
            CatalogOutputEntry {
                manifest,
                manifest_hash,
            },
        );
        Ok(())
    }

    pub fn register_product(&self, manifest: ProductManifest) {
        let manifest_hash = manifest.hash();
        self.inner.write().unwrap().products.insert(
            manifest.product_id.clone(),
            CatalogProductEntry {
                manifest,
                manifest_hash,
            },
        );
    }

    pub fn component(&self, component_id: &str) -> Option<CatalogComponentEntry> {
        self.inner
            .read()
            .unwrap()
            .components
            .get(component_id)
            .cloned()
    }

    pub fn product(&self, product_id: &str) -> Option<CatalogProductEntry> {
        self.inner.read().unwrap().products.get(product_id).cloned()
    }

    pub fn products(&self) -> Vec<CatalogProductEntry> {
        self.inner
            .read()
            .unwrap()
            .products
            .values()
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct PeerRuntime {
    peer_id: Arc<str>,
    catalog: Catalog,
}

impl PeerRuntime {
    pub fn new(peer_id: impl Into<Arc<str>>) -> Self {
        Self {
            peer_id: peer_id.into(),
            catalog: Catalog::default(),
        }
    }

    pub fn peer_id(&self) -> &str {
        &self.peer_id
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
}

/// One observation returned by an Observable.
#[derive(Debug, Serialize, Deserialize)]
pub struct Observation<T> {
    pub output: OutputReference,
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub payload: Arc<T>,
}

impl<T> Clone for Observation<T> {
    fn clone(&self) -> Self {
        Self {
            output: self.output.clone(),
            sequence: self.sequence,
            timestamp_ns: self.timestamp_ns,
            payload: Arc::clone(&self.payload),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OutputTransition {
    pub previous: OutputReference,
    pub replacement: OutputReference,
    pub previous_last_sequence: Option<u64>,
    pub effective_at_timestamp_ns: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationFailure {
    pub output: OutputReference,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ObservationEvent<T> {
    Observation(Observation<T>),
    /// Terminal for an observation pinned to `previous`; transitional for an
    /// explicit follow-current observation of the containing output slot.
    Reconfigured(OutputTransition),
    /// Terminal failure reported by the producing Component for this Output.
    Failed(ObservationFailure),
}

impl<T> Clone for ObservationEvent<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Observation(observation) => Self::Observation(observation.clone()),
            Self::Reconfigured(transition) => Self::Reconfigured(transition.clone()),
            Self::Failed(failure) => Self::Failed(failure.clone()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ObservableTarget {
    Pinned(OutputReference),
    CurrentOutputSlot {
        peer_id: String,
        component_id: String,
        slot: String,
    },
}

/// A typed interface through which a Component can show another Component
/// observations. This first slice implements only continuing live observation.
pub struct Observable<T> {
    name: Arc<str>,
    target: ObservableTarget,
    access: Arc<[ObservationAccess]>,
    port: OutputPort<ObservationEvent<T>>,
}

impl<T> Clone for Observable<T> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            target: self.target.clone(),
            access: Arc::clone(&self.access),
            port: self.port.clone(),
        }
    }
}

impl<T> fmt::Debug for Observable<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Observable")
            .field("name", &self.name)
            .field("target", &self.target)
            .field("access", &self.access)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EverySelectedDelivery {
    Inline,
    Queued {
        capacity: usize,
        when_full: EveryFullPolicy,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationDelivery {
    EverySelected(EverySelectedDelivery),
    CoalesceLatest,
}

impl ObservationDelivery {
    pub const fn inline_every_selected() -> Self {
        Self::EverySelected(EverySelectedDelivery::Inline)
    }

    pub const fn queued_every_selected(capacity: usize, when_full: EveryFullPolicy) -> Self {
        Self::EverySelected(EverySelectedDelivery::Queued {
            capacity,
            when_full,
        })
    }

    pub const fn coalesce_latest() -> Self {
        Self::CoalesceLatest
    }

    fn connection_options(self) -> ConnectionOptions {
        match self {
            Self::EverySelected(EverySelectedDelivery::Inline) => ConnectionOptions::InlineEvery,
            Self::EverySelected(EverySelectedDelivery::Queued {
                capacity,
                when_full,
            }) => ConnectionOptions::QueuedEvery {
                capacity,
                when_full,
            },
            Self::CoalesceLatest => ConnectionOptions::Latest,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationError {
    UnsupportedRequest(ObservationAccess),
    Connection(ConnectionError),
}

impl fmt::Display for ObservationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRequest(access) => {
                write!(formatter, "Observable does not support {access:?}")
            }
            Self::Connection(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ObservationError {}

impl From<ConnectionError> for ObservationError {
    fn from(error: ConnectionError) -> Self {
        Self::Connection(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationStatus {
    Active,
    Completed,
    Reconfigured(Box<OutputTransition>),
    Failed(String),
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TransportStats {
    pub encoded_messages: u64,
    pub encoded_bytes: u64,
    pub decoded_messages: u64,
    pub decoded_bytes: u64,
}

#[derive(Debug, Default)]
struct TransportCounters {
    encoded_messages: AtomicU64,
    encoded_bytes: AtomicU64,
    decoded_messages: AtomicU64,
    decoded_bytes: AtomicU64,
}

impl TransportCounters {
    fn snapshot(&self) -> TransportStats {
        TransportStats {
            encoded_messages: self.encoded_messages.load(Ordering::Relaxed),
            encoded_bytes: self.encoded_bytes.load(Ordering::Relaxed),
            decoded_messages: self.decoded_messages.load(Ordering::Relaxed),
            decoded_bytes: self.decoded_bytes.load(Ordering::Relaxed),
        }
    }

    fn encoded(&self, bytes: usize) {
        self.encoded_messages.fetch_add(1, Ordering::Relaxed);
        self.encoded_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn decoded(&self, bytes: usize) {
        self.decoded_messages.fetch_add(1, Ordering::Relaxed);
        self.decoded_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ObservationStats {
    pub accepted: u64,
    pub delivered: u64,
    pub coalesced: u64,
    pub overruns: u64,
    pub closed: bool,
    pub failed: bool,
    pub transport: TransportStats,
}

/// Owns one continuing `follow_new` relationship.
///
/// Finite retained Product queries return finite values directly and do not
/// manufacture a long-lived handle.
#[must_use = "dropping an ObservationHandle cancels the continuing relationship"]
pub struct ObservationHandle<T: Send + Sync + 'static> {
    status: Arc<Mutex<ObservationStatus>>,
    connection: Connection<ObservationEvent<T>>,
    transport: Option<Arc<TransportCounters>>,
}

impl<T: Send + Sync + 'static> fmt::Debug for ObservationHandle<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservationHandle")
            .field("status", &self.status())
            .field("stats", &self.stats())
            .finish()
    }
}

impl<T: Send + Sync + 'static> ObservationHandle<T> {
    pub fn status(&self) -> ObservationStatus {
        let failure = self.connection.failure();
        let mut status = self.status.lock().unwrap();
        if *status == ObservationStatus::Active
            && let Some(failure) = failure
        {
            *status = ObservationStatus::Failed(failure.to_string());
        }
        status.clone()
    }

    pub fn stats(&self) -> ObservationStats {
        let ConnectionStats {
            accepted,
            delivered,
            replaced,
            overruns,
            closed,
            failed,
        } = self.connection.stats();
        ObservationStats {
            accepted,
            delivered,
            coalesced: replaced,
            overruns,
            closed,
            failed,
            transport: self
                .transport
                .as_ref()
                .map_or_else(TransportStats::default, |counters| counters.snapshot()),
        }
    }

    pub fn cancel(&self) {
        self.connection.disconnect();
        let mut status = self.status.lock().unwrap();
        if *status == ObservationStatus::Active {
            *status = ObservationStatus::Cancelled;
        }
    }

    fn with_transport(mut self, counters: Arc<TransportCounters>) -> Self {
        self.transport = Some(counters);
        self
    }
}

impl<T: Send + Sync + 'static> Drop for ObservationHandle<T> {
    fn drop(&mut self) {
        self.connection.disconnect();
        let mut status = self.status.lock().unwrap();
        if *status == ObservationStatus::Active {
            *status = ObservationStatus::Cancelled;
        }
    }
}

impl<T: Send + Sync + 'static> Observable<T> {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn target(&self) -> &ObservableTarget {
        &self.target
    }

    pub fn supported_access(&self) -> &[ObservationAccess] {
        &self.access
    }

    pub fn supports(&self, access: ObservationAccess) -> bool {
        self.access.contains(&access)
    }

    /// Requests the newest observation that already exists.
    ///
    /// This fresh-only experiment Observable has no retained backing. The
    /// explicit error is preferable to manufacturing a sensor sample or
    /// silently treating a live subscription as a finite query.
    pub fn latest_existing(&self) -> Result<Option<Observation<T>>, ObservationError> {
        Err(ObservationError::UnsupportedRequest(
            ObservationAccess::LatestExisting,
        ))
    }

    pub fn time_range(
        &self,
        _request: crate::product::TimeRangeRequest,
    ) -> Result<crate::product::FiniteObservations<T>, ObservationError> {
        Err(ObservationError::UnsupportedRequest(
            ObservationAccess::TimeRange,
        ))
    }

    pub fn follow_new(
        &self,
        observer: &InputPort<ObservationEvent<T>>,
        delivery: ObservationDelivery,
    ) -> Result<ObservationHandle<T>, ObservationError> {
        self.follow_new_using(observer, |input| {
            connect(&self.port, input, delivery.connection_options())
        })
    }

    /// Runs this relationship on a fixed worker pool shared with other
    /// relationships. Observation queues remain per relationship and bounded.
    pub fn follow_new_shared(
        &self,
        observer: &InputPort<ObservationEvent<T>>,
        dispatcher: &SharedDispatcher,
        delivery: SharedDelivery,
    ) -> Result<ObservationHandle<T>, ObservationError> {
        self.follow_new_using(observer, |input| {
            connect_shared(&self.port, input, dispatcher, delivery)
        })
    }

    fn follow_new_using(
        &self,
        observer: &InputPort<ObservationEvent<T>>,
        connector: impl FnOnce(
            &InputPort<ObservationEvent<T>>,
        ) -> Result<Connection<ObservationEvent<T>>, ConnectionError>,
    ) -> Result<ObservationHandle<T>, ObservationError> {
        if !self.supports(ObservationAccess::FollowNew) {
            return Err(ObservationError::UnsupportedRequest(
                ObservationAccess::FollowNew,
            ));
        }

        let status = Arc::new(Mutex::new(ObservationStatus::Active));
        let control: Arc<Mutex<Option<ConnectionControl<ObservationEvent<T>>>>> =
            Arc::new(Mutex::new(None));
        let pinned = matches!(self.target, ObservableTarget::Pinned(_));
        let observer = observer.clone();
        let callback_status = Arc::clone(&status);
        let callback_control = Arc::clone(&control);
        let input = InputPort::with_component_errors(
            format!("{}.follow-new", self.name),
            move |envelope: &Envelope<ObservationEvent<T>>| {
                match &envelope.payload {
                    ObservationEvent::Reconfigured(transition) if pinned => {
                        *callback_status.lock().unwrap() =
                            ObservationStatus::Reconfigured(Box::new(transition.clone()));
                    }
                    ObservationEvent::Failed(failure) => {
                        *callback_status.lock().unwrap() =
                            ObservationStatus::Failed(failure.reason.clone());
                    }
                    ObservationEvent::Observation(_) | ObservationEvent::Reconfigured(_) => {}
                }
                let terminal =
                    !matches!(*callback_status.lock().unwrap(), ObservationStatus::Active);
                observer.accept(envelope)?;
                if terminal && let Some(control) = callback_control.lock().unwrap().as_ref() {
                    control.disconnect();
                }
                Ok(())
            },
        );
        let connection = connector(&input)?;
        *control.lock().unwrap() = Some(connection.control());
        if !matches!(*status.lock().unwrap(), ObservationStatus::Active) {
            connection.disconnect();
        }
        Ok(ObservationHandle {
            status,
            connection,
            transport: None,
        })
    }
}

pub(crate) struct ObservationEmitter<T> {
    port: OutputPort<ObservationEvent<T>>,
}

impl<T> Clone for ObservationEmitter<T> {
    fn clone(&self) -> Self {
        Self {
            port: self.port.clone(),
        }
    }
}

impl<T: Send + Sync + 'static> ObservationEmitter<T> {
    pub(crate) fn emit(&self, timestamp_ns: u64, event: ObservationEvent<T>) -> PublishReport {
        self.port.publish(timestamp_ns, event)
    }
}

pub(crate) fn pinned_observable<T: Send + Sync + 'static>(
    output: OutputReference,
    access: impl Into<Arc<[ObservationAccess]>>,
) -> (Observable<T>, ObservationEmitter<T>) {
    let name: Arc<str> = format!(
        "{}.{}.{}",
        output.component_id, output.slot, output.output_id
    )
    .into();
    let port = OutputPort::new(Arc::clone(&name));
    (
        Observable {
            name,
            target: ObservableTarget::Pinned(output),
            access: access.into(),
            port: port.clone(),
        },
        ObservationEmitter { port },
    )
}

pub(crate) fn current_output_observable<T: Send + Sync + 'static>(
    peer_id: &str,
    component_id: &str,
    slot: &str,
    access: impl Into<Arc<[ObservationAccess]>>,
) -> (Observable<T>, ObservationEmitter<T>) {
    let name: Arc<str> = format!("{component_id}.{slot}.current").into();
    let port = OutputPort::new(Arc::clone(&name));
    (
        Observable {
            name,
            target: ObservableTarget::CurrentOutputSlot {
                peer_id: peer_id.to_owned(),
                component_id: component_id.to_owned(),
                slot: slot.to_owned(),
            },
            access: access.into(),
            port: port.clone(),
        },
        ObservationEmitter { port },
    )
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InvocationContext {
    pub invocation_id: String,
    pub caller_peer_id: String,
    pub caller_component_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Invocation<R> {
    pub invocation_id: String,
    pub result: R,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InvocationError {
    NotExposed,
    Unauthorized,
    TargetUnavailable,
    Rejected(String),
}

impl fmt::Display for InvocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotExposed => formatter.write_str("Operable is not exposed to this caller"),
            Self::Unauthorized => formatter.write_str("caller is not authorized"),
            Self::TargetUnavailable => formatter.write_str("target Component is unavailable"),
            Self::Rejected(reason) => write!(formatter, "instruction rejected: {reason}"),
        }
    }
}

impl std::error::Error for InvocationError {}

type OperationHandler<I, R> =
    dyn Fn(&InvocationContext, I) -> Result<R, InvocationError> + Send + Sync;
type Authorizer = dyn Fn(&InvocationContext) -> bool + Send + Sync;

/// A typed interface through which another Component may intentionally cause
/// configuration or execution.
pub struct Operable<I, R> {
    name: Arc<str>,
    owner: ComponentReference,
    exposure: Exposure,
    handler: Arc<OperationHandler<I, R>>,
    authorizer: Arc<Authorizer>,
}

impl<I, R> Clone for Operable<I, R> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            owner: self.owner.clone(),
            exposure: self.exposure,
            handler: Arc::clone(&self.handler),
            authorizer: Arc::clone(&self.authorizer),
        }
    }
}

impl<I, R> fmt::Debug for Operable<I, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Operable")
            .field("name", &self.name)
            .field("owner", &self.owner)
            .field("exposure", &self.exposure)
            .finish_non_exhaustive()
    }
}

impl<I, R> Operable<I, R> {
    pub fn new(
        name: impl Into<Arc<str>>,
        owner: ComponentReference,
        exposure: Exposure,
        authorizer: impl Fn(&InvocationContext) -> bool + Send + Sync + 'static,
        handler: impl Fn(&InvocationContext, I) -> Result<R, InvocationError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            owner,
            exposure,
            handler: Arc::new(handler),
            authorizer: Arc::new(authorizer),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn exposure(&self) -> Exposure {
        self.exposure
    }

    pub fn invoke(
        &self,
        context: InvocationContext,
        instruction: I,
    ) -> Result<Invocation<R>, InvocationError> {
        let remote = context.caller_peer_id != self.owner.peer_id;
        if remote && self.exposure != Exposure::Cluster {
            return Err(InvocationError::NotExposed);
        }
        if !(self.authorizer)(&context) {
            return Err(InvocationError::Unauthorized);
        }
        let result = (self.handler)(&context, instruction)?;
        Ok(Invocation {
            invocation_id: context.invocation_id,
            result,
        })
    }
}

/// An in-memory stand-in for a transport. It deliberately preserves the
/// Component-facing API while keeping production networking out of scope.
#[derive(Clone, Copy, Debug, Default)]
pub struct InMemoryTransport;

impl InMemoryTransport {
    pub fn follow_new<T: Send + Sync + 'static>(
        &self,
        observable: &Observable<T>,
        observer: &InputPort<ObservationEvent<T>>,
        delivery: ObservationDelivery,
    ) -> Result<ObservationHandle<T>, ObservationError> {
        observable.follow_new(observer, delivery)
    }

    pub fn invoke<I, R>(
        &self,
        operable: &Operable<I, R>,
        context: InvocationContext,
        instruction: I,
    ) -> Result<Invocation<R>, InvocationError> {
        operable.invoke(context, instruction)
    }
}

/// A transport-shaped test fixture that performs a real serialization round
/// trip while remaining independent of production networking.
///
/// Its byte counters make it impossible to mistake local `Arc` sharing for
/// network zero-copy evidence.
#[derive(Clone, Debug, Default)]
pub struct SerializedInMemoryTransport {
    counters: Arc<TransportCounters>,
}

impl SerializedInMemoryTransport {
    pub fn stats(&self) -> TransportStats {
        self.counters.snapshot()
    }

    pub(crate) fn round_trip<T>(&self, value: &T) -> Result<T, String>
    where
        T: Serialize + DeserializeOwned,
    {
        let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        self.counters.encoded(bytes.len());
        let decoded = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        self.counters.decoded(bytes.len());
        Ok(decoded)
    }

    pub fn follow_new<T>(
        &self,
        observable: &Observable<T>,
        observer: &InputPort<ObservationEvent<T>>,
        delivery: ObservationDelivery,
    ) -> Result<ObservationHandle<T>, ObservationError>
    where
        T: Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        let transport = self.clone();
        let observer = observer.clone();
        let input = InputPort::with_component_errors(
            format!("{}.serialized-transport", observable.name()),
            move |envelope: &Envelope<ObservationEvent<T>>| {
                let decoded = transport.round_trip(&envelope.payload).map_err(|error| {
                    ComponentError::Reported(format!("serialized observation: {error}"))
                })?;
                observer.accept(&Envelope::new(
                    envelope.sequence,
                    envelope.timestamp_ns,
                    decoded,
                ))
            },
        );
        observable
            .follow_new(&input, delivery)
            .map(|handle| handle.with_transport(Arc::clone(&self.counters)))
    }

    pub fn follow_new_shared<T>(
        &self,
        observable: &Observable<T>,
        observer: &InputPort<ObservationEvent<T>>,
        dispatcher: &SharedDispatcher,
        delivery: SharedDelivery,
    ) -> Result<ObservationHandle<T>, ObservationError>
    where
        T: Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        let transport = self.clone();
        let observer = observer.clone();
        let input = InputPort::with_component_errors(
            format!("{}.serialized-transport", observable.name()),
            move |envelope: &Envelope<ObservationEvent<T>>| {
                let decoded = transport.round_trip(&envelope.payload).map_err(|error| {
                    ComponentError::Reported(format!("serialized observation: {error}"))
                })?;
                observer.accept(&Envelope::new(
                    envelope.sequence,
                    envelope.timestamp_ns,
                    decoded,
                ))
            },
        );
        observable
            .follow_new_shared(&input, dispatcher, delivery)
            .map(|handle| handle.with_transport(Arc::clone(&self.counters)))
    }

    pub fn invoke<I, R>(
        &self,
        operable: &Operable<I, R>,
        context: InvocationContext,
        instruction: I,
    ) -> Result<Invocation<R>, InvocationError>
    where
        I: Serialize + DeserializeOwned,
        R: Serialize + DeserializeOwned,
    {
        let (context, instruction) = self
            .round_trip(&(context, instruction))
            .map_err(|error| InvocationError::Rejected(format!("transport request: {error}")))?;
        let result = operable.invoke(context, instruction)?;
        self.round_trip(&result)
            .map_err(|error| InvocationError::Rejected(format!("transport result: {error}")))
    }
}

/// Utility for observers that want the payload event while retaining the
/// existing port experiment's borrowed callback shape.
pub fn observation_input<T>(
    name: impl Into<Arc<str>>,
    handler: impl Fn(&ObservationEvent<T>) + Send + Sync + 'static,
) -> InputPort<ObservationEvent<T>> {
    InputPort::new(name, move |envelope: &Envelope<ObservationEvent<T>>| {
        handler(&envelope.payload);
    })
}
