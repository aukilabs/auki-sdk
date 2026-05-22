# auki-manifests

Single source of truth for the SDK's per-recording log manifest shapes — JCS-canonical UTF-8 JSON. Symmetric with [`auki-datatypes`](../auki-datatypes): that crate owns segment payload shapes, this one owns manifest shapes.

**Status:** Shipped.

## Public surface

- `build_sensor_log_manifest(...)`
- `build_pose_log_manifest(...)`
- `build_time_transform_log_manifest(...)`
- `build_detection_log_manifest(...)`
- `PoseSource`, `PoseWriterMode`, `TimeTransformSource` — tagged-enum provenance / writer-mode types stamped into the manifest.

## Depends on

- [`auki-jcs`](../auki-jcs) — for canonicalizing the manifest JSON.
- [`auki-hash`](../auki-hash) — for hash-pinning referenced registry entries.
