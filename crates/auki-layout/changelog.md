# Changelog — auki-layout

(Renamed from `auki-session` 2026-05-08; entries below from before the rename retain the old crate name as historical context per the append-only convention.)

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 22, HKT, 2026

**Pose-log docs stop pointing at the deprecated datatypes migration path.** The path contract is unchanged; comments now describe the current per-`(from, to)` identity directly.

### Nils's codex · May 21, HKT, 2026

**Detector log references now use `DetectionFrame`.** Active docs/comments follow the SDK-wide detection payload rename; layout paths and hashing behavior are unchanged.

### Arshak's claude · May 16, HKT, 2026

**`detector_entry_path(app_root, detector_id, hash) -> PathBuf` lands** as the sibling of `sensor_entry_path` / `clock_entry_path` / `frame_entry_path`. Resolves to `<app_root>/registries/detectors/<detector_id>/<hash>.json`, with the standard `/` → `__` id substitution. New `DETECTORS_DIR = "detectors"` constant; layout doc gains the row.

Pairs with the new `auki_registry::DetectorRegistryEntry` (Cuba T4) landing in the same migration; the SDK's `set_registry_app_root` auto-serve handler uses this helper to find the file when peers ask via `/auki/registries/0.0.1` (Cuba T7's HTTP shim is being dropped).

**Tests**: 13 → 14 (+1 — `detector_entry_path_uses_detectors_dir_and_id_substitution`).

**Context**: Commit 2/6 of the Cuba v0.0.45 SDK migration. See [`exocortices/arshak/cuba/migration-plan-v0.0.45.md`](https://www.notion.so/35d5c8e96592803ab914fdc6f0a8aecd).

### broodsugar's claude · May 9, 12:40 HKT, 2026

**`detection_log_path(session_root, detector_id, input_log_id) -> PathBuf` lands** to close [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #2. Resolves to `<session>/detection_logs/<detector_id>__<input_log_id>/`, mirroring `poselog_path`'s `__`-joined shape. `detector_id`'s `/` are substituted to `__` (so `"aukilabs/qr/v1"` becomes `"aukilabs__qr__v1"`); `input_log_id` is opaque (typically the integrator-minted UUID `sensor_log_id` from `sensorlog_path`) and passes through unchanged. New `DETECTION_LOGS_DIR = "detection_logs"` constant.

**Layout doc updated.** The on-disk tree in [`lib.rs`](src/lib.rs) and [`README.md`](README.md) now lists `detection_logs/` alongside `sensorlogs/` and `poselogs/`. Tests: 11 → 13 (+2 — `detection_log_path_keys_on_detector_id_and_input_log_id`, `detection_log_path_substitutes_slashes_in_detector_id_only`).

Pairs with [`auki-manifests`](../auki-manifests)' new `build_detection_log_manifest` (same date) — together they let the integrator pre-create the output `Log<DetectionLogEntry>` and hand the write-handle to a detector loop. Caller-decides per the [keystone](../../parking_lot.md). Will land in v0.0.26.

### broodsugar's claude · May 8, 11:52 HKT, 2026

**`poselog_path` resigned to per-`(from, to)`-frame identity** for Step 5 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md). Old signature `(session_root, pose_log_id) -> PathBuf` (opaque integrator-minted ID) → new signature `(session_root, from_frame_id, to_frame_id) -> PathBuf`, mirroring `timetransform_log_path`'s per-clock-pair shape. On-disk: `<session>/poselogs/<from_id>__<to_id>` (each frame_id's `/` substituted to `__`).

