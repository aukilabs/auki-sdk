//! Public construction API for live experimental Components.
//!
//! The important invariant is that Catalog projection follows construction of
//! typed runtime handles. Applications cannot mutate the Catalog directly.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::PublishReport;
use crate::component::{
    Catalog, CatalogError, ComponentManifest, ComponentReference, Exposure, InvocationContext,
    InvocationError, InvocationOrdering, Observable, ObservableContract, Observation,
    ObservationAccess, ObservationEmitter, ObservationEnd, ObservationEndReason, ObservationEvent,
    Operable, OperableContract, OutputManifest, OutputReference, PayloadContract, PeerRuntime,
    output_observable,
};

/// Binds a Rust payload or instruction type to its advertised datatype.
///
/// Schemas and semantic fields remain explicit contract assertions, while the
/// most basic representation mismatch (for example `String` advertised as
/// `float64`) is rejected during typed handle construction.
pub trait ContractType {
    const DATATYPE: &'static str;
}

macro_rules! primitive_contract_type {
    ($type:ty, $name:literal) => {
        impl ContractType for $type {
            const DATATYPE: &'static str = $name;
        }
    };
}

primitive_contract_type!(f32, "float32");
primitive_contract_type!(f64, "float64");
primitive_contract_type!(i16, "int16");
primitive_contract_type!(i32, "int32");
primitive_contract_type!(i64, "int64");
primitive_contract_type!(u16, "uint16");
primitive_contract_type!(u32, "uint32");
primitive_contract_type!(u64, "uint64");
primitive_contract_type!(bool, "bool");
primitive_contract_type!(String, "string");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentSpec {
    pub component_id: String,
    pub observables: Vec<ObservableContract>,
    pub operables: Vec<OperableContract>,
}

impl ComponentSpec {
    pub fn new(component_id: impl Into<String>) -> Self {
        Self {
            component_id: component_id.into(),
            observables: Vec::new(),
            operables: Vec::new(),
        }
    }

    pub fn observable(mut self, contract: ObservableContract) -> Self {
        self.observables.push(contract);
        self
    }

