# Changelog — auki-geometry

Detailed changes for `auki-geometry`. Latest entry on top.

---

### Nils's codex · May 15, 2026

Initial `auki-geometry` crate scaffolded as the pure spatial-math home for the SDK. Phase 1 ships convention conversion: `meters_per_unit`, `axis_convention_matrix`, `convention_matrix`, `convert_point_convention`, `convert_vector_convention`, `convert_direction_convention`, and `convert_pose_convention`. The crate depends on `auki-registry` for `FrameRegistryEntry` declarations and `auki-datatypes` for `SpatialTransform` / `Vec3` / `Quat`, but does no registry IO, log IO, or networking. Conversion is convention-agnostic at the public API: direct declared convention A -> declared convention B, with no exposed canonical Auki frame. `convert_pose_convention` re-expresses the same physical pose in a target convention; full pose-log graph/path `convert_pose` remains future work. Tests lock ROS optical/body, OpenGL/Three.js, and Unity signed permutations; unit scaling; handedness mismatch rejection; and quaternion basis-change behavior.
