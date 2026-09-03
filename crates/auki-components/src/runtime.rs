//! Public construction API for live Components.
//!
//! The important invariant is that Catalog projection follows construction of
//! typed runtime handles. Applications cannot mutate the Catalog directly.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::PublishReport;
use crate::buffer::{BufferReader, BufferReaderStats, CursorStart};
use crate::component::{
    Catalog, CatalogError, ComponentManifest, ComponentReference, ComponentRuntime, Exposure,
    InvocationContext, InvocationError, InvocationOrdering, Observable, ObservableContract,
    Observation, ObservationAccess, ObservationEmitter, ObservationEnd, ObservationEndReason,
    ObservationEvent, Operable, OperableContract, OutputManifest, OutputReference, PayloadContract,
    ProductForm, ProductInputBindingManifest, ProductInputContract, output_observable,
};
use crate::ports::InputPort;
use crate::product::RetainedProduct;

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
    pub product_inputs: Vec<ProductInputContract>,
    pub observables: Vec<ObservableContract>,
    pub operables: Vec<OperableContract>,
}

impl ComponentSpec {
    pub fn new(component_id: impl Into<String>) -> Self {
        Self {
            component_id: component_id.into(),
            product_inputs: Vec::new(),
            observables: Vec::new(),
            operables: Vec::new(),
        }
    }

