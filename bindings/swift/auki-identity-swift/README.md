# auki-identity-swift

UniFFI Swift bindings for [`auki-identity`](../../../crates/auki-identity)'s `Wallet`.

Thin scaffolding host: the actual UniFFI proc-macros live on the upstream types under the `swift-bindings` cargo feature. This crate's job is `uniffi::setup_scaffolding!()` and `pub use` re-exports so UniFFI's metadata scanner sees the upstream types.

Produces a `staticlib` for iOS consumption and a `cdylib` for host `uniffi-bindgen` introspection.

**Status:** Shipped.

## Public surface

- `Wallet` (re-exported from `auki-identity`)
Plus everything UniFFI infers from its annotated methods.

This current binding intentionally exposes no `PeerIdentity` or network
runtime. The removed Manager-era `auki-network-swift` package is available only
at source tag `v0.0.60` and is not compatible with the authenticated Stage 1
runtime.

## Depends on

- [`auki-identity`](../../../crates/auki-identity) — upstream crate the UniFFI annotations live in.
