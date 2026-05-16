# Sprint - auki-domain

Current work and next steps for the Hagall cluster-lifecycle layer.

## Current

`ClusterManager` is now the single SDK entry point for Discovery + cluster lifecycle:

- Apps declare intent with `ClusterTarget`.
- `ClusterManager::bootstrap` owns list/decide/create-or-join for headless daemons.
- `create_cluster` and `join_cluster` take a Discovery URL, not a caller-built `DiscoveryClient`.
- The manager owns Discovery liveness checks, join handling, membership gossip, peer liveness/election, info/catalog/registry request handlers, and stream openings.

The code has moved past the old Greenland `DomainIdentity` / `init_domain` plan. Any docs or downstream code still mentioning `DomainHandle`, `init_domain`, `ClusterRuntime`, `cluster.json`, or Discovery SSE membership refresh are stale.

## Next

- Demote or narrow direct `auki_network::discovery_client::DiscoveryClient` app usage after Park and Boosterapp confirm the SDK-fronted path in live deployments.
- Pin the v2 successor-token format and Discovery verification path. v1 keeps `successor_token` opaque and accepts trust-by-shape for the demo.
- Add SDK-side relay-reservation support once LAN-only Hagall flows are stable and Park-from-home earns the work.
- Keep `ClusterManager` and `auki-domain-py` APIs aligned as Python consumers adopt `ClusterTarget.bootstrap`.
- Add catalog bundling only if the extra registry-fetch roundtrip becomes operationally annoying; v0 registry exchange already resolves exact entries by `(kind, id, hash)`.

## Decisions To Honor

- Discovery is a bootstrap/directory service; cluster membership is owned by the current Manager and gossiped peer-to-peer.
- Manager -> Discovery liveness check is HTTP `/clusters/{name}/liveness` every 1 second.
- Peer-side Manager-death detection uses libp2p `/auki/heartbeat/0.0.1`.
- Graceful and ungraceful Manager exits use the same survivor election path. A Manager only deregisters the cluster when it is the last member.
- Election chooses the earliest reachable member by `(join_ts_ns, peer_id)`.
- App daemons should talk to `ClusterManager`, not manually compose Discovery + network runtime.

## Open Items

See [`../parking_lot.md`](../parking_lot.md) for live design questions.
