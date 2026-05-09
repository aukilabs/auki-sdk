# auki-manifests-py

PyO3 bindings for [`auki-manifests`](../auki-manifests) — pure-function wrappers around the four `build_*_log_manifest` builders. Lets a Python consumer construct SDK-canonical log manifest dicts without re-implementing field shapes.

Filed alongside [`auki-layout-py`](../auki-layout-py) as the companion to [`auki-logs-py`](../auki-logs-py).

## Surface

```python
import auki_manifests

# The one the ESL detector uses:
m = auki_manifests.build_detection_log_manifest(
    app_id="boosterapp",
    session_id="550e8400-e29b-41d4-a716-446655440000",
    detector_id="aukilabs/esl/v1",
    detector_hash="...",
    input_log_id="rec-456",
    input_sensor_id="K1-AABBCCDDEEFF/head_left_cam",
    input_sensor_hash="...",
    clock_id="K1-AABBCCDDEEFF/utc",
    clock_hash="...",
    segment_duration_ns=1_000_000_000,
    retention_ns=60_000_000_000,
)
# m is a Python dict — pass it straight to auki_logs.Log.open(...).

# Sensor / Pose / TimeTransform builders also exposed:
auki_manifests.build_sensor_log_manifest(...)
auki_manifests.build_pose_log_manifest(
    ...,
    source={"kind": "ros2_tf", "publishers": ["amcl", "..."]},  # PoseSource as dict
    writer_mode="movable",                                       # "rigid" or "movable"
    ...,
)
auki_manifests.build_time_transform_log_manifest(
    ...,
    source={"kind": "local_clock_read"},                         # TimeTransformSource as dict
    ...,
)
```

All return Python `dict`. Pass directly to `auki_logs.Log.open(path, manifest)`.

## Enum seam

`PoseSource`, `PoseWriterMode`, and `TimeTransformSource` are Rust tagged enums. The Python surface takes them as **dicts** / **strings** rather than introducing PyClass equivalents:

- `PoseSource::Ros2Tf { publishers }` → `{"kind": "ros2_tf", "publishers": [...]}`
- `PoseWriterMode::Rigid | Movable` → `"rigid"` | `"movable"`
- `TimeTransformSource::LocalClockRead` → `{"kind": "local_clock_read"}`

Wrappers parse via `serde_json` into the Rust enum, surfacing decode errors as `ValueError`. Keeps the Python footprint small and matches the natural JSON shape consumers already think in.

## Errors

Decode failures (unknown enum variant, garbage in dict) → `ValueError` with prefix (`source:` / `writer_mode:` / `decode:`).

## Install

```sh
cd crates/auki-manifests-py
python -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop --release
pytest python_tests/
```

## Status

Crate landed 2026-05-09 alongside [`auki-layout-py`](../auki-layout-py). Four manifest builders exposed; matches the Rust crate's API one-to-one. Tests: 2 Rust-side smoke + 6 Python-side covering all four builders + an enum-rejection test.

`PoseSource::canonical_bytes` / `hash` content-addressing helpers are **out of scope** for v1 — Python consumers that need them today re-implement the canonicalize-via-JCS + XXH3 dance themselves; expose later if a real Python consumer needs the graduation primitives. Filed in [`parking_lot.md`](parking_lot.md).

See [`src/readme.md`](src/readme.md) for the implementation detail.
