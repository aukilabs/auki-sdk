//! Declarative log registration specs.

use std::time::Duration;
use auki_registry::{RegistryRef, LogRef};
use auki_manifests::{PoseSource, PoseWriterMode, TimeTransformSource};

#[derive(Debug, Clone)]
pub enum HeadSpec {
    Rolling { retention_ns: i64 },
    Fixed,
}

#[derive(Debug, Clone)]
pub struct SensorLogSpec {
    pub sensor: RegistryRef,
    pub clock: RegistryRef,
    pub frame: Option<RegistryRef>,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}

#[derive(Debug, Clone)]
pub struct PoseLogSpec {
    pub from_frame: RegistryRef,
    pub to_frame: RegistryRef,
    pub clock: RegistryRef,
    pub source: PoseSource,
    pub writer_mode: PoseWriterMode,
    pub expected_rate_hz: u32,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}

#[derive(Debug, Clone)]
pub struct TimeTransformLogSpec {
    pub from_clock: RegistryRef,
    pub to_clock: RegistryRef,
    pub source: TimeTransformSource,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}

#[derive(Debug, Clone)]
pub struct DetectionLogSpec {
    pub detector: RegistryRef,
    pub input_log: LogRef,
    pub input_sensor: RegistryRef,
    pub clock: RegistryRef,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}
