# auki-domain-py

PyO3 bindings for [`auki-domain`](../auki-domain). This is the Python daemon entry point for cluster lifecycle.

Python applications should construct `ClusterManager` here rather than assembling `auki_network.DiscoveryClient`, swarms, and protocol handlers themselves. The wrapper is sync-shaped; it owns a process-wide tokio runtime internally.

## Example

```python
import auki_domain
from auki_network import cluster

daemon = auki_domain.DaemonInfo(
    app="boosterapp",
    name="k1-walker",
    session_id=session_id,
    session_clock_id=clock_id,
    session_clock_hash=clock_hash,
    app_instance=app_instance,
)

target = auki_domain.ClusterTarget.most_recent_or_create("hagall")

manager = auki_domain.ClusterManager.bootstrap(
    target=target,
    wallet_seed=wallet_seed,
    discovery_url="http://discovery.lan:8080",
    listen_addresses=["/ip4/0.0.0.0/tcp/0", "/ip4/0.0.0.0/udp/0/quic-v1"],
    agent_version="boosterapp/0.1",
    daemon_info=daemon,
    stream_provider=stream_provider,       # optional
    external_addresses=None,               # optional operator override
)
manager.set_registry_app_root(app_root)    # serve registry entries to peers

manager.set_resource_catalog_provider(lambda: [
    auki_domain.TransformEdgeResource(
        id="K1-LIVE01/camera_link->K1-LIVE01/head_left_cam_optical",
        from_frame_id="K1-LIVE01/camera_link",
        from_frame_hash=camera_link_frame_hash,
        to_frame_id="K1-LIVE01/head_left_cam_optical",
        to_frame_hash=head_left_frame_hash,
        writer_mode="rigid",
        transform=auki_domain.ResourceSpatialTransform(
            translation=auki_domain.ResourceVec3(0, 0, 0),
            orientation=auki_domain.ResourceQuat(0.5, -0.5, 0.5, -0.5),
        ),
        source_json='{"kind":"ros2_tf"}',
    )
])

def stream_provider(_requester_peer_id, request):
    manifest = auki_domain.StreamManifestBuilder.from_registry(
        app_root,
        request.sensor_id,
        sensor_hash_by_id[request.sensor_id],
        clock_id,
        clock_hash,
    )
    return cluster.StreamDecision.accept_pointcloud(
        manifest=manifest,
        source=pointcloud_source(),
    )

print(manager.cluster_name, manager.local_peer_id, manager.is_manager)
print(manager.participant_info().to_json())
manager.shutdown()
```

## Surface

Value types:

- `ClusterMember(peer_id, multiaddrs, join_ts_ns, successor_token=None)`
- `ClusterMembership(cluster_name)` plus `.from_json(...)`, `.admit(...)`, `.to_json()`
- `DaemonInfo(app, name, session_id, session_clock_id, session_clock_hash, app_instance)`
- `ParticipantInfo` returned by `manager.participant_info()` / `fetch_participant_info(...)`
- `SensorEntry(sensor_id, sensor_hash, kind, sensor_entry_json=None, frame_entry_json=None)`
- Resource catalog values: `ResourcePinholeIntrinsics`, `ResourceVec3`, `ResourceQuat`, `ResourceSpatialTransform`, `SensorStreamResource`, `TransformEdgeResource`
- `ClusterTarget.create(name)`, `.join(name)`, `.join_or_create(name)`, `.most_recent_or_create(fallback_name)`
- `StreamManifestBuilder.from_registry(app_root, sensor_id, sensor_hash, clock_id, clock_hash)`

Static manager entry points:

- `ClusterManager.list_clusters(discovery_url) -> list[auki_network.ClusterEntry]`
- `ClusterManager.bootstrap(target, wallet_seed, discovery_url, listen_addresses, agent_version, daemon_info, stream_provider=None, external_addresses=None)`
- `ClusterManager.create_cluster(...)`
- `ClusterManager.join_cluster(...)`

Manager instance methods/properties:

