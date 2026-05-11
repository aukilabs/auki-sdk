# Changelog — auki-domain

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's dobby · May 11, 16:10 HKT, 2026

**Greenland PR 2b — Manager-role state machine (T2 + T4 + T6 + T7 logic).** Lands `auki-domain::manager` — a transport-agnostic state machine for the Manager role, tested in isolation without standing up a real swarm. Builds on PR 2a's wire layer (`/auki/heartbeat/0.0.1` and `/auki/registry/0.0.1` definitions in `auki-network`).

- **`Manager`** — owns the authoritative in-memory Cluster Registry for one Domain. Holds the Manager's own peer/addresses/app-id/note plus a `HashMap<PeerId, MemberEntry>` of tracked Members. Mutation API: `add_member`, `remove_member`, `tick`, `record_response`, `record_failure`.
- **T2 — tick cadence.** Caller drives `tick(now_ns, sink)` at `HEARTBEAT_INTERVAL` (10 s, tunable via `ManagerConfig` for tests). Each tick emits one `SendHeartbeat` effect per still-alive Member carrying `HeartbeatRequest { tick_ns: now_ns, manager_peer_id: self }`.
- **T4 — departure detection.** Per-Member `consecutive_missed_ticks` increments on every tick before dispatch; resets to 0 on every matching `record_response`. At `MISSED_TICKS_FOR_DEPARTURE` (2) the Member is removed. Multiple departures in one tick are coalesced into ONE post-tick snapshot.
- **T6 — Manager-authoritative registry.** Add/remove/tick are the only paths that mutate state; each mutation bumps `last_mutation_ns` (caller-supplied monotonic clock). The Manager itself is NOT in the `members` map — it appears in outgoing snapshots through `self_*` fields.
- **T7 — mutation-driven snapshot broadcast.** Every mutation emits exactly one `BroadcastSnapshot` effect carrying a fresh `SnapshotEnvelope { mutation_ns, doc: ClusterDoc }`. Snapshot peers are sorted by `PeerId` so the JSON wire form is stable for the same logical state — important for future signature-over-JSON receivers.
- **Transport-agnostic via `EffectSink`.** `Manager` emits `ManagerEffect`s (`SendHeartbeat` / `BroadcastSnapshot` / `MemberDeparted`) into a sink the caller provides. `MockEffectSink` in tests collects effects for assertions; a swarm-backed transport (PR 2c / PR 3) translates each effect into the corresponding libp2p call. The state machine is `!Send`-friendly — single-task driver, no internal locks.
- **16 unit tests in `manager::tests`** — covers idempotent add, no-op remove of unknown, coalesced batch departure, miss-counter reset on response, deterministic emit order, sorted snapshot peers, manager's own addresses in snapshot, mutation_ns monotonicity. All passing (28 total in `auki-domain` lib including PR 1's identity tests).
- **Deferred to PR 2c (or absorbed into PR 3):** the `ClusterRuntime`-backed transport that bridges `ManagerEffect`s onto real libp2p calls, plus the `Member` consumer that applies inbound snapshots via `ClusterRuntime::update_cluster_doc` and responds to heartbeats. PR 2b's Manager is verified by test in isolation.

`Cargo.toml` adds: `auki-network` now opts into the `swarm` feature (needed for `heartbeat_protocol`, `registry_protocol`, and `cluster_doc::SUPPORTED_VERSION` exports); `libp2p-identity` direct dep matching the workspace pin (`0.2`, `["ed25519", "peerid", "serde"]`).

