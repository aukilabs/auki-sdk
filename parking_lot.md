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

## Control API `PATCH /api/sensor_logs/<id>` — mutability scope

v0.0.23 spec restricts PATCH to `retention_ns` and `duration_ns`; identity fields (`sensor_id`, `sensor_hash`, `clock_id`, `clock_hash`, `session_id`) are immutable on the rationale that mutating any of them is semantically a different log. Confirm this is the right call before implementers wire it. The relaxation case worth thinking about: a `sensor_hash` that drifts mid-session because the operator updated the camera's intrinsics and the daemon re-registered the sensor — does the running log carry the old hash forever, or does the daemon close it and open a new one with the new hash? My lean: close-and-reopen is the right model, so PATCH stays restricted; flagging here so the boosterapp implementer can push back if the close-and-reopen window costs them frames they need.

## Control API — cross-session enumeration without a live session

v0.0.23 spec calls out `GET /api/sensor_logs` listing every on-disk session by default but keeps "browse-only daemon mode" (no live session, only read endpoints) explicitly out of scope. The "daemon must be running, with a live session, to serve any request" coupling is convenient for v1 but burns operators who want to browse historical recordings on a machine where the producer app has been uninstalled. Whether to add a `--browse-only <app_root>` daemon mode (or a separate read-only browser binary) is a v2 question. Pinning the answer affects how Park frames its "open recording from disk" UX.

## Control API — `started_after` / `started_before` clock interpretation

v0.0.23 spec has `started_after` / `started_before` query parameters compared per-log against each log's own `clock_id`. This works when a daemon's logs share a single clock (BoosterApp v1 — every log under one `CLOCK_REALTIME`-backed clock); it gets ambiguous as soon as a daemon writes logs across multiple clocks (e.g. a robot running both a `K1-AABBCCDDEEFF/utc` clock and a `K1-AABBCCDDEEFF/session-monotonic` clock). Options when this gets messy: (a) require a `clock_id` query param when filter values are supplied, (b) pin a designated "filter clock" the daemon advertises in `/api/info`, (c) silently drop logs whose clock is incompatible with the filter (fragile). Decide before the first heterogeneous-clock daemon ships.

## Glossary.md term coverage

[Glossary.md](Glossary.md) covers protocol-level concepts: Real World Web, Daemon, the Domain model (Domain / Domain Owner / Domain ID, Cluster / ClusterDoc, Scenegraph / Scenegraph ID, Map), the two core operations (`convert_time`, `convert_pose`), identity primitives (Wallet, Peer ID), lifecycle and id model (Session ID, App ID, App Instance, Sensor / Clock / Frame ID), all three registries that ship today (Sensor, Clock, Frame), Coordinate convention, Manifest, SpatialTransform, all four logs, Pose Source, Anchor, TagClaim, Discovery. Per-crate implementation details stay in each crate's README; the Glossary links out where the boundary is unclear. **Resolved 2026-05-07** — original parking-lot question was whether each crate-owned term lived in the Glossary or stayed with the crate; the answer is hybrid (protocol concepts in the Glossary, implementation details in the crate), as documented above. Coverage expanded 2026-05-08 (12 new entries + Vinland leak scrubbed).

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

`PoseLogEntry { transforms: Vec<TransformSample> }` with inline `PoseSource` in the manifest (no Pose Source Registry — payload is self-describing; provenance only). [`dataproducts.md`](dataproducts.md)'s `FrameTransformAvailability` is still marked "TBD — pending Pose Log"; refresh to reflect that the shape is now concrete (`log_handle` references `<session>/poselogs/<pose_log_id>/`, `to_frame_id` / `to_frame_hash` / `to_frame_entry` resolve via the Frame Registry which now exists). `convert_pose` itself is still pending — capture and read are in place; composition / path-finding is not.

**Note:** the Pose Log shape itself is being rewritten via the synthesis decided 2026-05-07 (per-`(from, to)` identity instead of per-producer; flat `SpatialTransform` segment entries instead of `PoseLogEntry { transforms: Vec<...> }`; rigid vs movable writer mode). That redesign supersedes this Propagate task; see [`crates/auki-datatypes/src/sprint.md`](crates/auki-datatypes/src/sprint.md) step 5 for the migration sequence. Once that lands, this Propagate task gets replaced with the synthesis-resolved version.

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

### `/auki/capabilities/1.0.0` — Layer 2 topic-based addressing

