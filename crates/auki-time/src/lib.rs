//! TimeTransform math, NTP-style samples, and the 1 Hz
//! `local_clock_read` sampler.
//!
//! Schema spec: [`../README.md`](../README.md).
//!
//! - [`auki_proto::time_transform::TimeTransformEntry`] is the
//!   per-sample payload (re-exported here for short call sites). Lives
//!   in [`auki-proto`](../auki-proto) since Step 6 of the
//!   migration (2026-05-08) — encoding switched from CBOR-via-ciborium
//!   to protobuf via prost; pre-migration `source` and `discontinuous`
//!   fields are gone (`source` moved to manifest, `discontinuous` is
//!   now reader-computed).
//! - [`TimeTransformSource`](auki_manifests::TimeTransformSource) is
//!   re-exported from `auki-manifests` (it's manifest metadata, not a
//!   per-entry field).
//! - [`tick`] is the unit-testable primitive: read three clocks, build
//!   one entry.
//! - [`Sampler`] wraps `tick` in a 1 Hz background thread for
//!   production use.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub use auki_logs;
use auki_registry::{ClockBody, ClockMeta, ClockRegistryEntry, Scope};

// Re-exports for short call sites at consumer crates.
pub use auki_manifests::TimeTransformSource;
pub use auki_proto::time_transform::TimeTransformEntry;

/// A fixed affine relationship from one clock to another.
///
/// `offset_ns` is `to_clock - from_clock`, so conversion is
/// `to_timestamp = from_timestamp + offset_ns`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeTransform {
    from_clock_id: String,
    to_clock_id: String,
    /// Offset to add to a timestamp in `from_clock_id` to express it
    /// in `to_clock_id`.
    pub offset_ns: i64,
    /// Conservative error bound for the offset estimate.
    pub uncertainty_ns: u64,
    /// Reading of `from_clock_id` when this transform was observed.
    pub observed_at_clock_ns: i64,
}

impl TimeTransform {
    pub fn new(
        from_clock_id: impl Into<String>,
        to_clock_id: impl Into<String>,
        offset_ns: i64,
        uncertainty_ns: u64,
        observed_at_clock_ns: i64,
    ) -> Self {
        Self {
            from_clock_id: from_clock_id.into(),
            to_clock_id: to_clock_id.into(),
            offset_ns,
            uncertainty_ns,
            observed_at_clock_ns,
        }
    }

    pub fn from_clock_id(&self) -> &str {
        &self.from_clock_id
    }

    pub fn to_clock_id(&self) -> &str {
        &self.to_clock_id
    }

    pub fn convert_ns(&self, timestamp_ns: i64) -> Option<i64> {
        timestamp_ns.checked_add(self.offset_ns)
    }
}

/// Four timestamps from one NTP-style heartbeat exchange.
///
/// Local timestamps are read from the clock being synchronized.
/// Remote timestamps are read from the clock being synchronized to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NtpExchange {
    /// Local clock when the request/heartbeat was sent.
    pub local_send_ns: i64,
    /// Remote clock when that frame was received.
    pub remote_receive_ns: i64,
    /// Remote clock when the echoing frame was sent.
    pub remote_send_ns: i64,
    /// Local clock when the echoing frame was received.
    pub local_receive_ns: i64,
}

/// One offset estimate from an [`NtpExchange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NtpSample {
    /// Estimated `remote_clock - local_clock` offset.
    pub offset_ns: i64,
    /// Round-trip time minus remote processing time.
    pub uncertainty_ns: u64,
    /// Local elapsed time across the whole exchange.
    pub round_trip_ns: u64,
    /// Remote elapsed time between receive and send.
    pub remote_processing_ns: u64,
    /// Local clock reading when the sample completed.
    pub observed_at_clock_ns: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NtpSampleError {
    LocalClockWentBackwards,
    RemoteClockWentBackwards,
    RoundTripShorterThanRemoteProcessing,
    OffsetOutOfRange,
}

impl std::fmt::Display for NtpSampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalClockWentBackwards => write!(f, "local clock went backwards"),
            Self::RemoteClockWentBackwards => write!(f, "remote clock went backwards"),
            Self::RoundTripShorterThanRemoteProcessing => {
                write!(f, "round trip shorter than remote processing time")
            }
            Self::OffsetOutOfRange => write!(f, "computed offset does not fit i64"),
        }
    }
}

impl std::error::Error for NtpSampleError {}

/// Compute one NTP-style offset sample.
///
/// The returned offset is `remote_clock - local_clock`, regardless of
/// whether the two clocks have similar epochs. This is why the formula
/// works for independent monotonic clocks that both started at zero in
/// different sessions.
pub fn compute_ntp_sample(exchange: NtpExchange) -> Result<NtpSample, NtpSampleError> {
    let local_elapsed = exchange
        .local_receive_ns
        .checked_sub(exchange.local_send_ns)
        .ok_or(NtpSampleError::LocalClockWentBackwards)?;
    if local_elapsed < 0 {
        return Err(NtpSampleError::LocalClockWentBackwards);
    }

    let remote_elapsed = exchange
        .remote_send_ns
        .checked_sub(exchange.remote_receive_ns)
        .ok_or(NtpSampleError::RemoteClockWentBackwards)?;
    if remote_elapsed < 0 {
        return Err(NtpSampleError::RemoteClockWentBackwards);
    }

    if local_elapsed < remote_elapsed {
        return Err(NtpSampleError::RoundTripShorterThanRemoteProcessing);
    }

    let offset = ((exchange.remote_receive_ns as i128 - exchange.local_send_ns as i128)
        + (exchange.remote_send_ns as i128 - exchange.local_receive_ns as i128))
        / 2;
    let offset_ns = i64::try_from(offset).map_err(|_| NtpSampleError::OffsetOutOfRange)?;

    Ok(NtpSample {
        offset_ns,
        uncertainty_ns: (local_elapsed - remote_elapsed) as u64,
        round_trip_ns: local_elapsed as u64,
        remote_processing_ns: remote_elapsed as u64,
        observed_at_clock_ns: exchange.local_receive_ns,
    })
}

