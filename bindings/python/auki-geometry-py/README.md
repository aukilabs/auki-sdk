# auki-geometry-py

PyO3 bindings for [`auki-geometry`](../../../crates/auki-geometry). Lets Python consumers compose, invert, and convert spatial transforms and frame conventions on equal footing with Rust callers.

**Status:** Shipped.

## Surface

Math types (`Vec3`, `Quat`, `SpatialTransform`) cross the seam as flat float arrays — matching the robotics-team convention:

- `Vec3` ↔ `[x, y, z]`
- `Quat` ↔ `[qx, qy, qz, qw]` (scalar-last, Hamilton / ROS / prost wire order)
- `SpatialTransform` ↔ `[tx, ty, tz, qx, qy, qz, qw]`

Categorical types (`FrameRegistryEntry`, `AxisConvention`, axis directions, length units, handedness) stay as dicts and strings — exactly what [`auki-registry-py`](../auki-registry-py) returns.

### Public functions

- `meters_per_unit(unit)` — `"meters"` / `"centimeters"` / `"millimeters"` → float multiplier.
- `axis_convention_matrix(from_axes, to_axes)` → 3×3 nested list.
- `convention_matrix(from_entry, to_entry)` → 4×4 nested list (axis permutation + unit scale).
- `convert_point_convention(point, from_entry, to_entry)` → 3-list (axis + unit).
- `convert_vector_convention(vector, from_entry, to_entry)` → 3-list (axis + unit).
- `convert_direction_convention(direction, from_entry, to_entry)` → 3-list (axis only, no unit scale).
- `convert_pose_convention(pose, from_entry, to_entry)` → 7-list.
- `inverse_spatial_transform(transform)` → 7-list.
- `compose_spatial_transforms(from_to_mid, mid_to_to)` → 7-list (the `from→to` composition).
- `relative_spatial_transform(common_to_from, common_to_to)` → 7-list (the `from→to` derivation).
- `spatial_transform_to_matrix4(pose)` → 4×4 nested list.
- `spatial_transform_from_matrix4(matrix)` → 7-list.

### Errors

- `GeometryError` (subclass of `ValueError`) — raised for invalid axes, handedness mismatch, or zero-length orientation quaternion.
- Plain `ValueError` — raised for array length mismatches, non-numeric elements, or malformed registry dicts.

## Depends on

- [`auki-geometry`](../../../crates/auki-geometry) — Rust crate it wraps.
- [`auki-registry-py`](../auki-registry-py) — source of `FrameRegistryEntry` dicts the convention helpers consume.
