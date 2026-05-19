# `auki-network-py/src/`

Implementation status for [`auki-network-py`](../README.md).

## Files

- [`lib.rs`](lib.rs) - module entry point, root Discovery pyclasses, shared tokio runtime, error mapping, and module population.
- [`stream_types.rs`](stream_types.rs) - `auki_network.cluster` stream pyclasses, producer adapter, PyCapsule bridge, stream exceptions, and subscription/entry iterator wrappers.
- [`stream_bridge.rs`](stream_bridge.rs) - Python async-iterator to Rust `futures::Stream` bridge used by producer-side stream sources.

## Public Python Surface

Root module:

```python
auki_network.DiscoveryClient(base_url: str)
auki_network.ClusterEntry
auki_network.CreateClusterOutcome
```

`DiscoveryClient` methods:

```python
client.list_clusters() -> list[ClusterEntry]
client.create_cluster(name: str, manager_peer_id: str, manager_multiaddrs: list[str]) -> CreateClusterOutcome
client.liveness_check(name: str, peer_count: int) -> ClusterEntry
client.rotate_manager(name: str, manager_peer_id: str, manager_multiaddrs: list[str]) -> ClusterEntry
client.deregister(name: str) -> None
```

`auki_network.cluster`:

```python
StreamRequest(sensor_id=...)
StreamManifest(sensor_id=..., sensor_hash=..., clock_id=..., clock_hash=..., frame_id=..., frame_hash=...)
CameraFrame(frame: bytes, dynamic_intrinsics=None)
PointCloudFrame(bytes)
JointEncodersFrame(angles_rad)
AudioFrame(data)
StreamItem(timestamp_ns=..., payload=...)
StreamEntry
DeclineReason.sensor_not_found()
DeclineReason.sensor_unavailable()
DeclineReason.producer_shutting_down()
DeclineReason.other(detail=...)
EndReason.source_ended()
EndReason.producer_shutting_down()
EndReason.session_ended()
EndReason.producer_error(detail=...)
StreamDecision.accept_camera(manifest=..., source=...)
StreamDecision.accept_pointcloud(manifest=..., source=...)
StreamDecision.accept_joint_encoders(manifest=..., source=...)
StreamDecision.accept_audio(manifest=..., source=...)
StreamDecision.decline(reason)
StreamSubscription.manifest
StreamSubscription.entries()
StreamEntryIterator
StreamEndOfStream
StreamConnectionLost
StreamProtocolError
StreamDeclined
StreamUnreachable
_build_stream_provider(callable) -> PyCapsule
```

There is intentionally no `cluster.spawn`, `cluster.load_doc`, `ClusterRuntime`, `ClusterDoc`, `PeerSnapshot`, or Python `DiscoveryClient.register/fetch` surface.

## Rust Mapping

| Python | Rust |
|---|---|
| `DiscoveryClient` | `auki_network::discovery_client::DiscoveryClient` |
| `ClusterEntry` | `auki_network::discovery_client::ClusterEntry` |
| `CreateClusterOutcome` | `auki_network::discovery_client::CreateClusterOutcome` |
| stream pyclasses | `auki_network::{stream_protocol, stream_runtime}` types |
| `_build_stream_provider` | `stream_types::build_stream_provider` wrapped in a named `PyCapsule` |

`cluster_tokio_runtime()` is a process-wide multi-thread tokio runtime used for Discovery calls and blocking frame iteration. The Python API stays synchronous.

## Verification

```bash
cargo test -p auki-network-py
maturin develop -m crates/auki-network-py/Cargo.toml
pytest crates/auki-network-py/python_tests/
```
