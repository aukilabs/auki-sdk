# auki-layout-py

PyO3 bindings for [`auki-layout`](../../../crates/auki-layout) — pure-function wrappers around the on-disk path helpers. Lets a Python consumer compute SDK-canonical paths without re-implementing the `__`-substitution + directory-name conventions.

Filed alongside [`auki-manifests-py`](../auki-manifests-py) as the companion to [`auki-logs-py`](../auki-logs-py); together they close the path / manifest construction gap that previously left Python consumers (e.g. the [ESL detector](https://github.com/aukilabs/detectors)) hand-rolling strings and dicts.

## Surface

```python
import auki_layout

# Registry paths
auki_layout.registries_root("/app")
auki_layout.sensor_entry_path("/app", "K1-AABBCCDDEEFF/head_left_cam", "deadbeef")
auki_layout.clock_entry_path("/app", "K1-AABBCCDDEEFF/utc", "1234")
auki_layout.frame_entry_path("/app", "K1-AABBCCDDEEFF/base_link", "5678")

# Session paths
session = auki_layout.session_root("/app", "abc-123")
auki_layout.timetransform_log_path(session, "K1-AABB/utc", "K1-AABB/monotonic")
auki_layout.sensorlog_path(session, "rec-456")
auki_layout.poselog_path(session, "K1-AABB/base_link", "K1-AABB/cam_optical")
auki_layout.detection_log_path(session, "aukilabs/qr/v1", "rec-456")

# Helper
auki_layout.id_to_segment("foo/bar")  # → "foo__bar"
```

All return `str`. No state, no PyClasses — each call is a thin wrapper around the corresponding `auki-layout` Rust function.

## Why a Python wrapper

Python consumers could hand-roll the path concat (`f"{session}/detection_logs/{detector_id.replace('/', '__')}__{input_log_id}"`) but that creates a silent drift risk: if the Rust crate's substitution rule or directory-name conventions change, the Python side breaks invisibly. Wrapping keeps both sides reading from one source of truth.

## Install

```sh
cd bindings/python/auki-layout-py
python -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop --release
pytest python_tests/
```

## Status

Crate landed 2026-05-09 alongside [`auki-manifests-py`](../auki-manifests-py). 10 path functions exposed; matches the Rust crate's API one-to-one. Tests: 3 Rust-side smoke tests + 9 Python-side tests pinning the substitution rules.

See [`src/readme.md`](src/readme.md) for the implementation detail.
