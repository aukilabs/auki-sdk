# `auki-network-swift/src/`

Implementation status for [`auki-network-swift`](../README.md). Honest about what is real today.

## Files

- [`lib.rs`](lib.rs) — the whole Stage 1 binding: `DiscoveryClient` UniFFI object, `ClusterEntry` / `CreateClusterOutcome` records, the flattened `DiscoveryError`, PeerId/Multiaddr seam parsing, and non-network unit tests.
- [`bin/uniffi-bindgen.rs`](bin/uniffi-bindgen.rs) — host Swift-codegen entry point, gated behind the `cli` feature.

## What works today

- **Builds and tests on the host.** `cargo build -p auki-network-swift` and `cargo test -p auki-network-swift` are green. UniFFI proc-macro scaffolding (`uniffi::setup_scaffolding!()`) compiles into the library; `#[uniffi::export(async_runtime = "tokio")]` drives the async Discovery methods.
- **Full Discovery surface wired**: `new`, `base_url`, `list_clusters`, `create_cluster`, `liveness_check`, `rotate_manager`, `deregister` — 1:1 with `auki_network::discovery_client::DiscoveryClient`.
- **Error mapping** `DiscoveryError` ← `auki_network` `DiscoveryError`, exhaustive, covered by unit tests (`Status`, `InvalidPeerId`, `InvalidMultiaddr`, the `ClusterEntry` stringify conversion, and seam-parse rejection).

## What does NOT work yet

- **No generated Swift / XCFramework is verified.** `build-xcframework.sh` exists but the iOS cross-compile + `uniffi-bindgen generate` + `xcodebuild -create-xcframework` flow has **not** been run/validated here (no iOS targets installed, depends on `ring`/`aws-lc-rs` cross-compile, `SystemConfiguration.framework` link — see `src/sprint.md` and `parking_lot.md`).
- **No stream/audio surface.** `StreamRequest`, `AudioFrame`, producer/consumer — Stage 2.
- **No cluster lifecycle / peer enumeration / `ParticipantInfo`.** That is a separate future `auki-domain-swift`, mirroring the `auki-network-py` → `auki-domain-py` split.
- **Not wired into iosapp.** The `Bridge/` SPM integration is an iosapp-repo task.

## Rust mapping

| Swift | Rust |
|---|---|
| `DiscoveryClient` | `auki_network::discovery_client::DiscoveryClient` |
| `ClusterEntry` | `auki_network::discovery_client::ClusterEntry` |
| `CreateClusterOutcome` | `auki_network::discovery_client::CreateClusterOutcome` |
| `DiscoveryError` | `auki_network::discovery_client::DiscoveryError` (flattened, reqwest/libp2p types hidden) |

## Verification

```bash
cargo test -p auki-network-swift          # host gate (green)
cargo build -p auki-network-swift         # host gate (green)
crates/auki-network-swift/build-xcframework.sh   # iOS (NOT yet validated)
```
