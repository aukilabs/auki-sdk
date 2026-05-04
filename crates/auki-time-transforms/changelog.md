# Changelog — auki-time-transforms

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 4, 08:52 HKT, 2026

TimeTransform Log manifest gains a required `app_id: string` field, carrying the same identifier as the daemon's `/api/info` `app` value (e.g. `boosterapp`, `sentinel`). Mandatory addition; breaking against existing on-disk logs (acceptable under v0.x). Mirrors the same change in the Sensor Log family manifest. Implementation/tests still pending.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
