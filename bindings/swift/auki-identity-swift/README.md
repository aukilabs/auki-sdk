# auki-identity-swift

Thin UniFFI binding for [`auki-identity`](../../../crates/auki-identity)'s
`Wallet` type.

The annotated implementation remains in `auki-identity` behind its
`swift-bindings` feature. This crate only re-exports that surface and installs
the UniFFI scaffolding needed to build an iOS `staticlib` or inspect metadata
through a host `cdylib`.

This is a `Wallet` binding, not the Swift/iOS networking SDK. It does not yet
expose the canonical authenticated `AukiPeer` facade, relay lifecycle, or
portable protocol clients and endpoints.
