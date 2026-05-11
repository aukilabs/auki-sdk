# Parking lot — auki-domain

Open questions for the auki-domain crate. Cross-cutting questions that involve other crates live in the [root `parking_lot.md`](../../parking_lot.md) or [`crates/parking_lot.md`](../parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](../../CLAUDE.md) for the workflow.

---

## Glossary reconciliation — `Domain ID` vs `{wallet_id}/{name}`

**Resolved 2026-05-11** in PR 1 (T1). [`Glossary.md`](../../Glossary.md) gains a new `Domain Identity` entry alongside the existing `Domain ID` — `Domain Identity = {Domain ID}/{name}` for user-named Domains, just `Vinland` for the reserved singleton. `Domain ID` keeps its existing definition (`hash(domain_owner_pubkey)`) and continues to identify TagClaims; the network-topic / Discovery-indexing role moves to `Domain Identity`. The `"Vinland"` singleton (T12) is the exception — no wallet prefix.

---

## Greenland architectural decisions — pre-filed from the Notion Tasks table

Transcribed from the [Greenland quest page](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2)'s Tasks table so PR 1+ implementing PRs reference these decisions instead of relitigating them. Each block names the Greenland T-number; full task descriptions live in the Notion page (canonical source).

### Decision — Domain identity is wallet-scoped: `{wallet_id}/{name}` (T1)

Decided 2026-05-11 (Nils, Greenland status log). Canonical Domain identity is `{wallet_id}/{name}` for user-named Domains. `wallet_id` derives from the caller's existing wallet credentials. Identity is stable across Manager failover.

**Singleton exception (from T12):** if `name == "Vinland"`, the canonical identity is just `Vinland` (no wallet prefix) — this is the reserved default-Domain namespace headless daemons fall back to, and Discovery serializes its creation to enforce singleton-ness.

Options considered and closed: bare-name-string rejected (collides on shared Discovery); wallet-scoped name chosen; opaque ID + display name deferred; `DomainConfig` struct deferred. Name-string validation: Greenland T9 confirmed any string is accepted in v1.

### Decision — Heartbeat tick interval = 10s, single global default (T2 ← Q2)

Decided 2026-05-11 (Nils, Greenland Q2 resolution). Manager-role nodes send a heartbeat to every cluster member on a 10-second tick. Members respond. Single global default for v1 — no per-cluster policy yet. Tick payload carries liveness only (TimeTransform deferred per Greenland scope).

### Decision — Heartbeat transport over libp2p peer-to-peer (T3 ← Q1)

Decided 2026-05-11 (Nils, Greenland Q1 resolution). Manager↔member heartbeats travel directly between peers over libp2p — they do not pass through Discovery. Non-Manager members rely on the Manager's registry-broadcast snapshots for other-member liveness (they do not observe other members' heartbeats directly). Discovery's role is unchanged: it surfaces registry mutations via the existing SSE channel.

**Implementation note:** PR 2 introduces a new libp2p protocol id sibling to `/auki/stream/0.1.0` and `/auki/message/0.0.1`. Lab-mode versioning per [`auki-labs-repos`](../../CLAUDE.md) Rule 11.5: `/auki/heartbeat/0.0.1`, not `1.0.0`.

### Decision — Mark a member departed after 2 consecutive missed heartbeats (T4 ← Q3)

Decided 2026-05-11 (Nils, Greenland Q3 resolution). Manager removes a member from the Cluster Registry and broadcasts the change after the member fails to respond to 2 consecutive heartbeats. At T2's 10s tick this means a ~20s departure-detection window.

### Decision — JoinRequest fields: peer identity + reachability only (T5 ← Q4)

Decided 2026-05-11 (Nils, Greenland Q4 resolution). JoinRequest validation for v1 requires exactly two fields: the joining peer's `libp2p PeerId` and a `ReachabilityRecord` (the addrs the Manager should dial back on). No wallet credential enforcement, no advertised network/application capabilities, no data products. Same shape for both UI-driven joins (Park) and headless autodiscover joins (Booster, Sentinel).

### Decision — Manager-authoritative registry mutations; wallet is one-shot at creation (T6 ← Q8)

Decided 2026-05-11 (Nils, Greenland Q8 resolution). Registry-mutation authority is held by whoever is currently Manager, identified by peer identity. The wallet's authority is one-shot at Domain creation (T1) — it names the Domain forever, but does not gate ongoing mutations. On failover (T10+), the new Manager's peer identity becomes the new mutation-signing identity; no wallet re-signature is required for the role transfer.

