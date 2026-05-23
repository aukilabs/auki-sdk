# auki-geometry-py Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `bindings/python/auki-geometry-py` as PyO3 bindings for `auki-geometry`'s convention helpers, spatial transform composition helpers (PR #193), and two new 4×4 ↔ 7-array bridge helpers — with math types crossing the Python seam as flat float arrays.

**Architecture:** Two-part change. (1) Two new public helpers in `crates/auki-geometry/src/lib.rs` (`spatial_transform_to_matrix4`, `spatial_transform_from_matrix4`) — small additions that reuse the crate's existing private `spatial_transform_rotation` / `spatial_transform_translation` / `matrix_to_quat` helpers. (2) A new binding crate `bindings/python/auki-geometry-py/` with one `#[pymodule]`, 12 `#[pyfunction]`s, a custom `GeometryError` exception class, and marshal helpers for flat arrays + registry dicts.

**Tech Stack:** Rust 2024 edition, PyO3 0.22 (abi3-py38), maturin, pytest, `auki-geometry`, `auki-datatypes` (prost-generated `Vec3` / `Quat` / `SpatialTransform`), `auki-registry` (serde-derived `FrameRegistryEntry` / `AxisConvention`).

**GitHub Task:** [#199](https://github.com/aukilabs/auki-sdk/issues/199)

**Spec:** [`docs/superpowers/specs/2026-05-22-auki-geometry-py-design.md`](../specs/2026-05-22-auki-geometry-py-design.md)

---

## File Structure

```
crates/auki-geometry/
  src/lib.rs                                    # MODIFY: +2 public helpers, +unit tests

bindings/python/auki-geometry-py/               # CREATE
  Cargo.toml                                    # CREATE
  pyproject.toml                                # CREATE
  README.md                                     # CREATE
  src/lib.rs                                    # CREATE — module + pyfunctions + marshal helpers
  python_tests/test_geometry.py                 # CREATE — pytest smoke tests

Cargo.toml                                      # MODIFY: add binding to workspace members
```

Spec deviation note: the spec said workspace member placement is "alphabetically between auki-domain-py and auki-identity-py." The actual convention in this workspace is **crate-then-its-bindings** (e.g. `auki-identity` immediately followed by `auki-identity-py`). This plan places `bindings/python/auki-geometry-py` right after `crates/auki-geometry`. The spec author (the previous brainstorming session) was wrong on placement; this plan supersedes that one detail.

All Rust crate additions go in the same file (`crates/auki-geometry/src/lib.rs`) as the existing helpers — adding two more public functions and unit tests is well within the file's current scope. All binding logic goes in one `src/lib.rs` (~300 lines, manageable single file).

---

### Task 1: Add Rust helpers — `spatial_transform_to_matrix4` and `spatial_transform_from_matrix4`

**Files:**
- Modify: `crates/auki-geometry/src/lib.rs`
- Test: `crates/auki-geometry/src/lib.rs` (existing `#[cfg(test)] mod tests` block)

- [ ] **Step 1: Add failing unit tests inside the existing `mod tests` block**

Add these tests at the end of the `mod tests { ... }` block in `crates/auki-geometry/src/lib.rs` (just before the closing `}` of the `mod tests` block):

```rust
    #[test]
    fn spatial_transform_to_matrix4_identity() {
        let identity = SpatialTransform {
            translation: Some(Vec3 { x: 0.0, y: 0.0, z: 0.0 }),
            orientation: Some(Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }),
        };
        assert_matrix4_close(
            spatial_transform_to_matrix4(&identity).unwrap(),
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        );
    }

    #[test]
    fn spatial_transform_to_matrix4_translation_only() {
        let t = SpatialTransform {
            translation: Some(Vec3 { x: 1.0, y: 2.0, z: 3.0 }),
            orientation: Some(Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }),
        };
        assert_matrix4_close(
            spatial_transform_to_matrix4(&t).unwrap(),
            [
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 2.0],
                [0.0, 0.0, 1.0, 3.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        );
    }

    #[test]
    fn spatial_transform_to_matrix4_rotation_only() {
        // 90° rotation around +Z: x→y, y→−x
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let t = SpatialTransform {
            translation: Some(Vec3 { x: 0.0, y: 0.0, z: 0.0 }),
            orientation: Some(Quat { x: 0.0, y: 0.0, z: half, w: half }),
        };
        assert_matrix4_close(
            spatial_transform_to_matrix4(&t).unwrap(),
            [
                [0.0, -1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        );
    }

    #[test]
    fn spatial_transform_to_matrix4_treats_missing_as_zero_identity() {
        // Both None: should produce 4x4 identity.
        let none = SpatialTransform { translation: None, orientation: None };
        assert_matrix4_close(
            spatial_transform_to_matrix4(&none).unwrap(),
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        );
    }

    #[test]
    fn spatial_transform_from_matrix4_round_trip() {
        // Build a pose, send it to matrix4, decode it back. Should round-trip.
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let original = SpatialTransform {
            translation: Some(Vec3 { x: 1.0, y: 2.0, z: 3.0 }),
            orientation: Some(Quat { x: 0.0, y: 0.0, z: half, w: half }),
        };
        let matrix = spatial_transform_to_matrix4(&original).unwrap();
        let decoded = spatial_transform_from_matrix4(matrix).unwrap();
        assert_vec3_close(decoded.translation.unwrap(), original.translation.unwrap());
        assert_quat_equivalent(decoded.orientation.unwrap(), original.orientation.unwrap());
    }

    #[test]
    fn spatial_transform_from_matrix4_identity() {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let decoded = spatial_transform_from_matrix4(identity).unwrap();
        assert_vec3_close(decoded.translation.unwrap(), Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        assert_quat_equivalent(
            decoded.orientation.unwrap(),
            Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },
        );
    }
```

Also add this helper near the top of the `mod tests { ... }` block, right after `assert_matrix3_close`:

```rust
    fn assert_matrix4_close(actual: Matrix4, expected: Matrix4) {
        for row in 0..4 {
            for col in 0..4 {
                assert!(
                    (actual[row][col] - expected[row][col]).abs() < 1.0e-9,
                    "matrix4 mismatch at [{row}][{col}]: actual={} expected={}",
                    actual[row][col],
                    expected[row][col]
                );
            }
        }
    }
```

- [ ] **Step 2: Run the tests, confirm they fail with the expected error**

Run: `cargo test -p auki-geometry spatial_transform_to_matrix4 spatial_transform_from_matrix4`
Expected: compile errors — `cannot find function spatial_transform_to_matrix4 in this scope` and similar for `_from_matrix4`. (The new test helper `assert_matrix4_close` will compile but be unused — that's fine.)

- [ ] **Step 3: Implement the two public helpers**

Add these two public functions to `crates/auki-geometry/src/lib.rs`. Place them immediately after `relative_spatial_transform` (around line 218) and before the existing private `convert_orientation_convention`:

```rust
/// Build a 4×4 homogeneous transformation matrix from a `SpatialTransform`.
///
/// The matrix has the rotation in the upper-left 3×3, the translation in
/// the right column, and `[0, 0, 0, 1]` as the bottom row. Missing
/// translation is treated as zero; missing orientation is treated as
/// identity — matching the input contract of the PR #193 composition
/// helpers.
pub fn spatial_transform_to_matrix4(transform: &SpatialTransform) -> Result<Matrix4> {
    let rotation = spatial_transform_rotation(transform)?;
    let translation = spatial_transform_translation(transform);
    Ok([
        [rotation[0][0], rotation[0][1], rotation[0][2], translation.x],
        [rotation[1][0], rotation[1][1], rotation[1][2], translation.y],
        [rotation[2][0], rotation[2][1], rotation[2][2], translation.z],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

/// Decompose a 4×4 homogeneous transformation matrix into a
/// `SpatialTransform`.
///
/// Translation comes from the right column (`matrix[0..3][3]`).
/// Rotation comes from the upper-left 3×3 submatrix via `matrix_to_quat`,
/// which normalizes the result — small numerical drift in the rotation
/// submatrix is tolerated. The bottom row of the input is not validated;
/// callers are responsible for supplying a proper homogeneous transform.
pub fn spatial_transform_from_matrix4(matrix: Matrix4) -> Result<SpatialTransform> {
    let rotation: Matrix3 = [
        [matrix[0][0], matrix[0][1], matrix[0][2]],
        [matrix[1][0], matrix[1][1], matrix[1][2]],
        [matrix[2][0], matrix[2][1], matrix[2][2]],
    ];
    let translation = Vec3 {
        x: matrix[0][3],
        y: matrix[1][3],
        z: matrix[2][3],
    };
    let orientation = matrix_to_quat(rotation)?;
    Ok(SpatialTransform {
        translation: Some(translation),
        orientation: Some(orientation),
    })
}
```

- [ ] **Step 4: Run the tests, confirm they pass**

Run: `cargo test -p auki-geometry`
Expected: all tests pass, including the six new ones. Output ends with `test result: ok. <N> passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-geometry/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(auki-geometry): add 4x4 transformation matrix bridge helpers (#199)

why: the existing quat_to_matrix and matrix_to_quat helpers are private.
The forthcoming auki-geometry-py binding needs public access so its
robotics-facing 4x4 surface can avoid duplicating the math.

how: add spatial_transform_to_matrix4 (rotation in upper-left 3x3,
translation in right column) and spatial_transform_from_matrix4
(extract translation from right column, convert upper-left 3x3 to
quaternion via the existing matrix_to_quat path). Both delegate to
existing private helpers — no new math.

what: two new public functions plus six unit tests covering identity,
translation-only, rotation-only, missing-component, identity-decode,
and round-trip cases.

xoxo Broodsugar's exocortex
EOF
)"
```

---

### Task 2: Scaffold the binding crate

**Files:**
- Create: `bindings/python/auki-geometry-py/Cargo.toml`
- Create: `bindings/python/auki-geometry-py/pyproject.toml`
- Create: `bindings/python/auki-geometry-py/README.md`
- Create: `bindings/python/auki-geometry-py/src/lib.rs` (minimal — empty pymodule for now)
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create the Cargo manifest**

Write `bindings/python/auki-geometry-py/Cargo.toml`:

```toml
[package]
name = "auki-geometry-py"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "PyO3 bindings for auki-geometry's convention helpers, spatial transform composition, and 4x4 transformation matrix bridge — flat-array surface for Python robotics consumers."

[lib]
name = "auki_geometry"
crate-type = ["cdylib", "rlib"]

[features]
default = []
extension-module = ["pyo3/extension-module"]

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

- [ ] **Step 2: Create the pyproject manifest**

Write `bindings/python/auki-geometry-py/pyproject.toml`:

```toml
[build-system]
requires = ["maturin>=1.5,<2.0"]
build-backend = "maturin"

[project]
name = "auki-geometry-py"
version = "0.0.0"
description = "PyO3 bindings for auki-geometry's convention helpers and spatial transform composition — flat-array surface for Python robotics consumers."
readme = "README.md"
license = { text = "MIT" }
authors = [{ name = "Auki Labs Limited" }]
requires-python = ">=3.8"
classifiers = [
    "Programming Language :: Rust",
    "Programming Language :: Python :: Implementation :: CPython",
    "Operating System :: POSIX :: Linux",
    "Operating System :: MacOS",
    "License :: OSI Approved :: MIT License",
]

[project.urls]
Repository = "https://github.com/aukilabs/auki-sdk"

[tool.maturin]
features = ["pyo3/extension-module"]
module-name = "auki_geometry"

[tool.pytest.ini_options]
testpaths = ["python_tests"]
```

- [ ] **Step 3: Create the README**

Write `bindings/python/auki-geometry-py/README.md`:

```markdown
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
- `inverse_spatial_transform(pose)` → 7-list.
- `compose_spatial_transforms(a_to_b, b_to_c)` → 7-list (`a_to_c`).
- `relative_spatial_transform(common_to_a, common_to_b)` → 7-list (`a_to_b`).
- `spatial_transform_to_matrix4(pose)` → 4×4 nested list.
- `spatial_transform_from_matrix4(matrix)` → 7-list.

### Errors

- `GeometryError` (subclass of `ValueError`) — raised for invalid axes, handedness mismatch, or zero-length orientation quaternion.
- Plain `ValueError` — raised for array length mismatches, non-numeric elements, or malformed registry dicts.

## Depends on

- [`auki-geometry`](../../../crates/auki-geometry) — Rust crate it wraps.
- [`auki-registry-py`](../auki-registry-py) — source of `FrameRegistryEntry` dicts the convention helpers consume.
```

- [ ] **Step 4: Create a minimal `src/lib.rs`**

Write `bindings/python/auki-geometry-py/src/lib.rs` with just enough to compile:

```rust
//! PyO3 bindings for [`auki-geometry`](../../../../crates/auki-geometry).
//!
//! Math types cross the seam as flat float arrays — `[x, y, z]` for
//! `Vec3`, `[qx, qy, qz, qw]` for `Quat`, `[tx, ty, tz, qx, qy, qz, qw]`
//! for `SpatialTransform`. Categorical types (`FrameRegistryEntry`,
//! `AxisConvention`, etc.) stay as dicts and strings, matching
//! [`auki-registry-py`](../../auki-registry-py).

use pyo3::prelude::*;
use pyo3::types::PyModule;

#[pymodule]
fn auki_geometry(_py: Python<'_>, _m: &Bound<'_, PyModule>) -> PyResult<()> {
    Ok(())
}
```

- [ ] **Step 5: Register the binding in the workspace**

Modify `Cargo.toml` (workspace root). Find the line `"crates/auki-geometry",` (around line 6) and add a new line immediately after it:

```toml
    "crates/auki-geometry",
    "bindings/python/auki-geometry-py",
```

The list should now have `bindings/python/auki-geometry-py` between `crates/auki-geometry` and `crates/auki-identity` — matching the crate-then-binding pattern used elsewhere in the file.

- [ ] **Step 6: Confirm the crate builds and the workspace still compiles**

Run: `cargo check -p auki-geometry-py`
Expected: clean build (warnings about unused imports OK at this stage; no errors).

Run: `cargo check --workspace`
Expected: no errors anywhere; the new binding compiles alongside the rest of the workspace.

- [ ] **Step 7: Commit**

```bash
git add bindings/python/auki-geometry-py Cargo.toml
git commit -m "$(cat <<'EOF'
chore(auki-geometry-py): scaffold binding crate (#199)

why: lay the foundation for the PyO3 binding before any pyfunctions
land, so each subsequent task can focus on one slice of the surface.

how: Cargo.toml mirroring the auki-registry-py / auki-logs-py shape
(pyo3 0.22 abi3-py38, cdylib+rlib, extension-module feature; renamed
upstream crate to auki-geometry-rs); pyproject.toml configured for
maturin; minimal src/lib.rs with an empty #[pymodule]; README; added
to workspace members right after crates/auki-geometry.

what: new bindings/python/auki-geometry-py/ crate with empty module,
workspace integration, README documenting the planned surface.

xoxo Broodsugar's exocortex
EOF
)"
```

---

### Task 3: Marshal helpers and `GeometryError` exception class

**Files:**
- Modify: `bindings/python/auki-geometry-py/src/lib.rs`

This task lands the foundation every subsequent pyfunction will use: flat-array `Vec3` / `Quat` / `SpatialTransform` extraction, list output builders, matrix builders, registry dict serde round-trip, error mapping, and the custom exception class. No `#[pyfunction]`s yet — those come in the next tasks.

- [ ] **Step 1: Rewrite `src/lib.rs` with the full marshal infrastructure**

Replace the contents of `bindings/python/auki-geometry-py/src/lib.rs` with:

```rust
//! PyO3 bindings for [`auki-geometry`](../../../../crates/auki-geometry).
//!
//! Math types cross the seam as flat float arrays — `[x, y, z]` for
//! `Vec3`, `[qx, qy, qz, qw]` for `Quat` (scalar-last, Hamilton / ROS /
//! prost wire order), `[tx, ty, tz, qx, qy, qz, qw]` for
//! `SpatialTransform`. Categorical types (`FrameRegistryEntry`,
//! `AxisConvention`, etc.) stay as dicts and strings, matching
//! [`auki-registry-py`](../../auki-registry-py).
//!
//! `GeometryError` is a `ValueError` subclass declared via PyO3's
//! `create_exception!` macro, matching the pattern `auki-network-py`
//! uses for its custom exception types.

use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
use auki_geometry_rs as geometry;
use auki_registry_rs::{AxisConvention, FrameRegistryEntry};
use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyList, PyModule, PySequence};
use serde::de::DeserializeOwned;

// ─── GeometryError exception class ─────────────────────────────────

create_exception!(
    auki_geometry,
    GeometryError,
    pyo3::exceptions::PyValueError,
    "Auki geometry validation errors — invalid axes, handedness mismatch, or zero-length quaternion."
);

fn err_to_py(e: geometry::GeometryError) -> PyErr {
    GeometryError::new_err(e.to_string())
}

fn map_err<T>(r: geometry::Result<T>) -> PyResult<T> {
    r.map_err(err_to_py)
}

// ─── Flat-array math marshal helpers ───────────────────────────────

fn extract_floats(seq: &Bound<'_, PyAny>, expected_len: usize, name: &str) -> PyResult<Vec<f64>> {
    let sequence = seq
        .downcast::<PySequence>()
        .map_err(|_| PyValueError::new_err(format!("{name}: expected a sequence of floats")))?;
    let len = sequence.len()?;
    if len != expected_len {
        return Err(PyValueError::new_err(format!(
            "{name}: expected {expected_len} floats, got {len}"
        )));
    }
    let mut out = Vec::with_capacity(expected_len);
    for i in 0..expected_len {
        let item = sequence.get_item(i)?;
        let value: f64 = item.extract().map_err(|_| {
            PyValueError::new_err(format!("{name}[{i}]: expected a float"))
        })?;
        out.push(value);
    }
    Ok(out)
}

fn extract_vec3_array(any: &Bound<'_, PyAny>, name: &str) -> PyResult<Vec3> {
    let floats = extract_floats(any, 3, name)?;
    Ok(Vec3 { x: floats[0], y: floats[1], z: floats[2] })
}

fn extract_spatial_transform_array(any: &Bound<'_, PyAny>, name: &str) -> PyResult<SpatialTransform> {
    let floats = extract_floats(any, 7, name)?;
    Ok(SpatialTransform {
        translation: Some(Vec3 { x: floats[0], y: floats[1], z: floats[2] }),
        orientation: Some(Quat { x: floats[3], y: floats[4], z: floats[5], w: floats[6] }),
    })
}

fn vec3_to_pylist(py: Python<'_>, v: &Vec3) -> PyResult<PyObject> {
    Ok(PyList::new_bound(py, [v.x, v.y, v.z]).into())
}

fn spatial_transform_to_pylist(py: Python<'_>, t: &SpatialTransform) -> PyResult<PyObject> {
    // Output always carries both translation and orientation per the
    // Rust crate's contract; if either were `None` here it would be a
    // bug in the upstream crate.
    let translation = t.translation.clone().ok_or_else(|| {
        PyRuntimeError::new_err("internal: SpatialTransform output missing translation")
    })?;
    let orientation = t.orientation.clone().ok_or_else(|| {
        PyRuntimeError::new_err("internal: SpatialTransform output missing orientation")
    })?;
    Ok(PyList::new_bound(
        py,
        [
            translation.x, translation.y, translation.z,
            orientation.x, orientation.y, orientation.z, orientation.w,
        ],
    )
    .into())
}

// ─── Matrix marshal helpers ────────────────────────────────────────

fn matrix3_to_pylist(py: Python<'_>, m: geometry::Matrix3) -> PyResult<PyObject> {
    let rows: Vec<Bound<'_, PyList>> = m
        .iter()
        .map(|row| PyList::new_bound(py, row))
        .collect();
    Ok(PyList::new_bound(py, rows).into())
}

fn matrix4_to_pylist(py: Python<'_>, m: geometry::Matrix4) -> PyResult<PyObject> {
    let rows: Vec<Bound<'_, PyList>> = m
        .iter()
        .map(|row| PyList::new_bound(py, row))
        .collect();
    Ok(PyList::new_bound(py, rows).into())
}

fn extract_matrix4(any: &Bound<'_, PyAny>, name: &str) -> PyResult<geometry::Matrix4> {
    let outer = any
        .downcast::<PySequence>()
        .map_err(|_| PyValueError::new_err(format!("{name}: expected a 4x4 nested sequence")))?;
    let outer_len = outer.len()?;
    if outer_len != 4 {
        return Err(PyValueError::new_err(format!(
            "{name}: expected 4 rows, got {outer_len}"
        )));
    }
    let mut matrix = [[0.0_f64; 4]; 4];
    for row in 0..4 {
        let row_any = outer.get_item(row)?;
        let row_name = format!("{name}[{row}]");
        let row_floats = extract_floats(&row_any, 4, &row_name)?;
        matrix[row].copy_from_slice(&row_floats);
    }
    Ok(matrix)
}

// ─── Registry dict serde round-trip (mirrors auki-registry-py) ─────

fn py_to_json(py: Python<'_>, value: &Bound<'_, PyAny>, name: &str) -> PyResult<serde_json::Value> {
    let json = py.import_bound("json")?;
    let s: String = json.call_method1("dumps", (value,))?.extract()?;
    serde_json::from_str(&s).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

fn parse_py<T: DeserializeOwned>(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<T> {
    let json = py_to_json(py, value, name)?;
    serde_json::from_value(json).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

fn parse_frame_entry(py: Python<'_>, value: &Bound<'_, PyAny>, name: &str) -> PyResult<FrameRegistryEntry> {
    parse_py(py, value, name)
}

fn parse_axis_convention(py: Python<'_>, value: &Bound<'_, PyAny>, name: &str) -> PyResult<AxisConvention> {
    parse_py(py, value, name)
}

// ─── Module entry point (no pyfunctions yet) ───────────────────────

#[pymodule]
fn auki_geometry(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("GeometryError", py.get_type_bound::<GeometryError>())?;
    Ok(())
}
```

YAGNI'd out of this set vs the spec: `extract_quat_array`, `quat_to_pylist`, and a `struct_to_pyobject` writer — no callsite for them through Task 7, so they don't land. Add later if a future surface slice needs them.

- [ ] **Step 2: Confirm the binding still builds; warnings about unused private helpers are expected**

Run: `cargo check -p auki-geometry-py`
Expected: clean build. **Warnings about unused functions** (`extract_floats`, `extract_vec3_array`, `extract_spatial_transform_array`, `vec3_to_pylist`, `spatial_transform_to_pylist`, `matrix3_to_pylist`, `matrix4_to_pylist`, `extract_matrix4`, `parse_frame_entry`, `parse_axis_convention`, `py_to_json`, `parse_py`, `map_err`) are expected at this stage — the next four tasks consume them. Do not add `#[allow(dead_code)]` annotations; Task 8's clippy run verifies every helper is genuinely used by the end.

- [ ] **Step 3: Add a tiny Rust unit test for the exception class**

Append this to `src/lib.rs` (replace the existing module entry-point area is unnecessary; add a `#[cfg(test)]` block at the bottom of the file):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::Python;

    #[test]
    fn module_registers_geometry_error() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_geometry").unwrap();
            auki_geometry(py, &module).unwrap();
            let err_type = module.getattr("GeometryError").unwrap();
            // Confirm it's a class (a type object) and not None.
            assert!(err_type.is_instance_of::<pyo3::types::PyType>());
        });
    }
}
```

- [ ] **Step 4: Run the Rust-side smoke test**

Run: `cargo test -p auki-geometry-py`
Expected: `test module_registers_geometry_error ... ok`. The `auto-initialize` PyO3 dev-feature spins up a Python interpreter for the test.

- [ ] **Step 5: Commit**

```bash
git add bindings/python/auki-geometry-py/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(auki-geometry-py): marshal helpers and GeometryError class (#199)

why: every subsequent pyfunction will need the same flat-array
extraction / list-building infrastructure plus the GeometryError
exception type. Land the foundation in one task so each surface slice
that follows is a small focused diff.

how: extract_vec3/quat/spatial_transform_array helpers parse Sequence
inputs into prost SpatialTransform; vec3/quat/spatial_transform_to_pylist
helpers emit plain Python lists; matrix3/matrix4_to_pylist build nested
PyList; extract_matrix4 validates 4x4 nested sequences for the inverse
bridge; parse_frame_entry / parse_axis_convention reuse the
auki-registry-py json round-trip for the registry seam; GeometryError
declared via create_exception! mirroring the auki-network-py pattern.

what: src/lib.rs grows the marshal infrastructure and module
registration for GeometryError. No pyfunctions yet — each surface
slice lands in its own task.

xoxo Broodsugar's exocortex
EOF
)"
```

---

### Task 4: Scalars and convention matrices — `meters_per_unit`, `axis_convention_matrix`, `convention_matrix`

**Files:**
- Modify: `bindings/python/auki-geometry-py/src/lib.rs`
- Create: `bindings/python/auki-geometry-py/python_tests/test_geometry.py`

- [ ] **Step 1: Drop the anti-warning shim and add the three `#[pyfunction]`s**

