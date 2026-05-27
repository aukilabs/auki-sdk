//! Log handles returned by Session::register_*_log.

use auki_registry::LogRef;

pub struct SensorLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
}

impl SensorLogHandle {
    pub fn resource_id(&self) -> &str { &self.resource_id }
    pub fn log_ref(&self) -> &LogRef { &self.log_ref }
}

pub struct PoseLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
}

impl PoseLogHandle {
    pub fn resource_id(&self) -> &str { &self.resource_id }
    pub fn log_ref(&self) -> &LogRef { &self.log_ref }
}

pub struct TimeTransformLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
}

impl TimeTransformLogHandle {
    pub fn resource_id(&self) -> &str { &self.resource_id }
    pub fn log_ref(&self) -> &LogRef { &self.log_ref }
}

pub struct DetectionLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
}

impl DetectionLogHandle {
    pub fn resource_id(&self) -> &str { &self.resource_id }
    pub fn log_ref(&self) -> &LogRef { &self.log_ref }
}

pub struct MaterializedLogHandle {
    pub log_ref: LogRef,
}
