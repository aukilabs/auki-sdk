use std::time::Duration;

use crate::core;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = buildSensorLogManifestJson)]
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

#[wasm_bindgen(js_name = buildPoseLogManifestJson)]
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
    source_publishers: Vec<String>,
    writer_mode: String,
    expected_rate_hz: u32,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> Result<String, JsValue> {
    let source = core::PoseSource::Ros2Tf {
        publishers: source_publishers,
    };
    Ok(manifest_json(core::build_pose_log_manifest(
        &app_id,
        &session_id,
        &from_frame_id,
        &from_frame_hash,
        &to_frame_id,
        &to_frame_hash,
        &clock_id,
        &clock_hash,
        &source,
        parse_pose_writer_mode(&writer_mode)?,
        expected_rate_hz,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    )))
}

#[wasm_bindgen(js_name = buildTimeTransformLogManifestJson)]
#[allow(clippy::too_many_arguments)]
pub fn build_time_transform_log_manifest_json(
    app_id: String,
    session_id: String,
    from_clock_id: String,
    from_clock_hash: String,
    to_clock_id: String,
    to_clock_hash: String,
    source_kind: String,
    segment_duration_ns: u64,
    retention_ns: u64,
) -> Result<String, JsValue> {
    let source = parse_time_transform_source(&source_kind)?;
    Ok(manifest_json(core::build_time_transform_log_manifest(
        &app_id,
        &session_id,
        &from_clock_id,
        &from_clock_hash,
        &to_clock_id,
        &to_clock_hash,
        &source,
        Duration::from_nanos(segment_duration_ns),
        Duration::from_nanos(retention_ns),
    )))
}

#[wasm_bindgen(js_name = buildDetectionLogManifestJson)]
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

#[wasm_bindgen(js_name = poseSourceRos2TfCanonicalJson)]
pub fn pose_source_ros2_tf_canonical_json(publishers: Vec<String>) -> String {
    utf8(core::PoseSource::Ros2Tf { publishers }.canonical_bytes())
}

#[wasm_bindgen(js_name = poseSourceRos2TfHash)]
pub fn pose_source_ros2_tf_hash(publishers: Vec<String>) -> String {
    core::PoseSource::Ros2Tf { publishers }.hash()
}

#[wasm_bindgen(js_name = timeTransformSourceCanonicalJson)]
pub fn time_transform_source_canonical_json(source_kind: String) -> Result<String, JsValue> {
    Ok(utf8(
        parse_time_transform_source(&source_kind)?.canonical_bytes(),
    ))
}

#[wasm_bindgen(js_name = timeTransformSourceHash)]
pub fn time_transform_source_hash(source_kind: String) -> Result<String, JsValue> {
    Ok(parse_time_transform_source(&source_kind)?.hash())
}

fn manifest_json(value: serde_json::Value) -> String {
    utf8(auki_jcs::canonicalize(&value))
}

fn utf8(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("JCS output is valid UTF-8")
}

fn parse_pose_writer_mode(mode: &str) -> Result<core::PoseWriterMode, JsValue> {
    match mode {
        "rigid" => Ok(core::PoseWriterMode::Rigid),
        "movable" => Ok(core::PoseWriterMode::Movable),
        _ => Err(JsValue::from_str(
            "writer_mode must be either 'rigid' or 'movable'",
        )),
    }
}

fn parse_time_transform_source(kind: &str) -> Result<core::TimeTransformSource, JsValue> {
    match kind {
        "local_clock_read" => Ok(core::TimeTransformSource::LocalClockRead),
        _ => Err(JsValue::from_str("source kind must be 'local_clock_read'")),
    }
}
