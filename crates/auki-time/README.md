# auki-time

Transport-neutral time primitives for the Auki SDK: `SessionClock`, fixed affine `TimeTransform` math, and the 1 Hz `local_clock_read` sampler that writes a TimeTransform Log.

This crate deliberately does not provide peer heartbeat, NTP synchronization, or a shared Domain clock. Products keep timestamping data with local/session clocks and can record explicit clock transforms without implying Domain-wide clock consensus.

**Status:** Shipped. The `convert_time` operation that consumes the TimeTransform Log is not yet implemented.

## Public surface

- `SessionClock` — monotonic per-session clock used as the default sample timestamp source.
- `TimeTransform` — a fixed affine offset with uncertainty and observation metadata.
- `Clock` (trait), `SystemClock`, `Sampler`, `tick(...)` — the 1 Hz sampler that writes TimeTransform Log entries.
- Re-exports: `TimeTransformEntry` (from `auki-datatypes`), `TimeTransformSource` (from `auki-manifests`).

## Depends on

- [`auki-registry`](../auki-registry) — for `ClockRegistryEntry`.
- [`auki-logs`](../auki-logs) — to write the TimeTransform Log.
- [`auki-datatypes`](../auki-datatypes) — for `TimeTransformEntry` payloads.
- [`auki-manifests`](../auki-manifests) — for `build_time_transform_log_manifest` and `TimeTransformSource`.
