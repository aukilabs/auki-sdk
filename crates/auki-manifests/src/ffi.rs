use std::time::Duration;

use crate::core;

uniffi::setup_scaffolding!();

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum PoseSource {
    Ros2Tf { publishers: Vec<String> },
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseWriterMode {
    Rigid,
    Movable,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeTransformSource {
    LocalClockRead,
}

#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn build_sensor_log_manifest_json(
    app_id: String,
    session_id: String,
    sensor_id: String,
    sensor_hash: String,
    clock_id: String,
    clock_hash: String,
    frame_id: Option<String>,
    frame_hash: Option<String>,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> String {
    manifest_json(core::build_sensor_log_manifest(
        &app_id,
        &session_id,
        &sensor_id,
        &sensor_hash,
        &clock_id,
        &clock_hash,
        frame_id.as_deref(),
        frame_hash.as_deref(),
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    ))
}

#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn build_pose_log_manifest_json(
    app_id: String,
    session_id: String,
    from_frame_id: String,
    from_frame_hash: String,
    to_frame_id: String,
    to_frame_hash: String,
    clock_id: String,
    clock_hash: String,
    source: PoseSource,
    writer_mode: PoseWriterMode,
    expected_rate_hz: u32,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> String {
    let source = core::PoseSource::from(source);
    manifest_json(core::build_pose_log_manifest(
        &app_id,
        &session_id,
        &from_frame_id,
        &from_frame_hash,
        &to_frame_id,
        &to_frame_hash,
        &clock_id,
        &clock_hash,
        &source,
        writer_mode.into(),
        expected_rate_hz,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    ))
}

#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn build_time_transform_log_manifest_json(
    app_id: String,
    session_id: String,
    from_clock_id: String,
    from_clock_hash: String,
    to_clock_id: String,
    to_clock_hash: String,
    source: TimeTransformSource,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> String {
    let source = core::TimeTransformSource::from(source);
    manifest_json(core::build_time_transform_log_manifest(
        &app_id,
        &session_id,
        &from_clock_id,
        &from_clock_hash,
        &to_clock_id,
        &to_clock_hash,
        &source,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    ))
}

#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn build_detection_log_manifest_json(
    app_id: String,
    session_id: String,
    detector_id: String,
    detector_hash: String,
    input_log_id: String,
    input_sensor_id: String,
    input_sensor_hash: String,
    clock_id: String,
    clock_hash: String,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> String {
    manifest_json(core::build_detection_log_manifest(
        &app_id,
        &session_id,
        &detector_id,
        &detector_hash,
        &input_log_id,
        &input_sensor_id,
        &input_sensor_hash,
        &clock_id,
        &clock_hash,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    ))
}

#[uniffi::export]
pub fn pose_source_canonical_json(source: PoseSource) -> String {
    utf8(core::PoseSource::from(source).canonical_bytes())
}

#[uniffi::export]
pub fn pose_source_hash(source: PoseSource) -> String {
    core::PoseSource::from(source).hash()
}

#[uniffi::export]
pub fn time_transform_source_canonical_json(source: TimeTransformSource) -> String {
    utf8(core::TimeTransformSource::from(source).canonical_bytes())
}

#[uniffi::export]
pub fn time_transform_source_hash(source: TimeTransformSource) -> String {
    core::TimeTransformSource::from(source).hash()
}

fn manifest_json(value: serde_json::Value) -> String {
    utf8(auki_jcs::canonicalize(&value))
}

fn utf8(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("JCS output is valid UTF-8")
}

impl From<PoseSource> for core::PoseSource {
    fn from(source: PoseSource) -> Self {
        match source {
            PoseSource::Ros2Tf { publishers } => Self::Ros2Tf { publishers },
        }
    }
}

impl From<PoseWriterMode> for core::PoseWriterMode {
    fn from(mode: PoseWriterMode) -> Self {
        match mode {
            PoseWriterMode::Rigid => Self::Rigid,
            PoseWriterMode::Movable => Self::Movable,
        }
    }
}

impl From<TimeTransformSource> for core::TimeTransformSource {
    fn from(source: TimeTransformSource) -> Self {
        match source {
            TimeTransformSource::LocalClockRead => Self::LocalClockRead,
        }
    }
}
