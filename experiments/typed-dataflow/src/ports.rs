use std::collections::VecDeque;
use std::fmt;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, unbounded};

/// One typed value moving through the graph.
#[derive(Debug)]
pub struct Envelope<T> {
    /// Sequence assigned by the producing output port.
    pub sequence: u64,
    /// Timestamp in the producer's declared clock, in nanoseconds.
    pub timestamp_ns: u64,
    /// The typed value. Large values may themselves be external-storage handles.
    pub payload: T,
}

impl<T> Envelope<T> {
    pub fn new(sequence: u64, timestamp_ns: u64, payload: T) -> Self {
        Self {
            sequence,
            timestamp_ns,
            payload,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComponentError {
    Reported(String),
    Panicked(String),
}

impl fmt::Display for ComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reported(reason) => write!(formatter, "Component reported an error: {reason}"),
            Self::Panicked(reason) => write!(formatter, "Component panicked: {reason}"),
        }
    }
}

impl std::error::Error for ComponentError {}

fn panic_reason(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(reason) = payload.downcast_ref::<&str>() {
        (*reason).to_owned()
    } else if let Some(reason) = payload.downcast_ref::<String>() {
        reason.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

/// A named, typed Component input.
type InputHandler<T> = dyn Fn(&Envelope<T>) -> Result<(), ComponentError> + Send + Sync;

pub struct InputPort<T> {
    name: Arc<str>,
    handler: Arc<InputHandler<T>>,
}

impl<T> Clone for InputPort<T> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            handler: Arc::clone(&self.handler),
        }
    }
}

impl<T> fmt::Debug for InputPort<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InputPort")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<T> InputPort<T> {
    pub fn new(
        name: impl Into<Arc<str>>,
        handler: impl Fn(&Envelope<T>) + Send + Sync + 'static,
    ) -> Self {
        Self::try_new(name, move |envelope| {
            handler(envelope);
            Ok(())
        })
    }

    pub fn try_new(
        name: impl Into<Arc<str>>,
        handler: impl Fn(&Envelope<T>) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        Self::with_component_errors(name, move |envelope| {
            handler(envelope).map_err(ComponentError::Reported)
        })
    }

    pub(crate) fn with_component_errors(
        name: impl Into<Arc<str>>,
        handler: impl Fn(&Envelope<T>) -> Result<(), ComponentError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            handler: Arc::new(handler),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn accept(&self, envelope: &Envelope<T>) -> Result<(), ComponentError> {
        match catch_unwind(AssertUnwindSafe(|| (self.handler)(envelope))) {
            Ok(result) => result,
            Err(payload) => Err(ComponentError::Panicked(panic_reason(payload))),
        }
    }
}

/// A named, typed Component output that can be connected at runtime.
pub struct OutputPort<T> {
    inner: Arc<OutputInner<T>>,
}

struct OutputInner<T> {
    name: Arc<str>,
    next_sequence: AtomicU64,
    subscribers: Mutex<Vec<Arc<dyn Endpoint<T>>>>,
}

impl<T> Clone for OutputPort<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> fmt::Debug for OutputPort<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputPort")
            .field("name", &self.inner.name)
            .finish_non_exhaustive()
    }
}

