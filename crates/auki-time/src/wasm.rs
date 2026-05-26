use crate::core;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TimeTransformJson {
    from_clock_id: String,
    to_clock_id: String,
    offset_ns: i64,
    uncertainty_ns: u64,
    observed_at_clock_ns: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct NtpExchangeJson {
    local_send_ns: i64,
    remote_receive_ns: i64,
    remote_send_ns: i64,
    local_receive_ns: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct NtpSampleJson {
    offset_ns: i64,
    uncertainty_ns: u64,
    round_trip_ns: u64,
    remote_processing_ns: u64,
    observed_at_clock_ns: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct ClockSyncConfigJson {
    max_samples_per_pair: u64,
    max_sample_age_ns: u64,
    max_uncertainty_ns: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ClockSyncObservationJson {
    local_clock_id: String,
    local_clock_hash: String,
    remote_clock_id: String,
    remote_clock_hash: String,
    sample: NtpSampleJson,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ClockTransformEstimateJson {
    from_clock_id: String,
    from_clock_hash: String,
    to_clock_id: String,
    to_clock_hash: String,
    offset_ns: i64,
    uncertainty_ns: u64,
    observed_at_clock_ns: i64,
    sample_count: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DomainClockDescriptorJson {
    cluster_name: String,
    domain_clock_id: String,
    domain_clock_hash: String,
    backing_peer_id: String,
    backing_clock_id: String,
    backing_clock_hash: String,
    backing_to_domain_offset_ns: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DomainClockEstimateJson {
    cluster_name: String,
    local_clock_id: String,
    local_clock_hash: String,
    domain_clock_id: String,
    domain_clock_hash: String,
    backing_peer_id: String,
    backing_clock_id: String,
    backing_clock_hash: String,
    peer_to_backing_offset_ns: i64,
    backing_to_domain_offset_ns: i64,
    total_offset_ns: i64,
    uncertainty_ns: u64,
    observed_at_clock_ns: i64,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct ConvertedTimestampJson {
    timestamp_ns: i64,
}

#[wasm_bindgen(js_name = defaultClockSyncConfigJson)]
pub fn default_clock_sync_config_json() -> String {
    to_json(&ClockSyncConfigJson::from(core::ClockSyncConfig::default()))
}

#[wasm_bindgen(js_name = timeTransformConvertNsJson)]
pub fn time_transform_convert_ns_json(
    transform_json: String,
    timestamp_ns: i64,
) -> Result<String, JsValue> {
    let transform = parse_time_transform(&transform_json)?;
    Ok(match transform.convert_ns(timestamp_ns) {
        Some(timestamp_ns) => to_json(&Some(ConvertedTimestampJson { timestamp_ns })),
        None => to_json(&Option::<ConvertedTimestampJson>::None),
    })
}

#[wasm_bindgen(js_name = computeNtpSampleJson)]
pub fn compute_ntp_sample_json(exchange_json: String) -> Result<String, JsValue> {
    let exchange = parse_ntp_exchange(&exchange_json)?;
    core::compute_ntp_sample(exchange)
        .map(|sample| to_json(&NtpSampleJson::from(sample)))
        .map_err(time_error)
}

#[wasm_bindgen(js_name = computeNtpOffsetJson)]
pub fn compute_ntp_offset_json(exchange_json: String) -> Result<i64, JsValue> {
    let exchange = parse_ntp_exchange(&exchange_json)?;
    core::compute_ntp_offset(exchange).map_err(time_error)
}

#[wasm_bindgen(js_name = selectBestNtpSampleJson)]
pub fn select_best_ntp_sample_json(samples_json: String) -> Result<String, JsValue> {
    let samples = serde_json::from_str::<Vec<NtpSampleJson>>(&samples_json)
        .map_err(json_error)?
        .into_iter()
        .map(Into::into)
        .collect::<Vec<_>>();
    Ok(to_json(
        &core::select_best_ntp_sample(&samples).map(NtpSampleJson::from),
    ))
}

#[wasm_bindgen(js_name = clockTransformEstimateIdentityJson)]
pub fn clock_transform_estimate_identity_json(
    clock_id: String,
    clock_hash: String,
    observed_at_clock_ns: i64,
) -> String {
    to_json(&ClockTransformEstimateJson::from(
        core::ClockTransformEstimate::identity(clock_id, clock_hash, observed_at_clock_ns),
    ))
}

#[wasm_bindgen(js_name = clockTransformEstimateTimeTransformJson)]
pub fn clock_transform_estimate_time_transform_json(
    estimate_json: String,
) -> Result<String, JsValue> {
    let estimate = parse_clock_transform_estimate(&estimate_json)?;
    Ok(to_json(&TimeTransformJson::from(estimate.time_transform())))
}

#[wasm_bindgen(js_name = estimateDomainClockJson)]
pub fn estimate_domain_clock_json(
    local_to_backing_json: String,
    descriptor_json: String,
) -> Result<String, JsValue> {
    let local_to_backing = parse_clock_transform_estimate(&local_to_backing_json)?;
    let descriptor = serde_json::from_str::<DomainClockDescriptorJson>(&descriptor_json)
        .map_err(json_error)?
        .into();
    core::estimate_domain_clock(local_to_backing, descriptor)
        .map(|estimate| to_json(&DomainClockEstimateJson::from(estimate)))
        .map_err(time_error)
}

#[wasm_bindgen(js_name = ClockSyncState)]
pub struct WasmClockSyncState {
    inner: core::ClockSyncState,
}

#[wasm_bindgen(js_class = ClockSyncState)]
impl WasmClockSyncState {
    #[wasm_bindgen(constructor)]
    pub fn new(config_json: String) -> Result<WasmClockSyncState, JsValue> {
        let config = serde_json::from_str::<ClockSyncConfigJson>(&config_json)
            .map_err(json_error)?
            .try_into()?;
        Ok(Self {
            inner: core::ClockSyncState::new(config),
        })
    }

    pub fn observe(&mut self, observation_json: String) -> Result<String, JsValue> {
        let observation = serde_json::from_str::<ClockSyncObservationJson>(&observation_json)
            .map_err(json_error)?
            .into();
        Ok(to_json(
            &self
                .inner
                .observe(observation)
                .map(ClockTransformEstimateJson::from),
        ))
    }

    pub fn estimate(
        &self,
        local_clock_id: String,
        remote_clock_id: String,
    ) -> Result<String, JsValue> {
        Ok(to_json(
            &self
                .inner
                .estimate(&local_clock_id, &remote_clock_id)
                .map(ClockTransformEstimateJson::from),
        ))
    }

    pub fn estimates(&self) -> String {
        to_json(
            &self
                .inner
                .estimates()
                .into_iter()
                .map(ClockTransformEstimateJson::from)
                .collect::<Vec<_>>(),
        )
    }
}

fn parse_time_transform(json: &str) -> Result<core::TimeTransform, JsValue> {
    serde_json::from_str::<TimeTransformJson>(json)
        .map_err(json_error)
        .map(Into::into)
}

fn parse_ntp_exchange(json: &str) -> Result<core::NtpExchange, JsValue> {
    serde_json::from_str::<NtpExchangeJson>(json)
        .map_err(json_error)
        .map(Into::into)
}

fn parse_clock_transform_estimate(json: &str) -> Result<core::ClockTransformEstimate, JsValue> {
    serde_json::from_str::<ClockTransformEstimateJson>(json)
        .map_err(json_error)
        .and_then(TryInto::try_into)
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("binding records serialize")
}

fn json_error(err: serde_json::Error) -> JsValue {
    JsValue::from_str(&format!("JSON is not valid: {err}"))
}

fn time_error<E: std::fmt::Display>(err: E) -> JsValue {
    JsValue::from_str(&err.to_string())
}

impl From<TimeTransformJson> for core::TimeTransform {
    fn from(transform: TimeTransformJson) -> Self {
        Self::new(
            transform.from_clock_id,
            transform.to_clock_id,
            transform.offset_ns,
            transform.uncertainty_ns,
            transform.observed_at_clock_ns,
        )
    }
}

impl From<core::TimeTransform> for TimeTransformJson {
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

impl From<NtpExchangeJson> for core::NtpExchange {
    fn from(exchange: NtpExchangeJson) -> Self {
        Self {
            local_send_ns: exchange.local_send_ns,
            remote_receive_ns: exchange.remote_receive_ns,
            remote_send_ns: exchange.remote_send_ns,
            local_receive_ns: exchange.local_receive_ns,
        }
    }
}

impl From<core::NtpSample> for NtpSampleJson {
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

impl From<NtpSampleJson> for core::NtpSample {
    fn from(sample: NtpSampleJson) -> Self {
        Self {
            offset_ns: sample.offset_ns,
            uncertainty_ns: sample.uncertainty_ns,
            round_trip_ns: sample.round_trip_ns,
            remote_processing_ns: sample.remote_processing_ns,
            observed_at_clock_ns: sample.observed_at_clock_ns,
        }
    }
}

impl From<core::ClockSyncConfig> for ClockSyncConfigJson {
    fn from(config: core::ClockSyncConfig) -> Self {
        Self {
            max_samples_per_pair: config.max_samples_per_pair as u64,
            max_sample_age_ns: config.max_sample_age_ns,
            max_uncertainty_ns: config.max_uncertainty_ns,
        }
    }
}

impl TryFrom<ClockSyncConfigJson> for core::ClockSyncConfig {
    type Error = JsValue;

    fn try_from(config: ClockSyncConfigJson) -> Result<Self, Self::Error> {
        Ok(Self {
            max_samples_per_pair: usize::try_from(config.max_samples_per_pair)
                .map_err(|_| JsValue::from_str("max_samples_per_pair does not fit usize"))?,
            max_sample_age_ns: config.max_sample_age_ns,
            max_uncertainty_ns: config.max_uncertainty_ns,
        })
    }
}

impl From<ClockSyncObservationJson> for core::ClockSyncObservation {
    fn from(observation: ClockSyncObservationJson) -> Self {
        Self::new(
            observation.local_clock_id,
            observation.local_clock_hash,
            observation.remote_clock_id,
            observation.remote_clock_hash,
            observation.sample.into(),
        )
    }
}

impl From<core::ClockTransformEstimate> for ClockTransformEstimateJson {
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

impl TryFrom<ClockTransformEstimateJson> for core::ClockTransformEstimate {
    type Error = JsValue;

    fn try_from(estimate: ClockTransformEstimateJson) -> Result<Self, Self::Error> {
        Ok(Self::new(
            estimate.from_clock_id,
            estimate.from_clock_hash,
            estimate.to_clock_id,
            estimate.to_clock_hash,
            estimate.offset_ns,
            estimate.uncertainty_ns,
            estimate.observed_at_clock_ns,
            usize::try_from(estimate.sample_count)
                .map_err(|_| JsValue::from_str("sample_count does not fit usize"))?,
        ))
    }
}

impl From<DomainClockDescriptorJson> for core::DomainClockDescriptor {
    fn from(descriptor: DomainClockDescriptorJson) -> Self {
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

impl From<core::DomainClockEstimate> for DomainClockEstimateJson {
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