    pub fn operable(mut self, contract: OperableContract) -> Self {
        self.operables.push(contract);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfiguredObservableSpec {
    pub interface: String,
    pub output_id: String,
    pub clock_id: String,
    pub spatial_frame_id: Option<String>,
    pub payload: PayloadContract,
}

impl ConfiguredObservableSpec {
    pub fn new(
        interface: impl Into<String>,
        output_id: impl Into<String>,
        clock_id: impl Into<String>,
        payload: PayloadContract,
    ) -> Self {
        Self {
            interface: interface.into(),
            output_id: output_id.into(),
            clock_id: clock_id.into(),
            spatial_frame_id: None,
            payload,
        }
    }

    pub fn in_spatial_frame(mut self, spatial_frame_id: impl Into<String>) -> Self {
        self.spatial_frame_id = Some(spatial_frame_id.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComponentBuildError {
    EmptyComponentId,
    DuplicateInterface(String),
    LocalInterfaceInClusterManifest(String),
    UnknownObservable(String),
    UnknownOperable(String),
    UnsupportedLiveAccess {
        interface: String,
        access: ObservationAccess,
    },
    DuplicateConfiguredObservable(String),
    DuplicateOperable(String),
    ContractMismatch {
        interface: String,
        expected_datatype: String,
        actual_datatype: String,
        expected_schema: String,
        actual_schema: String,
    },
    RustDatatypeMismatch {
        interface: String,
        rust_datatype: String,
        contract_datatype: String,
    },
    OperationContractMismatch {
        interface: String,
        expected_instruction: String,
        actual_instruction: String,
        expected_result: String,
        actual_result: String,
    },
    MissingObservable(String),
    MissingOperable(String),
    DroppedObservable(String),
    NotExposed,
    NotCurrentOutput {
        interface: String,
        output_id: String,
    },
    ReplacementOutputIdReused(String),
    OutputAlreadyEnded(String),
    AlreadyExposed,
    Catalog(CatalogError),
}

impl fmt::Display for ComponentBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyComponentId => formatter.write_str("Component ID must not be empty"),
            Self::DuplicateInterface(name) => write!(formatter, "duplicate interface {name}"),
            Self::LocalInterfaceInClusterManifest(name) => write!(
                formatter,
                "local-only interface {name} must not be declared in a cluster Component Manifest"
            ),
            Self::UnknownObservable(name) => {
                write!(formatter, "Component does not declare Observable {name}")
            }
            Self::UnknownOperable(name) => {
                write!(formatter, "Component does not declare Operable {name}")
            }
            Self::UnsupportedLiveAccess { interface, access } => write!(
                formatter,
                "fresh Observable {interface} cannot advertise retained request {access:?}"
            ),
            Self::DuplicateConfiguredObservable(name) => {
                write!(formatter, "Observable {name} is already configured")
            }
            Self::DuplicateOperable(name) => write!(formatter, "Operable {name} is already live"),
            Self::ContractMismatch {
                interface,
                expected_datatype,
                actual_datatype,
                expected_schema,
                actual_schema,
            } => write!(
                formatter,
                "configured Observable {interface} declares {actual_datatype}/{actual_schema}, \
                 but its Component contract requires {expected_datatype}/{expected_schema}"
            ),
            Self::RustDatatypeMismatch {
                interface,
                rust_datatype,
                contract_datatype,
            } => write!(
                formatter,
                "Rust type for {interface} declares datatype {rust_datatype}, but the contract declares {contract_datatype}"
            ),
            Self::OperationContractMismatch {
                interface,
                expected_instruction,
                actual_instruction,
                expected_result,
                actual_result,
            } => write!(
                formatter,
                "Operable {interface} requires {expected_instruction} -> {expected_result}, but its Rust types declare {actual_instruction} -> {actual_result}"
            ),
            Self::MissingObservable(name) => {
                write!(
                    formatter,
                    "cannot expose Component: Observable {name} is not live"
                )
            }
            Self::MissingOperable(name) => {
                write!(
                    formatter,
                    "cannot expose Component: Operable {name} is not live"
                )
            }
            Self::DroppedObservable(name) => write!(
                formatter,
                "cannot expose Component: Observable {name} was dropped before exposure"
            ),
            Self::NotExposed => formatter.write_str("Component is not exposed"),
            Self::NotCurrentOutput {
                interface,
                output_id,
            } => write!(
                formatter,
                "Output {output_id} is not the current configured Output for Observable {interface}"
            ),
            Self::ReplacementOutputIdReused(output_id) => write!(
                formatter,
                "replacement Output must use a new Output ID; {output_id} is already current"
            ),
            Self::OutputAlreadyEnded(output_id) => {
                write!(formatter, "Output {output_id} has already ended")
            }
            Self::AlreadyExposed => formatter.write_str("Component is already exposed"),
            Self::Catalog(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ComponentBuildError {}

impl From<CatalogError> for ComponentBuildError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

#[derive(Default)]
struct ComponentState {
    configured_outputs: BTreeMap<String, ConfiguredOutputRegistration>,
    live_operables: BTreeMap<String, std::sync::Weak<()>>,
    exposed: bool,
}

struct ConfiguredOutputRegistration {
    manifest: OutputManifest,
    liveness: std::sync::Weak<()>,
}

struct ComponentInner {
    manifest: ComponentManifest,
    reference: ComponentReference,
    catalog: Catalog,
    state: Mutex<ComponentState>,
}

/// A live unit of behavior hosted by one experimental Peer runtime.
///
/// Call [`Component::expose`] only after constructing every cluster-visible
/// Observable and Operable declared by the Component. Exposure fails rather
/// than projecting a manifest with missing behavior.
#[derive(Clone)]
pub struct Component {
    inner: Arc<ComponentInner>,
}

impl fmt::Debug for Component {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Component")
            .field("reference", &self.inner.reference)
            .field("exposed", &self.inner.state.lock().unwrap().exposed)
            .finish_non_exhaustive()
    }
}

impl PeerRuntime {
    pub fn component(&self, spec: ComponentSpec) -> Result<Component, ComponentBuildError> {
        if spec.component_id.is_empty() {
            return Err(ComponentBuildError::EmptyComponentId);
        }
        let mut names = BTreeSet::new();
        for contract in &spec.observables {
            if contract.exposure != Exposure::Cluster {
                return Err(ComponentBuildError::LocalInterfaceInClusterManifest(
                    contract.name.clone(),
                ));
            }
            if !names.insert(("observable", contract.name.as_str())) {
                return Err(ComponentBuildError::DuplicateInterface(
                    contract.name.clone(),
                ));
            }
        }
        for contract in &spec.operables {
            if contract.exposure != Exposure::Cluster {
                return Err(ComponentBuildError::LocalInterfaceInClusterManifest(
                    contract.name.clone(),
                ));
            }
            if !names.insert(("operable", contract.name.as_str())) {
                return Err(ComponentBuildError::DuplicateInterface(
                    contract.name.clone(),
                ));
            }
        }

        let manifest = ComponentManifest {
            schema: "auki.component-manifest/v1".to_owned(),
            peer_id: self.peer_id().to_owned(),
            component_id: spec.component_id,
            observables: spec.observables,
            operables: spec.operables,
        };
        let reference = manifest.reference();
        Ok(Component {
            inner: Arc::new(ComponentInner {
                manifest,
                reference,
                catalog: self.catalog().clone(),
                state: Mutex::new(ComponentState::default()),
            }),
        })
    }
}

impl Component {
    pub fn manifest(&self) -> &ComponentManifest {
        &self.inner.manifest
    }

    pub fn reference(&self) -> &ComponentReference {
        &self.inner.reference
    }

    pub fn configured_observable<T: ContractType + Send + Sync + 'static>(
        &self,
        spec: ConfiguredObservableSpec,
    ) -> Result<ConfiguredObservable<T>, ComponentBuildError> {
        let contract = self
            .inner
            .manifest
            .observables
            .iter()
            .find(|contract| contract.name == spec.interface)
            .ok_or_else(|| ComponentBuildError::UnknownObservable(spec.interface.clone()))?;
        if let Some(access) = contract
            .access
            .iter()
            .copied()
            .find(|access| *access != ObservationAccess::FollowNew)
        {
            return Err(ComponentBuildError::UnsupportedLiveAccess {
                interface: contract.name.clone(),
                access,
            });
        }
        if contract.datatype != spec.payload.datatype() || contract.schema != spec.payload.schema()
        {
            return Err(ComponentBuildError::ContractMismatch {
                interface: spec.interface,
                expected_datatype: contract.datatype.clone(),
                actual_datatype: spec.payload.datatype().to_owned(),
                expected_schema: contract.schema.clone(),
                actual_schema: spec.payload.schema().to_owned(),
            });
        }
        if T::DATATYPE != contract.datatype {
            return Err(ComponentBuildError::RustDatatypeMismatch {
                interface: contract.name.clone(),
                rust_datatype: T::DATATYPE.to_owned(),
                contract_datatype: contract.datatype.clone(),
            });
        }

        let manifest = OutputManifest {
            schema: "auki.component-output-manifest/v1".to_owned(),
            peer_id: self.inner.reference.peer_id.clone(),
            component_id: self.inner.reference.component_id.clone(),
            component_manifest_hash: self.inner.reference.manifest_hash.clone(),
            slot: contract.name.clone(),
            output_id: spec.output_id,
            clock_id: spec.clock_id,
            spatial_frame_id: spec.spatial_frame_id,
            payload: spec.payload,
        };
        let reference = manifest.reference();
        let (observable, emitter) = output_observable(reference.clone(), contract.access.clone());
        let liveness = Arc::new(());

        let mut state = self.inner.state.lock().unwrap();
        if state.exposed {
            return Err(ComponentBuildError::AlreadyExposed);
        }
        if state
            .configured_outputs
            .insert(
                contract.name.clone(),
                ConfiguredOutputRegistration {
                    manifest: manifest.clone(),
                    liveness: Arc::downgrade(&liveness),
                },
            )
            .is_some()
        {
            return Err(ComponentBuildError::DuplicateConfiguredObservable(
                contract.name.clone(),
            ));
        }
        drop(state);

        Ok(ConfiguredObservable {
            manifest,
            reference,
            observable,
            emitter,
            state: Arc::new(Mutex::new(PublisherState {
                next_sequence: 0,
                last_sequence: None,
                ended: false,
            })),
            owner: Arc::downgrade(&self.inner),
            _liveness: liveness,
        })
    }

    /// Replaces one exposed, configured Output while preserving the stable
    /// Component and Observable interface.
    ///
    /// The replacement receives a distinct Output identity. The Catalog is
    /// updated before the previous Output emits its terminal `Reconfigured`
    /// notice, so observers can immediately resolve the referenced successor.
    /// Existing Products remain bound to the previous Output; callers must
    /// explicitly attach new Products to [`ConfiguredObservableReplacement::replacement`].
    pub fn replace_configured_observable<T: ContractType + Send + Sync + 'static>(
        &self,
        current: &ConfiguredObservable<T>,
        spec: ConfiguredObservableSpec,
        effective_at_timestamp_ns: u64,
    ) -> Result<ConfiguredObservableReplacement<T>, ComponentBuildError> {
        let contract = self
            .inner
            .manifest
            .observables
            .iter()
            .find(|contract| contract.name == spec.interface)
            .ok_or_else(|| ComponentBuildError::UnknownObservable(spec.interface.clone()))?;
        if let Some(access) = contract
            .access
            .iter()
            .copied()
            .find(|access| *access != ObservationAccess::FollowNew)
        {
            return Err(ComponentBuildError::UnsupportedLiveAccess {
                interface: contract.name.clone(),
                access,
            });
        }
        if contract.datatype != spec.payload.datatype() || contract.schema != spec.payload.schema()
        {
            return Err(ComponentBuildError::ContractMismatch {
                interface: spec.interface,
                expected_datatype: contract.datatype.clone(),
                actual_datatype: spec.payload.datatype().to_owned(),
                expected_schema: contract.schema.clone(),
                actual_schema: spec.payload.schema().to_owned(),
            });
        }
        if T::DATATYPE != contract.datatype {
            return Err(ComponentBuildError::RustDatatypeMismatch {
                interface: contract.name.clone(),
                rust_datatype: T::DATATYPE.to_owned(),
                contract_datatype: contract.datatype.clone(),
            });
        }
        if !current.owner.ptr_eq(&Arc::downgrade(&self.inner))
            || current.reference.slot != contract.name
        {
            return Err(ComponentBuildError::NotCurrentOutput {
                interface: contract.name.clone(),
                output_id: current.reference.output_id.clone(),
            });
        }
        if current.reference.output_id == spec.output_id {
            return Err(ComponentBuildError::ReplacementOutputIdReused(
                spec.output_id,
            ));
        }

        let manifest = OutputManifest {
            schema: "auki.component-output-manifest/v1".to_owned(),
            peer_id: self.inner.reference.peer_id.clone(),
            component_id: self.inner.reference.component_id.clone(),
            component_manifest_hash: self.inner.reference.manifest_hash.clone(),
            slot: contract.name.clone(),
            output_id: spec.output_id,
            clock_id: spec.clock_id,
            spatial_frame_id: spec.spatial_frame_id,
            payload: spec.payload,
        };
        let reference = manifest.reference();
        let (observable, emitter) = output_observable(reference.clone(), contract.access.clone());
        let liveness = Arc::new(());
        let replacement = ConfiguredObservable {
            manifest: manifest.clone(),
            reference: reference.clone(),
            observable,
            emitter,
            state: Arc::new(Mutex::new(PublisherState {
                next_sequence: 0,
                last_sequence: None,
                ended: false,
            })),
            owner: Arc::downgrade(&self.inner),
            _liveness: Arc::clone(&liveness),
        };

        let mut publisher_state = current.state.lock().unwrap();
        if publisher_state.ended {
            return Err(ComponentBuildError::OutputAlreadyEnded(
                current.reference.output_id.clone(),
            ));
        }
        let mut component_state = self.inner.state.lock().unwrap();
        if !component_state.exposed {
            return Err(ComponentBuildError::NotExposed);
        }
        let registered = component_state
            .configured_outputs
            .get(&contract.name)
            .ok_or_else(|| ComponentBuildError::NotCurrentOutput {
                interface: contract.name.clone(),
                output_id: current.reference.output_id.clone(),
            })?;
        if registered.manifest.reference() != current.reference {
            return Err(ComponentBuildError::NotCurrentOutput {
                interface: contract.name.clone(),
                output_id: current.reference.output_id.clone(),
            });
        }

        self.inner.catalog.set_current_output(manifest.clone())?;
        component_state.configured_outputs.insert(
            contract.name.clone(),
            ConfiguredOutputRegistration {
                manifest,
                liveness: Arc::downgrade(&liveness),
            },
        );
        publisher_state.ended = true;
        let previous_end = ObservationEnd {
            output: current.reference.clone(),
            last_sequence: publisher_state.last_sequence,
            timestamp_ns: effective_at_timestamp_ns,
            reason: ObservationEndReason::Reconfigured {
                replacement: Some(reference),
            },
        };
        drop(component_state);
        drop(publisher_state);
        current.emitter.emit(
            effective_at_timestamp_ns,
            ObservationEvent::Ended(previous_end.clone()),
        );

        Ok(ConfiguredObservableReplacement {
            previous_end,
            replacement,
        })
    }

    pub fn operable<I: ContractType, R: ContractType>(
        &self,
        name: &str,
        authorizer: impl Fn(&InvocationContext) -> bool + Send + Sync + 'static,
        handler: impl Fn(&InvocationContext, I) -> Result<R, InvocationError> + Send + Sync + 'static,
    ) -> Result<Operable<I, R>, ComponentBuildError> {
        self.operable_ordered(name, InvocationOrdering::Concurrent, authorizer, handler)
    }

    pub fn operable_ordered<I: ContractType, R: ContractType>(
        &self,
        name: &str,
        ordering: InvocationOrdering,
        authorizer: impl Fn(&InvocationContext) -> bool + Send + Sync + 'static,
        handler: impl Fn(&InvocationContext, I) -> Result<R, InvocationError> + Send + Sync + 'static,
    ) -> Result<Operable<I, R>, ComponentBuildError> {
        let contract = self
            .inner
            .manifest
            .operables
            .iter()
            .find(|contract| contract.name == name)
            .ok_or_else(|| ComponentBuildError::UnknownOperable(name.to_owned()))?;
        if contract.instruction != I::DATATYPE || contract.result != R::DATATYPE {
            return Err(ComponentBuildError::OperationContractMismatch {
                interface: contract.name.clone(),
                expected_instruction: contract.instruction.clone(),
                actual_instruction: I::DATATYPE.to_owned(),
                expected_result: contract.result.clone(),
                actual_result: R::DATATYPE.to_owned(),
            });
        }
        let mut state = self.inner.state.lock().unwrap();
        if state.exposed {
            return Err(ComponentBuildError::AlreadyExposed);
        }
        if state.live_operables.contains_key(name) {
            return Err(ComponentBuildError::DuplicateOperable(name.to_owned()));
        }
        let operable = Operable::new_ordered(
            name,
            self.inner.reference.clone(),
            contract.exposure,
            ordering,
            authorizer,
            handler,
        );
        state
            .live_operables
            .insert(name.to_owned(), operable.liveness());
        drop(state);
        Ok(operable)
    }

    pub fn local_operable<I, R>(
        &self,
        name: impl Into<Arc<str>>,
        authorizer: impl Fn(&InvocationContext) -> bool + Send + Sync + 'static,
        handler: impl Fn(&InvocationContext, I) -> Result<R, InvocationError> + Send + Sync + 'static,
    ) -> Operable<I, R> {
        Operable::new(
            name,
            self.inner.reference.clone(),
            Exposure::Local,
            authorizer,
            handler,
        )
    }

    /// Atomically projects this live Component and all of its configured
    /// cluster interfaces into the read-only Catalog.
    pub fn expose(&self) -> Result<(), ComponentBuildError> {
        let mut state = self.inner.state.lock().unwrap();
        if state.exposed {
            return Err(ComponentBuildError::AlreadyExposed);
        }
        for contract in &self.inner.manifest.observables {
            let Some(configured) = state.configured_outputs.get(&contract.name) else {
                return Err(ComponentBuildError::MissingObservable(
                    contract.name.clone(),
                ));
            };
            if configured.liveness.upgrade().is_none() {
                return Err(ComponentBuildError::DroppedObservable(
                    contract.name.clone(),
                ));
            }
        }
        for contract in &self.inner.manifest.operables {
            let Some(liveness) = state.live_operables.get(&contract.name) else {
                return Err(ComponentBuildError::MissingOperable(contract.name.clone()));
            };
            if liveness.upgrade().is_none() {
                return Err(ComponentBuildError::MissingOperable(contract.name.clone()));
            }
        }

        self.inner
            .catalog
            .register_component(self.inner.manifest.clone())?;
        for output in state.configured_outputs.values() {
            self.inner
                .catalog
                .set_current_output(output.manifest.clone())?;
        }
        state.exposed = true;
        Ok(())
    }
}

struct PublisherState {
    next_sequence: u64,
    last_sequence: Option<u64>,
    ended: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublishError {
    Ended,
}

impl fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("configured Observable has ended")
    }
}

impl std::error::Error for PublishError {}

/// One live, typed Observable bound to one immutable configured Output.
///
/// Clones share the publisher sequence and terminal state. Local observers
/// receive the same immutable payload allocation held by the returned
/// [`Observation`].
pub struct ConfiguredObservable<T> {
    manifest: OutputManifest,
    reference: OutputReference,
    observable: Observable<T>,
    emitter: ObservationEmitter<T>,
    state: Arc<Mutex<PublisherState>>,
    owner: std::sync::Weak<ComponentInner>,
    _liveness: Arc<()>,
}

/// The result of replacing one immutable configured Output with another.
pub struct ConfiguredObservableReplacement<T> {
    pub previous_end: ObservationEnd,
    pub replacement: ConfiguredObservable<T>,
}

impl<T> fmt::Debug for ConfiguredObservableReplacement<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfiguredObservableReplacement")
            .field("previous_end", &self.previous_end)
            .field("replacement", &self.replacement.reference)
            .finish()
    }
}

