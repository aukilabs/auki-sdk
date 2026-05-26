# auki-layout

Path helpers for the Auki SDK's on-disk session shape. Single source of truth for the layout that registries, logs, and sessions occupy under an app root — so that the SDK, downstream consumers, and any cross-language reimplementation all agree on where things live.

> **Renamed from `auki-session` 2026-05-08.** The previous name implied a runtime `Session` abstraction (lifecycle, clock binding, sensor-id minting) that this crate doesn't provide; this crate is the *layout contract*. `auki-session` is now reserved for the future Rust counterpart of [`auki-session-py`](../auki-session-py)'s in-process `Session` surface — see the [root `Session.open` Propagate item](../../parking_lot.md) for the planned shape.

The Rust API remains `&Path` / `PathBuf`. Generated Python, Swift, and JavaScript bindings expose the same helpers as UTF-8 path strings through crate-owned UniFFI and wasm-bindgen adapters.

## On-disk layout

```text
<app_root>/
├── registries/
│   ├── sensors/<sensor_id>/<hash>.json   ← shared across all sessions of this app
│   ├── clocks/<clock_id>/<hash>.json
│   └── frames/<frame_id>/<hash>.json     ← Frame Registry (v0.0.22)
└── <session_id>/                          ← UUIDv4 minted at app boot
    ├── timetransform_logs/<from_id>__<to_id>/
    │   ├── log_manifest.json
    │   ├── tags.jsonl                    ← optional TagClaim sidecar; see ../tags.md
    │   └── segments/<padded-ns>.seg      ← one TT log per session
    ├── sensorlogs/
    │   ├── <sensor_log_id_1>/             ← one sensor stream per log
    │   │   ├── log_manifest.json
    │   │   ├── tags.jsonl                ← optional TagClaim sidecar; see ../tags.md
    │   │   └── segments/<padded-ns>.seg
    │   ├── <sensor_log_id_2>/
    │   │   └── ...
    │   └── <sensor_log_id_3>/
    ├── poselogs/
    │   ├── <from_id>__<to_id>/            ← one (from_frame_id, to_frame_id) pair per log
    │   │   ├── log_manifest.json
    │   │   ├── tags.jsonl                ← optional TagClaim sidecar; see ../tags.md
    │   │   └── segments/<padded-ns>.seg
    │   └── <from_id_2>__<to_id_2>/
    └── detection_logs/
        ├── <detector_id>__<input_log_id>/ ← one (Detector, input sensor log) pair (2026-05-09)
        │   ├── log_manifest.json
        │   ├── tags.jsonl                ← optional TagClaim sidecar; see ../tags.md
        │   └── segments/<padded-ns>.seg
        └── ...
```

`tags.jsonl` is the reserved sidecar for [`TagClaim`](../../tags.md) records (domain membership, anchor citations, contribution credits, …). The SDK doesn't currently write or read it — TagClaim handling lives outside the crate boundary — but the filename is documented here so any tooling that enumerates a log directory accounts for it.

## Session lifecycle

**A session begins on app boot and ends when the daemon exits** (cleanly or otherwise). The integrator generates a fresh **UUIDv4** at boot and uses it as the session directory name and as the `session_id` in every log manifest written during the run. A daemon restart begins a new session with a new UUID; nothing on disk ties two consecutive sessions together at the SDK layer.

