# auki-registry

Sensor / Clock / Frame / Detector / Map / Device Model identity catalogs and their on-disk IO. Content-addressed: the XXH3-128 hash of the RFC 8785 canonical JSON is the entry's version, and refining an entry is a sibling-write under the same id.

The crate's scope is identity catalogs plus content-addressed binary blobs (`blobs/<sha256>`) referenced by device models — log payload types live in [`auki-datatypes`](../auki-datatypes). Spatial sensor bodies pin exact frame references to the Frame Registry via the shared `RegistryRef` type.

**Status:** Shipped.

## Public surface

- `SensorRegistryEntry` (`peer_id`, `sensor_id`, body) + `SensorBody` (`Camera` / `Rangefinder` / `Rf` / `Audio` / `JointEncoders` / `Scalar`). Every body has an open-string `type` field (e.g. `"rgb"`, `"point_cloud"`, `"pcm"`, `"battery_charge"`). Spatial bodies carry a `frame: RegistryRef`; `Scalar` is deliberately non-spatial and instead pins an open-string `unit` and `expected_rate_hz`. Camera bodies also pin the immutable frame-byte contract: `image_encoding`, `pixel_format`, dimensions, and `row_stride_bytes`. `CameraFrame` therefore carries only per-frame data. `PointCloud` was renamed to `Rangefinder`; `point_cloud` is now a `sensor.type` value under that variant.
- `ClockRegistryEntry` (`peer_id`, `clock_id`, body)
- `FrameRegistryEntry` (`peer_id`, `frame_id`, convention fields) + preset constructors `FrameRegistryEntry::ros_body(peer_id, frame_id)`, `ros_optical(...)`, `opengl(...)`, `unity(...)` — all take `peer_id` as the first parameter.
- `DetectorRegistryEntry` (`peer_id`, `detector_id`, body, `input_types`, `output_types`). `DetectorInput` contracts make sensor compatibility discoverable before an instance starts; camera requirements can pin image encoding and pixel format. `DetectorBody::Custom` is the open extension point for third-party implementations: its namespaced `kind` and arbitrary JSON `configuration` are both content-addressed.
- `MapRegistryEntry` (`peer_id`, `map_id`, body) + `MapBody::Voxel(VoxelMap)`. The voxel contract pins the exact frame, voxel size, chunk dimension, additive evidence model, and semantic labels.
- `DeviceModelRegistryEntry` (`peer_id`, `device_model_id`, body) + tagged `DeviceModelFormat::Urdf { urdf_sha256, meshes }` (glTF later). `write_device_model` requires every referenced blob to already exist and updates an on-disk `TIP` pointer so List returns one tip per id.
- `put_blob` / `get_blob` / `read_blob_range` — SHA-256 content-addressed bytes under `<app_root>/blobs/<sha256>` (cap `MAX_BLOB_BYTES`).
- `put_urdf_package(app_root, urdf_path, root_convention, package_root=None)` — rewrite mesh filenames, blob URDF + meshes, return a ready `DeviceModelBody`. Default package dir is the URDF's parent (flattened packs). Pass `package_root` for stock ROS `pkg/urdf/` + `pkg/meshes/` trees (fail-closed under that root).
- `list_device_models(app_root, peer_id)` — tip-per-id List rows (TIP preferred; mtime fallback for pre-TIP trees).
- `RegistryRef { peer_id, id, hash }` — shared reference type replacing per-field `(sensor_id, sensor_hash)` pairs in manifests and sensor bodies.
- `LogRef { source_peer_id, resource_id }` — reference type for identifying logs by identity tuple (not a single content hash).
- `write_sensor(app_root, entry)` / `read_sensor(app_root, peer_id, sensor_id, hash)` (and `_clock` / `_frame` / `_detector` / `_map` / `_device_model` variants). Disk paths include the `peer_id` segment: `registries/<kind>/<peer_id>/<id>/<hash>.json`.
- `validate_registry_id(id)` — rejects `>`, `@`, and whitespace. Used by `Session::register_*` in `auki-session`.
- Locked vector pins the canonical-JSON + XXH3-128 chain for the M1 example sensor and clock entries (plus a `device_model_urdf` fixture).

## Depends on

- [`auki-jcs`](../auki-jcs) — for canonicalizing entries before hashing.
- [`auki-hash`](../auki-hash) — for the content hash.
- [`auki-layout`](../auki-layout) — for the on-disk path of each entry.
- `sha2` — for blob content addresses.
