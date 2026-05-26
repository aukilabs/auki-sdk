use std::sync::{Arc, Mutex};

use crate::core;

uniffi::setup_scaffolding!();

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct TimeTransform {
    pub from_clock_id: String,
    pub to_clock_id: String,
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub observed_at_clock_ns: i64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct NtpExchange {
    pub local_send_ns: i64,
    pub remote_receive_ns: i64,
    pub remote_send_ns: i64,
    pub local_receive_ns: i64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct NtpSample {
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub round_trip_ns: u64,
    pub remote_processing_ns: u64,
    pub observed_at_clock_ns: i64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct ClockSyncConfig {
    pub max_samples_per_pair: u64,
    pub max_sample_age_ns: u64,
    pub max_uncertainty_ns: u64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct ClockSyncObservation {
    pub local_clock_id: String,
    pub local_clock_hash: String,
    pub remote_clock_id: String,
    pub remote_clock_hash: String,
    pub sample: NtpSample,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct ClockTransformEstimate {
    pub from_clock_id: String,
    pub from_clock_hash: String,
    pub to_clock_id: String,
    pub to_clock_hash: String,
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub observed_at_clock_ns: i64,
    pub sample_count: u64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DomainClockDescriptor {
    pub cluster_name: String,
    pub domain_clock_id: String,
    pub domain_clock_hash: String,
    pub backing_peer_id: String,
    pub backing_clock_id: String,
    pub backing_clock_hash: String,
    pub backing_to_domain_offset_ns: i64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DomainClockEstimate {
    pub cluster_name: String,
    pub local_clock_id: String,
    pub local_clock_hash: String,
    pub domain_clock_id: String,
    pub domain_clock_hash: String,
    pub backing_peer_id: String,
    pub backing_clock_id: String,
    pub backing_clock_hash: String,
    pub peer_to_backing_offset_ns: i64,
    pub backing_to_domain_offset_ns: i64,
    pub total_offset_ns: i64,
    pub uncertainty_ns: u64,
    pub observed_at_clock_ns: i64,
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum TimeBindingError {
    #[error("NTP sample error: {message}")]
    NtpSample { message: String },
    #[error("backing clock id mismatch: expected {expected:?}, got {actual:?}")]
    BackingClockIdMismatch { expected: String, actual: String },
    #[error("backing clock hash mismatch: expected {expected:?}, got {actual:?}")]
    BackingClockHashMismatch { expected: String, actual: String },
    #[error("composed domain offset does not fit i64")]
    TotalOffsetOutOfRange,
    #[error("sample count does not fit this platform")]
    SampleCountOutOfRange,
    #[error("max_samples_per_pair does not fit this platform")]
    MaxSamplesPerPairOutOfRange,
    #[error("clock sync state lock poisoned")]
    LockPoisoned,
    #[error("registry entry JSON is not valid UTF-8: {message}")]
    RegistryEntryUtf8 { message: String },
}

#[derive(uniffi::Object)]
pub struct ClockSyncState {
    inner: Mutex<core::ClockSyncState>,
}

#[uniffi::export]
impl ClockSyncState {
    #[uniffi::constructor]
    pub fn new(config: ClockSyncConfig) -> Result<Arc<Self>, TimeBindingError> {
        Ok(Arc::new(Self {
            inner: Mutex::new(core::ClockSyncState::new(config_to_core(config)?)),
        }))
    }

    pub fn observe(
        &self,
        observation: ClockSyncObservation,
    ) -> Result<Option<ClockTransformEstimate>, TimeBindingError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| TimeBindingError::LockPoisoned)?;
        Ok(state.observe(observation.into()).map(Into::into))
    }

    pub fn estimate(
        &self,
        local_clock_id: String,
        remote_clock_id: String,
    ) -> Result<Option<ClockTransformEstimate>, TimeBindingError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| TimeBindingError::LockPoisoned)?;
        Ok(state
            .estimate(&local_clock_id, &remote_clock_id)
            .map(Into::into))
    }

    pub fn estimates(&self) -> Result<Vec<ClockTransformEstimate>, TimeBindingError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| TimeBindingError::LockPoisoned)?;
        Ok(state.estimates().into_iter().map(Into::into).collect())
    }
}

#[derive(uniffi::Object)]
pub struct SessionClock {
    inner: core::SessionClock,
}

#[uniffi::export]
impl SessionClock {
    #[uniffi::constructor]
    pub fn new(peer_id: String, session_id: String, name: String) -> Arc<Self> {
        Arc::new(Self {
            inner: core::SessionClock::new(peer_id, session_id, name),
        })
    }

    pub fn now_ns(&self) -> u64 {
        self.inner.now_ns()
    }

    pub fn now_i64_ns(&self) -> i64 {
        self.inner.now_i64_ns()
    }

    pub fn clock_id(&self) -> String {
        self.inner.clock_id().to_string()
    }

    pub fn clock_hash(&self) -> String {
        self.inner.clock_hash()
    }

    pub fn registry_entry_json(&self) -> Result<String, TimeBindingError> {
        String::from_utf8(self.inner.registry_entry().canonical_bytes()).map_err(|err| {
            TimeBindingError::RegistryEntryUtf8 {
                message: err.to_string(),
            }
        })
    }
}

#[uniffi::export]
pub fn default_clock_sync_config() -> ClockSyncConfig {
    core::ClockSyncConfig::default().into()
}

#[uniffi::export]
pub fn time_transform_convert_ns(transform: TimeTransform, timestamp_ns: i64) -> Option<i64> {
    core::TimeTransform::from(transform).convert_ns(timestamp_ns)
}

#[uniffi::export]
pub fn compute_ntp_sample(exchange: NtpExchange) -> Result<NtpSample, TimeBindingError> {
    core::compute_ntp_sample(exchange.into())
        .map(Into::into)
        .map_err(Into::into)
}

#[uniffi::export]
pub fn compute_ntp_offset(exchange: NtpExchange) -> Result<i64, TimeBindingError> {
    core::compute_ntp_offset(exchange.into()).map_err(Into::into)
}

#[uniffi::export]
pub fn select_best_ntp_sample(samples: Vec<NtpSample>) -> Option<NtpSample> {
    let samples = samples.into_iter().map(Into::into).collect::<Vec<_>>();
    core::select_best_ntp_sample(&samples).map(Into::into)
}

#[uniffi::export]
pub fn clock_transform_estimate_identity(
    clock_id: String,
    clock_hash: String,
    observed_at_clock_ns: i64,
) -> ClockTransformEstimate {
    core::ClockTransformEstimate::identity(clock_id, clock_hash, observed_at_clock_ns).into()
}

#[uniffi::export]
pub fn clock_transform_estimate_time_transform(
    estimate: ClockTransformEstimate,
) -> Result<TimeTransform, TimeBindingError> {
    Ok(estimate_to_core(estimate)?.time_transform().into())
}

#[uniffi::export]
pub fn estimate_domain_clock(
    local_to_backing: ClockTransformEstimate,
    descriptor: DomainClockDescriptor,
) -> Result<DomainClockEstimate, TimeBindingError> {
    core::estimate_domain_clock(estimate_to_core(local_to_backing)?, descriptor.into())
        .map(Into::into)
        .map_err(Into::into)
}

fn config_to_core(config: ClockSyncConfig) -> Result<core::ClockSyncConfig, TimeBindingError> {
    Ok(core::ClockSyncConfig {
        max_samples_per_pair: usize::try_from(config.max_samples_per_pair)
            .map_err(|_| TimeBindingError::MaxSamplesPerPairOutOfRange)?,
        max_sample_age_ns: config.max_sample_age_ns,
        max_uncertainty_ns: config.max_uncertainty_ns,
    })
}

fn estimate_to_core(
    estimate: ClockTransformEstimate,
) -> Result<core::ClockTransformEstimate, TimeBindingError> {
    Ok(core::ClockTransformEstimate::new(
        estimate.from_clock_id,
        estimate.from_clock_hash,
        estimate.to_clock_id,
        estimate.to_clock_hash,
        estimate.offset_ns,
        estimate.uncertainty_ns,
        estimate.observed_at_clock_ns,
        usize::try_from(estimate.sample_count)
            .map_err(|_| TimeBindingError::SampleCountOutOfRange)?,
    ))
}

impl From<core::TimeTransform> for TimeTransform {
    fn from(transform: core::TimeTransform) -> Self {
        Self {
            from_clock_id: transform.from_clock_id().to_string(),
            to_clock_id: transform.to_clock_id().to_string(),
            offset_ns: transform.offset_ns,
            uncertainty_ns: transform.uncertainty_ns,
            observed_at_clock_ns: transform.observed_at_clock_ns,
        }
    }
}

impl From<TimeTransform> for core::TimeTransform {
    fn from(transform: TimeTransform) -> Self {
        Self::new(
            transform.from_clock_id,
            transform.to_clock_id,
            transform.offset_ns,
            transform.uncertainty_ns,
            transform.observed_at_clock_ns,
        )
    }
}

impl From<NtpExchange> for core::NtpExchange {
    fn from(exchange: NtpExchange) -> Self {
        Self {
            local_send_ns: exchange.local_send_ns,
            remote_receive_ns: exchange.remote_receive_ns,
            remote_send_ns: exchange.remote_send_ns,
            local_receive_ns: exchange.local_receive_ns,
        }
    }
}

impl From<core::NtpSample> for NtpSample {
    fn from(sample: core::NtpSample) -> Self {
        Self {
            offset_ns: sample.offset_ns,
            uncertainty_ns: sample.uncertainty_ns,
            round_trip_ns: sample.round_trip_ns,
            remote_processing_ns: sample.remote_processing_ns,
            observed_at_clock_ns: sample.observed_at_clock_ns,
        }
    }
}

impl From<NtpSample> for core::NtpSample {
    fn from(sample: NtpSample) -> Self {
        Self {
            offset_ns: sample.offset_ns,
            uncertainty_ns: sample.uncertainty_ns,
            round_trip_ns: sample.round_trip_ns,
            remote_processing_ns: sample.remote_processing_ns,
            observed_at_clock_ns: sample.observed_at_clock_ns,
        }
    }
}

impl From<core::ClockSyncConfig> for ClockSyncConfig {
    fn from(config: core::ClockSyncConfig) -> Self {
        Self {
            max_samples_per_pair: config.max_samples_per_pair as u64,
            max_sample_age_ns: config.max_sample_age_ns,
            max_uncertainty_ns: config.max_uncertainty_ns,
        }
    }
}

impl From<ClockSyncObservation> for core::ClockSyncObservation {
    fn from(observation: ClockSyncObservation) -> Self {
        Self::new(
            observation.local_clock_id,
            observation.local_clock_hash,
            observation.remote_clock_id,
            observation.remote_clock_hash,
            observation.sample.into(),
        )
    }
}

impl From<core::ClockTransformEstimate> for ClockTransformEstimate {
    fn from(estimate: core::ClockTransformEstimate) -> Self {
        Self {
            from_clock_id: estimate.from_clock_id().to_string(),
            from_clock_hash: estimate.from_clock_hash().to_string(),
            to_clock_id: estimate.to_clock_id().to_string(),
            to_clock_hash: estimate.to_clock_hash().to_string(),
            offset_ns: estimate.offset_ns,
            uncertainty_ns: estimate.uncertainty_ns,
            observed_at_clock_ns: estimate.observed_at_clock_ns,
            sample_count: estimate.sample_count as u64,
        }
    }
}

impl From<DomainClockDescriptor> for core::DomainClockDescriptor {
    fn from(descriptor: DomainClockDescriptor) -> Self {
        Self::new(
            descriptor.cluster_name,
            descriptor.domain_clock_id,
            descriptor.domain_clock_hash,
            descriptor.backing_peer_id,
            descriptor.backing_clock_id,
            descriptor.backing_clock_hash,
            descriptor.backing_to_domain_offset_ns,
        )
    }
}

impl From<core::DomainClockEstimate> for DomainClockEstimate {
    fn from(estimate: core::DomainClockEstimate) -> Self {
        Self {
            cluster_name: estimate.cluster_name,
            local_clock_id: estimate.local_clock_id,
            local_clock_hash: estimate.local_clock_hash,
            domain_clock_id: estimate.domain_clock_id,
            domain_clock_hash: estimate.domain_clock_hash,
            backing_peer_id: estimate.backing_peer_id,
            backing_clock_id: estimate.backing_clock_id,
            backing_clock_hash: estimate.backing_clock_hash,
            peer_to_backing_offset_ns: estimate.peer_to_backing_offset_ns,
            backing_to_domain_offset_ns: estimate.backing_to_domain_offset_ns,
            total_offset_ns: estimate.total_offset_ns,
            uncertainty_ns: estimate.uncertainty_ns,
            observed_at_clock_ns: estimate.observed_at_clock_ns,
        }
    }
}

impl From<core::NtpSampleError> for TimeBindingError {
    fn from(err: core::NtpSampleError) -> Self {
        Self::NtpSample {
            message: err.to_string(),
        }
    }
}

impl From<core::DomainClockEstimateError> for TimeBindingError {
    fn from(err: core::DomainClockEstimateError) -> Self {
        match err {
            core::DomainClockEstimateError::BackingClockIdMismatch { expected, actual } => {
                Self::BackingClockIdMismatch { expected, actual }
            }
            core::DomainClockEstimateError::BackingClockHashMismatch { expected, actual } => {
                Self::BackingClockHashMismatch { expected, actual }
            }
            core::DomainClockEstimateError::TotalOffsetOutOfRange => Self::TotalOffsetOutOfRange,
        }
    }
}
