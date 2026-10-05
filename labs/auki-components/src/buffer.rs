use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::ports::{
    ComponentError, Connection, ConnectionStats, DeliveryStatus, Endpoint, EndpointKind, Envelope,
    InputPort, OutputPort,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
    next_waiter: u64,
    waiters: BTreeMap<u64, Waker>,
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
                    next_waiter: 0,
                    waiters: BTreeMap::new(),
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
        let waiters = std::mem::take(&mut state.waiters);
        drop(state);
        drop(current_limits);
        self.inner.changed.notify_all();
        wake_waiters(waiters);
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
        let waiters = std::mem::take(&mut state.waiters);
        drop(state);
        drop(limits);
        self.inner.changed.notify_all();
        wake_waiters(waiters);
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
    /// Source-time ordering is governed by the Buffer's explicit timestamp
    /// policy; this query performs selection without rewriting timestamps.
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
        let mut state = self.inner.state.lock().unwrap();
        state.closed = true;
        let waiters = std::mem::take(&mut state.waiters);
        drop(state);
        self.inner.changed.notify_all();
        wake_waiters(waiters);
    }
}

fn wake_waiters(waiters: BTreeMap<u64, Waker>) {
    // Never invoke an executor while holding the Buffer's locks.
    for waker in waiters.into_values() {
        waker.wake();
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

    /// Await an observation, retention gap, or closure without a timer or thread.
    /// Cancelling the pending future unregisters its waker without advancing
    /// the cursor. Each reader still leases only the envelope it consumes.
    pub fn next_async(&mut self) -> BufferNext<'_, T> {
        BufferNext {
            cursor: self,
            waiter: None,
        }
    }

    pub fn next_timeout(&mut self, timeout: Duration) -> CursorRead<T> {
        let deadline = Instant::now() + timeout;
        let mut state = self.buffer.inner.state.lock().unwrap();

        loop {
            if let Some(ready) = cursor_ready(&state, &mut self.next_sequence) {
                return ready;
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

fn cursor_ready<T>(state: &BufferState<T>, sequence: &mut u64) -> Option<CursorRead<T>> {
    if let Some(next) = state
        .entries
        .iter()
        .find(|entry| entry.envelope.sequence >= *sequence)
    {
        if next.envelope.sequence > *sequence {
            let gap = Gap {
                requested_sequence: *sequence,
                available_from: next.envelope.sequence,
            };
            *sequence = next.envelope.sequence;
            return Some(CursorRead::Gap(gap));
        }
        let envelope = Arc::clone(&next.envelope);
        *sequence = envelope.sequence.saturating_add(1);
        return Some(CursorRead::Item(envelope));
    }
    state.closed.then_some(CursorRead::Closed)
}

/// Cancellation-safe asynchronous Buffer read. No retained history is copied.
pub struct BufferNext<'a, T> {
    cursor: &'a mut BufferCursor<T>,
    waiter: Option<u64>,
}

impl<T> Future for BufferNext<'_, T> {
    type Output = CursorRead<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut state = this.cursor.buffer.inner.state.lock().unwrap();
        if let Some(ready) = cursor_ready(&state, &mut this.cursor.next_sequence) {
            if let Some(id) = this.waiter.take() {
                state.waiters.remove(&id);
            }
            return Poll::Ready(ready);
        }
        let id = *this.waiter.get_or_insert_with(|| {
            let id = state.next_waiter;
            state.next_waiter = state
                .next_waiter
                .checked_add(1)
                .expect("Buffer waiter IDs exhausted");
            id
        });
        state.waiters.insert(id, cx.waker().clone());
        Poll::Pending
    }
}

impl<T> Drop for BufferNext<'_, T> {
    fn drop(&mut self) {
        if let Some(id) = self.waiter {
            self.cursor
                .buffer
                .inner
                .state
                .lock()
                .unwrap()
                .waiters
                .remove(&id);
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
            .name("auki-components-buffer-reader".into())
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

#[cfg(test)]
mod async_tests {
    use super::*;
    use std::task::Wake;

    #[derive(Default)]
    struct WakeCount(AtomicU64);
    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn async_readers_wake_and_share_the_same_envelope() {
        let buffer = Buffer::new("shared", 2).unwrap();
        let mut a = buffer.subscribe(CursorStart::Latest);
        let mut b = buffer.subscribe(CursorStart::Latest);
        let wakes = Arc::new(WakeCount::default());
        let waker = Waker::from(wakes.clone());
        let mut cx = Context::from_waker(&waker);
        let mut first = Box::pin(a.next_async());
        let mut second = Box::pin(b.next_async());
        assert!(first.as_mut().poll(&mut cx).is_pending());
        assert!(second.as_mut().poll(&mut cx).is_pending());
        assert_eq!(buffer.inner.state.lock().unwrap().waiters.len(), 2);
        let envelope = Arc::new(Envelope::new(0, 10, 42));
        buffer.append_shared(envelope.clone()).unwrap();
        assert_eq!(wakes.0.load(Ordering::Relaxed), 2);
        for read in [&mut first, &mut second] {
            let Poll::Ready(CursorRead::Item(item)) = read.as_mut().poll(&mut cx) else {
                panic!("not woken");
            };
            assert!(Arc::ptr_eq(&item, &envelope));
        }
        assert!(buffer.inner.state.lock().unwrap().waiters.is_empty());
    }

    #[test]
    fn cancelling_a_pending_async_read_removes_its_waker_without_advancing() {
        let buffer = Buffer::<u64>::new("cancel", 2).unwrap();
        let mut cursor = buffer.subscribe(CursorStart::Latest);
        let wakes = Arc::new(WakeCount::default());
        let waker = Waker::from(wakes.clone());
        let mut cx = Context::from_waker(&waker);
        for _ in 0..100 {
            let mut pending = Box::pin(cursor.next_async());
            assert!(pending.as_mut().poll(&mut cx).is_pending());
            assert!(pending.as_mut().poll(&mut cx).is_pending());
            assert_eq!(buffer.inner.state.lock().unwrap().waiters.len(), 1);
        }
        assert!(buffer.inner.state.lock().unwrap().waiters.is_empty());
        assert_eq!(cursor.next_sequence(), 0);
        buffer
            .append_shared(Arc::new(Envelope::new(0, 1, 10)))
            .unwrap();
        assert_eq!(wakes.0.load(Ordering::Relaxed), 0);
        assert!(matches!(
            Pin::new(&mut cursor.next_async()).poll(&mut cx),
            Poll::Ready(CursorRead::Item(_))
        ));
    }

    #[test]
    fn async_cursor_reports_eviction_and_internal_gaps_then_drains_before_closing() {
        let buffer = Buffer::new("gaps", 2).unwrap();
        let mut cursor = buffer.subscribe(CursorStart::FromSequence(0));
        let wakes = Arc::new(WakeCount::default());
        let waker = Waker::from(wakes.clone());
        let mut cx = Context::from_waker(&waker);
        let mut read = Box::pin(cursor.next_async());
        assert!(read.as_mut().poll(&mut cx).is_pending());
        for sequence in [0, 2, 4] {
            buffer
                .append_shared(Arc::new(Envelope::new(sequence, sequence, 1)))
                .unwrap();
        }
        buffer.close();
        assert!(matches!(
            read.as_mut().poll(&mut cx),
            Poll::Ready(CursorRead::Gap(Gap {
                requested_sequence: 0,
                available_from: 2
            }))
        ));
        drop(read);
        assert!(matches!(
            Pin::new(&mut cursor.next_async()).poll(&mut cx),
            Poll::Ready(CursorRead::Item(_))
        ));
        assert!(matches!(
            Pin::new(&mut cursor.next_async()).poll(&mut cx),
            Poll::Ready(CursorRead::Gap(Gap {
                requested_sequence: 3,
                available_from: 4
            }))
        ));
        assert!(matches!(
            Pin::new(&mut cursor.next_async()).poll(&mut cx),
            Poll::Ready(CursorRead::Item(_))
        ));
        assert!(matches!(
            Pin::new(&mut cursor.next_async()).poll(&mut cx),
            Poll::Ready(CursorRead::Closed)
        ));
        let empty = Buffer::<u64>::new("empty", 1).unwrap();
        let mut empty_cursor = empty.subscribe(CursorStart::Latest);
        let mut pending = Box::pin(empty_cursor.next_async());
        assert!(pending.as_mut().poll(&mut cx).is_pending());
        let before = wakes.0.load(Ordering::Relaxed);
        empty.close();
        assert_eq!(wakes.0.load(Ordering::Relaxed), before + 1);
        assert!(matches!(
            pending.as_mut().poll(&mut cx),
            Poll::Ready(CursorRead::Closed)
        ));
    }
}
