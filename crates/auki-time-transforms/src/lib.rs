//! TimeTransform Log + 1 Hz `local_clock_read` sampler.
//!
//! Schema spec: [`docs/timetransform-log.md`](../../../docs/timetransform-log.md).
//!
//! - [`TimeTransformEntry`] is the per-sample payload written to an [`auki_logs::Log`].
//! - [`tick`] is the unit-testable primitive: read three clocks, build one entry.
//! - [`Sampler`] wraps `tick` in a 1 Hz background thread for production use.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use auki_logs;

/// One TimeTransform sample. Lives in the segment payload (CBOR-encoded).
///
/// The framing's `timestamp_ns` (added by `auki_logs::Log::append`) is the
/// from-clock reading at the sample instant — not duplicated here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeTransformEntry {
    /// `to_clock - from_clock` at the sample instant, in nanoseconds.
    pub offset_ns: i64,
    /// Window during which the to-clock was read, in from-clock nanoseconds.
    pub uncertainty_ns: u32,
    pub source: TimeTransformSource,
    /// `true` iff `|offset_ns - prev_offset_ns| ≥ threshold` set on the sampler.
    /// Always `false` on the first sample (no prior offset to compare against).
    pub discontinuous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeTransformSource {
    LocalClockRead,
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
    debug_assert_eq!(rc, 0, "clock_gettime failed; errno={}", std::io::Error::last_os_error());
    (ts.tv_sec as i64).saturating_mul(1_000_000_000) + ts.tv_nsec as i64
}

/// State carried between consecutive `tick` calls — the prior offset (for
/// discontinuity detection) and the threshold (in nanoseconds).
#[derive(Debug, Clone, Copy)]
pub struct SamplerState {
    pub last_offset_ns: Option<i64>,
    pub threshold_ns: i64,
}

impl SamplerState {
    pub fn new(threshold_ns: i64) -> Self {
        Self {
            last_offset_ns: None,
            threshold_ns: threshold_ns.max(0),
        }
    }
}

/// Take one sample. Reads from-clock, to-clock, from-clock back-to-back
/// (see schema spec for the three-read protocol). Returns the
/// `(timestamp_ns, entry)` tuple ready for `Log::append`.
pub fn tick(clock: &dyn Clock, state: &mut SamplerState) -> (i64, TimeTransformEntry) {
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

    let discontinuous = match state.last_offset_ns {
        Some(prev) => offset_ns
            .checked_sub(prev)
            .map(i64::unsigned_abs)
            .map(|d| d >= state.threshold_ns as u64)
            .unwrap_or(true),
        None => false,
    };
    state.last_offset_ns = Some(offset_ns);

    let entry = TimeTransformEntry {
        offset_ns,
        uncertainty_ns,
        source: TimeTransformSource::LocalClockRead,
        discontinuous,
    };
    (timestamp_ns, entry)
}

fn midpoint(a: i64, b: i64) -> i64 {
    // Avoids overflow that `(a + b) / 2` can hit at i64 extremes.
    a + (b - a) / 2
}

/// Build a TimeTransform Log manifest with the four required clock-binding
/// fields plus auki-logs's required `segment_duration_ns` / `retention_ns`.
pub fn build_manifest(
    from_clock_id: &str,
    from_clock_hash: &str,
    to_clock_id: &str,
    to_clock_hash: &str,
    segment_duration: Duration,
    retention: Duration,
) -> serde_json::Value {
    serde_json::json!({
        "from_clock_id": from_clock_id,
        "from_clock_hash": from_clock_hash,
        "to_clock_id": to_clock_id,
        "to_clock_hash": to_clock_hash,
        "segment_duration_ns": duration_as_i64_ns(segment_duration),
        "retention_ns": duration_as_i64_ns(retention),
    })
}

fn duration_as_i64_ns(d: Duration) -> i64 {
    d.as_nanos().min(i64::MAX as u128) as i64
}

/// Background sampler. Calls `tick` every `period`, appending each entry to
/// the log. Stops cleanly via the returned handle.
pub struct Sampler {
    handle: JoinHandle<auki_logs::Log<TimeTransformEntry>>,
    stop: Arc<AtomicBool>,
}

