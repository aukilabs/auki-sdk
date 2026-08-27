# Release history

One entry per SDK tag from v0.0.50 onward. Tags older than v0.0.50 predate the #216 schema migration and aren't useful to current integrators — pull the [tag list](https://github.com/aukilabs/auki-sdk/tags) if you need them for archaeology.

Each entry summarizes what changed, who's affected, and any migration notes. The annotated git tag message (`git show vX.Y.Z`) is the canonical source; this page is the long-form narrative companion.

> **Cadence note.** The SDK is pre-1.0; breaking changes tick the minor version.
> Downstream consumers should pin a coordinated tag rather than a branch and
> upgrade communicating peers together.

---

## Unreleased v0.1.0 — authenticated Domain Stage 1

**Status:** source-complete migration guidance; the Stage 1 release gate and
coordinated tag are still pending. Do not substitute a Manager-era v0.0.x tag
for local Stage 1 evaluation.

- Native Rust and Python share one owned `auki-domain::Domain` runtime with
  host-supplied DDS authority, listeners, and exact-peer routes.
- Manager, membership, election, heartbeat, hidden Domain time, Discovery-owned
  startup, `NetworkRuntime`, and `auki-network-py` are removed from this line.
- The `auki-network` crate is removed. `auki-p2p` owns canonical identity and
  transport, `auki-protocols` owns exact wire contracts, and `auki-domain` owns
  hosting policy and protocol tasks.
- Retained application protocols negotiate only authenticated
  `/auki/auth/1/...` IDs; there is no legacy-wire fallback.
- `ServedProtocols` defaults to none; hosts opt in to each exact inbound
  protocol version while client operations remain available independently.
- `auki-domain-py==0.1.0` is paired exactly with
  `auki-session-py==0.1.0` from the same SDK build.
- The diagnostic CLI provides a real two-process direct-TCP catalog proof and
  fail-closed wrong-Domain/wrong-Peer/malformed-credential checks.
- The active Cargo workspace pins Rust `1.89.0`; `auki-domain`,
  `auki-protocols`, and `auki-p2p` are versioned `0.1.0`. Rust Domain and its
  wire-contract crate are consumed from the coordinated source tag, while the
  transport crate is published independently. Posemesh owns its dataset
  application protocol and pins this transport revision/version.

**Who's affected:** every native networking consumer. Upgrade a communicating
Rust/Python group together. The Manager-era Swift network and browser sources
are deleted from HEAD and remain available only at `v0.0.60` until their later
external authenticated-engine migrations.

