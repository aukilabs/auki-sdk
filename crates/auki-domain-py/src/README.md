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
| `ClusterTarget` pyclass | shipped |
| `ClusterManager.list_clusters` | shipped |
| `ClusterManager.bootstrap` | shipped |
| `ClusterManager.create_cluster` / `join_cluster` | shipped |
| `ClusterManager` role/membership accessors | shipped |
| `participant_info` / `fetch_participant_info` | shipped |
| `set_sensor_catalog_provider` / `fetch_sensors_catalog` | shipped |
| `open_jpeg_stream` / `open_pointcloud_stream` / `open_joint_encoders_stream` / `open_audio_stream` | shipped |
| `stream_provider` kwarg via `auki_network.cluster._build_stream_provider` PyCapsule | shipped |
| `external_addresses` advertise override | shipped |

## Shape

The wrapper accepts Python-friendly inputs (seed bytes, multiaddr strings, peer-id strings), builds the Rust `PeerIdentity` + swarm internally, resolves advertised multiaddrs, then calls `auki_domain::ClusterManager`.

All async Rust calls are `block_on`ed on a process-wide multi-thread tokio runtime so Python callers keep a synchronous API.

## Deferred

- Typed Python exception hierarchy for every Rust error variant. Current mapping uses `ValueError`, `RuntimeError`, and `OSError`.
- Dedicated relay-reservation helper once the Rust SDK has one.

See [`sprint.md`](sprint.md) for current follow-ups.
