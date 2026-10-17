# Auki time

`SessionClock`, offset transforms, and clock-pair sampling. The existing `tick`
and `Sampler` APIs remain unchanged. Enable the optional `components` feature
to publish a session-to-UTC relationship as an ordinary Component Product and
resolve explicitly declared temporal paths in memory.
The feature adds no network dependency or background worker.

## Publish the existing session clock

```rust
# #[cfg(feature = "components")]
# fn example() -> Result<(), Box<dyn std::error::Error>> {
use auki_components::ComponentRuntime;
use auki_time::{SessionClock, components::SessionClockPublisher};

let runtime = ComponentRuntime::new("peer-a");
// The application owns one unique execution session. Reuse its clock here.
let clock = SessionClock::new("peer-a", "unique-execution-session", "capture");
let mut publisher = SessionClockPublisher::new(&runtime, &clock)?;
let first = publisher.publish_measurement()?;
let product = publisher.product();
assert_eq!(first.sequence, 0);
assert_eq!(product.producer.clock, publisher.contract().from_reference());
// Host: retain/serve both definitions in publisher.contract(), then explicitly
// endpoint.export_product(&product)? on the existing ComponentProtocolEndpoint.
// Later, at the host's chosen cadence, call publisher.publish_measurement()
// again on this same publisher. The clock and Product identities do not change.
# Ok(())
# }
```

Normally construct/publish at session startup before capture workloads, then
occasionally call `publish_measurement()` again. The helper clones
the supplied `SessionClock`, preserving its epoch; it does not create a session
or another monotonic clock. The runtime must own the clock. Its device-local UTC
definition has the same peer/session and an explicit Unix epoch. Actual session
or clock resets require new identities, supplied by the application.

Local Catalog registration is not network export. Keep the publisher alive for
the desired lifetime and retain/serve its clock definitions through the existing
registry mechanism. The caller owns explicit endpoint export and shutdown.
No app, camera, capture timestamp, transport or Domain-selection policy changes
are required to compile this opt-in helper.

## Versioned retained-history Component profile

- Payload: `auki.session-utc-sample/v1` (`SessionUtcSample`).
- Catalog metadata: `auki.session-utc-history/v1` (`SessionUtcContract`).
- Endpoints: complete existing `ClockRegistryEntry` definitions. Their exact
  `{peer_id, id, hash}` references are obtained through `from_reference()` and
  `to_reference()`; matching names or epochs never establish identity.
- Output clock: exactly the source session-clock reference. The Output ID binds
  both definitions and the declared UTC accuracy by the SDK canonical hash,
  with a history-profile-specific prefix.
- Observation time: the midpoint of session-clock readings around a *current*
  UTC read. The original Output reference and sequence are SDK provenance.
- Metadata `source_sequence` identifies the latest successfully retained
  measurement, including unavailable outcomes. The endpoint definitions and
  contract value remain unchanged as this sequence and the buffer state advance.

One Component and Output publish into one SDK retained Buffer Product. The
Catalog advertises the Product and its contract; the Product holds the history,
not the Catalog itself. Sequence numbers start at zero and advance for each
successfully retained measurement. Earlier observations, including their source
timestamps, payloads and provenance, remain unchanged.

An available payload contains `offset_ns` and
`uncertainty_ns`: `UTC_ns = session_ns + offset_ns`, with checked arithmetic.
This measures the relationship **at that observation's timestamp**, not a held
relationship for the entire session. Forward or backward wall-clock corrections
appear in later observations without resetting the monotonic clock or replacing
earlier measurements. This profile does not prescribe validity between samples,
interpolation, extrapolation, or smoothing across UTC discontinuities. Later
evidence may inform a consumer's explicit conversion policy; it must never
silently relabel source timestamps or apply the latest offset to all history.

UTC read failures publish `Unavailable`, with a typed reason, not a valid zero
offset. A later successful measurement can recover while the unavailable sample
remains in history. The caller must handle unavailable temporal conversion
explicitly. `publish_measurement_with_utc`
allows a fallible current UTC reader; the helper still owns both session readings.
It does not accept a queued sensor timestamp as a clock reading.

### Retention and reading

`new()` retains up to 4,096 fixed-size samples; `with_capacity()` accepts another
positive entry limit. No duration or byte eviction is configured. When full,
`HistoryFull` is returned **before reading UTC or publishing**. Existing evidence
is not evicted to make room. Choose capacity for the intended session and cadence;
automatic archival, resizing, and disk persistence are outside this helper.

The returned Product is for reading/export. Do not append to, close, or change the
limits of its underlying local buffer. Detected retention changes or publication/
capture failures stop further publication; previously retained samples remain
readable. This prevents a failed inline capture from causing invisible sequence
gaps on retry. A non-increasing measurement timestamp rejects before publication
without inventing a replacement time; a later measurement can retry normally.

