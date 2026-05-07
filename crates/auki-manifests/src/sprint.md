# Sprint — auki-manifests

Current work and next steps. Spec: [outer `README.md`](../README.md).

## Now

Step 0 of the [`auki-datatypes` migration](../../auki-datatypes/src/sprint.md) — pure refactor extracting builders + `PoseSource` from `auki-registry` and `auki-time-transforms`. No behavior change, no encoding change. Crate just landed; tests pass.

## Next, gated on the broader migration

1. **Pose Log manifest reshape** (Step 5 of the auki-datatypes sprint, per the 2026-05-07 synthesis): `build_pose_log_manifest`'s signature gains `from_frame_id`, `from_frame_hash`, `to_frame_id`, `to_frame_hash`, `writer_mode`, `expected_rate_hz`. The Pose Log identity becomes `(from, to)` per the [Notion Pose Log doc](https://www.notion.so/34b5c8e9659280bd9580c25991f5d491). Don't pre-rewrite — lands together with the segment-side switch from `PoseLogEntry`-wrapper to flat `SpatialTransform` to keep the manifest + segment shape changes atomic.

2. **TimeTransform Log manifest reshape** (Step 6 of the auki-datatypes sprint): `build_time_transform_log_manifest` gains a `source: TimeTransformSource` field as the per-entry `source` moves up to the manifest (slop fix in [`../auki-datatypes/parking_lot.md`](../../auki-datatypes/parking_lot.md)).

3. **Read-side parsers + validators** (open question in [`parking_lot.md`](../parking_lot.md)): typed `SensorLogManifest` / `PoseLogManifest` / `TimeTransformLogManifest` structs with `Deserialize` impls + `validate()`. Adds when a second reader (Park's Rust integration, future Sentinel) starts pulling manifests in earnest.

4. **PoseSource graduation to a sibling registry** (open in [`parking_lot.md`](../parking_lot.md)): if a real SLAM/odometry producer brings substantial identity, `PoseSource` extracts to a content-addressed file (the existing `canonical_bytes` + `hash` are exactly the graduation primitives).

## Out-of-band

- Manifest encoding stays JCS-canonical JSON forever — pinned in the [auki-datatypes parking-lot](../../auki-datatypes/parking_lot.md). Don't reopen.
- Segment payload encoding (protobuf) is the [`auki-datatypes`](../../auki-datatypes) crate's concern; this crate stays JCS-only.
