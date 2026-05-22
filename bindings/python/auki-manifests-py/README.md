# auki-manifests-py

PyO3 bindings for [`auki-manifests`](../../../crates/auki-manifests)'s four `build_*_log_manifest` builders. Each builder takes typed args and returns a Python `dict` mirroring the JCS-canonical JSON the Rust crate produces, so Python consumers can hand the dict to [`auki_logs.Log.open`](../auki-logs-py) without re-implementing field names, types, or ordering.

`PoseSource`, `PoseWriterMode`, and `TimeTransformSource` are taken as Python **dicts** / strings rather than PyClasses — matches the natural JSON shape Python callers think in.

**Status:** Shipped.

## Public surface

- `build_sensor_log_manifest(...)`
- `build_pose_log_manifest(..., source={"kind": "ros2_tf", "publishers": [...]}, writer_mode=...)`
- `build_time_transform_log_manifest(..., source={"kind": "local_clock_read"})`
- `build_detection_log_manifest(...)` — the one the [ESL detector](https://github.com/aukilabs/detectors) uses.

## Depends on

- [`auki-manifests`](../../../crates/auki-manifests) — Rust crate it wraps.
