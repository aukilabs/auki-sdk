# Changelog — docs/superpowers/specs

Append-only timeline of design spec changes. Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

Added the SDK signaled WebRTC transport design spec, defining SDK-owned Discovery signaling, bidirectional WebRTC data-channel routing, native Swift/browser binding adapters, and the `auki-domain` signaled peer facade.

### Nils's codex · May 27, HKT, 2026

Added the iOS camera streamer design spec, defining the generated Swift binding app as a native `auki-domain` producer peer that logs typed camera frames and streams them for Overwatch consumption.

### Nils's codex · May 22, HKT, 2026

Corrected the iOS Auki network UniFFI design so the primary artifact is an iOS test app consuming generated Swift bindings from the Rust crates, with the message-networking facade exported by `auki-network` through UniFFI.

### Nils's codex · May 22, HKT, 2026

Added the iOS Auki network UniFFI design spec, locking the no-swift-libp2p rule and scoping the first browser-to-iOS milestone to `/auki/message/0.0.1` over Rust `auki-network` exposed through Swift UniFFI.

### Nils's codex · May 22, HKT, 2026

Updated the Auki proto generation design so only the generated Rust `auki-proto` crate is committed; JavaScript/TypeScript, Swift, and Python protobuf outputs are generated under ignored `bindings/` paths.

### Nils's codex · May 22, HKT, 2026

Updated the Auki proto generation design so Python protobuf output is generated on demand and not committed in the initial `auki-proto` migration.

### Nils's codex · May 22, HKT, 2026

Added the Auki proto generation design spec, locking root `proto/auki` schemas, generated per-platform `auki-proto` packages, and `auki-datatypes` deprecation as a Rust compatibility shim.

### Nils's codex · May 21, HKT, 2026

Refined the stream naming cleanup design spec to describe the already-renamed final vocabulary without reintroducing legacy identifiers into active docs.

### Nils's codex · May 21, HKT, 2026

Added the stream naming cleanup design spec, locking the breaking full rename to `CameraFrame`, `DetectionFrame`, and `SensorBody::Camera` with no compatibility aliases or legacy registry tags.

### Nils's codex · May 21, HKT, 2026

Updated the browser Domain peer adapter spec language to name the first browser sensor as an SDK `audio` sensor backed by microphone capture, not a separate microphone sensor kind.

### Nils's codex · May 19, HKT, 2026

Added the native Auki pointcloud design spec, capturing the approved breaking refactor from ROS CDR pointcloud streams to a shared `auki.point_cloud.PointCloudFrame { point_count, data }` record for logs and streams.

### Nils's codex · May 19, HKT, 2026

Added the wasm libp2p browser transport spike spec, narrowing Park's browser-peer transport question to a rust-libp2p Wasm probe with WebRTC Direct first, WebTransport second, and Secure WebSocket only as fallback.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain peer adapter design spec for Park Milestone 0, defining the `auki-domain-browser` package shape, SDK-owned networking rule, identity/Discovery/roster first slice, and transport blockers before audio.
