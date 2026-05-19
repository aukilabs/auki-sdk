# Changelog — auki-ros-adapter

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 19, HKT, 2026

**ROS pointcloud translation now emits native frames.** `build_point_cloud_log_entry` now returns `PointCloudFrame { point_count, data }`, converts ROS big-endian numeric fields to native little-endian bytes while repacking, respects `row_step` for organized source data, and validates that normalized registry layouts start with canonical `x/y/z` float32 fields. `build_point_cloud_registry_entry` drops `is_bigendian`, recomputing the locked pointcloud hash to **`d62ed811edcfb3e1a400f7aaa290eb85`**. Tests now cover native `point_count`, big-endian XYZ conversion, organized row padding, RGB/RGBA normalization under the native prefix, and non-color fields trailing XYZ.

### Nils's codex · May 19, HKT, 2026

**Implementation status doc names native pointcloud frames.** `src/readme.md` now documents `build_point_cloud_log_entry(msg) -> (timestamp_ns, PointCloudFrame)` and notes that ROS `PointCloud2` input is converted into native `{ point_count, data }`.

### Nils's codex · May 19, HKT, 2026

**README points pointcloud translation at the native frame.** Active docs now describe `build_point_cloud_log_entry(msg) -> (timestamp_ns, PointCloudFrame)` and `PointCloudFrame { point_count, data }`, with interpretation resolved through `SensorBody::PointCloud { fields, point_step, frame_id, frame_hash }`.

### Nils's codex · May 16, 17:53 HKT, 2026

**Frame hashes thread through the ROS registry builders.** `StaticCameraMetadata` gains `frame_hash: &'a str`, and `build_point_cloud_registry_entry(sensor_id, msg, frame_rate_hz, frame_id, frame_hash)` now writes both frame fields into the `auki-registry` body. The adapter stays pure translation: it does not scan registries or infer hashes; integrators pass the exact `FrameRegistryEntry` hash they wrote.

**Locked hashes recomputed in lockstep with `auki-registry`:** `build_rgb_camera_registry_entry_matches_m1_example_hash` and `end_to_end_translation_from_subscription_to_log_entry` now expect **`69f37478490cf1c0b226dbb86d3454fc`**; `build_point_cloud_registry_entry_matches_locked_hash` now expects **`2c480838a9be0b14608a8a0d72ee319f`**. The cross-crate equality remains the schema-parity guard.

**Tests:** 19 tests pass in the focused run with `auki-registry` and `auki-domain`.

### broodsugar's claude · May 8, 10:51 HKT, 2026

**`build_point_cloud_log_entry` now returns the prost type** — Step 3 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md). Function signature `(msg: &PointCloud2Msg) -> (i64, PointCloudLogEntry)` is unchanged, but `PointCloudLogEntry` is now [`auki_datatypes::point_cloud::PointCloudLogEntry`](../auki-datatypes/src/lib.rs) (re-exported here) instead of the departed `auki_registry::PointCloudLogEntry`. Construction now sets only `data`; `width` / `height` / `is_dense` are no longer carried per-frame. The producer-side ROS shape interpretation (`apply_normalization` using `msg.width × msg.height` for `num_points`) is unchanged — the result still gets flattened into the bytes the same way.

**Tests**: 21 → 19 (-2; dropped `point_cloud_log_entry_round_trips_through_cbor` and `point_cloud_log_entry_data_uses_cbor_byte_string`, both CBOR-specific encoding tests; the type is now prost). The remaining `build_point_cloud_log_entry_extracts_timestamp_and_data` test no longer asserts `width` / `height` / `is_dense`, just `timestamp + data length + data equality`.

### broodsugar's claude · May 8, 11:30 HKT, 2026

**`build_sensor_log_entry` now produces the prost type** — Step 1 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md). The function still has the same `(info, image) -> (i64, T)` shape, but `T` is now [`auki_datatypes::camera::PinholeCameraLogEntry`](../auki-datatypes/src/lib.rs) instead of the departed `auki_registry::SensorLogEntry`. `dynamic_intrinsics_from` returns `auki_datatypes::camera::DynamicIntrinsics` (same field names; prost-derived). `build_sensor_log_entry`'s constructor now wraps `Some(dynamic_intrinsics_from(info))` per the inline-optional decision; consumers that read the field handle the `Option<...>`.

**Re-exports updated**: `pub use auki_datatypes::camera::{DynamicIntrinsics, PinholeCameraLogEntry};` (camera log payload) and `pub use auki_registry::PointCloudLogEntry;` (still-CBOR; departs at Step 3). New regular dep on `auki-datatypes`. Module-doc + inline comment block updated to reflect the new home for the camera types.

**Tests**: `dynamic_intrinsics_extracts_correct_indices` / `_passes_distortion_through_unchanged` / `_accepts_empty_distortion_for_none_model` continue to pass — the prost-generated `DynamicIntrinsics` has the same field layout. `sensor_log_entry_round_trips_through_cbor` deleted; the equivalent prost round-trip lives next to the type definition in `auki-datatypes`'s locked-vector tests. Two callers (`build_sensor_log_entry_combines_info_and_image`, `end_to_end_translation_from_subscription_to_log_entry`) read `entry.dynamic_intrinsics.as_ref().unwrap().fx` per the `Option<...>` shape. `prost` added as dev-dep for the `Message` import. **Test count: 32 → 31** (one CBOR round-trip test deleted, no replacement needed). Will land in v0.0.24.

### broodsugar's claude · May 7, 11:00 HKT, 2026

**Breaking** — `build_rgb_camera_registry_entry` and `build_point_cloud_registry_entry` updated to thread the new `frame_id` field through to the `auki-registry` `RgbCamera` / `PointCloud` structs. `StaticCameraMetadata` gains `frame_id: &'a str`; `build_point_cloud_registry_entry` gains a `frame_id: impl Into<String>` parameter (positioned after `frame_rate_hz`, mirroring its out-of-band-supplied shape — `PointCloud2Msg` does not currently mirror ROS's `header.frame_id`, so integrators source the value from topic configuration or platform knowledge). Locked hashes recomputed in lockstep with `auki-registry`: `build_rgb_camera_registry_entry_matches_m1_example_hash` and `end_to_end_translation_from_subscription_to_log_entry` `e8cb38..` → `d798fa..`; `build_point_cloud_registry_entry_matches_locked_hash` `35b318..` → `79b58e..`. The same hashes are pinned in `auki-registry`'s tests; the cross-crate equality is the schema-parity guard. 22 tests still passing. Will land in v0.0.22 alongside the Frame Registry rollout.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
