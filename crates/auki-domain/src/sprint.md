# Sprint - auki-domain

Current work and next steps for the Hagall cluster-lifecycle layer.

## Current

`ClusterManager` is now the single SDK entry point for Discovery + cluster lifecycle:

- Apps declare intent with `ClusterTarget`.
- `ClusterManager::bootstrap` owns list/decide/create-or-join for headless daemons.
- `create_cluster` and `join_cluster` take a Discovery URL, not a caller-built `DiscoveryClient`.
- The manager owns Discovery liveness checks, join handling, membership gossip, peer liveness/election, info/catalog/registry request handlers, and stream openings.
- `ParticipantInfo.session_now_ns`, `session_clock_id`, and `session_clock_hash` are sourced from the SDK-owned `auki_time::SessionClock`, not caller-built `DaemonInfo` clock fields.
- The crate follows the SDK binding standard. Direct Rust behavior lives in `core.rs`, native Python/Swift UniFFI adapters live in `ffi.rs`, and browser-safe wasm-bindgen helpers live in `wasm.rs`.
- Generated Python and Swift expose a bounded `DomainClusterManager` facade covering cluster lifecycle, manager admission, membership inspection, participant info, domain time, clock estimates, diagnostics, catalog/resource/registry providers, catalog/resource/registry fetches, and camera/detection byte streams. Browser JavaScript exposes membership/election helpers, domain DTO validators, and an `AukiDomainClient` facade that composes the `auki-network` browser transport for request/response flows; there is no browser `ClusterManager` runtime.
- Generated-language smoke coverage now exercises the generated Python, Swift, and JavaScript/Wasm packages directly, including native manager bootstrap, membership JSON, participant info, catalog fetches, and browser `AukiDomainClient` request/response composition.

The code has moved past the old Greenland `DomainIdentity` / `init_domain` plan. Any docs or downstream code still mentioning `DomainHandle`, `init_domain`, `ClusterRuntime`, `cluster.json`, or Discovery SSE membership refresh are stale.

## Next

- Demote or narrow direct `auki_network::discovery_client::DiscoveryClient` app usage after Park and Boosterapp confirm the SDK-fronted path in live deployments.
- Pin the v2 successor-token format and Discovery verification path. v1 keeps `successor_token` opaque and accepts trust-by-shape for the demo.
- Add SDK-side relay-reservation support once LAN-only Hagall flows are stable and Park-from-home earns the work.
- Migrate Python consumers from the legacy PyO3 wrapper to the generated UniFFI `DomainClusterManager` facade where that bounded surface is enough.
- Keep the sensor-catalog detail path thin: default catalog fetches stay lightweight, while `SensorsRequest::with_frame_entries()` is the opt-in path for Park-style consumers that want Sensor / Frame Registry JSON embedded by value.
- Keep expanding heartbeat time sync through the shared `SessionClock` foundation; do not add a parallel heartbeat-specific clock identity.

## Decisions To Honor

- Discovery is a bootstrap/directory service; cluster membership is owned by the current Manager and gossiped peer-to-peer.
- Manager -> Discovery liveness check is HTTP `/clusters/{name}/liveness` every 1 second.
- Peer-side Manager-death detection uses libp2p `/auki/heartbeat/0.0.1`.
- Graceful and ungraceful Manager exits use the same survivor election path. A Manager only deregisters the cluster when it is the last member.
- Election chooses the earliest reachable member by `(join_ts_ns, peer_id)`.
- App daemons should talk to `ClusterManager`, not manually compose Discovery + network runtime.
- Stream providers should build accept manifests with `StreamManifestBuilder::from_registry` so spatial sensors commit to the exact `FrameRegistryEntry` hash declared by their sensor body.
- SDK-minted session clock ids are peer-id rooted (`<peer_id>/<session_id>/monotonic`); the old machine-id clock naming convention is stale for new domain-owned participant info.

## Open Items

See [`../parking_lot.md`](../parking_lot.md) for live design questions.
