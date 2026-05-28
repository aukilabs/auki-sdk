# auki-layout

On-disk path layout for the Auki SDK — the single source of truth for `<app_root>/registries/...`, `<app_root>/<session_id>/sensorlogs/...`, pose-log, time-transform-log, and detection-log paths. Plus the `id_to_segment` substitution rule (the `/` → `__` mapping) every consumer needs.

Lives at the bottom of the workspace so any consumer can ask "where does this go on disk?" without depending on logs, registries, or networking.

**Status:** Shipped.

## Public surface

- `registries_root(app_root)`
- `sensor_entry_path(app_root, peer_id, sensor_id, hash)`, `clock_entry_path(app_root, peer_id, clock_id, hash)`, `frame_entry_path(app_root, peer_id, frame_id, hash)`, `detector_entry_path(app_root, peer_id, detector_id, hash)` — all registry path helpers now take `peer_id` as a parameter; disk layout is `registries/<kind>/<peer_id>/<id>/<hash>.json`.
- `session_root(app_root, session_id)`
- `timetransform_log_path(session_root, from_id, to_id)`
- `sensorlog_path(session_root, sensor_log_id)`
- `poselog_path(session_root, from_frame_id, to_frame_id)`
- `detection_log_path(session_root, detector_id, input_log_id)`
- `id_to_segment(id)` — the substitution rule used inside all of the above.

## Depends on

Nothing in the workspace.
