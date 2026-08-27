use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::buffer::Buffer;
use crate::component::{
    Observation, ObservationAccess, OutputManifest, ProductManifest, SerializedInMemoryTransport,
};

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
#[derive(Clone)]
pub struct RetainedProduct<T> {
    pub manifest: ProductManifest,
    pub manifest_hash: String,
    pub producer: OutputManifest,
    pub buffer: Buffer<Observation<T>>,
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