Grimsby D2 resolved on `sensor_id`-only addressing for v1. The long-term shape is topic-based addressing (consumers asking for `rgb/head_left` instead of an internal `K1-AABBCCDDEEFF/head_left_cam`), which requires a new libp2p request-response protocol returning a `Vec<{topic, sensor_id, sensor_hash}>` so consumers can resolve a topic name to the currently-active `(sensor_id, sensor_hash)` pair offered by a peer. (Capability *discovery* — what configurations a peer's sensor supports — falls out of `<sensor_id>/<hash>.json` directory listing per the [Resolved: Sensor Capability Registry](#resolved-sensor-capability-registry-filed-by-dobby-2026-05-08) framing above.) Sibling to `/auki/cluster/1.0.0` (ansuz #3) and `/auki/stream/1.0.0` (grimsby #1); same `request_response::json::Behaviour` codec pattern. Belongs in `auki-network`. State of SDK doc flags this as "the biggest single gap on the networking side." Not gating grimsby; a parallel track once Layer 3 (admission) becomes a near-term need.

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

## API-surface review — items filed by Dobby, 2026-05-08

These six items came out of an API-surface review walkthrough on 2026-05-08, after PR #55 landed Step 0 of the [`auki-datatypes` migration](crates/auki-datatypes/src/sprint.md). The marquee elevation finding (`TransformSample` is wrong-layered as an `auki-registry` payload helper when it's the SDK's flagship transform primitive) is **resolved** — the migration plan moves it to `auki-datatypes` and renames it `SpatialTransform` per the Pose Log synthesis decided 2026-05-07. The remaining six items are smaller; each lives in the most-specific parking-lot it belongs in. Listed here for visibility and so a future reader can find the whole set.

- **`auki-session` is path helpers, not a session.** Filed at [`crates/auki-session/parking_lot.md`](crates/auki-session/parking_lot.md). Crate name vs scope mismatch — rename to `auki-paths` now (zero in-workspace consumers) or footnote the README, mirroring PR #55's "departing" pattern for `auki-registry`. Reserves `auki-session` for the runtime abstraction the [`Session.open` Propagate item](#propagate-sessionopen-mints-session_id-sdk-mints-with-optional-kwarg-escape-hatch) above already specs.
- **`Capability(pub String)` — open-string vs typed enum.** Filed at [`crates/auki-network/parking_lot.md`](crates/auki-network/parking_lot.md). Doc-comment the open-string-by-design contract (lean) or tighten to a typed enum with `Other(String)` escape hatch.
- **`PEER_DERIVATION_LABEL` lives in the wrong crate.** Filed at [`crates/auki-network/parking_lot.md`](crates/auki-network/parking_lot.md). Constant's *meaning* belongs in `auki-identity` (it's a `Wallet::derive_child` label); only the consumer lives in `auki-network`. Move + re-export.
- **`StreamDispatch` is the streaming-stability lever — README should call it out.** Filed at [`crates/auki-network/parking_lot.md`](crates/auki-network/parking_lot.md). Closed-enum-by-design is correct (Dagaz Batch 1 #1); the disclosure is missing. One-sentence README addition.
- **`auki-identity` missing `Result<T>` aliases.** Filed at [`crates/auki-identity/parking_lot.md`](crates/auki-identity/parking_lot.md). Sister crates ship `pub type Result<T>`; this one does not, and has two error types (`VerifyError` + `SeedError`) so the alias needs splitting (`VerifyResult<T>` + `SeedResult<T>`).
- **Rust vs Python surface namespacing mismatch.** Filed at [`crates/parking_lot.md`](crates/parking_lot.md). `auki_network.cluster.*` / `auki_network.discovery.*` in Python vs flat `auki_network::` in Rust. Pick one, converge.

---

## API-surface — README "four surfaces" framing _(filed by Dobby, 2026-05-08)_

The root [`README.md`](README.md) "API surface" section markets four peer surfaces: (1) Rust crates, (2) PyO3 bindings, (3) HTTP control API, (4) libp2p wire protocols. They aren't peers. They're two axes:

- **Library surfaces.** Rust crates are the substrate; PyO3 is a *binding* of the substrate. Same contracts, two languages. Stability flows from the Rust side downhill.
- **Protocol surfaces.** HTTP control + libp2p wire are *contracts other parties implement* — the SDK specifies, daemons implement. These are what carry locked cross-language conformance vectors and version negotiation.

A reader who sees them as a flat four-list misses the asymmetry: a PyO3 binding change is non-breaking if it preserves Rust-side semantics; a `/auki/stream/1.0.0` payload addition is a coordinated bump (cf. the [`StreamDispatch` parking-lot item](crates/auki-network/parking_lot.md) on the streaming-stability lever).

Suggested reframe: replace the flat numbered list with a 2×2 framing — "Library bindings" (Rust + Python) above, "Protocol contracts" (HTTP control + libp2p wire) below. The cross-language conformance-vector section that already exists then has an obvious home (it's about protocol contracts, not bindings).

Doc-only; not gating any in-flight work. Pin before any future surface (e.g. a TypeScript/WASM binding for browser-side Park, or a `/auki/capabilities/1.0.0` protocol) muddies the categorization further.

---

## API-surface — Rule 1 quest-name leaks in the public README _(filed by Dobby, 2026-05-08)_

Per the no-quest-codenames-in-public-docs convention, the root [`README.md`](README.md) leaks codenames in five places. Quest names belong in `parking_lot.md`, internal changelogs, and the `CLAUDE.md` agent guide — never in the public README.

| Line | Current | Replace with |
|---|---|---|
| 213 | *"v0.0.22 ships `T = JpegFrame` (**grimsby v1** — byte-identical to …) and `T = PointCloudFrame` (**Dagaz Batch 1** — …)"* | *"v0.0.22 ships `T = JpegFrame` (RGB camera streaming — byte-identical to …) and `T = PointCloudFrame` (CDR-encoded `PointCloud2` streaming — …)"* |
| 214 | *"Discovery REST (**Vinland**) | Multi-cluster registry"* | *"Discovery REST | Multi-cluster registry"* |
| 268 | *"`sign_canonical_json(<**Vinland**-shaped registration>)`"* | *"`sign_canonical_json(<Discovery-registration shape>)`"* (or describe the shape directly without naming the quest) |
| 271 | *"**Vinland** Discovery `register` signed payload"* | *"Discovery `register` signed payload"* |
| 274 | *"The **Vinland** and PointCloudFrame vectors pin the wire shapes…"* | *"The Discovery-registration and PointCloudFrame vectors pin the wire shapes…"* |

Mechanical scrub; no semantic change. Doc-only PR. Cheap; should land before the next external-facing release tag.

---

## API-surface — `auki-logs` README bullet missing Pose Log _(filed by Dobby, 2026-05-08)_

The root [`README.md`](README.md) "On-disk format" section's `auki-logs` bullet reads:

> `auki-logs` — segmented ring-buffer log layout (used by both Sensor and TimeTransform Logs)

That bullet has been stale since the Pose Log capture primitive landed. The on-disk layout diagram immediately above the bullet shows `poselogs/<pose_log_id>/manifest.json + segments/<padded-ns>.seg` — that's the same auki-logs shape, not a parallel primitive. `auki-manifests::build_pose_log_manifest` requires `segment_duration_ns` + `retention_ns` — the auki-logs base contract. Pose Log uses the same `Log<T>` generic; only the segment payload type differs.

Suggested fix: change "**both** Sensor and TimeTransform Logs" → "Sensor, TimeTransform, **and Pose** Logs". Or expand into a sentence making the architectural point explicit: *"All log types — Sensor (with the Point Cloud and Audio sibling payloads), TimeTransform, and Pose — sit on the auki-logs ring-buffer primitive; the manifest schema and segment payload shape vary per log type, the segment machinery is shared."*

Doc-only PR. Cheap; bundles naturally with the quest-name scrub above.

---

## Resolved: Sensor Capability Registry _(filed by Dobby, 2026-05-08)_

Don't need one for now. The application can just make a `SensorRegistryEntry` for each configuration it's open to supporting; the active configuration is whichever hash sits in the live sensor log's manifest. The hash *is* the configuration agreement.

---

## Subfolder summary

- [`crates/`](crates/parking_lot.md) — schema versioning coordination; sprint.md scaffolding still missing; **Rust vs Python surface namespacing mismatch** (filed 2026-05-08)
- [`crates/auki-datatypes/`](crates/auki-datatypes/parking_lot.md) — `.proto` package naming convention; field number allocation strategy; locked conformance vector format; schema versioning policy; two per-type slop fixes remaining (TimeTransformEntry source/discontinuous, TimeTransformSource collapse). Step 0 (extract `auki-manifests`) **landed 2026-05-08**; Step 1 (`auki.camera` — `PinholeCameraLogEntry` + `DynamicIntrinsics`) **landed 2026-05-08**; Step 2 (`auki.frame_stream` + `auki.point_cloud_stream` + `auki.stream` envelope, libp2p substream wire) **landed 2026-05-08** — full envelope moves to protobuf, drops `base64` dep on the swarm path; `/auki/stream/1.0.0` retired in favour of `/auki/stream/0.1.0`; sibling rename of `/auki/cluster/1.0.0` → `0.0.1` and `/auki/identify/1.0.0` → `0.0.1`. Step 3 (`auki.point_cloud` — `PointCloudLogEntry`, opaque-bytes-only) **landed 2026-05-08** — symmetric with the wire's `PointCloudFrame { bytes }`; resolves the on-disk-vs-wire drift slop point. Step 4 (`auki.audio` — `AudioLogEntry`, opaque-bytes-only) **landed 2026-05-08** — same stance; resolves the implicit-vs-explicit chunk metadata slop point; drops `serde_bytes` from [`auki-registry`](crates/auki-registry). Remaining slop fixes resolve at their matching migration step.
- [`crates/auki-manifests/`](crates/auki-manifests/parking_lot.md) — read-side parsers + validators (deferred until a second reader needs them); `PoseSource` graduation to a sibling registry (deferred until a real SLAM/odometry producer lands); Pose Log manifest reshape gated on the 2026-05-07 synthesis (lands in Step 5); manifest-schema versioning convention.
- [`crates/auki-session-py/`](crates/auki-session-py/parking_lot.md) — `payload: bytes` encoding contract resolved (protobuf via auki-datatypes); libp2p control-plane design timing (deferred until this crate stabilizes); 6 resolved design decisions waiting to propagate when first implementation lands
