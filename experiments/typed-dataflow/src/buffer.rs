use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::ports::{
    ComponentError, Connection, ConnectionStats, DeliveryStatus, Endpoint, EndpointKind, Envelope,
    InputPort, OutputPort,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferLimits {
    pub max_entries: Option<usize>,
    pub max_bytes: Option<usize>,
    pub target_duration: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceTimestampPolicy {
    /// Every accepted source timestamp must be greater than the previous one.
    #[default]
    StrictlyIncreasing,
    /// Equal source timestamps are accepted; backwards timestamps are not.
    NonDecreasing,
    /// Source timestamps may arrive in any order. Duration eviction must then
    /// use arrival time, while source-time range queries remain a scan.
    Unordered,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DurationTimeBasis {
    #[default]
    SourceTimestamp,
    ArrivalTime,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BufferTimePolicy {
    pub source_timestamps: SourceTimestampPolicy,
    pub duration_basis: DurationTimeBasis,
}

impl BufferLimits {
    pub const fn entries(max_entries: usize) -> Self {
        Self {
            max_entries: Some(max_entries),
            max_bytes: None,
            target_duration: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BufferRange {
    pub first_sequence: Option<u64>,
    pub last_sequence: Option<u64>,
    pub first_timestamp_ns: Option<u64>,
    pub last_timestamp_ns: Option<u64>,
    pub entries: usize,
    /// Bytes reported by the Buffer's explicit payload-size accounting function.
    pub retained_payload_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorStart {
    /// Follow values published after the cursor is created.
    Latest,
    /// Return the newest retained value, then follow subsequent values.
    Current,
    /// Start from this source sequence or report a gap.
    FromSequence(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Gap {
    pub requested_sequence: u64,
    pub available_from: u64,
}

#[derive(Debug)]
pub enum CursorRead<T> {
    Item(Arc<Envelope<T>>),
    Gap(Gap),
    Timeout,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BufferError {
    MissingHardLimit,
    ZeroEntryLimit,
    ZeroByteLimit,
    PayloadExceedsByteLimit {
        payload_bytes: usize,
        limit: usize,
    },
    NonMonotonicSequence {
        previous: u64,
        incoming: u64,
    },
    NonMonotonicTimestamp {
        previous: u64,
        incoming: u64,
        policy: SourceTimestampPolicy,
    },
    UnorderedSourceDuration,
    Closed,
}

impl fmt::Display for BufferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHardLimit => {
                formatter.write_str("Buffer requires max_entries or max_bytes")
            }
            Self::ZeroEntryLimit => formatter.write_str("max_entries must be positive"),
            Self::ZeroByteLimit => formatter.write_str("max_bytes must be positive"),
            Self::PayloadExceedsByteLimit {
                payload_bytes,
                limit,
            } => write!(
                formatter,
                "payload retains {payload_bytes} bytes, exceeding Buffer limit {limit}"
            ),
            Self::NonMonotonicSequence { previous, incoming } => write!(
                formatter,
                "incoming sequence {incoming} is not newer than {previous}"
            ),
            Self::NonMonotonicTimestamp {
                previous,
                incoming,
                policy,
            } => write!(
                formatter,
                "incoming source timestamp {incoming} violates {policy:?} policy after {previous}"
            ),
            Self::UnorderedSourceDuration => formatter.write_str(
                "source-time duration eviction requires ordered source timestamps; use arrival time",
            ),
            Self::Closed => formatter.write_str("Buffer is closed"),
        }
    }
}

impl std::error::Error for BufferError {}

/// A bounded, in-memory retained data product.
pub struct Buffer<T> {
    inner: Arc<BufferInner<T>>,
}

struct Retained<T> {
    envelope: Arc<Envelope<T>>,
    bytes: usize,
    arrival: Instant,
}

struct BufferState<T> {
    entries: VecDeque<Retained<T>>,
    retained_bytes: usize,
    high_water: Option<u64>,
    last_source_timestamp_ns: Option<u64>,
    closed: bool,
}

struct BufferInner<T> {
    name: Arc<str>,
    limits: Mutex<BufferLimits>,
    retained_size: Arc<dyn Fn(&T) -> usize + Send + Sync>,
    time_policy: BufferTimePolicy,
    state: Mutex<BufferState<T>>,
    changed: Condvar,
}

impl<T> Clone for Buffer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> fmt::Debug for Buffer<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Buffer")
            .field("name", &self.inner.name)
            .field("limits", &self.limits())
            .field("range", &self.range())
            .finish()
    }
}

impl<T> Buffer<T> {
    pub fn new(name: impl Into<Arc<str>>, max_entries: usize) -> Result<Self, BufferError> {
        Self::with_limits(name, BufferLimits::entries(max_entries), |_| {
            std::mem::size_of::<T>()
        })
    }

    pub fn with_limits(
        name: impl Into<Arc<str>>,
        limits: BufferLimits,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<Self, BufferError> {
        Self::with_limits_and_time_policy(name, limits, BufferTimePolicy::default(), retained_size)
    }

    pub fn with_limits_and_time_policy(
        name: impl Into<Arc<str>>,
        limits: BufferLimits,
        time_policy: BufferTimePolicy,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<Self, BufferError> {
        validate_limits(limits)?;
        if limits.target_duration.is_some()
            && time_policy.duration_basis == DurationTimeBasis::SourceTimestamp
            && time_policy.source_timestamps == SourceTimestampPolicy::Unordered
        {
            return Err(BufferError::UnorderedSourceDuration);
        }
        Ok(Self {
            inner: Arc::new(BufferInner {
                name: name.into(),
                limits: Mutex::new(limits),
                retained_size: Arc::new(retained_size),
                time_policy,
                state: Mutex::new(BufferState {
                    entries: VecDeque::new(),
                    retained_bytes: 0,
                    high_water: None,
                    last_source_timestamp_ns: None,
                    closed: false,
                }),
                changed: Condvar::new(),
            }),
        })
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn limits(&self) -> BufferLimits {
        *self.inner.limits.lock().unwrap()
    }

    /// Reconfigures this Buffer's retention limits and immediately evicts
    /// entries that do not fit the replacement policy.
    pub fn set_limits(&self, limits: BufferLimits) -> Result<(), BufferError> {
        validate_limits(limits)?;
        if limits.target_duration.is_some()
            && self.inner.time_policy.duration_basis == DurationTimeBasis::SourceTimestamp
            && self.inner.time_policy.source_timestamps == SourceTimestampPolicy::Unordered
        {
            return Err(BufferError::UnorderedSourceDuration);
        }
        let mut current_limits = self.inner.limits.lock().unwrap();
        let mut state = self.inner.state.lock().unwrap();
        if state.closed {
            return Err(BufferError::Closed);
        }
        *current_limits = limits;
        evict_to_limits(&mut state, limits, self.inner.time_policy);
        drop(state);
        drop(current_limits);
        self.inner.changed.notify_all();
        Ok(())
    }

    pub fn time_policy(&self) -> BufferTimePolicy {
        self.inner.time_policy
    }

    /// Retains an already shared envelope without copying its payload.
    pub fn append_shared(&self, envelope: Arc<Envelope<T>>) -> Result<(), BufferError> {
        self.append_shared_at(envelope, Instant::now())
    }

    /// Deterministic arrival-time variant used by transport/storage tests.
    pub fn append_shared_at(
        &self,
        envelope: Arc<Envelope<T>>,
        arrival: Instant,
    ) -> Result<(), BufferError> {
        let bytes = (self.inner.retained_size)(&envelope.payload);
        let limits = self.inner.limits.lock().unwrap();
        if let Some(limit) = limits.max_bytes
            && bytes > limit
        {
            return Err(BufferError::PayloadExceedsByteLimit {
                payload_bytes: bytes,
                limit,
            });
        }

        let mut state = self.inner.state.lock().unwrap();
        if state.closed {
            return Err(BufferError::Closed);
        }
        if let Some(previous) = state.high_water
            && envelope.sequence <= previous
        {
            return Err(BufferError::NonMonotonicSequence {
                previous,
                incoming: envelope.sequence,
            });
        }

        if let Some(previous) = state.last_source_timestamp_ns {
            let invalid = match self.inner.time_policy.source_timestamps {
                SourceTimestampPolicy::StrictlyIncreasing => envelope.timestamp_ns <= previous,
                SourceTimestampPolicy::NonDecreasing => envelope.timestamp_ns < previous,
                SourceTimestampPolicy::Unordered => false,
            };
            if invalid {
                return Err(BufferError::NonMonotonicTimestamp {
                    previous,
                    incoming: envelope.timestamp_ns,
                    policy: self.inner.time_policy.source_timestamps,
                });
            }
        }

        state.high_water = Some(envelope.sequence);
        state.last_source_timestamp_ns = Some(envelope.timestamp_ns);
        state.retained_bytes += bytes;
        state.entries.push_back(Retained {
            envelope,
            bytes,
            arrival,
        });
        evict_to_limits(&mut state, *limits, self.inner.time_policy);
        drop(state);
        drop(limits);
        self.inner.changed.notify_all();
        Ok(())
    }

    pub fn range(&self) -> BufferRange {
        let state = self.inner.state.lock().unwrap();
        BufferRange {
            first_sequence: state.entries.front().map(|entry| entry.envelope.sequence),
            last_sequence: state.entries.back().map(|entry| entry.envelope.sequence),
            first_timestamp_ns: state
                .entries
                .front()
                .map(|entry| entry.envelope.timestamp_ns),
            last_timestamp_ns: state
                .entries
                .back()
                .map(|entry| entry.envelope.timestamp_ns),
            entries: state.entries.len(),
            retained_payload_bytes: state.retained_bytes,
        }
    }

    pub fn snapshot(&self, first: u64, last: u64) -> Vec<Arc<Envelope<T>>> {
        self.inner
            .state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|entry| (first..=last).contains(&entry.envelope.sequence))
            .map(|entry| Arc::clone(&entry.envelope))
            .collect()
    }

    /// Returns retained entries whose declared timestamps fall inside the
    /// inclusive range, preserving source sequence order.
    ///
    /// Phase 1 deliberately performs selection only. The policy for malformed
    /// or out-of-order timestamps remains a later experiment phase.
    pub fn snapshot_time_ns(&self, start_ns: u64, end_ns: u64) -> Vec<Arc<Envelope<T>>> {
        self.inner
            .state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|entry| (start_ns..=end_ns).contains(&entry.envelope.timestamp_ns))
            .map(|entry| Arc::clone(&entry.envelope))
            .collect()
    }

    pub fn subscribe(&self, start: CursorStart) -> BufferCursor<T> {
        let state = self.inner.state.lock().unwrap();
        let after_high_water = state
            .high_water
            .map_or(0, |sequence| sequence.saturating_add(1));
        let next_sequence = match start {
            CursorStart::Latest => after_high_water,
            CursorStart::Current => state
                .entries
                .back()
                .map_or(after_high_water, |entry| entry.envelope.sequence),
            CursorStart::FromSequence(sequence) => sequence,
        };
        BufferCursor {
            buffer: self.clone(),
            next_sequence,
        }
    }

    pub fn close(&self) {
        self.inner.state.lock().unwrap().closed = true;
        self.inner.changed.notify_all();
    }
}

fn validate_limits(limits: BufferLimits) -> Result<(), BufferError> {
    if limits.max_entries.is_none() && limits.max_bytes.is_none() {
        return Err(BufferError::MissingHardLimit);
    }
    if limits.max_entries == Some(0) {
        return Err(BufferError::ZeroEntryLimit);
    }
    if limits.max_bytes == Some(0) {
        return Err(BufferError::ZeroByteLimit);
    }
    Ok(())
}

fn evict_to_limits<T>(
    state: &mut BufferState<T>,
    limits: BufferLimits,
    time_policy: BufferTimePolicy,
) {
    loop {
        let exceeds_entries = limits
            .max_entries
            .is_some_and(|limit| state.entries.len() > limit);
        let exceeds_bytes = limits
            .max_bytes
            .is_some_and(|limit| state.retained_bytes > limit);
        let exceeds_duration = limits.target_duration.is_some_and(|target| {
            let Some(first) = state.entries.front() else {
                return false;
            };
            let Some(last) = state.entries.back() else {
                return false;
            };
            let retained_duration = match time_policy.duration_basis {
                DurationTimeBasis::SourceTimestamp => Duration::from_nanos(
                    last.envelope
                        .timestamp_ns
                        .saturating_sub(first.envelope.timestamp_ns),
                ),
                DurationTimeBasis::ArrivalTime => {
                    last.arrival.saturating_duration_since(first.arrival)
                }
            };
            retained_duration > target
        });

        if !(exceeds_entries || exceeds_bytes || exceeds_duration) {
            break;
        }
        if let Some(evicted) = state.entries.pop_front() {
            state.retained_bytes -= evicted.bytes;
        }
    }
}

pub struct BufferCursor<T> {
    buffer: Buffer<T>,
    next_sequence: u64,
}

impl<T> fmt::Debug for BufferCursor<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BufferCursor")
            .field("buffer", &self.buffer.name())
            .field("next_sequence", &self.next_sequence)
            .finish()
    }
}

impl<T> BufferCursor<T> {
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    pub fn next_timeout(&mut self, timeout: Duration) -> CursorRead<T> {
        let deadline = Instant::now() + timeout;
        let mut state = self.buffer.inner.state.lock().unwrap();

        loop {
            if let Some(first) = state.entries.front()
                && self.next_sequence < first.envelope.sequence
            {
                let gap = Gap {
                    requested_sequence: self.next_sequence,
                    available_from: first.envelope.sequence,
                };
                self.next_sequence = first.envelope.sequence;
                return CursorRead::Gap(gap);
            }

            if let Some(next) = state
                .entries
                .iter()
                .find(|entry| entry.envelope.sequence >= self.next_sequence)
            {
                if next.envelope.sequence > self.next_sequence {
                    let gap = Gap {
                        requested_sequence: self.next_sequence,
                        available_from: next.envelope.sequence,
                    };
                    self.next_sequence = next.envelope.sequence;
                    return CursorRead::Gap(gap);
                }

                let envelope = Arc::clone(&next.envelope);
                self.next_sequence = envelope.sequence.saturating_add(1);
                return CursorRead::Item(envelope);
            }

            if state.closed {
                return CursorRead::Closed;
            }

            let now = Instant::now();
            if now >= deadline {
                return CursorRead::Timeout;
            }
            let remaining = deadline.saturating_duration_since(now);
            let (new_state, wait) = self
                .buffer
                .inner
                .changed
                .wait_timeout(state, remaining)
                .unwrap();
            state = new_state;
            if wait.timed_out() {
                return CursorRead::Timeout;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BufferReaderStats {
    pub delivered: u64,
    pub gap_events: u64,
    pub gap_entries: u64,
    pub cancelled: bool,
    pub failed: bool,
}

/// One Component's bounded cursor over a Buffer. The reader leases only its
/// current shared envelope; it does not own a second history queue.
#[must_use = "dropping a BufferReader stops delivery to the Component"]
pub struct BufferReader<T> {
    cancelled: Arc<AtomicBool>,
    delivered: Arc<AtomicU64>,
    gap_events: Arc<AtomicU64>,
    gap_entries: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
    _payload: std::marker::PhantomData<fn(T)>,
}

impl<T: Send + Sync + 'static> BufferReader<T> {
    pub fn start(buffer: &Buffer<T>, start: CursorStart, input: &InputPort<T>) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let delivered = Arc::new(AtomicU64::new(0));
        let gap_events = Arc::new(AtomicU64::new(0));
        let gap_entries = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let mut cursor = buffer.subscribe(start);
        let input = input.clone();
        let worker_cancelled = Arc::clone(&cancelled);
        let worker_delivered = Arc::clone(&delivered);
        let worker_gap_events = Arc::clone(&gap_events);
        let worker_gap_entries = Arc::clone(&gap_entries);
        let worker_failed = Arc::clone(&failed);
        let worker = thread::Builder::new()
            .name("typed-dataflow-buffer-reader".into())
            .spawn(move || {
                while !worker_cancelled.load(Ordering::Acquire) {
                    match cursor.next_timeout(Duration::from_millis(2)) {
                        CursorRead::Item(envelope) => {
                            if input.accept(&envelope).is_err() {
                                worker_failed.store(true, Ordering::Release);
                                return;
                            }
                            worker_delivered.fetch_add(1, Ordering::Relaxed);
                        }
                        CursorRead::Gap(gap) => {
                            worker_gap_events.fetch_add(1, Ordering::Relaxed);
                            worker_gap_entries.fetch_add(
                                gap.available_from.saturating_sub(gap.requested_sequence),
                                Ordering::Relaxed,
                            );
                        }
                        CursorRead::Timeout => {}
                        CursorRead::Closed => return,
                    }
                }
            })
            .expect("failed to spawn Buffer reader");

        Self {
            cancelled,
            delivered,
            gap_events,
            gap_entries,
            failed,
            worker: Mutex::new(Some(worker)),
            _payload: std::marker::PhantomData,
        }
    }

    pub fn stats(&self) -> BufferReaderStats {
        BufferReaderStats {
            delivered: self.delivered.load(Ordering::Relaxed),
            gap_events: self.gap_events.load(Ordering::Relaxed),
            gap_entries: self.gap_entries.load(Ordering::Relaxed),
            cancelled: self.cancelled.load(Ordering::Acquire),
            failed: self.failed.load(Ordering::Acquire),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(worker) = self.worker.lock().unwrap().take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

impl<T> Drop for BufferReader<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(worker) = self.worker.get_mut().unwrap().take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

struct BufferEndpoint<T> {
    buffer: Buffer<T>,
    accepted: AtomicU64,
    overruns: AtomicU64,
    closed: AtomicBool,
    failure: Mutex<Option<ComponentError>>,
}

impl<T: Send + Sync + 'static> Endpoint<T> for BufferEndpoint<T> {
    fn kind(&self) -> EndpointKind {
        EndpointKind::Owning
    }

    fn deliver_owned(&self, envelope: Arc<Envelope<T>>) -> DeliveryStatus {
        if self.closed.load(Ordering::Acquire) {
            return DeliveryStatus::Disconnected;
        }
        match self.buffer.append_shared(envelope) {
            Ok(()) => {
                self.accepted.fetch_add(1, Ordering::Relaxed);
                DeliveryStatus::Accepted
            }
            Err(error) => {
                self.overruns.fetch_add(1, Ordering::Relaxed);
                self.closed.store(true, Ordering::Release);
                *self.failure.lock().unwrap() = Some(ComponentError::Reported(error.to_string()));
                DeliveryStatus::Failed
            }
        }
    }

    fn stats(&self) -> ConnectionStats {
        let accepted = self.accepted.load(Ordering::Relaxed);
        ConnectionStats {
            accepted,
            delivered: accepted,
            replaced: 0,
            overruns: self.overruns.load(Ordering::Relaxed),
            closed: self.closed.load(Ordering::Acquire),
            failed: self.failure.lock().unwrap().is_some(),
        }
    }

    fn failure(&self) -> Option<ComponentError> {
        self.failure.lock().unwrap().clone()
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

pub fn connect_buffer<T: Send + Sync + 'static>(
    from: &OutputPort<T>,
    buffer: &Buffer<T>,
) -> Connection<T> {
    from.attach(Arc::new(BufferEndpoint {
        buffer: buffer.clone(),
        accepted: AtomicU64::new(0),
        overruns: AtomicU64::new(0),
        closed: AtomicBool::new(false),
        failure: Mutex::new(None),
    }))
}
