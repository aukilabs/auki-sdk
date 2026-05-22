# auki-geometry

Pure spatial math for the Auki SDK.

This crate is the home for operations that *use* registry declarations and payload datatypes without owning either:

- [`auki-registry`](../auki-registry) declares frame conventions (`FrameRegistryEntry`, `AxisConvention`, `LengthUnit`).
- [`auki-proto`](../auki-proto) declares generated pose payloads (`SpatialTransform`, `Vec3`, `Quat`).
- `auki-geometry` converts and composes those values.

It does not read registry files, fetch missing hashes, open logs, or speak libp2p. Callers resolve the relevant entries first; this crate only performs deterministic geometry.

## Phase 1: convention conversion

`convert_pose_convention` re-expresses the same physical pose in another declared coordinate convention:

```rust
use auki_geometry::convert_pose_convention;
use auki_registry::FrameRegistryEntry;

let from = FrameRegistryEntry::ros_optical("producer/head_cam_points");
let to = FrameRegistryEntry::opengl("viewer/world");

let converted = convert_pose_convention(&pose, &from, &to)?;
```

This is **not** full `convert_pose`. It does not search a frame graph, compose pose-log edges, or account for physical offsets between named frames. It is the convention-only layer that full `convert_pose` will use while traversing pose logs.

## Public surface

```rust
pub fn meters_per_unit(unit: LengthUnit) -> f64;

pub fn axis_convention_matrix(
    from: &AxisConvention,
    to: &AxisConvention,
) -> Result<[[f64; 3]; 3]>;

pub fn convention_matrix(
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<[[f64; 4]; 4]>;

pub fn convert_point_convention(
    point: Vec3,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<Vec3>;

pub fn convert_vector_convention(
    vector: Vec3,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<Vec3>;

pub fn convert_direction_convention(
    direction: Vec3,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<Vec3>;

pub fn convert_pose_convention(
    pose: &SpatialTransform,
    from: &FrameRegistryEntry,
    to: &FrameRegistryEntry,
) -> Result<SpatialTransform>;
```

`convert_point_convention` and `convert_vector_convention` apply both axis permutation and length-unit scaling. `convert_direction_convention` applies axis permutation only, for unitless directions such as ray directions.

## Convention agnostic by design

The public API is direct convention A -> convention B. The SDK does not expose an Auki world frame or require producers to normalize to ROS, OpenGL, Unity, or any other convention before publishing. The only shared contract is the vocabulary in `AxisDirection`, `Handedness`, and `LengthUnit`.

`FrameRegistryEntry` already stores both an axis triplet and a declared handedness. `auki-registry` validates only orthogonality at write time; `auki-geometry` additionally rejects entries whose axis determinant disagrees with the declared handedness, because converting geometry from inconsistent metadata should fail loudly.

## Future scope

`auki-geometry` is the natural home for:

- full `convert_pose` over pose-log graph/path composition,
- pose composition, inverse, and interpolation,
- ray construction from camera intrinsics,
- ray-plane / ray-AABB / ray-mesh intersections,
- frustum and projection helpers,
- point-cloud convention transforms.

Networking, registry exchange, inline dictionaries, and file IO stay in their owning crates.
