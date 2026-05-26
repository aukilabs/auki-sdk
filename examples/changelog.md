# Changelog - examples

One-line summaries of changes under `examples/`.

Latest entry on top.

---

### Nils's codex · May 24, HKT, 2026

**[`diagnostic-app`](diagnostic-app/changelog.md) — domain binding defaults avoided.** The diagnostic app keeps using the direct Rust `auki-domain` API with `default-features = false` after the domain crate adopted generated binding defaults.

### Nils's codex · May 22, HKT, 2026

**[`ios`](ios/changelog.md) — browser-to-iOS message smoke harness added.** The iOS test app now includes a Node/js-libp2p script that sends a protobuf `MessageEnvelope` to the generated Swift/Rust message node over `/auki/message/0.0.1`.

### Nils's codex · May 22, HKT, 2026

**[`ios`](ios/changelog.md) — iOS generated-binding Auki network test host added.** The SwiftUI test app consumes generated `auki_network` and `AukiProto` Swift packages while SDK networking behavior remains inside Rust crates exposed through UniFFI.

### Nils's codex · May 21, HKT, 2026

**[`diagnostic-app`](diagnostic-app/changelog.md) — Domain flash mode now uses live cluster domain time.** The app reads `ClusterManager::domain_clock_estimate()` plus `domain_time_now()`, surfaces domain status/offset/uncertainty, and flashes on domain time only when explicit heartbeat sync is available.