impl<T: Send + Sync + 'static> OutputPort<T> {
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            inner: Arc::new(OutputInner {
                name: name.into(),
                next_sequence: AtomicU64::new(0),
                subscribers: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Publishes one payload. Inline consumers borrow the stack envelope; if
    /// any owning consumer exists, all owning consumers share one `Arc`.
    pub fn publish(&self, timestamp_ns: u64, payload: T) -> PublishReport {
        let sequence = self.inner.next_sequence.fetch_add(1, Ordering::Relaxed);
        let envelope = Envelope::new(sequence, timestamp_ns, payload);

        let subscribers = {
            let mut subscribers = self.inner.subscribers.lock().unwrap();
            subscribers.retain(|subscriber| !subscriber.is_closed());
            subscribers.clone()
        };

        let mut report = PublishReport {
            sequence,
            ..PublishReport::default()
        };

        for subscriber in subscribers
            .iter()
            .filter(|subscriber| subscriber.kind() == EndpointKind::Borrowed)
        {
            match subscriber.deliver_borrowed(&envelope) {
                DeliveryStatus::Accepted => report.accepted += 1,
                DeliveryStatus::Disconnected => report.disconnected += 1,
                DeliveryStatus::Failed => report.failed += 1,
            }
        }

        let owning_count = subscribers
            .iter()
            .filter(|subscriber| subscriber.kind() == EndpointKind::Owning)
            .count();

        if owning_count > 0 {
            let shared = Arc::new(envelope);
            for subscriber in subscribers
                .iter()
                .filter(|subscriber| subscriber.kind() == EndpointKind::Owning)
            {
                match subscriber.deliver_owned(Arc::clone(&shared)) {
                    DeliveryStatus::Accepted => report.accepted += 1,
                    DeliveryStatus::Disconnected => report.disconnected += 1,
                    DeliveryStatus::Failed => report.failed += 1,
                }
            }
        }

        report
    }

    pub(crate) fn attach(&self, endpoint: Arc<dyn Endpoint<T>>) -> Connection<T> {
        self.inner
            .subscribers
            .lock()
            .unwrap()
            .push(endpoint.clone());
        Connection {
            endpoint,
            _payload: PhantomData,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PublishReport {
    pub sequence: u64,
    pub accepted: usize,
    pub disconnected: usize,
    pub failed: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionOptions {
    InlineEvery,
    QueuedEvery {
        capacity: usize,
        when_full: EveryFullPolicy,
    },
    Latest,
}

impl ConnectionOptions {
    pub const fn inline_every() -> Self {
        Self::InlineEvery
    }

    pub const fn queued_every(capacity: usize, when_full: EveryFullPolicy) -> Self {
        Self::QueuedEvery {
            capacity,
            when_full,
        }
    }

    pub const fn latest() -> Self {
        Self::Latest
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EveryFullPolicy {
    Backpressure,
    Disconnect,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConnectionStats {
    pub accepted: u64,
    pub delivered: u64,
    pub replaced: u64,
    pub overruns: u64,
    pub closed: bool,
    pub failed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionError {
    ZeroCapacity,
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => formatter.write_str("QueuedEvery capacity must be positive"),
        }
    }
}

impl std::error::Error for ConnectionError {}

/// Owns the lifetime of exactly one port connection.
#[must_use = "dropping a Connection disconnects its ports"]
pub struct Connection<T: Send + Sync + 'static> {
    endpoint: Arc<dyn Endpoint<T>>,
    _payload: PhantomData<fn(T)>,
}

/// A clonable cancellation capability for a [`Connection`] that does not own
/// the connection's lifetime.
///
/// This lets a relationship mark itself terminal from inside its delivery
/// callback without making `Connection` itself clonable. Dropping the control
/// handle has no effect; dropping the owning `Connection` still disconnects.
pub struct ConnectionControl<T: Send + Sync + 'static> {
    endpoint: Arc<dyn Endpoint<T>>,
    _payload: PhantomData<fn(T)>,
}

impl<T: Send + Sync + 'static> Clone for ConnectionControl<T> {
    fn clone(&self) -> Self {
        Self {
            endpoint: Arc::clone(&self.endpoint),
            _payload: PhantomData,
        }
    }
}

impl<T: Send + Sync + 'static> fmt::Debug for ConnectionControl<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionControl")
            .field("stats", &self.stats())
            .finish()
    }
}

impl<T: Send + Sync + 'static> ConnectionControl<T> {
    pub fn stats(&self) -> ConnectionStats {
        self.endpoint.stats()
    }

    pub fn disconnect(&self) {
        self.endpoint.close();
    }

    pub fn failure(&self) -> Option<ComponentError> {
        self.endpoint.failure()
    }
}

impl<T: Send + Sync + 'static> fmt::Debug for Connection<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Connection")
            .field("stats", &self.stats())
            .finish()
    }
}

