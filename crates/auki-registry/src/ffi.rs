use std::path::Path;

use crate::core;

uniffi::setup_scaffolding!();

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct RegistryWriteOutcome {
    pub status: String,
    pub hash: String,
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("JSON is not valid: {message}")]
    InvalidJson { message: String },
    #[error("registry I/O error: {message}")]
    Io { message: String },
    #[error("registry id mismatch: expected {expected}, found {found}")]
    IdMismatch { expected: String, found: String },
    #[error("frame axes are invalid: {message}")]
    InvalidAxes { message: String },
    #[error("sensor {sensor_id} references missing frame ({frame_id}, {frame_hash})")]
    FrameReferenceMissing {
        sensor_id: String,
        frame_id: String,
        frame_hash: String,
    },
}

#[uniffi::export]
pub fn sensor_entry_canonical_json(entry_json: String) -> Result<String, RegistryError> {
    let entry: core::SensorRegistryEntry = parse_json(&entry_json)?;
    Ok(utf8(entry.canonical_bytes()))
}

#[uniffi::export]
pub fn sensor_entry_hash(entry_json: String) -> Result<String, RegistryError> {
    let entry: core::SensorRegistryEntry = parse_json(&entry_json)?;
    Ok(entry.hash())
}

#[uniffi::export]
pub fn clock_entry_canonical_json(entry_json: String) -> Result<String, RegistryError> {
    let entry: core::ClockRegistryEntry = parse_json(&entry_json)?;
    Ok(utf8(entry.canonical_bytes()))
}

#[uniffi::export]
pub fn clock_entry_hash(entry_json: String) -> Result<String, RegistryError> {
    let entry: core::ClockRegistryEntry = parse_json(&entry_json)?;
    Ok(entry.hash())
}

#[uniffi::export]
pub fn frame_entry_canonical_json(entry_json: String) -> Result<String, RegistryError> {
    let entry: core::FrameRegistryEntry = parse_json(&entry_json)?;
    Ok(utf8(entry.canonical_bytes()))
}

#[uniffi::export]
pub fn frame_entry_hash(entry_json: String) -> Result<String, RegistryError> {
    let entry: core::FrameRegistryEntry = parse_json(&entry_json)?;
    Ok(entry.hash())
}

#[uniffi::export]
pub fn detector_entry_canonical_json(entry_json: String) -> Result<String, RegistryError> {
    let entry: core::DetectorRegistryEntry = parse_json(&entry_json)?;
    Ok(utf8(entry.canonical_bytes()))
}

#[uniffi::export]
pub fn detector_entry_hash(entry_json: String) -> Result<String, RegistryError> {
    let entry: core::DetectorRegistryEntry = parse_json(&entry_json)?;
    Ok(entry.hash())
}

#[uniffi::export]
pub fn frame_ros_body_json(frame_id: String) -> String {
    utf8(core::FrameRegistryEntry::ros_body(frame_id).canonical_bytes())
}

#[uniffi::export]
pub fn frame_ros_optical_json(frame_id: String) -> String {
    utf8(core::FrameRegistryEntry::ros_optical(frame_id).canonical_bytes())
}

#[uniffi::export]
pub fn frame_opengl_json(frame_id: String) -> String {
    utf8(core::FrameRegistryEntry::opengl(frame_id).canonical_bytes())
}

#[uniffi::export]
pub fn frame_unity_json(frame_id: String) -> String {
    utf8(core::FrameRegistryEntry::unity(frame_id).canonical_bytes())
}

#[uniffi::export]
pub fn write_sensor_entry_json(
    app_root: String,
    entry_json: String,
) -> Result<RegistryWriteOutcome, RegistryError> {
    let entry: core::SensorRegistryEntry = parse_json(&entry_json)?;
    core::write_sensor(Path::new(&app_root), &entry)
        .map(Into::into)
        .map_err(Into::into)
}

#[uniffi::export]
pub fn write_clock_entry_json(
    app_root: String,
    entry_json: String,
) -> Result<RegistryWriteOutcome, RegistryError> {
    let entry: core::ClockRegistryEntry = parse_json(&entry_json)?;
    core::write_clock(Path::new(&app_root), &entry)
        .map(Into::into)
        .map_err(Into::into)
}

#[uniffi::export]
pub fn write_frame_entry_json(
    app_root: String,
    entry_json: String,
) -> Result<RegistryWriteOutcome, RegistryError> {
    let entry: core::FrameRegistryEntry = parse_json(&entry_json)?;
    core::write_frame(Path::new(&app_root), &entry)
        .map(Into::into)
        .map_err(Into::into)
}

#[uniffi::export]
pub fn write_detector_entry_json(
    app_root: String,
    entry_json: String,
) -> Result<RegistryWriteOutcome, RegistryError> {
    let entry: core::DetectorRegistryEntry = parse_json(&entry_json)?;
    core::write_detector(Path::new(&app_root), &entry)
        .map(Into::into)
        .map_err(Into::into)
}

#[uniffi::export]
pub fn read_sensor_entry_json(
    app_root: String,
    sensor_id: String,
    hash: String,
) -> Result<Option<String>, RegistryError> {
    core::read_sensor(Path::new(&app_root), &sensor_id, &hash)
        .map(|entry| entry.map(|entry| utf8(entry.canonical_bytes())))
        .map_err(Into::into)
}

#[uniffi::export]
pub fn read_clock_entry_json(
    app_root: String,
    clock_id: String,
    hash: String,
) -> Result<Option<String>, RegistryError> {
    core::read_clock(Path::new(&app_root), &clock_id, &hash)
        .map(|entry| entry.map(|entry| utf8(entry.canonical_bytes())))
        .map_err(Into::into)
}

#[uniffi::export]
pub fn read_frame_entry_json(
    app_root: String,
    frame_id: String,
    hash: String,
) -> Result<Option<String>, RegistryError> {
    core::read_frame(Path::new(&app_root), &frame_id, &hash)
        .map(|entry| entry.map(|entry| utf8(entry.canonical_bytes())))
        .map_err(Into::into)
}

#[uniffi::export]
pub fn read_detector_entry_json(
    app_root: String,
    detector_id: String,
    hash: String,
) -> Result<Option<String>, RegistryError> {
    core::read_detector(Path::new(&app_root), &detector_id, &hash)
        .map(|entry| entry.map(|entry| utf8(entry.canonical_bytes())))
        .map_err(Into::into)
}

fn parse_json<T>(json: &str) -> Result<T, RegistryError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(json).map_err(|err| RegistryError::InvalidJson {
        message: err.to_string(),
    })
}

fn utf8(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("JCS output is valid UTF-8")
}

impl From<core::WriteOutcome> for RegistryWriteOutcome {
    fn from(outcome: core::WriteOutcome) -> Self {
        match outcome {
            core::WriteOutcome::Created(hash) => Self {
                status: "created".into(),
                hash,
            },
            core::WriteOutcome::AlreadyExists(hash) => Self {
                status: "already_exists".into(),
                hash,
            },
        }
    }
}

impl From<core::Error> for RegistryError {
    fn from(err: core::Error) -> Self {
        match err {
            core::Error::Io(err) => Self::Io {
                message: err.to_string(),
            },
            core::Error::Json(message) => Self::InvalidJson { message },
            core::Error::IdMismatch { expected, found } => Self::IdMismatch { expected, found },
            core::Error::InvalidAxes(message) => Self::InvalidAxes { message },
            core::Error::FrameReferenceMissing {
                sensor_id,
                frame_id,
                frame_hash,
            } => Self::FrameReferenceMissing {
                sensor_id,
                frame_id,
                frame_hash,
            },
        }
    }
}
