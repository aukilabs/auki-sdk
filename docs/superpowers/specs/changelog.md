# Changelog — docs/superpowers/specs

Append-only timeline of design spec changes. Latest entry on top.

---

### Nils's claude · May 21, 13:38 HKT, 2026

Rewrote the SDK Swift binding expansion design (`2026-05-20-sdk-swift-binding-expansion-design.md`, now revision 2) after user pushback on the v1 hand-wrapping approach. v2 pivots to **upstream proc-macros under a new `swift-bindings` cargo feature** on each of `crates/auki-identity`, `crates/auki-network`, `crates/auki-domain`. The binding crates under `bindings/swift/` shrink to thin scaffolding hosts (a `setup_scaffolding!()` call + custom-type registrations for `PeerId`/`Multiaddr` → `String` + minor re-exports). UniFFI introspects the upstream types and generates the Swift surface directly; hand-mapped Records, wrapper structs (`pub struct FooSwift { inner: Foo }`), and per-method shims largely vanish. The only hand-written upstream additions are 2–4 small `_for_swift` shims for methods whose Rust shape is FFI-incompatible (e.g. `NetworkRuntime::spawn`'s 8-mpsc return). Implementation staging unchanged in spirit (three sequential PRs) but each PR now touches both the upstream crate (feature + annotations) and the binding crate (scaffolding).

### Nils's claude · May 21, 13:24 HKT, 2026

Added the SDK Swift binding expansion design spec (`2026-05-20-sdk-swift-binding-expansion-design.md`). Three new/expanded binding crates under `bindings/swift/` covering the SDK surface that `aukilabs/iosapp`'s proof-of-load demo will exercise: `auki-identity-swift` (Wallet + PeerIdentity), `auki-network-swift` expansion (NetworkRuntime spawn + PeerLivenessEvent callback + full stream surface — Stage 2 folded in), `auki-domain-swift` (full ClusterManager parity with `bindings/python/auki-domain-py`, including `open_stream`, catalogs, registries, provider callbacks). UniFFI 0.31 with async-tokio runtime per crate, callback interfaces for event streams and provider hooks, prost payloads as opaque bytes (Swift decodes via swift-protobuf against the committed `crates/auki-datatypes/proto/*.proto`). iOS cross-compile risks (SystemConfiguration link, libp2p-stream pin) called out. Staged for three sequential PRs (A: identity, B: network expansion, C: domain). Blocks Spec 2 (iosapp wiring + proof-of-load UI).

### Nils's codex · May 19, HKT, 2026

Added the native Auki pointcloud design spec, capturing the approved breaking refactor from ROS CDR pointcloud streams to a shared `auki.point_cloud.PointCloudFrame { point_count, data }` record for logs and streams.