pub fn compute_ntp_offset(exchange: NtpExchange) -> Result<i64, NtpSampleError> {
    Ok(compute_ntp_sample(exchange)?.offset_ns)
}

/// Select the best sample by lowest uncertainty, breaking ties by
/// newest local observation.
pub fn select_best_ntp_sample(samples: &[NtpSample]) -> Option<NtpSample> {
    samples.iter().copied().min_by(compare_ntp_sample_quality)
}

fn compare_ntp_sample_quality(a: &NtpSample, b: &NtpSample) -> std::cmp::Ordering {
    a.uncertainty_ns
        .cmp(&b.uncertainty_ns)
        .then_with(|| b.observed_at_clock_ns.cmp(&a.observed_at_clock_ns))
}

const DEFAULT_CLOCK_SYNC_MAX_SAMPLES_PER_PAIR: usize = 32;
const DEFAULT_CLOCK_SYNC_MAX_SAMPLE_AGE_NS: u64 = 10_000_000_000;
const DEFAULT_CLOCK_SYNC_MAX_UNCERTAINTY_NS: u64 = 500_000_000;

/// Policy for retaining heartbeat-derived NTP samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSyncConfig {
    /// Maximum accepted samples retained per ordered clock pair.
    pub max_samples_per_pair: usize,
    /// Maximum age of a retained sample in the local clock's nanoseconds.
    pub max_sample_age_ns: u64,
    /// Samples noisier than this are rejected before they enter the window.
    pub max_uncertainty_ns: u64,
}

impl Default for ClockSyncConfig {
    fn default() -> Self {
        Self {
            max_samples_per_pair: DEFAULT_CLOCK_SYNC_MAX_SAMPLES_PER_PAIR,
            max_sample_age_ns: DEFAULT_CLOCK_SYNC_MAX_SAMPLE_AGE_NS,
            max_uncertainty_ns: DEFAULT_CLOCK_SYNC_MAX_UNCERTAINTY_NS,
        }
    }
}

/// One clock-id/hash-tagged NTP sample ready for peer-clock sync state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockSyncObservation {
    pub local_clock_id: String,
    pub local_clock_hash: String,
    pub remote_clock_id: String,
    pub remote_clock_hash: String,
    pub sample: NtpSample,
}

impl ClockSyncObservation {
    pub fn new(
        local_clock_id: impl Into<String>,
        local_clock_hash: impl Into<String>,
        remote_clock_id: impl Into<String>,
        remote_clock_hash: impl Into<String>,
        sample: NtpSample,
    ) -> Self {
        Self {
            local_clock_id: local_clock_id.into(),
            local_clock_hash: local_clock_hash.into(),
            remote_clock_id: remote_clock_id.into(),
            remote_clock_hash: remote_clock_hash.into(),
            sample,
        }
    }
}

/// Best current transform estimate for one ordered clock pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockTransformEstimate {
    from_clock_id: String,
    from_clock_hash: String,
    to_clock_id: String,
    to_clock_hash: String,
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub observed_at_clock_ns: i64,
    pub sample_count: usize,
}

impl ClockTransformEstimate {
    pub fn identity(
        clock_id: impl Into<String>,
        clock_hash: impl Into<String>,
        observed_at_clock_ns: i64,
    ) -> Self {
        let clock_id = clock_id.into();
        let clock_hash = clock_hash.into();
        Self {
            from_clock_id: clock_id.clone(),
            from_clock_hash: clock_hash.clone(),
            to_clock_id: clock_id,
            to_clock_hash: clock_hash,
            offset_ns: 0,
            uncertainty_ns: 0,
            observed_at_clock_ns,
            sample_count: 0,
        }
    }

    pub fn from_clock_id(&self) -> &str {
        &self.from_clock_id
    }

    pub fn from_clock_hash(&self) -> &str {
        &self.from_clock_hash
    }

    pub fn to_clock_id(&self) -> &str {
        &self.to_clock_id
    }

    pub fn to_clock_hash(&self) -> &str {
        &self.to_clock_hash
    }

