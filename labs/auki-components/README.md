# Auki Components

Typed, network-independent building blocks for executable Components, live
Observables and Operables, retained Products, explicit buffers, and a
read-only Catalog projection.

This crate owns local component semantics. It deliberately has no dependency
on `auki-sdk`, `AukiPeer`, or a wire protocol. Network transport is layered on
top by `auki-component-protocol`, so local and remote connections preserve the
same contracts without making transport part of a Component's identity.

A Buffer Product retains its exact producer's terminal notice in shared runtime
state. `RetainedProduct::end_notice()` exposes reconfiguration or failure without
changing the Product manifest or hash. Retained observations remain fetchable
after closure; readers drain them before ending. Imports use `imported_buffer`
and `close_with_notice`, which rejects mismatched or conflicting source notices.
Construct Products through the runtime's capture APIs or this import API, not
public struct literals.

See [networked Components](../../docs/reference/component-protocols.md) for the
optional authenticated protocol adapter and its fetch/subscription limits.

`BufferCursor::next_async()` is a cancellation-safe, wake-driven read of the
same retained history as `next_timeout`. It reports items, gaps, and closure
without a timer or worker thread. Dropping a pending read unregisters its waker;
it does not advance the cursor or create a second history queue.

Buffer captures can attach application discovery data through
`capture_buffer_with_metadata`. Each optional `CatalogProductEntry.metadata`
contains a schema, original observation sequence and JSON value (up to 256 KiB
including its envelope). Projection and validation happen before retention;
errors reject that capture and appear in `errors()`. After retention, Product
state and metadata are updated together in one Catalog revision, even if entry
count did not change. Metadata never changes the Product Manifest hash.

The field remains optional in Catalog JSON: entries without it decode as `None`. Rust `CatalogProductEntry` literals must
provide it. Metadata is publisher-supplied discovery data, not an authorization
grant or a guarantee that its observation remains retained. Consumers compare its
source sequence with a fetched observation when they require an exact match.

Clock references are mandatory in Output Manifest v2. `ClockReference` reuses
`auki_registry::RegistryRef`: `{ peer_id, id, hash }` identifies the exact clock
registry entry, whose definition includes session/boot, clock type, native units,
epoch and scope. The host must retain and make that definition available; a
reference alone is not clock synchronization. All `*_ns` fields use nanoseconds
relative to that clock's epoch (convert native ticks before publishing).

An Observation and its terminal notice obtain their clock through their exact
Output Manifest reference. Time-range requests carry `clock` explicitly and
compare all three reference fields. Missing/malformed references are rejected at
configuration, catalog and Product import boundaries. Cross-clock comparison
requires an explicit time transform; matching names do not establish compatibility.
Forwarding a Product preserves its source clock, including its owner and hash.

Migration: pass a registered `ClockReference` to `ConfiguredObservableSpec::new`
and `CameraComponent::new`, replacing string clock names. Camera resolution
instructions/results also carry `clock`. There is no implicit conversion from a
legacy clock ID. Coordinate frames remain explicit in pose/map contracts.

`Buffer::bracket_time_ns` atomically returns shared leases on the nearest retained
samples before/after a timestamp. Exact matches occupy both sides; missing bounds
stay `None`. It rejects unordered and duplicate-permitting timestamp policies.
The enclosing Product supplies the clock. For explicitly framed pose publication
and checked interpolation over these standard Buffer Products, see
[auki-odometry](../auki-odometry/README.md).
