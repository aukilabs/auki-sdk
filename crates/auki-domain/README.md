# auki-domain

The SDK's network-presence layer. An app that wants its peer and session visible in a cluster calls `Domain::join(&peer, &session, DomainConfig)` — this crate composes the two [`auki-session`](../auki-session) halves, bootstraps a `ClusterManager`, and serves the session's resource catalog to remote peers. Post-#282 the dependency points **from** `auki-domain` **to** `auki-session`, not the other way around.

**Status:** Shipped. App-facing via `Domain`; `ClusterManager` is the engine underneath.

## Public surface

### Domain — the app entry point

- `Domain::join(&Peer, &Session, DomainConfig)` (async) — bootstraps the cluster per `config.target`, wraps the session's logs + peer's registries in a live catalog bridge (`SessionHandle`), and stamps the session identity into `DaemonInfo`:
  - **Identity guard (#284):** before bootstrap or Discovery I/O, rejects unless the supplied `Peer`, originating `Session`, configured `PeerIdentity`, and pre-built swarm local peer id are identical.
  - **Clock stamping (#284):** `daemon_info.session_id` / `session_clock_id` / `session_clock_hash` are overwritten from the session's auto-minted monotonic clock; apps pass placeholders.
- `Domain::catalog()` → `Vec<ResourceEntry>` — the rows currently served over `/auki/resources/0.2.0`.
- `DomainBuilder::new(&Peer, &Session, DomainConfig).message_channel(row, capacity).join()` — composes receiver-owned live message channels before join. It validates that each row owner is the joining `Peer`, that the clock `RegistryRef` exactly matches a clock registered in the supplied `Session` (peer/id/hash), and rejects duplicate owner/resource-id pairs before binding the v0.3 catalog row, bounded receiver, and `NetworkRuntime` registration together.
- `Domain::take_message_channel_receiver(resource_id)` → `MessageChannelReceiver` — hands the application the declared bounded async receiver. Each `MessageEvent` carries the exact channel Resource, authenticated sender `PeerId`, opaque type string, `timestamp_ns`, and opaque payload. The receiver owns registration lifetime: dropping it removes the v0.3 row and closes channel endpoints; Domain leave/drop also closes a retained receiver.
- `Domain::fetch_resources_catalog_v3[_with](peer, ...)` — explicitly fetches `/auki/resources/0.3.0`; an unsupported peer returns `FetchResourcesCatalogV3Error::UnsupportedProtocol` and there is no silent fallback to v0.2. `Domain::fetch_resources_catalog(peer)` remains the unchanged v0.2 fetch.
- `Domain::open_message_channel(peer, &row)` — verifies the discovered row owner equals the authenticated serving peer and returns a persistent `MessageChannelSender`. `Domain::send_message(...)` is the open/send-once convenience.
- `Domain::cluster_manager()` → `&ClusterManager` — escape hatch to the engine (membership, Manager state, domain clock estimates, participant info, stream opens).
- `Domain::leave()` (async) — clean shutdown of the cluster presence. Non-Managers best-effort notify the Manager over `/auki/leave/0.0.1` (wait ≤2s for Ack) so membership can shrink immediately; then local teardown. Crash/partition still uses heartbeat loss timeout.
- `catalog_of(&Peer, &Session)` → `Vec<ResourceEntry>` — pure helper, no network; builds exactly the rows `Domain` would serve. Useful for tests and dry runs.

`DomainConfig` fields: `target: ClusterTarget`, `local_identity: PeerIdentity`, `local_multiaddrs: Vec<Multiaddr>`, `discovery_url: String`, `swarm: Swarm<Behaviour>`, `stream_provider: StreamProvider`, `daemon_info: DaemonInfo`.

Typed messaging is live and ephemeral. Send success is only a transport ACK
after the receiver runtime queues the event; it is not application acceptance.
If queueing succeeds but the ACK is lost, send returns an error even though the
event may already have been delivered. Callers must treat that result as
indeterminate and must not automatically retry.
There is no history, persistence, retry, replay, queue across disconnect,
materialization, or SDK interpretation of type/payload/timestamp. The channel
Resource carries an existing clock `RegistryRef`; each message carries only
`timestamp_ns`. Applications use existing clock declarations and time
transforms and own freshness, scheduling, and action policy.

Any current Noise-authenticated cluster member may send in this milestone.
`NetworkRuntime` membership is the coarse trust boundary; there is no generic
channel-level ACL. Unknown and removed peers cannot deliver payloads to the
application receiver.

### ClusterManager — the engine

`ClusterManager` handles Discovery + cluster bootstrap: list / create / join / bootstrap (policy-driven via `ClusterTarget`), membership, Manager election + rotation, Discovery liveness checks, relay hint preservation, participant info, v0.2 and v0.3 resource catalog serving/fetching, hash-pinned registry-entry fetch, typed stream and message-channel opens, domain clock estimates, and clean shutdown. `SessionHandle` is defined in `auki-network` to avoid a dependency cycle.

### Manager arbitration

Discovery's cluster row is the tiebreak authority for the Manager role: a peer
holds the role only while the row names it. Election only nominates — a
successful `rotate_manager` (or row re-create after a sweep) commits. A
follower that loses Manager heartbeats consults Discovery before electing:
**defer + rejoin** while the row still names the lost Manager, **follow +
rejoin** when the row names someone else, **elect** only when the row was
swept. A Manager whose liveness response names another peer steps down and
rejoins it. Re-joining as a current member is idempotent (multiaddrs refresh;
`join_ts_ns` — and therefore election order — unchanged). A follower watches
its current Manager even when it is not in the local membership document, so
displaced or evicted peers keep retrying instead of stranding. Discovery
unreachable means no election can commit; the cluster runs headless on the
data plane until Discovery returns. Dropping a `ClusterManager` without
`shutdown()` aborts its background tasks, so a dead handle cannot keep the
row fresh.

Peers can join the cluster before their resource catalog is ready. The resources handler answers each inbound `/auki/resources/0.2.0` request with a fresh snapshot from the registered `ResourceCatalogProvider`, or from `SessionHandle::catalog()` when no provider is installed. Producers should only return resources that can currently accept stream opens; unavailable resources are omitted until they become requestable again.

### Also exported

- `ClusterManager`, `ClusterTarget`, `DaemonInfo`, `ResourceCatalogProvider`, `elect_successor(...)`, `LIVENESS_CHECK_INTERVAL`, `DiagnosticMessage` / `InboundDiagnosticMessage`, `DiscoveryClusterEntry`
- Error types: `AdmitError`, `BootstrapError`, `CreateClusterError`, `JoinClusterError`, `DiscoveryClientError`, `FetchParticipantInfoError`, `FetchRegistryEntryError`, `FetchResourcesCatalogError`, `FetchResourcesCatalogV3Error`, `DomainBuilderError`, `DomainOpenMessageChannelError`, `DomainSendMessageError`, `DomainClockEstimateUnavailable`, `DomainTimeNowError`
- `ClusterMembership`, `ClusterMember`
- `StreamManifestBuilder` (+ `BuildStreamManifestError`)
- Re-exports: `SessionHandle`, `RegistryKind`, v0.2 `ResourceEntry` / `ResourcesRequest` / `ResourcesResponse`, v0.3 `ResourceEntryV3` / `ResourceVariantV3` / `ResourcesRequestV3` / `ResourcesResponseV3`, `MessageChannelResource` / `MessageChannelSender` (from `auki-network`); `ClockTransformEstimate` / `DomainClockEstimate` (from `auki-time`); `SensorRegistryEntry` / `ClockRegistryEntry` / `FrameRegistryEntry` (from `auki-registry`)

## Depends on

- [`auki-session`](../auki-session) — `Peer` + `Session`, composed by `Domain`. This is the #282 inversion: the session layer knows nothing about domains.
- [`auki-network`](../auki-network) — libp2p, peer protocols, Discovery client, and the `SessionHandle` trait.
- [`auki-identity`](../auki-identity) — Wallet → PeerId derivation.
- [`auki-hash`](../auki-hash), [`auki-jcs`](../auki-jcs) (optional) — canonical membership / cluster docs.
- [`auki-registry`](../auki-registry) (optional) — hash-pinned registry-entry fetch.
- [`auki-time`](../auki-time) (optional) — clock-stamped membership / heartbeat, clock-sync estimates.
