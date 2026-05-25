# auki-geometry-py Design

Date: 2026-05-22
Status: Approved for implementation (pending second colleague review of array-shape pivot)

## Revision Note

Initial draft (also dated 2026-05-22) used a dict surface for `Vec3` / `Quat` / `SpatialTransform` to match the established convention of `auki-registry-py` and `auki-manifests-py`. The robotics team — the first concrete consumer — pushed back: their working formats for poses and transforms are flat `[x, y, z, qx, qy, qz, qw]` arrays and 4×4 transformation matrices, not field-named dicts.

The original convention defended a false consistency: dicts fit `registry-py` because its data is **categorical** (frame ids, axis direction enums, handedness, units); the same shape fits **numeric** data poorly. Pivoting math types to arrays leaves the registry surface untouched and brings the binding into line with how robotics callers actually use these shapes (ROS `geometry_msgs/Quaternion` scalar-last order, tf2, Bullet, prost wire format — all agree).

## Goal

Ship `bindings/python/auki-geometry-py` as the Python surface for [`auki-geometry`](../../../crates/auki-geometry). Mirror the full public Rust API (convention helpers plus the PR #193 spatial transform composition helpers) so Python sidecars — and any future Python consumer of the SDK's spatial math — can convert frame conventions, invert / compose / derive `SpatialTransform`s, and read convention matrices without re-implementing the math.

Math types (`Vec3`, `Quat`, `SpatialTransform`) cross the Python seam as flat float arrays. Matrices as nested float lists. Categorical types (`FrameRegistryEntry`, `AxisConvention`, `LengthUnit`, `Handedness`) remain dicts and strings — matching `auki-registry-py` since those values already live there.

This PR also adds two small public helpers to the underlying `auki-geometry` Rust crate (`spatial_transform_to_matrix4` and `spatial_transform_from_matrix4`) since the existing `quat_to_matrix` / `matrix_to_quat` math is private; without them the Python bridge helpers would duplicate math or hand-reach into internals.

## Non-Goals

- No `Vec3` / `Quat` / `SpatialTransform` PyO3 class wrappers — bare arrays cover the use case the robotics team raised.
- No NumPy dependency — inputs duck-typed `Sequence[float]` (lists, tuples, `np.ndarray` all work); outputs always plain Python `list[float]`. Callers can wrap with `np.array(...)` if they want.
- No `pose_to_dict` / `pose_from_dict` JCS round-trip helpers. If Park or Booster needs them later, follow-up card.
- No helpers beyond `auki-geometry`'s current public surface plus the two new `spatial_transform_to_matrix4` / `spatial_transform_from_matrix4` Rust helpers (no full `convert_pose`, no interpolation, no pose-log path-finding).

## Public Surface

All ten of `auki-geometry`'s public functions are exposed as top-level `#[pyfunction]`s on the `auki_geometry` module, plus two new bridge helpers and one custom exception class.

### Scalars

```python
auki_geometry.meters_per_unit(unit: str) -> float
```
- `unit` is one of `"meters"`, `"centimeters"`, `"millimeters"`. Anything else raises `GeometryError`.

### Convention matrices

```python
auki_geometry.axis_convention_matrix(from_axes: dict, to_axes: dict) -> list[list[float]]
auki_geometry.convention_matrix(from_entry: dict, to_entry: dict) -> list[list[float]]
```
- `axis_convention_matrix` returns a 3×3 row-major nested list.
- `convention_matrix` returns a 4×4 row-major nested list, with the scale on the diagonal and a zero translation column.
- Both raise `GeometryError` on invalid axes or handedness mismatch.

### Point / vector / direction / pose conversion

```python
auki_geometry.convert_point_convention(point: Sequence[float], from_entry: dict, to_entry: dict) -> list[float]
auki_geometry.convert_vector_convention(vector: Sequence[float], from_entry: dict, to_entry: dict) -> list[float]
auki_geometry.convert_direction_convention(direction: Sequence[float], from_entry: dict, to_entry: dict) -> list[float]
auki_geometry.convert_pose_convention(pose: Sequence[float], from_entry: dict, to_entry: dict) -> list[float]
```
- Points / vectors / directions are 3-element `[x, y, z]` arrays.
- Poses are 7-element `[tx, ty, tz, qx, qy, qz, qw]` arrays.
- `convert_point_convention` / `convert_vector_convention` apply axis permutation + unit scale.
- `convert_direction_convention` applies axis permutation only (unitless).
- `convert_pose_convention` re-expresses translation and orientation in the target convention.

### Spatial transform composition (PR #193 additions)

```python
auki_geometry.inverse_spatial_transform(transform: Sequence[float]) -> list[float]
auki_geometry.compose_spatial_transforms(from_to_mid: Sequence[float], mid_to_to: Sequence[float]) -> list[float]
auki_geometry.relative_spatial_transform(common_to_from: Sequence[float], common_to_to: Sequence[float]) -> list[float]
```
- All inputs and outputs are 7-element pose arrays.
- Raise `GeometryError` on zero-length orientation quaternions.

### 4×4 ↔ 7-array bridge

```python
auki_geometry.spatial_transform_to_matrix4(pose: Sequence[float]) -> list[list[float]]
auki_geometry.spatial_transform_from_matrix4(matrix: Sequence[Sequence[float]]) -> list[float]
```
- `spatial_transform_to_matrix4` builds a 4×4 homogeneous transformation matrix from a 7-array.
- `spatial_transform_from_matrix4` extracts the translation from `M[0..3][3]` and converts the upper-left 3×3 rotation submatrix to a quaternion. Small numerical drift in the rotation submatrix is tolerated and normalized through; severely non-orthogonal input still produces a unit quaternion but it may not represent a useful rotation (matches the project's "tolerate small drift, don't silently renormalize" stance from `pose.proto`).

## Data Shapes At The Seam

| Type | Python representation |
|---|---|
| `Vec3` | `[x, y, z]` — 3-element `Sequence[float]` in / `list[float]` out |
| `Quat` | `[qx, qy, qz, qw]` — 4-element scalar-last (Hamilton / ROS / prost order) |
| `SpatialTransform` | `[tx, ty, tz, qx, qy, qz, qw]` — 7-element flat array |
| `Matrix3` | `[[float; 3]; 3]` row-major nested list |
| `Matrix4` | `[[float; 4]; 4]` row-major nested list |
| `FrameRegistryEntry` | Exactly the dict returned by `auki_registry.frame_*` (`frame_id`, `handedness`, `axes`, `units`) |
| `AxisConvention` | `{"x": dir, "y": dir, "z": dir}` where `dir` is one of `"right"`, `"left"`, `"up"`, `"down"`, `"forward"`, `"backward"` |
| `LengthUnit` | Lowercase string: `"meters"`, `"centimeters"`, `"millimeters"` |
| `Handedness` | Lowercase string: `"right"`, `"left"` |

### Option<Vec3> / Option<Quat> Semantic

Rust's `SpatialTransform` has `translation: Option<Vec3>` and `orientation: Option<Quat>`. Rust callers can pass `None` to mean "zero translation" or "identity orientation"; the PR #193 helpers treat them that way and emit `Some(...)` on output.

In a flat 7-array there is no way to express `None`. The Python seam enforces both-present-on-input — callers pass `[0, 0, 0, 0, 0, 0, 1]` to mean "identity transform." The output side is unchanged (Rust output guarantees both `Some(...)`, so the 7-array carries everything). For convention conversion, Python users likewise pass full 7-arrays.

This is a deliberate small loss: the explicit "this field wasn't set" signal disappears at the Python boundary. The math is identical. Users who need the explicit absence signal would have to round-trip through the wire format manually — flag if anyone hits this.

## Marshaling

The seam logic lives entirely in `bindings/python/auki-geometry-py/src/lib.rs` plus the two new public Rust helpers in `crates/auki-geometry/src/lib.rs`. No other upstream crates change.

### Math types — flat-array extraction

`auki-datatypes`'s prost-generated `Vec3` / `Quat` / `SpatialTransform` are constructed field-by-field from the flat arrays:

```rust
fn extract_vec3_array(any: &Bound<'_, PyAny>) -> PyResult<Vec3>;        // expects length-3 sequence
fn extract_quat_array(any: &Bound<'_, PyAny>) -> PyResult<Quat>;        // expects length-4 sequence
fn extract_spatial_transform_array(any: &Bound<'_, PyAny>) -> PyResult<SpatialTransform>;  // expects length-7

fn vec3_to_pylist(py: Python<'_>, v: &Vec3) -> PyResult<PyObject>;       // [x, y, z]
fn quat_to_pylist(py: Python<'_>, q: &Quat) -> PyResult<PyObject>;       // [x, y, z, w]
fn spatial_transform_to_pylist(py: Python<'_>, t: &SpatialTransform) -> PyResult<PyObject>;  // [tx,ty,tz,qx,qy,qz,qw]
```

Length mismatches raise `ValueError`. Non-numeric elements surface PyO3's standard extraction error. Output is always a plain Python `list`. Both `translation` and `orientation` in the constructed `SpatialTransform` are `Some(...)` — the Rust composition helpers accept that shape.

### Registry types — serde round-trip

`auki-registry`'s `FrameRegistryEntry` and `AxisConvention` already derive `serde::{Serialize, Deserialize}` (used by `auki-registry-py`). `auki-geometry-py` reuses the same pattern:

```rust
fn parse_frame_entry(py: Python<'_>, any: &Bound<'_, PyAny>) -> PyResult<FrameRegistryEntry>;
fn parse_axis_convention(py: Python<'_>, any: &Bound<'_, PyAny>) -> PyResult<AxisConvention>;
```

Both go through `json.dumps(any)` → `serde_json::from_str`, mirroring `auki-registry-py`'s `parse_py`.

### Matrices

```rust
fn matrix3_to_pylist(py: Python<'_>, m: Matrix3) -> PyResult<PyObject>;
fn matrix4_to_pylist(py: Python<'_>, m: Matrix4) -> PyResult<PyObject>;
fn extract_matrix4(any: &Bound<'_, PyAny>) -> PyResult<Matrix4>;        // for spatial_transform_from_matrix4
```

`matrix*_to_pylist` builds `PyList` of `PyList` of `f64` in row-major order. `extract_matrix4` walks a 4-row × 4-col `Sequence[Sequence[float]]` and raises `ValueError` on shape mismatch.

## Rust Crate Additions

`crates/auki-geometry/src/lib.rs` gains two public helpers (small — the math already exists privately):

```rust
/// Build a 4×4 homogeneous transformation matrix from a SpatialTransform.
/// Missing translation/orientation are treated as zero/identity, matching
/// the rest of the PR 193 composition helpers' input contract.
pub fn spatial_transform_to_matrix4(transform: &SpatialTransform) -> Result<Matrix4>;

/// Decompose a 4×4 homogeneous transformation matrix into a SpatialTransform.
/// Translation comes from the right column; rotation comes from the upper-left
/// 3×3 submatrix via the existing `matrix_to_quat` path (small numerical drift
/// in the submatrix is tolerated and normalized through).
pub fn spatial_transform_from_matrix4(matrix: Matrix4) -> Result<SpatialTransform>;
```

Both delegate to the existing private `quat_to_matrix` / `matrix_to_quat` / `apply_matrix3_to_vec3` helpers. Implementation is roughly:

- `spatial_transform_to_matrix4`: read translation (zero if `None`); read orientation (identity if `None`); build matrix from rotation in upper-left 3×3, translation in the right column, `1` in `[3][3]`.
- `spatial_transform_from_matrix4`: extract upper-left 3×3, call `matrix_to_quat`; extract translation from `[0][3]`, `[1][3]`, `[2][3]`; return `SpatialTransform { translation: Some(...), orientation: Some(...) }`. Bottom row is not validated (callers' responsibility — matches the project's "tolerate small drift" stance).

Rust unit tests cover: identity round-trip, translation-only round-trip, rotation-only round-trip, full pose round-trip, and the result of `spatial_transform_from_matrix4(spatial_transform_to_matrix4(t))` matching `t` for several non-trivial poses.

## Error Handling

`auki-geometry`'s `GeometryError` enum (`InvalidAxes`, `HandednessMismatch`, `ZeroQuaternion`) is surfaced as a custom Python exception class:

```python
class GeometryError(ValueError):
    """Auki geometry validation errors — invalid axes, handedness mismatch, or zero-length quaternion."""
```

Declared via PyO3's `create_exception!(auki_geometry, GeometryError, pyo3::exceptions::PyValueError)` macro (same pattern `auki-network-py` uses) and added to the module via `m.add("GeometryError", py.get_type_bound::<GeometryError>())?;`. Each Rust variant's `Display` becomes the Python exception message. Subclassing `ValueError` lets consumers catch either broadly (`except ValueError`) or precisely (`except auki_geometry.GeometryError`).

Marshaling errors — array length mismatches, non-numeric elements, malformed registry dicts — raise plain `ValueError` via the standard PyO3 path; they are **not** `GeometryError`s.

## File Layout

```
crates/auki-geometry/src/
  lib.rs                # +2 public helpers, +unit tests

bindings/python/auki-geometry-py/
  Cargo.toml            # pyo3 0.22 abi3-py38, cdylib+rlib, extension-module feature
  pyproject.toml        # maturin, module-name = "auki_geometry"
  README.md             # surface summary, mirrors auki-registry-py
  src/
    lib.rs              # one #[pymodule], twelve #[pyfunction]s, GeometryError, marshal helpers
  python_tests/
    test_geometry.py    # pytest smoke tests for every function + every error variant + bridge round-trips
```

Root `Cargo.toml` workspace `members` gains `"bindings/python/auki-geometry-py"` alphabetically between the existing `auki-domain-py` and `auki-identity-py` entries.

## Cargo Dependencies

```toml
[dependencies]
auki-geometry-rs = { package = "auki-geometry", path = "../../../crates/auki-geometry" }
auki-datatypes = { path = "../../../crates/auki-datatypes" }
auki-registry-rs = { package = "auki-registry", path = "../../../crates/auki-registry" }
pyo3 = { version = "0.22", features = ["abi3-py38"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[dev-dependencies]
pyo3 = { version = "0.22", features = ["abi3-py38", "auto-initialize"] }
```

`auki-geometry-rs` is the renamed import of the crate it wraps (mirroring `auki-registry-rs` and `auki-logs-rs` in their bindings — keeps the upstream crate name from colliding with this crate's own lib name `auki_geometry`). `auki-datatypes` is needed for `Vec3` / `Quat` / `SpatialTransform` construction in marshal helpers. `auki-registry-rs` is needed for `FrameRegistryEntry` / `AxisConvention` serde round-trip.

## Tests

`python_tests/test_geometry.py` is the source of truth for behavioral coverage. One pytest module with these cases:

1. `meters_per_unit` returns the locked multipliers for all three units; raises on an unknown unit string.
2. `axis_convention_matrix` returns the locked ROS-optical → OpenGL permutation `[[1,0,0],[0,-1,0],[0,0,-1]]`.
3. `convention_matrix` round-trips to identity over all four registry presets (ros_body, ros_optical, opengl, unity) — pairwise `A → B → A` reduces to identity 4×4.
4. `convert_point_convention([100, 200, 300], cm-source, m-target)` returns `[1, -2, -3]` (axis flip + unit scale).
5. `convert_direction_convention([1, 2, 3], cm-source, m-target)` returns `[1, -2, -3]` (axis flip only, no unit scale).
6. `convert_pose_convention([1, 2, 3, 0, 0, 0.707, 0.707], ros-optical, opengl)` reproduces the Rust test case.
7. `inverse_spatial_transform([1, 2, 3, 0, 0, 0.707, 0.707])` composed with the original via `compose_spatial_transforms` reduces to identity `[0, 0, 0, 0, 0, 0, 1]`.
8. `compose_spatial_transforms` returns the correct `A → C` from a known `A → B` and `B → C` (rotation then translation).
9. `relative_spatial_transform([0,0,0,0,0,0,1], [1,0,0,0,0,0,1])` reproduces the Rust same-origin-derivation test case.
10. `spatial_transform_to_matrix4([0,0,0,0,0,0,1])` returns the 4×4 identity.
11. `spatial_transform_from_matrix4(spatial_transform_to_matrix4(t))` returns the original 7-array for several poses.
12. `GeometryError` is raised (as a `ValueError` subclass) when passing a zero quaternion, a handedness-mismatched frame, or an unknown axis direction string.
13. `ValueError` (not `GeometryError`) is raised when passing a 6-element array to a function expecting a 7-array; ditto for non-numeric elements; ditto for a 3×3 nested list to `spatial_transform_from_matrix4`.

The `auto-initialize` PyO3 dev-feature also lets a minimal `cargo test -p auki-geometry-py` smoke run pass without maturin — same shape as the other py-bindings. Plus `cargo test -p auki-geometry` covers the two new Rust helpers.

## Card Flow

Card already created: [#199](https://github.com/aukilabs/auki-sdk/issues/199), in the **Tasks** column.

Once colleague review of this revised spec is in, the developer moves the card to **In progress**. Then: branch off `develop` as `feat/199-auki-geometry-py`, commit the spec, write the implementation plan via `superpowers:writing-plans`, implement, open PR with `Closes #199` in the body, move the card to **In review**.

## Out Of Scope

- `pose_to_dict` / `pose_from_dict` helpers for JCS-canonical round-trip. Add if Park or Booster needs them.
- Pyclass migration for `auki-datatypes-py` / `auki-registry-py` / `auki-manifests-py` / `auki-geometry-py` — separate architecture project; deferred unless registry-py migrates first.
- Full `convert_pose` that traverses a pose-log graph — Rust crate doesn't expose it yet.
- Interpolation between `SpatialTransform`s — Rust crate doesn't expose it yet.
- NumPy / ndarray return types — adds a heavy dep for callers who don't need it; users can `np.array(result)` on plain lists.
- Swift bindings for `auki-geometry` — separate task if/when needed.
