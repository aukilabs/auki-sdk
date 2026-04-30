# `auki-time-transforms/src/`

TimeTransform Log entry shape + 1 Hz `local_clock_read` sampler. Schema spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs).

The crate has three composable layers:
1. The **entry payload type** ([`TimeTransformEntry`]) — a serde struct rideable on `auki_logs::Log`.
2. The **unit-testable primitive** ([`tick`]) — pure function over a `Clock` trait + `SamplerState`.
3. The **production thread** ([`Sampler`]) — wraps `tick` in a 1 Hz background loop.

## Entry payload

```rust
pub struct TimeTransformEntry {
    pub offset_ns: i64,         // to_clock - from_clock at the sample instant
    pub uncertainty_ns: u32,    // window during which to_clock was read
    pub source: TimeTransformSource,
    pub discontinuous: bool,    // |offset - prev_offset| ≥ threshold
}

#[serde(rename_all = "snake_case")]
pub enum TimeTransformSource {
    LocalClockRead,    // serializes as "local_clock_read"
}
```

The framing's `timestamp_ns` (provided by `auki_logs::Log::append`) is the **from-clock reading** at the sample instant. It's not duplicated in the payload.

## `Clock` trait

```rust
pub trait Clock: Send + Sync {
    fn read_from_ns(&self) -> i64;
    fn read_to_ns(&self) -> i64;
}
```

Production: `SystemClock` reads `CLOCK_MONOTONIC` (from-clock) and `CLOCK_REALTIME` (to-clock) via `libc::clock_gettime`.

Tests: `ScriptedClock` (in `#[cfg(test)] mod tests`) pops pre-loaded readings off two queues, one per clock.

## The three-read protocol

`tick(clock, state)` reads:

1. `m1 = clock.read_from_ns()`
2. `r  = clock.read_to_ns()`
3. `m2 = clock.read_from_ns()`

And computes:

- `timestamp_ns = midpoint(m1, m2)` — overflow-safe via `m1 + (m2 - m1) / 2`
- `offset_ns   = r - timestamp_ns`
- `uncertainty_ns = m2 - m1` (saturating to `u32::MAX`)

The third read makes `uncertainty_ns` a **real** bound on the from-clock, not a guess. It's the actual elapsed time during which the to-clock was sampled.

## Discontinuity flag

```rust
pub struct SamplerState {
    pub last_offset_ns: Option<i64>,
    pub threshold_ns: i64,
}
```

If `|offset - prev_offset| ≥ threshold_ns`, the entry's `discontinuous` flag is set. The first sample is always `false` (no prior offset to compare against). The default threshold is **10 ms** (Sampler-level), chosen to flag NTP step corrections cleanly while ignoring chrony's smaller per-sample slews.

## Manifest builder

```rust
pub fn build_manifest(
    from_clock_id: &str,
    from_clock_hash: &str,
    to_clock_id: &str,
    to_clock_hash: &str,
    segment_duration: Duration,
    retention: Duration,
) -> serde_json::Value;
```

Produces a `serde_json::Value` containing the four clock-binding fields plus `auki-logs`'s required `segment_duration_ns` / `retention_ns`.

## Background `Sampler`

```rust
pub struct Sampler { ... }

impl Sampler {
    pub fn start(
        log: auki_logs::Log<TimeTransformEntry>,
        clock: Box<dyn Clock>,
        period: Duration,
        discontinuity_threshold: Duration,
    ) -> Self;

    pub fn stop(self) -> auki_logs::Log<TimeTransformEntry>;
}
```

`start` spawns a thread that calls `tick` every `period` and appends each entry to the log. `stop` signals the thread, joins, and **returns the log back** so the caller can flush/drop it on the main thread (matters for fsync ordering during shutdown).

Append failures are logged via `eprintln!` and the loop continues — a per-tick I/O hiccup shouldn't kill the sampler.

## Tests (10 total)

| Test | Asserts |
|------|---------|
| `tick_computes_offset_uncertainty_and_timestamp` | The math is right for one canned set of readings |
| `first_tick_never_flags_discontinuous` | No prior offset → `false` regardless of magnitude |
| `drift_smaller_than_threshold_does_not_flag` | Sub-threshold drift between two samples → `false` |
| `step_larger_than_threshold_flags_discontinuous` | Above-threshold step → `true` |
| `step_below_threshold_does_not_flag` | Just-below-threshold step → `false` (boundary check) |
| `negative_step_also_flags` | Backwards UTC correction → `true` (uses `unsigned_abs`) |
| `source_serializes_snake_case` | `LocalClockRead` → `"local_clock_read"` |
| `entry_round_trips_through_cbor` | Full `TimeTransformEntry` survives a CBOR encode/decode cycle |
| `build_manifest_contains_required_fields` | All 6 manifest fields present and typed correctly |
| `sampler_writes_entries_then_stops_cleanly` | Threaded integration test against `ScriptedClock`; entries present, sources correct, first-entry `discontinuous` invariant holds |

## Consumers in this workspace

- `auki-k1-binary` — opens a `TimeTransform` log + spawns a `Sampler` for the lifetime of a session
