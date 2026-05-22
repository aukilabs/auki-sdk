# Changelog - AukiNetworkTestApp

Append-only changelog for the iOS Auki network generated-binding test app.

Latest entry on top.

---

### Nils's codex · May 22, HKT, 2026

**Browser-to-iOS message smoke harness added.** `browser-message-smoke.mjs` initializes the generated browser `auki-network` package, creates a js-libp2p peer from wallet seed bytes, dials an iOS listen multiaddr, and writes a length-prefixed `MessageEnvelope` over `/auki/message/0.0.1`.

Tests: `node examples/ios/AukiNetworkTestApp/browser-message-smoke.mjs --help`; `git diff --check`.

### Nils's codex · May 22, HKT, 2026

**Generated Swift binding test host added.** The app imports generated `auki_network` UniFFI bindings and generated SwiftProtobuf `AukiProto`, starts an `AukiMessageNode` from wallet seed bytes, exposes WebRTC Direct listen addresses, and provides basic dial/send/poll controls for `/auki/message/0.0.1` smoke testing. The Xcode project is generated from `project.yml` and links `SystemConfiguration.framework` for Rust libp2p transitive dependencies.

Tests: `xcodebuild -project examples/ios/AukiNetworkTestApp/AukiNetworkTestApp.xcodeproj -scheme AukiNetworkTestApp -destination 'generic/platform=iOS Simulator' build`; `git diff --check`.
