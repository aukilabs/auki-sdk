//! Declarative log registration specs.

use auki_manifests::{DetectionCadence, PoseSource, PoseWriterMode, TimeTransformSource};
use auki_registry::{LogRef, RegistryRef};
use std::time::Duration;

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
    pub instance_id: String,
    pub detector: RegistryRef,
    pub input_log: LogRef,
    pub input_sensor: RegistryRef,
    pub clock: RegistryRef,
    pub cadence: DetectionCadence,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}

/// Application-facing settings for one running detector instance.
///
/// Detector identity, input provenance, and clock are derived by the detector
/// adapter from its registration and selected Sensor Log rather than repeated
/// by the application.
#[derive(Debug, Clone)]
pub struct DetectorInstanceSpec {
    pub instance_id: String,
    pub cadence: DetectionCadence,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}

impl DetectorInstanceSpec {
    pub fn rolling(
        instance_id: impl Into<String>,
        cadence: DetectionCadence,
        retention: Duration,
        segment_duration: Duration,
    ) -> Self {
        Self {
            instance_id: instance_id.into(),
            cadence,
            head: HeadSpec::Rolling {
                retention_ns: retention.as_nanos().min(i64::MAX as u128) as i64,
            },
            segment_duration,
            retention,
        }
    }
}
