//! PyO3 bindings for [`auki-raster`](../../../auki-raster).
//!
//! `cross_section(mesh, height, ppm)` accepts an `auki_geometry.TriangleMesh`
//! (duck-typed: `vertices` / `indices`).

#![allow(unsafe_op_in_unsafe_fn)]
#![allow(unexpected_cfgs)]
#![allow(clippy::useless_conversion)]

use auki_geometry::{MeshGroup, TriangleMesh};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyDict, PyModule};

fn py_to_triangle_mesh(obj: &Bound<'_, PyAny>) -> PyResult<TriangleMesh> {
    let mut verts = Vec::new();
    for item in obj.getattr("vertices")?.iter()? {
        let p = item?;
        if let Ok(seq) = p.extract::<Vec<f32>>() {
            if seq.len() != 3 {
                return Err(PyValueError::new_err("point sequence must have 3 floats"));
            }
            verts.push(auki_geometry::mesh::Vec3::new(seq[0], seq[1], seq[2]));
        } else {
            verts.push(auki_geometry::mesh::Vec3::new(
                p.get_item("x")?.extract()?,
                p.get_item("y")?.extract()?,
                p.get_item("z")?.extract()?,
            ));
        }
    }
    let mut tris = Vec::new();
    for item in obj.getattr("indices")?.iter()? {
        let face: Vec<u32> = item?.extract()?;
        if face.len() != 3 {
            return Err(PyValueError::new_err("each triangle needs 3 indices"));
        }
        tris.push([face[0], face[1], face[2]]);
    }
    let mut groups = Vec::new();
    if let Ok(glist) = obj.getattr("groups") {
        if !glist.is_none() {
            for item in glist.iter()? {
                let g = item?;
                groups.push(MeshGroup {
                    name: g.get_item("name")?.extract()?,
                    triangle_range: {
                        let start: usize = g.get_item("start")?.extract()?;
                        let end: usize = g.get_item("end")?.extract()?;
                        start..end
                    },
                });
            }
        }
    }
    TriangleMesh::from_indexed(verts, tris, groups)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction]
#[pyo3(signature = (mesh, height, ppm))]
fn cross_section(
    py: Python<'_>,
    mesh: &Bound<'_, PyAny>,
    height: f32,
    ppm: f32,
) -> PyResult<PyObject> {
    let mesh = py_to_triangle_mesh(mesh)?;
    let cs = py.allow_threads(|| auki_raster_rs::cross_section(&mesh, height, ppm));
    let d = PyDict::new_bound(py);
    d.set_item("png", PyBytes::new_bound(py, &cs.png))?;
    d.set_item("map_yaml", cs.map_yaml)?;
    d.set_item("width", cs.width)?;
    d.set_item("height", cs.height)?;
    d.set_item("resolution", cs.resolution)?;
    d.set_item("origin_x", cs.origin_x)?;
    d.set_item("origin_z", cs.origin_z)?;
    Ok(d.into())
}

#[pymodule]
fn auki_raster(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(cross_section, m)?)?;
    Ok(())
}
