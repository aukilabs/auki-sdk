//! TimeTransform Log + 1 Hz `local_clock_read` sampler.
//!
//! Schema spec: [`../README.md`](../README.md).
//!
//! - [`auki_datatypes::time_transform::TimeTransformEntry`] is the
//!   per-sample payload (re-exported here for short call sites). Lives
//!   in [`auki-datatypes`](../auki-datatypes) since Step 6 of the
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

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub use auki_logs;

// Re-exports for short call sites at consumer crates.
pub use auki_datatypes::time_transform::TimeTransformEntry;
pub use auki_manifests::TimeTransformSource;

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
    debug_assert_eq!(rc, 0, "clock_gettime failed; errno={}", std::io::Error::last_os_error());
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
    let uncertainty_ns: u32 = m2
        .saturating_sub(m1)
        .max(0)
        .try_into()
        .unwrap_or(u32::MAX);

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

// `build_manifest` (renamed `build_time_transform_log_manifest`) moved to
// [`auki-manifests`] in Step 0 of the auki-datatypes migration.

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
                    eprintln!("auki-time-transforms: append failed: {e}");
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
            self.from.lock().unwrap().pop_front().expect("from-clock script exhausted")
        }
        fn read_to_ns(&self) -> i64 {
            self.to.lock().unwrap().pop_front().expect("to-clock script exhausted")
        }
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

    // `build_manifest_contains_required_fields` moved to [`auki-manifests`]
    // (renamed `build_time_transform_log_manifest_contains_required_fields`)
    // in Step 0 of the auki-datatypes migration. Discontinuity-detection
    // tests dropped at Step 6 — `discontinuous` is a reader-side
    // computation now, no longer baked into the entry. Source-snake-case
    // test moved to [`auki-manifests`] alongside `TimeTransformSource`.
    // CBOR round-trip test dropped — entry encoding moved to prost in
    // [`auki-datatypes`](../auki-datatypes), where the round-trip test
    // also lives.

    #[test]
    fn sampler_writes_entries_then_stops_cleanly() {
        // Use ScriptedClock with plenty of pre-loaded readings so the sampler
        // doesn't run out before we stop it. Each tick consumes 2 from-reads + 1 to-read.
        const N: usize = 200;
        let from: Vec<i64> = (0..(2 * N) as i64).map(|i| i * 1_000).collect();
        let to: Vec<i64> = (0..N as i64).map(|i| 5_000_000_000 + i * 1_000_000).collect();
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
        // Sample contents are pinned by auki-datatypes' round-trip tests;
        // here we just confirm the sampler wrote something readable.
        for e in &entries {
            // `uncertainty_ns` is bounded; ScriptedClock advances linearly.
            assert!(e.payload.uncertainty_ns < 1_000_000_000);
        }
    }
}
