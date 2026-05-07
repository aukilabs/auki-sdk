# Changelog — auki-manifests

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

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
