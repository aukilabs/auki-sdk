# Parking lot — root

Open architectural questions and cross-cutting design decisions for the Auki SDK as a whole. Crate-specific questions live in each crate's `parking_lot.md`; cross-crate questions live in [`crates/parking_lot.md`](crates/parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](CLAUDE.md) for the workflow.

---

## Hagall (Networking) — SDK-side open questions _(filed by Nils's claude, 2026-05-13)_

The [Hagall quest](https://www.notion.so/35e5c8e9659280e69b86f5edc32641a0) is a clean rewrite of Auki peer-to-peer cluster networking — Vinland and Greenland are reference, not foundation. The [SDK plan subpage](https://www.notion.so/35f5c8e9659281b3afa7e713bcc89a50) maps the SDK's responsibilities into 12 SDK tasks and 5 design questions; 4 questions resolved 2026-05-13. **Only SDK-Q3 remains open.** Decision trail lives in [`changelog.md`](changelog.md) and the SDK plan's status log.

- [**SDK-Q3**](crates/auki-domain/parking_lot.md#sdk-q3--hagall-successor-token-format-bare-signed-json-jwt-or-prost-in-auki-datatypes-_filed-by-nilss-claude-2026-05-13_) — Successor-token encoding (prost / JWT / bare signed JSON). Lean: prost, ~60%. **Not gating implementation** — v1 Discovery contract skips signature verification entirely, so this defers to v2.

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

## Propagate: Pose Log capture shape (Step 5 resolution, 2026-05-08)

Step 5 of the [`auki-datatypes` migration](crates/auki-datatypes/src/sprint.md) landed the synthesis decided 2026-05-07: per-`(from, to)` identity instead of per-producer; flat [`SpatialTransform`](crates/auki-datatypes/src/lib.rs) segment entries (no `PoseLogEntry` wrapper, no per-sample `parent_frame`/`child_frame`); manifest carries `from_frame_id` + `from_frame_hash`, `to_frame_id` + `to_frame_hash`, `writer_mode` (`"rigid"` / `"movable"`), and `expected_rate_hz`. `build_pose_log_manifest` rewritten in [`auki-manifests`](crates/auki-manifests); [`auki-layout`](crates/auki-layout)'s `poselog_path` now mirrors `timetransform_log_path`'s `(from, to)`-keyed shape.

[`dataproducts.md`](dataproducts.md)'s `FrameTransformAvailability` is still marked "TBD — pending Pose Log"; refresh to reflect that the shape is now concrete: `log_handle` references `<session>/poselogs/<from_id>__<to_id>/`, the manifest carries `(from_frame_id, from_frame_hash)` + `(to_frame_id, to_frame_hash)` resolving via the Frame Registry. `convert_pose` itself is still pending — capture and read are in place; composition / path-finding is not.

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

## Subscription-as-materialization (the unified Detector ingestion architecture) _(filed by Dobby, 2026-05-08)_

**Decision: when a node subscribes to a peer's sensor stream, the SDK materializes a local sensor log on that node's disk, structurally identical to a locally-captured one. There is one ingestion primitive — tail a local sensor log — and the Detector binds to that, regardless of whether the bytes were captured here, subscribed from a peer, or opened from a file.**

This is the architectural keystone for the [`detectors`](https://github.com/aukilabs/detectors) example repo, but it ripples through the whole SDK. The decision unifies three cases that would otherwise need separate code paths:

1. **Own sensor.** Local sensor driver writes the log; Detector tails it.
2. **Peer's sensor stream.** Subscription writes the log to my disk; Detector tails it. The "live stream" is just the log being actively appended-to over the wire.
3. **Recording.** A file on disk is the log; Detector tails it. A "recording" is colloquially what we call a log whose writer has detached. There is no separate recording format.

The Detector code is identical across all three. The SDK handles transport (zero-hop in case 1, libp2p in case 2, file-source in case 3). Confidence: high — the recording symmetry argument (case 3 ≡ case 2 because both are "consuming a log I don't currently own") is what locks it in. Any architecture where recordings need special-casing has the wrong shape.

**What this implies (each item is a sub-decision worth pinning):**

- **Subscription = materialization.** "I'm subscribed to peer X's sensor S" means "I have a sensor log on my disk that is being live-appended-to from peer X." Closing the subscription stops the appending; the bytes stay. Every subscriber is therefore a potential republisher (BitTorrent / IPFS shape: download = seed) — see redistribution policy below.
- **Detection Logs work the same way.** A Detector's output log can be subscribed to by peers; their subscription materializes a copy on their disk. The Detector / Detection Log / sensor log story is one primitive: "publishable log, subscribers materialize copies." Otherwise the abstraction is split.
- **Two log intents at creation time: `buffer | intent_recording`.** Set at subscription-init, immutable for the log's life. **Buffer** = sliding window (ring-buffer eviction by size or time), used by Detectors that only need recent frames. **Intent recording** = keep-forever, used when you'll replay, audit, train on, or share onward. When the subscriber starts the log they decide which one, before any bytes flow. (Promotion / demotion changes the meaning of "what bytes do I have," and either should require an explicit operation that creates a new log, not a runtime knob.)
- **Log identity for dedup.** Buffer logs dedup on `sensor_manifest_hash` — one buffer log per sensor per node, window size = max of consumers' requests, shrinks as consumers leave. Intent-recording logs are **strict** per `start_recording()` call — even two callers requesting identical sensor + identical range get two separate logs. Identity is the call, not the request shape; no coincidental sharing. This keeps the planned-range-immutable invariant clean.
- **Planned range is immutable, actual fill can right-truncate.** A log's planned range is set at creation (`[T0, T1]` for an intent recording, "from-now-onward-until-closed" for a live subscription). The actual filled portion can be a right-truncated subrange if the source died early. Recorded in the log manifest as `planned_range` + `truncated_at` (nullable). No mid-range gaps; if you want to splice multiple ranges from multiple peers, you make a new log explicitly.
- **Redistribution policy: permissive (for now).** Subscribing materializes the bytes; the bytes outlive the publisher's session. A subscriber can serve them onward to a third peer. This is a real change from the implicit "if I stop publishing, the data is gone" model. Permissive is the right starting point for an OSS spatial SDK; capability-flag (manifest carries `redistribute_allowed: yes/no`) and crypto-enforcement (encrypted logs with revocable per-subscriber keys) are forward paths if a security tier emerges. Pin tighter only when an integrator needs the constraint.
- **Provenance is in the manifest, by way of `sensor_id`.** A recording is self-provenant because `sensor_id` follows the `<platform-tag>-<machine-id>/<sensor-name>` convention (e.g. `K1-AABBCCDDEEFF/head_left_cam` — the MAC encodes the producing device). Move the log to another peer, archive it for a year, replay it — `sensor_id` stays. The Sensor Registry entry travels with the log too (content-addressed at `<app_root>/registries/sensors/<id>/<hash>.json`; same `(id, hash)` produces byte-identical JCS-JSON, so registry entries are interchangeable across peers). No additional `peer_id` field needed on the manifest — the producer is encoded in the ID. Confidence: high. The convention is currently a recommendation, not enforced — see the [auki-registry parking-lot item](crates/auki-registry/parking_lot.md) for the hardening question. The pose-log analog is weaker; see the [auki-manifests parking-lot item](crates/auki-manifests/parking_lot.md).

**Where this lives.** The unified subscription primitive doesn't have a single home today — it spans `auki-network` (transport), `auki-logs` (the log abstraction that gets shared between writer and reader regardless of origin), and `auki-manifests` (the metadata that travels with a log). Implementation will probably extend `auki-logs` with subscription-source semantics (a `Log<T>::open_subscribed(peer, sensor_id, intent)` constructor that returns a write handle filled by a libp2p subscription), and `auki-network` with the wire protocol. Naming the home is downstream of the implementing PR; for now this is filed at root because it cuts across crates.

**What this enables.** Once this lands, the [`detectors`](https://github.com/aukilabs/detectors) example repo's three reference Detectors (QR, ESL, people) are all written against one ingestion API — `Log<SensorLogEntry>::tail()` — and they don't care about local-vs-remote-vs-recording. The corresponding `auki-sdk-py` Python binding (also referenced in `detectors/parking_lot.md`) becomes "wrap `Log<T>::tail()` and `Log<DetectionLogEntry>::append()`," not "wrap a sensor-callback API plus a separate replay API."

**What this defers.** Three sub-decisions are explicitly **not** pinned in this entry, to keep the keystone landable on its own merits: the buffer-log dynamic-resizing semantics when consumers come and go, the manifest field shape for `planned_range` + `truncated_at` (lives at [auki-manifests](crates/auki-manifests/parking_lot.md)), and storage backpressure when disk write throughput lags the network. File-and-revisit when the implementing PRs need them.

**✓ Read-side primitive landed 2026-05-08 — `Log<T>::tail()` in [`auki-logs`](crates/auki-logs).** The simplest viable shape: starts at current EOF, polls the segments directory at a configurable cadence (default 10ms), blocking `Iterator::next()` plus a non-blocking `try_next()`. Handles segment rollover, mid-write torn reads (surfaces as `Ok(None)`), and segment eviction. No EOF detection, no `tail_from(timestamp_ns)` checkpoint, no notify-based backend — those are filed as `auki-logs` parking-lot follow-ups for when a real consumer needs them. The transport-side of the keystone (libp2p materialization of a peer's stream into a local log) and the producer-side `Log::open_subscribed` constructor are still pending; tail is the read API both will eventually feed into. Resolves [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #1.

---

## Detection log lifecycle = sensor log lifecycle, with intent decoupled per detector instance _(filed by Dobby, 2026-05-08)_

**Decision: a detection log is `Log<DetectionLogEntry>` — same primitive as a sensor log, same `buffer | intent_recording` choice at creation, same dedup / redistribution / provenance story. The intent is chosen by the detector instance, independently of the input sensor log's intent.**

This is the producer-side closure of the [Subscription-as-materialization](#subscription-as-materialization-the-unified-detector-ingestion-architecture-filed-by-dobby-2026-05-08) keystone. The keystone established that detection logs subscribe-and-materialize like sensor logs on the consumer side; this entry says they're created with the same lifecycle on the producer side. There is no "DetectionLog" abstraction — just `Log<T>` with `T = DetectionLogEntry` and an intent picked at start. Confidence: high.

**Why intent must decouple from the input log's intent.** The intent of a log is a function of who consumes its bytes downstream, not of who produced its inputs upstream. Concrete cases:

- Camera intent-recording, detector buffer — capturing a 10-min walkthrough for replay, but only the live "is this shelf empty right now" alert matters. Raw frames preserved; detection log is a sliding window.
- Camera buffer, detector intent-recording — don't keep huge raw video forever, but do keep a permanent compliance trail of every QR scanned. Sensor evicts; detection log is durable.
- Same sensor, multiple detectors, different intents each — one camera log gets tailed by ESL (buffer, live shelf state), QR (intent-recording, audit trail), people-counter (buffer, real-time foot traffic). Three detectors, three independent intent choices, one input feed.

The detector instance picks its own intent based on its downstream consumers. The input log's intent is irrelevant to that decision.

**Open sub-question: buffer-intent dedup identity for detection logs.** Sensor logs dedup buffer-intent on `sensor_manifest_hash` — one buffer log per sensor per node. The natural detection-log analog is `(detector_id, input_log_id)` — two detectors of the same kind running on the same input feed share a buffer log; two detectors on different input feeds don't. Likely correct, but worth pinning explicitly before [`auki-logs`](crates/auki-logs) implements it. Intent-recording detection logs are strict-per-call (mirrors the sensor side) and don't have this question.

**✓ Resolved 2026-05-09 — caller-decides.** Adjudicated in favour of (a). The detector binding API ([`auki-layout::detection_log_path`](crates/auki-layout) + [`auki-manifests::build_detection_log_manifest`](crates/auki-manifests), 2026-05-09) hands the integrator the path + manifest; the caller opens `Log<DetectionLogEntry>` and passes the write-handle to the detector loop. Mirrors Park's existing log-lifecycle ownership for sensor logs — one consistent owner across all log types.

**Implication for Detector phase 2 in [`detectors`](https://github.com/aukilabs/detectors).** The `DetectionLogEntry` payload type can land in [`auki-datatypes`](crates/auki-datatypes) cleanly as a new `auki.detection` package (mirrors `auki.camera`, `auki.point_cloud`, etc.). No new lifecycle code — it inherits everything from `Log<T>`. This sharpens phase-2 blocker #3 ("`DetectionLogEntry` type") into a well-shaped single PR.

**✓ Resolved 2026-05-08 (Step 8 of the [`auki-datatypes` migration](crates/auki-datatypes/src/sprint.md)) — `auki.detection.DetectionLogEntry { bytes data = 1; }` (opaque-bytes-only, same stance as Steps 3 / 4) + `LogPayload` impl + locked conformance vectors landed.** The detection schema is per-Detector — the SDK doesn't interpret detector-specific fields (a QR detector emits portal-uid + corners; an ESL detector emits class + bbox + confidence; a people detector emits person bboxes); carrying detector-specific fields on the prost type would either lock the SDK into knowing every detector's schema or force a degenerate `oneof` of every shipped detector. Closes the producer side of the keystone for [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #3.

**✓ Resolved 2026-05-09 — Detector binding API landed in [`auki-layout`](crates/auki-layout) + [`auki-manifests`](crates/auki-manifests).** `detection_log_path(session_root, detector_id, input_log_id) -> PathBuf` resolves to `<session>/detection_logs/<detector_id>__<input_log_id>/`; `build_detection_log_manifest(...)` carries `(detector_id, detector_hash)` content-addressed producer identity and copies `(input_log_id, input_sensor_id, input_sensor_hash)` from the input log for self-containedness. Caller-decides lifecycle. Closes [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #2.

**✓ Resolved 2026-05-09 — Python SDK binding landed at the bytes level in [`auki-logs-py`](crates/auki-logs-py).** New PyO3 crate exposing `Log.open / append / flush / read / tail`, `LogReader.entries`, blocking + non-blocking iterator, context-manager protocol. Opaque-bytes-only — mirrors the Rust crate's `LogPayload` philosophy (no Python equivalent trait; Python users decode prost themselves). Closes [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #4 at the bytes level. The remaining piece for typed-message convenience is `betterproto`-generated `auki-datatypes` Python types — Step 9 of the [`auki-datatypes` migration sprint](crates/auki-datatypes/src/sprint.md), filed as the next dependency. Companion `auki-layout-py` + `auki-manifests-py` (path / manifest helpers) filed as a sibling follow-up. Two follow-ups filed in [`auki-manifests/parking_lot.md`](crates/auki-manifests/parking_lot.md): the `DetectorRegistryEntry` shape (what bytes go through the `detector_hash` hasher; deferred until Park / Boosterapp need provenance UX); uniform `intent` field across every manifest builder (the keystone's `buffer | intent_recording` dimension applies to every log but the existing builders all omit it, so the detection-log builder matches them — file-and-revisit when subscription / republishing makes it concrete).

---

## Subfolder summary

- [`crates/`](crates/parking_lot.md) — schema versioning coordination; sprint.md scaffolding still missing; **Rust vs Python surface namespacing mismatch** (filed 2026-05-08)
- [`crates/auki-datatypes/`](crates/auki-datatypes/parking_lot.md) — `.proto` package naming convention; field number allocation strategy; locked conformance vector format; schema versioning policy; **structured prost fields vs opaque bytes — when does each apply?** (filed 2026-05-09 after [#77](https://github.com/aukilabs/auki-sdk/pull/77) made the split precedent visible; lean: structured if single canonical interpretation, opaque-bytes if multiple layouts or schema-owned-downstream). **All per-type slop fixes resolved.** Step 0 (extract `auki-manifests`) **landed 2026-05-08**; Step 1 (`auki.camera` — `PinholeCameraLogEntry` + `DynamicIntrinsics`) **landed 2026-05-08**; Step 2 (`auki.frame_stream` + `auki.point_cloud_stream` + `auki.stream` envelope, libp2p substream wire) **landed 2026-05-08** — full envelope moves to protobuf, drops `base64` dep on the swarm path; `/auki/stream/1.0.0` retired in favour of `/auki/stream/0.1.0`; sibling rename of `/auki/cluster/1.0.0` → `0.0.1` and `/auki/identify/1.0.0` → `0.0.1`. Step 3 (`auki.point_cloud` — `PointCloudLogEntry`, opaque-bytes-only) **landed 2026-05-08** — symmetric with the wire's `PointCloudFrame { bytes }`; resolves the on-disk-vs-wire drift slop point. Step 4 (`auki.audio` — `AudioLogEntry`, opaque-bytes-only) **landed 2026-05-08** — same stance; resolves the implicit-vs-explicit chunk metadata slop point; drops `serde_bytes` from [`auki-registry`](crates/auki-registry). Step 5 (`auki.pose` — flat `SpatialTransform` + `Vec3` + `Quat`; `PoseLogEntry` wrapper gone; per-`(from, to)` Pose Log identity) **landed 2026-05-08** — coordinated reshape of [`auki-manifests`](crates/auki-manifests)' `build_pose_log_manifest` (now takes frame-pair + `writer_mode` + `expected_rate_hz`) and [`auki-layout`](crates/auki-layout)'s `poselog_path` (mirrors `timetransform_log_path`); drops `ciborium` from [`auki-registry`](crates/auki-registry). Resolves the 2026-05-07 synthesis. Step 6 (`auki.time_transform` — `TimeTransformEntry { offset_ns, uncertainty_ns }`) **landed 2026-05-08** — `source` moved to manifest as a tagged-enum `TimeTransformSource` (mirrors `PoseSource`); `discontinuous: bool` dropped (computed on read); `Sampler` simplified (no more `SamplerState` or threshold arg); resolves all three remaining slop points. Step 7 (placeholder cleanup) **landed 2026-05-08**. Step 8 (`auki.detection` — `DetectionLogEntry`, opaque-bytes-only) **landed 2026-05-08** — closes the producer side of the [Detector keystone](#detection-log-lifecycle--sensor-log-lifecycle-with-intent-decoupled-per-detector-instance-filed-by-dobby-2026-05-08); per-Detector schema, SDK doesn't interpret. **On-disk migration complete; every payload type the SDK ships lives here.**
- [`crates/auki-geometry/`](crates/auki-geometry/parking_lot.md) — no open questions yet
- [`crates/auki-manifests/`](crates/auki-manifests/parking_lot.md) — read-side parsers + validators (deferred until a second reader needs them); `PoseSource` graduation to a sibling registry (deferred until a real SLAM/odometry producer lands); manifest-schema versioning convention; **Pose Log + TimeTransform Log self-provenance gap** (filed 2026-05-08; Step 5 landed without folding the fix in — to be addressed in a follow-up). Pose Log manifest reshape **resolved 2026-05-08** at Step 5.
- [`crates/auki-session-py/`](crates/auki-session-py/parking_lot.md) — `payload: bytes` encoding contract resolved (protobuf via auki-datatypes); libp2p control-plane design timing (deferred until this crate stabilizes); 6 resolved design decisions waiting to propagate when first implementation lands
- [`crates/auki-domain-py/`](crates/auki-domain-py/parking_lot.md) — Phase 2 follow-ups: `stream_provider` Python callable, `runtime.open_*_stream()` consumer methods, `update_cluster_doc()` SSE feed, `DomainAlreadyExists.existing` full ClusterDoc — all four blocked on the `auki-network` ↔ `auki-network-py` `[lib] name` collision; recommended resolution path is renaming `auki-network-py`'s lib to `auki_network_py` with maturin's `module-name` preserving the Python import path (filed 2026-05-13)
