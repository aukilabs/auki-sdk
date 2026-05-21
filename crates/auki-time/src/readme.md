# `auki-time/src/`

SDK time primitives, time-transform math, NTP-style offset samples, peer-clock sync state, domain-clock composition, and the 1 Hz `local_clock_read` sampler producing `TimeTransformEntry` records. Schema spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). Step 6 of the [`auki-datatypes` migration](../../auki-datatypes/src/sprint.md) (2026-05-08) moved the log payload definition out; this crate owns session clocks, the producer sampler, and pure time-transform math while re-exporting the canonical TimeTransform payload types.

The crate has six composable layers:
1. The **session clock** ([`SessionClock`]) — SDK-owned session-monotonic identity and reader. Clock ids are `<peer_id>/<session_id>/monotonic`; the first segment is the authoring peer id, and the registry entry's epoch marker is the session id.
2. The **pure transform API** (`TimeTransform`, `NtpExchange`, `NtpSample`, `compute_ntp_sample`, `select_best_ntp_sample`) — no IO, no background tasks.
3. The **peer-clock sync state** (`ClockSyncState`, `ClockSyncHandle`, `ClockSyncObservation`, `ClockTransformEstimate`) — bounded retention and selection policy over heartbeat-derived samples, plus a cloneable shared handle for runtime/event tasks.
4. The **domain-clock composition API** (`DomainClockDescriptor`, `DomainClockEstimate`, `estimate_domain_clock`) — pure composition from `local -> backing` estimate plus `backing -> domain` descriptor into `local -> domain`.
5. The **unit-testable local sampler primitive** ([`tick`]) — pure function over a `Clock` trait. Reads three clocks, builds one entry. No state.
6. The **production thread** ([`Sampler`]) — wraps `tick` in a 1 Hz background loop.

## Re-exports

```rust
pub use auki_datatypes::time_transform::TimeTransformEntry;  // { offset_ns: i64, uncertainty_ns: u32 }
pub use auki_manifests::TimeTransformSource;                  // tagged enum, lives in the manifest
```

The framing's `timestamp_ns` (provided by `auki_logs::Log::append`) is the **from-clock midpoint** at the sample instant. Not duplicated in the payload.

## Pure transform API

```rust
pub struct TimeTransform {
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub observed_at_clock_ns: i64,
}

pub struct NtpExchange {
    pub local_send_ns: i64,
    pub remote_receive_ns: i64,
    pub remote_send_ns: i64,
    pub local_receive_ns: i64,
}

pub struct NtpSample {
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub round_trip_ns: u64,
    pub remote_processing_ns: u64,
    pub observed_at_clock_ns: i64,
}

pub fn compute_ntp_sample(exchange: NtpExchange) -> Result<NtpSample, NtpSampleError>;
pub fn compute_ntp_offset(exchange: NtpExchange) -> Result<i64, NtpSampleError>;
pub fn select_best_ntp_sample(samples: &[NtpSample]) -> Option<NtpSample>;
```

NTP samples use `offset_ns = remote_clock - local_clock`. The helper accepts independent monotonic epochs; clock identities live outside the math values and are named on `TimeTransform`.

## Peer-clock sync state

```rust
pub struct ClockSyncConfig {
    pub max_samples_per_pair: usize,
    pub max_sample_age_ns: u64,
    pub max_uncertainty_ns: u64,
}

pub struct ClockSyncObservation {
    pub local_clock_id: String,
    pub local_clock_hash: String,
    pub remote_clock_id: String,
    pub remote_clock_hash: String,
    pub sample: NtpSample,
}

pub struct ClockTransformEstimate {
    pub offset_ns: i64,
    pub uncertainty_ns: u64,
    pub observed_at_clock_ns: i64,
    pub sample_count: usize,
}

impl ClockTransformEstimate {
    pub fn identity(clock_id: impl Into<String>, clock_hash: impl Into<String>, observed_at_clock_ns: i64) -> Self;
}

pub struct ClockSyncState { ... }

impl ClockSyncState {
    pub fn new(config: ClockSyncConfig) -> Self;
    pub fn observe(&mut self, observation: ClockSyncObservation) -> Option<ClockTransformEstimate>;
    pub fn estimate(&self, local_clock_id: &str, remote_clock_id: &str) -> Option<ClockTransformEstimate>;
    pub fn estimates(&self) -> Vec<ClockTransformEstimate>;
}

pub struct ClockSyncHandle { ... }

impl ClockSyncHandle {
    pub fn new(config: ClockSyncConfig) -> Self;
    pub fn observe(&self, observation: ClockSyncObservation) -> Option<ClockTransformEstimate>;
    pub fn estimate(&self, local_clock_id: &str, remote_clock_id: &str) -> Option<ClockTransformEstimate>;
    pub fn estimates(&self) -> Vec<ClockTransformEstimate>;
}
```

