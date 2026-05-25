# auki-network-py

PyO3 bindings for the value-shaped pieces of [`auki-network`](../../../crates/auki-network) — Discovery client + the shared stream pyclasses used across the Python SDK surface. Cluster-runtime construction lives in [`auki-domain-py`](../auki-domain-py).

**Status:** Shipped.

## Public surface

Root module:

- `DiscoveryClient` — HTTP client for the Discovery service, including relay-aware create/rotate helpers.
- `ClusterEntry`, `CreateClusterOutcome`; `ClusterEntry` exposes `relay_multiaddrs`.

`auki_network.cluster` submodule (shared with `auki-domain-py`):

- `StreamRequest`, `StreamManifest`
- Payload pyclasses: `CameraFrame`, `PointCloudFrame`, `JointEncodersFrame`, `AudioFrame`, `SpatialTransformFrame`
- `DeclineReason`, `EndReason`, `StreamItem`, `StreamEntry`, `StreamDecision`, `StreamSubscription`, `StreamEntryIterator`
- Stream exceptions.

## Depends on

- [`auki-network`](../../../crates/auki-network) — Rust crate it wraps.
