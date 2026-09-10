use std::fmt;
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::ComponentRuntime;
use crate::buffer::{Buffer, BufferError, BufferLimits, BufferRange};
use crate::component::{
    CatalogError, Observation, ObservationAccess, ObservationDelivery, ObservationEnd,
    ObservationError, ObservationEvent, ObservationHandle, OutputManifest, ProductForm,
    ProductManifest, ProductReference, ProductState, SerializedInMemoryTransport,
    observation_input,
};
use crate::episode::{Episode, EpisodeError, EpisodeState};
use crate::ports::Envelope;
use crate::runtime::ConfiguredObservable;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimeRangeRequest {
    pub clock_id: String,
    pub start_ns: u64,
    pub end_ns: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FiniteObservations<T> {
    pub observations: Vec<Observation<T>>,
}

impl<T> FiniteObservations<T> {
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }

    pub fn len(&self) -> usize {
        self.observations.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProductAccessError {
    UnsupportedRequest(ObservationAccess),
    InvalidTimeRange { start_ns: u64, end_ns: u64 },
    ClockMismatch { expected: String, requested: String },
    Transport(String),
}

impl fmt::Display for ProductAccessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRequest(access) => {
                write!(formatter, "Product does not support {access:?}")
            }
            Self::InvalidTimeRange { start_ns, end_ns } => {
                write!(
                    formatter,
                    "invalid time range: {start_ns} is after {end_ns}"
                )
            }
            Self::ClockMismatch {
                expected,
                requested,
            } => write!(
                formatter,
                "time range uses clock {requested}, but Product timestamps use {expected}"
            ),
            Self::Transport(error) => write!(formatter, "transport serialization failed: {error}"),
        }
    }
}

impl std::error::Error for ProductAccessError {}

/// Typed retained-data access for one Buffer or Episode Product.
///
/// This deliberately is not a Component and does not implement `Observable`.
pub struct RetainedProduct<T> {
    pub manifest: ProductManifest,
    pub manifest_hash: String,
    pub producer: OutputManifest,
    pub buffer: Buffer<Observation<T>>,
}

impl<T> Clone for RetainedProduct<T> {
    fn clone(&self) -> Self {
        Self {
            manifest: self.manifest.clone(),
            manifest_hash: self.manifest_hash.clone(),
            producer: self.producer.clone(),
            buffer: self.buffer.clone(),
        }
    }
}

impl<T> fmt::Debug for RetainedProduct<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedProduct")
            .field("manifest", &self.manifest)
            .field("manifest_hash", &self.manifest_hash)
            .field("producer", &self.producer.reference())
            .field("range", &self.buffer.range())
            .finish()
    }
}

