# Changelog — root

Append-only timeline of changes across the repo. Detailed entries land in the most-specific (leaf) `changelog.md`; one-liners propagate up through every parent to here. See [CLAUDE.md](CLAUDE.md) for the propagation rules and entry format.

Latest entry on top.

---

### broodsugar's claude · May 2, 13:50 HKT, 2026

Audio sensor support added to `auki-registry`: `SensorBody::Microphone` variant + `AudioLogEntry` payload type. PCM-only in v1; multi-mic arrays modelled as one sensor with `channels = N`. Schemas + canonical-bytes test + locked hash + write/read round-trip; 85 workspace tests green (was 82). Cross-app surface unchanged — this is additive; no consumer-coordination tag yet (waiting until an audio source is actually being captured).

### broodsugar's claude · May 1, 19:28 HKT, 2026

Session-shape revision: a recording is now one sensor stream. Dropped the `<sensor_id>` sublayer from `<session>/sensorlogs/<recording_uuid>/`; recording directories are now complete `auki-logs` log dirs (manifest + segments) for exactly one sensor. Multi-sensor capture = multiple parallel recordings sharing a session. Buffer vs. intent distinguished only by `retention_ns`. `auki-session::sensorlog_path` signature dropped its `sensor_id` parameter. Updated diagrams across root README, `auki-session/README.md` + inner readme, and the path-example bullets in `auki-logs/README.md` and `auki-registry/README.md`. Breaking from v0.0.6; v0.0.7 will be the consumer-coordination tag.

### broodsugar's claude · May 1, 15:56 HKT, 2026

Added [`docs/control-api.md`](docs/control-api.md) — the v1 cross-app HTTP control API spec. Six endpoints (`/api/state`, `/api/preview/latest.jpg`, `/api/recordings`, `/api/recordings/<id>`, `/api/buffer`, `/api/quit`) plus the `_auki._tcp.local.` mDNS discovery convention. Lets BoosterApp, Sentinel, and future daemons share one operator-control surface so [Park](https://github.com/aukilabs/park) can drive any of them through a single contract — implementing the "Park is the unified operator UI" architectural decision (Reid quest, May 1). Cross-linked from the root README under a new "Operator control API" section. Parked the open registries-app-rooted-vs-domain-scoped question in [`parking_lot.md`](parking_lot.md) as a separate evolution of the session shape.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Bootstrapped `changelog.md` at every folder level — root, `crates/`, and all seven crates. Prior history lives in git log; this changelog tracks changes from this point forward. Same PR also fixed an existing convention violation: open questions buried inside `tags.md` and `dataproducts.md` moved to root [`parking_lot.md`](parking_lot.md), where they belong per the project's parking-lot convention. Removed the now-resolved "changelog.md per-crate scaffolding missing" item from `crates/parking_lot.md`.
