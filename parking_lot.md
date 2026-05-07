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

## Control API `PATCH /api/sensor_logs/<id>` — mutability scope

v0.0.23 spec restricts PATCH to `retention_ns` and `duration_ns`; identity fields (`sensor_id`, `sensor_hash`, `clock_id`, `clock_hash`, `session_id`) are immutable on the rationale that mutating any of them is semantically a different log. Confirm this is the right call before implementers wire it. The relaxation case worth thinking about: a `sensor_hash` that drifts mid-session because the operator updated the camera's intrinsics and the daemon re-registered the sensor — does the running log carry the old hash forever, or does the daemon close it and open a new one with the new hash? My lean: close-and-reopen is the right model, so PATCH stays restricted; flagging here so the boosterapp implementer can push back if the close-and-reopen window costs them frames they need.

## Control API — cross-session enumeration without a live session

v0.0.23 spec calls out `GET /api/sensor_logs` listing every on-disk session by default but keeps "browse-only daemon mode" (no live session, only read endpoints) explicitly out of scope. The "daemon must be running, with a live session, to serve any request" coupling is convenient for v1 but burns operators who want to browse historical recordings on a machine where the producer app has been uninstalled. Whether to add a `--browse-only <app_root>` daemon mode (or a separate read-only browser binary) is a v2 question. Pinning the answer affects how Park frames its "open recording from disk" UX.

## Control API — `started_after` / `started_before` clock interpretation

v0.0.23 spec has `started_after` / `started_before` query parameters compared per-log against each log's own `clock_id`. This works when a daemon's logs share a single clock (BoosterApp v1 — every log under one `CLOCK_REALTIME`-backed clock); it gets ambiguous as soon as a daemon writes logs across multiple clocks (e.g. a robot running both a `K1-AABBCCDDEEFF/utc` clock and a `K1-AABBCCDDEEFF/session-monotonic` clock). Options when this gets messy: (a) require a `clock_id` query param when filter values are supplied, (b) pin a designated "filter clock" the daemon advertises in `/api/info`, (c) silently drop logs whose clock is incompatible with the filter (fragile). Decide before the first heterogeneous-clock daemon ships.

## Glossary.md — additional terms to seed

[Glossary.md](Glossary.md) seeded May 4, 2026 with Domain, Domain Owner, Domain ID, Cluster, Scenegraph, Scenegraph ID, Map, Session ID. Pending entries: Frame, Pose Log, Sensor Log, Detection Log, TimeTransform Log, Pose Source, Anchor, App ID. Open question — does each crate-owned term live in the Glossary or stay in the crate's README, with the Glossary linking out? Default for now: protocol-level concepts (Domain, Map, Cluster, identifier model) live in the Glossary; per-crate implementation details stay with the crate.

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

## Propagate: Pose Log capture shape decided

`PoseLogEntry { transforms: Vec<TransformSample> }` with inline `PoseSource` in the manifest (no Pose Source Registry — payload is self-describing; provenance only). Update [`dataproducts.md`](dataproducts.md)'s `FrameTransformAvailability.log_handle` to point at the actual Pose Log layout (`<session>/poselogs/<pose_log_id>/`); confirm the discovery descriptor reads cleanly against the new shape. `convert_pose` itself is still pending — capture and read are in place; composition / path-finding is not.

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

## Propagate: `Session.open` mints `session_id` (SDK-mints, with optional kwarg escape hatch)

Resolved 2026-05-07: `Session.open(app_root, *, app_id, app_instance, session_id=None)` — when `session_id` is `None` the SDK mints a fresh UUIDv4; when supplied, the SDK validates filesystem-safety and uses it as-is. Default is SDK-mints; the kwarg is the escape hatch for deterministic / pre-known IDs (test harnesses, replay tooling). Applies symmetrically to the new Rust `auki_session::Session::open` shape (which doesn't exist yet — the `auki-session` crate today is path-helpers-only).

Why SDK-mints won: the "integrator-as-policy-boundary" framing didn't actually buy anything — every implementation mints its own UUIDs regardless, the policy "session ids are UUIDv4" is a doc-level claim either way. SDK-mints centralizes the one place that has to get UUIDv4 right and is strictly easier on callers. Cost is a `uuid` dep on `auki-session` (currently zero-dep — `uuid` v1 with the `v4` feature is tiny, no transitive deps).

Docs to update when the Rust `Session` struct + `auki-session-py` first implementation land: (a) [`crates/auki-session/README.md`](crates/auki-session/README.md) — add a `Session` section and revise the "integrator generates a fresh UUIDv4 at boot" line; (b) [`crates/auki-registry/README.md`](crates/auki-registry/README.md) and [`crates/auki-time-transforms/README.md`](crates/auki-time-transforms/README.md) manifest tables — relax "minted by the integrator at app boot" to "minted by the integrator at app boot, or by `auki_session::Session::open`." Lives at root because it's cross-cutting between `auki-session` and `auki-session-py`; everything `auki-session-py`-specific lives in [`crates/auki-session-py/parking_lot.md`](crates/auki-session-py/parking_lot.md).

---

## Subfolder summary

- [`crates/`](crates/parking_lot.md) — schema versioning coordination; sprint.md scaffolding still missing
- [`crates/auki-datatypes/`](crates/auki-datatypes/parking_lot.md) — `.proto` package naming convention; field number allocation strategy; locked conformance vector format; schema versioning policy; five per-type slop fixes (PinholeCameraLogEntry intrinsics placement, PointCloud on-disk-vs-wire drift, Audio chunk metadata, TimeTransformEntry source/discontinuous, TimeTransformSource collapse). None gating the scaffold; per-type slop fixes resolve at their matching migration step.
- [`crates/auki-session-py/`](crates/auki-session-py/parking_lot.md) — `payload: bytes` encoding contract resolved (protobuf via auki-datatypes); libp2p control-plane design timing (deferred until this crate stabilizes); 6 resolved design decisions waiting to propagate when first implementation lands
