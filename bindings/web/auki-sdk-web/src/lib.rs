//! Mechanical browser facade for authenticated, relay-backed Auki peers.
//!
//! The browser runtime is available only on `wasm32-unknown-unknown`. Portable
//! relay snapshot validation remains compiled and tested on native targets.

#[cfg(any(test, target_arch = "wasm32"))]
mod booking;
mod config;

pub use config::{AukiWebPeerConfig, AukiWebPeerConfigError, DEV_DMS_BASE_URL};

#[cfg(target_arch = "wasm32")]
mod runtime;
#[cfg(target_arch = "wasm32")]
pub use runtime::{
    AukiWebPeer, AukiWebPeerError, AukiWebPeerExit, AukiWebReachability, AukiWebRoute,
};