`ClockSyncState` is keyed by ordered local/remote clock pair. It rejects samples above `max_uncertainty_ns`, retains at most `max_samples_per_pair`, prunes stale samples using the local observation timestamp, and clears a pair's sample window when the local or remote clock hash changes. Estimates always describe `local_clock -> remote_clock`, and `ClockTransformEstimate::time_transform()` returns the matching `TimeTransform`. `ClockTransformEstimate::identity(...)` creates an exact zero-offset estimate for the case where the local clock is already the backing clock.

`ClockSyncHandle` wraps `ClockSyncState` in `Arc<Mutex<_>>` for tasks that receive heartbeat sample events. Clones share one peer-local sync state; no background thread is spawned.

## Domain-clock composition API

```rust
pub struct DomainClockDescriptor {
    pub cluster_name: String,
    pub domain_clock_id: String,
    pub domain_clock_hash: String,
    pub backing_peer_id: String,
    pub backing_clock_id: String,
    pub backing_clock_hash: String,
    pub backing_to_domain_offset_ns: i64,
}

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

pub fn estimate_domain_clock(
    local_to_backing: ClockTransformEstimate,
    descriptor: DomainClockDescriptor,
) -> Result<DomainClockEstimate, DomainClockEstimateError>;
```

`estimate_domain_clock` composes `local_clock -> backing_clock` with `backing_clock -> cluster_name/domain-clock`. It rejects backing clock id/hash mismatches before composing and returns `TotalOffsetOutOfRange` if the two offsets cannot fit in `i64`.

## `Clock` trait

```rust
pub trait Clock: Send + Sync {
    fn read_from_ns(&self) -> i64;
    fn read_to_ns(&self) -> i64;
}
```

Production: `SystemClock` reads `CLOCK_MONOTONIC` (from-clock) and `CLOCK_REALTIME` (to-clock) via `libc::clock_gettime`.

Tests: `ScriptedClock` (in `#[cfg(test)] mod tests`) pops pre-loaded readings off two queues, one per clock.

## `SessionClock`

```rust
let clock = SessionClock::new(peer_id.to_string(), session_id, "monotonic");
let now_ns = clock.now_ns();
let clock_id = clock.clock_id();
let clock_hash = clock.clock_hash();
let registry_entry = clock.registry_entry();
```

`SessionClock` is deliberately peer-id anchored so another peer can understand the source from the registry id convention. `Scope::DeviceLocal` is correct because the monotonic zero point is local to one peer/session; cluster/domain clocks use a separate `DomainLocal` identity. Time sync should consume this primitive rather than constructing a heartbeat-only clock identity.

## The three-read protocol

`tick(clock)` reads:

1. `m1 = clock.read_from_ns()`
2. `r  = clock.read_to_ns()`
3. `m2 = clock.read_from_ns()`

And computes:

- `timestamp_ns = midpoint(m1, m2)` — overflow-safe via `m1 + (m2 - m1) / 2`
- `offset_ns   = r - timestamp_ns`
- `uncertainty_ns = m2 - m1` (saturating to `u32::MAX`)

The third read makes `uncertainty_ns` a **real** bound on the from-clock, not a guess. It's the actual elapsed time during which the to-clock was sampled.

## Discontinuity is a reader concern

