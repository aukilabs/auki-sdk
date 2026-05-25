# auki-domain-py

PyO3 bindings for [`auki-domain`](../../../crates/auki-domain) — the Python daemon facade for `ClusterManager`. A Python daemon (Booster's K1 sidecar, Sentinel, Park tooling) becomes a cluster peer through this package.

**Status:** Shipped.

## Public surface

- `ClusterTarget` — list-existing / create-new / join-by-id.
- `ClusterManager.list_clusters(...)`, `ClusterManager.bootstrap(...)`, `ClusterManager.create_cluster(...)`, `ClusterManager.create_cluster_with_relay_multiaddrs(...)`, `ClusterManager.join_cluster(...)`.
- `participant_info`, peer info fetches, resource / sensor catalog fetches.
- Registry serving root registration.
- `StreamManifestBuilder.from_registry(...)` — registry-backed manifest construction.
- Stream provider wiring + typed stream openers (`open_camera_stream`, `open_point_cloud_stream`, `open_joint_encoders_stream`, `open_audio_stream`).
- `external_addresses` advertisement override and separate `relay_multiaddrs` Discovery hints for browser-dialable Domain Relays.

## Depends on

- [`auki-domain`](../../../crates/auki-domain) — Rust crate it wraps.
- [`auki-network-py`](../auki-network-py) — for the shared `auki_network.cluster` stream pyclasses.
