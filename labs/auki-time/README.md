# Auki time

`SessionClock`, offset transforms, and clock-pair sampling. The existing `tick`
and `Sampler` APIs remain unchanged. Enable the optional `components` feature
to publish a session-to-UTC relationship as an ordinary Component Product.
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
or bounded long-term accuracy from these samples. Inter-peer bridges,
drift-aware conversion and transform discovery are later work.

## Compatibility and validation

This SDK-only change replaces the one-shot helper source API
(`publish_startup_anchor`, `SessionUtcAnchor`, `read_session_anchor`) with the
measurement/history API. Its new payload and metadata schemas deliberately do
not reinterpret the previous `auki.session-utc-anchor/v1` held-anchor contract.
No backward-compatibility alias or decoder is added.

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
