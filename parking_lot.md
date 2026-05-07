# Parking lot — root

Open architectural questions and cross-cutting design decisions for the Auki SDK as a whole. Crate-specific questions live in each crate's `parking_lot.md`; cross-crate questions live in [`crates/parking_lot.md`](crates/parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](CLAUDE.md) for the workflow.

---

## README casing convention

Outer crate READMEs are `README.md` (uppercase). Inner per-crate implementation status is in `src/readme.md` (lowercase). [CONTRIBUTING.md](CONTRIBUTING.md) shows uppercase for both. Should the inner files be renamed for consistency, or is the casing intentional (e.g. "outer is the public spec, inner is the working notes")?

## Outer README framing — "aspirational" vs "current state"

[CONTRIBUTING.md](CONTRIBUTING.md) describes the outer crate `README.md` as the **aspirational spec** ("what this component should be") while `src/README.md` is **what is actually implemented today**. In practice the outer crate READMEs we just wrote describe what's implemented today, since spec and reality match. As the SDK grows and ambitions outpace implementation, the two framings diverge — which one is the outer README?

## Cross-language conformance vectors — coverage gaps

Locked vectors now exist in `auki-hash` (XXH3-128), `auki-identity` (`derive_child("peer/v1")` pubkey + `sign_canonical_json`), `auki-network` (seed → libp2p PeerId, Vinland Discovery `register` signing, PointCloudFrame on-wire), and `auki-registry` (M1 sensor + M1 point cloud + Frame Registry M1 vector). See the table in [`README.md`](README.md). Not yet locked: `auki-jcs` (canonicalization end-to-end, beyond what `auki-identity` exercises in passing), `auki-logs` (segment file binary layout — the most-load-bearing schema with no cross-language reference yet), `auki-time-transforms` (TimeTransform Log payload). Worth adding when a second-language consumer (Park's browser side, or a future Python `auki-logs-py`) starts touching that on-disk shape — drift is silent until then.

## `/api/state.session_uuid` vs `/api/info.session_id` naming

Post-ansuz `/api/info` redesign returns `session_id`; `/api/state` still returns `session_uuid`. Same value, two field names. The SDK's broader convention is `session_id` (per [`auki-session`](crates/auki-session/README.md) and the manifest spec); `session_uuid` predates that landing. Renaming `/api/state.session_uuid` → `session_id` is a breaking change to consumers expecting the old name (Park, etc.). Aligning makes the API consistent; deferring keeps existing consumers unbroken until the next coordination-tag PR. Decide with the next consumer-coordination round.

## Glossary.md term coverage

[Glossary.md](Glossary.md) covers protocol-level concepts (Domain, Map, Cluster, identifier model), all four logs, both registries that ship today (Sensor + Clock are described per-crate; Frame Registry has its own Glossary entry as of v0.0.22), Pose Source, Anchor, App ID, App Instance, Discovery. Per-crate implementation details stay in each crate's README; the Glossary links out where the boundary is unclear. **Resolved 2026-05-07** — original parking-lot question was whether each crate-owned term lived in the Glossary or stayed with the crate; the answer is hybrid (protocol concepts in the Glossary, implementation details in the crate), as documented above.

## Python bindings strategy

**Resolved 2026-05-06 — per-component (a).** [`auki-identity-py`](crates/auki-identity-py) ships the identity primitives; [`auki-network-py`](crates/auki-network-py) ships `ClusterRuntime` + `Stream<T>` + `discovery_client`. Per-component naming over an umbrella `auki-py`. Future bindings (`auki-logs-py`, `auki-session-py`, `auki-registry-py`, `auki-time-transforms-py`) follow the same pattern when downstream consumers need them.

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

## Propagate: Pose Log capture shape decided

`PoseLogEntry { transforms: Vec<TransformSample> }` with inline `PoseSource` in the manifest (no Pose Source Registry — payload is self-describing; provenance only). [`dataproducts.md`](dataproducts.md)'s `FrameTransformAvailability` is still marked "TBD — pending Pose Log"; refresh to reflect that the shape is now concrete (`log_handle` references `<session>/poselogs/<recording_uuid>/`, `to_frame_id` / `to_frame_hash` / `to_frame_entry` resolve via the Frame Registry which now exists). `convert_pose` itself is still pending — capture and read are in place; composition / path-finding is not.

## Discovery descriptor — `log_handle` semantics

What's the actual handle? `(sensor_id, sensor_hash)` pair? URL relative to a node base? Peer-ID-prefixed path? Depends on wire-protocol decisions that haven't been made yet.

## Discovery descriptor — aborted-status detection

The on-disk format doesn't currently mark clean-close vs. crash. Heuristic: "no recent updates AND no sealed marker." Pin before any consumer relies on the `status: "aborted"` field meaning anything specific.

## Discovery descriptor — self-hash

Would let peers cache by descriptor identity but adds a chicken-and-egg with `generated_at_ns`. Skip for v1; revisit when descriptor distribution becomes hot.

## Propagate: registries stay app-rooted (Park's domain-rooted counter-proposal rejected)

Decision: `<app_root>/registries/...` is the canonical layout. Sweep [`tags.md`](tags.md), `dataproducts.md`, and any crate README for residual hints at a domain-rooted alternative and remove them.

---

## Grimsby follow-ups (cross-cutting items surfaced during the [grimsby](https://www.notion.so/3575c8e965928079a955ed9573bbb398) decision walkthrough — none gating grimsby itself)

### `/auki/capabilities/1.0.0` — Layer 2 capability advertisement

Grimsby D2 resolved on `sensor_id`-only addressing for v1. The long-term shape is topic-based addressing (consumers asking for `rgb/head_left` instead of an internal `K1-AABBCCDDEEFF/head_left_cam`), which requires a new libp2p request-response protocol returning a `Vec<{topic, sensor_id, sensor_hash}>` so consumers can discover what a peer offers. Sibling to `/auki/cluster/1.0.0` (ansuz #3) and `/auki/stream/1.0.0` (grimsby #1); same `request_response::json::Behaviour` codec pattern. Belongs in `auki-network`. State of SDK doc flags this as "the biggest single gap on the networking side." Not gating grimsby; a parallel track once Layer 3 (admission) becomes a near-term need.

### `Log<T>` watcher primitive

Grimsby D3 considered (and deferred) a `stream_provider` shape that takes `Option<&Log<T>>` and lets the SDK tail the log onto the wire. Blocked on `auki-logs` not having a way to notify on `append` — the only way to tail a Log<T> today is to poll the segments directory. Designing a watcher (semantics for replay-from-tail vs. start-at-now, backpressure interaction with disk writes, multi-watcher support) is meaningful work. Once it lands, `Stream<T>` gets a one-line convenience adapter `handle.tail_log(&log)` so producers already writing to a Log<T> get streaming for free.

### `auki-network-py` async-iterator ↔ Rust Stream bridge

Grimsby D3 settled on `Fn(StreamRequest) -> StreamDecision<T>` where the app returns a `futures::Stream`. The Python binding for that signature (deliverable #4) needs to wrap a Python callable returning either `None` (decline) or an async iterator (accept), then bridge `__anext__()` calls onto tokio's runtime. The standard tool is [`pyo3-asyncio`](https://github.com/awestlake87/pyo3-asyncio); rolling our own is an option but adds a maintenance surface. Alternative wrapper shape: have the Python `stream_provider` return a callable `async def next() -> Optional[T]` instead of an async iterator — same wire semantics, simpler bridge. Pick when #4 starts.

---

## Subfolder summary

- [`crates/`](crates/parking_lot.md) — schema versioning coordination; sprint.md scaffolding still missing
