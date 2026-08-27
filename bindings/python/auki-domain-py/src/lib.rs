//! Python binding for one authenticated, Domain-owned Auki P2P node.
//!
//! The binding intentionally exposes no centralized topology, election,
//! relay-booking, or shared-clock control plane.

// PyO3 0.22 generates calls to unsafe helpers inside its generated unsafe
// functions. Rust 2024 warns about those macro expansions until the binding
// migrates to a newer PyO3 release.
#![allow(unsafe_op_in_unsafe_fn)]
// PyO3 0.22's generated wrappers contain same-type `.into()` calls.
#![allow(clippy::useless_conversion)]

mod domain;
mod providers;
mod session_bridge;
mod streams;
#[cfg(feature = "test-support")]
mod test_support;
mod values;

use pyo3::prelude::*;

pub(crate) fn runtime_error(error: impl std::fmt::Display) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(error.to_string())
}

#[pymodule]
fn auki_domain(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    values::register(module)?;
    streams::register(module)?;
    domain::register(module)?;
    #[cfg(feature = "test-support")]
    test_support::register(module)?;
    Ok(())
}
