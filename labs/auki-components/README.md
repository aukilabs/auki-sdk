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

The field is additive in Catalog v1 JSON: older entries decode as `None`, and
older readers ignore the new field. Rust `CatalogProductEntry` literals must
provide it. Metadata is publisher-supplied discovery data, not an authorization
grant or a guarantee that its observation remains retained. Consumers compare its
source sequence with a fetched observation when they require an exact match.

Browser buffer appends use `web_time::Instant` for arrival timestamps; native
builds retain `std::time::Instant`. WASM callers of `Buffer::append_shared_at`
must likewise use `web_time::Instant`. This fixes a runtime panic when publishing
Map Component snapshots in browsers, with no native API or wire-format change.
The collaborative mapping example runs its snapshot/edit regression suite in
Chromium through WASM. Blocking cursor reads and thread-backed workers remain
native facilities; browser consumers use `next_async` and inline captures.
