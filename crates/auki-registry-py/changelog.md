# Changelog — auki-registry-py

Detailed changes for `auki-registry-py`. Latest entry on top.

---

### Nils's codex · May 19, HKT, 2026

**Python pointcloud registry constructors follow the native layout contract.** `point_cloud_sensor_entry(...)` no longer accepts `is_bigendian`; Python producers declare fields, `point_step`, cadence, and frame reference only. `write_sensor` now surfaces the Rust native-layout validation error for non-canonical XYZ metadata.

### Nils's codex · May 16, 19:02 HKT, 2026

**Initial Python Registry bindings.** Added `auki_registry`, a dict-oriented PyO3 wrapper over `auki-registry` for Python producer sidecars. The module exposes frame constructors (`frame_ros_body`, `frame_ros_optical`, `frame_opengl`, `frame_unity`, and field-explicit `frame_entry`), sensor constructors (`rgb_camera_sensor_entry`, `point_cloud_sensor_entry`, `audio_sensor_entry`, `joint_encoders_sensor_entry`, plus `point_field`), clock constructors, canonical JSON / hash helpers, and hash-pinned `write_*` / `read_*` storage helpers.

**Boosterapp convention declaration path closed.** Python can now write a `FrameRegistryEntry`, feed the returned `frame_hash` into a spatial sensor entry, write the sensor entry, then use `auki_domain.StreamManifestBuilder.from_registry(...)` to produce accept-time manifests with `frame_id` + `frame_hash`. `write_sensor` delegates to Rust validation, so missing or empty frame references fail loudly with no directory scanning.

**Tests:** Rust PyO3 smoke tests cover module export, frame write/read, point-cloud sensor write/read, missing-frame rejection, and the documented flow through the real Python module call signatures. Python surface tests cover the same public flow at wheel level.
