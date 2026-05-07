# Changelog — auki-time-transforms

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

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
