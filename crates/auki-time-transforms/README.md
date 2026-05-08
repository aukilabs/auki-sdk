# auki-time-transforms

A TimeTransform Log captures the relationship between two clocks over time, sampled at 1 Hz. It's the primitive that `convert_time` will eventually consume to translate timestamps across clocks.

This crate provides:
- The 1 Hz `local_clock_read` sampler that produces `TimeTransformEntry` records.
- The three-read sampling protocol (`m1, r, m2`) and its `uncertainty_ns` computation.
- A `Clock` trait + `SystemClock` impl wired to `clock_gettime` for `CLOCK_MONOTONIC` / `CLOCK_REALTIME`.

The log itself is an [`auki-logs`](../auki-logs) `Log<TimeTransformEntry>` opened at `<session>/timetransform_logs/<from_id>__<to_id>/`. One TimeTransform Log per ordered clock pair per session — clock offsets are time-localized, so the session is the natural retention boundary. See [`auki-layout`](../auki-layout) for path helpers and the full session shape.

## Where the types live

Step 6 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md) (2026-05-08) moved the type definitions out of this crate; they live in their canonical homes now and are re-exported here for short call sites:

- **`TimeTransformEntry`** lives in [`auki-datatypes`](../auki-datatypes) under the `auki.time_transform` `.proto` package — protobuf via prost, two fields only (`int64 offset_ns`, `uint32 uncertainty_ns`). The pre-migration per-entry `source` field moved to the manifest; the per-entry `discontinuous: bool` is gone (computed on read, see below).
- **`TimeTransformSource`** lives in [`auki-manifests`](../auki-manifests) — manifest metadata, mirrors `PoseSource`'s tagged-enum shape. One variant today (`LocalClockRead`); future producers (`NtpSynced { server }`, `SyncedTo { peer_id }`, ...) attach metadata without a schema break.
- The `tick` and `Sampler` primitives in this crate are the producer of `TimeTransformEntry`. They drive `clock_gettime` and write entries to an `auki-logs::Log<TimeTransformEntry>`.

## Manifest

The manifest schema and `build_time_transform_log_manifest(...)` builder live in [`auki-manifests`](../auki-manifests). Step 6 added `&TimeTransformSource` as a builder argument; the manifest gains a `"source"` field carrying the producer identity inline (mirrors how Pose Log carries `PoseSource`). Encoding stays JCS-canonical UTF-8 JSON. See [`auki-manifests/README.md#timetransform-log`](../auki-manifests/README.md#timetransform-log) for the field table.

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

The third read costs one extra `clock_gettime` syscall (~200 ns on a Booster K1) and lets us bound `r`'s position within `[m1, m2]` rather than guess. The framing's `timestamp_ns` (added by auki-logs on `append`) is the from-clock midpoint — one source of truth, not duplicated in the payload.

## Discontinuity detection — reader-side

UTC can step (NTP correction, manual `date -s`). When it does, a smooth `convert_time` interpolation across the step would produce wrong answers. **Step 6 (2026-05-08) moved discontinuity detection to readers**; the on-disk entry no longer carries a `discontinuous: bool` flag.

The recipe is unchanged, just not baked into the bytes:

```
is_discontinuous(this_offset_ns, prev_offset_ns, threshold_ns) =
    abs(this_offset_ns - prev_offset_ns) >= threshold_ns
```

**Recommended threshold: 10 ms.** Justification:
- A 1 Hz sample of a healthy clock pair drifts by ~10–100 µs/s (10–100 ppm hardware oscillator drift). 10 ms is two-to-three orders of magnitude above expected drift jitter.
- `chronyd` and `systemd-timesyncd` slew sub-second corrections gradually; only adjustments larger than ~1 s become hard steps. 10 ms cleanly separates step-events from slew.

Two readers of the same log can disagree on what counts as discontinuous now — that's intentional. A debug viewer might want a tight threshold to highlight every NTP slew tick; a `convert_time` interpolator might want a looser one to avoid spuriously refusing interpolation across normal drift.

## Forward compatibility

Future sources may not be able to detect discontinuities at all (`heartbeat_exchange` measurements include round-trip noise that can dwarf real steps). The reader-computed model handles this gracefully — a reader can pick its threshold per-source, or skip discontinuity detection entirely if the source's metadata says it's untrustworthy. A new variant on `TimeTransformSource` (e.g. `HeartbeatExchange { rtt_jitter_ns }`) can attach the metadata readers need.

## Versioning

Schema version is **1**. Bump on incompatible field changes. The auki-logs segment format version is independent and currently also 1.
