# auki-registry-py

PyO3 bindings for [`auki-registry`](../../../crates/auki-registry). Lets Python producers declare and persist Sensor / Clock / Frame / Detector Registry entries with the same content-addressed identity Rust uses.

Mirrors the Rust API: dict-style constructors for entries, canonical-JSON + hash helpers, and hash-pinned `write_*` / `read_*` IO. Spatial sensors are validated against a `frame: RegistryRef` reference (a `{ "peer_id": ..., "id": ..., "hash": ... }` dict or `RegistryRef` pyclass instance).

**Status:** Shipped.

## Public surface

- `SensorRegistryEntry(...)`, `ClockRegistryEntry(...)`, `FrameRegistryEntry(...)`, `DetectorRegistryEntry(...)` — dict-style constructors.
- Frame Registry presets: `FrameRegistryEntry.ros_body(peer_id, frame_id)`, `ros_optical(peer_id, frame_id)`, `opengl(peer_id, frame_id)`, `unity(peer_id, frame_id)` — all take `peer_id` as the first parameter. Raster-convention presets follow the same shape: `raster_top_left(...)`, `raster_mirrored(...)`, `raster_normalized(...)`, `raster_normalized_mirrored(...)`.
- A `FrameRegistryEntry` optionally carries a `raster` block (`{ origin, u, v, units }`) describing the 2D byte layout of a published raster (image / video / depth map); it is absent on pure 3D spatial frames. The raster presets above return entries with this block populated.
- `RegistryRef` pyclass — `{ peer_id, id, hash }`. Returned by `write_*` helpers and accepted as a `frame` argument. Replaces the old `(sensor_id, sensor_hash)` pair construction.
- `LogRef` pyclass — `{ source_peer_id, resource_id }`. Used in manifest construction and catalog rows.
- `canonical_json(entry) -> bytes`, `content_hash(entry) -> str`.
- `write_sensor(app_root, entry)` / `read_sensor(app_root, peer_id, sensor_id, sensor_hash)` (and `_clock` / `_frame` / `_detector` variants). Both read and write functions take `peer_id` as a parameter; disk paths include the `peer_id` segment.

## Depends on

- [`auki-registry`](../../../crates/auki-registry) — Rust crate it wraps.