In `src/lib.rs`, find the `#[pymodule]` block and replace its body with:

```rust
#[pymodule]
fn auki_geometry(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("GeometryError", py.get_type_bound::<GeometryError>())?;
    m.add_function(wrap_pyfunction!(meters_per_unit, m)?)?;
    m.add_function(wrap_pyfunction!(axis_convention_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(convention_matrix, m)?)?;
    Ok(())
}
```

Just above the `#[pymodule]` block, add the three pyfunctions:

```rust
// ─── Scalars and convention matrices ───────────────────────────────

#[pyfunction]
fn meters_per_unit(unit: &str) -> PyResult<f64> {
    let parsed: auki_registry_rs::LengthUnit = serde_json::from_value(
        serde_json::Value::String(unit.to_string()),
    )
    .map_err(|e| PyValueError::new_err(format!("unit: {e}")))?;
    Ok(geometry::meters_per_unit(parsed))
}

#[pyfunction]
fn axis_convention_matrix(
    py: Python<'_>,
    from_axes: &Bound<'_, PyAny>,
    to_axes: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let from = parse_axis_convention(py, from_axes, "from_axes")?;
    let to = parse_axis_convention(py, to_axes, "to_axes")?;
    let matrix = map_err(geometry::axis_convention_matrix(&from, &to))?;
    matrix3_to_pylist(py, matrix)
}

#[pyfunction]
fn convention_matrix(
    py: Python<'_>,
    from_entry: &Bound<'_, PyAny>,
    to_entry: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let from = parse_frame_entry(py, from_entry, "from_entry")?;
    let to = parse_frame_entry(py, to_entry, "to_entry")?;
    let matrix = map_err(geometry::convention_matrix(&from, &to))?;
    matrix4_to_pylist(py, matrix)
}
```