Resolution forced by Q4 (peer-id + reachability only, no wallet creds in JoinRequest) — once wallet enforcement was deferred, Q8 options (A) and (C) collapsed to the same v1 reality and (B) became incoherent.

**Known security caveat:** any peer claiming Manager can write to the registry. Hardening pass (signature-on-mutation, manager-claim verification) is a follow-up quest.

### Decision — Broadcast full ClusterDoc snapshots on every registry change, libp2p Manager→members (T7 ← Q5, Q-disc-1)

Decided 2026-05-11 (Nils, Greenland Q5 + Q-disc-1 resolutions). Every Cluster Registry mutation (join, depart, capability update, endpoint rotation) produces a fresh full `ClusterDoc` snapshot the Manager publishes **directly to cluster members over libp2p** — not through Discovery. No deltas in v1; full snapshot on every mutation.

**Wire shape:** new dedicated libp2p protocol `/auki/registry/0.0.1`, sibling to `/auki/heartbeat/0.0.1` and `/auki/stream/0.1.0` / `/auki/message/0.0.1`. Lab-mode versioning per [`auki-labs-repos`](../../CLAUDE.md) Rule 11.5: `0.0.1`, not `1.0.0`.

**Discovery is not in the live-registry fan-out path.** Discovery's role for the cluster lifecycle is bootstrap rendezvous only (who's the Manager, where new joiners file JoinRequests via `GET /clusters/latest`; T8). Once a peer has joined the cluster, live state — heartbeat ticks (T2/T3/T4) and registry snapshots (T7) — flows peer-to-peer. The earlier framing in the Greenland Notion task description ("Manager publishes to Discovery, which fans it out over the existing SSE channel from PR #84") was design-time vocabulary that was inverted in the Q-disc-1 resolution: layering a parallel Discovery-pushed snapshot stream on top of the already-peer-to-peer heartbeat creates two live-state surfaces. Single surface (libp2p) is cleaner.

**Why a dedicated protocol** instead of multiplexing on `/auki/heartbeat/0.0.1`: cleaner namespacing. Heartbeat is liveness; registry snapshot is state. Future protocol additions (capability ads, etc.) get their own protocol id rather than overloading the heartbeat protocol's name with non-liveness traffic.

Snapshot size at v1's cluster sizes (≤10 peers) is small enough that deltas are premature optimization; revisit once there is a real scale problem.

### Decision — Election rule: oldest cluster member by registry join-time (T10 ← Q10)

Decided 2026-05-11 (Nils, Greenland Q10 resolution). On Manager departure, every surviving member runs the same deterministic rule locally: the member with the **earliest join time in the Cluster Registry** becomes the new Manager. Since the registry is Manager-attested and replicated, all surviving members already agree on join times — no voting round, no quorum, no inbox sync. Tiebreak (identical join timestamps, possible at clock resolution): lower `libp2p PeerId` wins. The newly-elected Manager republishes JoinBundle material so future joiners stop dialing the dead Manager.

### Decision — Sole-survivor election: N=1 quorum (T11 ← Q11)

Decided 2026-05-11 (Nils, Greenland Q11 resolution). Minimum cluster size for a valid election is **1**. A single surviving member takes over as Manager unilaterally. Folds into T10's implementation — no separate code path, just a documented zero-minimum check.

**Trade-off accepted:** a network partition that leaves both halves with ≥1 survivor produces two Managers temporarily (split-brain), reconciled when the partition heals. Same future-hardening bucket as Q8's security caveat.

### Decision — Failover triggers: both graceful announcement and crash detection (T13 ← Q9)

Decided 2026-05-11 (Nils, Greenland Q9 resolution). Manager departure can be detected via two paths, both feeding the same T10/T11 election machinery:

- **Graceful** — Park (or any Manager) publishes a "leaving" message to the cluster on clean shutdown — survivors run T10 immediately on receipt.
- **Crash** — T4's 2-missed-tick timeout fires on Manager unresponsiveness — survivors run T10 once the threshold is crossed.

**Race tolerance:** T10's election rule is deterministic, so both triggers can fire on the same departure event in different orders on different survivors without diverging — every survivor arrives at the same new-Manager peer ID regardless of which signal it acted on.

### Decision — Park UI accepts any string for Domain name in v1 (T9 ← Q12)