    pub fn product_input(mut self, contract: ProductInputContract) -> Self {
        self.product_inputs.push(contract);
        self
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
    UnknownProductInput(String),
    UnsupportedLiveAccess {
        interface: String,
        access: ObservationAccess,
    },
    DuplicateConfiguredObservable(String),
    DuplicateOperable(String),
    DuplicateConfiguredProductInput(String),
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
    ProductInputContractMismatch {
        interface: String,
        expected_form: ProductForm,
        actual_form: ProductForm,
        expected_datatype: String,
        actual_datatype: String,
        expected_schema: String,
        actual_schema: String,
    },
    ProductProducerMismatch(String),
    MissingObservable(String),
    MissingOperable(String),
    MissingProductInput(String),
    DroppedObservable(String),
    DroppedProductInput(String),
    NotExposed,
    NotCurrentOutput {
        interface: String,
        output_id: String,
    },
    NotCurrentProductInput {
        interface: String,
        product_id: String,
    },
    ReplacementProductInputReused(String),
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
            Self::UnknownProductInput(name) => {
                write!(formatter, "Component does not declare Product input {name}")
            }
            Self::UnsupportedLiveAccess { interface, access } => write!(
                formatter,
                "fresh Observable {interface} cannot advertise retained request {access:?}"
            ),
            Self::DuplicateConfiguredObservable(name) => {
                write!(formatter, "Observable {name} is already configured")
            }
            Self::DuplicateOperable(name) => write!(formatter, "Operable {name} is already live"),
            Self::DuplicateConfiguredProductInput(name) => {
                write!(formatter, "Product input {name} is already configured")
            }
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
            Self::ProductInputContractMismatch {
                interface,
                expected_form,
                actual_form,
                expected_datatype,
                actual_datatype,
                expected_schema,
                actual_schema,
            } => write!(
                formatter,
                "Product input {interface} requires {expected_form:?} {expected_datatype}/{expected_schema}, but the bound Product is {actual_form:?} {actual_datatype}/{actual_schema}"
            ),
            Self::ProductProducerMismatch(product_id) => write!(
                formatter,
                "Product {product_id} producer metadata does not match its producer reference"
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
            Self::MissingProductInput(name) => write!(
                formatter,
                "cannot expose Component: Product input {name} is not live"
            ),
            Self::DroppedObservable(name) => write!(
                formatter,
                "cannot expose Component: Observable {name} was dropped before exposure"
            ),
            Self::DroppedProductInput(name) => write!(
                formatter,
                "cannot expose Component: Product input {name} was dropped before exposure"
            ),
            Self::NotExposed => formatter.write_str("Component is not exposed"),
            Self::NotCurrentOutput {
                interface,
                output_id,
            } => write!(
                formatter,
                "Output {output_id} is not the current configured Output for Observable {interface}"
            ),
            Self::NotCurrentProductInput {
                interface,
                product_id,
            } => write!(
                formatter,
                "Product {product_id} is not the current configured Product for input {interface}"
            ),
            Self::ReplacementProductInputReused(product_id) => write!(
                formatter,
                "replacement Product input must use a new Product; {product_id} is already current"
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
    configured_product_inputs: BTreeMap<String, ConfiguredProductInputRegistration>,
    configured_outputs: BTreeMap<String, ConfiguredOutputRegistration>,
    live_operables: BTreeMap<String, std::sync::Weak<()>>,
    exposed: bool,
}

struct ConfiguredProductInputRegistration {
    manifest: ProductInputBindingManifest,
    liveness: std::sync::Weak<()>,
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

fn validate_input_product<T>(
    owner: &ComponentInner,
    product: &RetainedProduct<T>,
) -> Result<(), ComponentBuildError> {
    if product.manifest.peer_id != owner.reference.peer_id {
        let expected = product.manifest.hash();
        if product.manifest_hash != expected {
            return Err(ComponentBuildError::Catalog(
                CatalogError::ProductManifestMismatch {
                    product_id: product.manifest.product_id.clone(),
                    expected,
                    actual: product.manifest_hash.clone(),
                },
            ));
        }
        return Ok(());
    }

    let catalog_product = owner
        .catalog
        .product(&product.manifest.product_id)
        .ok_or_else(|| {
            ComponentBuildError::Catalog(CatalogError::UnknownProduct(
                product.manifest.product_id.clone(),
            ))
        })?;
    if product.manifest_hash != catalog_product.manifest_hash
        || product.manifest != catalog_product.manifest
    {
        return Err(ComponentBuildError::Catalog(
            CatalogError::ProductManifestMismatch {
                product_id: product.manifest.product_id.clone(),
                expected: catalog_product.manifest_hash,
                actual: product.manifest_hash.clone(),
            },
        ));
    }
    Ok(())
}

/// A live unit of behavior hosted by one Peer runtime.
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

impl ComponentRuntime {
    pub fn component(&self, spec: ComponentSpec) -> Result<Component, ComponentBuildError> {
        if spec.component_id.is_empty() {
            return Err(ComponentBuildError::EmptyComponentId);
        }
        let mut names = BTreeSet::new();
        for contract in &spec.product_inputs {
            if contract.exposure != Exposure::Cluster {
                return Err(ComponentBuildError::LocalInterfaceInClusterManifest(
                    contract.name.clone(),
                ));
            }
            if !names.insert(("product_input", contract.name.as_str())) {
                return Err(ComponentBuildError::DuplicateInterface(
                    contract.name.clone(),
                ));
            }
        }
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
            product_inputs: spec.product_inputs,
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

    /// Binds one declared Product input to a retained Buffer Product and starts
    /// its typed reader. The returned handle owns the live relationship.
    ///
    /// The Component cannot be exposed unless every declared Product input has
    /// one live binding. The Catalog projects the immutable input contract and
    /// the concrete Product binding separately.
    pub fn configured_buffer_input<T: ContractType + Send + Sync + 'static>(
        &self,
        name: &str,
        product: &RetainedProduct<T>,
        start: CursorStart,
        input: &InputPort<Observation<T>>,
    ) -> Result<ConfiguredBufferInput<T>, ComponentBuildError> {
        let contract = self
            .inner
            .manifest
            .product_inputs
            .iter()
            .find(|contract| contract.name == name)
            .ok_or_else(|| ComponentBuildError::UnknownProductInput(name.to_owned()))?;
        if product.producer.reference() != product.manifest.producer {
            return Err(ComponentBuildError::ProductProducerMismatch(
                product.manifest.product_id.clone(),
            ));
        }
        let actual_datatype = product.producer.payload.datatype();
        let actual_schema = product.producer.payload.schema();
        if contract.form != ProductForm::Buffer
            || product.manifest.form != contract.form
            || contract.datatype != actual_datatype
            || contract.schema != actual_schema
        {
            return Err(ComponentBuildError::ProductInputContractMismatch {
                interface: contract.name.clone(),
                expected_form: contract.form,
                actual_form: product.manifest.form,
                expected_datatype: contract.datatype.clone(),
                actual_datatype: actual_datatype.to_owned(),
                expected_schema: contract.schema.clone(),
                actual_schema: actual_schema.to_owned(),
            });
        }
        if T::DATATYPE != contract.datatype {
            return Err(ComponentBuildError::RustDatatypeMismatch {
                interface: contract.name.clone(),
                rust_datatype: T::DATATYPE.to_owned(),
                contract_datatype: contract.datatype.clone(),
            });
        }
        validate_input_product(&self.inner, product)?;

        let manifest = ProductInputBindingManifest {
            schema: "auki.component-product-input-binding/v1".to_owned(),
            peer_id: self.inner.reference.peer_id.clone(),
            component_id: self.inner.reference.component_id.clone(),
            component_manifest_hash: self.inner.reference.manifest_hash.clone(),
            slot: contract.name.clone(),
            product: product.manifest.clone(),
            product_manifest_hash: product.manifest_hash.clone(),
            producer: product.producer.clone(),
        };
        let liveness = Arc::new(());
        let mut state = self.inner.state.lock().unwrap();
        if state.exposed {
            return Err(ComponentBuildError::AlreadyExposed);
        }
        if state.configured_product_inputs.contains_key(name) {
            return Err(ComponentBuildError::DuplicateConfiguredProductInput(
                name.to_owned(),
            ));
        }
        state.configured_product_inputs.insert(
            name.to_owned(),
            ConfiguredProductInputRegistration {
                manifest: manifest.clone(),
                liveness: Arc::downgrade(&liveness),
            },
        );
        drop(state);

        Ok(ConfiguredBufferInput {
            manifest,
            reader: BufferReader::start(product.buffer(), start, input),
            owner: Arc::downgrade(&self.inner),
            _liveness: liveness,
        })
    }

    /// Replaces one exposed Component's configured Buffer Product input.
    ///
    /// The Component and input slot remain stable while the binding manifest
    /// moves to a different, contract-compatible Product. The old reader stays
    /// alive until its [`ConfiguredBufferInput`] handle is dropped; dropping it
    /// after this method returns cannot clear the replacement Catalog binding.
    pub fn replace_configured_buffer_input<T: ContractType + Send + Sync + 'static>(
        &self,
        current: &ConfiguredBufferInput<T>,
        product: &RetainedProduct<T>,
        start: CursorStart,
        input: &InputPort<Observation<T>>,
    ) -> Result<ConfiguredBufferInput<T>, ComponentBuildError> {
        let name = &current.manifest.slot;
        let contract = self
            .inner
            .manifest
            .product_inputs
            .iter()
            .find(|contract| contract.name == *name)
            .ok_or_else(|| ComponentBuildError::UnknownProductInput(name.clone()))?;
        if !current.owner.ptr_eq(&Arc::downgrade(&self.inner)) {
            return Err(ComponentBuildError::NotCurrentProductInput {
                interface: name.clone(),
                product_id: current.manifest.product.product_id.clone(),
            });
        }
        if current.manifest.product.reference() == product.reference() {
            return Err(ComponentBuildError::ReplacementProductInputReused(
                product.manifest.product_id.clone(),
            ));
        }
        if product.producer.reference() != product.manifest.producer {
            return Err(ComponentBuildError::ProductProducerMismatch(
                product.manifest.product_id.clone(),
            ));
        }
        let actual_datatype = product.producer.payload.datatype();
        let actual_schema = product.producer.payload.schema();
        if contract.form != ProductForm::Buffer
            || product.manifest.form != contract.form
            || contract.datatype != actual_datatype
            || contract.schema != actual_schema
        {
            return Err(ComponentBuildError::ProductInputContractMismatch {
                interface: contract.name.clone(),
                expected_form: contract.form,
                actual_form: product.manifest.form,
                expected_datatype: contract.datatype.clone(),
                actual_datatype: actual_datatype.to_owned(),
                expected_schema: contract.schema.clone(),
                actual_schema: actual_schema.to_owned(),
            });
        }
        if T::DATATYPE != contract.datatype {
            return Err(ComponentBuildError::RustDatatypeMismatch {
                interface: contract.name.clone(),
                rust_datatype: T::DATATYPE.to_owned(),
                contract_datatype: contract.datatype.clone(),
            });
        }
        validate_input_product(&self.inner, product)?;

        let manifest = ProductInputBindingManifest {
            schema: "auki.component-product-input-binding/v1".to_owned(),
            peer_id: self.inner.reference.peer_id.clone(),
            component_id: self.inner.reference.component_id.clone(),
            component_manifest_hash: self.inner.reference.manifest_hash.clone(),
            slot: contract.name.clone(),
            product: product.manifest.clone(),
            product_manifest_hash: product.manifest_hash.clone(),
            producer: product.producer.clone(),
        };
        let liveness = Arc::new(());
        let mut state = self.inner.state.lock().unwrap();
        if !state.exposed {
            return Err(ComponentBuildError::NotExposed);
        }
        let registered = state.configured_product_inputs.get(name).ok_or_else(|| {
            ComponentBuildError::NotCurrentProductInput {
                interface: name.clone(),
                product_id: current.manifest.product.product_id.clone(),
            }
        })?;
        if registered.manifest.hash() != current.manifest.hash() {
            return Err(ComponentBuildError::NotCurrentProductInput {
                interface: name.clone(),
                product_id: current.manifest.product.product_id.clone(),
            });
        }

        self.inner
            .catalog
            .set_current_product_input(manifest.clone())?;
        state.configured_product_inputs.insert(
            name.clone(),
            ConfiguredProductInputRegistration {
                manifest: manifest.clone(),
                liveness: Arc::downgrade(&liveness),
            },
        );
        drop(state);

        Ok(ConfiguredBufferInput {
            manifest,
            reader: BufferReader::start(product.buffer(), start, input),
            owner: Arc::downgrade(&self.inner),
            _liveness: liveness,
        })
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
        for contract in &self.inner.manifest.product_inputs {
            let Some(configured) = state.configured_product_inputs.get(&contract.name) else {
                return Err(ComponentBuildError::MissingProductInput(
                    contract.name.clone(),
                ));
            };
            if configured.liveness.upgrade().is_none() {
                return Err(ComponentBuildError::DroppedProductInput(
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
        for input in state.configured_product_inputs.values() {
            self.inner
                .catalog
                .set_current_product_input(input.manifest.clone())?;
        }
        for output in state.configured_outputs.values() {
            self.inner
                .catalog
                .set_current_output(output.manifest.clone())?;
        }
        state.exposed = true;
        Ok(())
    }
}

/// One live, typed binding from a Component input to a retained Buffer Product.
///
/// Dropping this handle stops its `BufferReader`. The binding manifest is the
/// same value projected under the Component's `current_product_inputs` Catalog
/// field when the Component is exposed.
pub struct ConfiguredBufferInput<T> {
    manifest: ProductInputBindingManifest,
    reader: BufferReader<Observation<T>>,
    owner: std::sync::Weak<ComponentInner>,
    _liveness: Arc<()>,
}

impl<T> fmt::Debug for ConfiguredBufferInput<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfiguredBufferInput")
            .field("slot", &self.manifest.slot)
            .field("product_id", &self.manifest.product.product_id)
            .finish_non_exhaustive()
    }
}

impl<T: Send + Sync + 'static> ConfiguredBufferInput<T> {
    pub fn manifest(&self) -> &ProductInputBindingManifest {
        &self.manifest
    }

    pub fn stats(&self) -> BufferReaderStats {
        self.reader.stats()
    }
}

impl<T> Drop for ConfiguredBufferInput<T> {
    fn drop(&mut self) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        if owner.state.lock().unwrap().exposed {
            owner.catalog.clear_current_product_input(
                &self.manifest.component_id,
                &self.manifest.slot,
                &self.manifest.hash(),
            );
        }
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
