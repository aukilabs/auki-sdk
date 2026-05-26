//! TimeTransform math, NTP-style samples, session clocks, and the 1 Hz
//! `local_clock_read` sampler.
//!
//! The stable Rust API is re-exported from [`core`]. Native UniFFI bindings
//! expose records/objects over the same binding-free implementation.
//! JavaScript/WebAssembly bindings expose the web-safe math and composition
//! surface only; host clocks, sampler threads, and filesystem logs remain
//! native Rust APIs.

#[cfg(all(target_arch = "wasm32", feature = "uniffi"))]
compile_error!(
    "auki-time does not support UniFFI on wasm32; build wasm with --no-default-features --features wasm"
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
