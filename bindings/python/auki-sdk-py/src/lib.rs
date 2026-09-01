//! Mechanical Python facade for the native Rust `AukiPeer` runtime.

// PyO3 0.22 macro expansions trigger these Rust 2024 and Clippy lints.
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::useless_conversion)]

pub mod cleanup;
mod facade;

use pyo3::prelude::*;

pub use facade::PyAukiPeer;

/// Register the generic facade in a larger, same-module protocol extension.
pub fn register_facade(module: &Bound<'_, PyModule>) -> PyResult<()> {
    facade::register(module)
}

#[pymodule]
fn auki_sdk(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    register_facade(module)
}