impl<T: Send + Sync + 'static> Connection<T> {
    pub fn stats(&self) -> ConnectionStats {
        self.endpoint.stats()
    }

    pub fn control(&self) -> ConnectionControl<T> {
        ConnectionControl {
            endpoint: Arc::clone(&self.endpoint),
            _payload: PhantomData,
        }
    }

    pub fn disconnect(&self) {
        self.endpoint.close();
    }

    pub fn failure(&self) -> Option<ComponentError> {
        self.endpoint.failure()
    }
}

impl<T: Send + Sync + 'static> Drop for Connection<T> {
    fn drop(&mut self) {
        self.endpoint.close();
    }
}

pub fn connect<T: Send + Sync + 'static>(
    from: &OutputPort<T>,
    to: &InputPort<T>,
    options: ConnectionOptions,
) -> Result<Connection<T>, ConnectionError> {
    let endpoint: Arc<dyn Endpoint<T>> = match options {
        ConnectionOptions::InlineEvery => Arc::new(InlineEndpoint::new(to.clone())),
        ConnectionOptions::QueuedEvery {
            capacity,
            when_full,
        } => {
            if capacity == 0 {
                return Err(ConnectionError::ZeroCapacity);
            }
            QueueEndpoint::spawn(to.clone(), capacity, when_full)
        }
        ConnectionOptions::Latest => LatestEndpoint::spawn(to.clone()),
    };

    Ok(from.attach(endpoint))
}

/// A concrete, monomorphizable inline path with no runtime port graph.
pub struct StaticConnection<T, F>
where
    F: FnMut(&Envelope<T>),
{
    next_sequence: u64,
    consumer: F,
    _payload: PhantomData<fn(T)>,
}

