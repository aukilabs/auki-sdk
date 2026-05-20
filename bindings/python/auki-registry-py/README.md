# auki-registry-py

PyO3 bindings for [`auki-registry`](../../../crates/auki-registry) — dict-oriented constructors and hash-pinned on-disk read/write helpers for Sensor, Clock, and Frame Registry entries.

This is the Python producer side of the stream-manifest work: Boosterapp can write a frame entry, pin its hash into a point-cloud / RGB sensor entry, write the sensor entry, then feed the hashes to `auki_domain.StreamManifestBuilder.from_registry(...)`.

## Surface

```python
import auki_registry
import auki_domain
from auki_network import cluster

frame = auki_registry.frame_ros_optical(
    "K1-AABBCCDDEEFF/head_left_cam_optical"
)
frame_hash = auki_registry.write_frame(app_root, frame)

sensor = auki_registry.point_cloud_sensor_entry(
    sensor_id="K1-AABBCCDDEEFF/head_depth_points",
    fields=[
        auki_registry.point_field("x", 0, "float32"),
        auki_registry.point_field("y", 4, "float32"),
        auki_registry.point_field("z", 8, "float32"),
    ],
    point_step=12,
    is_bigendian=False,
    frame_rate_hz=10,
    frame_id=frame["frame_id"],
    frame_hash=frame_hash,
)
sensor_hash = auki_registry.write_sensor(app_root, sensor)

clock = auki_registry.monotonic_clock_entry(
    clock_id="K1-AABBCCDDEEFF/monotonic",
    unit="milliseconds",
    scope="device-local",
)
clock_hash = auki_registry.write_clock(app_root, clock)

manifest = auki_domain.StreamManifestBuilder.from_registry(
    app_root,
    sensor["sensor_id"],
    sensor_hash,
    "K1-AABBCCDDEEFF/monotonic",
    clock_hash,
)
decision = cluster.StreamDecision.accept_pointcloud(
    manifest=manifest,
    source=pointcloud_source(),
)
```

Frame constructors:

- `frame_ros_body(frame_id)`
- `frame_ros_optical(frame_id)`
- `frame_opengl(frame_id)`
- `frame_unity(frame_id)`
- `frame_entry(frame_id=..., handedness=..., x=..., y=..., z=..., units=...)`

Sensor constructors:

- `point_field(name, offset, datatype, count=1)`
- `rgb_camera_sensor_entry(...)`
- `point_cloud_sensor_entry(...)`
- `audio_sensor_entry(...)`
- `joint_encoders_sensor_entry(...)`

Clock constructors:

- `monotonic_clock_entry(clock_id=..., unit=..., scope=..., epoch=None)`
- `utc_clock_entry(clock_id=..., unit=..., scope=..., epoch=...)`

Storage and hash helpers:

- `write_frame(app_root, frame) -> hash`
- `write_sensor(app_root, sensor) -> hash`
- `write_clock(app_root, clock) -> hash`
- `read_frame(app_root, frame_id, hash) -> dict | None`
- `read_sensor(app_root, sensor_id, hash) -> dict | None`
- `read_clock(app_root, clock_id, hash) -> dict | None`
- `hash_frame(frame)`, `hash_sensor(sensor)`, `hash_clock(clock)`
- `canonical_json_frame(frame)`, `canonical_json_sensor(sensor)`, `canonical_json_clock(clock)`

## Enum strings

The Python API uses the same strings that appear on disk:

- `handedness`: `"right"` | `"left"`
- axis directions: `"forward"` | `"backward"` | `"up"` | `"down"` | `"left"` | `"right"`
- `units`: `"meters"` | `"millimeters"` | `"centimeters"`
- `datatype`: `"int8"` | `"uint8"` | `"int16"` | `"uint16"` | `"int32"` | `"uint32"` | `"float32"` | `"float64"`
- `scope`: `"device-local"` | `"domain-local"` | `"global"`

## Errors

The Rust crate remains the source of truth for validation:

- Invalid enum strings, malformed dicts, non-orthogonal frame axes, id mismatches, or missing frame references raise `ValueError`.
- I/O failures raise `OSError`.
- `read_*` returns `None` when the requested hash-pinned file is absent.

`write_sensor` validates frame-bearing bodies exactly like Rust: `RgbCamera` and `PointCloud` entries must carry non-empty `frame_id` + `frame_hash`, and the exact frame file must already exist under `<app_root>/registries/frames/...`. There is no compatibility fallback or directory scan.

## Install

```sh
cd bindings/python/auki-registry-py
python -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop --release
pytest python_tests/
```

See [`src/readme.md`](src/readme.md) for implementation status.
