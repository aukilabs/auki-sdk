//! TimeTransform math, session clocks, and the 1 Hz `local_clock_read`
//! sampler.
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
//! - [`TimeTransformSource`] is
//!   re-exported from `auki-manifests` (it's manifest metadata, not a
//!   per-entry field).
//! - [`tick`] is the unit-testable primitive: read three clocks, build
//!   one entry.
//! - [`Sampler`] wraps `tick` in a 1 Hz background thread for
//!   production use.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub use auki_logs;
use auki_registry::{ClockBody, ClockMeta, ClockRegistryEntry, Scope};

// Re-exports for short call sites at consumer crates.
pub use auki_datatypes::time_transform::TimeTransformEntry;
pub use auki_manifests::TimeTransformSource;

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
    let nanoseconds = (ts.tv_sec as i128)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec as i128);
    nanoseconds.clamp(i64::MIN as i128, i64::MAX as i128) as i64
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
            peer_id: peer_id.clone(),
            session_id: session_id.clone(),
            clock_id: format!("{peer_id}/{session_id}/{name}"),
            body: ClockBody::MonotonicClock(ClockMeta {
                unit: "ns".into(),
                monotonic: true,
                // Monotonic clocks have no wall-clock zero — epoch is null.
                // The session is carried by the typed `session_id` field, not
                // smuggled into `epoch`. See #274 (D6).
                epoch: None,
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
    use auki_registry::RegistryRef;
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
    fn session_clock_builds_peer_anchored_registry_entry_with_typed_session_id() {
        let peer_id = "12D3KooWPeerExample";
        let clock = SessionClock::new(peer_id, "session-123", "monotonic");

        let entry = clock.registry_entry();
        assert_eq!(entry.clock_id, "12D3KooWPeerExample/session-123/monotonic");
        // The session is carried by the typed `session_id` field…
        assert_eq!(entry.session_id, "session-123");
        match &entry.body {
            ClockBody::MonotonicClock(meta) => {
                assert_eq!(meta.unit, "ns");
                assert!(meta.monotonic);
                assert_eq!(meta.scope, Scope::DeviceLocal);
                // …not smuggled into `epoch`. A monotonic clock has no
                // wall-clock zero, so epoch is null. See #274 (D6).
                assert_eq!(meta.epoch, None);
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
        let to: Vec<i64> = (0..N as i64)
            .map(|i| 5_000_000_000 + i * 1_000_000)
            .collect();
        let clock = Box::new(ScriptedClock::new(from, to));

        let dir = tempfile::tempdir().unwrap();
        let manifest = auki_manifests::build_time_transform_log_manifest(
            "test-peer",
            "test-peer",
            "test-app",
            "550e8400-e29b-41d4-a716-446655440000",
            RegistryRef {
                peer_id: "test-peer".into(),
                id: "test/from".into(),
                hash: "fhash".into(),
            },
            RegistryRef {
                peer_id: "test-peer".into(),
                id: "test/to".into(),
                hash: "thash".into(),
            },
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
