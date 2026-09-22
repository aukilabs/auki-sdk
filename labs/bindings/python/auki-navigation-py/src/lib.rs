//! PyO3 bindings for [`auki-navigation`](../../../auki-navigation).
//!
//! Mesh ingest lives in `auki_geometry`. Compute takes that mesh.
//! Occupancy slice is `auki_raster`.
//!
//! Points cross as `{x,y,z}` dicts. Paths expose a `full` list for
//! robot-runner-kit `auki_domain_navigation.DomainNavigation` compatibility.

#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::useless_conversion)]

use auki_navigation_rs as nav;
use auki_geometry::{MeshGroup, TriangleMesh};
use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};

create_exception!(
    auki_navigation,
    NavError,
    pyo3::exceptions::PyValueError,
    "auki-navigation bake / path errors."
);

fn map_err(e: nav::NavError) -> PyErr {
    NavError::new_err(e.to_string())
}

fn map_geom(e: auki_geometry::GeometryError) -> PyErr {
    NavError::new_err(e.to_string())
}

/// Accept `auki_geometry.TriangleMesh` (or any object with vertices/indices/groups).
fn py_to_triangle_mesh(obj: &Bound<'_, PyAny>) -> PyResult<TriangleMesh> {
    let mut verts = Vec::new();
    for item in obj.getattr("vertices")?.iter()? {
        let p = py_to_vec3(&item?)?;
        verts.push(auki_geometry::mesh::Vec3::new(p.x, p.y, p.z));
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
                let name: String = g.get_item("name")?.extract()?;
                let start: usize = g.get_item("start")?.extract()?;
                let end: usize = g.get_item("end")?.extract()?;
                groups.push(MeshGroup {
                    name,
                    triangle_range: start..end,
                });
            }
        }
    }
    TriangleMesh::from_indexed(verts, tris, groups).map_err(map_geom)
}

fn vec3_to_py(py: Python<'_>, v: nav::Vec3) -> PyResult<PyObject> {
    let d = PyDict::new_bound(py);
    d.set_item("x", v.x)?;
    d.set_item("y", v.y)?;
    d.set_item("z", v.z)?;
    Ok(d.into())
}

fn py_to_vec3(obj: &Bound<'_, PyAny>) -> PyResult<nav::Vec3> {
    if let Ok(seq) = obj.extract::<Vec<f32>>() {
        if seq.len() != 3 {
            return Err(PyValueError::new_err("point sequence must have 3 floats"));
        }
        return Ok(nav::Vec3::new(seq[0], seq[1], seq[2]));
    }
    Ok(nav::Vec3::new(
        obj.get_item("x")?.extract()?,
        obj.get_item("y")?.extract()?,
        obj.get_item("z")?.extract()?,
    ))
}

fn on_mesh_to_py(py: Python<'_>, p: &nav::OnMeshPoint) -> PyResult<PyObject> {
    let d = PyDict::new_bound(py);
    d.set_item("original", vec3_to_py(py, p.original)?)?;
    d.set_item("adjusted", vec3_to_py(py, p.adjusted)?)?;
    d.set_item("is_off_mesh", p.is_off_mesh)?;
    Ok(d.into())
}

fn path_to_py(py: Python<'_>, path: &nav::PathResult) -> PyResult<PyObject> {
    let d = PyDict::new_bound(py);
    let full = PyList::empty_bound(py);
    for p in &path.full {
        full.append(vec3_to_py(py, *p)?)?;
    }
    d.set_item("full", full)?;
    d.set_item("total_distance", path.total_distance)?;

    let wps = PyList::empty_bound(py);
    for p in &path.waypoints {
        wps.append(on_mesh_to_py(py, p)?)?;
    }
    d.set_item("waypoints", wps)?;

    let segs = PyList::empty_bound(py);
    for s in &path.segments {
        let sd = PyDict::new_bound(py);
        sd.set_item("start", on_mesh_to_py(py, &s.start)?)?;
        sd.set_item("end", on_mesh_to_py(py, &s.end)?)?;
        sd.set_item("distance", s.distance)?;
        let pts = PyList::empty_bound(py);
        for p in &s.path {
            pts.append(vec3_to_py(py, *p)?)?;
        }
        sd.set_item("path", pts)?;
        segs.append(sd)?;
    }
    d.set_item("segments", segs)?;
    Ok(d.into())
}

fn py_to_waypoints(obj: &Bound<'_, PyAny>) -> PyResult<Vec<nav::Vec3>> {
    let mut out = Vec::new();
    for item in obj.iter()? {
        out.push(py_to_vec3(&item?)?);
    }
    Ok(out)
}

fn py_to_extents(obj: Option<&Bound<'_, PyAny>>) -> PyResult<nav::Extents> {
    let Some(obj) = obj else {
        return Ok(nav::Extents::default());
    };
    if obj.is_none() {
        return Ok(nav::Extents::default());
    }
    Ok(nav::Extents {
        x: obj.get_item("x").ok().and_then(|v| v.extract().ok()).unwrap_or(100.0),
        y: obj.get_item("y").ok().and_then(|v| v.extract().ok()).unwrap_or(10.0),
        z: obj.get_item("z").ok().and_then(|v| v.extract().ok()).unwrap_or(100.0),
    })
}

fn opt_f32(obj: &Bound<'_, PyAny>, key: &str) -> Option<f32> {
    obj.get_item(key)
        .ok()
        .and_then(|v| if v.is_none() { None } else { v.extract().ok() })
}

