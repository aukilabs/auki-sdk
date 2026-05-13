# Changelog — auki-domain

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's claude · May 13, 14:30 HKT, 2026

**Bug fix: graceful Manager shutdown no longer deregisters the cluster when other peers exist.** Discovered when Booster surfaced "Manager leaving cluster causes cluster to close, no new manager is elected." The old `shutdown()` unconditionally `deregister`ed Discovery if we were the Manager. With multiple peers, the surviving peer's libp2p ConnectionClosed handler fired Lost → election → `rotate_manager` — but the cluster was already gone from Discovery (`404`). Per the Hagall design ("graceful and ungraceful Manager exits are the same code path — peers detect the loss + run the election + rotate"), the Manager should NOT deregister on graceful exit when other peers can take over. Fix: `shutdown()` checks `membership.peers.len() <= 1` (we're the last member) before deregistering; otherwise leaves the cluster alive for the survivors' handoff. New integration test `manager_graceful_shutdown_passes_cluster_to_surviving_peer` is a regression test for exactly this scenario. Existing tests adjusted: `cluster_manager_full_lifecycle_against_live_discovery` (admitted a fake peer, so `shutdown` now correctly skips deregister — added an explicit `deregister` for test cleanup); `two_managers_create_then_join_against_live_discovery` (added 500ms sleep between B.shutdown and A.shutdown so A's liveness handler has time to evict B before A's own shutdown checks `peers.len()`). All 4 live tests pass against `192.168.9.130:8080`.

### Nils's claude · May 13, 13:00 HKT, 2026

**SDK-T6 + SDK-T7 — cluster-internal election + Manager-handoff orchestration. Failover works end-to-end.** Three new pieces:

- **`elect_successor(membership, local_peer_id, connected) -> Option<PeerId>`** — pure function. Sorts membership by `(join_ts_ns, peer_id)` ascending; returns the earliest-joined peer that's "reachable" (in `connected` or equal to `local_peer_id`). 5 unit tests pin the rule (earliest-joined wins, unreachable earlier peers skipped, peer-id tie-break, local-alone-wins, empty-membership-returns-None).
- **`spawn_liveness_handler`** — task that drains `PeerLivenessEvent`s from the network runtime. On `Lost { peer_id: lost }`: if `lost` is the Manager and we're not, run the election; if we win, become Manager (update local state, call `discovery.rotate_manager`, spawn the Manager-side Discovery heartbeat tick, evict the dead Manager from membership + push the updated allow-list). If we're the Manager and a peer was lost, evict + push allow-list. Dedupe per disconnection.
- **`ClusterManager::heartbeat_task`** is now `Arc<Mutex<Option<JoinHandle<()>>>>` so the liveness handler can spawn it on Manager-promotion. `join_cluster` initializes it to `None` (joiner isn't Manager yet); `create_cluster` initializes it `Some` (creator is the initial Manager).

End-to-end live integration test (3rd in the file): A creates cluster `foo`; B `join_cluster`s it; A is `drop`'d without `shutdown` (unclean exit, simulates a process kill); within 5s, B detects loss via the heartbeat-timeout monitor, runs the election (B is the only reachable peer, so B wins), promotes itself, calls `discovery.rotate_manager`, and starts the Manager heartbeat tick. The test verifies B's `is_manager == true`, `manager_peer_id == B`, AND that Discovery's directory snapshot has rotated to B. All three live integration tests pass against `192.168.9.130:8080`: full-lifecycle, two-managers-join, and the new failover scenario.

`elect_successor` re-exported from `auki-domain`'s public surface for downstream testing.

### Nils's claude · May 13, 12:30 HKT, 2026

### Nils's claude · May 13, 12:30 HKT, 2026

**SDK-T3 — `ClusterManager::join_cluster` ships; Manager-side join handler task admits + gossips membership.** Two new pieces wire `auki-network`'s `/auki/join/0.0.1` protocol into a usable end-to-end flow:

- **`ClusterManager::join_cluster(name, identity, multiaddrs, discovery, swarm, stream_provider)`** — looks the cluster up in Discovery, spawns the runtime with the Manager pre-allowed for dial, waits for the libp2p connection to establish (up to 10s), opens a `/auki/join/0.0.1` substream, sends the `JoinRequest`, parses the Manager's `JoinResponse::Accept { membership_json, successor_token }`, expands the runtime's allow-list to cover every peer in the Manager-gossiped membership, returns a `ClusterManager` with `is_manager = false` and `manager_peer_id` pointing at the Manager. On `Reject` returns `JoinClusterError::Rejected(reason)`.
- **Manager-side join handler task** (`spawn_join_handler`) — drains the `JoinEvent` channel returned by `NetworkRuntime::spawn`, decides admit-or-reject. As Manager: appends the new peer to the membership, builds the updated allow-list, pushes it via `NetworkRuntimeHandle::set_allowed_peers`, replies with `Accept { membership_json, successor_token }`. As non-Manager: always replies with `Reject { reason: "not the manager" }`. Duplicate admits return `Reject { reason: "already a member" }`. The task is spawned by both `create_cluster` and `join_cluster`; cancelled on `shutdown`.

Live integration test (2-peer): peer A `create_cluster`s; peer B `join_cluster`s the same cluster; both verify identical membership (Manager + joiner); peer B sees A as its Manager; both shutdown cleanly and Discovery's entry is gone. Passes against the running deployment at `192.168.9.130:8080`.

Python: `auki_domain.ClusterManager.join_cluster(wallet_seed, cluster_name, discovery_url, listen_addresses, agent_version)` mirrors the Rust API. Same daemon-friendly façade as `create_cluster`. Booster / Park can now `import auki_domain; mgr = auki_domain.ClusterManager.join_cluster(...)` and get a fully-populated handle ready for `/api/info`.

Deps: `serde_json` promoted from dev-dep to runtime dep (the Manager-side handler serializes the membership; the join-side parses the Manager-gossiped membership). New error type `JoinClusterError` with 6 variants: `Discovery`, `NotFound(name)`, `SendJoin(SendJoinRequestError)`, `Rejected(reason)`, `InvalidMembership(serde_json::Error)`, `Runtime(SpawnError)`.

### Nils's claude · May 13, 11:45 HKT, 2026

**SDK-T2 lands — `ClusterManager` ships with `create_cluster` + `admit_peer` + `participant_info` + Manager-side Discovery heartbeat.** New `cluster_manager` module. `ClusterManager` owns the cluster's `ClusterMembership`, the libp2p `NetworkRuntime` (from `auki-network`), the `DiscoveryClient`, and a Manager-side Discovery heartbeat tick (3s cadence — matches the v1 contract's 10s sweep). Surface:

- `ClusterManager::create_cluster(name, identity, multiaddrs, discovery, swarm, stream_provider)` — atomic create on Discovery, initialize membership with self as the sole member, spawn the runtime, spawn the heartbeat tick. Returns the handle.
- Accessors: `cluster_name`, `local_peer_id`, `is_manager`, `manager_peer_id`, `membership`, `peer_count`.
- `admit_peer(peer_id, multiaddrs) -> ClusterMember` — Manager-only; appends to membership, pushes the updated allow-list to the runtime. Duplicate admit returns `AdmitError::AlreadyMember`. Non-Manager admit returns `AdmitError::NotManager { cluster, manager }`. v1 successor token is empty bytes (signature verification disabled per Discovery v1 contract); SDK-T4 swaps in a real signed token.
- `participant_info(daemon_info) -> ParticipantInfo` — builds the `/api/info` JSON shape with cluster-aware fields (`is_manager`, `manager_peer_id`, `peer_id`) populated by the SDK; daemon supplies its own identity fields via `DaemonInfo`. Per BA-Q3.
- `shutdown(self)` — cancels the heartbeat tick, deregisters from Discovery (if we're the Manager), shuts down the runtime.

End-to-end live integration test against the running Discovery at `192.168.9.130:8080` verifies the full lifecycle: create → accessors → participant_info shape → admit_peer + duplicate rejection → heartbeat keeps Discovery's entry alive past the 10s sweep window (test waits 12s and asserts the cluster is still there) → shutdown deregisters cleanly. Two unit tests + 1 integration test (ignored by default).

**Not in this commit** (deferred to follow-up PRs): the libp2p join protocol (SDK-T3 — needed for `join_cluster`), peer-side heartbeat (SDK-T5), cluster-internal election (SDK-T6), Manager-handoff orchestration (SDK-T7), signed successor tokens (SDK-T4, blocked on SDK-Q3), anti-entropy / reconciliation / last-writer-wins (SDK-Q5 deeper convergence). `join_cluster` as a method is not yet exposed — only `create_cluster` works end-to-end.

New deps: `tokio` (was dev-only) elevated to a runtime dep for the heartbeat tick (`rt`/`time`/`macros` features); `libp2p` + `futures` added to dev-deps for the integration test (waits for the swarm's OS-chosen listen port before construction).

### Nils's claude · May 13, 09:45 HKT, 2026

**Hagall SDK-T1 — `ClusterMembership` type + serde lands.** New `cluster_membership` module: `ClusterMembership { cluster_name, peers: Vec<ClusterMember> }` carries the cluster's authoritative membership document. `ClusterMember` fields = `peer_id: PeerId`, `multiaddrs: Vec<Multiaddr>`, `join_ts_ns: i64`, `successor_token: Option<Vec<u8>>` (opaque per SDK-Q3 still being open; v1 Discovery contract skips signature verification entirely, so empty bytes are fine for the demo). serde JSON with the same Multiaddr-as-string adapter as `auki-network`'s `ClusterDoc`. `ClusterMembership::filename()` returns `<cluster_name>.json` — the wire/disk filename per Hagall convention; the `foo.json` of the Hagall demo cluster is exactly that, no special-casing. `ClusterMembership::admit(member)` appends in admission order and returns the index. 9 unit tests cover round-trip (with peers, empty cluster, member-without-token, empty-multiaddrs), peer-order preservation, filename derivation, and a wire-shape-locked test that pins JSON key names against rename. 1 new doctest. **Greenland's `ClusterDoc` is untouched** — per SDK-Q1's resolution (replace), the deletion of `init_domain` / `init_or_join_domain` / Greenland-era types lands in a follow-up breaking PR once Hagall is functional end-to-end. New deps: `libp2p-identity` (with `serde` feature for canonical `PeerId` strings), `serde` (derive), and `serde_json` (dev). All 21 existing tests still pass; 0 new clippy warnings.

### broodsugar's claude · May 13, HKT, 2026

**`init_or_join_domain` added — race-loss collapsed into the happy path.** Sibling to `init_domain`; same arg shape, different semantics. `init_domain` returns `Err(AlreadyExists)` when Discovery's atomic `create_cluster` 409s — the caller learns Manager-vs-joiner role and can branch. `init_or_join_domain` collapses both outcomes into a "I just want into this Domain" success path: whichever peer wins `create_cluster`, the caller registers against the resulting cluster and builds the runtime exactly once. The swarm is consumed exactly once regardless of which `CreateClusterOutcome` variant fires, so there's no race window in which the swarm would need rebuilding.

Targeted at producer-only daemons (BoosterApp, Sentinel) that don't care about Manager identity today — Greenland's Manager-role state (T2+T3+T4+T6+T7) is still stubbed, so the create-vs-join distinction doesn't affect functional behaviour. Daemons that need the discrimination later (failover trigger, JoinRequest admission, etc.) continue calling `init_domain` and branching on `AlreadyExists`. Two public entry points cleanly separate the use-cases.

Implementation is a thin variant of `init_domain` — same DomainIdentity derivation, same register + from_swarm sequence; only `create_cluster`'s `Outcome::AlreadyExists` branch is `_ = ...`'d instead of returning `Err`. No new error variant; the same `InitDomainError::{Discovery, RuntimeSpawn}` cases apply (the third — `AlreadyExists` — is unreachable from this function by construction). ~40 LOC added; existing `init_domain` unchanged.

### broodsugar's claude · May 13, 11:21 HKT, 2026

**`init_domain` becomes the canonical (and only sanctioned) public `ClusterRuntime` constructor.** Pairs with the [`auki-network` PR B](../auki-network/changelog.md) that killed `cluster.json` and made `ClusterRuntime::from_swarm` `#[doc(hidden)] pub`. Together they close every bypass: peers only visible within their cluster, no fallback, no Discovery-less path.

**Signature change.** `init_domain` now takes `swarm: Swarm<Behaviour>`, `participant_provider: ParticipantInfoProvider`, `stream_provider: StreamProvider` in addition to the previous args, and returns `DomainHandle { identity: DomainIdentity, runtime: ClusterRuntime }` with both fields public. The `ClusterDoc` Discovery's `register` returns never leaves the SDK — it goes straight into `ClusterRuntime::from_swarm`, whose `apply_initial_doc` step populates the libp2p allow-list before the event loop starts. Park's bypass-init_domain shortcut (which existed because the old `init_domain` discarded the `ClusterDoc`) becomes unnecessary.

**`DomainHandle` now owns the runtime.** Was a thin wrapper around `DomainIdentity` only; now `pub struct DomainHandle { pub identity: DomainIdentity, pub runtime: ClusterRuntime }`. Daemons feed `discovery.subscribe(&cluster_name)` events into `handle.runtime.update_cluster_doc(new_doc)` themselves — the runtime doesn't yet own its SSE subscription (filed in `auki-network/parking_lot.md` as a tightening follow-up).

**`InitDomainError::RuntimeSpawn(SpawnError)`** variant added for the post-register failure path. Discovery-side calls (`create_cluster`, `register`) have already succeeded by the time the runtime is constructed, so a `RuntimeSpawn` failure means the cluster is created and the peer is registered but the runtime didn't construct — caller may need to deregister before retrying.

**`Cargo.toml` dep change.** `auki-network` feature set gains `"swarm"` (previously had `"discovery_client"` only). Needed to name `Swarm<Behaviour>` and pass it to `ClusterRuntime::from_swarm`.

**Tests** — 12 unit tests + 1 doctest pass with `--all-features`. (The old `init_domain` doctest's signature changed; the unchanged tests cover `DomainIdentity` shape.)

**Daemon-side migration:** Park / BoosterApp / Sentinel daemons each need a per-repo PR to migrate from the old `init_domain(wallet, name, discovery, addresses, ...)` to the new `init_domain(wallet, name, discovery, swarm, addresses, ..., participant_provider, stream_provider)`. Park's PR #36 bypass path (which called `DiscoveryClient::register` directly) goes away — `init_domain` now does everything Park needs.

### broodsugar's claude · May 11, 17:46 HKT, 2026

**`init_domain` now creates the cluster before registering — `InitDomainError::AlreadyExists` variant landed.** Pairs with the [`auki-network::DiscoveryClient::create_cluster`](../auki-network/changelog.md) addition shipped in the same PR. Closes the rollout-breakage the SDK agent surfaced: Discovery's new T8 deployment ([aukilabs/discovery#2](https://github.com/aukilabs/discovery/pull/2) merged) removed lazy-create on `POST /clusters/{name}/peers`, so any SDK consumer that called `register` against a fresh cluster would 404 against the new Discovery.

**Call sequence inside `init_domain`** is now:

1. Build `DomainIdentity` from `wallet` + `name` (Vinland singleton handled as before).
2. `DiscoveryClient::create_cluster(wallet, &cluster_name)` — signed JCS over `{ cluster_name, op: "create", peer_id, public_key, timestamp_ns }`. The signer is recorded by Discovery as the initial Manager (`ClusterDoc.current_manager_peer_id`).
3. On `CreateClusterOutcome::Created` — fall through to step 4.
4. On `CreateClusterOutcome::AlreadyExists { existing }` — return `InitDomainError::AlreadyExists { identity, existing }` so the caller can route to the Vinland-race fall-back-to-join branch in Greenland T12.
5. `DiscoveryClient::register` — unchanged.

**`InitDomainError::AlreadyExists`** carries the local caller's `DomainIdentity` plus the winner's full `ClusterDoc` (parsed from Discovery's `{ error: "already_exists", existing: ClusterDoc }` 409 body). The winner's `current_manager_peer_id` is the live Manager the loser should route a `JoinRequest` at once Greenland T5 lands. Lifts the variant out of the PR 3 deferral noted in the prior schema PR — Discovery now produces a real 409 to map.

**Glossary / Notion alignment.** The Notion T1 description called for `existing: DomainIdentity` on the variant; this PR ships `existing: ClusterDoc` instead — strictly more information (the cluster doc carries the winning Manager peer-id + creation timestamp + peer list), the identity is recoverable from `existing.cluster_name`. Caller-side ergonomics win.

No new `auki-domain` tests beyond what the existing 12 cover; the wire-shape and signing tests live in `auki-network::discovery_client` (6 new tests there). `cargo test -p auki-domain --all-features` 12/12 + 2 doctests green.

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
