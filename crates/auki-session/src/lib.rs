//! Session — per-process declarative API for the Auki SDK.
//!
//! Apps construct a [`Session`], register their sensors / clocks / frames /
//! detectors, and own the resulting local logs. Networking composes this
//! model separately; see the repository's `dataproducts.md` reference.

#![deny(unused_must_use)]

mod camera_detector_package;
mod detector_runner;
mod error;
mod log_handles;
mod log_specs;
mod materialization;
mod peer;
mod registry_store;
mod session;

pub use auki_manifests::DetectionCadence;
pub use camera_detector_package::{
    CameraDetectorPackageError, CameraDetectorTask, CameraStreamDescriptor,
    RegisteredCameraDetector, StreamingCameraDetectorTask,
};
pub use detector_runner::{
    CameraDetector, CameraFrameHub, CameraFrameSample, CameraInputBinding, DetectorOutput,
    DetectorRunnerError, DetectorTask, StreamingDetectorTask,
};
pub use error::{Result, SessionError};
pub use log_handles::{
    DetectionLogHandle, MapLogHandle, MaterializedLogHandle, PoseLogHandle, SensorLogHandle,
    TimeTransformLogHandle,
};
pub use log_specs::{
    DetectionLogSpec, DetectorInstanceSpec, HeadSpec, MapLogSpec, PoseLogSpec, SensorLogSpec,
    TimeTransformLogSpec,
};
pub use materialization::MaterializationError;
pub use peer::{FrameDef, Peer, PeerRegistries};
pub use registry_store::RegistryStore;
pub use session::{Session, SessionLogs};

/// Versioned private PyCapsule name used to pass an `Arc<Peer>` between the
/// paired `auki-session-py` and `auki-domain-py` wheels.
///
/// This is not a stable application API. The exact name intentionally fences
/// native extensions built from different SDK releases before either side
/// reads the Rust payload.
#[doc(hidden)]
pub const PY_DOMAIN_PEER_CAPSULE_ABI_NAME: &str = "auki_sdk.python.peer.arc.sdk-0.1.0.p13.v1";

/// Versioned private PyCapsule name used to pass an `Arc<Session>` between the
/// paired `auki-session-py` and `auki-domain-py` wheels.
///
/// This is not a stable application API. The exact name intentionally fences
/// native extensions built from different SDK releases before either side
/// reads the Rust payload.
#[doc(hidden)]
pub const PY_DOMAIN_SESSION_CAPSULE_ABI_NAME: &str = "auki_sdk.python.session.arc.sdk-0.1.0.p13.v1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_publishes_session_type() {
        let _: Option<Session> = None;
    }
}
