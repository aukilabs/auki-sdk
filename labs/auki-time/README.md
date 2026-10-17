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
let outcome = publisher.publish_startup_anchor()?;
let product = publisher.product();
assert_eq!(outcome.sequence, 0);
assert_eq!(product.producer.clock, publisher.contract().from_reference());
// Host: retain/serve both definitions in publisher.contract(), then explicitly
// endpoint.export_product(&product)? on the existing ComponentProtocolEndpoint.
# Ok(())
# }
```

Construct/publish at session startup before capture workloads. The helper clones
the supplied `SessionClock`, preserving its epoch; it does not create a session
or another monotonic clock. The runtime must own the clock. Its device-local UTC
definition has the same peer/session and an explicit Unix epoch. Actual session
or clock resets require new identities, supplied by the application.

Local Catalog registration is not network export. Keep the publisher alive for
the desired lifetime and retain/serve its clock definitions through the existing
registry mechanism. The caller owns explicit endpoint export and shutdown.
No app, camera, capture timestamp, transport or Domain-selection policy changes
are required to compile this opt-in helper.

## Versioned Component profile

- Payload: `auki.session-utc-anchor/v1` (`SessionUtcAnchor`).
- Catalog metadata: `auki.session-utc-contract/v1` (`SessionUtcContract`).
- Endpoints: complete existing `ClockRegistryEntry` definitions. Their exact
  `{peer_id, id, hash}` references are obtained through `from_reference()` and
  `to_reference()`; matching names or epochs never establish identity.
- Output clock: exactly the source session-clock reference. The Output ID binds
  both definitions and the declared UTC accuracy by the SDK canonical hash.
- Observation time: the midpoint of session-clock readings around a *current*
  UTC read. The original Output reference and sequence are SDK provenance.
- Metadata `source_sequence` is zero and identifies that retained observation;
  validate the fetched Product/Output and sequence using `read_session_anchor`.

One startup attempt is retained in an SDK buffer (one entry, 1 KiB payload cap,
no duration eviction). An available payload contains `offset_ns` and
`uncertainty_ns`: `UTC_ns = session_ns + offset_ns`, with checked arithmetic.
The fixed relationship describes the startup UTC time scale, including the
session's zero; it does not follow later wall-clock corrections. A second
publication fails **without reading the clocks**. Imported sequence replacements,
wrong clock definitions, malformed Outputs and inconsistent provenance reject.

UTC read failures publish `Unavailable`, with a typed reason, not a valid zero
offset. Failure is also a final startup outcome; this helper does not retry or
retroactively assign a new anchor to previously captured data. The caller must
handle unavailable temporal conversion explicitly. `publish_startup_anchor_with_utc`
allows a fallible current UTC reader; the helper still owns both session readings.
It does not accept a queued sensor timestamp as a clock reading.

Sampling uses the SDK's three-read `tick` after checked bounds validation.
Unrepresentable UTC values, invalid sample order and a read window exceeding
`u32::MAX` nanoseconds produce unavailable status rather than saturation.
Read-window uncertainty is not UTC accuracy. `utc_accuracy_ns` is unknown (`null`)
for this publisher, and this profile supplies **no oscillator drift bound**.
Consumers must not infer synchronization between distinct devices' UTC clocks,
or bounded long-term accuracy from this startup sample. Inter-peer bridges,
drift-aware conversion and transform discovery are later work.

## Compatibility and validation

This additive SDK profile is distinct from the harness's
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