    pub fn time_transform(&self) -> TimeTransform {
        TimeTransform::new(
            self.from_clock_id.clone(),
            self.to_clock_id.clone(),
            self.offset_ns,
            self.uncertainty_ns,
            self.observed_at_clock_ns,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClockPairKey {
    local_clock_id: String,
    remote_clock_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClockPairSamples {
    local_clock_id: String,
    local_clock_hash: String,
    remote_clock_id: String,
    remote_clock_hash: String,
    samples: VecDeque<NtpSample>,
}

impl ClockPairSamples {
    fn new(observation: &ClockSyncObservation) -> Self {
        Self {
            local_clock_id: observation.local_clock_id.clone(),
            local_clock_hash: observation.local_clock_hash.clone(),
            remote_clock_id: observation.remote_clock_id.clone(),
            remote_clock_hash: observation.remote_clock_hash.clone(),
            samples: VecDeque::new(),
        }
    }

    fn update_clock_hashes(&mut self, observation: &ClockSyncObservation) {
        if self.local_clock_hash != observation.local_clock_hash
            || self.remote_clock_hash != observation.remote_clock_hash
        {
            self.local_clock_hash = observation.local_clock_hash.clone();
            self.remote_clock_hash = observation.remote_clock_hash.clone();
            self.samples.clear();
        }
    }

    fn push(&mut self, sample: NtpSample, config: ClockSyncConfig) {
        self.samples.push_back(sample);
        self.prune_stale(sample.observed_at_clock_ns, config.max_sample_age_ns);
        while self.samples.len() > config.max_samples_per_pair {
            self.samples.pop_front();
        }
    }

    fn prune_stale(&mut self, now_local_clock_ns: i64, max_sample_age_ns: u64) {
        self.samples
            .retain(|sample| sample_is_fresh(*sample, now_local_clock_ns, max_sample_age_ns));
    }

    fn estimate(&self) -> Option<ClockTransformEstimate> {
        let best = self
            .samples
            .iter()
            .copied()
            .min_by(compare_ntp_sample_quality)?;

        Some(ClockTransformEstimate {
            from_clock_id: self.local_clock_id.clone(),
            from_clock_hash: self.local_clock_hash.clone(),
            to_clock_id: self.remote_clock_id.clone(),
            to_clock_hash: self.remote_clock_hash.clone(),
            offset_ns: best.offset_ns,
            uncertainty_ns: best.uncertainty_ns,
            observed_at_clock_ns: best.observed_at_clock_ns,
            sample_count: self.samples.len(),
        })
    }
}

fn sample_is_fresh(sample: NtpSample, now_local_clock_ns: i64, max_sample_age_ns: u64) -> bool {
    let age_ns = now_local_clock_ns as i128 - sample.observed_at_clock_ns as i128;
    age_ns <= 0 || age_ns as u128 <= max_sample_age_ns as u128
}

/// Rolling NTP-sample state keyed by ordered local/remote clock pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockSyncState {
    config: ClockSyncConfig,
    pairs: HashMap<ClockPairKey, ClockPairSamples>,
}

impl Default for ClockSyncState {
    fn default() -> Self {
        Self::new(ClockSyncConfig::default())
    }
}

impl ClockSyncState {
    pub fn new(config: ClockSyncConfig) -> Self {
        Self {
            config,
            pairs: HashMap::new(),
        }
    }

    pub fn observe(&mut self, observation: ClockSyncObservation) -> Option<ClockTransformEstimate> {
        if self.config.max_samples_per_pair == 0
            || observation.sample.uncertainty_ns > self.config.max_uncertainty_ns
        {
            return None;
        }

        let key = ClockPairKey {
            local_clock_id: observation.local_clock_id.clone(),
            remote_clock_id: observation.remote_clock_id.clone(),
        };
        let pair = self
            .pairs
            .entry(key)
            .or_insert_with(|| ClockPairSamples::new(&observation));
        pair.update_clock_hashes(&observation);
        pair.push(observation.sample, self.config);
        pair.estimate()
    }

    pub fn estimate(
        &self,
        local_clock_id: &str,
        remote_clock_id: &str,
    ) -> Option<ClockTransformEstimate> {
        let key = ClockPairKey {
            local_clock_id: local_clock_id.to_string(),
            remote_clock_id: remote_clock_id.to_string(),
        };
        self.pairs.get(&key).and_then(ClockPairSamples::estimate)
    }

    pub fn estimates(&self) -> Vec<ClockTransformEstimate> {
        let mut estimates = self
            .pairs
            .values()
            .filter_map(ClockPairSamples::estimate)
            .collect::<Vec<_>>();
        estimates.sort_by(|a, b| {
            a.from_clock_id()
                .cmp(b.from_clock_id())
                .then_with(|| a.to_clock_id().cmp(b.to_clock_id()))
        });
        estimates
    }
}

/// Cloneable handle around [`ClockSyncState`] for runtime/event tasks.
#[derive(Debug, Clone)]
pub struct ClockSyncHandle {
    state: Arc<Mutex<ClockSyncState>>,
}

impl Default for ClockSyncHandle {
    fn default() -> Self {
        Self::new(ClockSyncConfig::default())
    }
}

impl ClockSyncHandle {
    pub fn new(config: ClockSyncConfig) -> Self {
        Self {
            state: Arc::new(Mutex::new(ClockSyncState::new(config))),
        }
    }

    pub fn observe(&self, observation: ClockSyncObservation) -> Option<ClockTransformEstimate> {
        self.state
            .lock()
            .expect("clock sync state lock")
            .observe(observation)
    }

    pub fn estimate(
        &self,
        local_clock_id: &str,
        remote_clock_id: &str,
    ) -> Option<ClockTransformEstimate> {
        self.state
            .lock()
            .expect("clock sync state lock")
            .estimate(local_clock_id, remote_clock_id)
    }

    pub fn estimates(&self) -> Vec<ClockTransformEstimate> {
        self.state
            .lock()
            .expect("clock sync state lock")
            .estimates()
    }
}

/// Stable domain-clock metadata supplied by the domain layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainClockDescriptor {
    pub cluster_name: String,
    pub domain_clock_id: String,
    pub domain_clock_hash: String,
    pub backing_peer_id: String,
    pub backing_clock_id: String,
    pub backing_clock_hash: String,
    pub backing_to_domain_offset_ns: i64,
}

impl DomainClockDescriptor {
    pub fn new(
        cluster_name: impl Into<String>,
        domain_clock_id: impl Into<String>,
        domain_clock_hash: impl Into<String>,
        backing_peer_id: impl Into<String>,
        backing_clock_id: impl Into<String>,
        backing_clock_hash: impl Into<String>,
        backing_to_domain_offset_ns: i64,
    ) -> Self {
        Self {
            cluster_name: cluster_name.into(),
            domain_clock_id: domain_clock_id.into(),
            domain_clock_hash: domain_clock_hash.into(),
            backing_peer_id: backing_peer_id.into(),
            backing_clock_id: backing_clock_id.into(),
            backing_clock_hash: backing_clock_hash.into(),
            backing_to_domain_offset_ns,
        }
    }
}

/// A composed estimate from a local clock into a cluster domain clock.
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl DomainClockEstimate {
    pub fn time_transform(&self) -> TimeTransform {
        TimeTransform::new(
            self.local_clock_id.clone(),
            self.domain_clock_id.clone(),
            self.total_offset_ns,
            self.uncertainty_ns,
            self.observed_at_clock_ns,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainClockEstimateError {
    BackingClockIdMismatch { expected: String, actual: String },
    BackingClockHashMismatch { expected: String, actual: String },
    TotalOffsetOutOfRange,
}

impl std::fmt::Display for DomainClockEstimateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BackingClockIdMismatch { expected, actual } => write!(
                f,
                "backing clock id mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::BackingClockHashMismatch { expected, actual } => write!(
                f,
                "backing clock hash mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::TotalOffsetOutOfRange => write!(f, "composed domain offset does not fit i64"),
        }
    }
}

impl std::error::Error for DomainClockEstimateError {}

pub fn estimate_domain_clock(
    local_to_backing: ClockTransformEstimate,
    descriptor: DomainClockDescriptor,
) -> Result<DomainClockEstimate, DomainClockEstimateError> {
    if local_to_backing.to_clock_id() != descriptor.backing_clock_id {
        return Err(DomainClockEstimateError::BackingClockIdMismatch {
            expected: descriptor.backing_clock_id,
            actual: local_to_backing.to_clock_id().to_string(),
        });
    }
    if local_to_backing.to_clock_hash() != descriptor.backing_clock_hash {
        return Err(DomainClockEstimateError::BackingClockHashMismatch {
            expected: descriptor.backing_clock_hash,
            actual: local_to_backing.to_clock_hash().to_string(),
        });
    }

    let total_offset_ns = local_to_backing
        .offset_ns
        .checked_add(descriptor.backing_to_domain_offset_ns)
        .ok_or(DomainClockEstimateError::TotalOffsetOutOfRange)?;

    Ok(DomainClockEstimate {
        cluster_name: descriptor.cluster_name,
        local_clock_id: local_to_backing.from_clock_id().to_string(),
        local_clock_hash: local_to_backing.from_clock_hash().to_string(),
        domain_clock_id: descriptor.domain_clock_id,
        domain_clock_hash: descriptor.domain_clock_hash,
        backing_peer_id: descriptor.backing_peer_id,
        backing_clock_id: local_to_backing.to_clock_id().to_string(),
        backing_clock_hash: local_to_backing.to_clock_hash().to_string(),
        peer_to_backing_offset_ns: local_to_backing.offset_ns,
        backing_to_domain_offset_ns: descriptor.backing_to_domain_offset_ns,
        total_offset_ns,
        uncertainty_ns: local_to_backing.uncertainty_ns,
        observed_at_clock_ns: local_to_backing.observed_at_clock_ns,
    })
}

/// A pair of clock readings the sampler can pull. Production reads `CLOCK_MONOTONIC`
/// and `CLOCK_REALTIME` via `clock_gettime`; tests script the readings.
pub trait Clock: Send + Sync {
    fn read_from_ns(&self) -> i64;
    fn read_to_ns(&self) -> i64;
}

/// `clock_gettime`-backed `Clock` reading from the host's `CLOCK_MONOTONIC` /
/// `CLOCK_REALTIME` pair.
pub struct SystemClock;

impl Clock for SystemClock {
    fn read_from_ns(&self) -> i64 {
        clock_gettime_ns(libc::CLOCK_MONOTONIC)
    }
    fn read_to_ns(&self) -> i64 {
        clock_gettime_ns(libc::CLOCK_REALTIME)
    }
}

fn clock_gettime_ns(clk: libc::clockid_t) -> i64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let rc = unsafe { libc::clock_gettime(clk, &mut ts) };
    debug_assert_eq!(
        rc,
        0,
        "clock_gettime failed; errno={}",
        std::io::Error::last_os_error()
    );
    (ts.tv_sec as i64).saturating_mul(1_000_000_000) + ts.tv_nsec as i64
}

/// Take one sample. Reads from-clock, to-clock, from-clock back-to-back
/// (see schema spec for the three-read protocol). Returns the
/// `(timestamp_ns, entry)` tuple ready for `Log::append`.
///
/// Step 6 simplification (2026-05-08): no more sampler state.
/// Discontinuity detection is the reader's responsibility — readers
/// compute `|offset_ns - prev_offset_ns| ≥ reader_threshold` against
/// their own tolerance, instead of consuming a bool baked into the
/// bytes by the writer's choice. `source` likewise moved to the
/// manifest.
pub fn tick(clock: &dyn Clock) -> (i64, TimeTransformEntry) {
    let m1 = clock.read_from_ns();
    let r = clock.read_to_ns();
    let m2 = clock.read_from_ns();

    let timestamp_ns = midpoint(m1, m2);
    let offset_ns = r.saturating_sub(timestamp_ns);
    let uncertainty_ns: u32 = m2.saturating_sub(m1).max(0).try_into().unwrap_or(u32::MAX);

    let entry = TimeTransformEntry {
        offset_ns,
        uncertainty_ns,
    };
    (timestamp_ns, entry)
}

fn midpoint(a: i64, b: i64) -> i64 {
    // Avoids overflow that `(a + b) / 2` can hit at i64 extremes.
    a + (b - a) / 2
}

/// SDK-owned session-monotonic clock identity and reader.
///
/// The first path segment of `clock_id` is the authoring peer id. The
/// `session_id` is encoded both in the id and as the registry epoch marker,
/// because a monotonic clock's zero point is only meaningful for one process
/// lifetime.
#[derive(Debug, Clone)]
pub struct SessionClock {
    registry_entry: ClockRegistryEntry,
    started: Instant,
}

impl SessionClock {
    pub fn new(
        peer_id: impl Into<String>,
        session_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        let peer_id = peer_id.into();
        let session_id = session_id.into();
        let name = name.into();
        let registry_entry = ClockRegistryEntry {
            clock_id: format!("{peer_id}/{session_id}/{name}"),
            body: ClockBody::MonotonicClock(ClockMeta {
                unit: "ns".into(),
                monotonic: true,
                epoch: Some(session_id),
                scope: Scope::DeviceLocal,
            }),
        };
        Self {
            registry_entry,
            started: Instant::now(),
        }
    }

    pub fn now_ns(&self) -> u64 {
        self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }

    pub fn now_i64_ns(&self) -> i64 {
        self.started.elapsed().as_nanos().min(i64::MAX as u128) as i64
    }

    pub fn clock_id(&self) -> &str {
        &self.registry_entry.clock_id
    }

    pub fn clock_hash(&self) -> String {
        self.registry_entry.hash()
    }

    pub fn registry_entry(&self) -> ClockRegistryEntry {
        self.registry_entry.clone()
    }
}

// `build_manifest` (renamed `build_time_transform_log_manifest`) moved to
// [`auki-manifests`] in Step 0 of the auki-proto migration.

/// Background sampler. Calls `tick` every `period`, appending each entry to
/// the log. Stops cleanly via the returned handle.
pub struct Sampler {
    handle: JoinHandle<auki_logs::Log<TimeTransformEntry>>,
    stop: Arc<AtomicBool>,
}

impl Sampler {
    /// Spawn the sampling thread. Calls `tick` every `period` and
    /// appends each entry to the log. Step 6 (2026-05-08) dropped the
    /// `discontinuity_threshold` parameter — readers compute
    /// discontinuity from neighboring entries on read.
    pub fn start(
        log: auki_logs::Log<TimeTransformEntry>,
        clock: Box<dyn Clock>,
        period: Duration,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            let mut log = log;
            while !stop_clone.load(Ordering::Relaxed) {
                let (timestamp_ns, entry) = tick(&*clock);
                if let Err(e) = log.append(timestamp_ns, &entry) {
                    eprintln!("auki-time: append failed: {e}");
                }
                thread::sleep(period);
            }
            // Best-effort flush before handing the log back.
            let _ = log.flush();
            log
        });

        Sampler { handle, stop }
    }

