# auki-domain

The SDK's network-presence layer. An app that wants its peer and session visible in a cluster calls `Domain::join(&peer, &session, DomainConfig)` — this crate composes the two [`auki-session`](../auki-session) halves, bootstraps a `ClusterManager`, and serves the session's resource catalog to remote peers. Post-#282 the dependency points **from** `auki-domain` **to** `auki-session`, not the other way around.

**Status:** Shipped. App-facing via `Domain`; `ClusterManager` is the engine underneath.

## Public surface

### Domain — the app entry point

- `Domain::join(&Peer, &Session, DomainConfig)` (async) — bootstraps the cluster per `config.target`, wraps the session's logs + peer's registries in a live catalog bridge (`SessionHandle`), and stamps the session identity into `DaemonInfo`:
  - **Identity guard (#284):** rejects with `DomainError::IdentityMismatch` if `peer.peer_id() != config.local_identity.peer_id()` — the registered session clock and the cluster's runtime clock must belong to the same peer.
  - **Clock stamping (#284):** `daemon_info.session_id` / `session_clock_id` / `session_clock_hash` are overwritten from the session's auto-minted monotonic clock; apps pass placeholders.
- `Domain::catalog()` → `Vec<ResourceEntry>` — the rows currently served over `/auki/resources/0.2.0`.
- `Domain::cluster_manager()` → `&ClusterManager` — escape hatch to the engine (membership, Manager state, domain clock estimates, participant info, stream opens).
- `Domain::leave()` (async) — clean shutdown of the cluster presence.
- `catalog_of(&Peer, &Session)` → `Vec<ResourceEntry>` — pure helper, no network; builds exactly the rows `Domain` would serve. Useful for tests and dry runs.

`DomainConfig` fields: `target: ClusterTarget`, `local_identity: PeerIdentity`, `local_multiaddrs: Vec<Multiaddr>`, `discovery_url: String`, `swarm: Swarm<Behaviour>`, `stream_provider: StreamProvider`, `daemon_info: DaemonInfo`.

### ClusterManager — the engine

`ClusterManager` handles Discovery + cluster bootstrap: list / create / join / bootstrap (policy-driven via `ClusterTarget`), membership, Manager election + rotation, Discovery liveness checks, relay hint preservation, participant info, resource catalog serving (reads from the `SessionHandle` installed by `Domain::join`, or a `ResourceCatalogProvider` fallback), hash-pinned registry-entry fetch, typed stream opens, domain clock estimates, and clean shutdown. `SessionHandle` is defined in `auki-network` to avoid a dependency cycle.

Peers can join the cluster before their resource catalog is ready. The resources handler answers each inbound `/auki/resources/0.2.0` request with a fresh snapshot from the registered `ResourceCatalogProvider`, or from `SessionHandle::catalog()` when no provider is installed. Producers should only return resources that can currently accept stream opens; unavailable resources are omitted until they become requestable again.

### Also exported

- `ClusterManager`, `ClusterTarget`, `DaemonInfo`, `ResourceCatalogProvider`, `elect_successor(...)`, `LIVENESS_CHECK_INTERVAL`, `DiagnosticMessage` / `InboundDiagnosticMessage`, `DiscoveryClusterEntry`
- Error types: `AdmitError`, `BootstrapError`, `CreateClusterError`, `JoinClusterError`, `DiscoveryClientError`, `FetchParticipantInfoError`, `FetchRegistryEntryError`, `FetchResourcesCatalogError`, `DomainClockEstimateUnavailable`, `DomainTimeNowError`
- `ClusterMembership`, `ClusterMember`
- `StreamManifestBuilder` (+ `BuildStreamManifestError`)
- Re-exports: `SessionHandle`, `RegistryKind`, `ResourceEntry` / `ResourcesRequest` / `ResourcesResponse` (from `auki-network`); `ClockTransformEstimate` / `DomainClockEstimate` (from `auki-time`); `SensorRegistryEntry` / `ClockRegistryEntry` / `FrameRegistryEntry` (from `auki-registry`)

## Depends on

- [`auki-session`](../auki-session) — `Peer` + `Session`, composed by `Domain`. This is the #282 inversion: the session layer knows nothing about domains.
- [`auki-network`](../auki-network) — libp2p, peer protocols, Discovery client, and the `SessionHandle` trait.
- [`auki-identity`](../auki-identity) — Wallet → PeerId derivation.
- [`auki-hash`](../auki-hash), [`auki-jcs`](../auki-jcs) (optional) — canonical membership / cluster docs.
- [`auki-registry`](../auki-registry) (optional) — hash-pinned registry-entry fetch.
- [`auki-time`](../auki-time) (optional) — clock-stamped membership / heartbeat, clock-sync estimates.
