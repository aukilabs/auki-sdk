# auki-domain

Internal cluster lifecycle layer. Apps do not construct `ClusterManager` directly — they use `Session::join_domain` in [`auki-session`](../auki-session), which bootstraps the `ClusterManager` and wires it a `SessionHandle` so the cluster can serve catalog rows to remote peers.

`ClusterManager` handles Discovery + cluster bootstrap: list / create / join / bootstrap, membership, Manager election, Discovery liveness checks, relay hint preservation, participant info, resource catalog serving (reads from a `SessionHandle: Send + Sync`), registry-entry fetch, stream open, and clean shutdown. `SessionHandle` is defined in `auki-network` to avoid a dependency cycle.

Peers can join the cluster before their resource catalog is ready. The
resources handler answers each inbound `/auki/resources/0.2.0` request with a
fresh snapshot from the registered `ResourceCatalogProvider`, or from
`SessionHandle::catalog()` when no provider is installed. Producers should only
return resources that can currently accept stream opens; unavailable resources
are omitted until they become requestable again.

**Status:** Shipped. Internal to `auki-session` for app use.

## Public surface (consumed by `auki-session`)

- `ClusterManager` + `ClusterTarget`
- Relay-aware cluster creation, Manager relay reservation, and Manager-rotation hint preservation
- `ClusterMembership`, `ClusterMember`, `DaemonInfo`
- `ResourceCatalogProvider` + `SessionHandle`-based catalog snapshot
- `SensorCatalogProvider`, `SensorEntry`, `SensorKind`, `SensorsResponse`
- Manager / election / bootstrap error types
- `LIVENESS_CHECK_INTERVAL`, `elect_successor(...)`

## Depends on

- [`auki-identity`](../auki-identity) — for Wallet → PeerId derivation.
- [`auki-network`](../auki-network) — for libp2p, peer protocols, Discovery client, and `SessionHandle` trait.
- [`auki-hash`](../auki-hash) (optional), [`auki-jcs`](../auki-jcs) (optional) — for canonical membership / cluster docs.
- [`auki-registry`](../auki-registry) (optional) — for hash-pinned registry-entry fetch.
- [`auki-time`](../auki-time) (optional) — for clock-stamped membership / heartbeat.