    /// Signal the thread to stop, join, and recover the log.
    pub fn stop(self) -> auki_logs::Log<TimeTransformEntry> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.join().expect("sampler thread panicked")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Test clock that pops scripted nanosecond readings on each call.
    struct ScriptedClock {
        from: Mutex<VecDeque<i64>>,
        to: Mutex<VecDeque<i64>>,
    }

    impl ScriptedClock {
        fn new(from: impl IntoIterator<Item = i64>, to: impl IntoIterator<Item = i64>) -> Self {
            Self {
                from: Mutex::new(from.into_iter().collect()),
                to: Mutex::new(to.into_iter().collect()),
            }
        }
    }

    impl Clock for ScriptedClock {
        fn read_from_ns(&self) -> i64 {
            self.from
                .lock()
                .unwrap()
                .pop_front()
                .expect("from-clock script exhausted")
        }
        fn read_to_ns(&self) -> i64 {
            self.to
                .lock()
                .unwrap()
                .pop_front()
                .expect("to-clock script exhausted")
        }
    }

    #[test]
    fn session_clock_builds_peer_anchored_registry_entry_with_epoch_marker() {
        let peer_id = "12D3KooWPeerExample";
        let clock = SessionClock::new(peer_id, "session-123", "monotonic");

        let entry = clock.registry_entry();
        assert_eq!(entry.clock_id, "12D3KooWPeerExample/session-123/monotonic");
        match &entry.body {
            ClockBody::MonotonicClock(meta) => {
                assert_eq!(meta.unit, "ns");
                assert!(meta.monotonic);
                assert_eq!(meta.scope, Scope::DeviceLocal);
                assert_eq!(meta.epoch.as_deref(), Some("session-123"));
            }
            ClockBody::UtcClock(_) => panic!("session clock must be monotonic"),
        }
        assert_eq!(clock.clock_hash(), entry.hash());
    }

