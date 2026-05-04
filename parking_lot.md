# Parking lot — root

Open architectural questions and cross-cutting design decisions for the Auki SDK as a whole. Crate-specific questions live in each crate's `parking_lot.md`; cross-crate questions live in [`crates/parking_lot.md`](crates/parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](CLAUDE.md) for the workflow.

---

## README casing convention

Outer crate READMEs are `README.md` (uppercase). Inner per-crate implementation status is in `src/readme.md` (lowercase). [CONTRIBUTING.md](CONTRIBUTING.md) shows uppercase for both. Should the inner files be renamed for consistency, or is the casing intentional (e.g. "outer is the public spec, inner is the working notes")?

## Outer README framing — "aspirational" vs "current state"

[CONTRIBUTING.md](CONTRIBUTING.md) describes the outer crate `README.md` as the **aspirational spec** ("what this component should be") while `src/README.md` is **what is actually implemented today**. In practice the outer crate READMEs we just wrote describe what's implemented today, since spec and reality match. As the SDK grows and ambitions outpace implementation, the two framings diverge — which one is the outer README?

## Cross-language conformance vectors

`auki-hash` publishes locked conformance vectors so any reimplementation can be validated. Should `auki-jcs`, `auki-logs`, `auki-registry`, and `auki-time-transforms` also publish locked vectors? Concretely: a `tests/cross_language/` directory with golden bytes any port must reproduce. Boosterapp's Python sidecar is already a de facto second implementation — vectors would catch drift automatically.

## Glossary.md

[CLAUDE.md](CLAUDE.md) references a root-level `Glossary.md` for "definitions of all key terms." It doesn't exist yet. Who drafts it, and what's the seed list of terms (Domain, Cluster, Map, Frame, Pose Log, etc.)?

## Python bindings strategy

Listed under "Not yet implemented" in [README.md](README.md). Path forward options: (a) PyO3 wrapper exposing `auki-logs`, `auki-registry`, `auki-time-transforms`; (b) pure-Python re-implementation per the on-disk specs; (c) bless boosterapp's existing Python sidecar as the official binding. Each has different drift-risk, packaging, and effort tradeoffs.

---

## TagClaim — `tags.md` ownership

[`tags.md`](tags.md) sits at the repo root and defines the `TagClaim` schema and the `tags.jsonl` sidecar that lives next to every log's `manifest.json`. The spec is currently homeless — it references the auki-logs path layout but no crate owns it, which is why `tags.jsonl` was missing from the per-crate READMEs until just now. Possible homes: stay at root (cross-cutting like [`dataproducts.md`](dataproducts.md)), fold into `auki-logs` (the sidecar layout is its concern), or live in a future `auki-tags` crate (when actual TagClaim read/write lands). Decide before any SDK code starts producing or consuming TagClaims.

## TagClaim — tag removal vs revocation semantics

`tags.jsonl` is append-only; there's no removal. Revocation is a *new* claim of `claim_type: "revoke"` referencing the prior claim's hash. Whether and how receivers honor revocation is application-layer policy. Pin the resolution rules before any peer-to-peer trust depends on them.

## TagClaim — `tag_id` derivation per claim type

Today the schema treats `tag_id` as opaque bytes/hex. Once identity lands, derivations differ: `domain_id = hash(domain_owner_pubkey)`, `anchor_id = hash(anchor_record)`, `contributor_id = hash(contributor_pubkey)`, etc. Document the derivation per claim type alongside the identity layer.

## TagClaim — cross-data / set-scoped claims

A claim today references one `data_id`. If we need claims about *sets* of data products (e.g. "this whole session is steve-domain") we'll need a session-id-scoped or set-scoped variant. Decide before tag-claim consumers proliferate.

## TagClaim — tag inheritance / propagation across derived data

If Park merges data from booster (booster-domain) and galbot (galbot-domain), what tags does the merged result carry? Tags don't capture structural derivation; combining all source tags loses the merge relationship, retaining only the result's own tag erases provenance. Eventually needs a separate `derivation_chain` primitive.

## TagClaim — self-hash of the claim

Would let peers cache by claim identity, but adds a chicken-and-egg with `issued_at_ns`. Trade-off worth pinning before tag stores get distributed.

## Discovery descriptor — Pose Log shape

[`dataproducts.md`](dataproducts.md)'s `FrameTransformAvailability.log_handle` only resolves to something fetchable once Pose Log exists. Pose Log is listed as not-yet-implemented in the root README; its concrete schema is unblocked by this descriptor.

## Discovery descriptor — `log_handle` semantics

What's the actual handle? `(sensor_id, sensor_hash)` pair? URL relative to a node base? Peer-ID-prefixed path? Depends on wire-protocol decisions that haven't been made yet.

## Discovery descriptor — aborted-status detection

The on-disk format doesn't currently mark clean-close vs. crash. Heuristic: "no recent updates AND no sealed marker." Pin before any consumer relies on the `status: "aborted"` field meaning anything specific.

## Discovery descriptor — self-hash

Would let peers cache by descriptor identity but adds a chicken-and-egg with `generated_at_ns`. Skip for v1; revisit when descriptor distribution becomes hot.

## Propagate: registries stay app-rooted (Park's domain-rooted counter-proposal rejected)

Decision: `<app_root>/registries/...` is the canonical layout. Sweep [`tags.md`](tags.md), `dataproducts.md`, and any crate README for residual hints at a domain-rooted alternative and remove them.

---

## Subfolder summary

- [`crates/`](crates/parking_lot.md) — schema versioning coordination; sprint.md scaffolding still missing