**Migration:** follow the
[authenticated Domain migration guide](https://github.com/aukilabs/auki-sdk/blob/develop/docs/authenticated-domain-migration.md).

---

## v0.0.60 — SDK-native voxel mapping and scalar sensors

**Released:** 2026-08-07 · [`git show v0.0.60`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.60)

- Added SDK-native voxel mapping, replay checkpoints, frame aliases, map
  source color, and independent Mapper output clocks.
- Added application-owned detector execution and streamed detection-log
  resources.
- Added live typed messages and Python send/receive surfaces.
- Added the scalar sensor family across registries, logs, resources, streams,
  and Python bindings.
- Hardened stream teardown and expanded geometry conversion bindings.

**Who's affected:** Manager-era native, Swift, and browser consumers that need
the final pre-Stage-1 source line. This tag remains the pinned prior line while
authenticated Swift/browser engines are developed separately.

**Migration:** Do not mix `v0.0.60` peers with authenticated `v0.1.0` peers.
Upgrade a communicating Rust/Python group together, or keep the entire group on
this tag.

---

## v0.0.59 — Manager tiebreak hardening + heartbeat diagnostics

**Released:** 2026-06-25 · [`git show v0.0.59`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.59)

- **[#296](https://github.com/aukilabs/auki-sdk/pull/296) (#295)** Manager tiebreaker: Discovery-arbitrated promotion, step-down, and rejoin — closes races where two peers could each believe they're Manager.
- **[#294](https://github.com/aukilabs/auki-sdk/pull/294) (#293)** Identity confirmed libp2p-only: the HTTP `/api/info` control-API path for `ParticipantInfo` is removed. `ParticipantInfo` is now gated by cluster membership over `/auki/info/0.0.1` exclusively.
- **[#305](https://github.com/aukilabs/auki-sdk/pull/305) (#304)** Heartbeat write stalls instrumented for field diagnosis — logging only, no behavior change.
- **[#300](https://github.com/aukilabs/auki-sdk/pull/300)** `.understand-anything` knowledge-graph + dashboard added for GitHub Pages (repo-internal tooling, not an SDK surface).

**Who's affected:** Anyone polling HTTP `/api/info` for peer identity — that path is gone; use the libp2p `/auki/info/0.0.1` protocol instead. Cluster operators get more reliable Manager failover.

**Migration:** Drop any HTTP-based identity/`is_manager` polling added in v0.0.58 in favor of `/auki/info/0.0.1` over libp2p.

---

## v0.0.58 — Peer/Session/Domain doc sweep + control-api Manager fields

**Released:** 2026-06-10 · [`git show v0.0.58`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.58)

- **[#292](https://github.com/aukilabs/auki-sdk/pull/292) (#288)** Repo-wide doc sweep for the #274/#282 Peer/Session/Domain split — READMEs, the app-builder skill, and `docs/control-api.md` updated to the current shape.
- **[#284](https://github.com/aukilabs/auki-sdk/pull/284)** Session clock unified: `ClusterManager` reads time through the shared SDK clock primitive; `examples/diagnostic-app` becomes a proper peer instead of hand-rolling its own clock.
- Control API's `/api/info` gains `is_manager` + `manager_peer_id` fields. (Superseded in v0.0.59, which removes the HTTP `/api/info` identity path entirely — those two fields did not survive.)

**Who's affected:** Contributors reading docs/examples for the current Peer/Session/Domain shape.

**Migration:** None beyond the v0.0.57 Peer/Session/Domain split.

---

## v0.0.57 — Peer / Session / Domain split (#274)

**Released:** 2026-06-09 · [`git show v0.0.57`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.57) · **breaking**

- **[#282](https://github.com/aukilabs/auki-sdk/pull/282) (#274)** `auki-session` + bindings split into a three-layer API:
  - `Session::new(...)` → `Peer::new(...)` + `peer.start_session()`
  - Registry registration (sensor/frame/detector) moved from `Session` to `Peer`
  - `session.join_domain(cfg)` → `Domain::join(&peer, &session, cfg)` (`auki-domain`)
  - `session.catalog()` → `Domain::catalog()` / `auki_domain::catalog_of(&peer, &session)`
  - `auki-session` is now network-free; `auki-domain` depends on it, not the other way around.
  - Python (`auki_session`): new `Peer` class owns registration + `start_session()`; `Session` is thinned to `register_clock` + `register_*_log` + getters.
- **[#272](https://github.com/aukilabs/auki-sdk/pull/272)** Auki SDK app-builder skill added (after a false start reverted in #271).

**Who's affected:** Everyone on `auki-session` — Rust and Python. The catalog wire shape is unchanged, so this is an API-surface break, not a protocol break.

**Migration:** Replace `Session::new` call sites with `Peer::new(...).start_session()`; move sensor/frame/detector registration onto the `Peer`; replace `session.join_domain(...)` / `session.catalog()` with `Domain::join(...)` / `Domain::catalog()`.

---

## v0.0.56 — /auki/resources is the only peer discovery surface

**Released:** 2026-05-28 · [`git show v0.0.56`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.56) · **breaking**

- **[#253](https://github.com/aukilabs/auki-sdk/pull/253) (#251)** `/auki/sensors` protocol deleted entirely — `/auki/resources/0.2.0` is now the single peer-discovery contract. Removed: `SensorsRequest`/`SensorsResponse`, the protocol-flavored `SensorEntry`, `SENSORS_PROTOCOL`, `SensorCatalogProvider`, `ClusterManager::{set_sensor_catalog_provider, fetch_sensors_catalog, spawn_sensors_handler}`, `FetchSensorsCatalogError`, the matching Python wrappers, and the TS browser handling in `auki-domain-browser`. `proto/sensors.proto` deleted. No backwards-compat shim.
- **[#250](https://github.com/aukilabs/auki-sdk/pull/250) (#249)** `ResourceEntry.from_dict(d)` / `.from_json(s)` added to `auki_domain.ResourceEntry` — Python producers can now construct catalog rows for `set_resource_catalog_provider` instead of being stuck returning `[]`. All four variants (`sensor_log` / `pose_log` / `time_transform_log` / `detection_log`) supported.
- **[#248](https://github.com/aukilabs/auki-sdk/pull/248)** Retained-source-tail Tokio runtime bug fixed.
- Wiki pages drafted: [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs) (#242), [The Five Questions](The-Five-Questions) (#244, corrected in #247), this [Glossary](Glossary) (#245), [Crate map](Crate-Map) + this Release History page (#246).

**Who's affected:** Anyone still calling `fetch_sensors_catalog` / `set_sensor_catalog_provider` — both gone, no fallback.

**Migration:** Replace `fetch_sensors_catalog` with `fetch_resources_catalog` filtered to `variant == "sensor_log"` (matches the old `/auki/sensors` scope). Replace `set_sensor_catalog_provider` with `set_resource_catalog_provider` returning `ResourceEntry` rows built via `from_dict`/`from_json`. Python wire-shape note: `PoseSource`/`TimeTransformSource` are serde-tagged enums — pass `{"kind": "manual"}`, not a bare string.

---

## v0.0.55 — type unification + wiki Quickstart

**Released:** 2026-05-28 · [`git show v0.0.55`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.55)

- **[#243](https://github.com/aukilabs/auki-sdk/pull/243) (#236)** `auki-session-py` no longer redefines `RegistryRef` / `LogRef` locally — they come from `auki-registry-py`, the canonical source. `auki-session-py` depends on `auki-registry-py` and re-exports both classes in the `auki_session` namespace so existing `isinstance(x, auki_session.RegistryRef)` callers keep working. Input parsing duck-types via `getattr` / `getitem`, accepting any object with the right field names.
- **[#241](https://github.com/aukilabs/auki-sdk/pull/241) (#234)** Wiki [Quickstart](Quickstart) page drafted, replacing the placeholder from #229. Also fixes the wiki-mirror GitHub Action's `permissions: contents: write` so the mirror can push.

**Who's affected:** Python integrators get a cleaner type story — no more identically-named pyclasses across two packages. The Rust surface is unchanged.

**Migration:** None. Drop-in for v0.0.54 callers. `auki_session.RegistryRef` and `auki_registry.RegistryRef` are now the *same* class.

---

## v0.0.54 — Python bridge for #216 + GitHub wiki

**Released:** 2026-05-28 · [`git show v0.0.54`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.54)

- **[#231](https://github.com/aukilabs/auki-sdk/pull/231)** `auki-domain-py` resurrected from the 13-line stub the #216 schema migration left it in. ~3000-line PyO3 binding restored: `ClusterManager` + lifecycle constructors, `DaemonInfo`, `ParticipantInfo`, `ClusterMembership`, `ClusterMember`, error types, stream open, payload types, `StreamManifestBuilder`. Wire types updated to match #216 (new `ResourceEntry`, new `StreamRequest`, updated `ResourcesRequest.variants`). The deleted-in-#216 types (`SensorStreamResource`, `TransformEdgeResource`, `PoseStreamResource`) stay deleted.
- **[#229](https://github.com/aukilabs/auki-sdk/pull/229)** GitHub wiki set up with `docs/wiki/` source-of-truth in the main repo. CI mirrors to https://github.com/aukilabs/auki-sdk.wiki on push to `develop`. Initial structure: Home + section landings + six placeholder pages to be drafted in follow-up cards.
- **[#226 / #232](https://github.com/aukilabs/auki-sdk/pull/232)** Python `Session.with_storage_root` now preserves `session_id`. Adds `Session::set_storage_root(&self, root)` — an in-place mutator that the PyO3 wrapper can call instead of reconstructing via `Session::new()`. Resolves the v0.0.53 known issue.

**Who's affected:** Booster (Python) — unblocks the v0.0.53 migration. Park indirectly, through whatever uses `auki-domain-py`. Contributors get a wiki for narrative docs.

**Migration:** No breaking changes vs v0.0.53. Python apps that were stuck on v0.0.52 because `auki-domain-py` was a stub can now move to v0.0.54.

---

## v0.0.53 — SDK as robot data plane (#216)

**Released:** 2026-05-28 · [`git show v0.0.53`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.53) · **breaking**

- **[#216](https://github.com/aukilabs/auki-sdk/issues/216)** Registry / manifest / wire-shape schema migration. The big one. App code now uses `auki-session` as the declarative entry point; `auki-domain` becomes internal.
  - **Wire protocols** bumped: `/auki/resources/0.2.0`, `/auki/registries/0.2.0`, `/auki/stream/0.2.0`. Old clients cannot speak the new format.
  - **Registry entries** gain `peer_id`; disk paths gain a `peer_id` segment.
  - **`SensorBody`** restructured: `PointCloud` renamed to `Rangefinder` (with `point_cloud` becoming a `sensor.type`), new `Rf` variant for radio-frequency sensors, every body gains a `type: String`.
  - **Manifests** gain `source_peer_id` + `writer_peer_id` (canonical origin vs file writer split, required to make materialization correct).
  - **Catalog row** reshaped: `variant` discriminator (`sensor_log` / `pose_log` / `time_transform_log` / `detection_log`) + variant-specific `sensor` / `pose` / `manifest` blocks. `SensorStreamResource` / `TransformEdgeResource` / `PoseStreamResource` deleted. Static transforms are now sealed one-sample pose logs.
- **New crates:** [`crates/auki-session`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-session) (declarative app API) and [`bindings/python/auki-session-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-session-py).

**Who's affected:** Everyone. Booster, Park, Galbot adapters all need to migrate. See [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs) for the conceptual model the migration installed.

**Migration:** Wipe on-disk caches — the new SDK refuses to parse pre-#216 registry entries, manifests, or catalog rows. Replace hand-built manifests with `Session::register_*_log`. Replace `ClusterManager` direct calls with `Session::join_domain`. Filter `pose_log` rows where `pose.writer_mode == "rigid"` and `state == "sealed"` to replace the deleted `TransformEdgeResource` path.

**Known issues at release:** `with_storage_root` in `auki-session-py` regenerated `session_id`. Fixed in v0.0.54.

---

## v0.0.52 — enforce closed sensor kinds

**Released:** 2026-05-26 · [`git show v0.0.52`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.52)

- **[#217](https://github.com/aukilabs/auki-sdk/pull/217)** Sensor kinds become a closed enum at the registry layer. Producers can no longer write a sensor entry with an unrecognized `kind`; consumers can decode the kind without string parsing.

**Who's affected:** Anyone writing a `SensorRegistryEntry`. Open-ended kind strings are rejected at registration time.

**Migration:** Map any custom kinds onto the canonical set (`camera`, `point_cloud` → renamed in v0.0.53 to `rangefinder`, `audio`, `joint_encoders`). v0.0.53 then adds `rf` and renames `point_cloud` → `rangefinder` with `point_cloud` becoming a `sensor.type`.

---

## v0.0.51 — browser full peer + direct audio + protobuf control-plane

**Released:** 2026-05-22 · [`git show v0.0.51`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.51)

- **[#198](https://github.com/aukilabs/auki-sdk/pull/198)** Browser peers move to direct libp2p, dropping the relay-only intermediate. Browser dashboards (Park) can now reach native peers more efficiently.
- **[#193](https://github.com/aukilabs/auki-sdk/pull/193)** `auki-geometry` ships spatial transform composition helpers — the convention-conversion layer that sits underneath the still-pending full `convert_pose`.
- **[#181](https://github.com/aukilabs/auki-sdk/pull/181)** [`dataproducts.md`](https://github.com/aukilabs/auki-sdk/blob/develop/dataproducts.md) refreshed to match the shipped types (point cloud, audio, joint encoders, pose, time transforms, detection).
- **[#177](https://github.com/aukilabs/auki-sdk/pull/177)** Protobuf control-plane protocol types added — foundation for future structured protocols.
- **[#176](https://github.com/aukilabs/auki-sdk/pull/176)** Disk/wire mirror protos collapsed to one `Data` message per modality — eliminates the dual `*_stream` packages.
- **[#183](https://github.com/aukilabs/auki-sdk/pull/183)** Project board flow and git hygiene rules codified in [`CONTRIBUTING.md`](https://github.com/aukilabs/auki-sdk/blob/develop/CONTRIBUTING.md) (and [`CLAUDE.md`](https://github.com/aukilabs/auki-sdk/blob/develop/CLAUDE.md) for AI agents).

**Who's affected:** Park (browser) — better reachability. Anyone consuming the `*_stream` protobuf packages — collapsed in #176; switch to the single `<modality>::Data`.

---

## v0.0.50 — domain clock heartbeat sync + relocation

**Released:** 2026-05-21 · [`git show v0.0.50`](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.50)

- **[#161](https://github.com/aukilabs/auki-sdk/pull/161)** Heartbeat-driven domain clock sync. Heartbeats now carry NTP-style timing observations; each peer computes a `ClockTransformEstimate` against the Manager and composes with the Manager-announced domain clock to produce a live `DomainClockEstimate`. (See [The Five Questions § Temporal](The-Five-Questions#temporal--when-did-this-happen) for the live-vs-recorded distinction.)
- **[#162 / #158](https://github.com/aukilabs/auki-sdk/pull/158)** Diagnostic app and diagnostic-flash timestamp logging.
- **[#156](https://github.com/aukilabs/auki-sdk/pull/156)** Python bindings relocated under `bindings/python/` (away from sibling crate dirs). Cargo paths and import paths updated.
- **[#155](https://github.com/aukilabs/auki-sdk/pull/155)** Retained stream source bridge.
- **[#153](https://github.com/aukilabs/auki-sdk/pull/153)** Generic stream opener — the lower-level construct that the typed `open_*_stream` helpers wrap.
- **[#151](https://github.com/aukilabs/auki-sdk/pull/151)** `auki-network-swift` UniFFI Discovery binding (Stage 1).

**Who's affected:** Anyone with custom paths to `auki-*-py` — the binding paths moved to `bindings/python/`. Anyone needing cross-peer time alignment can now consume `ClusterManager::domain_clock_estimate(local_clock_id)`.

---

## Older tags

v0.0.49 and earlier predate the #216 schema and the `auki-session` declarative API. They live on for git history; integrators should not pin to them. The full list:

```bash
git tag --list --sort=-v:refname
```

For an individual older release:

```bash
git show vX.Y.Z
```

---

## See also

- [Top-level README](https://github.com/aukilabs/auki-sdk/blob/develop/README.md) — current shipped status per crate
- [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs) — the conceptual model #216 installed
- [Crate map](Crate-Map) — what each crate does today
- [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5) — what's in flight for the next release

---

[← Back to: Design + Architecture](Design-and-Architecture) · [Crate map →](Crate-Map)