    #[test]
    fn session_clock_now_is_monotonic() {
        let clock = SessionClock::new("12D3KooWPeerExample", "session-123", "monotonic");
        let a = clock.now_ns();
        let b = clock.now_ns();
        assert!(b >= a);
    }

    #[test]
    fn tick_computes_offset_uncertainty_and_timestamp() {
        // m1=1000, r=2_000_000_500, m2=1200 → midpoint=1100, offset=2_000_000_500-1100=1_999_999_400, uncertainty=200.
        let clock = ScriptedClock::new([1_000, 1_200], [2_000_000_500]);
        let (ts, entry) = tick(&clock);

        assert_eq!(ts, 1_100);
        assert_eq!(entry.offset_ns, 1_999_999_400);
        assert_eq!(entry.uncertainty_ns, 200);
    }

    #[test]
    fn time_transform_converts_from_clock_to_to_clock() {
        let transform = TimeTransform::new(
            "12D3KooWPeer/session-123/monotonic",
            "utc",
            1_999_999_400,
            200,
            1_100,
        );

        assert_eq!(transform.convert_ns(1_100), Some(2_000_000_500));
        assert_eq!(
            transform.from_clock_id(),
            "12D3KooWPeer/session-123/monotonic"
        );
        assert_eq!(transform.to_clock_id(), "utc");
    }

    fn clock_estimate(
        from_clock_id: &str,
        from_clock_hash: &str,
        to_clock_id: &str,
        to_clock_hash: &str,
        offset_ns: i64,
        uncertainty_ns: u64,
    ) -> ClockTransformEstimate {
        ClockTransformEstimate {
            from_clock_id: from_clock_id.into(),
            from_clock_hash: from_clock_hash.into(),
            to_clock_id: to_clock_id.into(),
            to_clock_hash: to_clock_hash.into(),
            offset_ns,
            uncertainty_ns,
            observed_at_clock_ns: 42,
            sample_count: 3,
        }
    }

    #[test]
    fn clock_transform_estimate_identity_has_zero_offset_and_uncertainty() {
        let estimate =
            ClockTransformEstimate::identity("peer/local/session-1/monotonic", "local-hash", 123);

        assert_eq!(estimate.from_clock_id(), "peer/local/session-1/monotonic");
        assert_eq!(estimate.from_clock_hash(), "local-hash");
        assert_eq!(estimate.to_clock_id(), "peer/local/session-1/monotonic");
        assert_eq!(estimate.to_clock_hash(), "local-hash");
        assert_eq!(estimate.offset_ns, 0);
        assert_eq!(estimate.uncertainty_ns, 0);
        assert_eq!(estimate.observed_at_clock_ns, 123);
        assert_eq!(estimate.sample_count, 0);
    }

