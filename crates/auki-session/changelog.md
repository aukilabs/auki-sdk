# Changelog — auki-session

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 1, 19:28 HKT, 2026

Recording shape pivot: a recording is now one sensor stream. Dropped the `<sensor_id>` sublayer from `sensorlog_path` — signature is now `sensorlog_path(session_root, recording_uuid) -> PathBuf`, returning `<session>/sensorlogs/<recording_uuid>`. The sensor identity moves out of the path and into the recording's `manifest.json` (`sensor_id` + `sensor_hash`). Multi-sensor capture = multiple parallel recordings sharing a session, not a multi-sensor recording. Buffer vs. intent capture distinguished only by `retention_ns` in the manifest. Updated the layout diagram in `lib.rs` doc-comment + outer README + inner readme. Tests: replaced `sensorlog_path_includes_recording_layer` with two new tests (`sensorlog_path_is_session_join_sensorlogs_join_recording`, `sensorlog_path_does_not_substitute_recording_uuid`); 8 tests total, all green. Breaking change for v0.0.6 consumers; v0.0.7 will be the consumer-coordination tag.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