impl<T> RetainedProduct<T> {
    /// Construct a local mirror of an exact remote Buffer Product.
    ///
    /// The Product keeps its remote peer, Manifest hash, producer identity,
    /// source sequence, and source timestamps. It is intentionally not added
    /// as a locally-produced Catalog Product; a Component input binding embeds
    /// the exact remote Product and Output Manifests instead.
    pub fn imported_buffer(
        manifest: ProductManifest,
        manifest_hash: String,
        producer: OutputManifest,
        limits: BufferLimits,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<Self, ProductImportError> {
        if manifest.form != ProductForm::Buffer {
            return Err(ProductImportError::NotBuffer(manifest.form));
        }
        let expected_hash = manifest.hash();
        if manifest_hash != expected_hash {
            return Err(ProductImportError::ManifestHashMismatch {
                expected: expected_hash,
                actual: manifest_hash,
            });
        }
        if producer.reference() != manifest.producer {
            return Err(ProductImportError::ProducerMismatch);
        }
        let product_id = manifest.product_id.clone();
        Ok(Self {
            manifest,
            manifest_hash,
            producer,
            buffer: Buffer::with_limits(
                product_id,
                limits,
                move |observation: &Observation<T>| retained_size(&observation.payload),
            )?,
        })
    }

    pub fn reference(&self) -> ProductReference {
        ProductReference {
            peer_id: self.manifest.peer_id.clone(),
            product_id: self.manifest.product_id.clone(),
            manifest_hash: self.manifest_hash.clone(),
        }
    }

    pub fn buffer(&self) -> &Buffer<Observation<T>> {
        &self.buffer
    }

    pub fn supports(&self, access: ObservationAccess) -> bool {
        self.manifest.access.contains(&access)
    }

    pub fn latest_existing(&self) -> Result<Option<Observation<T>>, ProductAccessError> {
        self.require(ObservationAccess::LatestExisting)?;
        let Some(sequence) = self.buffer.range().last_sequence else {
            return Ok(None);
        };
        Ok(self
            .buffer
            .snapshot(sequence, sequence)
            .into_iter()
            .next()
            .map(|envelope| envelope.payload.clone()))
    }

    pub fn time_range(
        &self,
        request: TimeRangeRequest,
    ) -> Result<FiniteObservations<T>, ProductAccessError> {
        self.require(ObservationAccess::TimeRange)?;
        if request.start_ns > request.end_ns {
            return Err(ProductAccessError::InvalidTimeRange {
                start_ns: request.start_ns,
                end_ns: request.end_ns,
            });
        }
        if request.clock_id != self.producer.clock_id {
            return Err(ProductAccessError::ClockMismatch {
                expected: self.producer.clock_id.clone(),
                requested: request.clock_id,
            });
        }
        Ok(FiniteObservations {
            observations: self
                .buffer
                .snapshot_time_ns(request.start_ns, request.end_ns)
                .into_iter()
                .map(|envelope| envelope.payload.clone())
                .collect(),
        })
    }

    fn require(&self, access: ObservationAccess) -> Result<(), ProductAccessError> {
        if self.supports(access) {
            Ok(())
        } else {
            Err(ProductAccessError::UnsupportedRequest(access))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProductImportError {
    NotBuffer(ProductForm),
    ManifestHashMismatch { expected: String, actual: String },
    ProducerMismatch,
    Buffer(BufferError),
}

impl fmt::Display for ProductImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotBuffer(form) => {
                write!(formatter, "cannot mirror {form:?} as a Buffer Product")
            }
            Self::ManifestHashMismatch { expected, actual } => write!(
                formatter,
                "remote Product Manifest hash {actual} does not match canonical hash {expected}"
            ),
            Self::ProducerMismatch => {
                formatter.write_str("remote Product producer does not match its Output Manifest")
            }
            Self::Buffer(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProductImportError {}

impl From<BufferError> for ProductImportError {
    fn from(error: BufferError) -> Self {
        Self::Buffer(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProductCaptureError {
    Buffer(BufferError),
    Episode(EpisodeError),
    Observation(ObservationError),
    Catalog(CatalogError),
    ProducerNotExposed,
}

impl fmt::Display for ProductCaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Buffer(error) => error.fmt(formatter),
            Self::Episode(error) => error.fmt(formatter),
            Self::Observation(error) => error.fmt(formatter),
            Self::Catalog(error) => error.fmt(formatter),
            Self::ProducerNotExposed => {
                formatter.write_str("cannot expose Product before its producer Observable")
            }
        }
    }
}

impl std::error::Error for ProductCaptureError {}

impl From<BufferError> for ProductCaptureError {
    fn from(error: BufferError) -> Self {
        Self::Buffer(error)
    }
}

impl From<EpisodeError> for ProductCaptureError {
    fn from(error: EpisodeError) -> Self {
        Self::Episode(error)
    }
}

impl From<ObservationError> for ProductCaptureError {
    fn from(error: ObservationError) -> Self {
        Self::Observation(error)
    }
}

impl From<CatalogError> for ProductCaptureError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

struct BufferCaptureState<T> {
    product: RetainedProduct<T>,
    end: Option<ObservationEnd>,
    errors: Vec<String>,
}

/// A live Buffer Product attached to one exact configured Observable.
pub struct BufferProductCapture<T: Send + Sync + 'static> {
    state: Arc<Mutex<BufferCaptureState<T>>>,
    observation: ObservationHandle<T>,
    catalog: crate::Catalog,
}

impl<T: Send + Sync + 'static> BufferProductCapture<T> {
    pub fn product(&self) -> RetainedProduct<T> {
        self.state.lock().unwrap().product.clone()
    }

    pub fn end_notice(&self) -> Option<ObservationEnd> {
        self.state.lock().unwrap().end.clone()
    }

    pub fn errors(&self) -> Vec<String> {
        self.state.lock().unwrap().errors.clone()
    }

    pub fn cancel(&self) {
        self.observation.cancel();
        self.state.lock().unwrap().product.buffer.close();
    }

    /// Reconfigures the live Buffer Product's retention policy and updates its
    /// Catalog state after any immediate eviction.
    pub fn set_limits(&self, limits: BufferLimits) -> Result<BufferRange, ProductCaptureError> {
        let state = self.state.lock().unwrap();
        state.product.buffer.set_limits(limits)?;
        let range = state.product.buffer.range();
        let product_id = state.product.manifest.product_id.clone();
        drop(state);
        self.catalog.update_product_state(
            &product_id,
            ProductState::Buffer {
                entries: range.entries,
                at_entry_capacity: limits
                    .max_entries
                    .is_some_and(|limit| range.entries == limit),
                limits: Some(limits),
            },
        );
        Ok(range)
    }

    /// Permanently stops this capture and removes its Product from the Catalog.
    ///
    /// Existing `RetainedProduct` clones remain valid until their owners drop
    /// them, but the deleted Product can no longer be discovered or selected by
    /// new Catalog readers.
    pub fn delete(self) -> Result<ProductManifest, ProductCaptureError> {
        let product = self.product();
        self.catalog
            .unregister_product(&product.manifest.product_id)?;
        self.cancel();
        Ok(product.manifest)
    }
}

pub struct EpisodeProduct<T> {
    pub manifest: ProductManifest,
    pub manifest_hash: String,
    pub producer: OutputManifest,
    pub episode: Episode<Observation<T>>,
}

impl<T> Clone for EpisodeProduct<T> {
    fn clone(&self) -> Self {
        Self {
            manifest: self.manifest.clone(),
            manifest_hash: self.manifest_hash.clone(),
            producer: self.producer.clone(),
            episode: self.episode.clone(),
        }
    }
}

impl<T> fmt::Debug for EpisodeProduct<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EpisodeProduct")
            .field("manifest", &self.manifest)
            .field("manifest_hash", &self.manifest_hash)
            .field("producer", &self.producer.reference())
            .field("state", &self.episode.state())
            .finish()
    }
}

impl<T> EpisodeProduct<T> {
    pub fn state(&self) -> EpisodeState {
        self.episode.state()
    }

