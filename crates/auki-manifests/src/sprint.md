# Sprint — auki-manifests

Current work and next steps. Spec: [outer `README.md`](../README.md).

## Now

`build_detection_log_manifest` landed 2026-05-09 to close [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #2 (Detector binding API). Mirrors the existing builders' shape; carries `(detector_id, detector_hash)` content-addressed producer identity plus `(input_log_id, input_sensor_id, input_sensor_hash)` for self-containedness. No `intent` field — match-the-existing-builders for v1; uniform `intent` rollout across every builder is filed below.

Step 0 of the [`auki-datatypes` migration](../../auki-datatypes/src/sprint.md) — pure refactor extracting builders + `PoseSource` from `auki-registry` and `auki-time`. No behavior change, no encoding change.

## Next, gated on the broader migration

1. **Pose Log manifest reshape** (Step 5 of the auki-datatypes sprint, per the 2026-05-07 synthesis): `build_pose_log_manifest`'s signature gains `from_frame_id`, `from_frame_hash`, `to_frame_id`, `to_frame_hash`, `writer_mode`, `expected_rate_hz`. The Pose Log identity becomes `(from, to)` per the [Notion Pose Log doc](https://www.notion.so/34b5c8e9659280bd9580c25991f5d491). Don't pre-rewrite — lands together with the segment-side switch from `PoseLogEntry`-wrapper to flat `SpatialTransform` to keep the manifest + segment shape changes atomic.

2. **TimeTransform Log manifest reshape** (Step 6 of the auki-datatypes sprint): `build_time_transform_log_manifest` gains a `source: TimeTransformSource` field as the per-entry `source` moves up to the manifest (slop fix in [`../auki-datatypes/parking_lot.md`](../../auki-datatypes/parking_lot.md)).

3. **Read-side parsers + validators** (open question in [`parking_lot.md`](../parking_lot.md)): typed `SensorLogManifest` / `PoseLogManifest` / `TimeTransformLogManifest` / `DetectionLogManifest` structs with `Deserialize` impls + `validate()`. Adds when a second reader (Park's Rust integration, future Sentinel) starts pulling manifests in earnest.

4. **PoseSource graduation to a sibling registry** (open in [`parking_lot.md`](../parking_lot.md)): if a real SLAM/odometry producer brings substantial identity, `PoseSource` extracts to a content-addressed file (the existing `canonical_bytes` + `hash` are exactly the graduation primitives).

5. **DetectorRegistry shape** (filed alongside the Detection Log manifest — see [`parking_lot.md`](../parking_lot.md)): the manifest's `(detector_id, detector_hash)` pair currently uses opaque-string `detector_hash`; the `DetectorRegistryEntry` shape that pins exactly what's hashed (commit SHA + model weights + config? schema + binary?) is deferred until Park / Boosterapp have a concrete provenance UX driving the choice.

6. **Uniform `intent` field across every manifest builder**: the keystone's `buffer | intent_recording` dimension applies to every log, but the Detection Log manifest currently omits it for parity with siblings. Add `LogIntent` enum + the field to all four builders together; the existing `build_detection_log_manifest_omits_intent_field` test pins the absence and will need to flip when this lands. Lean: tagged enum at the manifest layer, mirrors `PoseSource` / `TimeTransformSource`.

## Out-of-band

- Manifest encoding stays JCS-canonical JSON forever — pinned in the [auki-datatypes parking-lot](../../auki-datatypes/parking_lot.md). Don't reopen.
- Segment payload encoding (protobuf) is the [`auki-datatypes`](../../auki-datatypes) crate's concern; this crate stays JCS-only.
