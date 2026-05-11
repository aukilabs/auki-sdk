# auki-domain

Domain lifecycle for the Auki SDK. A **Domain** is the unit of cluster identity (see [`Glossary.md`](../../Glossary.md#domain)) — the topic peers cluster around on the network, and the tag that asserts data describes a specific physical space.

This crate owns Domain *lifecycle*: creating a Domain, joining an existing one, the Manager/Member roles, heartbeats, the live Cluster Registry, and Manager failover. It is **not** the home for `convert_time` / `convert_pose` — those operate inside a Domain but live elsewhere. It is **not** the home for log-writing session lifecycle (sensor logs, pose logs, registry entries) — that's [`auki-session-py`](../auki-session-py)'s eventual Rust sibling.

## Status

Scaffolding only. First implementation lands in Greenland PR 1 (T1 — `DomainIdentity` + `init_domain`). See [`src/readme.md`](src/readme.md) for current state, [`src/sprint.md`](src/sprint.md) for the PR sequence.

## What this is

The [Greenland quest](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2) closes the gap where early joiners don't see late joiners. Park, BoosterApp, and Sentinel daemons need to dynamically discover each other on a LAN without restarting. This crate is the SDK-side home of that work.

## Aspirational surface

Targeting the four entry points the Greenland demo flow requires:

```rust
use auki_domain::{init_domain, join_domain, DomainHandle, DomainIdentity};
use auki_identity::Wallet;
use auki_network::discovery_client::DiscoveryClient;

// Park boots, prompts user, becomes Manager of a fresh Domain.
let handle: DomainHandle = init_domain(&wallet, "demo-2026-05", &discovery).await?;

// Booster / Sentinel autodiscover via Discovery, file a JoinRequest.
let handle: DomainHandle = join_domain(&wallet, &target, &discovery).await?;

// Identity primitive — canonical-string `{wallet_id}/{name}` with a
// reserved "Vinland" singleton exception per T12 of Greenland.
let id: DomainIdentity = handle.identity();
```

Where `DomainHandle` owns:

- **Role state** — `Role::Manager { registry_writer, heartbeat_sender, join_admission }` or `Role::Member { registry_subscriber, heartbeat_responder }`.
- **Cluster Registry** — Manager-owned writable copy on the Manager; replicated read-only snapshots on Members. Each mutation produces a fresh full snapshot the Manager publishes to Discovery (T7).
- **Heartbeat loop** — 10-second tick (T2) over libp2p peer-to-peer (T3); members marked departed after 2 consecutive missed responses (T4).
- **Failover** — on Manager departure (graceful or crash), surviving members run the deterministic election rule (T10: oldest cluster member by registry join-time wins; T11: N=1 quorum; T13: both triggers fire the same machinery).

Each of these lands in a separate PR (see [`src/sprint.md`](src/sprint.md)).

## Relationship to existing crates

This crate **consumes** but does not duplicate the libp2p substrate:

- [`auki-identity`](../auki-identity) — `Wallet`, `WalletId`. Domain identity is wallet-scoped: `{wallet_id}/{name}`.
- [`auki-network`](../auki-network) — `DiscoveryClient` (Domain registration + Cluster Registry snapshot fan-out via existing SSE), `ClusterRuntime` (peer dial / connect / reconnect machinery the Manager and Member roles plug into), `PeerIdentity` (libp2p identity carried into `JoinRequest`'s reachability record).

The libp2p substrate stays in `auki-network`. This crate is the cluster-lifecycle / Domain-management layer on top.

## Naming note — `Domain` already exists in the Glossary

[`Glossary.md`](../../Glossary.md#domain-id) defines `Domain ID = hash(domain_owner_pubkey)`. The Greenland identity `{wallet_id}/{name}` extends that — `wallet_id` is itself `hash(wallet_pubkey)` per [`auki-identity`](../auki-identity), so the existing definition becomes "the wallet-component of a Domain identity" with the new `{name}` component letting a single wallet own multiple Domains.

The reconciliation is filed in [`parking_lot.md`](parking_lot.md). Touching the Glossary entry is out of scope for the scaffolding PR; PR 1 (T1) lands the canonical string and updates the Glossary in the same change.
