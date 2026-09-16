//! DMS job submission, task types, and opt-in worker polling for native hosts.
//!
//! The default `client` feature enables HTTP/auth and polling helpers on native
//! targets. Disable default features for types and paths without those dependencies.
//! The separate `jobs` feature enables peer-free HTTP job operations on native
//! and WASM targets using a shared User/App authentication session.
//! Constructing a client starts no requests, polling, or task execution.

pub mod types;
pub use types::{LeaseEnvelope, TaskSpec};

mod paths;
pub use paths::DmsPaths;

/// Peer-free job submission and monitoring with shared User/App sessions.
#[cfg(feature = "jobs")]
pub mod jobs;

#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
pub mod client;
#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
pub mod poller;