impl<T, F> StaticConnection<T, F>
where
    F: FnMut(&Envelope<T>),
{
    pub fn new(consumer: F) -> Self {
        Self {
            next_sequence: 0,
            consumer,
            _payload: PhantomData,
        }
    }

    #[inline]
    pub fn publish(&mut self, timestamp_ns: u64, payload: T) {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        (self.consumer)(&Envelope::new(sequence, timestamp_ns, payload));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EndpointKind {
    Borrowed,
    Owning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeliveryStatus {
    Accepted,
    Disconnected,
    Failed,
}

pub(crate) trait Endpoint<T>: Send + Sync {
    fn kind(&self) -> EndpointKind;
    fn deliver_borrowed(&self, _envelope: &Envelope<T>) -> DeliveryStatus {
        DeliveryStatus::Disconnected
    }
    fn deliver_owned(&self, _envelope: Arc<Envelope<T>>) -> DeliveryStatus {
        DeliveryStatus::Disconnected
    }
    fn stats(&self) -> ConnectionStats;
    fn failure(&self) -> Option<ComponentError> {
        None
    }
    fn is_closed(&self) -> bool;
    fn close(&self);
}

#[derive(Default)]
pub(crate) struct AtomicStats {
    accepted: AtomicU64,
    delivered: AtomicU64,
    replaced: AtomicU64,
    overruns: AtomicU64,
    closed: AtomicBool,
    failed: AtomicBool,
}

impl AtomicStats {
    fn snapshot(&self) -> ConnectionStats {
        ConnectionStats {
            accepted: self.accepted.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            replaced: self.replaced.load(Ordering::Relaxed),
            overruns: self.overruns.load(Ordering::Relaxed),
            closed: self.closed.load(Ordering::Acquire),
            failed: self.failed.load(Ordering::Acquire),
        }
    }
}

struct InlineEndpoint<T> {
    input: InputPort<T>,
    stats: AtomicStats,
    failure: Mutex<Option<ComponentError>>,
}

impl<T> InlineEndpoint<T> {
    fn new(input: InputPort<T>) -> Self {
        Self {
            input,
            stats: AtomicStats::default(),
            failure: Mutex::new(None),
        }
    }
}

impl<T: Send + Sync + 'static> Endpoint<T> for InlineEndpoint<T> {
    fn kind(&self) -> EndpointKind {
        EndpointKind::Borrowed
    }

    fn deliver_borrowed(&self, envelope: &Envelope<T>) -> DeliveryStatus {
        if self.is_closed() {
            return DeliveryStatus::Disconnected;
        }
        self.stats.accepted.fetch_add(1, Ordering::Relaxed);
        match self.input.accept(envelope) {
            Ok(()) => {
                self.stats.delivered.fetch_add(1, Ordering::Relaxed);
                DeliveryStatus::Accepted
            }
            Err(error) => {
                *self.failure.lock().unwrap() = Some(error);
                self.stats.failed.store(true, Ordering::Release);
                self.stats.closed.store(true, Ordering::Release);
                DeliveryStatus::Failed
            }
        }
    }

    fn stats(&self) -> ConnectionStats {
        self.stats.snapshot()
    }

    fn failure(&self) -> Option<ComponentError> {
        self.failure.lock().unwrap().clone()
    }

    fn is_closed(&self) -> bool {
        self.stats.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        self.stats.closed.store(true, Ordering::Release);
    }
}

struct QueueState<T> {
    pending: VecDeque<Arc<Envelope<T>>>,
}

struct QueueEndpoint<T> {
    input: InputPort<T>,
    capacity: usize,
    when_full: EveryFullPolicy,
    state: Mutex<QueueState<T>>,
    not_empty: Condvar,
    not_full: Condvar,
    stats: AtomicStats,
    failure: Mutex<Option<ComponentError>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl<T: Send + Sync + 'static> QueueEndpoint<T> {
    fn spawn(input: InputPort<T>, capacity: usize, when_full: EveryFullPolicy) -> Arc<Self> {
        let endpoint = Arc::new(Self {
            input,
            capacity,
            when_full,
            state: Mutex::new(QueueState {
                pending: VecDeque::with_capacity(capacity),
            }),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
            stats: AtomicStats::default(),
            failure: Mutex::new(None),
            worker: Mutex::new(None),
        });

        let worker_endpoint = Arc::clone(&endpoint);
        let worker = thread::Builder::new()
            .name("typed-dataflow-every".into())
            .spawn(move || worker_endpoint.run())
            .expect("failed to spawn QueuedEvery worker");
        *endpoint.worker.lock().unwrap() = Some(worker);
        endpoint
    }

    fn run(&self) {
        loop {
            let next = {
                let mut state = self.state.lock().unwrap();
                while state.pending.is_empty() && !self.is_closed() {
                    state = self.not_empty.wait(state).unwrap();
                }
                if self.is_closed() {
                    state.pending.clear();
                    self.not_full.notify_all();
                    return;
                }
                let next = state.pending.pop_front();
                self.not_full.notify_one();
                next
            };

            if let Some(envelope) = next {
                match self.input.accept(&envelope) {
                    Ok(()) => {
                        self.stats.delivered.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        *self.failure.lock().unwrap() = Some(error);
                        self.stats.failed.store(true, Ordering::Release);
                        self.stats.closed.store(true, Ordering::Release);
                        let mut state = self.state.lock().unwrap();
                        state.pending.clear();
                        self.not_full.notify_all();
                        return;
                    }
                }
            }
        }
    }
}

impl<T: Send + Sync + 'static> Endpoint<T> for QueueEndpoint<T> {
    fn kind(&self) -> EndpointKind {
        EndpointKind::Owning
    }

    fn deliver_owned(&self, envelope: Arc<Envelope<T>>) -> DeliveryStatus {
        let mut state = self.state.lock().unwrap();
        while state.pending.len() >= self.capacity && !self.is_closed() {
            match self.when_full {
                EveryFullPolicy::Backpressure => {
                    state = self.not_full.wait(state).unwrap();
                }
                EveryFullPolicy::Disconnect => {
                    self.stats.overruns.fetch_add(1, Ordering::Relaxed);
                    self.stats.failed.store(true, Ordering::Release);
                    *self.failure.lock().unwrap() = Some(ComponentError::Reported(
                        "EverySelected queue reached its hard capacity".to_owned(),
                    ));
                    self.stats.closed.store(true, Ordering::Release);
                    state.pending.clear();
                    self.not_empty.notify_all();
                    self.not_full.notify_all();
                    return DeliveryStatus::Failed;
                }
            }
        }

        if self.is_closed() {
            return DeliveryStatus::Disconnected;
        }

        state.pending.push_back(envelope);
        self.stats.accepted.fetch_add(1, Ordering::Relaxed);
        self.not_empty.notify_one();
        DeliveryStatus::Accepted
    }

    fn stats(&self) -> ConnectionStats {
        self.stats.snapshot()
    }

    fn failure(&self) -> Option<ComponentError> {
        self.failure.lock().unwrap().clone()
    }

    fn is_closed(&self) -> bool {
        self.stats.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        let was_closed = self.stats.closed.swap(true, Ordering::AcqRel);
        {
            let mut state = self.state.lock().unwrap();
            state.pending.clear();
        }
        self.not_empty.notify_all();
        self.not_full.notify_all();

        if !was_closed
            && let Some(worker) = self.worker.lock().unwrap().take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

impl<T> Drop for QueueEndpoint<T> {
    fn drop(&mut self) {
        self.stats.closed.store(true, Ordering::Release);
        self.not_empty.notify_all();
        self.not_full.notify_all();
    }
}

struct LatestState<T> {
    pending: Option<Arc<Envelope<T>>>,
}

struct LatestEndpoint<T> {
    input: InputPort<T>,
    state: Mutex<LatestState<T>>,
    available: Condvar,
    stats: AtomicStats,
    failure: Mutex<Option<ComponentError>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl<T: Send + Sync + 'static> LatestEndpoint<T> {
    fn spawn(input: InputPort<T>) -> Arc<Self> {
        let endpoint = Arc::new(Self {
            input,
            state: Mutex::new(LatestState { pending: None }),
            available: Condvar::new(),
            stats: AtomicStats::default(),
            failure: Mutex::new(None),
            worker: Mutex::new(None),
        });

        let worker_endpoint = Arc::clone(&endpoint);
        let worker = thread::Builder::new()
            .name("typed-dataflow-latest".into())
            .spawn(move || worker_endpoint.run())
            .expect("failed to spawn Latest worker");
        *endpoint.worker.lock().unwrap() = Some(worker);
        endpoint
    }

    fn run(&self) {
        loop {
            let next = {
                let mut state = self.state.lock().unwrap();
                while state.pending.is_none() && !self.is_closed() {
                    state = self.available.wait(state).unwrap();
                }
                if self.is_closed() {
                    state.pending.take();
                    return;
                }
                state.pending.take()
            };

            if let Some(envelope) = next {
                match self.input.accept(&envelope) {
                    Ok(()) => {
                        self.stats.delivered.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        *self.failure.lock().unwrap() = Some(error);
                        self.stats.failed.store(true, Ordering::Release);
                        self.stats.closed.store(true, Ordering::Release);
                        self.state.lock().unwrap().pending.take();
                        return;
                    }
                }
            }
        }
    }
}

impl<T: Send + Sync + 'static> Endpoint<T> for LatestEndpoint<T> {
    fn kind(&self) -> EndpointKind {
        EndpointKind::Owning
    }

    fn deliver_owned(&self, envelope: Arc<Envelope<T>>) -> DeliveryStatus {
        if self.is_closed() {
            return DeliveryStatus::Disconnected;
        }

        let mut state = self.state.lock().unwrap();
        if self.is_closed() {
            return DeliveryStatus::Disconnected;
        }
        if state.pending.replace(envelope).is_some() {
            self.stats.replaced.fetch_add(1, Ordering::Relaxed);
        }
        self.stats.accepted.fetch_add(1, Ordering::Relaxed);
        self.available.notify_one();
        DeliveryStatus::Accepted
    }

    fn stats(&self) -> ConnectionStats {
        self.stats.snapshot()
    }

    fn failure(&self) -> Option<ComponentError> {
        self.failure.lock().unwrap().clone()
    }

    fn is_closed(&self) -> bool {
        self.stats.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        let was_closed = self.stats.closed.swap(true, Ordering::AcqRel);
        self.state.lock().unwrap().pending.take();
        self.available.notify_all();

        if !was_closed
            && let Some(worker) = self.worker.lock().unwrap().take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

impl<T> Drop for LatestEndpoint<T> {
    fn drop(&mut self) {
        self.stats.closed.store(true, Ordering::Release);
        self.available.notify_all();
    }
}

type ScheduledJob = Box<dyn FnOnce() + Send + 'static>;

enum SchedulerMessage {
    Run(ScheduledJob),
    Shutdown,
}

#[derive(Default)]
struct SchedulerCounters {
    scheduled: AtomicU64,
    completed: AtomicU64,
    active: AtomicUsize,
    max_active: AtomicUsize,
    job_panics: AtomicU64,
    closed: AtomicBool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SharedSchedulerStats {
    pub worker_count: usize,
    pub scheduled: u64,
    pub completed: u64,
    pub active: usize,
    pub max_active: usize,
    pub job_panics: u64,
    pub closed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    ZeroWorkers,
    Unavailable,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWorkers => {
                formatter.write_str("SharedScheduler requires at least one worker")
            }
            Self::Unavailable => formatter.write_str("SharedScheduler is unavailable"),
        }
    }
}

impl std::error::Error for SchedulerError {}

/// A clonable submission capability for a fixed-size worker pool.
///
/// Connections retain this lightweight dispatcher, not ownership of an OS
/// thread. A connection can have at most one queued drain in addition to a
/// drain currently running.
#[derive(Clone)]
pub struct SharedDispatcher {
    sender: Sender<SchedulerMessage>,
    counters: Arc<SchedulerCounters>,
    worker_count: usize,
}

impl fmt::Debug for SharedDispatcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedDispatcher")
            .field("stats", &self.stats())
            .finish()
    }
}

impl SharedDispatcher {
    pub(crate) fn schedule(&self, job: ScheduledJob) -> Result<(), SchedulerError> {
        if self.counters.closed.load(Ordering::Acquire) {
            return Err(SchedulerError::Unavailable);
        }
        self.sender
            .send(SchedulerMessage::Run(job))
            .map_err(|_| SchedulerError::Unavailable)?;
        self.counters.scheduled.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub fn stats(&self) -> SharedSchedulerStats {
        SharedSchedulerStats {
            worker_count: self.worker_count,
            scheduled: self.counters.scheduled.load(Ordering::Relaxed),
            completed: self.counters.completed.load(Ordering::Relaxed),
            active: self.counters.active.load(Ordering::Relaxed),
            max_active: self.counters.max_active.load(Ordering::Relaxed),
            job_panics: self.counters.job_panics.load(Ordering::Relaxed),
            closed: self.counters.closed.load(Ordering::Acquire),
        }
    }
}

/// Experimental bounded-concurrency scheduler shared by many connections.
///
/// The task channel is unbounded, but outstanding task creation is bounded by
/// the number of live connections plus active workers. Observation queues
/// themselves remain explicitly bounded.
pub struct SharedScheduler {
    dispatcher: SharedDispatcher,
    workers: Vec<JoinHandle<()>>,
}

impl fmt::Debug for SharedScheduler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedScheduler")
            .field("stats", &self.stats())
            .finish()
    }
}

