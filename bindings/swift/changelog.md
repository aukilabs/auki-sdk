# Changelog — Swift bindings

One-line summaries of changes in Swift binding packages. Detailed entries live in each package's `changelog.md`.

Latest entry on top.

---

### Nils's claude · May 21, 15:41 HKT, 2026

**New crate `auki-identity-swift` (PR A of Spec 1).** Thin UniFFI scaffolding host for `auki-identity::Wallet` and the identity-shaped slice of `auki-network::PeerIdentity`. UniFFI proc-macros live on the upstream types behind a new `swift-bindings` cargo feature on each of `crates/auki-identity` and `crates/auki-network`. Surface: `Wallet::{new, from_seed, seed, wallet_id_str}`, `PeerIdentity::{from_wallet, peer_id_string}`. UniFFI 0.31 type-system constraints cascaded to `Wallet::from_seed` (now `Vec<u8>` → `Result<Arc<Self>, IdentityError>`) and `PeerIdentity::from_wallet` (now `Arc<Wallet>` parameter); workspace adapted at all internal callers. Host gate green; iOS XCFramework build scripted.

### Nils's claude · May 20, 13:31 HKT, 2026

**`auki-network-swift` relocated from `crates/` to `bindings/swift/`.** Brings the UniFFI Swift binding under the `bindings/<language>/` convention established by PR #156 for the Python packages. Package name, lib name, surface, and runtime behavior unchanged; only paths and relative doc links moved.
