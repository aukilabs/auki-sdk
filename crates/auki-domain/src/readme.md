# auki-domain - implementation status

What is implemented today. See [`../README.md`](../README.md) for the crate-level spec.

## Files

- [`lib.rs`](lib.rs) - module exports and public re-exports.
- [`cluster_membership.rs`](cluster_membership.rs) - `ClusterMembership`, `ClusterMember`, membership JSON shape, filename helper, admission ordering.
- [`cluster_manager.rs`](cluster_manager.rs) - `ClusterManager`, `ClusterTarget`, daemon info, sensor catalog provider trait, Discovery bootstrap logic, Manager/member state, join/liveness/membership/info/sensors tasks, stream opener, shutdown, and election helper.
- [`stream_manifest.rs`](stream_manifest.rs) - producer-side `StreamManifestBuilder` that derives accept metadata from local Sensor / Frame registries.

## Implemented

- `ClusterMembership::new`, `filename`, and `admit`.
- `ClusterManager::list_clusters`, `bootstrap`, `create_cluster`, and `join_cluster`.
- `ClusterTarget::{create, join, join_or_create, most_recent_or_create}`.
- Manager admission through `/auki/join/0.0.1`.
- Membership gossip through `/auki/membership/0.0.1`.
- Manager-star heartbeat/liveness detection through `/auki/heartbeat/0.0.1`, with topology and timeout semantics owned here rather than in `auki-network`.
- Manager election and Discovery `rotate_manager` handoff.
- Manager -> Discovery `liveness_check` loop every `LIVENESS_CHECK_INTERVAL` (1 second).
- SDK-owned `ParticipantInfo` generation plus `/auki/info/0.0.1` fetches.
- Sensor catalog provider registration plus `/auki/sensors/0.0.1` fetches, including the detail request that can embed local Sensor / Frame Registry JSON by value.
- Registry app-root registration plus `/auki/registries/0.0.1` typed fetches for Sensor / Clock / Frame Registry entries.
- `StreamManifestBuilder::from_registry`, which constructs stream accept manifests from a producer's local registry and verifies exact frame references for spatial sensors.
- Cluster-handle `open_stream::<T>` delegating to `NetworkRuntime`.
- Shared-reference, idempotent `shutdown`.

## Public Re-exports

`lib.rs` re-exports:

- `ClusterManager`
- `ClusterTarget`
- `ClusterMembership`
- `ClusterMember`
- `DaemonInfo`
- `SensorCatalogProvider`
- `SensorEntry`
- `SensorsRequest`
- `SensorsResponse`
- `SensorRegistryEntry`
- `ClockRegistryEntry`
- `FrameRegistryEntry`
- `RegistryKind`
- `StreamManifestBuilder`
- `LIVENESS_CHECK_INTERVAL`
- Error types for bootstrap/create/join/admit/fetch paths
- `elect_successor`

## Deferred

- Typed successor-token format and Discovery-side verification.
- SDK-managed relay reservation helper for non-LAN clusters.
- Possible demotion of direct `DiscoveryClient` usage after app migrations prove the SDK-fronted `ClusterManager` path.

## Verification

For implementation changes:

```bash
cargo test -p auki-domain
DISCOVERY_URL=http://127.0.0.1:8080 cargo test -p auki-domain --test cluster_manager_integration -- --ignored
```
