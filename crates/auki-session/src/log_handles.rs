//! Log handles returned by Session::register_*_log.

use auki_manifests::{
    DetectionLogManifest, PoseLogManifest, PoseWriterMode, SensorLogManifest,
    TimeTransformLogManifest,
};
use auki_registry::LogRef;
use std::path::{Path, PathBuf};

use crate::log_specs::HeadSpec;

pub struct SensorLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest. Public so `auki-domain` can build catalog rows; the
    /// sensor kind/type are derived from the peer's sensor registry at
    /// catalog-build time, not stored here. See #274 (D3).
    pub manifest: SensorLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    pub(crate) root: PathBuf,
}

impl SensorLogHandle {
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub fn log_ref(&self) -> &LogRef {
        &self.log_ref
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub struct PoseLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub manifest: PoseLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    /// Writer mode (derived from spec).
    pub writer_mode: PoseWriterMode,
    pub(crate) root: PathBuf,
}

impl PoseLogHandle {
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub fn log_ref(&self) -> &LogRef {
        &self.log_ref
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub struct TimeTransformLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub manifest: TimeTransformLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    pub(crate) root: PathBuf,
}

impl TimeTransformLogHandle {
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub fn log_ref(&self) -> &LogRef {
        &self.log_ref
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub struct DetectionLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub manifest: DetectionLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    pub(crate) root: PathBuf,
}

impl DetectionLogHandle {
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub fn log_ref(&self) -> &LogRef {
        &self.log_ref
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub struct MaterializedLogHandle {
    pub log_ref: LogRef,
}
