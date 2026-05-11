# auki-domain — implementation status

What is implemented today, in honest detail. See [`../README.md`](../README.md) for the aspirational spec.

## Today (PR 1 — Greenland T1)

- **`DomainIdentity`** — wallet-scoped `{wallet_id}/{name}` value type with the reserved `"Vinland"` singleton exception. Constructors: `user_named(&Wallet, &str)` and `singleton()`. `canonical_string()` produces the string Discovery indexes on. Implements `Display`, `PartialEq`, `Eq`, `Hash`, `Clone`, `Debug`.
- **`init_domain`** — async entry point. Constructs a `DomainIdentity` from `wallet` + `name`, registers the local daemon as the first peer of the Domain via `DiscoveryClient::register`, returns a `DomainHandle`. If `name == "Vinland"` the constructor builds the singleton identity instead of a user-named one.
- **`DomainHandle`** — minimum viable handle. Exposes `identity() -> &DomainIdentity`. Manager-role state (heartbeat tick, registry write authority, JoinRequest admission) lands in PR 2.
- **Glossary update** — `Glossary.md` gains a `Domain Identity` entry. `Domain ID` keeps its existing definition (`hash(domain_owner_pubkey)`) for TagClaim purposes; the network-topic role moves to `Domain Identity` (`{Domain ID}/{name}` or `Vinland` for the singleton).

12 unit tests + 2 doctests + 2 locked cross-language conformance vectors (user-named string structure, singleton string).

## Not yet implemented (deferred)

- Manager role (heartbeat sender, JoinRequest admission, registry write authority, broadcast) — PR 2.
- Member role (heartbeat responder, registry subscriber) — PR 2.
- Failover (election, sole-survivor handling, graceful + crash triggers) — PR 3.
- `join_domain` / `JoinRequest` — PR 4.
- PyO3 binding (`auki-domain-py`) — separate follow-up when downstream Python consumers need it.

## What lands next

See [`sprint.md`](sprint.md) for the four-PR Greenland sequence.
