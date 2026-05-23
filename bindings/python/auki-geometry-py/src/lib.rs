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
