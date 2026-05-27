//! Log handles returned by Session::register_*_log.

use auki_registry::LogRef;

pub struct SensorLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
}

pub struct PoseLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
}

pub struct TimeTransformLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
}

pub struct DetectionLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
}

pub struct MaterializedLogHandle {
    pub log_ref: LogRef,
}
