use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::buffer::{Buffer, CursorRead, CursorStart};
use crate::ports::{
    Connection, ConnectionStats, DeliveryStatus, Endpoint, EndpointKind, Envelope, OutputPort,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SinkFullPolicy {
    Backpressure,
    DropNewest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PumpOptions {
    pub sink_capacity: usize,
    pub when_full: SinkFullPolicy,
    pub receiver_delay: Duration,
    pub cursor_poll_interval: Duration,
}

impl Default for PumpOptions {
    fn default() -> Self {
        Self {
            sink_capacity: 8,
            when_full: SinkFullPolicy::Backpressure,
            receiver_delay: Duration::ZERO,
            cursor_poll_interval: Duration::from_millis(2),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PumpStats {
    pub source_items: u64,
    pub source_gap_events: u64,
    pub source_gap_entries: u64,
    pub sink_drops: u64,
    pub delivered: u64,
    pub delivered_sequence: Option<u64>,
    pub recipient_failures: u64,
    pub cancelled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PumpError {
    ZeroCapacity,
}

impl fmt::Display for PumpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => formatter.write_str("StreamPump sink capacity must be positive"),
        }
    }
}

impl std::error::Error for PumpError {}

#[derive(Default)]
struct AtomicPumpStats {
    source_items: AtomicU64,
    source_gap_events: AtomicU64,
    source_gap_entries: AtomicU64,
    sink_drops: AtomicU64,
    delivered: AtomicU64,
    delivered_sequence_plus_one: AtomicU64,
    recipient_failures: AtomicU64,
}

impl AtomicPumpStats {
    fn snapshot(&self, cancelled: bool) -> PumpStats {
        let delivered_sequence_plus_one = self.delivered_sequence_plus_one.load(Ordering::Relaxed);
        PumpStats {
            source_items: self.source_items.load(Ordering::Relaxed),
            source_gap_events: self.source_gap_events.load(Ordering::Relaxed),
            source_gap_entries: self.source_gap_entries.load(Ordering::Relaxed),
            sink_drops: self.sink_drops.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            delivered_sequence: delivered_sequence_plus_one.checked_sub(1),
            recipient_failures: self.recipient_failures.load(Ordering::Relaxed),
            cancelled,
        }
    }
}

/// One Buffer-following delivery loop for one recipient.
#[must_use = "dropping a StreamPump stops delivery to its recipient"]
pub struct StreamPump<T> {
    cancelled: Arc<AtomicBool>,
    stats: Arc<AtomicPumpStats>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    _payload: std::marker::PhantomData<fn(T)>,
}

impl<T: Send + Sync + 'static> StreamPump<T> {
    pub fn start(
        source: &Buffer<T>,
        start: CursorStart,
        recipient: &Buffer<T>,
        options: PumpOptions,
    ) -> Result<Self, PumpError> {
        if options.sink_capacity == 0 {
            return Err(PumpError::ZeroCapacity);
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(AtomicPumpStats::default());
        let (sender, receiver) = mpsc::sync_channel(options.sink_capacity);

        let mut cursor = source.subscribe(start);
        let pump_cancelled = Arc::clone(&cancelled);
        let pump_stats = Arc::clone(&stats);
        let pump_thread = thread::Builder::new()
            .name("typed-dataflow-pump".into())
            .spawn(move || {
                while !pump_cancelled.load(Ordering::Acquire) {
                    match cursor.next_timeout(options.cursor_poll_interval) {
                        CursorRead::Item(envelope) => {
                            pump_stats.source_items.fetch_add(1, Ordering::Relaxed);
                            if !send_to_sink(&sender, envelope, options.when_full, &pump_stats) {
                                return;
                            }
                        }
                        CursorRead::Gap(gap) => {
                            pump_stats.source_gap_events.fetch_add(1, Ordering::Relaxed);
                            pump_stats.source_gap_entries.fetch_add(
                                gap.available_from.saturating_sub(gap.requested_sequence),
                                Ordering::Relaxed,
                            );
                        }
                        CursorRead::Timeout => {}
                        CursorRead::Closed => return,
                    }
                }
            })
            .expect("failed to spawn StreamPump source worker");

        let recipient = recipient.clone();
        let receiver_cancelled = Arc::clone(&cancelled);
        let receiver_stats = Arc::clone(&stats);
        let receiver_thread = thread::Builder::new()
            .name("typed-dataflow-sink".into())
            .spawn(move || {
                while !receiver_cancelled.load(Ordering::Acquire) {
                    match receiver.recv_timeout(options.cursor_poll_interval) {
                        Ok(envelope) => {
                            if !options.receiver_delay.is_zero() {
                                thread::sleep(options.receiver_delay);
                            }
                            let sequence = envelope.sequence;
                            if recipient.append_shared(envelope).is_err() {
                                receiver_stats
                                    .recipient_failures
                                    .fetch_add(1, Ordering::Relaxed);
                                return;
                            }
                            receiver_stats.delivered.fetch_add(1, Ordering::Relaxed);
                            receiver_stats
                                .delivered_sequence_plus_one
                                .store(sequence.saturating_add(1), Ordering::Relaxed);
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("failed to spawn StreamPump receiver worker");

        Ok(Self {
            cancelled,
            stats,
            workers: Mutex::new(vec![pump_thread, receiver_thread]),
            _payload: std::marker::PhantomData,
        })
    }

    pub fn stats(&self) -> PumpStats {
        self.stats.snapshot(self.cancelled.load(Ordering::Acquire))
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let workers = std::mem::take(&mut *self.workers.lock().unwrap());
        for worker in workers {
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

impl<T> Drop for StreamPump<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let workers = std::mem::take(self.workers.get_mut().unwrap());
        for worker in workers {
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

fn send_to_sink<T>(
    sender: &SyncSender<Arc<Envelope<T>>>,
    envelope: Arc<Envelope<T>>,
    policy: SinkFullPolicy,
    stats: &AtomicPumpStats,
) -> bool {
    match policy {
        SinkFullPolicy::Backpressure => sender.send(envelope).is_ok(),
        SinkFullPolicy::DropNewest => match sender.try_send(envelope) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                stats.sink_drops.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        },
    }
}

/// Control path: a latest-only pump directly from an OutputPort to a recipient
/// Buffer. This exists to measure whether the source Buffer is worth requiring.
pub fn connect_direct_latest_pump<T: Send + Sync + 'static>(
    from: &OutputPort<T>,
    recipient: &Buffer<T>,
    receiver_delay: Duration,
) -> Connection<T> {
    from.attach(DirectPumpEndpoint::spawn(recipient.clone(), receiver_delay))
}

struct DirectState<T> {
    pending: Option<Arc<Envelope<T>>>,
}

struct DirectPumpEndpoint<T> {
    recipient: Buffer<T>,
    receiver_delay: Duration,
    state: Mutex<DirectState<T>>,
    available: Condvar,
    accepted: AtomicU64,
    delivered: AtomicU64,
    replaced: AtomicU64,
    closed: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl<T: Send + Sync + 'static> DirectPumpEndpoint<T> {
    fn spawn(recipient: Buffer<T>, receiver_delay: Duration) -> Arc<Self> {
        let endpoint = Arc::new(Self {
            recipient,
            receiver_delay,
            state: Mutex::new(DirectState { pending: None }),
            available: Condvar::new(),
            accepted: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            replaced: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            worker: Mutex::new(None),
        });
        let worker_endpoint = Arc::clone(&endpoint);
        let worker = thread::Builder::new()
            .name("typed-dataflow-direct-pump".into())
            .spawn(move || worker_endpoint.run())
            .expect("failed to spawn direct pump worker");
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
                if !self.receiver_delay.is_zero() {
                    thread::sleep(self.receiver_delay);
                }
                if self.recipient.append_shared(envelope).is_err() {
                    self.closed.store(true, Ordering::Release);
                    return;
                }
                self.delivered.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl<T: Send + Sync + 'static> Endpoint<T> for DirectPumpEndpoint<T> {
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
            self.replaced.fetch_add(1, Ordering::Relaxed);
        }
        self.accepted.fetch_add(1, Ordering::Relaxed);
        self.available.notify_one();
        DeliveryStatus::Accepted
    }

    fn stats(&self) -> ConnectionStats {
        ConnectionStats {
            accepted: self.accepted.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            replaced: self.replaced.load(Ordering::Relaxed),
            overruns: 0,
            closed: self.closed.load(Ordering::Acquire),
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        let was_closed = self.closed.swap(true, Ordering::AcqRel);
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
