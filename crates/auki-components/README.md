# Auki Components

Typed, network-independent building blocks for executable Components, live
Observables and Operables, retained Products, explicit buffers, and a
read-only Catalog projection.

This crate owns local component semantics. It deliberately has no dependency
on `auki-sdk`, `AukiPeer`, or a wire protocol. Network transport is layered on
top by `auki-component-protocol`, so local and remote connections preserve the
same contracts without making transport part of a Component's identity.
