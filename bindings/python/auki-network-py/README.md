# auki-network-py

PyO3 bindings for the Python-facing pieces of [`auki-network`](../../../crates/auki-network).

This crate no longer constructs cluster runtimes. Python daemons create and join clusters through [`auki-domain-py`](../auki-domain-py)'s `ClusterManager`. `auki-network-py` now provides:

- root-level Discovery HTTP client bindings;
- the shared `auki_network.cluster` stream pyclasses used by producer callbacks and consumer stream subscriptions;
- the `_build_stream_provider` PyCapsule bridge that lets sibling PyO3 crates reuse the stream-provider adapter safely.

## Root Surface

```python
import auki_network

client = auki_network.DiscoveryClient("http://discovery.lan:8080")

clusters = client.list_clusters()
outcome = client.create_cluster(
    name="demo",
    manager_peer_id=manager_peer_id,
    manager_multiaddrs=["/ip4/192.168.9.72/tcp/4001"],
)

entry = client.liveness_check("demo", peer_count=2)
entry = client.rotate_manager("demo", manager_peer_id, manager_multiaddrs)
client.deregister("demo")
```

Root classes:

| Class | Notes |
|---|---|
| `DiscoveryClient` | Sync-shaped wrapper around async `auki_network::discovery_client::DiscoveryClient` |
| `ClusterEntry` | `name`, `manager_peer_id`, `manager_multiaddrs`, `peer_count`, `created_ns`, `last_liveness_check_ns` |
| `CreateClusterOutcome` | `is_already_exists` plus `entry` when creation won |

Errors map to built-in Python exceptions: transport failures become `OSError`, Discovery non-2xx statuses become `RuntimeError`, invalid peer ids or multiaddrs become `ValueError`.

## Stream Surface

`auki_network.cluster` owns stream types so `auki-domain-py` and user code agree on one set of PyO3 classes.

Producer-side:

```python
import auki_logs
from auki_network import cluster

camera_log = auki_logs.Log.open("/path/to/head_left_cam.log", camera_manifest)
camera_source = camera_log.stream_source(
    sensor_id="head_left_cam",
    sensor_hash="...",
    clock_id="robot/clock",
    clock_hash="...",
    payload_kind="camera",
    frame_id="robot/head_left_cam_optical",
    frame_hash="...",
)

def stream_provider(requester_peer_id: str, req: cluster.StreamRequest):
    if req.sensor_id == "head_left_cam":
        return cluster.StreamDecision.accept_source(camera_source)
    return cluster.StreamDecision.decline(cluster.DeclineReason.sensor_not_found())
```

The provider signature is `Callable[[str, StreamRequest], StreamDecision]`. The first argument is the requester's libp2p peer id string; producers use it for per-requester policy. Returning or raising anything other than `StreamDecision` is normalized to a typed `DeclineReason.other(...)` so requesters get a failure instead of a hung substream.

For retained sensor logs, `StreamDecision.accept_source(source)` is the recommended producer API. The source is created by `auki_logs.Log.stream_source(...)`; the SDK builds the `StreamManifest`, tails retained log bytes, decodes them according to `payload_kind`, and dispatches internally to the typed stream runtime arms.

Supported payload pyclasses:

- `CameraFrame(frame: bytes, dynamic_intrinsics=None)` with `.frame`
- `PointCloudFrame(bytes)` with `.bytes`
- `JointEncodersFrame(list[float])` with `.angles_rad`
- `AudioFrame(bytes)` with `.data`

Factory methods:

- `StreamDecision.accept_source(source)` for SDK-owned retained logs, recommended for app producers
- `StreamDecision.accept_camera(manifest=..., source=...)`
- `StreamDecision.accept_pointcloud(manifest=..., source=...)`
- `StreamDecision.accept_joint_encoders(manifest=..., source=...)`
- `StreamDecision.accept_audio(manifest=..., source=...)`
- `StreamDecision.decline(reason)`

The typed `accept_*` factories remain available for SDK tests and custom live sources that already have typed async iterators. App producers serving retained logs should prefer `accept_source(source)` so they do not construct stream manifests, decode retained bytes, or pick type-specific factories themselves.

Consumer-side `StreamSubscription`, `StreamEntryIterator`, and `StreamEntry` are returned by `auki_domain.ClusterManager.open_*_stream(...)`.

## Install And Test

```bash
maturin develop -m bindings/python/auki-network-py/Cargo.toml
pytest bindings/python/auki-network-py/python_tests/
cargo test -p auki-network-py
```

The Python module name remains `auki_network`; the Rust library name is `auki_network_py` so sibling PyO3 crates can depend on it without colliding with the upstream Rust crate.

See [`src/readme.md`](src/readme.md) for the implementation map.
