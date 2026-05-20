# Changelog — auki-time

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 20, HKT, 2026

**`SessionClock` added.** `auki-time` now owns the SDK's session-monotonic clock primitive: peer-id rooted ids (`<peer_id>/<session_id>/monotonic`), a `ClockRegistryEntry` with `Scope::DeviceLocal` and session epoch marker, `clock_hash()`, `registry_entry()`, and monotonic `now_ns()` / `now_i64_ns()` readers. This gives heartbeat time sync and domain participant info one reusable clock identity instead of a heartbeat-specific abstraction. Added `auki-registry` as a dependency. Tests: `cargo test -p auki-time`.

### Nils's codex · May 20, HKT, 2026

**Crate renamed `auki-time-transforms` → `auki-time`.** The old name described the TimeTransform Log sampler only; the new name leaves room for the cleaner SDK timekeeping foundation (`SessionClock`, local clock reads, and future `convert_time` support) while keeping the existing sampler APIs in the same crate. Mechanical scope: directory rename (`crates/auki-time-transforms/` → `crates/auki-time/`), Cargo package/workspace/Cargo.lock rename, Rust crate import path rename (`auki_time_transforms` → `auki_time`), live docs/plan/path references, and parking-lot summaries. Historical changelog entries retain the old crate name as context.

### broodsugar's claude · May 8, 12:43 HKT, 2026

**`TimeTransformEntry` + `TimeTransformSource` departed at Step 6 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md).** `TimeTransformEntry` moved to [`auki-datatypes`](../auki-datatypes) under the `auki.time_transform` `.proto` package — protobuf via prost, two fields only (`offset_ns: i64`, `uncertainty_ns: u32`). `TimeTransformSource` moved to [`auki-manifests`](../auki-manifests) — manifest metadata, mirrors `PoseSource`. Both re-exported here for short call sites.

**Per-step decisions**: (a) per-entry `source` → manifest; (b) per-entry `discontinuous: bool` dropped (computed on read with reader's threshold); (c) `TimeTransformSource` kept as tagged enum at manifest layer (Option 2 — matches `PoseSource`'s extension pattern).

**Simplified `tick`/`Sampler`**: no more `SamplerState` (the prev-offset-tracking struct), no more `discontinuity_threshold` arg on `Sampler::start`. The sampler is now a pure `clock → entry` pipeline; readers handle discontinuity with their own thresholds. Manual `LogPayload` impl gone — covered by `auki-datatypes`' `impl_log_payload!` macro.

**Tests**: 10 → 2 (-8). Dropped: 5 discontinuity-detection tests (logic moved to readers; producer-side tests don't apply); `source_serializes_snake_case` (moved to [`auki-manifests`](../auki-manifests) where the type lives now); `entry_round_trips_through_cbor` (replaced by prost round-trip in [`auki-datatypes::tests`](../auki-datatypes/src/lib.rs)). Kept: `tick_computes_offset_uncertainty_and_timestamp` (math sanity check) + `sampler_writes_entries_then_stops_cleanly` (threaded integration test).

**Cargo.toml**: dropped `ciborium`, `serde`, `serde_json` (encoding moved to prost via `auki-datatypes`); added `auki-datatypes` and `auki-manifests` as path-deps for the re-exports.

**Docs**: README's "Entry payload (CBOR)" + "Discontinuity flag" sections rewritten — entry payload now lives in `auki-datatypes`; discontinuity is reader-side. `src/readme.md` reflects the simplified two-layer crate (no more "entry payload type" layer; just `tick` + `Sampler`).

### broodsugar's claude · May 8, 11:30 HKT, 2026

**`LogPayload` impl for `TimeTransformEntry` over ciborium.** Companion to Step 1 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md), which switched [`auki-logs`](../auki-logs)'s `Log<T>` bound from `T: Serialize + DeserializeOwned` to `T: LogPayload`. `TimeTransformEntry` doesn't move to a `.proto` until Step 6 of the migration, so the impl uses ciborium directly to preserve the on-disk encoding identically. Promoted ciborium from a dev-dep to a regular dep — gone again at Step 6. **No on-disk change.** Test count 9 → 9.

### broodsugar's claude · May 8, 09:00 HKT, 2026

**Step 0 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md).** `pub fn build_manifest(...)` moved to [`auki-manifests`](../auki-manifests) and renamed `build_time_transform_log_manifest` for unambiguity vs siblings (`build_sensor_log_manifest`, `build_pose_log_manifest`). The private `duration_as_i64_ns` helper was inlined at its one remaining production caller (`Sampler::start`'s threshold conversion). The `Sampler` integration test (`sampler_writes_entries_then_stops_cleanly`) now constructs its manifest via `auki_manifests::build_time_transform_log_manifest` — added as a new dev-dep on this crate. The `build_manifest_contains_required_fields` unit test moved to `auki-manifests` (renamed `build_time_transform_log_manifest_contains_required_fields`). README manifest section trimmed to a one-paragraph pointer at [`auki-manifests/README.md`](../auki-manifests/README.md). **Test count: 10 → 9** (one moved). All semantics preserved; pure refactor. Will land in v0.0.24.

### broodsugar's claude · May 7, 17:30 HKT, 2026

README manifest table: `session_id` row now references `/api/info`'s `session_id` instead of `/api/state`'s `session_uuid` (`/api/state` is gone in the v0.0.23 [Control API rewrite](../../docs/control-api.md)). Doc-only.

### broodsugar's claude · May 4, 10:38 HKT, 2026

`build_manifest` signature gains required `app_id: &str` and `session_id: &str` parameters, threaded into the resulting `serde_json::Value`. **Breaking API change** — every caller must now supply the application identifier and the session UUIDv4. Test assertions extended to cover the two new fields; sampler integration test updated to pass them through. Closes the implementation half of the `app_id` (May 4, 08:52) and `session_id` (May 4, 10:22) spec PRs.

### broodsugar's claude · May 4, 10:22 HKT, 2026

TimeTransform Log manifest gains a required `session_id: string` field — UUIDv4 minted by the integrator at app boot, same value as the parent session directory name and `/api/state`'s `session_uuid`. Mirrors the same change in the Sensor Log family manifest. Spec-only; implementation/tests pending.

### broodsugar's claude · May 4, 08:52 HKT, 2026

TimeTransform Log manifest gains a required `app_id: string` field, carrying the same identifier as the daemon's `/api/info` `app` value (e.g. `boosterapp`, `sentinel`). Mandatory addition; breaking against existing on-disk logs (acceptable under v0.x). Mirrors the same change in the Sensor Log family manifest. Implementation/tests still pending.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
