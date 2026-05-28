# auki-manifests

Single source of truth for the SDK's per-recording log manifest shapes — JCS-canonical UTF-8 JSON. Symmetric with [`auki-datatypes`](../auki-datatypes): that crate owns segment payload shapes, this one owns manifest shapes.

**Status:** Shipped.

## Public surface

- `build_sensor_log_manifest(source_peer_id, writer_peer_id, app_id, session_id, sensor: RegistryRef, clock: RegistryRef, frame: Option<RegistryRef>, ...)`
- `build_pose_log_manifest(source_peer_id, writer_peer_id, ..., from_frame: RegistryRef, to_frame: RegistryRef, clock: RegistryRef, source, writer_mode, ...)`
- `build_time_transform_log_manifest(source_peer_id, writer_peer_id, ..., from_clock: RegistryRef, to_clock: RegistryRef, ...)`
- `build_detection_log_manifest(source_peer_id, writer_peer_id, ..., detector: RegistryRef, input_log: LogRef, input_sensor: RegistryRef, clock: RegistryRef, ...)`
- Manifest structs: `SensorLogManifest`, `PoseLogManifest`, `TimeTransformLogManifest`, `DetectionLogManifest` — all have `source_peer_id` and `writer_peer_id` as separate fields. `source_peer_id` is the data origin; `writer_peer_id` is the peer that wrote this manifest file (may differ when a remote peer materializes the log).
- `PoseSource`, `PoseWriterMode`, `TimeTransformSource` — tagged-enum provenance / writer-mode types stamped into the manifest.
- Registry references use `RegistryRef { peer_id, id, hash }` (imported from `auki-registry`). `DetectionLogManifest.input_log` uses `LogRef { source_peer_id, resource_id }`.

## Depends on

- [`auki-jcs`](../auki-jcs) — for canonicalizing the manifest JSON.
- [`auki-hash`](../auki-hash) — for hash-pinning referenced registry entries.
- [`auki-registry`](../auki-registry) — for `RegistryRef` and `LogRef` shared types.
