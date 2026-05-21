# `auki-identity-swift/src/`

Implementation status for [`auki-identity-swift`](../README.md). Honest about what is real today.

## Files

- [`lib.rs`](lib.rs) — `uniffi::setup_scaffolding!()` + `pub use` re-exports of `auki_identity_rs::Wallet` and `auki_network_rs::PeerIdentity`. A smoke test asserts the re-exports compose correctly.
- [`bin/uniffi-bindgen.rs`](bin/uniffi-bindgen.rs) — host Swift-codegen entry point, gated behind the `cli` feature.

## What works today

- **Host build + tests green.** `cargo build -p auki-identity-swift` and `cargo test -p auki-identity-swift` succeed.
- **Wallet surface**: `from_seed` (constructor, `Vec<u8>` + length check → `Result<Arc<Self>, IdentityError>`), `new` (constructor, CSPRNG → `Arc<Self>`), `seed() -> Vec<u8>`, `wallet_id_str() -> String`. UniFFI proc-macros expand the upstream `auki-identity::Wallet` type into a UniFFI `Object` when the upstream `swift-bindings` feature is on.
- **PeerIdentity surface**: `from_wallet(Arc<Wallet>) -> Self` (constructor — `Arc<Wallet>` not `&Wallet` because UniFFI 0.31 doesn't impl `LiftRef` for foreign-crate Objects), `peer_id_string()` — returns the canonical libp2p peer-id string.
- **iOS XCFramework build scripted** in `build-xcframework.sh`; not yet validated end-to-end on this crate (it follows the same shape as `auki-network-swift`'s script which was validated by Stage 1 PR #152). Run `bindings/swift/auki-identity-swift/build-xcframework.sh` once to confirm.

## What does NOT work yet

- `derive_child`, `sign`/`sign_canonical_json`, `public_key()`, `id()` (typed `WalletId`) — see `parking_lot.md`.
- Stream surfaces, NetworkRuntime, ClusterManager — those are PRs B and C of Spec 1.

## Rust mapping

| Swift | Rust |
|---|---|
| `Wallet` | `auki_identity::Wallet` |
| `PeerIdentity` | `auki_network::PeerIdentity` |

## Verification

```bash
cargo test -p auki-identity-swift                                          # host gate
cargo build -p auki-identity-swift                                          # host gate
cargo build --features swift-bindings -p auki-identity                      # upstream-feature gate
cargo build --features swift-bindings -p auki-network                       # upstream-feature gate
bindings/swift/auki-identity-swift/build-xcframework.sh                     # iOS XCFramework
```
