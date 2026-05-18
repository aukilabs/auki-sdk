# auki-domain

Cluster lifecycle for the Auki SDK.

This crate is the app-facing layer above [`auki-network`](../auki-network). It owns cluster creation, joining, membership, Manager role state, peer liveness, successor election, Discovery liveness checks, Manager handoff, peer identity lookup, resource catalog lookup, transform-edge discovery, and stream access. It is not the home for `convert_time` or `convert_pose`; those operations consume transforms produced elsewhere.

## Current Surface

The central type is `ClusterManager`. App daemons should treat it as the single SDK handle for Discovery + cluster lifecycle.

```rust
use auki_domain::{ClusterManager, ClusterTarget, DaemonInfo};

let manager = ClusterManager::bootstrap(
    ClusterTarget::most_recent_or_create("hagall"),
    local_identity,
    advertise_multiaddrs,
    "http://discovery.lan:8080",
    swarm,
    stream_provider,
    daemon_info,
).await?;
```

Operator-driven UIs can call the explicit primitives:

- `ClusterManager::list_clusters(discovery_url)` - read Discovery's directory, newest-first.
- `ClusterManager::create_cluster(...)` - create exactly this cluster and become its initial Manager.
- `ClusterManager::join_cluster(...)` - join exactly this existing cluster through its current Manager.
- `ClusterManager::bootstrap(target, ...)` - policy-driven create/join used by headless daemons.

`ClusterTarget` captures the four app intent shapes:

- `Create { name }`
- `Join { name }`
- `JoinOrCreate { name }`
- `MostRecentOrCreate { fallback_name }`

## What `ClusterManager` Owns

- A `ClusterMembership` document: cluster name plus ordered `ClusterMember` rows.
- The local peer id, local advertised multiaddrs, current Manager peer id, and Manager/member role state.
- A `NetworkRuntime` that drives libp2p.
- Discovery client calls for create, list, liveness, Manager rotation, and final deregistration.
- Manager-star heartbeat topology and timeout/loss semantics; `auki-network` only carries `/auki/heartbeat/0.0.1` frames and reports carrier events, while raw carrier close waits for the heartbeat timeout before election or eviction.
- Background tasks for join admission, peer liveness, membership gossip, info requests, resource catalog requests, sensor catalog requests, registry entry requests, and Manager liveness checks.

Useful methods:

| Method | Purpose |
|---|---|
| `cluster_name()` / `local_peer_id()` / `local_multiaddrs()` | Local identity snapshot |
| `is_manager()` / `manager_peer_id()` | Current role view |
| `membership()` / `peer_count()` | Current cluster membership snapshot |
| `participant_info()` | Fresh SDK-owned `/api/info` payload |
| `fetch_participant_info(peer_id)` | Fetch a peer's `ParticipantInfo` over `/auki/info/0.0.1` |
| `set_resource_catalog_provider(provider)` | Install producer-owned resources beyond auto-lifted sensor streams, starting with rigid transform edges |
| `fetch_resources_catalog(peer_id)` | Fetch a peer's resource catalog over `/auki/resources/0.0.1` |
| `fetch_resources_catalog_with(peer_id, request)` | Fetch filtered resource rows and optionally embed Sensor / Frame Registry JSON |
| `set_sensor_catalog_provider(provider)` | Install the producer's current sensor catalog source |
| `fetch_sensors_catalog(peer_id)` | Fetch a peer's sensor catalog over `/auki/sensors/0.0.1` |
| `fetch_sensors_catalog_with(peer_id, request)` | Fetch a sensor catalog with optional embedded Sensor / Frame Registry JSON |
| `set_registry_app_root(app_root)` | Install the producer's app root for serving registry entries |
| `fetch_sensor_entry` / `fetch_clock_entry` / `fetch_frame_entry` | Fetch and verify hash-pinned registry entries over `/auki/registries/0.0.1` |
| `open_stream::<T>(peer_id, request)` | Open a typed stream through the cluster handle |
| `shutdown()` | Idempotent shared-reference shutdown |

Producer-side stream providers can use `StreamManifestBuilder::from_registry(app_root, sensor_id, sensor_hash, clock_id, clock_hash)` to build an accept manifest from the local registry. For `RgbCamera` and `PointCloud` sensors it copies `frame_id` + `frame_hash` from the sensor body and verifies the exact frame entry exists; for `Audio` and `JointEncoders` it leaves frame fields empty.

`/auki/resources/0.0.1` is the primary live discovery surface. `ClusterManager` auto-lifts the registered `SensorCatalogProvider` into `sensor_stream` resource rows, and `ResourceCatalogProvider` supplies additional resource rows such as `transform_edge`. Producers that have live camera calibration can override an auto-lifted sensor row with a `sensor_stream` row carrying `ResourcePinholeIntrinsics`. When the requester asks for embedded registry details and the producer has called `set_registry_app_root(app_root)`, the resources handler attaches the exact Sensor / Frame Registry JSON after hash checks.

`shutdown()` deregisters from Discovery only when this peer is the last member. If a Manager exits while peers remain, the survivors detect the lost peer, elect a successor, rotate the Manager hint in Discovery, and gossip the new Manager peer id with the updated membership snapshot.

## Membership

`ClusterMembership` is the cluster's in-memory membership document:

```rust
pub struct ClusterMembership {
    pub cluster_name: String,
    pub peers: Vec<ClusterMember>,
}

pub struct ClusterMember {
    pub peer_id: libp2p_identity::PeerId,
    pub multiaddrs: Vec<multiaddr::Multiaddr>,
    pub join_ts_ns: i64,
    pub successor_token: Option<Vec<u8>>,
}
```

Admission order is preserved. On Manager loss, election removes the timed-out Manager and chooses the earliest surviving member by `(join_ts_ns, peer_id)`. Followers then watch that selected successor by heartbeat; if the candidate is also dead, its timeout advances the election again. Successor tokens are opaque in v1; Discovery-side verification is deferred.

## Relationship To Other Crates

- [`auki-network`](../auki-network) provides the swarm, protocols, typed stream runtime, and Discovery HTTP client. This crate owns the lifecycle policy using those primitives.
- [`auki-domain-py`](../auki-domain-py) mirrors this crate for Python daemons.
- [`auki-identity`](../auki-identity) supplies wallets; `PeerIdentity` is still derived in `auki-network`.

See [`src/readme.md`](src/readme.md) for implementation detail and [`src/sprint.md`](src/sprint.md) for current work.
