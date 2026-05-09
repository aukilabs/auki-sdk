# `auki-manifests-py/src/`

PyO3 bindings for `auki-manifests`. Spec: [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). Four `#[pyfunction]`s — one per builder — plus three internal helpers (`pydict_to_json`, `json_to_pyobject`, the enum parsers).

## Public surface

```python
def build_sensor_log_manifest(
    app_id, session_id, sensor_id, sensor_hash, clock_id, clock_hash,
    segment_duration_ns, retention_ns,
) -> dict

def build_pose_log_manifest(
    app_id, session_id,
    from_frame_id, from_frame_hash,
    to_frame_id, to_frame_hash,
    clock_id, clock_hash,
    source: dict,           # {"kind": "ros2_tf", "publishers": [...]}
    writer_mode: str,       # "rigid" | "movable"
    expected_rate_hz: int,
    segment_duration_ns, retention_ns,
) -> dict

def build_time_transform_log_manifest(
    app_id, session_id,
    from_clock_id, from_clock_hash,
    to_clock_id, to_clock_hash,
    source: dict,           # {"kind": "local_clock_read"}
    segment_duration_ns, retention_ns,
) -> dict

def build_detection_log_manifest(
    app_id, session_id,
    detector_id, detector_hash,
    input_log_id,
    input_sensor_id, input_sensor_hash,
    clock_id, clock_hash,
    segment_duration_ns, retention_ns,
) -> dict
```

## Manifest dict seam

Python `dict` ↔ Rust `serde_json::Value` round-trips via Python's stdlib `json` module — `json.dumps` on entry (for the source dict args), `json.loads` on the way out (for the returned manifest dict). Same pattern as [`auki-logs-py`](../../auki-logs-py); avoids hand-coding a pydict-to-serde walker.

## Enum parsing

`PoseSource` / `TimeTransformSource` arrive as Python dicts. The wrappers JSON-stringify them and `serde_json::from_value` into the typed Rust enum. Unknown variant or missing `kind` → `ValueError` prefixed `source:`. `PoseWriterMode` arrives as `"rigid"` / `"movable"`; same parse path with prefix `writer_mode:`.

## Errors

| Failure mode | Python exception |
|---|---|
| Unknown enum variant | `ValueError` (prefix `source:` / `writer_mode:`) |
| Manifest dict decode | `ValueError` (prefix `decode:`) |

## Tests

### Rust-side (`cargo test -p auki-manifests-py`, 2 tests)

| Test | Asserts |
|------|---------|
| `parse_pose_writer_mode_accepts_canonical_strings` | `"rigid"` / `"movable"` parse correctly. |
| `parse_pose_writer_mode_rejects_garbage` | `"nonsense"` → `Err`. |

### Python-side (`pytest python_tests/`, 6 tests)

Per-builder field-presence tests + an enum-rejection test (`writer_mode="nonsense"` raises `ValueError`).

## Out of scope

- **`PoseSource::canonical_bytes` / `hash`** — content-addressing helpers for the graduation path. Filed in [`parking_lot.md`](../parking_lot.md). Re-expose if a Python consumer needs them.
- **PyClass equivalents of the enums** — declined for now in favour of the dict / string seam. PyClass adds new types Python users have to learn; the dict shape is what they'd serialize to anyway.

## Consumers

- [`detectors`](https://github.com/aukilabs/detectors) (downstream) — the [ESL detector](https://github.com/aukilabs/detectors/tree/main/detector-esl) builds its detection-log manifest dict via `auki_manifests.build_detection_log_manifest(...)` and hands it straight to `auki_logs.Log.open(...)`.
