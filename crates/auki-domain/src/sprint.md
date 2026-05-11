# Sprint — auki-domain

Current work and next steps. This crate is being scaffolded as the home for the [Greenland quest](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2)'s SDK-side work.

## Now (PR 0 — scaffold + plan)

This PR. Lands:

- Folder convention (Cargo.toml, lib.rs stub, README, parking_lot.md, changelog.md, src/readme.md, src/sprint.md).
- Workspace registration in root `Cargo.toml`.
- [`parking_lot.md`](../parking_lot.md) pre-files of the Greenland architectural decisions transcribed from the Notion Tasks table.
- This sprint plan.

No functional code. No `auki-domain` reverse dependencies wired yet (no consumer references this crate; that happens in PR 1).

## Next — PR 1 (T1: `DomainIdentity` + `init_domain`)

Greenland T1 entry point. Lands:

- `DomainIdentity { wallet_id: WalletId, name: String }` value type with:
  - Canonical string form `{wallet_id}/{name}` (or just `Vinland` for the singleton exception per T12).
  - `cluster_name()` accessor producing the string Discovery's existing `register / fetch / subscribe` consume.
  - Locked cross-language conformance vectors for the canonical string.
- `init_domain(&Wallet, &str, &DiscoveryClient) -> Result<DomainHandle, InitDomainError>`:
  - Constructs the identity.
  - Registers the cluster with Discovery via existing `DiscoveryClient::register`.
  - Returns a minimal `DomainHandle` with `identity()` accessor. Role state is stubbed (returns `Role::Manager` unconditionally — heartbeat machinery lands in PR 2).
- Glossary update: reconcile the existing `Domain ID = hash(domain_owner_pubkey)` definition with `{wallet_id}/{name}` per the parking-lot question filed in PR 0.

Scope guardrails (NOT in PR 1): no heartbeats, no JoinRequest, no broadcast, no failover, no Manager-write authority semantics. Just identity + Discovery registration + the handle type.

## Then — PR 2 (heartbeat batch: T2 + T3 + T4 + T6 + T7)

The Manager role's core lifecycle. Lands:

- T2 — Manager heartbeat sender + member responder, 10s global tick.
- T3 — Heartbeat transport over libp2p peer-to-peer (new protocol `/auki/heartbeat/0.0.1` — sibling to `/auki/stream/0.1.0` and `/auki/message/0.0.1`). Lab-mode versioning per Rule 11.5 (`0.0.1`, not `1.0.0`).
- T4 — 2-consecutive-missed-tick departure detection (~20s window).
- T6 — Manager-authoritative registry mutations. Mutation authority = current Manager's peer identity. Wallet is one-shot at Domain creation.
- T7 — Manager publishes a fresh full `ClusterDoc` snapshot on every registry change. Reuses Vinland D6's existing SSE fan-out path on Discovery; no Discovery wire-shape change.

## Then — PR 3 (failover batch: T10 + T11 + T13)

Manager failover machinery. Lands:

- T10 — Deterministic election: oldest cluster member by registry join-time becomes Manager. Tiebreak: lower `PeerId`. No voting.
- T11 — Sole-survivor election: N=1 quorum. Folds into T10's implementation as a documented zero-minimum check.
- T13 — Both graceful-quit announcement and crash (T4 timeout) feed the same T10/T11 election machinery. The announcement is a new wire message on the heartbeat transport (T3).

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
- Broadcast envelope = full `ClusterDoc` snapshot on every mutation; existing SSE wire shape unchanged (T7 ← Q5).
- Election rule = oldest cluster member by registry join-time; tiebreak lower `PeerId`; no voting; no quorum check (T10 ← Q10).
- Sole-survivor election = N=1 (T11 ← Q11). Split-brain on network partition is the accepted trade-off.
- Failover triggers = both graceful announcement and crash detection feed the same election machinery (T13 ← Q9).
- Park UI: accept any string for Domain name in v1 (T9 ← Q12). T1's `init_domain(&str)` signature already permits this.
- Headless daemons fall back to default Domain `"Vinland"` when no Domain exists; reserved singleton (T12 ← Q7).

## Open items

See [`parking_lot.md`](../parking_lot.md). One question filed in this PR:

- Glossary reconciliation: existing `Domain ID = hash(domain_owner_pubkey)` definition predates `{wallet_id}/{name}`. PR 1 (T1) absorbs the reconciliation alongside the canonical-string implementation.
