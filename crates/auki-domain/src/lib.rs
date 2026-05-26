//! Cluster lifecycle for the Auki SDK.
//!
//! The stable Rust API is re-exported from [`core`]. Native UniFFI exposes a
//! bounded cluster-manager facade plus cluster membership helpers. Browser
//! WebAssembly exposes only web-safe membership JSON and election helpers.

#![warn(missing_docs)]

#[cfg(all(target_arch = "wasm32", feature = "uniffi"))]
compile_error!(
    "auki-domain does not support UniFFI on wasm32; build wasm with --no-default-features --features wasm"
);

pub mod core;
pub use core::*;

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
mod ffi;

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub use ffi::UniFfiTag;

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub use ffi::{
    BindingDomainError, BindingRegistryEntryProvider, BindingResourceCatalogProvider,
    BindingSensorCatalogProvider, ClusterTargetMode, DomainClusterManager, DomainRuntimeEvent,
    DomainStreamEntry, DomainStreamSubscription, bootstrap_domain_cluster_manager,
};

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod wasm;
