# auki-identity-swift

UniFFI Swift bindings for [`auki-identity`](../../../crates/auki-identity)'s `Wallet` and the identity-shaped pieces of [`auki-network`](../../../crates/auki-network)'s `PeerIdentity`.

Thin scaffolding host: the actual UniFFI proc-macros live on the upstream types under the `swift-bindings` cargo feature. This crate's job is `uniffi::setup_scaffolding!()` and `pub use` re-exports so UniFFI's metadata scanner sees the upstream types.

Produces a `staticlib` for iOS consumption and a `cdylib` for host `uniffi-bindgen` introspection.

**Status:** Shipped.

## Public surface

- `Wallet` (re-exported from `auki-identity`)
- `PeerIdentity` (re-exported from `auki-network`)

Plus everything UniFFI infers from their annotated methods.

## Depends on

- [`auki-identity`](../../../crates/auki-identity) — upstream crate the UniFFI annotations live in.
- [`auki-network`](../../../crates/auki-network) — for `PeerIdentity`.
