# Changelog — auki-network-swift

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's claude · May 19, 13:46 HKT, 2026

**New crate: UniFFI Swift bindings for `auki-network` (Stage 1 — Discovery surface).** Mirrors `auki-network-py`'s root Discovery surface and per-component binding convention (no umbrella `auki-swift`): `DiscoveryClient` UniFFI object with async `list_clusters`/`create_cluster`/`liveness_check`/`rotate_manager`/`deregister` plus sync `new`/`base_url`, `ClusterEntry`/`CreateClusterOutcome` records, and a flattened `DiscoveryError` (reqwest/libp2p types hidden; PeerId/Multiaddr cross as canonical strings). UniFFI proc-macro mode (`setup_scaffolding!`), futures driven by tokio via `#[uniffi::export(async_runtime = "tokio")]`. Depends on `auki-network` with only the `discovery_client` feature, so Stage 1 stays off the libp2p iOS-link sharp edges. Host `cargo build`/`cargo test -p auki-network-swift` green (error-mapping + conversion + seam-parse unit tests, no network). iOS XCFramework build is scripted (`build-xcframework.sh`) but not yet validated. Stage 2 (audio streams) and Stage 3 (`auki-domain-swift` cluster/peer enumeration) are scoped in `src/sprint.md`; the async-vs-`-py`-sync API-shape divergence is flagged in `parking_lot.md` for human confirmation. Implements the iosapp Q1 decision (UniFFI, thin, prost payloads via swift-protobuf later).
