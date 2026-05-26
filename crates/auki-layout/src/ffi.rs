use std::path::{Path, PathBuf};

use crate::core;

uniffi::setup_scaffolding!();

fn path_string(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[uniffi::export]
pub fn registries_root(app_root: String) -> String {
    path_string(core::registries_root(Path::new(&app_root)))
}

#[uniffi::export]
pub fn sensor_entry_path(app_root: String, sensor_id: String, hash: String) -> String {
    path_string(core::sensor_entry_path(
        Path::new(&app_root),
        &sensor_id,
        &hash,
    ))
}

#[uniffi::export]
pub fn clock_entry_path(app_root: String, clock_id: String, hash: String) -> String {
    path_string(core::clock_entry_path(
        Path::new(&app_root),
        &clock_id,
        &hash,
    ))
}

#[uniffi::export]
pub fn frame_entry_path(app_root: String, frame_id: String, hash: String) -> String {
    path_string(core::frame_entry_path(
        Path::new(&app_root),
        &frame_id,
        &hash,
    ))
}

#[uniffi::export]
pub fn detector_entry_path(app_root: String, detector_id: String, hash: String) -> String {
    path_string(core::detector_entry_path(
        Path::new(&app_root),
        &detector_id,
        &hash,
    ))
}

#[uniffi::export]
pub fn session_root(app_root: String, session: String) -> String {
    path_string(core::session_root(Path::new(&app_root), &session))
}

#[uniffi::export]
pub fn timetransform_log_path(session_root: String, from_id: String, to_id: String) -> String {
    path_string(core::timetransform_log_path(
        Path::new(&session_root),
        &from_id,
        &to_id,
    ))
}

#[uniffi::export]
pub fn sensorlog_path(session_root: String, sensor_log_id: String) -> String {
    path_string(core::sensorlog_path(
        Path::new(&session_root),
        &sensor_log_id,
    ))
}

#[uniffi::export]
pub fn poselog_path(session_root: String, from_frame_id: String, to_frame_id: String) -> String {
    path_string(core::poselog_path(
        Path::new(&session_root),
        &from_frame_id,
        &to_frame_id,
    ))
}

#[uniffi::export]
pub fn detection_log_path(
    session_root: String,
    detector_id: String,
    input_log_id: String,
) -> String {
    path_string(core::detection_log_path(
        Path::new(&session_root),
        &detector_id,
        &input_log_id,
    ))
}

#[uniffi::export]
pub fn id_to_segment(id: String) -> String {
    core::id_to_segment(&id)
}