Step 6 dropped the per-entry `discontinuous: bool` flag and the `SamplerState` struct that tracked the previous offset. Readers compute `|offset_ns - prev_offset_ns| ≥ reader_threshold` against their own tolerance — different readers can disagree on what counts as discontinuous, and that's the point. See the outer [README's discontinuity-detection section](../README.md#discontinuity-detection--reader-side) for the recommended 10 ms threshold and the rationale.

## Manifest builder

The `build_time_transform_log_manifest` builder lives in [`auki-manifests`](../../auki-manifests). Step 6 added a `&TimeTransformSource` argument; the manifest carries the producer identity inline.

## Background `Sampler`

```rust
pub struct Sampler { ... }

impl Sampler {
    pub fn start(
        log: auki_logs::Log<TimeTransformEntry>,
        clock: Box<dyn Clock>,
        period: Duration,
    ) -> Self;

    pub fn stop(self) -> auki_logs::Log<TimeTransformEntry>;
}
```

`start` spawns a thread that calls `tick` every `period` and appends each entry to the log. Step 6 dropped the `discontinuity_threshold` arg — discontinuity detection is now reader-side.

`stop` signals the thread, joins, and **returns the log back** so the caller can flush/drop it on the main thread (matters for fsync ordering during shutdown). Append failures are logged via `eprintln!` and the loop continues — a per-tick I/O hiccup shouldn't kill the sampler.

## Tests (21 total)

| Test | Asserts |
|------|---------|
| `session_clock_builds_peer_anchored_registry_entry_with_epoch_marker` | `SessionClock` creates a peer-id rooted monotonic `ClockRegistryEntry` with `Scope::DeviceLocal`, session epoch marker, and matching hash. |
| `session_clock_now_is_monotonic` | Session-clock readings do not move backwards. |
| `time_transform_converts_from_clock_to_to_clock` | `TimeTransform` adds `offset_ns` and preserves clock ids. |
| `ntp_sample_estimates_remote_minus_local_offset_across_independent_monotonic_epochs` | NTP formula works when local and remote monotonic clocks have unrelated epochs. |
| `best_ntp_sample_prefers_lowest_uncertainty_then_latest_observation` | Best-sample selection prefers low uncertainty, then recency. |
| `ntp_sample_rejects_backwards_exchange` | Invalid local clock ordering is rejected. |
| `clock_sync_state_estimates_local_to_remote_transform_from_one_sample` | One accepted sample produces a `local -> remote` estimate and `TimeTransform`. |
| `clock_sync_state_accepts_independent_monotonic_epochs` | Sync state consumes NTP samples whose local/remote clocks have unrelated monotonic epochs. |
| `clock_sync_state_prefers_lower_uncertainty_sample` | Retained sample selection prefers lower uncertainty. |
| `clock_sync_state_clears_pair_samples_when_clock_hash_changes` | A clock hash change clears stale samples for that clock pair. |
| `clock_sync_state_ages_out_stale_samples` | New observations prune old samples outside the configured age window. |
| `clock_sync_state_rejects_samples_above_uncertainty_limit` | Samples noisier than policy are not retained. |
| `clock_sync_state_returns_all_current_estimates` | State snapshots every current pair estimate in deterministic clock-id order. |
| `clock_sync_handle_shares_state_across_clones` | Cloned handles observe and read the same retained sample windows. |
| `domain_clock_estimate_for_initial_manager_is_identity` | Initial Manager backing clock composes to zero domain offset. |
| `domain_clock_estimate_composes_follower_to_backing_offset` | Follower `local -> backing` offset composes with `backing -> domain` offset. |
| `domain_clock_estimate_rejects_backing_clock_id_mismatch` | Descriptor backing id must match the peer-clock estimate target. |
| `domain_clock_estimate_rejects_backing_clock_hash_mismatch` | Descriptor backing hash must match the peer-clock estimate target hash. |
| `domain_clock_estimate_rejects_total_offset_overflow` | Offset composition fails loudly on `i64` overflow. |
| `clock_transform_estimate_identity_has_zero_offset_and_uncertainty` | Identity estimates use the same local/backing clock with zero offset and zero uncertainty. |
| `tick_computes_offset_uncertainty_and_timestamp` | The math is right for one canned set of readings (offset, uncertainty, timestamp). |
| `sampler_writes_entries_then_stops_cleanly` | Threaded integration test against `ScriptedClock`; entries are written and stop cleanly via the join handle. |

The Step 6 cleanup dropped 6 discontinuity-detection tests (logic moved to readers; no producer-side discontinuity to test in this crate any more), the snake-case `TimeTransformSource` test (moved to [`auki-manifests`](../../auki-manifests) where the type now lives), and the CBOR round-trip test (replaced by the prost round-trip in [`auki-datatypes::tests`](../../auki-datatypes/src/lib.rs)).

## Consumers in this workspace

- `auki-k1-binary` (downstream, planned) — opens a `TimeTransform` log + spawns a `Sampler` for the lifetime of a session.
