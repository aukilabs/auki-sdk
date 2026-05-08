# Changelog — auki-manifests

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 8, 11:52 HKT, 2026

**`build_pose_log_manifest` rewritten** for Step 5 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md) per the 2026-05-07 synthesis. New signature takes 13 args: `from_frame_id` + `from_frame_hash`, `to_frame_id` + `to_frame_hash` (mirrors `build_time_transform_log_manifest`'s clock-pair pattern), `clock_id` + `clock_hash`, the inline `PoseSource`, plus `writer_mode: PoseWriterMode` (`Rigid` or `Movable`, JSON `"rigid"` / `"movable"`) and `expected_rate_hz: u32`. Each Pose Log holds samples for one ordered frame pair; producer fans multi-pair messages into N parallel logs.

**New `PoseWriterMode` enum** — `Rigid` (stationary transform; one observation reads back at any query time) or `Movable` (time-varying; readers interpolate or step-look-up). Snake-case serialization. Lives in the manifest, not on segment entries.

**Tests**: 6 → 7 (+1 `build_pose_log_manifest_serializes_writer_mode_as_snake_case`; existing `build_pose_log_manifest_contains_all_required_fields` updated for the new shape). All resolved tests pass.

**Resolved parking-lot question**: Pose Log manifest reshape per the synthesis. Replaced with a "Resolved 2026-05-08" pointer.

### broodsugar's dobby · May 8, 11:27 HKT, 2026

**Filed: Pose Log + TimeTransform Log self-provenance gap.** Per the [root subscription-as-materialization keystone](../../parking_lot.md#subscription-as-materialization-the-unified-detector-ingestion-architecture-filed-by-dobby-2026-05-08), recordings need to be self-provenant — but Pose Log identity (`from_frame_id` / `to_frame_id` post-Step-5) names coordinate systems, not devices, and `PoseSource::Ros2Tf { publishers }` carries the producer kind without device identity. Two robots both running ROS 2 TF would produce indistinguishable `PoseSource` values. TimeTransform Log can ride on the existing clock-ID convention if formalized. Three forward paths logged with a lean toward (a) — require frame IDs to follow the same `<platform-tag>-<machine-id>/<frame-name>` device-encoding shape sensor IDs use. Lower priority than the sensor-log fix because the Pose Log manifest is mid-rewrite (Step 5 of [`../auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md)); fold the fix into the Step 5 redesign. Doc-only.

### broodsugar's claude · May 8, 11:30 HKT, 2026

**Test scaffolding `LogPayload` impl** — companion change for Step 1 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md). `TestEntry` (the trivial body type the round-trip tests use to drive `auki_logs::Log<T>::open`) gained a `LogPayload` impl over ciborium so the test still compiles after the trait switch. No production-code change in this crate.

### broodsugar's claude · May 8, 09:00 HKT, 2026

**New crate scaffolded — Step 0 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md).** Pure refactor extracting the manifest builders from existing crates into a single owner. Symmetric with [`auki-datatypes`](../auki-datatypes), which owns segment payload shapes; this crate owns manifest shapes.

**Moved from [`auki-registry`](../auki-registry):**

- `build_sensor_log_manifest(...)` — Sensor Log family (covers Sensor, Point Cloud, Audio Logs).
- `build_pose_log_manifest(...)` — Pose Log; `source` describes the producer inline.
- `PoseSource` enum + `canonical_bytes` + `hash` — tagged-enum producer identity, lives inline in the Pose Log manifest under `"source"`.
- Locked vector test `ros2_tf_source_serializes_to_canonical_bytes` (M1 example → exact JCS bytes).
- Locked vector test `ros2_tf_source_hash_is_locked` (M1 example → `f3d296341347589c72297a0cc7c81cd8`). Cross-cutting guard against drift in `auki-jcs` / `auki-hash` / this crate's serde shape.

**Moved from [`auki-time-transforms`](../auki-time-transforms):**

- `build_manifest(...)` → renamed `build_time_transform_log_manifest(...)`. The original ambiguous name was fine when it was the only `build_manifest` in its crate; with three sibling builders in this crate the prefix disambiguates.

**Encoding decision pinned**: manifests stay JCS-canonical UTF-8 JSON via [`auki-jcs`](../auki-jcs), not protobuf. Reasons in [`README.md`](README.md). This crate is consumer-facing for both writers (the integrators producing the manifests) and readers (Park, future Sentinel) of `auki-logs::Log<T>` directories.

**Test count: 6.** All five tests carrying locked semantics from the source crates round-trip identically here; no values changed. Plus `build_time_transform_log_manifest_contains_required_fields` (renamed) preserves the original `build_manifest_contains_required_fields` semantics. `cargo test -p auki-manifests` clean.

**Companion changes** in [`auki-registry`](../auki-registry) and [`auki-time-transforms`](../auki-time-transforms): builders + `PoseSource` removed, tests deleted, READMEs trimmed (manifest tables now live here, not there). `auki-ros-adapter` and other downstream consumers don't touch manifests directly so are unaffected. Will land in v0.0.24.
