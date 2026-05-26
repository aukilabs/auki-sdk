//! Generic segmented ring-buffer log primitive.
//!
//! The stable Rust API is re-exported from [`core`]. Native UniFFI bindings
//! expose an opaque-bytes log adapter for Python and Swift. JavaScript/WASM
//! bindings expose pure manifest and segment-byte helpers without filesystem
//! I/O.

#[cfg(all(target_arch = "wasm32", feature = "uniffi"))]
compile_error!(
    "auki-logs does not support UniFFI on wasm32; build wasm with --no-default-features --features wasm"
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
