//! Session — per-process declarative API for the Auki SDK.
//!
//! Apps construct a [`Session`], register their sensors / clocks / frames /
//! detectors and the logs they own, then join a domain to advertise them.
//! Spec: `docs/superpowers/specs/2026-05-27-216-schema-and-api-placement-design.md` §4.

#![deny(unused_must_use)]

mod error;
mod registry_store;
mod log_specs;
mod log_handles;
mod session;
mod materialization;

pub use error::{Result, SessionError};
pub use log_specs::{HeadSpec, SensorLogSpec, PoseLogSpec, TimeTransformLogSpec, DetectionLogSpec};
pub use log_handles::{SensorLogHandle, PoseLogHandle, TimeTransformLogHandle, DetectionLogHandle, MaterializedLogHandle};
pub use session::{Session, FrameDef, DomainConfig};
pub use registry_store::RegistryStore;
pub use materialization::MaterializationError;

// Convenience re-exports so callers only need `auki_session::` imports.
pub use auki_domain::{ClusterTarget, DaemonInfo, ClusterManager};
pub use auki_network::PeerIdentity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_publishes_session_type() {
        let _: Option<Session> = None;
    }
}
