# auki-domain

App-facing cluster lifecycle layer. `ClusterManager` is the single SDK entry point for Discovery + cluster bootstrap: list / create / join / bootstrap, membership, Manager election, Discovery liveness checks, relay hint preservation, participant info, resource catalogs, transform edges, pose streams, registry-entry fetch, stream open, and clean shutdown.

A daemon (Booster, Sentinel, Park) becomes a cluster peer through this crate. Higher-level than [`auki-network`](../auki-network), which it composes on top of.

**Status:** Shipped.

## Public surface

- `ClusterManager` + `ClusterTarget`
- Relay-aware cluster creation and Manager-rotation hint preservation
- `ClusterMembership`, `ClusterMember`, `DaemonInfo`
- `ResourceCatalogProvider`, `ResourceEntry`, `SensorStreamResource`, `TransformEdgeResource`, `PoseStreamResource`, `ResourcePinholeIntrinsics`, `ResourcesRequest`, `ResourcesResponse`
- `ResourcesRequest::sensor_streams()` and `ResourcesRequest::pose_streams()` helpers for catalog discovery
- `SensorCatalogProvider`, `SensorEntry`, `SensorsResponse`
- Manager / election / bootstrap error types
- `LIVENESS_CHECK_INTERVAL`, `elect_successor(...)`

## Depends on

- [`auki-identity`](../auki-identity) — for Wallet → PeerId derivation.
- [`auki-network`](../auki-network) — for libp2p, peer protocols, Discovery client.
- [`auki-hash`](../auki-hash) (optional), [`auki-jcs`](../auki-jcs) (optional) — for canonical membership / cluster docs.
- [`auki-registry`](../auki-registry) (optional) — for hash-pinned registry-entry fetch.
- [`auki-time`](../auki-time) (optional) — for clock-stamped membership / heartbeat.
