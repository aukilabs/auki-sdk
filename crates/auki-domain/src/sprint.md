# Sprint — auki-domain

Current work and next steps. This crate is being scaffolded as the home for the [Greenland quest](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2)'s SDK-side work.

## Now (PR 1 — Greenland T1: shipped)

Greenland T1 entry point. Landed:

- `DomainIdentity { wallet_id: Option<WalletId>, name: String }` value type with:
  - Canonical string form `{wallet_id}/{name}` for user-named Domains, just `Vinland` for the singleton.
  - `user_named(&Wallet, &str)` constructor (panics on reserved `"Vinland"` to redirect callers to `singleton()`).
  - `singleton()` constructor for the reserved `"Vinland"` Domain.
  - `canonical_string()` accessor producing the string Discovery indexes on.
  - `Display`, `PartialEq`, `Eq`, `Hash`, `Clone`, `Debug` derived/implemented.
  - 12 unit tests + 2 doctests + 2 locked cross-language conformance vectors (user-named string structure + full canonical concat against seed `[3u8; 32]`; singleton string).
- `init_domain(&Wallet, &str, &DiscoveryClient, &[Multiaddr], Option<&str>, Option<&str>) -> Result<DomainHandle, InitDomainError>`:
  - Constructs the identity (singleton if `name == "Vinland"`, user-named otherwise).
  - Calls `DiscoveryClient::register(wallet, &canonical_string, addresses, expected_app_id, note)`.
  - Returns `DomainHandle { identity }`. Role state stubbed for PR 2.
- Glossary update: new `Domain Identity` entry (the network-topic / Discovery-indexing string); existing `Domain ID = hash(domain_owner_pubkey)` keeps its TagClaim definition.

## Now (PR 2a — Greenland T3 + T7 wire layer: shipped, open at #92)

Wire-protocol definitions for the heartbeat + registry batch. Landed in `auki-network`:

- `/auki/heartbeat/0.0.1` (T3 wire) — `request_response::json::Behaviour` with `HeartbeatRequest { tick_ns, manager_peer_id }` / `HeartbeatResponse { responder_peer_id }`. Wired into the composed `Behaviour` in `swarm.rs` so the next batch (PR 2b) can drive it.
- `/auki/registry/0.0.1` (T7 wire) — `libp2p_stream` substream-per-message framing helpers (`write_envelope` / `read_envelope`) with `SnapshotEnvelope { mutation_ns, doc: ClusterDoc }`. `MAX_FRAME_BYTES = 1 MiB`. Generic multiplexer, no swarm-level wiring needed.
- 14 unit/integration tests passing. Build clean.

Pure wire. No state-machine logic. The Manager role lives in PR 2b.

## Now (PR 2b — Greenland T2 + T4 + T6 + T7 logic: in-progress)

Manager-role state machine in `auki-domain::manager`. Lands:

- T2 — `Manager::tick(now_ns, sink)` driven by the caller's timer at `HEARTBEAT_INTERVAL` (10 s) cadence. Emits one `SendHeartbeat` effect per still-alive member.
- T4 — Per-member `consecutive_missed_ticks`; departure at `MISSED_TICKS_FOR_DEPARTURE` (2) consecutive misses. Coalesced single `BroadcastSnapshot` per tick regardless of how many members depart together.
- T6 — Authoritative in-memory Cluster Registry holding `ClusterPeer` entries keyed by `PeerId`. `add_member` / `remove_member` are the mutation API; both bump `last_mutation_ns` and emit a snapshot.
- T7 — Mutation-driven (not periodic) `BroadcastSnapshot` carrying the full `ClusterDoc` over the PR 2a wire. Snapshot peers sorted by `PeerId` so the JSON form is stable for the same logical state.

Transport-agnostic: `Manager` emits `ManagerEffect`s into an `EffectSink`; a swarm-backed transport (PR 2c or absorbed into PR 3) translates each effect onto the wire. `MockEffectSink` in tests assert on emission directly without standing up a real swarm.

16 unit tests passing.

