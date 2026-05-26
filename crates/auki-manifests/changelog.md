# Changelog — auki-manifests

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 24, HKT, 2026

Swift package template now excludes the generated XCFramework directory from the source target while retaining it as a binary target, removing SwiftPM unhandled-file warnings from generated package builds.

### Nils's codex · May 24, HKT, 2026

Opted the dev-only `auki-logs` dependency out of default features after `auki-logs` adopted the binding standard, keeping manifest round-trip tests on the direct Rust log API without inheriting generated binding dependencies.

### Nils's codex · May 24, HKT, 2026

**Multiplatform binding standard added.** The manifest DTOs, builders, validation helpers, and source canonicalization/hash helpers moved into binding-free `src/core.rs`; `src/lib.rs` now only wires feature-gated modules and re-exports the existing Rust `serde_json::Value` builder API. Native `src/ffi.rs` exposes UniFFI adapters for Python and Swift, taking nanosecond counts instead of `Duration` and returning JCS-canonical JSON strings. `src/wasm.rs` exposes wasm-bindgen adapters for JavaScript/WebAssembly with the same JSON-string return shape. The crate now declares `staticlib` / `cdylib` / `rlib`, default `uniffi`, `cli`, and `wasm` features, a crate-local `uniffi-bindgen` binary, `bindings.toml`, and crate-owned Python, Swift, and JavaScript package templates plus a JavaScript smoke vector.

**Compatibility.** Rust call sites continue to import the same crate-root builders, enums, DTOs, and validators. Consumers that do not need generated bindings should depend on `auki-manifests` with `default-features = false`; the in-workspace `auki-time` dependency has been updated that way.

**Tests.** Added `tests/surface.rs` to pin crate-root source compatibility. Existing core unit tests remain the manifest/vector behavior lock; generated package checks cover Python import, JavaScript wasm smoke, and SwiftPM build output.

### Nils's codex · May 24, HKT, 2026

Opted the `auki-jcs` dependency out of default features after `auki-jcs` adopted the binding standard, keeping manifest source hashing on the direct Rust canonicalization API without pulling in UniFFI.

### Nils's codex · May 24, HKT, 2026

Opted the `auki-hash` dependency out of default features after `auki-hash` adopted the binding standard, keeping manifest construction on the direct Rust hash API without pulling in UniFFI.

### Nils's codex · May 22, HKT, 2026

**Manifest docs no longer link to removed datatypes crate.** Current README, sprint, and parking-lot text now describe segment payload protobuf ownership as `auki-proto` / root `proto/auki` while preserving the May 8 migration history.

### Nils's codex · May 22, HKT, 2026

**Manifest docs now point segment-payload readers at `auki-proto`.** The active README frames manifests as the JCS half and generated `auki-proto` payloads as the protobuf half; manifest JSON behavior is unchanged.

### Nils's codex · May 21, HKT, 2026

**Manifest docs now describe `CameraFrame` payloads.** Active examples and comments follow the SDK-wide stream payload vocabulary cleanup; manifest schema behavior is unchanged.

### broodsugar's claude · May 9, 12:40 HKT, 2026

**`build_detection_log_manifest` lands** to close [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #2 (Detector binding API). Mirrors the existing builders' shape — JCS-canonical JSON via [`auki-jcs`](../auki-jcs), 11 args:

```rust
pub fn build_detection_log_manifest(
    app_id, session_id,
    detector_id, detector_hash,            // content-addressed producer identity
    input_log_id,                          // sensor_log_id of the input log
    input_sensor_id, input_sensor_hash,    // copied from input log for self-containedness
    clock_id, clock_hash,
    segment_duration, retention,
) -> serde_json::Value;
```

**Producer identity mirrors `(sensor_id, sensor_hash)`.** `detector_id` is namespaced (`"aukilabs/qr/v1"`); `detector_hash` is a content-hash binding the producer to a specific build (e.g. `hash(commit-SHA + config)` for code-only detectors, `hash(commit-SHA + weights + config)` for ML detectors). The exact `DetectorRegistryEntry` shape — what bytes go through the hasher — is **deferred** to a sibling PR; the manifest carries `detector_hash` as an opaque hex string for v1, and the SDK doesn't validate it.

**Self-containedness.** The detection log copies `input_sensor_id` + `input_sensor_hash` from the input log's manifest. A reader holding only the detection log can still know what sensor produced its inputs, even after the input sensor log is evicted by retention.

**No `intent` field.** Per the [keystone's intent-decoupling entry](../../parking_lot.md), `buffer | intent_recording` applies to every log, but adding it uniformly across the existing builders is a separate PR — match-the-existing-builders for v1. Filed in [`parking_lot.md`](parking_lot.md).

**Caller-decides lifecycle.** Per PR #72's lean: the integrator (Park / Boosterapp) calls `build_detection_log_manifest` + [`auki_layout::detection_log_path`](../auki-layout/src/lib.rs) to pre-create the output `Log<DetectionLogEntry>`, then hands the write-handle to the detector loop. The detector itself doesn't construct the manifest.

**Tests**: 9 → 12 (+3 — `build_detection_log_manifest_contains_all_required_fields`, `build_detection_log_manifest_omits_intent_field` (pins absence so the future uniform-intent PR has a failing test to update), `detection_log_manifest_opens_a_log_round_trip` (end-to-end seam through `auki-logs::Log<T>`)).

**Filed alongside in [`parking_lot.md`](parking_lot.md):**
- **DetectorRegistry shape** — what does `detector_hash` actually hash? Lean: a structured `DetectorRegistryEntry { name, version, code_commit_sha, model_artifact_hash?, output_schema_hash, config_hash, ... }` symmetric with Sensor / Frame / Clock registry entries. Defer until Park / Boosterapp need to surface "where did this detection come from?" in a UI.
- **Uniform `intent` field across every manifest builder** — file-and-revisit when subscription / republishing makes it concrete.

Will land in v0.0.26.

### broodsugar's claude · May 8, 12:43 HKT, 2026

**`TimeTransformSource` lands here; `build_time_transform_log_manifest` gains a `source` arg** for Step 6 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md). Tagged-enum mirror of `PoseSource` (one variant today, `LocalClockRead`); the manifest's new `"source"` field carries inline producer identity. Moved over from [`auki-time-transforms`](../auki-time-transforms) where it was a per-entry field on `TimeTransformEntry` pre-Step-6 — manifest is its right home (matches Pose Log's shape).

**Tests**: 7 → 9 (+2: `local_clock_read_source_serializes_to_canonical_bytes` pins JCS canonical bytes `{"kind":"local_clock_read"}`; `local_clock_read_source_hash_is_locked` pins XXH3-128 `8dcea0b9b0b2219d651e0856f112cd65`). Existing `build_time_transform_log_manifest_contains_required_fields` updated for the new arg.

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
