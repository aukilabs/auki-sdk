# auki-registry-py

PyO3 bindings for [`auki-registry`](../../../crates/auki-registry). Lets Python producers declare and persist Sensor / Clock / Frame Registry entries with the same content-addressed identity Rust uses.

Mirrors the Rust API: dict-style constructors for entries, canonical-JSON + hash helpers, and hash-pinned `write_*` / `read_*` IO. Spatial sensors are validated against an exact `frame_id` + `frame_hash` reference.

**Status:** Shipped.

## Public surface

- `SensorRegistryEntry(...)`, `ClockRegistryEntry(...)`, `FrameRegistryEntry(...)` — dict-style constructors.
- Frame Registry presets: `FrameRegistryEntry.ros_body(...)`, `ros_optical(...)`, `opengl(...)`, `unity(...)`.
- `canonical_json(entry) -> bytes`, `content_hash(entry) -> str`.
- `write_sensor(app_root, entry)`, `read_sensor(app_root, sensor_id, sensor_hash)` (and `_clock` / `_frame` variants).

## Depends on

- [`auki-registry`](../../../crates/auki-registry) — Rust crate it wraps.
