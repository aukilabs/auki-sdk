# auki-datatypes-py

Python bindings for [`auki-datatypes`](../../../crates/auki-datatypes) — `betterproto`-generated dataclasses for the SDK's shared protobuf segment + wire payload types. Cross-language consumers regenerate from the same `.proto` files; this package ships the Python regeneration.

**Status:** Shipped.

## Public surface

`betterproto` dataclasses mirroring the Rust prost types, including:

- `auki.camera.CameraFrame`, `auki.camera.DynamicIntrinsics`
- `auki.point_cloud.Data`, `auki.audio.Data`, `auki.joint_encoders.Data`, `auki.scalar.Data`
- `auki.detection.DetectionFrame`
- `auki.pose.SpatialTransform`, `auki.pose.Vec3`, `auki.pose.Quat`
- `auki.map.MapUpdate`, `auki.map.VoxelChunkUpdate`, `auki.map.VoxelDelta`, `auki.map.SemanticDelta`
- `auki.time_transform.TimeTransformEntry`
- `auki.stream.StreamMessage` / `StreamRequest` / `StreamManifest` / `StreamEntry` / `DeclineReason` / `EndReason`

Locked test vectors in `tests/test_locked_vectors.py` pin the wire-byte output against the Rust crate's locked vectors.

## Depends on

- [`auki-datatypes`](../../../crates/auki-datatypes) — `.proto` schemas regenerated for Python.
- `betterproto==1.2.5` (runtime) — wire-format-compat pin.
