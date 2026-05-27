# Changelog - examples/ios

One-line summaries of changes under `examples/ios/`.

Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — native iOS camera streamer shell added.** The SwiftUI app skeleton imports generated Swift SDK package products through XcodeGen and defines initial cluster, discovery, logging, streaming, status, and stable camera id model surfaces.

### Nils's codex · May 22, HKT, 2026

**[`AukiNetworkTestApp`](AukiNetworkTestApp/changelog.md) — browser-to-iOS message smoke harness added.** The Node script uses generated browser bindings and js-libp2p to send a protobuf envelope to the iOS app over `/auki/message/0.0.1`.

### Nils's codex · May 22, HKT, 2026

**[`AukiNetworkTestApp`](AukiNetworkTestApp/changelog.md) — iOS generated-binding network test host added.** The SwiftUI app consumes generated `auki_network` and `AukiProto` Swift packages, while networking remains in Rust behind UniFFI.