**Deferred to PR 2c (or PR 3):** the `ClusterRuntime`-backed transport that bridges `ManagerEffect`s onto real libp2p calls + the `Member` consumer that applies inbound snapshots via `ClusterRuntime::update_cluster_doc` and responds to heartbeats. PR 2b's Manager is verified-by-test in isolation so PR 2c is "wire it up", not "design + test it".

## Next — PR 3 (failover batch: T10 + T11 + T13 + T14)

Manager failover machinery. Lands:

- T10 — Deterministic election: oldest cluster member by registry join-time becomes Manager. Tiebreak: lower `PeerId`. No voting.
- T11 — Sole-survivor election: N=1 quorum. Folds into T10's implementation as a documented zero-minimum check.
- T13 — Both graceful-quit announcement and crash (T4 timeout) feed the same T10/T11 election machinery. The announcement is a new wire message on the heartbeat transport (T3).
- T14 — Newly-elected Manager sends a signed handoff notification to Discovery (via a new `DiscoveryClient::notify_manager_handoff(wallet, cluster_name, new_manager_peer_id)` helper added in `auki-network`) so late-joiners hitting `GET /clusters/latest` route their JoinRequests to the live Manager rather than the dead one. Signed by the new Manager's `peer/v1` derivation key. See [`parking_lot.md`](../parking_lot.md) for the wire-shape spec.

## Then — PR 4 (T5: `JoinRequest`)

JoinRequest admission. Lands:

- T5 — `JoinRequest { peer_id: PeerId, reachability: ReachabilityRecord }`. Two fields, no wallet credentials, no capabilities, no data products. Same shape for UI-driven joins (Park) and headless autodiscover joins (Booster, Sentinel).
- `join_domain(&Wallet, &target, &DiscoveryClient) -> Result<DomainHandle, JoinDomainError>` entry point on top.

## Architectural decisions to honor

See [`parking_lot.md`](../parking_lot.md). Locked decisions transcribed from the Greenland Notion page:

- Domain identity = `{wallet_id}/{name}` with `"Vinland"` singleton exception (T1 + T12).
- Heartbeat tick = 10s, single global default for v1 (T2 ← Q2).
- Heartbeat transport = libp2p peer-to-peer; Discovery not in the heartbeat path (T3 ← Q1).
- Departure threshold = 2 consecutive missed responses (T4 ← Q3).
- JoinRequest fields = `peer_id` + reachability only; no wallet creds, no capabilities (T5 ← Q4).
- Mutation authority = current Manager's peer identity; wallet is one-shot at Domain creation (T6 ← Q8). Known security caveat documented for a future hardening quest.
- Broadcast envelope = full `ClusterDoc` snapshot on every mutation, **libp2p Manager→members directly** via new `/auki/registry/0.0.1` protocol; Discovery is bootstrap-rendezvous only, not in the live fan-out path (T7 ← Q5 + Q-disc-1).
- Election rule = oldest cluster member by registry join-time; tiebreak lower `PeerId`; no voting; no quorum check (T10 ← Q10).
- Sole-survivor election = N=1 (T11 ← Q11). Split-brain on network partition is the accepted trade-off.
- Failover triggers = both graceful announcement and crash detection feed the same election machinery (T13 ← Q9).
- Newly-elected Manager signs + sends a Manager-handoff notification to Discovery (T14 ← Q14) so late-joiners route to the live Manager. SDK side authors + signs; Discovery side verifies + persists.
- Discovery endpoint for headless join: `GET /clusters/latest` (T8 ← Q-disc-2). SDK code unchanged — already passes the canonical Domain Identity string as `cluster_name`. Discovery-side schema gets `created_ns: i64` + `current_manager_peer_id: Option<PeerId>` additions; SDK picks them up on the next `auki-network` tag bump.
- Park UI: accept any string for Domain name in v1 (T9 ← Q12). T1's `init_domain(&str)` signature already permits this.
- Headless daemons fall back to default Domain `"Vinland"` when no Domain exists; reserved singleton (T12 ← Q7).

## Open items

See [`parking_lot.md`](../parking_lot.md). One question filed in this PR:

- Glossary reconciliation: existing `Domain ID = hash(domain_owner_pubkey)` definition predates `{wallet_id}/{name}`. PR 1 (T1) absorbs the reconciliation alongside the canonical-string implementation.