- `.cluster_name`, `.local_peer_id`, `.is_manager`, `.manager_peer_id`, `.peer_count`
- `.membership()`
- `.admit_peer(peer_id, multiaddrs)`
- `.participant_info()`
- `.fetch_participant_info(peer_id)`
- `.set_sensor_catalog_provider(callable)`
- `.set_resource_catalog_provider(callable)`
- `.set_registry_app_root(app_root)`
- `.fetch_sensors_catalog(peer_id, include_registry_entries=False, include_frame_entries=False)`
- `.fetch_resources_catalog(peer_id, kinds=None, include_sensor_entries=False, include_frame_entries=False)`
- `.fetch_sensor_entry(peer_id, sensor_id, sensor_hash) -> str`
- `.fetch_clock_entry(peer_id, clock_id, clock_hash) -> str`
- `.fetch_frame_entry(peer_id, frame_id, frame_hash) -> str`
- `.open_stream(peer_id, sensor_id)` — recommended generic consumer opener. The SDK resolves the remote resource catalog row and returns a `StreamSubscription` whose entries carry the existing typed payload pyclasses (`CameraFrame`, `PointCloudFrame`, `JointEncodersFrame`, or `AudioFrame`).
- `.open_camera_stream(peer_id, sensor_id)`
- `.open_pointcloud_stream(peer_id, sensor_id)`
- `.open_joint_encoders_stream(peer_id, sensor_id)`
- `.open_audio_stream(peer_id, sensor_id)`
- `.shutdown()`

`stream_provider` uses the `auki_network.cluster` stream types. Its callable signature is `(requester_peer_id: str, request: StreamRequest) -> StreamDecision`.

`StreamManifestBuilder.from_registry(...)` returns an `auki_network.cluster.StreamManifest` constructed by the `auki_network` module itself, so it can be passed directly to `cluster.StreamDecision.accept_camera(...)`, `.accept_pointcloud(...)`, `.accept_joint_encoders(...)`, or `.accept_audio(...)`. Spatial sensors get `frame_id` + `frame_hash` from the local Sensor Registry entry and verify the exact Frame Registry entry exists; audio and joint encoders return empty frame fields.

`external_addresses` has replace semantics: if provided and non-empty, those exact multiaddrs are advertised to Discovery instead of auto-detected listen addresses.

## Notes

- Discovery is mandatory for cluster bootstrap.
- `ClusterManager` computes dynamic `ParticipantInfo` fields; daemons pass only static `DaemonInfo` at construction.
- Producer daemons should call `set_registry_app_root(app_root)` so peers can resolve hash-pinned Sensor / Clock / Frame registry entries over libp2p.
- Producer daemons should use `set_resource_catalog_provider(callable)` for transform edges and richer resource rows. Sensor streams are auto-lifted from `set_sensor_catalog_provider`, but a resource provider can override the auto-lifted `sensor_stream` row by returning the same `id`.
- Consumers can either call the exact registry fetch helpers, or ask `fetch_sensors_catalog(..., include_registry_entries=True, include_frame_entries=True)` for embedded Sensor / Frame Registry JSON when reducing round trips matters.
- New consumers should prefer `fetch_resources_catalog(...)` for live stream and transform-edge discovery; `/auki/sensors/0.0.1` remains available for the older sensor-only view.
- New stream consumers should call `open_stream(peer_id, sensor_id)`. The binding resolves the remote `sensor_stream` resource inside the SDK and delegates to the matching typed subscription internally. The older `open_camera_stream`, `open_pointcloud_stream`, `open_joint_encoders_stream`, and `open_audio_stream` methods remain for compatibility and SDK-internal use.
- Producer stream providers should use `StreamManifestBuilder.from_registry(...)` instead of hand-filling stream manifests.
- `shutdown()` is the explicit leave path. It deregisters only if this peer is the last member; otherwise surviving peers elect a successor.
- Stream classes live in `auki-network-py` so user callbacks and this wrapper share one PyO3 type registry.

## Build And Test

```bash
cargo test -p auki-domain-py
maturin develop -m crates/auki-domain-py/Cargo.toml
pytest crates/auki-domain-py/python_tests/
```

See [`src/README.md`](src/README.md) for the implementation map.
