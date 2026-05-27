# Changelog - examples/ios

One-line summaries of changes under `examples/ios/`.

Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — Overwatch runbook staging documented.** The manual E2E flow now includes generated JavaScript package staging before `npm install` so fresh checkouts can start Overwatch.

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — operator status and manual E2E runbook completed.** The example now surfaces live session/frame/error values in SwiftUI and documents the binding, Overwatch, and device verification flow.

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — capture lifecycle races hardened.** The example now drains frame-forwarding tasks before shutdown, avoids starting sessions after canceled permission prompts, and rolls back partial capture setup failures.

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — AVFoundation camera capture added.** The example now requests camera permission, captures back-camera JPEG frames at 10 fps, timestamps them from the SDK session clock, shows a local preview, and feeds the existing log/stream path.

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — session lifecycle races hardened.** Startup cancellation, shutdown cleanup, and concurrent keychain seed creation now preserve the running session and persisted peer identity contracts.

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — session Sensor Logs scoped per launch.** Camera logs now live under a session-specific root so a new producer session cannot append under a stale manifest from a prior run.

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — native camera session coordination added.** The example now bootstraps a generated-binding domain producer session, publishes camera catalogs/registry entries, writes Sensor Log bytes, and serves accepted camera streams from the same encoded frame payload.

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — camera stream fanout failure isolation added.** The example now keeps healthy stream ids receiving frames when one sink push fails and removes failed stream ids from the active fanout set.

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — camera frame codec and stream fanout added.** The example encodes JPEG payloads as generated `CameraFrame` protobuf bytes and fans them out to accepted stream ids with hardware-free unit coverage.

### Nils's codex · May 27, HKT, 2026

**[`AukiCameraStreamer`](AukiCameraStreamer/changelog.md) — native iOS camera streamer shell added.** The SwiftUI app skeleton imports generated Swift SDK package products through XcodeGen and defines initial cluster, discovery, logging, streaming, status, and stable camera id model surfaces.

### Nils's codex · May 22, HKT, 2026

**[`AukiNetworkTestApp`](AukiNetworkTestApp/changelog.md) — browser-to-iOS message smoke harness added.** The Node script uses generated browser bindings and js-libp2p to send a protobuf envelope to the iOS app over `/auki/message/0.0.1`.

### Nils's codex · May 22, HKT, 2026

**[`AukiNetworkTestApp`](AukiNetworkTestApp/changelog.md) — iOS generated-binding network test host added.** The SwiftUI app consumes generated `auki_network` and `AukiProto` Swift packages, while networking remains in Rust behind UniFFI.
