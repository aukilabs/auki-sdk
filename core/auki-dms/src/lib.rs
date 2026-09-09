//! DMS task types and opt-in polling for native hosts.
//!
//! The default `client` feature enables HTTP/auth and polling helpers on native
//! targets. Disable default features for types and paths without those dependencies.
//! Constructing a client starts no requests, polling, or task execution.

pub mod types;
pub use types::{LeaseEnvelope, TaskSpec};

mod paths;
pub use paths::DmsPaths;

#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
pub mod client;
#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
pub mod poller;
