# Changelog — auki-session

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

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