Add the `wrap_pyfunction!` import: at the top of `src/lib.rs`, change `use pyo3::prelude::*;` to ensure `wrap_pyfunction` is imported. (`pyo3::prelude::*;` already re-exports it.)

Also: remove the temporary `let _ = (...);` anti-warning shim from inside the `#[pymodule]` body — the helpers it referenced are now used by the three new pyfunctions (or will be by the next tasks).

- [ ] **Step 2: Write the first three pytest tests**

Create `bindings/python/auki-geometry-py/python_tests/test_geometry.py`:

```python
"""Smoke tests for the `auki_geometry` Python module.

Run after building the wheel:

    maturin develop -m bindings/python/auki-geometry-py/Cargo.toml
    pytest bindings/python/auki-geometry-py/python_tests/
"""

from __future__ import annotations

import pytest


def test_meters_per_unit_locked_values() -> None:
    import auki_geometry

    assert auki_geometry.meters_per_unit("meters") == 1.0
    assert auki_geometry.meters_per_unit("centimeters") == 0.01
    assert auki_geometry.meters_per_unit("millimeters") == 0.001


def test_meters_per_unit_rejects_unknown_unit() -> None:
    import auki_geometry

    with pytest.raises(ValueError):
        auki_geometry.meters_per_unit("furlongs")


def test_axis_convention_matrix_ros_optical_to_opengl() -> None:
    import auki_geometry

    ros_optical = {"x": "right", "y": "down", "z": "forward"}
    opengl = {"x": "right", "y": "up", "z": "backward"}
    assert auki_geometry.axis_convention_matrix(ros_optical, opengl) == [
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, -1.0],
    ]


def test_convention_matrix_round_trips_to_identity() -> None:
    import auki_geometry
    import auki_registry

    presets = [
        auki_registry.frame_ros_body("body"),
        auki_registry.frame_ros_optical("optical"),
        auki_registry.frame_opengl("opengl"),
        auki_registry.frame_unity("unity"),
    ]

    def matmul4(a: list[list[float]], b: list[list[float]]) -> list[list[float]]:
        return [
            [sum(a[r][k] * b[k][c] for k in range(4)) for c in range(4)]
            for r in range(4)
        ]

    identity4 = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]

    for a in presets:
        for b in presets:
            ab = auki_geometry.convention_matrix(a, b)
            ba = auki_geometry.convention_matrix(b, a)
            product = matmul4(ba, ab)
            for r in range(4):
                for c in range(4):
                    assert abs(product[r][c] - identity4[r][c]) < 1e-9
```

