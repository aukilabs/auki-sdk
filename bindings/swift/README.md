# Swift Bindings

Swift-facing SDK packages live here. The structure preserves the existing per-component package names: UniFFI wrappers sit alongside (eventually) generated SwiftPM artifacts, while Rust SDK implementation crates remain in [`../../crates`](../../crates) — same per-language rule the Python bindings follow.

| Package | What it does |
|---|---|
| [`auki-identity-swift`](auki-identity-swift) | UniFFI Swift bindings for `auki-identity::Wallet` and the identity-shaped slice of `auki-network::PeerIdentity`. Thin scaffolding host; UniFFI proc-macros live on the upstream types behind a `swift-bindings` cargo feature. |
| [`auki-network-swift`](auki-network-swift) | UniFFI Swift bindings for `auki-network`'s full v0 networking surface. `NetworkRuntime` + `spawn_for_swift` orchestrator + 5-payload stream surface (Audio, Camera, PointCloud, JointEncoders, Detection) + `PeerLivenessListener` / `HeartbeatTimestampProvider` / `SwiftStreamProvider` (two-call protocol) callback interfaces + 5 `Swift*Source` traits + upstream-annotated Discovery types (`DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`). Cluster lifecycle / peer enumeration will arrive as a future `auki-domain-swift`. |