`read_session_sample(&product, &contract, sequence)` validates an **exact**
retained observation in a local or imported Product: both clock definitions,
Output and Product provenance, envelope sequence/time, and payload arithmetic.
Missing samples return `MissingSample`, never an implicit latest fallback.
Imported partial histories are allowed; missing ranges remain missing.

Remote consumers use the existing Component protocol's clock-qualified
`TimeRange` access to fetch retained history, then select/validate the desired
sequence locally. `LatestExisting` is also available, but may have advanced past
a Catalog snapshot's `source_sequence`; it is not an exact-sequence fetch.
This helper adds no transport or new remote retrieval operation.

Sampling uses the SDK's three-read `tick` after checked bounds validation.
Unrepresentable UTC values, invalid sample order and a read window exceeding
`u32::MAX` nanoseconds produce unavailable status rather than saturation.
Read-window uncertainty is not UTC accuracy. `utc_accuracy_ns` is unknown (`null`)
for this publisher, and this profile supplies **no oscillator drift bound**.
Consumers must not infer synchronization between distinct devices' UTC clocks,
or bounded long-term accuracy from these samples. The resolver below can consume
explicit bridges and drift declarations; measuring those bridges or estimating
drift remains the publisher's responsibility.

## In-memory temporal resolver

`resolver::TemporalResolver` is an opt-in, network-independent index of exact
clock definitions and admitted observations from SDK retained Products. It does
not choose a Domain clock, start synchronization, fetch a Catalog, or add another
transport or persistent registry. Applications supply definitions and Products
through the existing SDK and explicitly authorize each evidence admission.

1. `register_clock(reference, definition)` verifies the full registry identity
   and nanosecond clock semantics. Equal names, owners, epochs, units or scopes
   do not connect clocks. Resets require a new definition/reference.
2. `insert_session_sample(product, contract, sequence, authorize)` reads the
   existing history profile **at one measured instant only**. It does not turn a
   startup sample into an offset valid indefinitely or interpolate later samples.
3. `insert_offset_sample(product, contract, sequence, authorize)` accepts an
   explicitly bounded offset relationship using the profile below. The reporter
   may be any peer, including one that owns neither clock; the caller decides
   whether to admit its claim. Valid hashes are not proof of authorization or
   of the measurement's physical accuracy.
4. `resolve(query)` returns a converted timestamp, conservative uncertainty,
   source-clock coverage, and exact immutable Product/Output/sequence evidence
   with each traversal direction. It never changes source timestamps.

```rust
# #[cfg(feature = "components")]
# fn convert(
#     resolver: &auki_time::resolver::TemporalResolver,
#     steve_clock: auki_components::ClockReference,
#     bob_clock: auki_components::ClockReference,
# ) -> Result<(), auki_time::resolver::TemporalError> {
use auki_time::resolver::TemporalQuery;

let converted = resolver.resolve(&TemporalQuery {
    from_clock: steve_clock,
    to_clock: bob_clock,
    timestamp_ns: 2_000_000_000,
    input_uncertainty_ns: 0,
    max_uncertainty_ns: 5_000_000,
})?;
// Use converted.timestamp_ns only in converted.to_clock. Its uncertainty and
// path describe the evidence used, not a promise of perfect synchronization.
# let _ = converted;
# Ok(())
# }
```

### Explicit bounded-offset Product profile

- Payload: `auki.clock-offset/v1` (`OffsetSample`), an additive model only.
- Catalog contract metadata: `auki.clock-offset-contract/v1` (`OffsetContract`).
  Publishers attach that descriptor when registering/exporting their ordinary
  Component Product. This module does not publish or export on their behalf.
- Contract: exact `from_clock`, exact `to_clock`, and `allow_inverse`. The Output
  clock is the source clock; its ID binds the contract hash. Registry definitions
  must already be verified in the index.
- Observation timestamp: measurement time in the declared source clock.
- Available payload: signed `offset_ns`, optional `uncertainty_ns`, optional
  `max_drift_ppb`, and an inclusive source-clock `validity` interval containing
  the measurement time. `to = from + offset_ns`. Unknown values are not zero.
- Unavailable payload: an explicit interval without an offset. It is not a zero
  transform or a command to revoke unrelated evidence. Publishers must delimit
  valid intervals around corrections or invalidations rather than expect a
  newer sample to silently override old claims. Incompatible overlapping claims
  produce an explicit conflict.

The resolver conservatively computes an edge error over its **whole declared
interval**: measurement uncertainty plus
`ceil(max_drift_ppb * maximum_distance_from_measurement_ns / 1e9)`.
This is deliberately not a fitted drift model or per-query interpolation.
Unknown drift cannot supply a finite bound over a nonzero interval, even when
the query is near the measurement. A point-only sample needs no drift bound.
The local session-to-local-UTC error is not enlarged by unknown absolute UTC
accuracy; connecting two different peers' UTC clocks still requires evidence.

