# auki-network-py

PyO3 bindings for the value-shaped pieces of [`auki-network`](../../../crates/auki-network) — Discovery client + the shared stream pyclasses used across the Python SDK surface. Cluster-runtime construction lives in [`auki-domain-py`](../auki-domain-py).

**Status:** Shipped.

## Public surface

Root module:

- `DiscoveryClient` — HTTP client for the Discovery service, including relay-aware create/rotate helpers.
- `ClusterEntry`, `CreateClusterOutcome`; `ClusterEntry` exposes `relay_multiaddrs`.

`auki_network.cluster` submodule (shared with `auki-domain-py`):

- `StreamRequest`, `StreamManifest`
- Payload pyclasses: `CameraFrame`, `PointCloudFrame`, `JointEncodersFrame`, `AudioFrame`, `ScalarFrame`, `SpatialTransformFrame`, `MapUpdateFrame`
- Scalar stream helper: `StreamDecision.accept_scalar(manifest=..., source=...)`; retained Scalar Sensor Logs use `StreamDecision.accept_source(...)` with `payload_kind="scalar"`.
- Pose stream helpers: `StreamManifest.pose_stream(...)` and `StreamDecision.accept_pose(...)`
- Map stream helpers: `StreamManifest.map_stream(...)`, `StreamDecision.accept_map(...)`, and retained Map Log dispatch through `StreamDecision.accept_source(...)`
- `DeclineReason`, `EndReason`, `StreamItem`, `StreamEntry`, `StreamDecision`, `StreamSubscription`, `StreamEntryIterator`
- Stream exceptions.

## Depends on

- [`auki-network`](../../../crates/auki-network) — Rust crate it wraps.
