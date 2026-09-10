//! Mechanical Python facade for the native Rust `AukiPeer` runtime.

// PyO3 0.22 macro expansions trigger these Rust 2024 and Clippy lints.
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::useless_conversion)]

pub mod cleanup;
mod facade;
#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob",
    feature = "message",
    feature = "stream"
))]
mod protocols;

use pyo3::prelude::*;

pub use facade::PyAukiPeer;

/// Register the generic facade in a larger, same-module protocol extension.
pub fn register_facade(module: &Bound<'_, PyModule>) -> PyResult<()> {
    facade::register(module)
}

/// Register every protocol role enabled by this crate's Cargo features.
#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob",
    feature = "message",
    feature = "stream"
))]
pub fn register_protocols(module: &Bound<'_, PyModule>) -> PyResult<()> {
    protocols::register(module)
}

/// Register the peer facade and every enabled standard protocol role.
pub fn register_sdk(module: &Bound<'_, PyModule>) -> PyResult<()> {
    register_facade(module)?;
    #[cfg(any(
        feature = "info",
        feature = "catalog",
        feature = "registry",
        feature = "blob",
        feature = "message",
        feature = "stream"
    ))]
    register_protocols(module)?;
    Ok(())
}

#[pymodule]
fn auki_sdk(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    register_sdk(module)
}
