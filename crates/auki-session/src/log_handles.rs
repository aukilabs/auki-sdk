//! Log handles returned by Session::register_*_log.

use auki_manifests::{
    SensorLogManifest, PoseLogManifest, TimeTransformLogManifest, DetectionLogManifest,
    PoseWriterMode,
};
use auki_network::resources_protocol::SensorKind;
use auki_registry::LogRef;

use crate::log_specs::HeadSpec;

pub struct SensorLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub(crate) manifest: SensorLogManifest,
    /// Head window spec, used for catalog row production.
    pub(crate) head_spec: HeadSpec,
    /// Closed sensor family (derived from manifest.sensor body).
    pub(crate) sensor_kind: SensorKind,
    /// Open-string sensor type (derived from manifest.sensor body).
    pub(crate) sensor_type: String,
}

impl SensorLogHandle {
    pub fn resource_id(&self) -> &str { &self.resource_id }
    pub fn log_ref(&self) -> &LogRef { &self.log_ref }
}

pub struct PoseLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub(crate) manifest: PoseLogManifest,
    /// Head window spec, used for catalog row production.
    pub(crate) head_spec: HeadSpec,
    /// Writer mode (derived from spec).
    pub(crate) writer_mode: PoseWriterMode,
}

impl PoseLogHandle {
    pub fn resource_id(&self) -> &str { &self.resource_id }
    pub fn log_ref(&self) -> &LogRef { &self.log_ref }
}

pub struct TimeTransformLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub(crate) manifest: TimeTransformLogManifest,
    /// Head window spec, used for catalog row production.
    pub(crate) head_spec: HeadSpec,
}

impl TimeTransformLogHandle {
    pub fn resource_id(&self) -> &str { &self.resource_id }
    pub fn log_ref(&self) -> &LogRef { &self.log_ref }
}

pub struct DetectionLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub(crate) manifest: DetectionLogManifest,
    /// Head window spec, used for catalog row production.
    pub(crate) head_spec: HeadSpec,
}

impl DetectionLogHandle {
    pub fn resource_id(&self) -> &str { &self.resource_id }
    pub fn log_ref(&self) -> &LogRef { &self.log_ref }
}

pub struct MaterializedLogHandle {
    pub log_ref: LogRef,
}
