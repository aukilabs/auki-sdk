# Sprint — auki-network-py

Current work and next steps to close the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now

This crate is no longer the Python cluster-runtime constructor. Python daemons bootstrap clusters through [`auki-domain-py`](../../auki-domain-py)'s `ClusterManager`. The current `auki_network` module owns two surfaces:

- **Root Discovery client.** `DiscoveryClient`, `ClusterEntry`, and `CreateClusterOutcome` wrap the `auki_network::discovery_client` API. Methods are sync-shaped and block on the crate's process-wide tokio runtime.
- **`auki_network.cluster` stream types.** The cluster submodule now contains the shared stream pyclasses and exceptions used by both user callbacks and `auki-domain-py` stream subscriptions.

Root-level Discovery methods:

| Python method | Rust method | Purpose |
|---|---|---|
| `list_clusters()` | `DiscoveryClient::list_clusters` | Read Discovery's cluster directory |
| `create_cluster(name, manager_peer_id, manager_multiaddrs)` | `DiscoveryClient::create_cluster` | Atomic create with Manager hint |
| `liveness_check(name, peer_count)` | `DiscoveryClient::liveness_check` | Manager liveness push |
| `rotate_manager(name, manager_peer_id, manager_multiaddrs)` | `DiscoveryClient::rotate_manager` | Publish a new Manager hint |
| `deregister(name)` | `DiscoveryClient::deregister` | Graceful cluster removal |

Current stream payload pyclasses:

- `JpegFrame(bytes)`
- `PointCloudFrame(point_count, data)`
- `JointEncodersFrame(list[float])`
- `AudioFrame(bytes)`

Producer-side callbacks use:

```python
def stream_provider(requester_peer_id: str, request: cluster.StreamRequest) -> cluster.StreamDecision:
    ...
```

The hidden `_build_stream_provider` helper returns a named `PyCapsule` containing the Rust `StreamProvider`. `auki-domain-py` imports that helper so callback adaptation happens inside the `auki_network` extension module, where the stream pyclass type ids match user-created `StreamDecision` instances.

Tests currently cover the root Discovery surface, stream type registration, stream-provider bridge, and Python-side stream callback behavior.

## Next

In priority order:

1. **Type stubs.** Add `auki_network.pyi` for root Discovery classes and the `cluster` stream submodule. This is the biggest usability win for Python daemon authors.

2. **Keep payload parity with `auki-network`.** When Rust adds a new `StreamDispatch` payload variant, add the Python pyclass, producer factory, and consumer extraction path in the same release.

3. **Discovery error classes.** The current wrapper maps failures to built-in exceptions. If Python consumers need branchable non-2xx handling, reintroduce typed exceptions around the new Discovery v1 shape.

## Smaller Follow-Ups

- `pyo3-log` or `tracing` to Python `logging` integration if downstream wants unified logging.
- PyPI publication and wheel matrix once the Python SDK surface settles.
- `__version__` attribute for conventional package introspection.

## Open Items

See [`parking_lot.md`](../parking_lot.md). Nothing blocks current `ClusterManager` consumers.

## Out Of Scope

- Cluster lifecycle construction; use `auki-domain-py`.
- Async public methods; this crate keeps a sync-shaped API and owns the tokio runtime internally.
- Re-exporting `auki-identity-py`; consumers import identity and network packages separately.
