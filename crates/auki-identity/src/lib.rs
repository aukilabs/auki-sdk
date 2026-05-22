//! Wallet primitive for the Auki SDK.
//!
//! The stable Rust API is re-exported from [`core`]. Native UniFFI and
//! JavaScript/WebAssembly bindings live in private adapter modules so Rust
//! workspace crates can keep depending on the direct Rust surface.

#[cfg(all(target_arch = "wasm32", feature = "uniffi"))]
compile_error!(
    "auki-identity does not support UniFFI on wasm32; build wasm with --no-default-features --features wasm"
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
