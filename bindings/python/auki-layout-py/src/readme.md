# `auki-layout-py/src/`

PyO3 bindings for `auki-layout`. Spec: [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). Ten `#[pyfunction]`s, each a thin wrapper that takes Python `str` arguments, calls the corresponding `auki-layout` function, and returns the resulting `PathBuf` as a `str` (via `to_string_lossy`).

No state, no PyClasses, no GIL gymnastics — every call is in-and-out string manipulation.

## Public surface

```python
def registries_root(app_root: str) -> str
def sensor_entry_path(app_root: str, sensor_id: str, hash: str) -> str
def clock_entry_path(app_root: str, clock_id: str, hash: str) -> str
def frame_entry_path(app_root: str, frame_id: str, hash: str) -> str
def session_root(app_root: str, session: str) -> str
def timetransform_log_path(session_root: str, from_id: str, to_id: str) -> str
def sensorlog_path(session_root: str, sensor_log_id: str) -> str
def poselog_path(session_root: str, from_frame_id: str, to_frame_id: str) -> str
def detection_log_path(session_root: str, detector_id: str, input_log_id: str) -> str
def id_to_segment(id: str) -> str
```

## Tests

### Rust-side (`cargo test -p auki-layout-py`, 3 tests)

| Test | Asserts |
|------|---------|
| `detection_log_path_substitutes_slashes_in_detector_id` | `aukilabs/qr/v1` + `rec-456` → `aukilabs__qr__v1__rec-456` |
| `sensor_entry_path_includes_id_subst_and_hash_filename` | `K1-AABBCCDDEEFF/head_left_cam` + `deadbeef` → `K1-AABBCCDDEEFF__head_left_cam/deadbeef.json` |
| `id_to_segment_substitutes_slashes` | `foo/bar/baz` → `foo__bar__baz`; no-op for ids without `/` |

### Python-side (`pytest python_tests/`, 9 tests)

Module-shape check + per-helper substitution tests mirroring the Rust crate's own tests.

## Consumers

- [`detectors`](https://github.com/aukilabs/detectors) (downstream) — the [ESL detector](https://github.com/aukilabs/detectors/tree/main/detector-esl) computes detection-log paths via `auki_layout.detection_log_path` instead of hand-rolling the `<session>/detection_logs/<detector_id>__<input_log_id>` shape.