Stack: lands on top of PR 2a (#92). PR 2a must merge first.

---

### broodsugar's dobby · May 11, 14:48 HKT, 2026

**Greenland design corrections — T7 inverted to libp2p, T8 endpoint name, T14 added.** Three updates landed on the [Greenland Notion page](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2) earlier today; this doc-only PR transcribes them into the local parking-lot and sprint plan ahead of PR 2 (heartbeat batch).

- **T7 (broadcast envelope) — Discovery is OUT of the live-registry fan-out path.** Manager publishes full `ClusterDoc` snapshots **directly to cluster members over libp2p** via a new dedicated protocol `/auki/registry/0.0.1` (sibling to `/auki/heartbeat/0.0.1`). The earlier framing — "Manager publishes to Discovery, which fans out over the existing SSE channel from PR #84" — is inverted (Q-disc-1 resolution). Rationale: heartbeat is already peer-to-peer per T3; layering a parallel Discovery-pushed snapshot stream creates two live-state surfaces. Single libp2p surface is cleaner. PR 2's T7 wire change is now SDK-only — no Discovery SSE plumbing required.
- **T8 (Discovery endpoint for headless join) — `GET /clusters/latest`, not `/domains/latest`.** Q-disc-2 resolution: Domain is the topic the cluster forms around — `cluster_name` IS the Domain identifier. SDK code unchanged (already passes the canonical Domain Identity string as `cluster_name`); the `/domains/*` noun in the original task description was design-time vocabulary. Discovery is relaxing its `cluster_name` regex to accept `/` in its own PR. Plus a Discovery-side schema add `created_ns: i64` to `ClusterDoc` (server-side timestamp); SDK picks it up on the next `auki-network` tag bump.
- **T14 (new task) — Manager-handoff notification to Discovery on election.** Promoted from Q14, resolved same session. On Manager handoff (graceful T13 or crash-detected T10), the newly-elected Manager sends a signed JCS payload to Discovery (probably `POST /clusters/{name}/manager`) so late-joiners hitting `GET /clusters/latest` route their JoinRequests to the live Manager peer-id rather than the dead one. Signed by the new Manager's `peer/v1` derivation key. Discovery's `ClusterDoc` response grows `current_manager_peer_id: Option<PeerId>`. T14 slots into PR 3 (failover batch) alongside T10/T11/T13.

Files touched: [`crates/auki-domain/parking_lot.md`](parking_lot.md) (T7 + T8 + T12 decision-block rewrites; new T14 decision block); [`crates/auki-domain/src/sprint.md`](src/sprint.md) (PR 2 + PR 3 section updates; "Architectural decisions to honor" bullet list refreshed).

### broodsugar's dobby · May 11, 14:34 HKT, 2026

**Greenland T1 — `DomainIdentity` + `init_domain` shipped.** First implementing PR of the [Greenland quest](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2). New `DomainIdentity` value type carrying the wallet-scoped canonical string `{wallet_id}/{name}` with the reserved `"Vinland"` singleton exception (T12). `init_domain(&Wallet, &str, &DiscoveryClient, &[Multiaddr], Option<&str>, Option<&str>)` builds the identity, calls `DiscoveryClient::register`, returns a minimal `DomainHandle{identity}` (Manager-role state grows in PR 2). 12 unit tests + 2 doctests + 2 locked cross-language conformance vectors (user-named string structure, singleton string). Glossary updated: new `Domain Identity` entry alongside the existing `Domain ID`; `Domain ID` keeps its existing definition for TagClaims, `Domain Identity` is the network-topic / Discovery-indexing string. Resolves the Glossary-reconciliation parking-lot question filed in PR 0.

### broodsugar's dobby · May 11, 14:20 HKT, 2026

**Crate scaffolded — [`auki-domain`](../auki-domain).** Greenland PR 0 lays the home for SDK-side Domain lifecycle (creation, joining, Manager/Member roles, heartbeats, live Cluster Registry, failover). No functional code — empty `lib.rs`. Lands the folder convention (`Cargo.toml`, `README.md`, `parking_lot.md`, `changelog.md`, `src/readme.md`, `src/sprint.md`), workspace registration in root `Cargo.toml`, and parking-lot pre-files of every Greenland architectural decision transcribed from the [Notion Tasks table](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2). PR 1 (T1: `DomainIdentity` + `init_domain`) lands next, per [`src/sprint.md`](src/sprint.md).