- [ ] **Step 3: Build the wheel and run the tests**

Run: `maturin develop -m bindings/python/auki-geometry-py/Cargo.toml --release` (or omit `--release` for faster iteration)
Expected: builds cleanly; produces `auki_geometry` module in the current Python environment.

Run: `pytest bindings/python/auki-geometry-py/python_tests/ -v`
Expected: all four tests pass.

If `auki_registry` is not yet installed in the current Python environment, install it first:
`maturin develop -m bindings/python/auki-registry-py/Cargo.toml`

- [ ] **Step 4: Commit**

```bash
git add bindings/python/auki-geometry-py/src/lib.rs bindings/python/auki-geometry-py/python_tests/test_geometry.py
git commit -m "$(cat <<'EOF'
feat(auki-geometry-py): meters_per_unit and convention matrices (#199)

why: first slice of the public surface — the simplest pyfunctions land
together to validate the marshal + module-registration infrastructure
from the previous task end-to-end.

how: meters_per_unit takes a string ("meters"/"centimeters"/"millimeters")
and round-trips through serde to the registry LengthUnit enum;
axis_convention_matrix and convention_matrix parse axis/frame dicts via
the registry-py json round-trip and return nested-list matrices.

what: three new #[pyfunction]s on the auki_geometry module, plus four
pytest cases including the locked ROS-optical-to-OpenGL permutation and
the four-preset convention round-trip.

xoxo Broodsugar's exocortex
EOF
)"
```

