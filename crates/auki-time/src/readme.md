# `auki-time/src/`

SDK time primitives and the 1 Hz `local_clock_read` sampler producing `TimeTransformEntry` records. Schema spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). Step 6 of the [`auki-datatypes` migration](../../auki-datatypes/src/sprint.md) (2026-05-08) moved the data-type definitions out; this crate now owns clock-reading primitives while re-exporting the canonical TimeTransform payload types.

The crate has three composable layers:
1. The **session clock** ([`SessionClock`]) — SDK-owned session-monotonic identity and reader. Clock ids are `<peer_id>/<session_id>/monotonic`; the first segment is the authoring peer id, and the registry entry's epoch marker is the session id.
2. The **unit-testable TimeTransform primitive** ([`tick`]) — pure function over a `Clock` trait. Reads three clocks, builds one entry. No state.
3. The **production sampler thread** ([`Sampler`]) — wraps `tick` in a 1 Hz background loop.

## Re-exports

```rust
pub use auki_datatypes::time_transform::TimeTransformEntry;  // { offset_ns: i64, uncertainty_ns: u32 }
pub use auki_manifests::TimeTransformSource;                  // tagged enum, lives in the manifest
```

The framing's `timestamp_ns` (provided by `auki_logs::Log::append`) is the **from-clock midpoint** at the sample instant. Not duplicated in the payload.

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

## Tests (4 total)

| Test | Asserts |
|------|---------|
| `session_clock_builds_peer_anchored_registry_entry_with_epoch_marker` | `SessionClock` creates a peer-id rooted monotonic `ClockRegistryEntry` with `Scope::DeviceLocal`, session epoch marker, and matching hash. |
| `session_clock_now_is_monotonic` | Session-clock readings do not move backwards. |
| `tick_computes_offset_uncertainty_and_timestamp` | The math is right for one canned set of readings (offset, uncertainty, timestamp). |
| `sampler_writes_entries_then_stops_cleanly` | Threaded integration test against `ScriptedClock`; entries are written and stop cleanly via the join handle. |

The Step 6 cleanup dropped 6 discontinuity-detection tests (logic moved to readers; no producer-side discontinuity to test in this crate any more), the snake-case `TimeTransformSource` test (moved to [`auki-manifests`](../../auki-manifests) where the type now lives), and the CBOR round-trip test (replaced by the prost round-trip in [`auki-datatypes::tests`](../../auki-datatypes/src/lib.rs)).

## Consumers in this workspace

- `auki-k1-binary` (downstream, planned) — opens a `TimeTransform` log + spawns a `Sampler` for the lifetime of a session.
