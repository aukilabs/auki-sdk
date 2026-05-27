//! `auki-domain-py` is deprecated as of #216.
//!
//! The declarative Session API lives in `auki-session`. A Python binding
//! for it (`auki-session-py`) is a follow-up card. This crate is preserved
//! as an empty extension module so the workspace continues to compile;
//! downstream Python code should migrate off it.

use pyo3::prelude::*;

#[pymodule]
fn auki_domain(_m: &Bound<'_, PyModule>) -> PyResult<()> {
    Ok(())
}
