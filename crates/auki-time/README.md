# auki-time

`auki-time` owns the SDK's time-transform primitives. A TimeTransform Log captures the relationship between two clocks over time, sampled at 1 Hz. `convert_time` will eventually consume these transforms to translate timestamps across clocks.

This crate provides:
- `SessionClock`, the SDK-owned session-monotonic clock primitive with a peer-id anchored `ClockRegistryEntry`.
- Pure `TimeTransform` math for converting timestamps from one named clock to another.
- NTP-style `NtpExchange` / `NtpSample` helpers for estimating the offset between two independent clocks.
- `ClockSyncState` / `ClockSyncHandle` for retaining heartbeat-derived NTP samples and producing best current peer-clock transform estimates.
- `DomainClockDescriptor` / `DomainClockEstimate` helpers for composing peer-clock estimates into `cluster_name/domain-clock` transforms.
- The 1 Hz `local_clock_read` sampler that produces `TimeTransformEntry` records.
- The three-read sampling protocol (`m1, r, m2`) and its `uncertainty_ns` computation.
- A `Clock` trait + `SystemClock` impl wired to `clock_gettime` for `CLOCK_MONOTONIC` / `CLOCK_REALTIME`.
- Crate-owned UniFFI bindings for Python/Swift and wasm-bindgen bindings for the browser-safe time math surface.

`TimeTransformEntry` is generated from the root protobuf schemas into [`auki-proto`](../auki-proto). The log itself is an [`auki-logs`](../auki-logs) `Log<TimeTransformEntry>` opened at `<session>/timetransform_logs/<from_id>__<to_id>/`. One TimeTransform Log per ordered clock pair per session — clock offsets are time-localized, so the session is the natural retention boundary. See [`auki-layout`](../auki-layout) for path helpers and the full session shape.

## SessionClock

`SessionClock` is the shared SDK primitive for one peer's session-monotonic time. It owns a `ClockRegistryEntry`, a content hash for that entry, and the monotonic `now_ns()` / `now_i64_ns()` reader used by `auki-domain::ClusterManager` and future heartbeat time sync.

SDK-minted session clock ids are anchored to the authoring peer id:

```text
<peer_id>/<session_id>/monotonic
```

The first path segment is expected to be the authoring `peer_id`. The older `<platform-tag>-<machine-id>/...` convention is stale for new SDK-minted session clocks. Because monotonic readings only make sense inside one process lifetime, the clock registry entry also records the session id as the monotonic epoch marker and uses `Scope::DeviceLocal`.

Heartbeat time sync must consume `SessionClock` for timestamp identity and readings. It should not introduce a parallel heartbeat-specific clock abstraction.

## Pure transforms

`TimeTransform` is an affine transform from `from_clock_id` to `to_clock_id`:

```
to_timestamp_ns = from_timestamp_ns + offset_ns
```

The offset is stored as `to_clock - from_clock`, with `uncertainty_ns` and the `observed_at_clock_ns` reading from the source clock. This crate does not discipline or rewrite system clocks; it produces values consumers can keep as local transforms.

## NTP-style samples

`compute_ntp_sample(NtpExchange { local_send_ns, remote_receive_ns, remote_send_ns, local_receive_ns })` returns an `NtpSample` whose `offset_ns` is `remote_clock - local_clock`.

```
offset_ns      = ((remote_receive_ns - local_send_ns)
               +  (remote_send_ns - local_receive_ns)) / 2
uncertainty_ns = (local_receive_ns - local_send_ns)
               - (remote_send_ns - remote_receive_ns)
```

The formula is valid for independent monotonic epochs because it estimates the transform between the two clock readings, not the absolute time-of-day of either clock. `select_best_ntp_sample` picks the lowest-uncertainty sample, breaking ties by the newest local observation.

## Generated bindings

`auki-time` follows the SDK binding standard:

- `src/core.rs` owns the binding-free Rust API.
- `src/ffi.rs` exports UniFFI records/objects for Python and Swift.
- `src/wasm.rs` exports wasm-bindgen JSON-string adapters for browser JavaScript.
- `bindings.toml` and `bindings/{python,swift,javascript}/` define the crate-owned package contracts.

Native UniFFI exposes the pure transform/NTP/sync/domain APIs plus `SessionClock` for host clock identity and registry-entry JSON. Browser wasm exposes only web-safe behavior: NTP math, best-sample selection, `ClockSyncState`, domain-clock composition, and timestamp conversion. `SystemClock`, `tick`, `Sampler`, and filesystem-backed log production stay native-only.

## Peer-clock sync state

`ClockSyncState` is the first policy layer above raw heartbeat timing. Callers feed it `ClockSyncObservation` values containing:

- local clock id/hash
- remote clock id/hash
- one `NtpSample`

The state is keyed by ordered local/remote clock pair. It keeps a bounded sample window, rejects samples above `ClockSyncConfig::max_uncertainty_ns`, prunes samples older than `max_sample_age_ns`, clears a pair's window when either clock hash changes, and returns a `ClockTransformEstimate` for `local_clock -> remote_clock`.

`ClockTransformEstimate` carries clock ids/hashes, the selected offset and uncertainty, the local observation timestamp, and the retained sample count. Its `time_transform()` helper returns the corresponding `TimeTransform`. `ClockTransformEstimate::identity(clock_id, clock_hash, observed_at_clock_ns)` builds the exact zero-offset transform used when a peer's local session clock is also the domain-clock backing clock.

`ClockSyncHandle` wraps the state in a cloneable shared handle for runtime/event tasks. Clones share the same sample windows, and callers can read one estimate with `estimate(local_clock_id, remote_clock_id)` or snapshot all current estimates with `estimates()`.

This is intentionally peer-local state. `auki-network` can emit heartbeat-derived samples, but it does not own selection policy. `auki-domain` can provide domain-clock context, but it does not become the NTP service.

## Domain-clock composition

`DomainClockDescriptor` describes the stable cluster domain clock from the domain layer:

- `cluster_name`
- domain clock id/hash
- backing peer id
- backing clock id/hash
- `backing_to_domain_offset_ns`

`estimate_domain_clock(local_to_backing, descriptor)` validates that `local_to_backing.to_clock_*` matches the descriptor's backing clock, then composes:

```
local_clock -> domain_clock =
    local_clock -> backing_clock
  + backing_clock -> domain_clock
```

The returned `DomainClockEstimate` carries both component offsets and the composed `total_offset_ns`; uncertainty is inherited from the peer-clock estimate. This is pure composition only. It does not decide which peer is backing the domain clock, nor when a cluster should change that backing source.

## Where the types live

The type definitions live in their canonical homes and are re-exported here for short call sites:

- **`TimeTransformEntry`** lives in [`auki-proto`](../auki-proto) under the `auki.time_transform` `.proto` package — protobuf via prost, two fields only (`int64 offset_ns`, `uint32 uncertainty_ns`). The pre-migration per-entry `source` field moved to the manifest; the per-entry `discontinuous: bool` is gone (computed on read, see below).
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
