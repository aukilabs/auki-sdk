# Sprint — auki-network-swift

Closing the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now

Stage 1 landed: the crate exists as a workspace member, mirrors the `auki-network-py` doc/convention layout, and exposes the **root Discovery surface** via UniFFI proc-macros with async tokio-driven methods. Host `cargo build`/`cargo test` are green. iOS XCFramework generation is scripted but unproven.

| Swift method | Rust method |
|---|---|
| `listClusters()` | `DiscoveryClient::list_clusters` |
| `createCluster(name, managerPeerId, managerMultiaddrs)` | `DiscoveryClient::create_cluster` |
| `livenessCheck(name, peerCount)` | `DiscoveryClient::liveness_check` |
| `rotateManager(name, managerPeerId, managerMultiaddrs)` | `DiscoveryClient::rotate_manager` |
| `deregister(name)` | `DiscoveryClient::deregister` |

## Done since Stage 1

- **iOS XCFramework validated.** `build-xcframework.sh` runs clean against `aarch64-apple-ios` / `aarch64-apple-ios-sim` / `x86_64-apple-ios` (rustc 1.94, Xcode 26.3) and emits a well-formed two-slice `AukiNetwork.xcframework` + correct async Swift bindings (see `src/readme.md`). The anticipated `ring`/`SystemConfiguration` sharp edges did **not** occur — rustls' default `aws-lc-rs` cross-compiles to iOS cleanly and Stage 1 is `discovery_client`-only. Build output is gitignored (`target-xcframework/`).

## Next

In priority order:

1. **Stage 2 — stream/audio surface.** Port `auki-network-py`'s `stream_types`/`stream_bridge` model to UniFFI: `StreamRequest`/`StreamDescriptor`, the producer `StreamProvider` callback, consumer subscription. Per iosapp's Q1, the prost payloads (`AudioFrame`, `StreamMessage`) cross the FFI as opaque `bytes` and are decoded Swift-side via swift-protobuf against the committed `crates/auki-datatypes/proto/*.proto`. UniFFI has no native `Stream`; use an async callback interface or a poll object + explicit `cancel()`. Note: Stage 2 pulls the `swarm` feature → libp2p, which is where the `SystemConfiguration.framework` link edge may finally surface (it did not at Stage 1).
2. **Stage 3 — `auki-domain-swift`.** Cluster join + peer enumeration + `ParticipantInfo`, mirroring `auki-domain-py`'s `ClusterManager`. This is what iosapp's peer list actually needs; it is a *separate* crate, not this one.
3. **Wire into iosapp.** SPM dependency + `Bridge/` shim in `aukilabs/iosapp` (that repo's Sprint 1 item) — now unblocked for the Discovery surface since the XCFramework builds.

## Open Items

See [`parking_lot.md`](parking_lot.md): the async-vs-sync API-shape divergence from `-py` (flagged for human confirmation) and where generated Swift/XCFramework artifacts live (committed SwiftPM package vs. downstream build step). The iOS crypto-backend question is resolved — `aws-lc-rs` (rustls default) cross-compiles to iOS with no intervention.

## Out Of Scope

- Cluster lifecycle / peer enumeration — `auki-domain-swift` (Stage 3).
- Re-exposing identity/seed primitives — a future `auki-identity-swift`, mirroring `auki-identity-py`.
- An umbrella `auki-swift` — explicitly disallowed by the per-component binding convention.
