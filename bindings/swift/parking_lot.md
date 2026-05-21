# Parking lot — Swift bindings

Cross-package Swift binding questions, plus a topic summary of per-package parking lots.

---

## Per-package parking lots

- [`auki-identity-swift/`](auki-identity-swift/parking_lot.md) — **new crate, landed 2026-05-21** (PR A of Spec 1: thin scaffolding host for `Wallet` + `PeerIdentity`); `WalletId` hidden behind a `wallet_id_str()` helper (upstream tuple struct not Record-compatible); async-shaped Swift API vs `-py` sync precedent (inherited from auki-network-swift); `Wallet`'s `sign`/`derive_child`/typed-`WalletId` surfaces deferred until iosapp needs them; `from_seed` returns `Result<Arc<Wallet>, IdentityError>` per UniFFI 0.31's constructor contract.
- [`auki-network-swift/`](auki-network-swift/parking_lot.md) — async-shaped Swift API vs the `-py` sync precedent (**flagged for human confirmation**); where generated Swift / XCFramework artifacts live + committed-vs-built distribution; stream-payload parity rule for Stage 2; `with_http` (custom reqwest::Client for proxies/TLS roots/timeouts) deliberately not exposed at Stage 1.
