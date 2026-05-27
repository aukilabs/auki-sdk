# Changelog - examples

One-line summaries of changes under `examples/`.

Latest entry on top.

---

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
