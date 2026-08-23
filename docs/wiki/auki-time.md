# auki-time and Time Sync

auki-time supplies the core time primitives used for peer clock sync and TimeTransform Log production in the SDK.

It owns NTP four-timestamp math, rolling per-pair offset estimates, domain clock composition from backing clocks, per-session monotonic clocks, and the 1 Hz sampler.

The surface spans auki-time (primitives), auki-datatypes (entry payload), auki-manifests (source enum + manifest builder), auki-session (log spec and handle), and auki-registry (clock entries).

Consumers reach it through Session for log registration or directly for sync state and tick.

See crate README at crates/auki-time/README.md and Crate-Map.md for overview.

## Post-Step-6 shape (2026-05-08)

TimeTransformEntry holds only offset_ns: i64 and uncertainty_ns: u32.

TimeTransformSource moved to the log manifest.

Discontinuity detection moved to readers: compare neighboring offset_ns values against a reader-chosen threshold.

Migration comments remain in source and proto for history.

## Public API reference

### TimeTransform
```rust
pub struct TimeTransform {
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub observed_at_clock_ns: i64,
    // from_clock_id, to_clock_id private
}
```
- new(from, to, offset, uncertainty, observed)
- from_clock_id(&self) -> &str
- to_clock_id(&self) -> &str
- convert_ns(&self, ts: i64) -> Option<i64>  (checked_add)

Affine offset from one clock to another. Used for local<->UTC and peer pairs.

### NtpExchange
```rust
pub struct NtpExchange {
    pub local_send_ns: i64,
    pub remote_receive_ns: i64,
    pub remote_send_ns: i64,
    pub local_receive_ns: i64,
}
```
Four timestamps from one heartbeat exchange.

### NtpSample
```rust
pub struct NtpSample {
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub round_trip_ns: u64,
    pub remote_processing_ns: u64,
    pub observed_at_clock_ns: i64,
}
```
Result of compute_ntp_sample.

### NtpSampleError
```rust
pub enum NtpSampleError {
    LocalClockWentBackwards,
    RemoteClockWentBackwards,
    RoundTripShorterThanRemoteProcessing,
    OffsetOutOfRange,
}
```
With Display + Error.

### compute_ntp_sample
```rust
pub fn compute_ntp_sample(exchange: NtpExchange) -> Result<NtpSample, NtpSampleError>
```
Core math. Handles independent monotonic epochs.

### compute_ntp_offset
Thin wrapper around compute_ntp_sample returning only the offset.

### select_best_ntp_sample
```rust
pub fn select_best_ntp_sample(samples: &[NtpSample]) -> Option<NtpSample>
```
Selects min uncertainty, then newest observed_at.

### ClockSyncConfig
```rust
pub struct ClockSyncConfig {
    pub max_samples_per_pair: usize,
    pub max_sample_age_ns: u64,
    pub max_uncertainty_ns: u64,
}
```
Default: 32 / 10_000_000_000 / 500_000_000

### ClockSyncObservation
```rust
pub struct ClockSyncObservation {
    pub local_clock_id: String,
    pub local_clock_hash: String,
    pub remote_clock_id: String,
    pub remote_clock_hash: String,
    pub sample: NtpSample,
}
```
Input to state. Hash change clears the sample window for that pair.

### ClockTransformEstimate
```rust
pub struct ClockTransformEstimate {
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub observed_at_clock_ns: i64,
    pub sample_count: usize,
    // clock ids and hashes private with accessors
}
```
- identity(clock_id, hash, observed)
- from_clock_id, from_clock_hash, to_*, time_transform() -> TimeTransform

Best estimate for one ordered clock pair.

### ClockSyncState
```rust
pub struct ClockSyncState { ... }
```
- new(config)
- observe(&mut self, obs) -> Option<ClockTransformEstimate>
- estimate(&self, local_id, remote_id) -> Option<...>
- estimates(&self) -> Vec<...>

Rolling window. Rejects high-uncertainty and stale samples. Prunes on hash change.

### ClockSyncHandle
```rust
pub struct ClockSyncHandle { ... }
```
Clone + Default. Arc<Mutex> wrapper.
Same methods as state, thread-safe for runtime tasks.

### DomainClockDescriptor
```rust
pub struct DomainClockDescriptor {
    pub cluster_name: String,
    pub domain_clock: RegistryRef,
    pub backing_peer_id: String,
    pub backing_clock: RegistryRef,
    pub backing_to_domain_offset_ns: i64,
    pub backing_to_domain_uncertainty_ns: u64,
}
```
new(...) 

Used by cluster manager to describe how a backing clock maps to domain clock.

### DomainClockEstimate
```rust
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
```
- time_transform() -> TimeTransform

Composed local -> domain offset.

