# auki-domain-swift

UniFFI Swift bindings for `auki-domain` — exposes the cluster Manager surface (bootstrap / membership / catalogs / streams / clock sync / diagnostics) to native iOS (Swift) peers. Aggregates the upstream-annotated types from `auki-identity` (PR A), `auki-network` (PR B), `auki-domain` (this PR), and `auki-time` under a single `AukiDomain.xcframework` umbrella.

## Scope (v0 — PR C)

- **Bootstrap orchestrators**: `bootstrap_swift`, `create_cluster_swift`, `join_cluster_swift`, plus the static `list_clusters` helper. Each takes a 32-byte wallet seed + listen multiaddrs + agent version + DaemonInfo + optional SwiftStreamProvider. Mirrors `auki-domain-py`'s `ClusterManager.bootstrap` / `.create_cluster` / `.join_cluster`.
- **ClusterManager surface**: full method set — `cluster_name`, `local_peer_id_string`, `local_multiaddr_strings`, `manager_peer_id_string`, `is_manager`, `peer_count_u32`, `membership`, `participant_info`, `fetch_participant_info`, `admit_peer`, `fetch_*_catalog` (sensors + resources), `fetch_*_entry` (sensor + clock + frame + detector), the 5 typed `open_*_stream` methods, `set_sensor_catalog_provider`, `set_resource_catalog_provider`, `set_registry_app_root`, `clock_sync_estimate` / `clock_sync_estimates` / `domain_clock_estimate` / `domain_time_now`, `broadcast_diagnostic_message` / `drain_diagnostic_messages`, `shutdown`.
- **Callback interfaces**: `SensorCatalogProvider`, `ResourceCatalogProvider` (upstream traits, exported as Swift protocols).
- **Stream surface**: 5 typed `StreamSubscription*` Objects + the typed `open_*_stream` methods, re-exported from PR B's `auki-network-swift`. Producer side via PR B's `SwiftStreamProvider` two-call protocol.
- **Registry typed records**: full `SensorBody` / `ClockBody` / `FrameRegistryEntry` / `DetectorRegistryEntry` trees annotated as UniFFI Records / Enums — Swift consumers get typed structs, not opaque JSON strings.

## Out of scope

- iosapp integration code — that's Spec 2.
- A published SwiftPM package — distribution stays build-from-source via the iosapp sync script.
- `browser_session.rs` — browser-runtime feature; not native iOS.
- `elect_successor` — internal upstream utility.

## Build

XCFramework via `./build-xcframework.sh`. Validated against Xcode 26.3 + rustc 1.94. Produces a two-slice framework: `ios-arm64` (device) + `ios-arm64_x86_64-simulator` (fat sim).

iOS targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`.

## Consuming from Swift (sketch)

```swift
import AukiDomain  // hypothetical umbrella module

let manager = try await bootstrapSwift(
    target: .create(name: "demo-cluster"),
    walletSeed: Data(/* 32 bytes from Keychain */),
    discoveryUrl: "http://192.168.9.130:8080",
    listenAddresses: ["/ip4/0.0.0.0/tcp/0"],
    agentVersion: "iosapp/0.0",
    daemonInfo: DaemonInfo(...),
    streamProvider: nil,
    externalAddresses: nil
)

print("Local peer-id: \(manager.localPeerIdString())")
let membership = manager.membership()
print("Members: \(membership.peers.count)")

let sensors = try await manager.fetchSensorsCatalog(peerId: peerIdString)
```
