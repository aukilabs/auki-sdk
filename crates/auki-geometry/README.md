# auki-geometry

Pure spatial math over [`auki-registry`](../auki-registry) frame declarations and [`auki-datatypes`](../auki-datatypes) pose types. Phase 1 ships convention conversion — the convention-only layer that sits underneath the future full `convert_pose` (which will also compose pose-log paths).

No registry IO, no log IO, no networking; just math.

**Status:** Shipped (Phase 1 — convention conversion only).

## Public surface

- `convert_pose_convention(pose, from, to)` — translate a `SpatialTransform` between frames whose only difference is convention.
- `convert_point_convention(...)`, `convert_vector_convention(...)`, `convert_direction_convention(...)` — the three companion conversions for points, length-bearing vectors, and unitless directions.
- `axis_convention_matrix(...)`, `convention_matrix(...)`, `raster_convention_matrix(...)`, `meters_per_unit(...)` — the underlying primitives.

## Depends on

- [`auki-datatypes`](../auki-datatypes) — for `SpatialTransform`, `Vec3`, `Quat`.
- [`auki-registry`](../auki-registry) — for `FrameRegistryEntry` convention declarations.