This shape matches what the [Control API](../../docs/control-api.md) implies (`/api/info` returns one `session_id`; multi-session daemons are out of scope for v1) and what the broader protocol model expects (the [Domain doc](https://www.notion.so/3565c8e965928154803af89f3b16d097) defines `session_id` as "Per-daemon UUID minted at session start; carries no implicit domain affiliation until tagged"). The SDK doesn't generate the UUID — that's the integrator's job — but every manifest writer requires the value, so the integrator must mint one before opening any log.

`session_id` is one of three identifiers that travel together at any call site:

- **`session_id`** — UUIDv4, integrator-minted at boot, opaque. Many per domain, no formal cardinality relation.
- **`domain_id`** — `hash(owner_wallet_pubkey)`, derived from a wallet, stable for the wallet's lifetime. Asserted *into* a session's data via [`TagClaim`](../../tags.md) records, never encoded in the path.
- **`scenegraph_id`** — hash of a constructed scenegraph's manifest. Many per domain; the owner marks one canonical.

None is derivable from the others.

## Rationale

- **`<app_root>` is chosen by the integrator.** The SDK doesn't prescribe `<robot-home>/auki/<app-name>/` or any specific structure above the registries — the app picks its name and where to write. (Boosterapp uses `/home/booster/auki/boosterapp/`.)
- **Registries shared across sessions.** Hash-keyed writes are idempotent; re-writing the same `<hash>.json` per session would be wasted work. A sensor that doesn't change between app starts produces the same `<hash>.json` regardless of session.
- **One TimeTransform Log per session.** Clock offsets are time-localized; the session is the natural retention boundary.
- **A log is one stream.** Each `<sensor_log_id>/` directory is a complete `auki-logs` log (manifest + segments) for exactly one sensor; each `<from_id>__<to_id>/` directory under `poselogs/` is a complete log for exactly one ordered frame pair (mirrors the `timetransform_logs/` shape, where each directory is one ordered clock pair). Multi-stream capture means multiple parallel logs sharing a session, not a multi-stream log. Buffers, intent recordings, and time-bounded captures are all the same kind of log on disk — they differ only in their manifest's `retention_ns` (backward window kept on disk; `0` = no eviction). Whether a daemon auto-creates any log at session boot is daemon-application policy, not SDK contract — see the [Control API spec](../../docs/control-api.md). For sensor logs, identity is the manifest's `sensor_id` + `sensor_hash` (the path's `<sensor_log_id>` is opaque); for pose logs, identity is the manifest's `(from_frame_id, to_frame_id)` pair (encoded in the path) plus the inline `source` block describing the producer.

## ID encoding

`/` in sensor / clock / frame ids is replaced with `__` so namespaced ids like `K1-AABBCCDDEEFF/head_left_cam` become a single filesystem-safe directory segment. The same substitution applies to `from_id`/`to_id` segments in TimeTransform Log and Pose Log paths.

## API

| Function                                                        | Returns                                                              |
|-----------------------------------------------------------------|----------------------------------------------------------------------|
| `registries_root(app_root)`                                     | `<app_root>/registries`                                              |
| `sensor_entry_path(app_root, sensor_id, hash)`                  | `<app_root>/registries/sensors/<sensor_id>/<hash>.json`              |
| `clock_entry_path(app_root, clock_id, hash)`                    | `<app_root>/registries/clocks/<clock_id>/<hash>.json`                |
| `frame_entry_path(app_root, frame_id, hash)`                    | `<app_root>/registries/frames/<frame_id>/<hash>.json`                |
| `detector_entry_path(app_root, detector_id, hash)`              | `<app_root>/registries/detectors/<detector_id>/<hash>.json`          |
| `session_root(app_root, session_id)`                            | `<app_root>/<session_id>`                                            |
| `timetransform_log_path(session_root, from_id, to_id)`          | `<session_id>/timetransform_logs/<from>__<to>`                       |
| `sensorlog_path(session_root, sensor_log_id)`                   | `<session_id>/sensorlogs/<sensor_log_id>`                            |
| `poselog_path(session_root, from_frame_id, to_frame_id)`        | `<session_id>/poselogs/<from>__<to>`                                 |
| `detection_log_path(session_root, detector_id, input_log_id)`   | `<session_id>/detection_logs/<detector_id>__<input_log_id>` (2026-05-09) |
| `id_to_segment(id)`                                             | id with `/` replaced by `__`                                         |

## Binding generation

`bindings.toml` enables all three generated package families:

- Python and Swift use UniFFI from the native `ffi.rs` string adapters.
- JavaScript uses wasm-bindgen from `wasm.rs`.
- Rust dependents that only need the direct path helpers should depend on this crate with `default-features = false`.

## Versioning

Layout version is **1**. Changes to directory names (`registries/`, `sensorlogs/`, `poselogs/`, `timetransform_logs/`) or to the per-log identifier layer (`<sensor_log_id>`, `<from>__<to>` for pose logs and TimeTransform logs) are breaking and require an SDK major bump.
