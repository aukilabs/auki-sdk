//! Small binding-generation proving crate.

#[cfg(all(target_arch = "wasm32", feature = "uniffi"))]
compile_error!(
    "auki-uniffi-test does not support UniFFI on wasm32; build wasm with --no-default-features --features wasm"
);

pub mod core;

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
mod ffi;

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
pub use ffi::*;

#[cfg(feature = "wasm")]
mod wasm;

#[cfg(all(feature = "wasm", not(feature = "uniffi")))]
pub use wasm::*;
