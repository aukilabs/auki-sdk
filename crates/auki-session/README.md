# auki-session

Path helpers for the Auki SDK's on-disk session shape. Single source of truth for the layout that registries, logs, and sessions occupy under an app root — so that the SDK, downstream consumers, and any cross-language reimplementation all agree on where things live.

## On-disk layout

```text
<app_root>/
├── registries/
│   ├── sensors/<sensor_id>/<hash>.json   ← shared across all sessions of this app
│   ├── clocks/<clock_id>/<hash>.json
│   └── frames/<frame_id>/<hash>.json     ← coming
└── <session_id>/                          ← UUIDv4 minted at app boot
    ├── timetransform_logs/<from_id>__<to_id>/
    │   ├── manifest.json
    │   ├── tags.jsonl                    ← optional TagClaim sidecar; see ../tags.md
    │   └── segments/<padded-ns>.seg      ← one TT log per session
    ├── sensorlogs/
    │   ├── <recording_uuid_1>/            ← one sensor stream per recording
    │   │   ├── manifest.json
    │   │   ├── tags.jsonl                ← optional TagClaim sidecar; see ../tags.md
    │   │   └── segments/<padded-ns>.seg
    │   ├── <recording_uuid_2>/
    │   │   └── ...
    │   └── <recording_uuid_3>/
    └── poselogs/
        ├── <recording_uuid_1>/            ← one pose source per recording
        │   ├── manifest.json
        │   ├── tags.jsonl                ← optional TagClaim sidecar; see ../tags.md
        │   └── segments/<padded-ns>.seg
        └── <recording_uuid_2>/
```

`tags.jsonl` is the reserved sidecar for [`TagClaim`](../../tags.md) records (domain membership, anchor citations, contribution credits, …). The SDK doesn't currently write or read it — TagClaim handling lives outside the crate boundary — but the filename is documented here so any tooling that enumerates a log directory accounts for it.

## Session lifecycle

**A session begins on app boot and ends when the daemon exits** (cleanly or otherwise). The integrator generates a fresh **UUIDv4** at boot and uses it as the session directory name and as the `session_id` in every log manifest written during the run. A daemon restart begins a new session with a new UUID; nothing on disk ties two consecutive sessions together at the SDK layer.

This shape matches what the [Control API](../../docs/control-api.md) already implies (`/api/state` returns one `session_uuid`; multi-session daemons are out of scope for v1) and what the broader protocol model expects (the [Domain doc](https://www.notion.so/3565c8e965928154803af89f3b16d097) defines `session_id` as "Per-daemon UUID minted at session start; carries no implicit domain affiliation until tagged"). The SDK doesn't generate the UUID — that's the integrator's job — but every manifest writer requires the value, so the integrator must mint one before opening any log.

`session_id` is one of three identifiers that travel together at any call site:

- **`session_id`** — UUIDv4, integrator-minted at boot, opaque. Many per domain, no formal cardinality relation.
- **`domain_id`** — `hash(owner_wallet_pubkey)`, derived from a wallet, stable for the wallet's lifetime. Asserted *into* a session's data via [`TagClaim`](../../tags.md) records, never encoded in the path.
- **`scenegraph_id`** — hash of a constructed scenegraph's manifest. Many per domain; the owner marks one canonical.

None is derivable from the others.

## Rationale

- **`<app_root>` is chosen by the integrator.** The SDK doesn't prescribe `<robot-home>/auki/<app-name>/` or any specific structure above the registries — the app picks its name and where to write. (Boosterapp uses `/home/booster/auki/boosterapp/`.)
- **Registries shared across sessions.** Hash-keyed writes are idempotent; re-writing the same `<hash>.json` per session would be wasted work. A sensor that doesn't change between app starts produces the same `<hash>.json` regardless of session.
- **One TimeTransform Log per session.** Clock offsets are time-localized; the session is the natural retention boundary.
- **A recording is one stream.** Each `<recording_uuid>/` directory is a complete `auki-logs` log (manifest + segments) for exactly one sensor (under `sensorlogs/`) or one pose source (under `poselogs/`). Multi-stream capture means multiple parallel recordings sharing a session, not a multi-stream recording. The auto-started ring buffer is just a recording with `retention_ns: 30s`; intent captures are recordings with `retention_ns: 0`. Nothing on disk distinguishes "buffer" from "intent" beyond the manifest's retention value. For sensor logs, identity is the manifest's `sensor_id` + `sensor_hash`; for pose logs, identity is the manifest's inline `source` block. Neither is encoded in the path.

## ID encoding

`/` in sensor / clock / frame ids is replaced with `__` so namespaced ids like `K1-AABBCCDDEEFF/head_left_cam` become a single filesystem-safe directory segment. The same substitution applies to `from_id`/`to_id` segments in TimeTransform Log paths.

## API

| Function                                                        | Returns                                                              |
|-----------------------------------------------------------------|----------------------------------------------------------------------|
| `registries_root(app_root)`                                     | `<app_root>/registries`                                              |
| `sensor_entry_path(app_root, sensor_id, hash)`                  | `<app_root>/registries/sensors/<sensor_id>/<hash>.json`              |
| `clock_entry_path(app_root, clock_id, hash)`                    | `<app_root>/registries/clocks/<clock_id>/<hash>.json`                |
| `session_root(app_root, session_id)`                            | `<app_root>/<session_id>`                                            |
| `timetransform_log_path(session_root, from_id, to_id)`          | `<session_id>/timetransform_logs/<from>__<to>`                       |
| `sensorlog_path(session_root, recording_uuid)`                  | `<session_id>/sensorlogs/<recording_uuid>`                           |
| `poselog_path(session_root, recording_uuid)`                    | `<session_id>/poselogs/<recording_uuid>`                             |
| `id_to_segment(id)`                                             | id with `/` replaced by `__`                                         |

## Versioning

Layout version is **1**. Changes to directory names (`registries/`, `sensorlogs/`, `timetransform_logs/`) or to the recording-uuid layer are breaking and require an SDK major bump.