impl Sampler {
    /// Spawn the sampling thread. Default discontinuity threshold is 10 ms;
    /// pass a `Duration` to override.
    pub fn start(
        log: auki_logs::Log<TimeTransformEntry>,
        clock: Box<dyn Clock>,
        period: Duration,
        discontinuity_threshold: Duration,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let threshold_ns = duration_as_i64_ns(discontinuity_threshold);

        let handle = thread::spawn(move || {
            let mut log = log;
            let mut state = SamplerState::new(threshold_ns);
            while !stop_clone.load(Ordering::Relaxed) {
                let (timestamp_ns, entry) = tick(&*clock, &mut state);
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
        let mut state = SamplerState::new(10_000_000); // 10 ms threshold
        let (ts, entry) = tick(&clock, &mut state);

        assert_eq!(ts, 1_100);
        assert_eq!(entry.offset_ns, 1_999_999_400);
        assert_eq!(entry.uncertainty_ns, 200);
        assert_eq!(entry.source, TimeTransformSource::LocalClockRead);
        assert!(!entry.discontinuous, "first entry never flagged");
    }

    #[test]
    fn first_tick_never_flags_discontinuous() {
        let clock = ScriptedClock::new([0, 0], [10_000_000_000]); // huge offset, no prior
        let mut state = SamplerState::new(1);
        let (_, entry) = tick(&clock, &mut state);
        assert!(!entry.discontinuous);
    }

    #[test]
    fn drift_smaller_than_threshold_does_not_flag() {
        // Two samples: offset stays approximately constant.
        let clock = ScriptedClock::new(
            [1_000, 1_100, 1_000_001_000, 1_000_001_100],
            [2_000_000_500, 3_000_001_500],
        );
        let mut state = SamplerState::new(10_000_000); // 10 ms

        let (_, e1) = tick(&clock, &mut state);
        let (_, e2) = tick(&clock, &mut state);

        // Offsets differ by ~1 µs (well under 10 ms).
        let drift = (e2.offset_ns - e1.offset_ns).abs();
        assert!(drift < 10_000_000, "drift was {drift}");
        assert!(!e1.discontinuous);
        assert!(!e2.discontinuous);
    }

    #[test]
    fn step_larger_than_threshold_flags_discontinuous() {
        // Sample 1: offset ≈ 2_000_000_000.
        // Sample 2: to-clock jumps forward by 50 ms (NTP step).
        let clock = ScriptedClock::new(
            [1_000, 1_100, 1_000_001_000, 1_000_001_100],
            [2_000_000_550, 3_050_001_550], // +50 ms step on top of natural progression
        );
        let mut state = SamplerState::new(10_000_000); // 10 ms

        let (_, e1) = tick(&clock, &mut state);
        let (_, e2) = tick(&clock, &mut state);
        assert!(!e1.discontinuous);
        assert!(
            e2.discontinuous,
            "expected step flag (offsets {} vs {})",
            e1.offset_ns, e2.offset_ns
        );
    }

    #[test]
    fn step_below_threshold_does_not_flag() {
        let clock = ScriptedClock::new(
            [1_000, 1_100, 1_000_001_000, 1_000_001_100],
            [2_000_000_550, 3_005_001_550], // +5 ms step (under 10 ms threshold)
        );
        let mut state = SamplerState::new(10_000_000);
        let (_, e1) = tick(&clock, &mut state);
        let (_, e2) = tick(&clock, &mut state);
        assert!(!e1.discontinuous);
        assert!(!e2.discontinuous);
    }

    #[test]
    fn negative_step_also_flags() {
        // UTC corrected backwards.
        let clock = ScriptedClock::new(
            [1_000, 1_100, 1_000_001_000, 1_000_001_100],
            [2_000_000_550, 950_001_550], // jumps backwards 1+ second
        );
        let mut state = SamplerState::new(10_000_000);
        let (_, _) = tick(&clock, &mut state);
        let (_, e2) = tick(&clock, &mut state);
        assert!(e2.discontinuous);
    }

    #[test]
    fn source_serializes_snake_case() {
        let json = serde_json::to_string(&TimeTransformSource::LocalClockRead).unwrap();
        assert_eq!(json, r#""local_clock_read""#);
    }

    #[test]
    fn entry_round_trips_through_cbor() {
        let entry = TimeTransformEntry {
            offset_ns: -123_456_789,
            uncertainty_ns: 42,
            source: TimeTransformSource::LocalClockRead,
            discontinuous: true,
        };
        let mut buf = Vec::new();
        ciborium::into_writer(&entry, &mut buf).unwrap();
        let back: TimeTransformEntry = ciborium::from_reader(&buf[..]).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn build_manifest_contains_required_fields() {
        let m = build_manifest(
            "K1-AABBCCDDEEFF/monotonic",
            "deadbeefcafefeed",
            "K1-AABBCCDDEEFF/utc",
            "1234567890abcdef",
            Duration::from_secs(1),
            Duration::from_secs(60),
        );
        assert_eq!(m["from_clock_id"], "K1-AABBCCDDEEFF/monotonic");
        assert_eq!(m["to_clock_hash"], "1234567890abcdef");
        assert_eq!(m["segment_duration_ns"], 1_000_000_000i64);
        assert_eq!(m["retention_ns"], 60_000_000_000i64);
    }

    #[test]
    fn sampler_writes_entries_then_stops_cleanly() {
        // Use ScriptedClock with plenty of pre-loaded readings so the sampler
        // doesn't run out before we stop it. Each tick consumes 2 from-reads + 1 to-read.
        const N: usize = 200;
        let from: Vec<i64> = (0..(2 * N) as i64).map(|i| i * 1_000).collect();
        let to: Vec<i64> = (0..N as i64).map(|i| 5_000_000_000 + i * 1_000_000).collect();
        let clock = Box::new(ScriptedClock::new(from, to));

        let dir = tempfile::tempdir().unwrap();
        let manifest = build_manifest(
            "test/from",
            "fhash",
            "test/to",
            "thash",
            Duration::from_millis(100),
            Duration::from_secs(60),
        );
        let log = auki_logs::Log::<TimeTransformEntry>::open(dir.path(), manifest).unwrap();

        let sampler = Sampler::start(log, clock, Duration::from_millis(5), Duration::from_millis(10));
        thread::sleep(Duration::from_millis(50));
        let log = sampler.stop();
        drop(log); // release file handles before re-reading

        let reader = auki_logs::Log::<TimeTransformEntry>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert!(
            !entries.is_empty(),
            "sampler should have written at least one entry"
        );
        // First entry never flagged.
        assert!(!entries[0].payload.discontinuous);
        // Sources are all local_clock_read.
        for e in &entries {
            assert_eq!(e.payload.source, TimeTransformSource::LocalClockRead);
        }
    }
}