    pub fn observations(&self) -> Vec<Observation<T>> {
        self.episode
            .snapshot()
            .into_iter()
            .map(|envelope| envelope.payload.clone())
            .collect()
    }

    pub fn latest_existing(&self) -> Result<Option<Observation<T>>, ProductAccessError> {
        self.require(ObservationAccess::LatestExisting)?;
        Ok(self
            .episode
            .snapshot()
            .last()
            .map(|envelope| envelope.payload.clone()))
    }

    pub fn all_available(&self) -> Result<FiniteObservations<T>, ProductAccessError> {
        self.require(ObservationAccess::AllAvailable)?;
        Ok(FiniteObservations {
            observations: self.observations(),
        })
    }

    pub fn time_range(
        &self,
        request: TimeRangeRequest,
    ) -> Result<FiniteObservations<T>, ProductAccessError> {
        self.require(ObservationAccess::TimeRange)?;
        if request.start_ns > request.end_ns {
            return Err(ProductAccessError::InvalidTimeRange {
                start_ns: request.start_ns,
                end_ns: request.end_ns,
            });
        }
        if request.clock_id != self.producer.clock_id {
            return Err(ProductAccessError::ClockMismatch {
                expected: self.producer.clock_id.clone(),
                requested: request.clock_id,
            });
        }
        Ok(FiniteObservations {
            observations: self
                .episode
                .snapshot()
                .into_iter()
                .filter(|envelope| {
                    (request.start_ns..=request.end_ns).contains(&envelope.timestamp_ns)
                })
                .map(|envelope| envelope.payload.clone())
                .collect(),
        })
    }

