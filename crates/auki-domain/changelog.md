# Changelog — auki-domain

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

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