Decided 2026-05-11 (Nils, Greenland Q12 resolution). On first boot, Park's UI prompts the user for a domain name, accepts whatever they type (no charset restriction, no length cap, no normalization, no reserved-name check), and passes the string verbatim to `init_domain(name)`. T1's `init_domain(&str)` signature already permits this — no SDK-side validation task because Q12's resolution confirms the lack of validation is deliberate.

### Decision — Headless daemons fall back to default Domain `"Vinland"` (T12 ← Q7)

Decided 2026-05-11 (Nils, Greenland Q7 resolution). On boot, Booster and Sentinel query Discovery via T8 (`GET /clusters/latest`). On 404 (no Domain exists yet), the daemon calls `init_domain("Vinland")` itself, becoming the initial Manager.

**Singleton identity:** `"Vinland"` is a reserved global Domain name — its canonical identity is **just `Vinland`**, not `{wallet_id}/Vinland`. This is a deliberate exception to T1's wallet-scoping rule for user-named Domains. Effect: Discovery serializes — whichever daemon registers first wins; any later daemon hitting `GET /clusters/latest` gets back the existing `Vinland` and joins it (T5) instead of creating a second instance.

### Decision — Discovery endpoint for headless join: `GET /clusters/latest` (T8 ← Q-disc-2)

Decided 2026-05-11 (Nils, Q-disc-2 resolution). The earlier framing in the Greenland Notion task description used `GET /domains/latest` as design-time vocabulary; the actual endpoint Discovery exposes is `GET /clusters/latest` on its existing `/clusters/*` surface. Domain identity (`{wallet_id}/{name}`) is opaque to Discovery — it's just a string passed as `cluster_name`. Discovery is relaxing its existing `cluster_name` regex (`^[A-Za-z0-9.-]+$` → permits `/`) in its own PR to accept the canonical Domain Identity string.

**Implication for SDK code:** none. T1 is already correct (the SDK computes the canonical string client-side and passes it to `DiscoveryClient::register` / `fetch` / `subscribe` as `cluster_name`). When Discovery's endpoint lands, the SDK will consume it via the `DiscoveryClient` surface — no additional wire-shape work on this crate.

**Discovery-side schema dependency (no SDK code change required):** Discovery is adding `created_ns: i64` to its `ClusterDoc` response (server-side timestamp on first upsert of a cluster). The SDK's `ClusterDoc` deserializer in `auki-network/src/cluster_doc.rs` will see a new field on the responses it already deserializes; a tag bump on `auki-network` follows once Discovery's PR lands.

### Decision — Manager-handoff notification to Discovery on election (T14 ← Q14)

Decided 2026-05-11 (Nils, Q14 resolution). On Manager handoff — graceful (T13's "leaving" announcement) or crash-detected (T10's 2-missed-tick timeout) — the newly-elected Manager sends a signed handoff notification to Discovery so late-joiners after a failover route JoinRequests to the live peer rather than the dead one.

**Why Discovery needs to know:** Discovery tracks the current Manager identity per cluster separately from the Domain-creator wallet. Late-joiners hitting `GET /clusters/latest` (T8) get the cluster's current Manager peer-id back along with the ClusterDoc; they route their JoinRequest (T5) to that Manager. Without this notification, a failover-survivor cluster would still advertise the dead Manager's peer-id and new joiners would dial nothing.

**Wire shape (Discovery-side spec, in flight in Discovery's PR):**

- New endpoint, probably `POST /clusters/{name}/manager`.
- Signed JCS payload: `{cluster_name, new_manager_peer_id, op: "manager-handoff", timestamp_ns}`.
- Signed by the **new Manager's peer-derivation key** (same key family as `register` / `deregister` — `Wallet::derive_child("peer/v1")` → `PeerIdentity`).
- Discovery verifier authorizes by checking `new_manager_peer_id` is already a member of that cluster (else how would it be the elected Manager).
- Adds `current_manager_peer_id: Option<PeerId>` to `ClusterDoc` (Discovery's response shape; the SDK's deserializer picks up the new optional field on the next tag bump).

**SDK side (this crate):** authoring + signing the handoff notification is in scope for PR 3 (failover batch — T10 + T11 + T13 + T14). The new Manager's election code (T10/T13) needs to call into a `DiscoveryClient::notify_manager_handoff(wallet, cluster_name, new_manager_peer_id)` helper (to be added in `auki-network` once Discovery's endpoint shape is locked). PR 3's sprint section names T14 alongside the failover triggers.

---
