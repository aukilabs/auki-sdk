# Parking lot — crates

Cross-crate questions, plus a topic summary of per-crate parking lots.

---

## Schema versioning coordination

Each crate that owns a wire format pins its own version: `auki-logs` segment format v1, `auki-registry` entry schema v1, `auki-time-transforms` payload v1. They're independent today. When any one bumps to v2, what's the coordination story for consumers? Does the manifest need a per-log version field separate from the entry schema, or is the segment-format version the single source of truth?

## src/sprint.md per-crate scaffolding missing

The convention specifies `src/sprint.md` per crate (current work + next steps). None of the seven crates have one. They'd need seeding before the convention is fully realized.

## changelog.md per-crate scaffolding missing

Same gap as above for `changelog.md`. None exist. Needs seeding, with established history backfilled or just starting from "now."

---

## Per-crate parking lots

- [`auki-hash/`](auki-hash/parking_lot.md) — cryptographic strength upgrade path
- [`auki-jcs/`](auki-jcs/parking_lot.md) — `serde_jcs` upstream vendoring strategy
- [`auki-logs/`](auki-logs/parking_lot.md) — per-entry checksums; reader streaming for unbounded captures
- [`auki-registry/`](auki-registry/parking_lot.md) — Frame Registry shape; UTC clock epoch format; sensor_id naming convention formalization; atomic-write tmp cleanup
- [`auki-ros-adapter/`](auki-ros-adapter/parking_lot.md) — `r2r` typesupport blocker
- [`auki-session/`](auki-session/parking_lot.md) — TimeTransform log path encoding ambiguity
- [`auki-time-transforms/`](auki-time-transforms/parking_lot.md) — future `TimeTransformSource` variants