### DomainClockEstimateError
```rust
pub enum DomainClockEstimateError {
    BackingClockIdMismatch { expected: String, actual: String },
    BackingClockHashMismatch { expected: String, actual: String },
    TotalOffsetOutOfRange,
}
```
With Display.

### estimate_domain_clock
```rust
pub fn estimate_domain_clock(
    local_to_backing: &ClockTransformEstimate,
    descriptor: DomainClockDescriptor,
) -> Result<DomainClockEstimate, DomainClockEstimateError>
```
Validates backing match then adds offsets.

### Clock trait and SystemClock
```rust
pub trait Clock {
    fn read_from_ns(&self) -> i64;
    fn read_to_ns(&self) -> i64;
}
pub struct SystemClock;
```
impl Clock using libc clock_gettime (MONOTONIC / REALTIME).

For testable injection.

### tick
```rust
pub fn tick(clock: &dyn Clock) -> (i64, TimeTransformEntry)
```
Three-read protocol: m1, r, m2. Midpoint timestamp + offset/uncertainty entry.

Long comment in source explains the protocol and Step-6 changes.

### SessionClock
```rust
pub struct SessionClock { ... }
```
- new(peer_id, session_id, name) -> Self
- now_ns(&self) -> u64
- now_i64_ns(&self) -> i64
- clock_id(&self) -> &str
- clock_hash(&self) -> &str
- registry_entry(&self) -> ClockRegistryEntry

Per-session monotonic. First segment of clock_id is peer. Epoch = session_id. Scope = DeviceLocal. Recommended default for TimeTransform Log samples.

### Sampler
```rust
pub struct Sampler { ... }
```
- start(log: Log<TimeTransformEntry>, clock: Box<dyn Clock>, period: Duration) -> Self
- stop(self) -> Log<TimeTransformEntry>

Spawns 1 Hz thread calling tick and appending. On append error: eprintln. stop joins and returns the log for manifest finalization.

### Re-exports
pub use auki_datatypes::time_transform::TimeTransformEntry;
pub use auki_manifests::TimeTransformSource;

Note: `pub use auki_logs;` also exists for internal convenience but external code should depend on the auki-logs crate directly.

### TimeTransformSource (from manifests)
```rust
pub enum TimeTransformSource {
    LocalClockRead,
    // future variants planned
}
```
Only LocalClockRead implemented. Extension point for NtpSynced / SyncedTo variants.

Methods: canonical_bytes, hash (JCS).

### TimeTransformLogManifest and builder (from manifests)
See auki-manifests for build_time_transform_log_manifest and the manifest struct.

### TimeTransformLogSpec and Handle (from session)
```rust
pub struct TimeTransformLogSpec {
    pub from_clock: RegistryRef,
    pub to_clock: RegistryRef,
    pub source: TimeTransformSource,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}
pub struct TimeTransformLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    // manifest and head_spec private
}
```
- resource_id(&self)
- log_ref(&self)

Register via Session::register_time_transform_log.

### Clock registry types (from registry)
ClockRegistryEntry, ClockBody::MonotonicClock(ClockMeta), ClockMeta { unit, monotonic, epoch, scope: Scope }, RegistryRef, Scope::DeviceLocal / DomainLocal / Global.

Used to describe clocks in catalog and manifests.

## Time sync flow

1. Each node creates a SessionClock and starts a Sampler writing to a TimeTransform Log (source = LocalClockRead).

2. Heartbeat protocol exchanges NtpExchange data between peers.

3. Receivers construct ClockSyncObservation and call ClockSyncHandle::observe. Returns best estimate when available.

4. ClusterManager selects domain clock source, builds DomainClockDescriptor, calls estimate_domain_clock to produce DomainClockEstimate for the cluster.

5. The resulting TimeTransform Log + manifest lets future readers apply offsets (convert_time pending).

When to use the sync surface: cluster daemons that need domain time for coordination or cross-peer log alignment. Low-level users call compute_ntp_sample or tick directly. Session users register the log spec.

## Edge cases and error conditions

- Clock hash change in observation: sample window cleared for that pair (prevents epoch mixing).
- Sample uncertainty > config max: rejected before window.
- Sample age > max: pruned.
- Ntp math errors: Local/Remote clock backwards, round-trip too short, offset out of range.
- Domain composition: backing id/hash mismatch or total offset overflows i64.
- Sampler append failure: eprintln only (no panic, log continues).
- No convert_time consumer yet (per crate status).

## Usage notes

- Prefer SessionClock over SystemClock for logs (peer-anchored + explicit epoch).
- Domain clock composition allows cluster time without electing a master clock.
- TimeTransform Log is the durable record; ClockSyncHandle is runtime state only.
- All timestamps in nanoseconds. Offsets are to_clock - from_clock.

See examples/diagnostic-app for usage patterns and tests in auki-time for edge coverage.