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
    m.add_function(wrap_pyfunction!(inverse_spatial_transform, m)?)?;
    m.add_function(wrap_pyfunction!(compose_spatial_transforms, m)?)?;
    m.add_function(wrap_pyfunction!(relative_spatial_transform, m)?)?;
    m.add_function(wrap_pyfunction!(spatial_transform_to_matrix4, m)?)?;
    m.add_function(wrap_pyfunction!(spatial_transform_from_matrix4, m)?)?;
    Ok(())
}

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
