# Changelog

Append-only. Latest entry on top.

### Jason + Cursor · Jul 31, 2026

`DiscoveryClient` / `ClusterManager` → peer-manager API (`put`/`get`/`heartbeat`/`deregister` under `/api/v1/domains/{id}/peer-manager`). Global list removed (410); `MostRecentOrCreate` fails closed.

### Jason + Cursor · Jul 31, 2026

`DiscoveryClient` optional `Authorization` on Manager writes; `JoinAuthConfig` / `DomainConfig.discovery_authorization` threaded through create/join/liveness (distinct from `join_authorization`).
