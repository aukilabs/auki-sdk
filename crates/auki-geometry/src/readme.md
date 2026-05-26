# `auki-geometry/src/`

Implementation status of `auki-geometry`. Spec: this crate's [outer `README.md`](../README.md).

## What's here

- [`core.rs`](core.rs) — binding-free typed geometry implementation and JSON-string adapters.
- [`lib.rs`](lib.rs) — feature-gated module wiring and crate-root re-exports.
- [`ffi.rs`](ffi.rs) — native UniFFI adapters for generated Python and Swift.
- [`wasm.rs`](wasm.rs) — wasm-bindgen adapters for generated JavaScript/WebAssembly.

## Public surface

```rust
pub type Matrix3 = [[f64; 3]; 3];
pub type Matrix4 = [[f64; 4]; 4];

pub enum GeometryError {
    InvalidJson(String),
    InvalidAxes(String),
    HandednessMismatch { frame_id, declared, axes_determinant },
    ZeroQuaternion,
}

pub fn meters_per_unit(unit: LengthUnit) -> f64;
pub fn axis_convention_matrix(from: &AxisConvention, to: &AxisConvention) -> Result<Matrix3>;
pub fn convention_matrix(from: &FrameRegistryEntry, to: &FrameRegistryEntry) -> Result<Matrix4>;
pub fn convert_point_convention(point: Vec3, from: &FrameRegistryEntry, to: &FrameRegistryEntry) -> Result<Vec3>;
pub fn convert_vector_convention(vector: Vec3, from: &FrameRegistryEntry, to: &FrameRegistryEntry) -> Result<Vec3>;
pub fn convert_direction_convention(direction: Vec3, from: &FrameRegistryEntry, to: &FrameRegistryEntry) -> Result<Vec3>;
pub fn convert_pose_convention(pose: &SpatialTransform, from: &FrameRegistryEntry, to: &FrameRegistryEntry) -> Result<SpatialTransform>;

pub fn meters_per_unit_json(unit: &str) -> Result<f64>;
pub fn axis_convention_matrix_json(from_axes_json: &str, to_axes_json: &str) -> Result<String>;
pub fn convention_matrix_json(from_frame_json: &str, to_frame_json: &str) -> Result<String>;
pub fn convert_point_convention_json(point_json: &str, from_frame_json: &str, to_frame_json: &str) -> Result<String>;
pub fn convert_vector_convention_json(vector_json: &str, from_frame_json: &str, to_frame_json: &str) -> Result<String>;
pub fn convert_direction_convention_json(direction_json: &str, from_frame_json: &str, to_frame_json: &str) -> Result<String>;
pub fn convert_pose_convention_json(pose_json: &str, from_frame_json: &str, to_frame_json: &str) -> Result<String>;
```

## Generated binding adapters

Generated Python, Swift, and JavaScript use the `*_json` functions. Frame inputs
are full `FrameRegistryEntry` JSON strings. Vector inputs and outputs are
`{"x": f64, "y": f64, "z": f64}`; quaternions are
`{"x": f64, "y": f64, "z": f64, "w": f64}`; pose JSON carries nullable
`translation` and `orientation` fields.

## Current behavior

- Axis convention conversion is a signed permutation between two declared `AxisConvention`s.
- Point/vector conversion applies axis permutation plus `LengthUnit` scale.
- Direction conversion applies axis permutation only.
- Pose conversion re-expresses `SpatialTransform.translation` and `SpatialTransform.orientation` in the target convention. Translation uses point/vector conversion. Orientation uses quaternion -> matrix, basis change, then matrix -> quaternion.
- Missing protobuf submessages (`translation: None`, `orientation: None`) are preserved as missing.
- `FrameRegistryEntry` inputs are rejected if the axis determinant disagrees with the declared `Handedness`.

## Tests

`cargo test -p auki-geometry` covers:

| Test | Asserts |
|------|---------|
| `meters_per_unit_is_locked` | `Meters`, `Centimeters`, and `Millimeters` scale factors. |
| `json_point_conversion_matches_locked_vector` | JSON adapter point conversion vector for generated bindings. |
| `json_pose_conversion_preserves_missing_submessages` | JSON adapter preserves nullable pose fields. |
| `ros_optical_to_opengl_axis_matrix_is_locked` | REP-103 optical -> OpenGL / Three.js signed permutation. |
| `ros_body_to_opengl_axis_matrix_is_locked` | REP-103 body -> OpenGL / Three.js signed permutation. |
| `unity_to_opengl_axis_matrix_is_locked` | Unity -> OpenGL handedness-crossing signed permutation. |
| `point_conversion_applies_axes_and_units` | Length-bearing conversion scales centimeters to meters. |
| `direction_conversion_does_not_apply_units` | Unitless directions ignore length-unit scale. |
| `convention_matrix_round_trips_to_identity` | All preset pairs round-trip through axis conversion. |
| `handedness_mismatch_is_rejected` | Inconsistent frame metadata fails before conversion. |
| `convert_pose_convention_reexpresses_translation_and_orientation` | Pose translation and quaternion basis change. |
| `converted_orientation_preserves_rotated_vectors` | Converted quaternion acts consistently on converted directions. |

`tests/surface.rs` adds the binding-standard source-compatibility check for the
crate-root Rust API when the default UniFFI feature is enabled.

## Dependencies

- [`auki-registry`](../../auki-registry) — frame convention declarations.
- [`auki-proto`](../../auki-proto) — generated `SpatialTransform`, `Vec3`, and `Quat`.

No filesystem, registry IO, networking, or log dependencies.