    #[test]
    fn domain_clock_estimate_for_initial_manager_is_identity() {
        let local_clock = "peer/12D3Manager/session-1/monotonic";
        let local_to_backing =
            clock_estimate(local_clock, "clock-hash", local_clock, "clock-hash", 0, 0);
        let descriptor = DomainClockDescriptor::new(
            "cluster-a",
            "cluster-a/domain-clock",
            "domain-hash",
            "12D3Manager",
            local_clock,
            "clock-hash",
            0,
        );

        let estimate = estimate_domain_clock(local_to_backing, descriptor).unwrap();

        assert_eq!(estimate.cluster_name, "cluster-a");
        assert_eq!(estimate.local_clock_id, local_clock);
        assert_eq!(estimate.domain_clock_id, "cluster-a/domain-clock");
        assert_eq!(estimate.backing_peer_id, "12D3Manager");
        assert_eq!(estimate.peer_to_backing_offset_ns, 0);
        assert_eq!(estimate.backing_to_domain_offset_ns, 0);
        assert_eq!(estimate.total_offset_ns, 0);
        assert_eq!(estimate.uncertainty_ns, 0);
        assert_eq!(estimate.time_transform().convert_ns(123), Some(123));
    }

    #[test]
    fn domain_clock_estimate_composes_follower_to_backing_offset() {
        let local_to_backing = clock_estimate(
            "peer/12D3Follower/session-9/monotonic",
            "follower-hash",
            "peer/12D3Manager/session-1/monotonic",
            "manager-hash",
            1_000_000,
            80,
        );
        let descriptor = DomainClockDescriptor::new(
            "cluster-a",
            "cluster-a/domain-clock",
            "domain-hash",
            "12D3Manager",
            "peer/12D3Manager/session-1/monotonic",
            "manager-hash",
            250,
        );

        let estimate = estimate_domain_clock(local_to_backing, descriptor).unwrap();

        assert_eq!(
            estimate.local_clock_id,
            "peer/12D3Follower/session-9/monotonic"
        );
        assert_eq!(
            estimate.backing_clock_id,
            "peer/12D3Manager/session-1/monotonic"
        );
        assert_eq!(estimate.peer_to_backing_offset_ns, 1_000_000);
        assert_eq!(estimate.backing_to_domain_offset_ns, 250);
        assert_eq!(estimate.total_offset_ns, 1_000_250);
        assert_eq!(estimate.uncertainty_ns, 80);
        assert_eq!(
            estimate.time_transform().convert_ns(10_000),
            Some(1_010_250)
        );
    }

    #[test]
    fn domain_clock_estimate_rejects_backing_clock_id_mismatch() {
        let local_to_backing = clock_estimate(
            "peer/12D3Follower/session-9/monotonic",
            "follower-hash",
            "peer/12D3Manager/session-1/monotonic",
            "manager-hash",
            1_000_000,
            80,
        );
        let descriptor = DomainClockDescriptor::new(
            "cluster-a",
            "cluster-a/domain-clock",
            "domain-hash",
            "12D3Manager",
            "peer/12D3Other/session-1/monotonic",
            "manager-hash",
            0,
        );

        let err = estimate_domain_clock(local_to_backing, descriptor).unwrap_err();

        assert_eq!(
            err,
            DomainClockEstimateError::BackingClockIdMismatch {
                expected: "peer/12D3Other/session-1/monotonic".into(),
                actual: "peer/12D3Manager/session-1/monotonic".into(),
            }
        );
    }

    #[test]
    fn domain_clock_estimate_rejects_backing_clock_hash_mismatch() {
        let local_to_backing = clock_estimate(
            "peer/12D3Follower/session-9/monotonic",
            "follower-hash",
            "peer/12D3Manager/session-1/monotonic",
            "old-manager-hash",
            1_000_000,
            80,
        );
        let descriptor = DomainClockDescriptor::new(
            "cluster-a",
            "cluster-a/domain-clock",
            "domain-hash",
            "12D3Manager",
            "peer/12D3Manager/session-1/monotonic",
            "new-manager-hash",
            0,
        );

        let err = estimate_domain_clock(local_to_backing, descriptor).unwrap_err();

        assert_eq!(
            err,
            DomainClockEstimateError::BackingClockHashMismatch {
                expected: "new-manager-hash".into(),
                actual: "old-manager-hash".into(),
            }
        );
    }

    #[test]
    fn domain_clock_estimate_rejects_total_offset_overflow() {
        let local_to_backing = clock_estimate(
            "peer/12D3Follower/session-9/monotonic",
            "follower-hash",
            "peer/12D3Manager/session-1/monotonic",
            "manager-hash",
            i64::MAX,
            80,
        );
        let descriptor = DomainClockDescriptor::new(
            "cluster-a",
            "cluster-a/domain-clock",
            "domain-hash",
            "12D3Manager",
            "peer/12D3Manager/session-1/monotonic",
            "manager-hash",
            1,
        );

        let err = estimate_domain_clock(local_to_backing, descriptor).unwrap_err();

        assert_eq!(err, DomainClockEstimateError::TotalOffsetOutOfRange);
    }

    #[test]
    fn ntp_sample_estimates_remote_minus_local_offset_across_independent_monotonic_epochs() {
        let sample = compute_ntp_sample(NtpExchange {
            local_send_ns: 1_000,
            remote_receive_ns: 1_001_050,
            remote_send_ns: 1_001_080,
            local_receive_ns: 1_130,
        })
        .expect("valid NTP exchange");

        assert_eq!(sample.offset_ns, 1_000_000);
        assert_eq!(sample.uncertainty_ns, 100);
        assert_eq!(sample.round_trip_ns, 130);
        assert_eq!(sample.remote_processing_ns, 30);
        assert_eq!(sample.observed_at_clock_ns, 1_130);
    }