impl<T> Clone for ConfiguredObservable<T> {
    fn clone(&self) -> Self {
        Self {
            manifest: self.manifest.clone(),
            reference: self.reference.clone(),
            observable: self.observable.clone(),
            emitter: self.emitter.clone(),
            state: Arc::clone(&self.state),
            owner: self.owner.clone(),
            _liveness: Arc::clone(&self._liveness),
        }
    }
}

impl<T> fmt::Debug for ConfiguredObservable<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfiguredObservable")
            .field("reference", &self.reference)
            .finish_non_exhaustive()
    }
}

impl<T: Send + Sync + 'static> ConfiguredObservable<T> {
    pub fn manifest(&self) -> &OutputManifest {
        &self.manifest
    }

    pub fn reference(&self) -> &OutputReference {
        &self.reference
    }

    pub fn observable(&self) -> Observable<T> {
        self.observable.clone()
    }

    pub(crate) fn owner_is_exposed(&self) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|owner| owner.state.lock().unwrap().exposed)
    }

    pub fn publish(
        &self,
        timestamp_ns: u64,
        payload: Arc<T>,
    ) -> Result<(Observation<T>, PublishReport), PublishError> {
        let observation = {
            let mut state = self.state.lock().unwrap();
            if state.ended {
                return Err(PublishError::Ended);
            }
            let sequence = state.next_sequence;
            state.next_sequence = state.next_sequence.saturating_add(1);
            state.last_sequence = Some(sequence);
            Observation {
                output: self.reference.clone(),
                sequence,
                timestamp_ns,
                payload,
            }
        };
        let report = self.emitter.emit(
            timestamp_ns,
            ObservationEvent::Observation(observation.clone()),
        );
        Ok((observation, report))
    }

    pub fn end(
        &self,
        timestamp_ns: u64,
        reason: ObservationEndReason,
    ) -> Result<ObservationEnd, PublishError> {
        let notice = {
            let mut state = self.state.lock().unwrap();
            if state.ended {
                return Err(PublishError::Ended);
            }
            state.ended = true;
            ObservationEnd {
                output: self.reference.clone(),
                last_sequence: state.last_sequence,
                timestamp_ns,
                reason,
            }
        };
        self.emitter
            .emit(timestamp_ns, ObservationEvent::Ended(notice.clone()));
        Ok(notice)
    }
}
