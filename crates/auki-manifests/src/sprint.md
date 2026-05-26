# Sprint — auki-manifests

Current work and next steps. Spec: [outer `README.md`](../README.md).

## Now

`build_detection_log_manifest` landed 2026-05-09 to close [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #2 (Detector binding API). Mirrors the existing builders' shape; carries `(detector_id, detector_hash)` content-addressed producer identity plus `(input_log_id, input_sensor_id, input_sensor_hash)` for self-containedness. No `intent` field — match-the-existing-builders for v1; uniform `intent` rollout across every builder is filed below.

The May 8 payload migration extracted builders + `PoseSource` from `auki-registry` and `auki-time` into this crate. No behavior change, no encoding change.

The crate now follows the SDK binding standard. Rust keeps the direct `serde_json::Value` builder API in `core.rs`; UniFFI and wasm-bindgen adapters return JCS-canonical JSON strings for generated Python, Swift, and JavaScript packages.

## Next, gated on the broader migration

1. **Pose Log manifest reshape** (landed during the May 8 payload migration, per the 2026-05-07 synthesis): `build_pose_log_manifest`'s signature gains `from_frame_id`, `from_frame_hash`, `to_frame_id`, `to_frame_hash`, `writer_mode`, `expected_rate_hz`. The Pose Log identity becomes `(from, to)` per the [Notion Pose Log doc](https://www.notion.so/34b5c8e9659280bd9580c25991f5d491). It landed together with the segment-side switch from `PoseLogEntry`-wrapper to flat `SpatialTransform` to keep the manifest + segment shape changes atomic.

2. **TimeTransform Log manifest reshape** (landed during the May 8 payload migration): `build_time_transform_log_manifest` gains a `source: TimeTransformSource` field as the per-entry `source` moves up to the manifest.

3. **Generated binding smoke expansion**: add Python and Swift package-level smoke programs that parse the returned manifest JSON and compare the same vectors as Rust/JavaScript. Current local gate covers Python import and SwiftPM build.

4. **Read-side parsers + validators** (open question in [`parking_lot.md`](../parking_lot.md)): typed `SensorLogManifest` / `PoseLogManifest` / `TimeTransformLogManifest` / `DetectionLogManifest` structs with `Deserialize` impls + `validate()`. Adds when a second reader (Park's Rust integration, future Sentinel) starts pulling manifests in earnest.

5. **PoseSource graduation to a sibling registry** (open in [`parking_lot.md`](../parking_lot.md)): if a real SLAM/odometry producer brings substantial identity, `PoseSource` extracts to a content-addressed file (the existing `canonical_bytes` + `hash` are exactly the graduation primitives).

6. **DetectorRegistry shape** (filed alongside the Detection Log manifest — see [`parking_lot.md`](../parking_lot.md)): the manifest's `(detector_id, detector_hash)` pair currently uses opaque-string `detector_hash`; the `DetectorRegistryEntry` shape that pins exactly what's hashed (commit SHA + model weights + config? schema + binary?) is deferred until Park / Boosterapp have a concrete provenance UX driving the choice.

7. **Uniform `intent` field across every manifest builder**: the keystone's `buffer | intent_recording` dimension applies to every log, but the Detection Log manifest currently omits it for parity with siblings. Add `LogIntent` enum + the field to all four builders together; the existing `build_detection_log_manifest_omits_intent_field` test pins the absence and will need to flip when this lands. Lean: tagged enum at the manifest layer, mirrors `PoseSource` / `TimeTransformSource`.

## Out-of-band

- Manifest encoding stays JCS-canonical JSON forever. Don't reopen.
- Segment payload encoding (protobuf) is the [`auki-proto`](../../auki-proto) crate's concern; this crate stays JCS-only.
