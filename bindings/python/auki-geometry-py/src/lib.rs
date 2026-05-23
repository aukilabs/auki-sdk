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
