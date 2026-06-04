# Changelog - examples

One-line summaries of changes under `examples/`.

Latest entry on top.

---

### Nils's codex · May 28, HKT, 2026

**[`relay-smoke`](relay-smoke/changelog.md) — relay-backed libp2p decision-gate harness added.** The new example pairs a Rust native target relay reservation smoke with a js-libp2p browser dialer that rejects non-browser-usable relay paths.

### Nils's codex · May 28, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer now awaits SDK stream-entry writes.** The native producer fanout now uses the async signaled Domain push path and covers chunked signaled WebRTC writes for large camera frames.

### Nils's codex · May 28, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer signaled two-peer join regression covered.** The native producer tests now prove SDK-owned Discovery-signaled WebRTC completes `/auki/join/0.0.1` between native peers and still times out unanswered peers.

### Nils's codex · May 28, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer signaled timeout regression covered.** The native producer test suite now locks the SDK-owned Discovery-signaled WebRTC no-answer path to a typed timeout instead of an indefinite startup wait.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer Discovery-signaled WebRTC transport enabled.** The native producer now uses SDK-owned Discovery-signaled WebRTC data channels for Overwatch browser interop instead of direct listen addresses.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer Overwatch staging documented.** The manual E2E flow now stages generated JavaScript packages before `npm install` so Overwatch starts from a fresh checkout.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer operator status and manual E2E runbook completed.** The native producer now shows live session/frame/error values and documents the binding, Overwatch, and device verification flow.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer capture lifecycle races hardened.** The native producer now drains pending frame forwarding before shutdown, respects stop during camera permission, and rolls back partial AVFoundation setup failures.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer AVFoundation capture added.** The native producer now requests camera permission, captures back-camera JPEG frames at 10 fps, timestamps them from the SDK session clock, previews them locally, and sends them through the existing Sensor Log/domain stream path.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer lifecycle races hardened.** The native producer now handles startup cancellation, best-effort shutdown cleanup, and duplicate keychain seed creation without leaving stale runtime or peer identity state.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer session logs scoped by session id.** The native producer now opens camera Sensor Logs below a session-specific root to avoid stale manifest reuse across app restarts.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer session coordination added.** The iOS example now uses generated Swift bindings to join/create a domain cluster, publish camera catalogs and registry entries, write Sensor Log payloads, and stream accepted camera frames.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer fanout failure isolation added.** Failed stream pushes are pruned without blocking healthy stream ids from receiving the encoded camera frame payload.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — AukiCameraStreamer frame codec and stream fanout added.** The native iOS example now serializes JPEG frames as generated `CameraFrame` protobuf payloads and tests fanout delivery/removal without camera hardware.

### Nils's codex · May 27, HKT, 2026

**[`ios`](ios/changelog.md) — Native iOS camera streamer shell added.** The new SwiftUI example scaffolds an XcodeGen app around generated Swift SDK package products with camera descriptor defaults and an initial operational control surface.

### Nils's codex · May 27, HKT, 2026

**[`overwatch`](overwatch/changelog.md) — Raw camera preview compatibility restored.** Camera-kind preview payloads now preserve raw JPEG streams and skip malformed generated-protobuf frames without stopping preview subscriptions.

### Nils's codex · May 27, HKT, 2026

**[`overwatch`](overwatch/changelog.md) — Native camera stream frames decode through generated protobuf bindings.** Overwatch now stages `@aukilabs/auki-proto`, carries camera sensor metadata on runtime frames, and decodes `CameraFrame.frame` bytes for JPEG previews while preserving raw non-camera payloads.

### Nils's codex · May 26, HKT, 2026

**[`overwatch`](overwatch/changelog.md) — Park brand assets copied into Overwatch.** Overwatch now ships the `/brand/*` SVG files referenced by Park's copied topbar and tests those Vite public asset paths.

### Nils's codex · May 26, HKT, 2026

**[`overwatch`](overwatch/changelog.md) — Park UI port completed with SDK browser runtime.** Overwatch now renders Park's operator UI while replacing Park backend data, registry, and stream paths with generated SDK JavaScript/WASM bindings and preserving the no app `/api/*` smoke invariant.

### Nils's codex · May 24, HKT, 2026

**[`diagnostic-app`](diagnostic-app/changelog.md) — domain binding defaults avoided.** The diagnostic app keeps using the direct Rust `auki-domain` API with `default-features = false` after the domain crate adopted generated binding defaults.

### Nils's codex · May 22, HKT, 2026

**[`ios`](ios/changelog.md) — browser-to-iOS message smoke harness added.** The iOS test app now includes a Node/js-libp2p script that sends a protobuf `MessageEnvelope` to the generated Swift/Rust message node over `/auki/message/0.0.1`.

### Nils's codex · May 22, HKT, 2026

**[`ios`](ios/changelog.md) — iOS generated-binding Auki network test host added.** The SwiftUI test app consumes generated `auki_network` and `AukiProto` Swift packages while SDK networking behavior remains inside Rust crates exposed through UniFFI.

### Nils's codex · May 21, HKT, 2026

**[`diagnostic-app`](diagnostic-app/changelog.md) — Domain flash mode now uses live cluster domain time.** The app reads `ClusterManager::domain_clock_estimate()` plus `domain_time_now()`, surfaces domain status/offset/uncertainty, and flashes on domain time only when explicit heartbeat sync is available.
