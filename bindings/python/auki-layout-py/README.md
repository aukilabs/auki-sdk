# auki-layout-py

PyO3 bindings for [`auki-layout`](../../../crates/auki-layout) — pure-function wrappers around the on-disk path helpers. Lets Python consumers (e.g. the ESL detector in [`detectors`](https://github.com/aukilabs/detectors)) compute SDK-canonical paths without re-implementing the `__`-substitution + directory-name conventions.

No state, no PyClasses. Each function returns a `str`.

**Status:** Shipped.

## Public surface

- `registries_root(app_root)`
- `sensor_entry_path(app_root, sensor_id, hash)`, `clock_entry_path(...)`, `frame_entry_path(...)`
- `session_root(app_root, session)`
- `timetransform_log_path(session_root, from_id, to_id)`
- `sensorlog_path(session_root, sensor_log_id)`
- `poselog_path(session_root, from_frame_id, to_frame_id)`
- `detection_log_path(session_root, detector_id, input_log_id)`
- `id_to_segment(id)`

## Depends on

- [`auki-layout`](../../../crates/auki-layout) — Rust crate it wraps.
