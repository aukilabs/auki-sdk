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

## Next

In priority order:

1. **Validate the iOS XCFramework build.** Run `build-xcframework.sh` end to end on a machine with the Apple Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`. Resolve the known sharp edges — `ring` vs `aws-lc-rs` cross-compile for the rustls/Discovery path, and the `SystemConfiguration.framework` link. This is the gating unknown before iosapp can consume anything.
2. **Stage 2 — stream/audio surface.** Port `auki-network-py`'s `stream_types`/`stream_bridge` model to UniFFI: `StreamRequest`/`StreamDescriptor`, the producer `StreamProvider` callback, consumer subscription. Per iosapp's Q1, the prost payloads (`AudioFrame`, `StreamMessage`) cross the FFI as opaque `bytes` and are decoded Swift-side via swift-protobuf against the committed `crates/auki-datatypes/proto/*.proto`. UniFFI has no native `Stream`; use an async callback interface or a poll object + explicit `cancel()`.
3. **Stage 3 — `auki-domain-swift`.** Cluster join + peer enumeration + `ParticipantInfo`, mirroring `auki-domain-py`'s `ClusterManager`. This is what iosapp's peer list actually needs; it is a *separate* crate, not this one.
4. **Wire into iosapp.** SPM dependency + `Bridge/` shim in `aukilabs/iosapp` (that repo's Sprint 1 item).

## Open Items

See [`parking_lot.md`](parking_lot.md): the async-vs-sync API-shape divergence from `-py` (flagged for human confirmation), where generated Swift/XCFramework artifacts live and whether they are committed or built downstream, and the iOS crypto-backend choice.

## Out Of Scope

- Cluster lifecycle / peer enumeration — `auki-domain-swift` (Stage 3).
- Re-exposing identity/seed primitives — a future `auki-identity-swift`, mirroring `auki-identity-py`.
- An umbrella `auki-swift` — explicitly disallowed by the per-component binding convention.
