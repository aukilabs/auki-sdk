# Changelog — auki-ros-adapter

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 7, 11:00 HKT, 2026

**Breaking** — `build_rgb_camera_registry_entry` and `build_point_cloud_registry_entry` updated to thread the new `frame_id` field through to the `auki-registry` `RgbCamera` / `PointCloud` structs. `StaticCameraMetadata` gains `frame_id: &'a str`; `build_point_cloud_registry_entry` gains a `frame_id: impl Into<String>` parameter (positioned after `frame_rate_hz`, mirroring its out-of-band-supplied shape — `PointCloud2Msg` does not currently mirror ROS's `header.frame_id`, so integrators source the value from topic configuration or platform knowledge). Locked hashes recomputed in lockstep with `auki-registry`: `build_rgb_camera_registry_entry_matches_m1_example_hash` and `end_to_end_translation_from_subscription_to_log_entry` `e8cb38..` → `d798fa..`; `build_point_cloud_registry_entry_matches_locked_hash` `35b318..` → `79b58e..`. The same hashes are pinned in `auki-registry`'s tests; the cross-crate equality is the schema-parity guard. 22 tests still passing. Will land in v0.0.22 alongside the Frame Registry rollout.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