---

### Task 5: Convention conversions — `convert_point_convention`, `convert_vector_convention`, `convert_direction_convention`, `convert_pose_convention`

**Files:**
- Modify: `bindings/python/auki-geometry-py/src/lib.rs`
- Modify: `bindings/python/auki-geometry-py/python_tests/test_geometry.py`

- [ ] **Step 1: Add the four `#[pyfunction]`s**

In `src/lib.rs`, add these four pyfunctions immediately after `convention_matrix`:

```rust
// ─── Convention conversions ─────────────────────────────────────────

#[pyfunction]
fn convert_point_convention(
    py: Python<'_>,
    point: &Bound<'_, PyAny>,
    from_entry: &Bound<'_, PyAny>,
    to_entry: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let point = extract_vec3_array(point, "point")?;
    let from = parse_frame_entry(py, from_entry, "from_entry")?;
    let to = parse_frame_entry(py, to_entry, "to_entry")?;
    let converted = map_err(geometry::convert_point_convention(point, &from, &to))?;
    vec3_to_pylist(py, &converted)
}

#[pyfunction]
fn convert_vector_convention(
    py: Python<'_>,
    vector: &Bound<'_, PyAny>,
    from_entry: &Bound<'_, PyAny>,
    to_entry: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let vector = extract_vec3_array(vector, "vector")?;
    let from = parse_frame_entry(py, from_entry, "from_entry")?;
    let to = parse_frame_entry(py, to_entry, "to_entry")?;
    let converted = map_err(geometry::convert_vector_convention(vector, &from, &to))?;
    vec3_to_pylist(py, &converted)
}

#[pyfunction]
fn convert_direction_convention(
    py: Python<'_>,
    direction: &Bound<'_, PyAny>,
    from_entry: &Bound<'_, PyAny>,
    to_entry: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let direction = extract_vec3_array(direction, "direction")?;
    let from = parse_frame_entry(py, from_entry, "from_entry")?;
    let to = parse_frame_entry(py, to_entry, "to_entry")?;
    let converted = map_err(geometry::convert_direction_convention(direction, &from, &to))?;
    vec3_to_pylist(py, &converted)
}

#[pyfunction]
fn convert_pose_convention(
    py: Python<'_>,
    pose: &Bound<'_, PyAny>,
    from_entry: &Bound<'_, PyAny>,
    to_entry: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let pose = extract_spatial_transform_array(pose, "pose")?;
    let from = parse_frame_entry(py, from_entry, "from_entry")?;
    let to = parse_frame_entry(py, to_entry, "to_entry")?;
    let converted = map_err(geometry::convert_pose_convention(&pose, &from, &to))?;
    spatial_transform_to_pylist(py, &converted)
}
```

Update the `#[pymodule]` block to register them:

```rust
#[pymodule]
fn auki_geometry(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("GeometryError", py.get_type_bound::<GeometryError>())?;
    m.add_function(wrap_pyfunction!(meters_per_unit, m)?)?;
    m.add_function(wrap_pyfunction!(axis_convention_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(convention_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(convert_point_convention, m)?)?;
    m.add_function(wrap_pyfunction!(convert_vector_convention, m)?)?;
    m.add_function(wrap_pyfunction!(convert_direction_convention, m)?)?;
    m.add_function(wrap_pyfunction!(convert_pose_convention, m)?)?;
    Ok(())
}
```

- [ ] **Step 2: Add the matching pytest cases**

Append to `bindings/python/auki-geometry-py/python_tests/test_geometry.py`:

