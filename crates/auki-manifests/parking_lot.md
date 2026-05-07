# Parking lot — auki-manifests

Open questions for the `auki-manifests` crate. Cross-cutting questions live in the [root `parking_lot.md`](../../parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](../../CLAUDE.md) for the workflow.

---

## Read-side parsers + validators

This crate currently exposes only **builders** (the producer side of the manifest contract). The reader side — typed structs that deserialize a manifest, validate required fields, surface `clock_id` / `sensor_hash` etc. as typed values — does not yet exist; consumers grab fields by `manifest["clock_id"]` against a `serde_json::Value`.

Worth adding `pub struct SensorLogManifest` / `PoseLogManifest` / `TimeTransformLogManifest` with `Deserialize` impls + a `validate()` method that catches missing required fields at read time. Today's untyped read pattern is fine for happy path; typed parsers harden the contract.

Lean: add when a second reader (Park's Rust integration, future Sentinel) starts pulling manifests in earnest. Until then, builders alone are enough.

## `PoseSource` graduation to a sibling registry

Today `PoseSource` is inline in the Pose Log manifest. Per the [auki-registry README](../auki-registry/README.md) Pose Log section, *"if/when a producer variant brings substantial identity (SLAM with `map_id` + algorithm parameters), graduating `source` to a sibling registry is straightforward — extract the body into a content-addressed JSON file and replace the inline value with `(source_id, source_hash)`."*

`PoseSource::canonical_bytes` and `PoseSource::hash` already exist for exactly this graduation path. The decision to graduate is downstream of a real SLAM/odometry producer landing — pin then.

## Pose Log manifest reshape per the synthesis (2026-05-07)

The Pose Log manifest will gain `from_frame_id`, `from_frame_hash`, `to_frame_id`, `to_frame_hash`, `writer_mode` (`"rigid"` / `"movable"`), and `expected_rate_hz` per the synthesis decided 2026-05-07. `build_pose_log_manifest`'s signature changes accordingly. This is **Step 5** of [`../auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md) — keep `build_pose_log_manifest` in its current shape until then; the rewrite lands with the segment-side switch from `PoseLogEntry`-wrapper to flat `SpatialTransform`.

## Manifest-side schema versioning vs auki-logs segment-format versioning

The README claims "schema version is 1 for all three manifest shapes" but there's no field in any manifest carrying that version number — the version is documented externally. Conversely auki-logs's segment format version IS in-band (the manifest's `version` field at the auki-logs level). Whether the manifest schema gains an explicit version field, or stays implicitly v1 documented externally, is a pin-before-second-version question. Lean: stay external until a v2 is actually staged; dual-versioning ahead of time is premature.