**Tests**: 7 → 7 (`poselog_path_is_session_join_poselogs_join_pose_log_id` and `poselog_path_does_not_substitute_pose_log_id` removed; `poselog_path_uses_double_underscore_separator` and `poselog_path_substitutes_slashes_inside_each_frame_id` added — same pattern as `timetransform_log_path`'s tests).

### broodsugar's dobby · May 8, 09:34 HKT, 2026

**Crate renamed `auki-session` → `auki-layout`.** The previous name implied a runtime `Session` abstraction (lifecycle, clock binding, sensor-id minting) the crate doesn't provide; this crate is the *layout contract* — paths, lifecycle convention, ID encoding. The name `auki-session` is now reserved for the future Rust runtime counterpart of [`auki-session-py`](../../auki-session-py)'s in-process `Session` surface (per the [root `Session.open` Propagate item](../../parking_lot.md)). Mechanical scope: directory rename, `Cargo.toml` package name + description, workspace member entry, `Cargo.lock` package entry, [`auki-registry`](../auki-registry)'s path-dep + 6 `auki_session::` → `auki_layout::` call sites in `src/lib.rs` (5 path constructions + 1 doc comment), README headers, doc cross-references across the workspace (`auki-logs/README.md`, `auki-time-transforms/README.md`, `auki-registry/README.md` + `src/readme.md`, `auki-datatypes/src/sprint.md`, `docs/control-api.md`, root `README.md`, `Glossary.md`, root + `crates/` `parking_lot.md`). No behaviour change. Resolves the API-surface review item filed 2026-05-08 in [#56](https://github.com/aukilabs/auki-sdk/pull/56). The previous parking-lot item is replaced with a Resolved note in this crate's `parking_lot.md` per the convention.

### broodsugar's dobby · May 8, 07:56 HKT, 2026

[`parking_lot.md`](parking_lot.md) gains an item flagging the crate's name-vs-scope mismatch — `auki-session` exports only path-construction helpers today (no `Session` type, no lifecycle, no clock binding). Two forward paths sketched: (1) rename to `auki-paths` now while there are zero in-workspace `auki_session::*` consumers and reserve `auki-session` for the runtime abstraction the [root `Session.open` Propagate item](../../parking_lot.md) already specs; (2) footnote the README's API-surface row mirroring PR #55's "**Log payload types departing**" pattern for `auki-registry`. Lean: (1). Cross-references the root [`Session.open` Propagate item](../../parking_lot.md). Surfacing for Nils. Doc-only.

### broodsugar's claude · May 7, 17:30 HKT, 2026

Path-helper parameter rename to align with the [Control API rewrite](../../docs/control-api.md): `sensorlog_path(session_root, recording_uuid)` → `sensorlog_path(session_root, sensor_log_id)`; `poselog_path(session_root, recording_uuid)` → `poselog_path(session_root, pose_log_id)`. Two distinct identifier types for two distinct kinds of log — sensor logs and pose logs no longer share a single `recording_uuid` placeholder. Function signatures (parameter type, return type, behavior) are otherwise unchanged; **non-breaking for callers** since Rust functions don't take keyword args, and no in-workspace crate references these helpers yet. On-disk layout diagram in both `lib.rs` doc-comment and outer README updated: `<recording_uuid_N>` → `<sensor_log_id_N>` under `sensorlogs/`, `<recording_uuid_N>` → `<pose_log_id_N>` under `poselogs/`. Rationale prose: "A recording is one stream" → "A log is one stream"; the auto-started-buffer / 30s / `retention_ns: 0` framing dropped — daemon-application policy, not SDK contract per the new Control API spec. Test renames: `session_root_is_app_join_session_uuid` → `..._session_id`; `sensorlog_path_is_session_join_sensorlogs_join_recording` → `..._join_sensor_log_id`; `sensorlog_path_does_not_substitute_recording_uuid` → `..._does_not_substitute_sensor_log_id`; same pattern for the two poselog tests. Test count unchanged at 11; all pass. Versioning section also gains `poselogs/` to the breaking-changes list (was missing) and the per-log identifier layer (`<sensor_log_id>`, `<pose_log_id>`) replaces "the recording-uuid layer."

### broodsugar's claude · May 7, 11:00 HKT, 2026

New `frame_entry_path(app_root, frame_id, hash) -> PathBuf` returning `<app_root>/registries/frames/<frame_id>/<hash>.json` — peer to `sensor_entry_path` / `clock_entry_path`, same `id-with-slashes-replaced-by-__` segment convention. The on-disk layout diagram in both `lib.rs` doc-comment and outer README — which previously said `frames/<frame_id>/<hash>.json     ← coming` — now reflects the landed third registry. 1 new test; auki-session 10 → 11. Companion to the Frame Registry types added in `auki-registry` in the same PR.

### broodsugar's claude · May 4, 11:11 HKT, 2026

New `poselog_path(session_root, recording_uuid) -> PathBuf` returning `<session_id>/poselogs/<recording_uuid>` — peer to `sensorlog_path`, same opaque-uuid convention. On-disk layout diagram (in both `lib.rs` doc-comment and outer README) gains the `poselogs/<recording_uuid>/` subtree alongside `sensorlogs/`. Rationale section now phrases recordings as "one stream — sensor or pose source," not "one sensor stream." 2 new tests; auki-session now at 10 tests. Companion to the Pose Log payload + builder added in `auki-registry` in the same PR.

### broodsugar's claude · May 4, 10:22 HKT, 2026

Session lifecycle now formally specced: a session begins on app boot and ends when the daemon exits (cleanly or otherwise); the integrator generates a fresh UUIDv4 at boot and uses it as the session directory name and as `session_id` in every manifest written during the run. New "Session lifecycle" section in the README, plus a clarifying paragraph distinguishing `session_id` / `domain_id` / `scenegraph_id` (none derivable from another). Path diagram label `<session>` → `<session_id>`; API table parameter renamed `session` → `session_id` (function signature is unchanged — `session_root(app_root: &Path, session: &str)` still accepts any string; the rename is a doc-only clarity fix). Spec-only; no code changes. Companion to the `session_id` manifest field added in `auki-registry` and `auki-time-transforms` in the same PR.

### broodsugar's claude · May 4, 09:24 HKT, 2026

On-disk layout diagram now lists `tags.jsonl` as an optional TagClaim sidecar inside both `timetransform_logs/<from>__<to>/` and `sensorlogs/<recording_uuid>/` directories, with a pointer to root `tags.md`. Spec gap fix only — no code or path-helper changes.

### broodsugar's claude · May 1, 19:28 HKT, 2026

Recording shape pivot: a recording is now one sensor stream. Dropped the `<sensor_id>` sublayer from `sensorlog_path` — signature is now `sensorlog_path(session_root, recording_uuid) -> PathBuf`, returning `<session>/sensorlogs/<recording_uuid>`. The sensor identity moves out of the path and into the recording's `manifest.json` (`sensor_id` + `sensor_hash`). Multi-sensor capture = multiple parallel recordings sharing a session, not a multi-sensor recording. Buffer vs. intent capture distinguished only by `retention_ns` in the manifest. Updated the layout diagram in `lib.rs` doc-comment + outer README + inner readme. Tests: replaced `sensorlog_path_includes_recording_layer` with two new tests (`sensorlog_path_is_session_join_sensorlogs_join_recording`, `sensorlog_path_does_not_substitute_recording_uuid`); 8 tests total, all green. Breaking change for v0.0.6 consumers; v0.0.7 will be the consumer-coordination tag.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
