# `auki-domain-py/src` - implementation status

## Files

- [`lib.rs`](lib.rs) - single-file PyO3 binding for `auki-domain`.
- [`sprint.md`](sprint.md) - current follow-ups.

## Implemented

| Feature | Status |
|---|---|
| `ClusterMember` / `ClusterMembership` pyclasses | shipped |
| `DaemonInfo` / `ParticipantInfo` pyclasses | shipped |
| `SensorEntry` pyclass | shipped |
| Resource catalog pyclasses | shipped |
| `StreamManifestBuilder.from_registry` | shipped |
| `ClusterTarget` pyclass | shipped |
| `ClusterManager.list_clusters` | shipped |
| `ClusterManager.bootstrap` | shipped |
| `ClusterManager.create_cluster` / `join_cluster` | shipped |
| `ClusterManager` role/membership accessors | shipped |
| `participant_info` / `fetch_participant_info` | shipped |
| `set_sensor_catalog_provider` / `fetch_sensors_catalog` | shipped |
| `set_resource_catalog_provider` / `fetch_resources_catalog` | shipped |
| `set_registry_app_root` | shipped |
| `fetch_sensor_entry` / `fetch_clock_entry` / `fetch_frame_entry` JSON helpers | shipped |
| `open_jpeg_stream` / `open_pointcloud_stream` / `open_joint_encoders_stream` / `open_audio_stream` | shipped |
| `stream_provider` kwarg via `auki_network.cluster._build_stream_provider` PyCapsule | shipped |
| `external_addresses` advertise override | shipped |

## Shape

The wrapper accepts Python-friendly inputs (seed bytes, multiaddr strings, peer-id strings), builds the Rust `PeerIdentity` + swarm internally, resolves advertised multiaddrs, then calls `auki_domain::ClusterManager`.

All async Rust calls are `block_on`ed on a process-wide multi-thread tokio runtime so Python callers keep a synchronous API.

Producer daemons can call `ClusterManager.set_registry_app_root(app_root)` with a string or `os.PathLike` so the Rust manager serves existing registry entries from `<app_root>/registries/{sensors,clocks,frames}/...` over `/auki/registries/0.0.1`. The same root powers the optional `ClusterManager.fetch_sensors_catalog(peer_id, include_registry_entries=True, include_frame_entries=True)` detail path, which fills `SensorEntry.sensor_entry_json` and `SensorEntry.frame_entry_json` when the producer can resolve the exact hashes locally.

Producer daemons can call `ClusterManager.set_resource_catalog_provider(callable)` with a zero-argument callable returning `list[SensorStreamResource | TransformEdgeResource]`. `ClusterManager.fetch_resources_catalog(peer_id, kinds=None, include_sensor_entries=False, include_frame_entries=False)` returns the same concrete value objects to consumers. `ResourcePinholeIntrinsics`, `ResourceVec3`, `ResourceQuat`, and `ResourceSpatialTransform` cover the nested numeric payloads.

Python consumers can call `ClusterManager.fetch_sensor_entry(...)`, `.fetch_clock_entry(...)`, and `.fetch_frame_entry(...)` for exact hash-addressed registry entries. These methods return canonical JSON strings and map missing entries to `FileNotFoundError`.

Producer stream providers can call `StreamManifestBuilder.from_registry(app_root, sensor_id, sensor_hash, clock_id, clock_hash)` to get an `auki_network.cluster.StreamManifest` backed by the local registry. The binding deliberately instantiates the object through the imported `auki_network` module so the returned manifest has the same PyO3 type identity that `auki_network.cluster.StreamDecision.accept_*` expects.

## Deferred

- Typed Python exception hierarchy for every Rust error variant. Current mapping uses `ValueError`, `RuntimeError`, `OSError`, and `FileNotFoundError`.
- Dedicated relay-reservation helper once the Rust SDK has one.

See [`sprint.md`](sprint.md) for current follow-ups.