impl SharedScheduler {
    pub fn new(worker_count: usize) -> Result<Self, SchedulerError> {
        if worker_count == 0 {
            return Err(SchedulerError::ZeroWorkers);
        }

        let (sender, receiver) = unbounded();
        let counters = Arc::new(SchedulerCounters::default());
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let receiver = receiver.clone();
            let counters = Arc::clone(&counters);
            workers.push(
                thread::Builder::new()
                    .name(format!("typed-dataflow-shared-{index}"))
                    .spawn(move || run_scheduler(receiver, counters))
                    .expect("failed to spawn SharedScheduler worker"),
            );
        }

        Ok(Self {
            dispatcher: SharedDispatcher {
                sender,
                counters,
                worker_count,
            },
            workers,
        })
    }

    pub fn dispatcher(&self) -> SharedDispatcher {
        self.dispatcher.clone()
    }

    pub fn stats(&self) -> SharedSchedulerStats {
        self.dispatcher.stats()
    }
}

impl Drop for SharedScheduler {
    fn drop(&mut self) {
        self.dispatcher
            .counters
            .closed
            .store(true, Ordering::Release);
        for _ in 0..self.workers.len() {
            let _ = self.dispatcher.sender.send(SchedulerMessage::Shutdown);
        }
        for worker in self.workers.drain(..) {
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

fn run_scheduler(receiver: Receiver<SchedulerMessage>, counters: Arc<SchedulerCounters>) {
    while let Ok(message) = receiver.recv() {
        let SchedulerMessage::Run(job) = message else {
            return;
        };
        let active = counters.active.fetch_add(1, Ordering::AcqRel) + 1;
        counters.max_active.fetch_max(active, Ordering::Relaxed);
        if catch_unwind(AssertUnwindSafe(job)).is_err() {
            counters.job_panics.fetch_add(1, Ordering::Relaxed);
        }
        counters.active.fetch_sub(1, Ordering::AcqRel);
        counters.completed.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedDelivery {
    EverySelected {
        capacity: usize,
        when_full: EveryFullPolicy,
    },
    CoalesceLatest,
}

impl SharedDelivery {
    pub const fn every_selected(capacity: usize, when_full: EveryFullPolicy) -> Self {
        Self::EverySelected {
            capacity,
            when_full,
        }
    }

    pub const fn coalesce_latest() -> Self {
        Self::CoalesceLatest
    }
}

struct ScheduledState<T> {
    pending: VecDeque<Arc<Envelope<T>>>,
    scheduled: bool,
}

struct ScheduledEndpoint<T> {
    input: InputPort<T>,
    dispatcher: SharedDispatcher,
    delivery: SharedDelivery,
    state: Mutex<ScheduledState<T>>,
    not_full: Condvar,
    stats: AtomicStats,
    failure: Mutex<Option<ComponentError>>,
    self_weak: Weak<ScheduledEndpoint<T>>,
}

impl<T: Send + Sync + 'static> ScheduledEndpoint<T> {
    fn new(
        input: InputPort<T>,
        dispatcher: SharedDispatcher,
        delivery: SharedDelivery,
    ) -> Result<Arc<Self>, ConnectionError> {
        if matches!(delivery, SharedDelivery::EverySelected { capacity: 0, .. }) {
            return Err(ConnectionError::ZeroCapacity);
        }
        Ok(Arc::new_cyclic(|self_weak| Self {
            input,
            dispatcher,
            delivery,
            state: Mutex::new(ScheduledState {
                pending: VecDeque::new(),
                scheduled: false,
            }),
            not_full: Condvar::new(),
            stats: AtomicStats::default(),
            failure: Mutex::new(None),
            self_weak: self_weak.clone(),
        }))
    }

    fn fail(&self, error: ComponentError) {
        *self.failure.lock().unwrap() = Some(error);
        self.stats.failed.store(true, Ordering::Release);
        self.stats.closed.store(true, Ordering::Release);
        let mut state = self.state.lock().unwrap();
        state.pending.clear();
        state.scheduled = false;
        self.not_full.notify_all();
    }

    fn schedule_drain(&self) -> Result<(), ComponentError> {
        let weak = self.self_weak.clone();
        self.dispatcher
            .schedule(Box::new(move || {
                if let Some(endpoint) = weak.upgrade() {
                    endpoint.run_one();
                }
            }))
            .map_err(|error| ComponentError::Reported(error.to_string()))
    }

    fn run_one(&self) {
        let next = {
            let mut state = self.state.lock().unwrap();
            if self.is_closed() {
                state.pending.clear();
                state.scheduled = false;
                self.not_full.notify_all();
                return;
            }
            let next = state.pending.pop_front();
            self.not_full.notify_one();
            next
        };

        if let Some(envelope) = next {
            match self.input.accept(&envelope) {
                Ok(()) => {
                    self.stats.delivered.fetch_add(1, Ordering::Relaxed);
                }
                Err(error) => {
                    self.fail(error);
                    return;
                }
            }
        }

        let schedule_again = {
            let mut state = self.state.lock().unwrap();
            if self.is_closed() || state.pending.is_empty() {
                state.scheduled = false;
                false
            } else {
                true
            }
        };
        if schedule_again && let Err(error) = self.schedule_drain() {
            self.fail(error);
        }
    }
}

impl<T: Send + Sync + 'static> Endpoint<T> for ScheduledEndpoint<T> {
    fn kind(&self) -> EndpointKind {
        EndpointKind::Owning
    }

    fn deliver_owned(&self, envelope: Arc<Envelope<T>>) -> DeliveryStatus {
        if self.is_closed() {
            return DeliveryStatus::Disconnected;
        }

        let should_schedule = {
            let mut state = self.state.lock().unwrap();
            if self.is_closed() {
                return DeliveryStatus::Disconnected;
            }
            match self.delivery {
                SharedDelivery::EverySelected {
                    capacity,
                    when_full,
                } => {
                    while state.pending.len() >= capacity && !self.is_closed() {
                        match when_full {
                            EveryFullPolicy::Backpressure => {
                                state = self.not_full.wait(state).unwrap();
                            }
                            EveryFullPolicy::Disconnect => {
                                drop(state);
                                self.stats.overruns.fetch_add(1, Ordering::Relaxed);
                                self.fail(ComponentError::Reported(
                                    "EverySelected queue reached its hard capacity".to_owned(),
                                ));
                                return DeliveryStatus::Failed;
                            }
                        }
                    }
                    if self.is_closed() {
                        return DeliveryStatus::Disconnected;
                    }
                    state.pending.push_back(envelope);
                }
                SharedDelivery::CoalesceLatest => {
                    if state.pending.pop_back().is_some() {
                        self.stats.replaced.fetch_add(1, Ordering::Relaxed);
                    }
                    state.pending.push_back(envelope);
                }
            }
            self.stats.accepted.fetch_add(1, Ordering::Relaxed);
            if state.scheduled {
                false
            } else {
                state.scheduled = true;
                true
            }
        };

        if should_schedule && let Err(error) = self.schedule_drain() {
            self.fail(error);
            return DeliveryStatus::Failed;
        }
        DeliveryStatus::Accepted
    }

    fn stats(&self) -> ConnectionStats {
        self.stats.snapshot()
    }

    fn failure(&self) -> Option<ComponentError> {
        self.failure.lock().unwrap().clone()
    }

    fn is_closed(&self) -> bool {
        self.stats.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        self.stats.closed.store(true, Ordering::Release);
        let mut state = self.state.lock().unwrap();
        state.pending.clear();
        state.scheduled = false;
        self.not_full.notify_all();
    }
}

pub fn connect_shared<T: Send + Sync + 'static>(
    from: &OutputPort<T>,
    to: &InputPort<T>,
    dispatcher: &SharedDispatcher,
    delivery: SharedDelivery,
) -> Result<Connection<T>, ConnectionError> {
    let endpoint: Arc<dyn Endpoint<T>> =
        ScheduledEndpoint::new(to.clone(), dispatcher.clone(), delivery)?;
    Ok(from.attach(endpoint))
}