    #[test]
    fn best_ntp_sample_prefers_lowest_uncertainty_then_latest_observation() {
        let samples = [
            NtpSample {
                offset_ns: 10_000,
                uncertainty_ns: 80,
                round_trip_ns: 100,
                remote_processing_ns: 20,
                observed_at_clock_ns: 10,
            },
            NtpSample {
                offset_ns: 10_005,
                uncertainty_ns: 40,
                round_trip_ns: 55,
                remote_processing_ns: 15,
                observed_at_clock_ns: 20,
            },
            NtpSample {
                offset_ns: 10_006,
                uncertainty_ns: 40,
                round_trip_ns: 55,
                remote_processing_ns: 15,
                observed_at_clock_ns: 30,
            },
        ];

        assert_eq!(select_best_ntp_sample(&samples), Some(samples[2]));
    }

    #[test]
    fn ntp_sample_rejects_backwards_exchange() {
        let err = compute_ntp_sample(NtpExchange {
            local_send_ns: 2_000,
            remote_receive_ns: 1_001_050,
            remote_send_ns: 1_001_080,
            local_receive_ns: 1_900,
        })
        .expect_err("local receive before local send must be rejected");

        assert_eq!(err, NtpSampleError::LocalClockWentBackwards);
    }

    #[test]
    fn clock_sync_state_estimates_local_to_remote_transform_from_one_sample() {
        let mut sync = ClockSyncState::default();

        let estimate = sync
            .observe(ClockSyncObservation::new(
                "peer-a/session-1/monotonic",
                "hash-a",
                "peer-b/session-7/monotonic",
                "hash-b",
                NtpSample {
                    offset_ns: 1_000_000,
                    uncertainty_ns: 40,
                    round_trip_ns: 100,
                    remote_processing_ns: 60,
                    observed_at_clock_ns: 10_000,
                },
            ))
            .expect("accepted sample should produce an estimate");

        assert_eq!(estimate.from_clock_id(), "peer-a/session-1/monotonic");
        assert_eq!(estimate.from_clock_hash(), "hash-a");
        assert_eq!(estimate.to_clock_id(), "peer-b/session-7/monotonic");
        assert_eq!(estimate.to_clock_hash(), "hash-b");
        assert_eq!(estimate.offset_ns, 1_000_000);
        assert_eq!(estimate.uncertainty_ns, 40);
        assert_eq!(estimate.observed_at_clock_ns, 10_000);
        assert_eq!(estimate.sample_count, 1);
        assert_eq!(estimate.time_transform().convert_ns(123), Some(1_000_123));
    }

    #[test]
    fn clock_sync_state_accepts_independent_monotonic_epochs() {
        let sample = compute_ntp_sample(NtpExchange {
            local_send_ns: 5_000,
            remote_receive_ns: 1_000_005_050,
            remote_send_ns: 1_000_005_090,
            local_receive_ns: 5_130,
        })
        .expect("valid exchange across unrelated epochs");

        let mut sync = ClockSyncState::default();
        let estimate = sync
            .observe(ClockSyncObservation::new(
                "peer-a/session-1/monotonic",
                "hash-a",
                "peer-b/session-7/monotonic",
                "hash-b",
                sample,
            ))
            .expect("sample should produce an estimate");

        assert_eq!(estimate.offset_ns, 1_000_000_005);
        assert_eq!(
            estimate.time_transform().convert_ns(5_130),
            Some(1_000_005_135)
        );
    }

    #[test]
    fn clock_sync_state_prefers_lower_uncertainty_sample() {
        let mut sync = ClockSyncState::default();
        sync.observe(ClockSyncObservation::new(
            "peer-a/session-1/monotonic",
            "hash-a",
            "peer-b/session-7/monotonic",
            "hash-b",
            NtpSample {
                offset_ns: 10_000,
                uncertainty_ns: 90,
                round_trip_ns: 120,
                remote_processing_ns: 30,
                observed_at_clock_ns: 100,
            },
        ));

        let estimate = sync
            .observe(ClockSyncObservation::new(
                "peer-a/session-1/monotonic",
                "hash-a",
                "peer-b/session-7/monotonic",
                "hash-b",
                NtpSample {
                    offset_ns: 9_980,
                    uncertainty_ns: 20,
                    round_trip_ns: 50,
                    remote_processing_ns: 30,
                    observed_at_clock_ns: 200,
                },
            ))
            .expect("second sample should produce an estimate");

        assert_eq!(estimate.offset_ns, 9_980);
        assert_eq!(estimate.uncertainty_ns, 20);
        assert_eq!(estimate.sample_count, 2);
    }

    #[test]
    fn clock_sync_state_clears_pair_samples_when_clock_hash_changes() {
        let mut sync = ClockSyncState::default();
        sync.observe(ClockSyncObservation::new(
            "peer-a/session-1/monotonic",
            "hash-a",
            "peer-b/session-7/monotonic",
            "hash-b-old",
            NtpSample {
                offset_ns: 10_000,
                uncertainty_ns: 10,
                round_trip_ns: 20,
                remote_processing_ns: 10,
                observed_at_clock_ns: 100,
            },
        ));

        let estimate = sync
            .observe(ClockSyncObservation::new(
                "peer-a/session-1/monotonic",
                "hash-a",
                "peer-b/session-7/monotonic",
                "hash-b-new",
                NtpSample {
                    offset_ns: 20_000,
                    uncertainty_ns: 80,
                    round_trip_ns: 100,
                    remote_processing_ns: 20,
                    observed_at_clock_ns: 200,
                },
            ))
            .expect("new-hash sample should replace the old pair window");

        assert_eq!(estimate.to_clock_hash(), "hash-b-new");
        assert_eq!(estimate.offset_ns, 20_000);
        assert_eq!(estimate.sample_count, 1);
    }

