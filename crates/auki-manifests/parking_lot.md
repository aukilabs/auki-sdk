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

## DetectorRegistry shape — what does `detector_hash` actually hash? _(filed 2026-05-09, alongside `build_detection_log_manifest`)_

The Detection Log manifest's `(detector_id, detector_hash)` pair mirrors `(sensor_id, sensor_hash)` for sensors, but unlike Sensor Registry there's no `DetectorRegistryEntry` shape pinning what the hash covers. For v1 the manifest carries `detector_hash` as an opaque string; the integrator decides what goes in.

Forward paths:
- **(a) Hash a structured `DetectorRegistryEntry`** — e.g. `{ name, version, code_commit_sha, model_artifact_hash?, output_schema_hash, config_hash, ... }` JCS-serialized + XXH3-128. Symmetric with Sensor / Frame / Clock registry entries; pinning *what's* hashed gives provenance teeth.
- **(b) Hash the build artifact directly** — the binary blob (Rust crate's compiled `.so` / Python wheel hash / model weights). Strong but coarse: a recompile with a no-op change re-hashes.
- **(c) Hash a per-detector "what mattered" string the author defines** — frees the SDK from ever schematizing it; weakens cross-language reproducibility.

Lean: (a). Mirrors the rest of the registry pattern. Defer implementing until Park / Boosterapp need to surface "where did this detection come from?" in a UI — provenance pressure drives the schema, not the other way around.

The manifest field shape doesn't change when this lands — `detector_hash` is already an opaque hex string; the only thing that pins down is what bytes the producer hashed before writing the manifest.

## Uniform `intent` field across every manifest builder _(filed 2026-05-09)_

Per the [keystone's detection-log lifecycle entry](../../parking_lot.md), intent (`buffer | intent_recording`) applies to every log under the keystone, not just detection logs. The current builders (`build_sensor_log_manifest`, `build_pose_log_manifest`, `build_time_transform_log_manifest`, `build_detection_log_manifest`) all omit the field — match-the-existing-builders for v1.

The follow-on PR adds `LogIntent` (tagged enum, `buffer` / `intent_recording`, mirrors `PoseSource` / `TimeTransformSource` shape) and threads it through every builder together. The existing `build_detection_log_manifest_omits_intent_field` test pins the absence — it will need to flip when this lands.

Out of scope for the detector-binding PR because the rollout is broader than detection logs. File-and-revisit when a real consumer (subscription / republishing) needs it.

## ✓ Resolved 2026-05-08 — Pose Log manifest reshape (Step 5)

Landed during the May 8 payload migration. `build_pose_log_manifest` now takes 13 args and emits a manifest with `from_frame_id` + `from_frame_hash`, `to_frame_id` + `to_frame_hash`, `writer_mode` (`PoseWriterMode::Rigid | Movable`, JSON `"rigid"` / `"movable"`), and `expected_rate_hz: u32`. The pre-migration shape is gone; segment-side switched from `PoseLogEntry { transforms: Vec<...> }` to flat [`auki_proto::pose::SpatialTransform`](../auki-proto/src/lib.rs) at the same time.

## Manifest-side schema versioning vs auki-logs segment-format versioning

The README claims "schema version is 1 for all three manifest shapes" but there's no field in any manifest carrying that version number — the version is documented externally. Conversely auki-logs's segment format version IS in-band (the manifest's `version` field at the auki-logs level). Whether the manifest schema gains an explicit version field, or stays implicitly v1 documented externally, is a pin-before-second-version question. Lean: stay external until a v2 is actually staged; dual-versioning ahead of time is premature.

## Pose Log + TimeTransform Log self-provenance gap _(filed by Dobby, 2026-05-08)_

Per the [root subscription-as-materialization decision](../../parking_lot.md#subscription-as-materialization-the-unified-detector-ingestion-architecture-filed-by-dobby-2026-05-08), recordings need to be self-provenant — moving a log between peers must preserve "who produced this." Sensor logs solve this implicitly: `sensor_id` follows `<platform-tag>-<machine-id>/<sensor-name>` (e.g. `K1-AABBCCDDEEFF/head_left_cam`), so the producing device is encoded in the ID and survives every move.

Pose Log and TimeTransform Log have no analogous convention.

- **Pose Log:** identity will be `(from_frame_id, to_frame_id)` post-Step-5 synthesis (and `from_frame_hash` / `to_frame_hash`). Frame IDs name coordinate systems, not devices. `PoseSource` carries the producer *kind* (e.g. `Ros2Tf { publishers: ["amcl", "robot_state_publisher"] }`) but not the device. Two robots both running ROS 2 TF would produce indistinguishable `PoseSource` values. A pose-log recording subscribed from a peer doesn't carry "which physical robot's TF tree."
- **TimeTransform Log:** identity is `(from_clock_id, to_clock_id, from_clock_hash, to_clock_hash)`. If clock IDs follow the same `<platform-tag>-<machine-id>/<clock-name>` convention as sensor IDs, this case is already covered by the existing convention — but the recommendation is currently buried in the registry README and not lifted into TimeTransform Log's contract. Worth pinning.

**Forward paths for Pose Log:** (a) require frame IDs to follow a device-encoding convention (e.g. `K1-AABBCCDDEEFF/base_link`), making frame IDs themselves carry provenance. Symmetric with sensor IDs. (b) Add a separate `producer_peer_id` field to the Pose Log manifest. Reintroduces the `peer_id`-on-manifest pattern explicitly rejected for sensor logs in the root keystone. (c) Extend `PoseSource` variants with device identity (`Ros2Tf { publishers, machine_id }`). Makes the source enum carry both kind and device.

Lean: (a). Frame IDs are already structured strings; threading the device prefix through is a documentation move, not a schema change. Symmetric with sensor IDs. Catches the case where two robots' `base_link` frames collide today.

**Lower priority than the sensor-log fix** because the Pose Log manifest was mid-rewrite when this was filed. Better to fold this into the Pose Log redesign than to land a fix on a shape that's about to change.