Path uncertainty is added conservatively, including the input bound. The whole
input interval must fit every forward relationship's coverage. Inversion is
allowed only when declared, and its **resulting** source interval, including
the edge's error, must fit the original source coverage. Consequently a
point-only measurement with nonzero error is generally not safely invertible
and cannot by itself compose an arbitrary capture-time path. Missing coverage
is unresolved, not permission to extrapolate or use the latest sample.

### Conflicts, history and resource bounds

Resolution enumerates bounded simple paths without revisiting a clock. It checks
all applicable finite candidate intervals before applying the caller's uncertainty
tolerance. If their common intersection is empty, it returns `Conflict` with the
candidate evidence. Otherwise it picks the lowest-error path, then the fewest
hops and a deterministic evidence-identity tie break; it does not average or fuse
claims into artificially improved accuracy. Inconsistent cyclic walks are not
solved as a global clock-adjustment problem.

Missing exact definitions, missing sequences, denied admission, invalid provenance
and resource limits are distinct errors. `Unresolved.issues` lists the problems
encountered while exploring evidence (coverage, availability, uncertainty or
arithmetic); it is not a causal diagnosis of the remote peer. A disconnected graph
can have no issue beyond the absence of a path. An identity query still requires
the exact clock definition and the caller's uncertainty tolerance.

Defaults bound the index to 256 clocks and 4,096 distinct source observations,
and search to 16 hops, 16,384 evidence scans and 128 candidate paths. Limits are
caller-configurable; maximum recursion depth is 64. Identity strings are bounded
to 1,024 bytes and session IDs to 256. Full storage rejects inserts without
eviction; truncated search returns `LimitExceeded`, never a partial answer that
could hide a conflict. Dispose/rebuild the index explicitly when appropriate.

Each admitted observation is copied as immutable evidence, not retained as a
live mutable buffer alias. Re-importing one Output/sequence through another
Product does not create independent evidence or spend another entry; the first
admitted Product remains its provenance. Contradictory copies are rejected.
Appending later UTC corrections, or eviction from the original SDK Product,
cannot rewrite already returned results. New contradictory evidence may change
future queries into conflicts, rather than silently replacing history.

### Steve and Bob acceptance fixture

The deterministic test uses independent sessions and complete registry references:

- Steve session → Steve local UTC: offset +100 s, error 1 ms.
- Steve local UTC → Bob local UTC: measured offset +20 s, error 2 ms, reported by
  a third peer admitted by the test's explicit policy.
- Bob session → Bob local UTC: offset +90 s, error 1 ms, inversion permitted.

All three test declarations have explicit sufficient coverage and a synthetic
zero-drift bound. Steve's `2 s` resolves to Bob's `32 s ±4 ms`; the reverse path
also succeeds. Without the middle bridge they remain disconnected despite both
UTC definitions naming the same epoch. An alternative fixture connects two
peers to one genuinely identical registered clock. No Home role, robot type,
Domain selection, real clock synchronization or app migration is involved.

See `tests/temporal_resolution.rs` for uncertainty/coverage boundaries, immutable
history, conflicting paths, malformed evidence and bounded-search regressions.

## Compatibility and validation

This SDK-only change replaces the one-shot helper source API
(`publish_startup_anchor`, `SessionUtcAnchor`, `read_session_anchor`) with the
measurement/history API. Its new payload and metadata schemas deliberately do
not reinterpret the previous `auki.session-utc-anchor/v1` held-anchor contract.
No backward-compatibility alias or decoder is added.

The temporal resolver is additive and leaves that history profile unchanged.
Its interval-offset profile is separate: it cannot silently reinterpret a
point-only measurement as held evidence. Affine-rate transforms, Catalog/network
discovery, spatial resolution, application adapters and public language bindings
are outside this slice. No application or deployed wire contract is migrated.

The history profile is also distinct from the harness's
`spatial-harness.time-transform/v1` and its contract metadata. It is also distinct
from the existing TimeTransform Log protobuf. No format is silently aliased or
reinterpreted, and the harness's native-capture uncertainty budget is not copied
into this generic clock relationship. Applications retain their existing profile
until their publishers and consumers explicitly migrate together. This change
does not alter any protocol ID or add a compatibility decoder.

Run from the SDK root:

```sh
cargo test --locked -p auki-time --features components
cargo test --locked -p auki-time --no-default-features
```

The new fixtures are local, with scripted clock values; they need no robot,
credentials, Home server, external backend or wall-clock sleeps. The existing
sampler lifecycle test separately exercises its unchanged worker thread.

The baseline `auki-time` crate uses native `libc::clock_gettime` unconditionally
and does not compile for `wasm32-unknown-unknown`, with or without this feature.
Browser clock portability and public language bindings are outside this slice.
