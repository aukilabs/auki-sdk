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
- `SensorEntry(sensor_id, sensor_hash, kind)`
- `ClusterTarget.create(name)`, `.join(name)`, `.join_or_create(name)`, `.most_recent_or_create(fallback_name)`

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
- `.fetch_sensors_catalog(peer_id)`
- `.open_jpeg_stream(peer_id, sensor_id)`
- `.open_pointcloud_stream(peer_id, sensor_id)`
- `.open_joint_encoders_stream(peer_id, sensor_id)`
- `.open_audio_stream(peer_id, sensor_id)`
- `.shutdown()`

`stream_provider` uses the `auki_network.cluster` stream types. Its callable signature is `(requester_peer_id: str, request: StreamRequest) -> StreamDecision`.

`external_addresses` has replace semantics: if provided and non-empty, those exact multiaddrs are advertised to Discovery instead of auto-detected listen addresses.

## Notes

- Discovery is mandatory for cluster bootstrap.
- `ClusterManager` computes dynamic `ParticipantInfo` fields; daemons pass only static `DaemonInfo` at construction.
- `shutdown()` is the explicit leave path. It deregisters only if this peer is the last member; otherwise surviving peers elect a successor.
- Stream classes live in `auki-network-py` so user callbacks and this wrapper share one PyO3 type registry.

## Build And Test

```bash
cargo test -p auki-domain-py
maturin develop -m crates/auki-domain-py/Cargo.toml
pytest crates/auki-domain-py/python_tests/
```

See [`src/README.md`](src/README.md) for the implementation map.
