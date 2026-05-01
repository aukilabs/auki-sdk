# auki-session

Path helpers for the Auki SDK's on-disk session shape. Single source of truth for the layout that registries, logs, and sessions occupy under an app root — so that the SDK, downstream consumers, and any cross-language reimplementation all agree on where things live.

## On-disk layout

```text
<app_root>/
├── registries/
│   ├── sensors/<sensor_id>/<hash>.json   ← shared across all sessions of this app
│   ├── clocks/<clock_id>/<hash>.json
│   └── frames/<frame_id>/<hash>.json     ← coming
└── <session>/
    ├── timetransform_logs/<from_id>__<to_id>/
    │   ├── manifest.json
    │   └── segments/<padded-ns>.seg      ← one TT log per session
    └── sensorlogs/
        ├── <recording_uuid_1>/            ← one sensor stream per recording
        │   ├── manifest.json
        │   └── segments/<padded-ns>.seg
        ├── <recording_uuid_2>/
        │   └── ...
        └── <recording_uuid_3>/
```

## Rationale

- **`<app_root>` is chosen by the integrator.** The SDK doesn't prescribe `<robot-home>/auki/<app-name>/` or any specific structure above the registries — the app picks its name and where to write. (Boosterapp uses `/home/booster/auki/boosterapp/`.)
- **Registries shared across sessions.** Hash-keyed writes are idempotent; re-writing the same `<hash>.json` per session would be wasted work. A sensor that doesn't change between app starts produces the same `<hash>.json` regardless of session.
- **One TimeTransform Log per session.** Clock offsets are time-localized; the session is the natural retention boundary.
- **A recording is one sensor stream.** Each `<recording_uuid>/` directory is a complete `auki-logs` log (manifest + segments) for exactly one sensor. Multi-sensor capture means multiple parallel recordings sharing a session, not a multi-sensor recording. The auto-started ring buffer is just a recording with `retention_ns: 30s`; intent captures are recordings with `retention_ns: 0`. Nothing on disk distinguishes "buffer" from "intent" beyond the manifest's retention value. The sensor identity is recorded in the log's manifest (`sensor_id` + `sensor_hash`), not encoded in the path.

## ID encoding

`/` in sensor / clock / frame ids is replaced with `__` so namespaced ids like `K1-AABBCCDDEEFF/head_left_cam` become a single filesystem-safe directory segment. The same substitution applies to `from_id`/`to_id` segments in TimeTransform Log paths.

## API

| Function                                                        | Returns                                                              |
|-----------------------------------------------------------------|----------------------------------------------------------------------|
| `registries_root(app_root)`                                     | `<app_root>/registries`                                              |
| `sensor_entry_path(app_root, sensor_id, hash)`                  | `<app_root>/registries/sensors/<sensor_id>/<hash>.json`              |
| `clock_entry_path(app_root, clock_id, hash)`                    | `<app_root>/registries/clocks/<clock_id>/<hash>.json`                |
| `session_root(app_root, session)`                               | `<app_root>/<session>`                                               |
| `timetransform_log_path(session_root, from_id, to_id)`          | `<session>/timetransform_logs/<from>__<to>`                          |
| `sensorlog_path(session_root, recording_uuid)`                  | `<session>/sensorlogs/<recording_uuid>`                              |
| `id_to_segment(id)`                                             | id with `/` replaced by `__`                                         |

## Versioning

Layout version is **1**. Changes to directory names (`registries/`, `sensorlogs/`, `timetransform_logs/`) or to the recording-uuid layer are breaking and require an SDK major bump.
