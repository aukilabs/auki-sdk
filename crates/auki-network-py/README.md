# auki-network-py

PyO3 bindings for the Python-facing pieces of [`auki-network`](../auki-network).

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
from auki_network import cluster

def stream_provider(requester_peer_id: str, req: cluster.StreamRequest):
    if req.sensor_id == "head_left_cam":
        async def source():
            async for jpeg in jpeg_fanout.subscribe():
                yield cluster.StreamItem(
                    timestamp_ns=session_clock_now_ns(),
                    payload=cluster.JpegFrame(jpeg),
                )

        return cluster.StreamDecision.accept(
            manifest=cluster.StreamManifest(
                sensor_id=req.sensor_id,
                sensor_hash="...",
                clock_id="...",
                clock_hash="...",
                frame_id="...",
                frame_hash="...",
            ),
            source=source(),
        )

    return cluster.StreamDecision.decline(cluster.DeclineReason.sensor_not_found())
```

The provider signature is `Callable[[str, StreamRequest], StreamDecision]`. The first argument is the requester's libp2p peer id string; producers use it for per-requester policy. Returning or raising anything other than `StreamDecision` is normalized to a typed `DeclineReason.other(...)` so requesters get a failure instead of a hung substream.

Supported payload pyclasses:

- `JpegFrame(bytes)` with `.bytes`
- `PointCloudFrame(bytes)` with `.bytes`
- `JointEncodersFrame(list[float])` with `.angles_rad`
- `AudioFrame(bytes)` with `.data`

Factory methods:

- `StreamDecision.accept(manifest=..., source=...)`
- `StreamDecision.accept_pointcloud(manifest=..., source=...)`
- `StreamDecision.accept_joint_encoders(manifest=..., source=...)`
- `StreamDecision.accept_audio(manifest=..., source=...)`
- `StreamDecision.decline(reason)`

Consumer-side `StreamSubscription`, `StreamEntryIterator`, and `StreamEntry` are returned by `auki_domain.ClusterManager.open_*_stream(...)`.

## Install And Test

```bash
maturin develop -m crates/auki-network-py/Cargo.toml
pytest crates/auki-network-py/python_tests/
cargo test -p auki-network-py
```

The Python module name remains `auki_network`; the Rust library name is `auki_network_py` so sibling PyO3 crates can depend on it without colliding with the upstream Rust crate.

See [`src/readme.md`](src/readme.md) for the implementation map.
