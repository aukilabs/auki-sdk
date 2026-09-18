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
optional authenticated protocol adapter and its retained-Product polling limits.