    #[test]
    fn clock_sync_state_ages_out_stale_samples() {
        let mut sync = ClockSyncState::new(ClockSyncConfig {
            max_samples_per_pair: 8,
            max_sample_age_ns: 100,
            max_uncertainty_ns: u64::MAX,
        });
        sync.observe(ClockSyncObservation::new(
            "peer-a/session-1/monotonic",
            "hash-a",
            "peer-b/session-7/monotonic",
            "hash-b",
            NtpSample {
                offset_ns: 10_000,
                uncertainty_ns: 10,
                round_trip_ns: 20,
                remote_processing_ns: 10,
                observed_at_clock_ns: 1_000,
            },
        ));

        let estimate = sync
            .observe(ClockSyncObservation::new(
                "peer-a/session-1/monotonic",
                "hash-a",
                "peer-b/session-7/monotonic",
                "hash-b",
                NtpSample {
                    offset_ns: 20_000,
                    uncertainty_ns: 80,
                    round_trip_ns: 100,
                    remote_processing_ns: 20,
                    observed_at_clock_ns: 1_200,
                },
            ))
            .expect("fresh sample should remain after pruning");

        assert_eq!(estimate.offset_ns, 20_000);
        assert_eq!(estimate.sample_count, 1);
    }

    #[test]
    fn clock_sync_state_rejects_samples_above_uncertainty_limit() {
        let mut sync = ClockSyncState::new(ClockSyncConfig {
            max_samples_per_pair: 8,
            max_sample_age_ns: u64::MAX,
            max_uncertainty_ns: 50,
        });

        let estimate = sync.observe(ClockSyncObservation::new(
            "peer-a/session-1/monotonic",
            "hash-a",
            "peer-b/session-7/monotonic",
            "hash-b",
            NtpSample {
                offset_ns: 10_000,
                uncertainty_ns: 51,
                round_trip_ns: 100,
                remote_processing_ns: 49,
                observed_at_clock_ns: 100,
            },
        ));

        assert!(estimate.is_none());
        assert!(
            sync.estimate("peer-a/session-1/monotonic", "peer-b/session-7/monotonic")
                .is_none()
        );
    }

    #[test]
    fn clock_sync_state_returns_all_current_estimates() {
        let mut sync = ClockSyncState::default();
        sync.observe(ClockSyncObservation::new(
            "peer-a/session-1/monotonic",
            "hash-a",
            "peer-b/session-7/monotonic",
            "hash-b",
            NtpSample {
                offset_ns: 10_000,
                uncertainty_ns: 20,
                round_trip_ns: 40,
                remote_processing_ns: 20,
                observed_at_clock_ns: 100,
            },
        ));
        sync.observe(ClockSyncObservation::new(
            "peer-a/session-1/monotonic",
            "hash-a",
            "peer-c/session-2/monotonic",
            "hash-c",
            NtpSample {
                offset_ns: -5_000,
                uncertainty_ns: 30,
                round_trip_ns: 50,
                remote_processing_ns: 20,
                observed_at_clock_ns: 120,
            },
        ));

        let estimates = sync.estimates();

        assert_eq!(estimates.len(), 2);
        assert_eq!(estimates[0].to_clock_id(), "peer-b/session-7/monotonic");
        assert_eq!(estimates[0].offset_ns, 10_000);
        assert_eq!(estimates[1].to_clock_id(), "peer-c/session-2/monotonic");
        assert_eq!(estimates[1].offset_ns, -5_000);
    }

    #[test]
    fn clock_sync_handle_shares_state_across_clones() {
        let handle = ClockSyncHandle::default();
        let clone = handle.clone();

        handle.observe(ClockSyncObservation::new(
            "peer-a/session-1/monotonic",
            "hash-a",
            "peer-b/session-7/monotonic",
            "hash-b",
            NtpSample {
                offset_ns: 10_000,
                uncertainty_ns: 20,
                round_trip_ns: 40,
                remote_processing_ns: 20,
                observed_at_clock_ns: 100,
            },
        ));

        let estimate = clone
            .estimate("peer-a/session-1/monotonic", "peer-b/session-7/monotonic")
            .expect("clone should see samples observed through original handle");
        assert_eq!(estimate.offset_ns, 10_000);
        assert_eq!(clone.estimates().len(), 1);
    }

    // `build_manifest_contains_required_fields` moved to [`auki-manifests`]
    // (renamed `build_time_transform_log_manifest_contains_required_fields`)
    // in Step 0 of the auki-proto migration. Discontinuity-detection
    // tests dropped at Step 6 — `discontinuous` is a reader-side
    // computation now, no longer baked into the entry. Source-snake-case
    // test moved to [`auki-manifests`] alongside `TimeTransformSource`.
    // CBOR round-trip test dropped — entry encoding moved to prost in
    // [`auki-proto`](../auki-proto), where the round-trip test
    // also lives.

    #[test]
    fn sampler_writes_entries_then_stops_cleanly() {
        // Use ScriptedClock with plenty of pre-loaded readings so the sampler
        // doesn't run out before we stop it. Each tick consumes 2 from-reads + 1 to-read.
        const N: usize = 200;
        let from: Vec<i64> = (0..(2 * N) as i64).map(|i| i * 1_000).collect();
        let to: Vec<i64> = (0..N as i64)
            .map(|i| 5_000_000_000 + i * 1_000_000)
            .collect();
        let clock = Box::new(ScriptedClock::new(from, to));

        let dir = tempfile::tempdir().unwrap();
        let manifest = auki_manifests::build_time_transform_log_manifest(
            "test-app",
            "550e8400-e29b-41d4-a716-446655440000",
            "test/from",
            "fhash",
            "test/to",
            "thash",
            &auki_manifests::TimeTransformSource::LocalClockRead,
            Duration::from_millis(100),
            Duration::from_secs(60),
        );
        let log = auki_logs::Log::<TimeTransformEntry>::open(dir.path(), manifest).unwrap();

        let sampler = Sampler::start(log, clock, Duration::from_millis(5));
        thread::sleep(Duration::from_millis(50));
        let log = sampler.stop();
        drop(log); // release file handles before re-reading

        let reader = auki_logs::Log::<TimeTransformEntry>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert!(
            !entries.is_empty(),
            "sampler should have written at least one entry"
        );
        // Sample contents are pinned by auki-proto' round-trip tests;
        // here we just confirm the sampler wrote something readable.
        for e in &entries {
            // `uncertainty_ns` is bounded; ScriptedClock advances linearly.
            assert!(e.payload.uncertainty_ns < 1_000_000_000);
        }
    }
}
