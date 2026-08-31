//! Network-free recording and registry model for the Auki SDK.
//!
//! Apps construct a long-lived [`Peer`], register local metadata, and create
//! [`Session`] timelines that own their logs. Networking composes this model
//! separately through
//! `auki_protocols::session_adapter::SessionProtocolProvider`.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_publishes_session_type() {
        let _: Option<Session> = None;
    }
}
