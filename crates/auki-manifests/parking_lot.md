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

## ✓ Resolved 2026-05-08 — Pose Log manifest reshape (Step 5)

Landed at Step 5 of [`../auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md). `build_pose_log_manifest` now takes 13 args and emits a manifest with `from_frame_id` + `from_frame_hash`, `to_frame_id` + `to_frame_hash`, `writer_mode` (`PoseWriterMode::Rigid | Movable`, JSON `"rigid"` / `"movable"`), and `expected_rate_hz: u32`. The pre-migration shape is gone; segment-side switched from `PoseLogEntry { transforms: Vec<...> }` to flat [`auki_datatypes::pose::SpatialTransform`](../auki-datatypes/src/lib.rs) at the same time.

## Manifest-side schema versioning vs auki-logs segment-format versioning

The README claims "schema version is 1 for all three manifest shapes" but there's no field in any manifest carrying that version number — the version is documented externally. Conversely auki-logs's segment format version IS in-band (the manifest's `version` field at the auki-logs level). Whether the manifest schema gains an explicit version field, or stays implicitly v1 documented externally, is a pin-before-second-version question. Lean: stay external until a v2 is actually staged; dual-versioning ahead of time is premature.

## Pose Log + TimeTransform Log self-provenance gap _(filed by Dobby, 2026-05-08)_

Per the [root subscription-as-materialization decision](../../parking_lot.md#subscription-as-materialization-the-unified-detector-ingestion-architecture-filed-by-dobby-2026-05-08), recordings need to be self-provenant — moving a log between peers must preserve "who produced this." Sensor logs solve this implicitly: `sensor_id` follows `<platform-tag>-<machine-id>/<sensor-name>` (e.g. `K1-AABBCCDDEEFF/head_left_cam`), so the producing device is encoded in the ID and survives every move.

Pose Log and TimeTransform Log have no analogous convention.

- **Pose Log:** identity will be `(from_frame_id, to_frame_id)` post-Step-5 synthesis (and `from_frame_hash` / `to_frame_hash`). Frame IDs name coordinate systems, not devices. `PoseSource` carries the producer *kind* (e.g. `Ros2Tf { publishers: ["amcl", "robot_state_publisher"] }`) but not the device. Two robots both running ROS 2 TF would produce indistinguishable `PoseSource` values. A pose-log recording subscribed from a peer doesn't carry "which physical robot's TF tree."
- **TimeTransform Log:** identity is `(from_clock_id, to_clock_id, from_clock_hash, to_clock_hash)`. If clock IDs follow the same `<platform-tag>-<machine-id>/<clock-name>` convention as sensor IDs, this case is already covered by the existing convention — but the recommendation is currently buried in the registry README and not lifted into TimeTransform Log's contract. Worth pinning.

**Forward paths for Pose Log:** (a) require frame IDs to follow a device-encoding convention (e.g. `K1-AABBCCDDEEFF/base_link`), making frame IDs themselves carry provenance. Symmetric with sensor IDs. (b) Add a separate `producer_peer_id` field to the Pose Log manifest. Reintroduces the `peer_id`-on-manifest pattern explicitly rejected for sensor logs in the root keystone. (c) Extend `PoseSource` variants with device identity (`Ros2Tf { publishers, machine_id }`). Makes the source enum carry both kind and device.

Lean: (a). Frame IDs are already structured strings; threading the device prefix through is a documentation move, not a schema change. Symmetric with sensor IDs. Catches the case where two robots' `base_link` frames collide today.

**Lower priority than the sensor-log fix** because the Pose Log manifest is mid-rewrite (Step 5 of [`../auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md)). Better to fold this into the Step 5 redesign than to land a fix on a shape that's about to change. Park here until Step 5 starts.
