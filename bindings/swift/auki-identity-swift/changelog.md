# Changelog — auki-identity-swift

Append-only changelog for this crate. See [CLAUDE.md](../../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's claude · May 21, 15:41 HKT, 2026

**New crate: auki-identity-swift (PR A of Spec 1).** Thin scaffolding host for the Swift binding to `auki-identity::Wallet` and the identity-shaped slice of `auki-network::PeerIdentity`. UniFFI 0.31 proc-macros live on the upstream types behind a new `swift-bindings` cargo feature; this crate is `uniffi::setup_scaffolding!()` + `pub use` re-exports + per-component docs + `build-xcframework.sh`. Surface at PR A: `Wallet::{new, from_seed, seed, wallet_id_str}` (constructors return `Arc<Self>` per UniFFI 0.31's Object contract; `from_seed` takes `Vec<u8>` with a length check returning `IdentityError`) and `PeerIdentity::{from_wallet, peer_id_string}` (`from_wallet` takes `Arc<Wallet>` because UniFFI 0.31 doesn't impl `LiftRef` for foreign-crate Objects). Stream surface lands in PR B (`auki-network-swift` expansion); ClusterManager in PR C (`auki-domain-swift`).
