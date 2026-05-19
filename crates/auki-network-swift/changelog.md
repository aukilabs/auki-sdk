# Changelog — auki-network-swift

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's claude · May 19, 14:27 HKT, 2026

**iOS XCFramework build validated end to end.** Ran `build-xcframework.sh` against `aarch64-apple-ios` / `aarch64-apple-ios-sim` / `x86_64-apple-ios` (rustc 1.94, Xcode 26.3): clean release builds, `uniffi-bindgen` Swift codegen, and `xcodebuild -create-xcframework` produce a well-formed two-slice `AukiNetwork.xcframework` (device `ios-arm64` + fat simulator `ios-arm64_x86_64-simulator`) with correct async Swift bindings (`DiscoveryClient.{createCluster,deregister,listClusters,livenessCheck,rotateManager}` as `async throws`, `ClusterEntry`/`CreateClusterOutcome`, `DiscoveryError: Swift.Error`). The anticipated `ring`/`SystemConfiguration` cross-compile edges did not occur — rustls' default `aws-lc-rs` cross-compiles to iOS with no intervention and Stage 1 is `discovery_client`-only (no libp2p). Added a crate `.gitignore` for the `target-xcframework/` build output (root `.gitignore`'s `/target` is root-anchored and doesn't cover it). Resolved and removed the `ring`-vs-`aws-lc-rs` parking-lot item; updated `src/readme.md` (XCFramework now in "works"), `src/sprint.md` (Next #1 done, renumbered), and the parent parking-lot summary. No code change.

### Nils's claude · May 19, 13:46 HKT, 2026

**New crate: UniFFI Swift bindings for `auki-network` (Stage 1 — Discovery surface).** Mirrors `auki-network-py`'s root Discovery surface and per-component binding convention (no umbrella `auki-swift`): `DiscoveryClient` UniFFI object with async `list_clusters`/`create_cluster`/`liveness_check`/`rotate_manager`/`deregister` plus sync `new`/`base_url`, `ClusterEntry`/`CreateClusterOutcome` records, and a flattened `DiscoveryError` (reqwest/libp2p types hidden; PeerId/Multiaddr cross as canonical strings). UniFFI proc-macro mode (`setup_scaffolding!`), futures driven by tokio via `#[uniffi::export(async_runtime = "tokio")]`. Depends on `auki-network` with only the `discovery_client` feature, so Stage 1 stays off the libp2p iOS-link sharp edges. Host `cargo build`/`cargo test -p auki-network-swift` green (error-mapping + conversion + seam-parse unit tests, no network). iOS XCFramework build is scripted (`build-xcframework.sh`) but not yet validated. Stage 2 (audio streams) and Stage 3 (`auki-domain-swift` cluster/peer enumeration) are scoped in `src/sprint.md`; the async-vs-`-py`-sync API-shape divergence is flagged in `parking_lot.md` for human confirmation. Implements the iosapp Q1 decision (UniFFI, thin, prost payloads via swift-protobuf later).
