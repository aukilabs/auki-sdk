# auki-geometry

Pure spatial math over [`auki-registry`](../auki-registry) frame declarations and [`auki-datatypes`](../auki-datatypes) pose types. Phase 1 ships convention conversion — the convention-only layer that sits underneath the future full `convert_pose` (which will also compose pose-log paths).

No registry IO, no log IO, no networking; just math.

**Status:** Shipped (Phase 1 — convention conversion only).

## Public surface

- `convert_pose_convention(pose, from, to)` — re-express both ends of a `SpatialTransform` at once, for frames whose only difference is convention.
- `convert_transform_source_convention(transform, from, to)`, `convert_transform_target_convention(transform, from, to)` — re-express only one side (input or output) of a `from -> to` transform, leaving the other side's convention untouched. Reject `from`/`to` pairs with mismatched handedness or length units, since a one-sided conversion between those would need a scaled or improper rotation that a quaternion can't represent.
- `convert_point_convention(...)`, `convert_vector_convention(...)`, `convert_direction_convention(...)` — the three companion conversions for points, length-bearing vectors, and unitless directions.
- `compose_spatial_transforms(...)`, `inverse_spatial_transform(...)`, `relative_spatial_transform(...)` — transform chaining: compose `from->mid` with `mid->to`, invert a transform, or derive `from->to` from two transforms sharing a common frame.
- `spatial_transform_to_matrix4(...)`, `spatial_transform_from_matrix4(...)` — convert a `SpatialTransform` to/from a 4×4 homogeneous matrix.
- `axis_convention_matrix(...)`, `convention_matrix(...)`, `meters_per_unit(...)` — the underlying primitives.

## Depends on

- [`auki-datatypes`](../auki-datatypes) — for `SpatialTransform`, `Vec3`, `Quat`.
- [`auki-registry`](../auki-registry) — for `FrameRegistryEntry` convention declarations.
