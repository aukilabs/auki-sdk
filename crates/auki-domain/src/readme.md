# auki-domain - implementation status

What is implemented today. See [`../README.md`](../README.md) for the crate-level spec.

## Files

- [`lib.rs`](lib.rs) - module exports and public re-exports.
- [`cluster_membership.rs`](cluster_membership.rs) - `ClusterMembership`, `ClusterMember`, membership JSON shape, filename helper, admission ordering.
- [`cluster_manager.rs`](cluster_manager.rs) - `ClusterManager`, `ClusterTarget`, daemon info, sensor/resource catalog provider traits, Discovery bootstrap logic, Manager/member state, join/liveness/membership/info/resources/sensors tasks, stream opener, shutdown, and election helper.
- [`stream_manifest.rs`](stream_manifest.rs) - producer-side `StreamManifestBuilder` that derives accept metadata from local Sensor / Frame registries.

## Implemented

- `ClusterMembership::new`, `filename`, and `admit`.
- `ClusterManager::list_clusters`, `bootstrap`, `create_cluster`, and `join_cluster`.
- `ClusterTarget::{create, join, join_or_create, most_recent_or_create}`.
- Manager admission through `/auki/join/0.0.1`.
- Membership gossip through `/auki/membership/0.0.1`, including the Manager peer id to converge handoff broadcasts.
- Manager-star heartbeat/liveness detection through `/auki/heartbeat/0.0.1`, with topology and timeout semantics owned here rather than in `auki-network`; raw carrier close is not treated as semantic death until the heartbeat timeout expires.
- Manager election and Discovery `rotate_manager` handoff.
- Manager -> Discovery `liveness_check` loop every `LIVENESS_CHECK_INTERVAL` (1 second).
- SDK-owned `ParticipantInfo` generation plus `/auki/info/0.0.1` fetches, with `session_now_ns` and session clock id/hash sourced from `auki_time::SessionClock`.
- Resource catalog provider registration plus `/auki/resources/0.0.1` fetches, auto-lifting sensor catalog rows into `sensor_stream` resources and accepting producer-supplied `transform_edge` rows.
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
- `ResourceCatalogProvider`
- `ResourceEntry`
- `ResourceKind`
- `ResourcePinholeIntrinsics`
- `ResourceQuat`
- `ResourceSpatialTransform`
- `ResourceVec3`
- `ResourcesRequest`
- `ResourcesResponse`
- `SensorStreamResource`
- `TransformEdgeResource`
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

## Timekeeping

`ClusterManager` constructs one SDK-owned `SessionClock` at create/join time using the local peer id and `DaemonInfo.session_id`. `ParticipantInfo.session_clock_id`, `session_clock_hash`, and `session_now_ns` come from that clock, not from caller-supplied `DaemonInfo.session_clock_id/hash`. Those input fields remain accepted for compatibility until the constructor surface can stop asking callers for session clock identity.

Heartbeat time sync is intentionally not implemented here yet. When it lands, heartbeat timestamps should read from this same `SessionClock` so the timestamp values and Clock Registry identity stay bound to one monotonic epoch.

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