```python
def test_convert_point_convention_applies_axes_and_units() -> None:
    import auki_geometry
    import auki_registry

    source = {
        "frame_id": "source",
        "handedness": "right",
        "axes": {"x": "right", "y": "down", "z": "forward"},
        "units": "centimeters",
    }
    target = auki_registry.frame_opengl("target")
    converted = auki_geometry.convert_point_convention([100.0, 200.0, 300.0], source, target)
    assert converted == pytest.approx([1.0, -2.0, -3.0])


def test_convert_direction_convention_skips_unit_scale() -> None:
    import auki_geometry
    import auki_registry

    source = {
        "frame_id": "source",
        "handedness": "right",
        "axes": {"x": "right", "y": "down", "z": "forward"},
        "units": "centimeters",
    }
    target = auki_registry.frame_opengl("target")
    converted = auki_geometry.convert_direction_convention([1.0, 2.0, 3.0], source, target)
    assert converted == pytest.approx([1.0, -2.0, -3.0])


def test_convert_pose_convention_translates_and_rotates() -> None:
    import math

    import auki_geometry
    import auki_registry

    half = 1.0 / math.sqrt(2)
    pose = [1.0, 2.0, 3.0, 0.0, 0.0, half, half]
    from_entry = auki_registry.frame_ros_optical("camera")
    to_entry = auki_registry.frame_opengl("world")

    converted = auki_geometry.convert_pose_convention(pose, from_entry, to_entry)

    # Translation: ROS-optical (x=right, y=down, z=forward) in meters →
    # OpenGL (x=right, y=up, z=backward) in meters. Same axis flips as
    # convert_point_convention without unit scale.
    assert converted[0] == pytest.approx(1.0)
    assert converted[1] == pytest.approx(-2.0)
    assert converted[2] == pytest.approx(-3.0)

    # Orientation should be a unit quaternion.
    qx, qy, qz, qw = converted[3:]
    assert abs(qx * qx + qy * qy + qz * qz + qw * qw - 1.0) < 1e-9


def test_convert_pose_convention_rejects_short_array() -> None:
    import auki_geometry
    import auki_registry

    pose = [1.0, 2.0, 3.0, 0.0, 0.0, 0.0]  # 6 elements, not 7
    with pytest.raises(ValueError, match="pose: expected 7 floats"):
        auki_geometry.convert_pose_convention(
            pose,
            auki_registry.frame_ros_optical("camera"),
            auki_registry.frame_opengl("world"),
        )
```

- [ ] **Step 3: Build and run**

Run: `maturin develop -m bindings/python/auki-geometry-py/Cargo.toml`
Expected: clean build.

Run: `pytest bindings/python/auki-geometry-py/python_tests/ -v`
Expected: all eight tests pass (four from Task 4 + four new).

- [ ] **Step 4: Commit**

```bash
git add bindings/python/auki-geometry-py/src/lib.rs bindings/python/auki-geometry-py/python_tests/test_geometry.py
git commit -m "$(cat <<'EOF'
feat(auki-geometry-py): point/vector/direction/pose convention helpers (#199)

why: cover the four convention-conversion entry points so Python
producers can re-express points and poses across declared frame
conventions on equal footing with Rust callers.

how: each pyfunction extracts the math input as a flat array (3-list
for points/vectors/directions, 7-list for poses), parses the
FrameRegistryEntry dicts via the registry-py json round-trip, delegates
to the corresponding auki-geometry function, and emits a plain Python
list. Length mismatches surface as ValueError before the Rust call.

what: four #[pyfunction]s on the auki_geometry module, plus four pytest
cases (axis+unit applied, axis-only direction, full pose translate+
rotate, 6-element array rejection).

xoxo Broodsugar's exocortex
EOF
)"
```

---

### Task 6: Spatial transform composition — `inverse_spatial_transform`, `compose_spatial_transforms`, `relative_spatial_transform`

**Files:**
- Modify: `bindings/python/auki-geometry-py/src/lib.rs`
- Modify: `bindings/python/auki-geometry-py/python_tests/test_geometry.py`

- [ ] **Step 1: Add the three composition pyfunctions**

In `src/lib.rs`, add immediately after `convert_pose_convention`:

```rust
// ─── Spatial transform composition (PR #193) ────────────────────────

#[pyfunction]
fn inverse_spatial_transform(
    py: Python<'_>,
    transform: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let t = extract_spatial_transform_array(transform, "transform")?;
    let inverse = map_err(geometry::inverse_spatial_transform(&t))?;
    spatial_transform_to_pylist(py, &inverse)
}

#[pyfunction]
fn compose_spatial_transforms(
    py: Python<'_>,
    from_to_mid: &Bound<'_, PyAny>,
    mid_to_to: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let a = extract_spatial_transform_array(from_to_mid, "from_to_mid")?;
    let b = extract_spatial_transform_array(mid_to_to, "mid_to_to")?;
    let composed = map_err(geometry::compose_spatial_transforms(&a, &b))?;
    spatial_transform_to_pylist(py, &composed)
}

#[pyfunction]
fn relative_spatial_transform(
    py: Python<'_>,
    common_to_from: &Bound<'_, PyAny>,
    common_to_to: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let a = extract_spatial_transform_array(common_to_from, "common_to_from")?;
    let b = extract_spatial_transform_array(common_to_to, "common_to_to")?;
    let relative = map_err(geometry::relative_spatial_transform(&a, &b))?;
    spatial_transform_to_pylist(py, &relative)
}
```

Update the `#[pymodule]` block to register them:

```rust
    m.add_function(wrap_pyfunction!(inverse_spatial_transform, m)?)?;
    m.add_function(wrap_pyfunction!(compose_spatial_transforms, m)?)?;
    m.add_function(wrap_pyfunction!(relative_spatial_transform, m)?)?;
```

(Place these after the `convert_*` registrations and before the `Ok(())`.)

- [ ] **Step 2: Add the pytest cases**

Append to `python_tests/test_geometry.py`:

```python
def test_inverse_then_compose_yields_identity() -> None:
    import math

    import auki_geometry

    half = 1.0 / math.sqrt(2)
    pose = [1.0, 2.0, 3.0, 0.0, 0.0, half, half]

    inverse = auki_geometry.inverse_spatial_transform(pose)
    composed = auki_geometry.compose_spatial_transforms(pose, inverse)

    # Identity: zero translation, identity quaternion (sign may flip).
    for i in range(3):
        assert composed[i] == pytest.approx(0.0, abs=1e-9)
    qx, qy, qz, qw = composed[3:]
    # Identity quaternion is (0, 0, 0, ±1).
    assert abs(qx) < 1e-9 and abs(qy) < 1e-9 and abs(qz) < 1e-9
    assert abs(abs(qw) - 1.0) < 1e-9


def test_compose_spatial_transforms_order() -> None:
    """Compose (A → B) with (B → C); apply to origin → translation of A → C."""
    import math

    import auki_geometry

    half = 1.0 / math.sqrt(2)
    # A → B: rotate 90° around +Z, no translation.
    a_to_b = [0.0, 0.0, 0.0, 0.0, 0.0, half, half]
    # B → C: translate +1 along x (post-rotation), no rotation.
    b_to_c = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]

    a_to_c = auki_geometry.compose_spatial_transforms(a_to_b, b_to_c)

    # Origin of A is (0, 0, 0). Apply A → B (rotation only): origin stays.
    # Apply B → C: now translation (1, 0, 0) in C. So composed translation = (1, 0, 0).
    assert a_to_c[0] == pytest.approx(1.0)
    assert a_to_c[1] == pytest.approx(0.0)
    assert a_to_c[2] == pytest.approx(0.0)


def test_relative_spatial_transform_derives_from_to() -> None:
    """Given common→A and common→B, derive A→B."""
    import auki_geometry

    # common → A: pure translation along +x.
    common_to_a = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]
    # common → B: pure translation along +y.
    common_to_b = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]

    # A → B = inverse(common → A) ∘ (common → B) = translate (-1, 1, 0).
    a_to_b = auki_geometry.relative_spatial_transform(common_to_a, common_to_b)
    assert a_to_b[0] == pytest.approx(-1.0)
    assert a_to_b[1] == pytest.approx(1.0)
    assert a_to_b[2] == pytest.approx(0.0)


def test_inverse_spatial_transform_rejects_zero_quaternion() -> None:
    import auki_geometry

    # Zero quaternion is invalid; Rust's normalize_quat surfaces ZeroQuaternion.
    pose = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    with pytest.raises(auki_geometry.GeometryError):
        auki_geometry.inverse_spatial_transform(pose)


def test_geometry_error_is_value_error_subclass() -> None:
    import auki_geometry

    assert issubclass(auki_geometry.GeometryError, ValueError)

    # Catchable as ValueError too.
    with pytest.raises(ValueError):
        auki_geometry.inverse_spatial_transform([0.0] * 7)
```

