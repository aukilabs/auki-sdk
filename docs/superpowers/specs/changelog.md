# Changelog — docs/superpowers/specs

Append-only timeline of design spec changes. Latest entry on top.

---

### Nils's claude · May 21, 13:24 HKT, 2026

Added the SDK Swift binding expansion design spec (`2026-05-20-sdk-swift-binding-expansion-design.md`). Three new/expanded binding crates under `bindings/swift/` covering the SDK surface that `aukilabs/iosapp`'s proof-of-load demo will exercise: `auki-identity-swift` (Wallet + PeerIdentity), `auki-network-swift` expansion (NetworkRuntime spawn + PeerLivenessEvent callback + full stream surface — Stage 2 folded in), `auki-domain-swift` (full ClusterManager parity with `bindings/python/auki-domain-py`, including `open_stream`, catalogs, registries, provider callbacks). UniFFI 0.31 with async-tokio runtime per crate, callback interfaces for event streams and provider hooks, prost payloads as opaque bytes (Swift decodes via swift-protobuf against the committed `crates/auki-datatypes/proto/*.proto`). iOS cross-compile risks (SystemConfiguration link, libp2p-stream pin) called out. Staged for three sequential PRs (A: identity, B: network expansion, C: domain). Blocks Spec 2 (iosapp wiring + proof-of-load UI).

### Nils's codex · May 19, HKT, 2026

Added the native Auki pointcloud design spec, capturing the approved breaking refactor from ROS CDR pointcloud streams to a shared `auki.point_cloud.PointCloudFrame { point_count, data }` record for logs and streams.
