use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::buffer::{Buffer, CursorRead, CursorStart};
use crate::ports::Envelope;

/// Experimental physical retention unit. It is not a catalogued data product.
pub struct Chunk<T> {
    entries: Arc<[Arc<Envelope<T>>]>,
    accounted_payload_bytes: usize,
}

impl<T> fmt::Debug for Chunk<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Chunk")
            .field("first_sequence", &self.first_sequence())
            .field("last_sequence", &self.last_sequence())
            .field("entries", &self.entries.len())
            .field("accounted_payload_bytes", &self.accounted_payload_bytes)
            .finish()
    }
}

impl<T> Chunk<T> {
    pub fn entries(&self) -> &[Arc<Envelope<T>>] {
        &self.entries
    }

    pub fn first_sequence(&self) -> Option<u64> {
        self.entries.first().map(|entry| entry.sequence)
    }

    pub fn last_sequence(&self) -> Option<u64> {
        self.entries.last().map(|entry| entry.sequence)
    }

    pub fn accounted_payload_bytes(&self) -> usize {
        self.accounted_payload_bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkBuilderConfig {
    pub max_entries: usize,
    pub max_bytes: usize,
    pub max_latency: Duration,
    pub poll_interval: Duration,
}

impl Default for ChunkBuilderConfig {
    fn default() -> Self {
        Self {
            max_entries: 64,
            max_bytes: 256 * 1024,
            max_latency: Duration::from_millis(10),
            poll_interval: Duration::from_millis(1),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChunkBuilderStats {
    pub observed_entries: u64,
    pub source_gap_events: u64,
    pub source_gap_entries: u64,
    pub sealed_chunks: u64,
    pub sealed_entries: u64,
    pub sealed_payload_bytes: u64,
    pub stopped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkBuilderError {
    ZeroEntryThreshold,
    ZeroByteThreshold,
    ZeroLatency,
}

impl fmt::Display for ChunkBuilderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroEntryThreshold => formatter.write_str("max_entries must be positive"),
            Self::ZeroByteThreshold => formatter.write_str("max_bytes must be positive"),
            Self::ZeroLatency => formatter.write_str("max_latency must be positive"),
        }
    }
}

impl std::error::Error for ChunkBuilderError {}

#[derive(Default)]
struct AtomicChunkStats {
    observed_entries: AtomicU64,
    source_gap_events: AtomicU64,
    source_gap_entries: AtomicU64,
    sealed_chunks: AtomicU64,
    sealed_entries: AtomicU64,
    sealed_payload_bytes: AtomicU64,
}

/// Asynchronous sidecar that follows a Buffer and seals immutable chunks.
/// Buffer append and live subscribers never wait for this worker.
#[must_use = "dropping a ChunkBuilder stops retained chunk construction"]
pub struct ChunkBuilder<T> {
    stopped: Arc<AtomicBool>,
    chunks: Arc<Mutex<Vec<Arc<Chunk<T>>>>>,
    stats: Arc<AtomicChunkStats>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl<T: Send + Sync + 'static> ChunkBuilder<T> {
    pub fn start(
        buffer: &Buffer<T>,
        start: CursorStart,
        config: ChunkBuilderConfig,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<Self, ChunkBuilderError> {
        if config.max_entries == 0 {
            return Err(ChunkBuilderError::ZeroEntryThreshold);
        }
        if config.max_bytes == 0 {
            return Err(ChunkBuilderError::ZeroByteThreshold);
        }
        if config.max_latency.is_zero() {
            return Err(ChunkBuilderError::ZeroLatency);
        }

        let stopped = Arc::new(AtomicBool::new(false));
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let stats = Arc::new(AtomicChunkStats::default());
        let retained_size = Arc::new(retained_size);

        let worker_stopped = Arc::clone(&stopped);
        let worker_chunks = Arc::clone(&chunks);
        let worker_stats = Arc::clone(&stats);
        let mut cursor = buffer.subscribe(start);
        let worker = thread::Builder::new()
            .name("typed-dataflow-chunk-builder".into())
            .spawn(move || {
                let mut open = Vec::with_capacity(config.max_entries);
                let mut open_bytes = 0usize;
                let mut opened_at = Instant::now();

                while !worker_stopped.load(Ordering::Acquire) {
                    match cursor.next_timeout(config.poll_interval) {
                        CursorRead::Item(envelope) => {
                            if open.is_empty() {
                                opened_at = Instant::now();
                            }
                            open_bytes += retained_size(&envelope.payload);
                            open.push(envelope);
                            worker_stats
                                .observed_entries
                                .fetch_add(1, Ordering::Relaxed);
                            if open.len() >= config.max_entries || open_bytes >= config.max_bytes {
                                seal(&worker_chunks, &worker_stats, &mut open, &mut open_bytes);
                            }
                        }
                        CursorRead::Gap(gap) => {
                            worker_stats
                                .source_gap_events
                                .fetch_add(1, Ordering::Relaxed);
                            worker_stats.source_gap_entries.fetch_add(
                                gap.available_from.saturating_sub(gap.requested_sequence),
                                Ordering::Relaxed,
                            );
                        }
                        CursorRead::Timeout => {
                            if !open.is_empty() && opened_at.elapsed() >= config.max_latency {
                                seal(&worker_chunks, &worker_stats, &mut open, &mut open_bytes);
                            }
                        }
                        CursorRead::Closed => break,
                    }
                }

                seal(&worker_chunks, &worker_stats, &mut open, &mut open_bytes);
            })
            .expect("failed to spawn chunk builder");

        Ok(Self {
            stopped,
            chunks,
            stats,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub fn chunks(&self) -> Vec<Arc<Chunk<T>>> {
        self.chunks.lock().unwrap().clone()
    }

    pub fn stats(&self) -> ChunkBuilderStats {
        ChunkBuilderStats {
            observed_entries: self.stats.observed_entries.load(Ordering::Relaxed),
            source_gap_events: self.stats.source_gap_events.load(Ordering::Relaxed),
            source_gap_entries: self.stats.source_gap_entries.load(Ordering::Relaxed),
            sealed_chunks: self.stats.sealed_chunks.load(Ordering::Relaxed),
            sealed_entries: self.stats.sealed_entries.load(Ordering::Relaxed),
            sealed_payload_bytes: self.stats.sealed_payload_bytes.load(Ordering::Relaxed),
            stopped: self.stopped.load(Ordering::Acquire),
        }
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.lock().unwrap().take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

impl<T> Drop for ChunkBuilder<T> {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.get_mut().unwrap().take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

fn seal<T>(
    chunks: &Mutex<Vec<Arc<Chunk<T>>>>,
    stats: &AtomicChunkStats,
    open: &mut Vec<Arc<Envelope<T>>>,
    open_bytes: &mut usize,
) {
    if open.is_empty() {
        return;
    }
    let entries = std::mem::take(open);
    let accounted_payload_bytes = std::mem::take(open_bytes);
    let entry_count = entries.len() as u64;
    chunks.lock().unwrap().push(Arc::new(Chunk {
        entries: entries.into(),
        accounted_payload_bytes,
    }));
    stats.sealed_chunks.fetch_add(1, Ordering::Relaxed);
    stats
        .sealed_entries
        .fetch_add(entry_count, Ordering::Relaxed);
    stats
        .sealed_payload_bytes
        .fetch_add(accounted_payload_bytes as u64, Ordering::Relaxed);
}
