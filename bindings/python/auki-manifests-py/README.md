# auki-manifests-py

PyO3 bindings for [`auki-manifests`](../../../crates/auki-manifests)'s four `build_*_log_manifest` builders. Each builder takes typed args and returns a Python `dict` mirroring the JCS-canonical JSON the Rust crate produces, so Python consumers can hand the dict to [`auki_logs.Log.open`](../auki-logs-py) without re-implementing field names, types, or ordering.

All four manifest builders now have `source_peer_id` and `writer_peer_id` as separate parameters. `source_peer_id` is the data origin peer; `writer_peer_id` is the peer writing this manifest file (may differ when a remote peer materializes the log).

`PoseSource`, `PoseWriterMode`, and `TimeTransformSource` are taken as Python **dicts** / strings rather than PyClasses — matches the natural JSON shape Python callers think in.

**Status:** Shipped.

## Public surface

- `build_sensor_log_manifest(source_peer_id, writer_peer_id, app_id, session_id, sensor, clock, frame, ...)` — `sensor`, `clock`, `frame` are `RegistryRef`-shaped dicts `{ "peer_id": ..., "id": ..., "hash": ... }`.
- `build_pose_log_manifest(source_peer_id, writer_peer_id, ..., from_frame, to_frame, clock, source={"kind": "ros2_tf", "publishers": [...]}, writer_mode=...)` — `from_frame`, `to_frame`, `clock` are `RegistryRef` dicts.
- `build_time_transform_log_manifest(source_peer_id, writer_peer_id, ..., from_clock, to_clock, source={"kind": "local_clock_read"})` — `from_clock`, `to_clock` are `RegistryRef` dicts.
- `build_detection_log_manifest(source_peer_id, writer_peer_id, ..., detector, input_log, input_sensor, clock, ...)` — `detector`, `input_sensor`, `clock` are `RegistryRef` dicts; `input_log` is a `LogRef`-shaped dict `{ "source_peer_id": ..., "resource_id": ... }`.

## Depends on

- [`auki-manifests`](../../../crates/auki-manifests) — Rust crate it wraps.
