//! Sensor, Clock, Frame, and Detector Registry entries with
//! content-addressed on-disk storage.
//!
//! The stable Rust API is re-exported from [`core`]. Native UniFFI and
//! JavaScript/WebAssembly bindings expose JSON-string adapters over the same
//! binding-free implementation.

#[cfg(all(target_arch = "wasm32", feature = "uniffi"))]
compile_error!(
    "auki-registry does not support UniFFI on wasm32; build wasm with --no-default-features --features wasm"
);

pub mod core;
pub use core::*;

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
mod ffi;

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub use ffi::UniFfiTag;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod wasm;
