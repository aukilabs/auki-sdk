# auki-time

Time primitives for the Auki SDK: `SessionClock`, pure `TimeTransform` math, NTP-style offset exchange, and the 1 Hz `local_clock_read` sampler that writes a TimeTransform Log. Combined with peer NTP exchanges, a cluster converges on a shared **domain time** without picking a canonical clock.

**Status:** Shipped. The `convert_time` operation that consumes the TimeTransform Log is not yet implemented.

## Public surface

- `SessionClock` — monotonic per-session clock used as the default sample timestamp source.
- `TimeTransform` — pure math over offset samples; interpolation and composition.
- `NtpExchange`, `NtpSample`, `compute_ntp_sample`, `select_best_ntp_sample` — NTP-style four-timestamp exchange + best-sample selection.
- `Clock` (trait), `SystemClock`, `Sampler`, `tick(...)` — the 1 Hz sampler that writes TimeTransform Log entries.
- Re-exports: `TimeTransformEntry` (from `auki-datatypes`), `TimeTransformSource` (from `auki-manifests`).

## Depends on

- [`auki-registry`](../auki-registry) — for `ClockRegistryEntry`.
- [`auki-logs`](../auki-logs) — to write the TimeTransform Log.
- [`auki-datatypes`](../auki-datatypes) — for `TimeTransformEntry` payloads.
- [`auki-manifests`](../auki-manifests) — for `build_time_transform_log_manifest` and `TimeTransformSource`.