    fn require(&self, access: ObservationAccess) -> Result<(), ProductAccessError> {
        if self.manifest.access.contains(&access) {
            Ok(())
        } else {
            Err(ProductAccessError::UnsupportedRequest(access))
        }
    }
}

struct EpisodeCaptureState<T> {
    product: EpisodeProduct<T>,
    errors: Vec<String>,
}

/// A live Episode Product which retains the complete observed interval until
/// explicitly concluded.
pub struct EpisodeProductCapture<T: Send + Sync + 'static> {
    state: Arc<Mutex<EpisodeCaptureState<T>>>,
    observation: Mutex<Option<ObservationHandle<T>>>,
    catalog: crate::Catalog,
}

impl<T: Send + Sync + 'static> EpisodeProductCapture<T> {
    pub fn product(&self) -> EpisodeProduct<T> {
        self.state.lock().unwrap().product.clone()
    }

    pub fn errors(&self) -> Vec<String> {
        self.state.lock().unwrap().errors.clone()
    }

    pub fn conclude(&self, end_timestamp_ns: u64) -> Result<(), ProductCaptureError> {
        let state = self.state.lock().unwrap();
        state.product.episode.conclude(end_timestamp_ns)?;
        let product_id = state.product.manifest.product_id.clone();
        let observations = state.product.episode.len();
        drop(state);
        if let Some(handle) = self.observation.lock().unwrap().take() {
            handle.cancel();
        }
        self.catalog.update_product_state(
            &product_id,
            ProductState::Episode {
                observations,
                concluded_at_ns: Some(end_timestamp_ns),
            },
        );
        Ok(())
    }
}

impl ComponentRuntime {
    pub fn capture_buffer<T: Send + Sync + 'static>(
        &self,
        product_id: impl Into<String>,
        output: &ConfiguredObservable<T>,
        limits: BufferLimits,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<BufferProductCapture<T>, ProductCaptureError> {
        if !output.owner_is_exposed() {
            return Err(ProductCaptureError::ProducerNotExposed);
        }
        let product_id = product_id.into();
        let manifest = ProductManifest {
            schema: "auki.product-manifest/v1".to_owned(),
            peer_id: self.peer_id().to_owned(),
            product_id: product_id.clone(),
            form: ProductForm::Buffer,
            producer: output.reference().clone(),
            access: vec![
                ObservationAccess::LatestExisting,
                ObservationAccess::TimeRange,
            ],
        };
        let product = RetainedProduct {
            manifest: manifest.clone(),
            manifest_hash: manifest.hash(),
            producer: output.manifest().clone(),
            buffer: Buffer::with_limits(
                product_id.clone(),
                limits,
                move |observation: &Observation<T>| retained_size(&observation.payload),
            )?,
        };
        self.catalog().register_product_with_state(
            manifest,
            ProductState::Buffer {
                entries: 0,
                at_entry_capacity: false,
                limits: Some(limits),
            },
        )?;
        let state = Arc::new(Mutex::new(BufferCaptureState {
            product,
            end: None,
            errors: Vec::new(),
        }));
        let input_state = Arc::clone(&state);
        let catalog = self.catalog().clone();
        let input = observation_input(format!("{product_id}.capture"), move |event| {
            let mut state = input_state.lock().unwrap();
            match event {
                ObservationEvent::Observation(observation) => {
                    if observation.output != state.product.manifest.producer {
                        state
                            .errors
                            .push("observation producer does not match Product".to_owned());
                        return;
                    }
                    if let Err(error) = state.product.buffer.append_shared(Arc::new(Envelope::new(
                        observation.sequence,
                        observation.timestamp_ns,
                        observation.clone(),
                    ))) {
                        state.errors.push(error.to_string());
                        return;
                    }
                    let range = state.product.buffer.range();
                    let limit = state.product.buffer.limits().max_entries;
                    catalog.update_product_state(
                        &state.product.manifest.product_id,
                        ProductState::Buffer {
                            entries: range.entries,
                            at_entry_capacity: limit.is_some_and(|limit| range.entries == limit),
                            limits: Some(state.product.buffer.limits()),
                        },
                    );
                }
                ObservationEvent::Ended(end) => {
                    state.product.buffer.close();
                    state.end = Some(end.clone());
                }
            }
        });
        let observation = output
            .observable()
            .follow_new(&input, ObservationDelivery::inline_every_selected())?;
        Ok(BufferProductCapture {
            state,
            observation,
            catalog: self.catalog().clone(),
        })
    }

