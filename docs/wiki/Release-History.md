# Release history

One entry per SDK tag from v0.0.50 onward. Tags older than v0.0.50 predate the #216 schema migration and aren't useful to current integrators — pull the [tag list](https://github.com/aukilabs/auki-sdk/tags) if you need them for archaeology.

Each entry summarizes what changed, who's affected, and any migration notes. The annotated git tag message (`git show vX.Y.Z`) is the canonical source; this page is the long-form narrative companion.

> **Cadence note.** The SDK is pre-1.0; breaking changes tick the patch version (v0.0.X). Downstream consumers should pin a tag rather than a branch. Major shifts get called out below.

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