- [ ] **Step 3: Build and run**

Run: `maturin develop -m bindings/python/auki-geometry-py/Cargo.toml`
Expected: clean build.

Run: `pytest bindings/python/auki-geometry-py/python_tests/ -v`
Expected: all thirteen tests pass (eight prior + five new).

- [ ] **Step 4: Commit**

```bash
git add bindings/python/auki-geometry-py/src/lib.rs bindings/python/auki-geometry-py/python_tests/test_geometry.py
git commit -m "$(cat <<'EOF'
feat(auki-geometry-py): spatial transform composition (#199)

why: expose the PR #193 helpers (inverse, compose, relative) so Python
consumers can derive direct frame-to-frame transforms without
re-implementing the math.

how: each pyfunction extracts SpatialTransform inputs as 7-element flat
arrays, delegates to the corresponding auki-geometry function, and
emits a plain 7-element Python list. GeometryError propagates from the
Rust crate for zero-quaternion / handedness / invalid-axes failures.

what: three new #[pyfunction]s on the auki_geometry module, plus five
pytest cases (inverse-then-compose identity, compose order check,
relative same-origin derivation, zero-quaternion rejection,
GeometryError ⊂ ValueError catchability).

xoxo Broodsugar's exocortex
EOF
)"
```

---

### Task 7: 4×4 ↔ 7-array bridge — `spatial_transform_to_matrix4`, `spatial_transform_from_matrix4`

**Files:**
- Modify: `bindings/python/auki-geometry-py/src/lib.rs`
- Modify: `bindings/python/auki-geometry-py/python_tests/test_geometry.py`

- [ ] **Step 1: Add the two bridge pyfunctions**

In `src/lib.rs`, add immediately after `relative_spatial_transform`:

```rust
// ─── 4x4 ↔ 7-array bridge ──────────────────────────────────────────

#[pyfunction]
fn spatial_transform_to_matrix4(
    py: Python<'_>,
    pose: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let t = extract_spatial_transform_array(pose, "pose")?;
    let matrix = map_err(geometry::spatial_transform_to_matrix4(&t))?;
    matrix4_to_pylist(py, matrix)
}

#[pyfunction]
fn spatial_transform_from_matrix4(
    py: Python<'_>,
    matrix: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let m = extract_matrix4(matrix, "matrix")?;
    let transform = map_err(geometry::spatial_transform_from_matrix4(m))?;
    spatial_transform_to_pylist(py, &transform)
}
```

Update the `#[pymodule]` block:

```rust
    m.add_function(wrap_pyfunction!(spatial_transform_to_matrix4, m)?)?;
    m.add_function(wrap_pyfunction!(spatial_transform_from_matrix4, m)?)?;
```

(After the composition registrations, before `Ok(())`.)

- [ ] **Step 2: Add the pytest cases**

Append to `python_tests/test_geometry.py`:

```python
def test_spatial_transform_to_matrix4_identity() -> None:
    import auki_geometry

    identity = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]
    matrix = auki_geometry.spatial_transform_to_matrix4(identity)
    assert matrix == [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]


def test_spatial_transform_matrix4_round_trip() -> None:
    import math

    import auki_geometry

    half = 1.0 / math.sqrt(2)
    poses = [
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, half, 0.0, 0.0, half],
        [0.0, 0.0, 0.0, 0.0, half, 0.0, half],
        [1.0, 2.0, 3.0, 0.0, 0.0, half, half],
    ]
    for original in poses:
        matrix = auki_geometry.spatial_transform_to_matrix4(original)
        decoded = auki_geometry.spatial_transform_from_matrix4(matrix)
        # Translation round-trips exactly.
        for i in range(3):
            assert decoded[i] == pytest.approx(original[i])
        # Quaternion can equal ±original_quaternion (Hamilton sign).
        same = all(abs(decoded[3 + i] - original[3 + i]) < 1e-9 for i in range(4))
        negated = all(abs(decoded[3 + i] + original[3 + i]) < 1e-9 for i in range(4))
        assert same or negated, f"pose {original} did not round-trip: {decoded}"


def test_spatial_transform_from_matrix4_rejects_wrong_shape() -> None:
    import auki_geometry

    # 3x3 instead of 4x4.
    with pytest.raises(ValueError, match="matrix: expected 4 rows"):
        auki_geometry.spatial_transform_from_matrix4([
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ])

    # 4 rows but inner row only 3 wide.
    with pytest.raises(ValueError, match="matrix\\[0\\]: expected 4 floats"):
        auki_geometry.spatial_transform_from_matrix4([
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
```

- [ ] **Step 3: Build and run**

Run: `maturin develop -m bindings/python/auki-geometry-py/Cargo.toml`
Expected: clean build.

Run: `pytest bindings/python/auki-geometry-py/python_tests/ -v`
Expected: all sixteen tests pass (thirteen prior + three new).

- [ ] **Step 4: Commit**

```bash
git add bindings/python/auki-geometry-py/src/lib.rs bindings/python/auki-geometry-py/python_tests/test_geometry.py
git commit -m "$(cat <<'EOF'
feat(auki-geometry-py): 4x4 transformation matrix bridge (#199)

why: robotics consumers work with both flat [x,y,z,qx,qy,qz,qw] arrays
and 4x4 homogeneous matrices; the bridge lets them convert at the seam
between the canonical 7-array surface and matrix-multiply workflows.

how: spatial_transform_to_matrix4 builds a 4x4 nested list via the new
Rust crate helper; spatial_transform_from_matrix4 validates the input
is a 4-row-by-4-col Sequence[Sequence[float]] and delegates the rotation
extraction to the Rust crate's matrix_to_quat path.

what: two new #[pyfunction]s on the auki_geometry module, plus three
pytest cases (identity emission, 5-pose round-trip with sign-aware
quaternion comparison, malformed-input rejection).

xoxo Broodsugar's exocortex
EOF
)"
```

---

### Task 8: Final end-to-end verification

**Files:**
- (No source changes — verification only)

- [ ] **Step 1: Run the full Cargo test suite**

Run: `cargo test -p auki-geometry -p auki-geometry-py`
Expected: all Rust unit tests pass (the six new ones in `auki-geometry`, the module-registration smoke test in `auki-geometry-py`).

- [ ] **Step 2: Run the full workspace check**

