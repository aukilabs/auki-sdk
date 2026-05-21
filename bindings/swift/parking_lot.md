# Parking lot — Swift bindings

Cross-package Swift binding questions, plus a topic summary of per-package parking lots.

---

## Per-package parking lots

- [`auki-identity-swift/`](auki-identity-swift/parking_lot.md) — **new crate, landed 2026-05-21** (PR A of Spec 1: thin scaffolding host for `Wallet` + `PeerIdentity`); `WalletId` hidden behind a `wallet_id_str()` helper (upstream tuple struct not Record-compatible); async-shaped Swift API vs `-py` sync precedent (inherited from auki-network-swift); `Wallet`'s `sign`/`derive_child`/typed-`WalletId` surfaces deferred until iosapp needs them; `from_seed` returns `Result<Arc<Wallet>, IdentityError>` per UniFFI 0.31's constructor contract.
- [`auki-network-swift/`](auki-network-swift/parking_lot.md) — 6 open items: async-shaped Swift API vs `-py` sync precedent (confirmed acceptable); where generated Swift / XCFramework artifacts live (committed vs. downstream build step); `with_http` (custom reqwest::Client) not exposed at v0; heartbeat-detail variants (`HeartbeatReceived`, `HeartbeatNtpSampleObserved`) dropped by drain task — widen if iosapp needs timing observation; `uniffi::custom_type!` reachability for `PeerId`/`Multiaddr` across `auki-domain-swift` (PR C); two-call `SwiftStreamProvider` protocol consistency contract (UniFFI 0.31 constraint, revisit on version bump).
- [`auki-domain-swift/parking_lot.md`](auki-domain-swift/parking_lot.md): 6 open items (BootstrapSwiftError variant mapping, cross-crate stream subscription propagation verification, deferred generic open_stream resolver, heartbeat-detail variant inheritance, TransformEdgeResource::source change audit, shared tokio runtime).
