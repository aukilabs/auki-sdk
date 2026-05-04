# auki-time-transforms

A TimeTransform Log captures the relationship between two clocks over time, sampled at 1 Hz. It's the primitive that `convert_time` will eventually consume to translate timestamps across clocks.

This crate provides:
- The `TimeTransformEntry` payload schema, pinned below as **wire format v1**.
- The 1 Hz `local_clock_read` sampler that produces those entries.
- The default discontinuity-detection threshold and its rationale.

The log itself is an [`auki-logs`](../auki-logs) `Log<TimeTransformEntry>` opened at `<session>/timetransform_logs/<from_id>__<to_id>/`. There is one TimeTransform Log per ordered clock pair per session — clock offsets are time-localized, so the session is the natural retention boundary. See [`auki-session`](../auki-session) for path helpers and the full session shape.

## Manifest

JCS-canonical UTF-8 JSON, written via the auki-logs primitive. Required keys:

| Key                    | Type    | Notes                                                                |
| ---------------------- | ------- | -------------------------------------------------------------------- |
| `segment_duration_ns`  | integer | > 0; from auki-logs                                                  |
| `retention_ns`         | integer | ≥ 0; from auki-logs (0 = unbounded)                                  |
| `app_id`               | string  | Identifier of the application that wrote this log. Same string as the daemon's `/api/info` `app` field (e.g. `boosterapp`, `sentinel`). |
| `from_clock_id`        | string  | The Clock Registry ID that the framing's `timestamp_ns` is in        |
| `from_clock_hash`      | string  | XXH3-128 hex of the from-clock's registry entry                      |
| `to_clock_id`          | string  | The Clock Registry ID that `offset_ns` carries you to                |
| `to_clock_hash`        | string  | XXH3-128 hex of the to-clock's registry entry                        |

A node maintains one TimeTransform Log per ordered clock pair per session.

## Entry payload (CBOR)

```
TimeTransformEntry {
  offset_ns:        i64    // to_clock - from_clock at this instant
  uncertainty_ns:   u32    // window during which to_clock was read, in from_clock units
  source:           string // "local_clock_read"; future: "heartbeat_exchange", "gps", ...
  discontinuous:    bool   // see below
}
```

There is **no `timestamp` field in the payload**. The framing's `timestamp_ns` (added by auki-logs on `append`) is the from-clock reading at the sample instant. One source of truth.

## Sampling protocol — `local_clock_read`

The sampler reads the from-clock, then the to-clock, then the from-clock again:

```
m1 = read from_clock
r  = read to_clock
m2 = read from_clock

timestamp_ns      = (m1 + m2) / 2
offset_ns         = r - (m1 + m2) / 2
uncertainty_ns    = m2 - m1
```

The third read costs one extra `clock_gettime` syscall (~200 ns on a Booster K1) and lets us bound `r`'s position within `[m1, m2]` rather than guess. This deviates from the sprint's two-read pseudocode but matches the design doc's intent of "the time between the two `clock_gettime` calls" being a real bound.

## Discontinuity flag

UTC can step (NTP correction, manual `date -s`). When it does, a smooth `convert_time` interpolation across the step would produce wrong answers. Marking the post-step sample lets consumers refuse interpolation across it.

The sampler holds the previous offset in memory. On each tick:

```
discontinuous = match previous_offset {
    Some(prev) => abs(offset_ns - prev) >= threshold_ns
    None       => false        // first sample has nothing to compare against
}
```

**Default threshold: 10 ms.** Justification:
- A 1 Hz sample of a healthy clock pair drifts by ~10–100 µs/s (10–100 ppm hardware oscillator drift). 10 ms is two-to-three orders of magnitude above expected drift jitter.
- `chronyd` and `systemd-timesyncd` slew sub-second corrections gradually; only adjustments larger than ~1 s become hard steps. A 10 ms threshold cleanly separates step-events from slew.
- Configurable per `Sampler::start` call.

The threshold lives in the sampler, not the manifest. Two readers of the same log will agree on `discontinuous` because the field is recorded — they don't reapply the threshold.

## Forward compatibility

Future sources may not be able to detect discontinuities locally (`heartbeat_exchange` measurements include round-trip noise that can dwarf real steps). When that lands, the field can widen to `Option<bool>` — CBOR is tolerant of unknown/missing fields under serde defaults, so older readers ignore the change and newer readers distinguish "false" (known continuous) from "null" (this source doesn't tell us).

## Versioning

Schema version is **1**. Bump on incompatible field changes. The auki-logs segment format version is independent and currently also 1.