Run: `cargo check --workspace`
Expected: no errors anywhere.

- [ ] **Step 3: Run clippy on the binding**

Run: `cargo clippy -p auki-geometry-py -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Rebuild the wheel and run all pytest cases**

Run: `maturin develop -m bindings/python/auki-geometry-py/Cargo.toml`
Run: `pytest bindings/python/auki-geometry-py/python_tests/ -v`
Expected: all sixteen tests pass.

- [ ] **Step 5: Confirm the module exposes exactly the expected surface**

Drop into a Python REPL (in the maturin-develop environment):

```python
import auki_geometry

expected = {
    "GeometryError",
    "meters_per_unit",
    "axis_convention_matrix",
    "convention_matrix",
    "convert_point_convention",
    "convert_vector_convention",
    "convert_direction_convention",
    "convert_pose_convention",
    "inverse_spatial_transform",
    "compose_spatial_transforms",
    "relative_spatial_transform",
    "spatial_transform_to_matrix4",
    "spatial_transform_from_matrix4",
}
actual = {name for name in dir(auki_geometry) if not name.startswith("_")}
missing = expected - actual
extra = actual - expected
print("missing:", missing)
print("extra:", extra)
```

Expected output:
```
missing: set()
extra: set()
```

If either set is non-empty, investigate before proceeding. (Some Python build environments expose extra magic attributes via PyO3 — `__loader__`, `__spec__`, etc.; only flag genuine pyfunction/class mismatches.)

- [ ] **Step 6: No commit needed — verification only**

If everything passes, the implementation is complete.

---

### Task 9: Open the PR

**Files:**
- (No source changes)

- [ ] **Step 1: Rebase onto latest develop**

Run: `git fetch origin && git rebase origin/develop`
Expected: clean rebase or trivial conflicts only. If non-trivial conflicts arise, stop and surface them to the developer.

- [ ] **Step 2: Push the branch**

Run: `git push -u origin feat/199-auki-geometry-py`

- [ ] **Step 3: Open the PR with `Closes #199`**

Run:

```bash
gh pr create --repo aukilabs/auki-sdk --base develop \
  --title "feat: PyO3 bindings for auki-geometry — convention helpers + spatial transform composition" \
  --body "$(cat <<'EOF'
## Summary

- Adds two public helpers to `auki-geometry` (`spatial_transform_to_matrix4` / `spatial_transform_from_matrix4`) so the binding can offer a 4×4 ↔ 7-array bridge without duplicating the math (the existing `quat_to_matrix` / `matrix_to_quat` are private).
- Ships `bindings/python/auki-geometry-py` with twelve `#[pyfunction]`s mirroring the full `auki-geometry` public surface plus the new bridge helpers; one custom `GeometryError` exception class that subclasses `ValueError`.
- Math types cross the Python seam as flat float arrays (`Vec3` → `[x,y,z]`, `Quat` → `[qx,qy,qz,qw]` scalar-last, `SpatialTransform` → 7-list); categorical types stay as dicts/strings matching `auki-registry-py`. This was a deliberate course correction after robotics-team feedback on the initial dict-shape draft — see the design doc for the rationale.
- Sixteen pytest cases covering every pyfunction + `GeometryError` propagation + array-length-mismatch rejection + 4×4 round-trip; six new Rust unit tests on the helpers.

## Spec

`docs/superpowers/specs/2026-05-22-auki-geometry-py-design.md` (committed on this branch).

## Test plan

- [ ] `cargo test -p auki-geometry -p auki-geometry-py` passes
- [ ] `cargo clippy -p auki-geometry-py -- -D warnings` is clean
- [ ] `maturin develop -m bindings/python/auki-geometry-py/Cargo.toml` builds
- [ ] `pytest bindings/python/auki-geometry-py/python_tests/ -v` passes (16/16)
- [ ] `dir(auki_geometry)` exposes exactly the twelve pyfunctions + `GeometryError`

Closes #199

xoxo Broodsugar's exocortex
EOF
)"
```

Capture the PR URL from the command output.

- [ ] **Step 4: Move card #199 from In progress → In review**

```bash
gh project item-edit \
  --id PVTI_lADOBOE4-s4BYdTfzgtixo8 \
  --project-id PVT_kwDOBOE4-s4BYdTf \
  --field-id PVTSSF_lADOBOE4-s4BYdTfzhTjDIM \
  --single-select-option-id df73e18b
```

The `Closes #199` line in the PR body auto-closes the issue on merge; the card moves itself to **Done** via project automation at that point. If it doesn't move automatically post-merge, move it manually:

```bash
gh project item-edit \
  --id PVTI_lADOBOE4-s4BYdTfzgtixo8 \
  --project-id PVT_kwDOBOE4-s4BYdTf \
  --field-id PVTSSF_lADOBOE4-s4BYdTfzhTjDIM \
  --single-select-option-id 98236657
```

- [ ] **Step 5: Surface the PR URL to the developer**

Print the PR URL captured in Step 3 and stop. The developer reviews / merges from here.

---

## Self-Review Notes

**Spec coverage:**

| Spec section | Covered by |
|---|---|
| Goal — full public API surface | Tasks 4–7 (10 original pyfunctions + 2 bridge helpers) |
| Two new Rust public helpers | Task 1 |
| Flat-array shapes (Vec3 / Quat / SpatialTransform) | Task 3 marshal helpers; consumed in Tasks 4–7 |
| Matrix nested-list shapes | Task 3 marshal helpers; consumed in Tasks 4, 7 |
| FrameRegistryEntry / AxisConvention serde round-trip | Task 3 |
| `GeometryError` exception class as `ValueError` subclass | Task 3 (class declaration); Task 6 (subclass check) |
| Marshaling-error vs GeometryError distinction | Task 5 (length-mismatch ValueError), Task 6 (zero-quaternion GeometryError) |
| Test coverage (every function + every error variant + 4×4 round-trip) | Task 4 (4 tests) + Task 5 (4 tests) + Task 6 (5 tests) + Task 7 (3 tests) = 16 |
| Cargo.toml workspace registration | Task 2 |
| README mirroring registry-py | Task 2 |
| No NumPy dependency | None — never added |
| No `pose_to_dict` / `pose_from_dict` | None — explicitly out of scope per spec |

**Placeholder scan:** No "TBD", "TODO", "implement later", or other placeholders. Every step has either exact code or an exact command.

**Type consistency:** Function names cross-checked between `src/lib.rs` and `crates/auki-geometry/src/lib.rs`. Marshal helper signatures consistent across all consumer tasks. The `extract_floats(... &str)` `name` parameter is used identically in every call site. The `parse_frame_entry` / `parse_axis_convention` signature takes `(py, value, name)` consistently.

**Scope check:** Single PR, ~600 lines of new code across two crates (Rust crate +~50 lines, binding ~300 lines lib.rs + ~250 lines tests + manifests + README). Properly task-decomposed: each task produces working, committable software.

**Exception detail:** The plan uses `pyo3::create_exception!` with four arguments (module, name, base, docstring). PyO3 0.22 supports this form. If `create_exception!` is invoked with only three args in a sibling binding, this plan's four-arg form is still valid — the docstring arg is optional.