fn py_to_segment_opts(obj: Option<&Bound<'_, PyAny>>) -> PyResult<nav::SegmentOpts> {
    let mut opts = nav::SegmentOpts::default();
    let Some(obj) = obj else {
        return Ok(opts);
    };
    if obj.is_none() {
        return Ok(opts);
    }
    if let Ok(start) = obj.get_item("start") {
        if !start.is_none() {
            opts.start = Some(py_to_vec3(&start)?);
        }
    }
    if let Ok(end) = obj.get_item("end") {
        if !end.is_none() {
            opts.end = Some(py_to_vec3(&end)?);
        }
    }
    if let Some(v) = opt_f32(obj, "interval") {
        opts.interval = v;
    }
    if let Ok(fixed_end) = obj.get_item("fixed_end") {
        if !fixed_end.is_none() {
            opts.fixed_end = fixed_end.extract()?;
        }
    }
    if let Ok(area) = obj.get_item("area") {
        if !area.is_none() {
            opts.area = Some(nav::AreaFilter {
                x_min: opt_f32(&area, "x_min"),
                x_max: opt_f32(&area, "x_max"),
                z_min: opt_f32(&area, "z_min"),
                z_max: opt_f32(&area, "z_max"),
            });
        }
    }
    Ok(opts)
}

#[pyclass(name = "BakeProfile")]
#[derive(Clone)]
struct PyBakeProfile {
    inner: nav::BakeProfile,
}

#[pymethods]
impl PyBakeProfile {
    #[staticmethod]
    fn gotu() -> Self {
        Self {
            inner: nav::BakeProfile::gotu(),
        }
    }

    /// DSC cells (cs=0.05, ch=0.01). `agent_radius` is **metres** → voxels via ceil.
    #[staticmethod]
    fn dsc(agent_radius: f32) -> Self {
        Self {
            inner: nav::BakeProfile::dsc(agent_radius),
        }
    }

    #[staticmethod]
    fn robot_r40() -> Self {
        Self {
            inner: nav::BakeProfile::robot_r40(),
        }
    }

    #[staticmethod]
    fn restrict_bake() -> Self {
        Self {
            inner: nav::BakeProfile::restrict_bake(),
        }
    }

    #[getter]
    fn walkable_radius(&self) -> f32 {
        self.inner.walkable_radius
    }
}

#[pyclass(name = "NavMesh")]
struct PyNavMesh {
    inner: nav::NavMesh,
}

#[pymethods]
impl PyNavMesh {
    #[staticmethod]
    fn bake(py: Python<'_>, mesh: &Bound<'_, PyAny>, profile: &PyBakeProfile) -> PyResult<Self> {
        let mesh = py_to_triangle_mesh(mesh)?;
        let profile = profile.inner.clone();
        let inner = py
            .allow_threads(|| nav::NavMesh::bake(&mesh, &profile))
            .map_err(map_err)?;
        Ok(Self { inner })
    }

    #[pyo3(signature = (point, extents=None))]
    fn find_closest_point(
        &self,
        py: Python<'_>,
        point: &Bound<'_, PyAny>,
        extents: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyObject> {
        let p = py_to_vec3(point)?;
        let extents = py_to_extents(extents)?;
        on_mesh_to_py(py, &self.inner.find_closest_point(p, extents))
    }

    fn find_path(&self, py: Python<'_>, waypoints: &Bound<'_, PyAny>) -> PyResult<PyObject> {
        let wps = py_to_waypoints(waypoints)?;
        let path = py
            .allow_threads(|| self.inner.find_path(&wps))
            .map_err(map_err)?;
        path_to_py(py, &path)
    }

    #[pyo3(signature = (waypoints, fixed_end=false))]
    fn find_optimized_path(
        &self,
        py: Python<'_>,
        waypoints: &Bound<'_, PyAny>,
        fixed_end: bool,
    ) -> PyResult<PyObject> {
        let wps = py_to_waypoints(waypoints)?;
        let result = py
            .allow_threads(|| self.inner.find_optimized_path(&wps, fixed_end))
            .map_err(map_err)?;
        let d = PyDict::new_bound(py);
        let path_obj = path_to_py(py, &result.path)?;
        let path_d = path_obj.bind(py);
        d.set_item("full", path_d.get_item("full")?)?;
        d.set_item("total_distance", path_d.get_item("total_distance")?)?;
        d.set_item("waypoints", path_d.get_item("waypoints")?)?;
        d.set_item("segments", path_d.get_item("segments")?)?;
        d.set_item("path", path_obj)?;
        d.set_item("waypoint_indices", result.waypoint_indices.clone())?;
        Ok(d.into())
    }

    #[pyo3(signature = (target, min_separation=0.5))]
    fn restrict(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        min_separation: f32,
    ) -> PyResult<PyObject> {
        let t = py_to_vec3(target)?;
        let r = self.inner.restrict(t, min_separation);
        let d = PyDict::new_bound(py);
        d.set_item("original", vec3_to_py(py, r.original)?)?;
        d.set_item("restricted", vec3_to_py(py, r.restricted)?)?;
        d.set_item("direction", vec3_to_py(py, r.direction)?)?;
        Ok(d.into())
    }

    #[pyo3(signature = (mesh, opts=None))]
    fn find_optimized_segment_path(
        &self,
        py: Python<'_>,
        mesh: &Bound<'_, PyAny>,
        opts: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyObject> {
        let mesh = py_to_triangle_mesh(mesh)?;
        let opts = py_to_segment_opts(opts)?;
        let path = py
            .allow_threads(|| self.inner.find_optimized_segment_path(&mesh, opts))
            .map_err(map_err)?;
        path_to_py(py, &path)
    }

    #[getter]
    fn agent_radius(&self) -> f32 {
        self.inner.agent_radius()
    }
}

#[pymodule]
fn auki_navigation(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("NavError", py.get_type_bound::<NavError>())?;
    m.add_class::<PyBakeProfile>()?;
    m.add_class::<PyNavMesh>()?;
    Ok(())
}