    pub fn capture_episode<T: Send + Sync + 'static>(
        &self,
        product_id: impl Into<String>,
        output: &ConfiguredObservable<T>,
    ) -> Result<EpisodeProductCapture<T>, ProductCaptureError> {
        if !output.owner_is_exposed() {
            return Err(ProductCaptureError::ProducerNotExposed);
        }
        let product_id = product_id.into();
        let manifest = ProductManifest {
            schema: "auki.product-manifest/v1".to_owned(),
            peer_id: self.peer_id().to_owned(),
            product_id: product_id.clone(),
            form: ProductForm::Episode,
            producer: output.reference().clone(),
            access: vec![
                ObservationAccess::LatestExisting,
                ObservationAccess::TimeRange,
                ObservationAccess::AllAvailable,
            ],
        };
        let product = EpisodeProduct {
            manifest: manifest.clone(),
            manifest_hash: manifest.hash(),
            producer: output.manifest().clone(),
            episode: Episode::empty(product_id.clone()),
        };
        self.catalog().register_product(manifest)?;
        let state = Arc::new(Mutex::new(EpisodeCaptureState {
            product,
            errors: Vec::new(),
        }));
        let input_state = Arc::clone(&state);
        let catalog = self.catalog().clone();
        let input = observation_input(format!("{product_id}.capture"), move |event| {
            let mut state = input_state.lock().unwrap();
            match event {
                ObservationEvent::Observation(observation) => {
                    if observation.output != state.product.manifest.producer {
                        state
                            .errors
                            .push("observation producer does not match Product".to_owned());
                        return;
                    }
                    if let Err(error) =
                        state.product.episode.append_shared(Arc::new(Envelope::new(
                            observation.sequence,
                            observation.timestamp_ns,
                            observation.clone(),
                        )))
                    {
                        state.errors.push(error.to_string());
                        return;
                    }
                    catalog.update_product_state(
                        &state.product.manifest.product_id,
                        ProductState::Episode {
                            observations: state.product.episode.len(),
                            concluded_at_ns: None,
                        },
                    );
                }
                ObservationEvent::Ended(end) => {
                    if let Err(error) = state.product.episode.conclude(end.timestamp_ns) {
                        state.errors.push(error.to_string());
                    }
                    catalog.update_product_state(
                        &state.product.manifest.product_id,
                        ProductState::Episode {
                            observations: state.product.episode.len(),
                            concluded_at_ns: Some(end.timestamp_ns),
                        },
                    );
                }
            }
        });
        let observation = output
            .observable()
            .follow_new(&input, ObservationDelivery::inline_every_selected())?;
        Ok(EpisodeProductCapture {
            state,
            observation: Mutex::new(Some(observation)),
            catalog: self.catalog().clone(),
        })
    }
}

impl SerializedInMemoryTransport {
    pub fn latest_existing<T>(
        &self,
        product: &RetainedProduct<T>,
    ) -> Result<Option<Observation<T>>, ProductAccessError>
    where
        T: Serialize + DeserializeOwned,
    {
        let _: (String, ObservationAccess) = self
            .round_trip(&(
                product.manifest.product_id.clone(),
                ObservationAccess::LatestExisting,
            ))
            .map_err(ProductAccessError::Transport)?;
        let response = product.latest_existing()?;
        self.round_trip(&response)
            .map_err(ProductAccessError::Transport)
    }

    pub fn time_range<T>(
        &self,
        product: &RetainedProduct<T>,
        request: TimeRangeRequest,
    ) -> Result<FiniteObservations<T>, ProductAccessError>
    where
        T: Serialize + DeserializeOwned,
    {
        let (_, request): (String, TimeRangeRequest) = self
            .round_trip(&(product.manifest.product_id.clone(), request))
            .map_err(ProductAccessError::Transport)?;
        let response = product.time_range(request)?;
        self.round_trip(&response)
            .map_err(ProductAccessError::Transport)
    }
}
