# Changelog — docs/superpowers/specs

Append-only timeline of design spec changes. Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

Updated the browser Domain peer adapter spec language to name the first browser sensor as an SDK `audio` sensor backed by microphone capture, not a separate microphone sensor kind.

### Nils's codex · May 19, HKT, 2026

Added the wasm libp2p browser transport spike spec, narrowing Park's browser-peer transport question to a rust-libp2p Wasm probe with WebRTC Direct first, WebTransport second, and Secure WebSocket only as fallback.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain peer adapter design spec for Park Milestone 0, defining the `auki-domain-browser` package shape, SDK-owned networking rule, identity/Discovery/roster first slice, and transport blockers before audio.

### Nils's codex · May 19, HKT, 2026

Added the native Auki pointcloud design spec, capturing the approved breaking refactor from ROS CDR pointcloud streams to a shared `auki.point_cloud.PointCloudFrame { point_count, data }` record for logs and streams.
