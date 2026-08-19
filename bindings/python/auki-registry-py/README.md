# auki-registry-py

PyO3 bindings for [`auki-registry`](../../../crates/auki-registry). Lets Python producers declare and persist Sensor / Clock / Frame / Detector / Map Registry entries with the same content-addressed identity Rust uses. Voxel Maps use `voxel_map_entry`, `write_map`, and `read_map`.

Mirrors the Rust API: dict-style constructors for entries, canonical-JSON + hash helpers, and hash-pinned `write_*` / `read_*` IO. Spatial sensors are validated against a `frame: RegistryRef` reference (a `{ "peer_id": ..., "id": ..., "hash": ... }` dict or `RegistryRef` pyclass instance). Non-spatial measurements use `scalar_sensor_entry(peer_id=..., sensor_id=..., sensor_type=..., unit=..., expected_rate_hz=...)` and need no frame. Camera constructors require the immutable frame-byte contract (`image_encoding`, `pixel_format`, dimensions, and `row_stride_bytes`) and accept optional static `calibration={fx, fy, cx, cy, distortion_coefficients}` for metric consumers.

**Status:** Shipped.

## Public surface

- `SensorRegistryEntry(...)`, `ClockRegistryEntry(...)`, `FrameRegistryEntry(...)`, `DetectorRegistryEntry(...)` — dict-style constructors. Detector entries carry discoverable `input_types` and `output_types` capability lists.
- Frame Registry presets: `FrameRegistryEntry.ros_body(peer_id, frame_id)`, `ros_optical(peer_id, frame_id)`, `opengl(peer_id, frame_id)`, `unity(peer_id, frame_id)` — all take `peer_id` as the first parameter.
- `RegistryRef` pyclass — `{ peer_id, id, hash }`. Returned by `write_*` helpers and accepted as a `frame` argument. Replaces the old `(sensor_id, sensor_hash)` pair construction.
- `LogRef` pyclass — `{ source_peer_id, resource_id }`. Used in manifest construction and catalog rows.
- `canonical_json(entry) -> bytes`, `content_hash(entry) -> str`.
- `write_sensor(app_root, entry)` / `read_sensor(app_root, peer_id, sensor_id, sensor_hash)` (and `_clock` / `_frame` / `_detector` variants). Both read and write functions take `peer_id` as a parameter; disk paths include the `peer_id` segment.
- `put_urdf_package(app_root, urdf_path, root_convention=None, package_root=None, mesh_substitutions=None)` — rewrite + blob a URDF package; optional `mesh_substitutions` dict maps raw `filename=` → `{advertised_path, source_path}`.
- `write_device_model` / `list_device_models` — device-model registry IO.

## Depends on

- [`auki-registry`](../../../crates/auki-registry) — Rust crate it wraps.
