use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::core;

uniffi::setup_scaffolding!();

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BytesLogEntry {
    pub timestamp_ns: i64,
    pub payload: Vec<u8>,
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum LogError {
    #[error("log I/O error: {message}")]
    Io { message: String },
    #[error("payload error: {message}")]
    Payload { message: String },
    #[error("manifest error: {message}")]
    Manifest { message: String },
    #[error("format error: {message}")]
    Format { message: String },
    #[error("log lock poisoned")]
    LockPoisoned,
}

#[derive(uniffi::Object)]
pub struct BytesLog {
    inner: Mutex<core::Log<core::BytesPayload>>,
}

#[uniffi::export]
impl BytesLog {
    #[uniffi::constructor]
    pub fn open(root: String, manifest_json: String) -> Result<Arc<Self>, LogError> {
        let manifest = parse_manifest_json(&manifest_json)?;
        let log = core::Log::open(Path::new(&root), manifest)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(log),
        }))
    }

    pub fn manifest_json(&self) -> Result<String, LogError> {
        let log = self.inner.lock().map_err(|_| LogError::LockPoisoned)?;
        Ok(manifest_json(log.manifest()))
    }

    pub fn append(&self, timestamp_ns: i64, payload: Vec<u8>) -> Result<(), LogError> {
        let mut log = self.inner.lock().map_err(|_| LogError::LockPoisoned)?;
        log.append(timestamp_ns, &core::BytesPayload { bytes: payload })
            .map_err(Into::into)
    }

    pub fn flush(&self) -> Result<(), LogError> {
        let mut log = self.inner.lock().map_err(|_| LogError::LockPoisoned)?;
        log.flush().map_err(Into::into)
    }

    pub fn set_retention(&self, retention_ns: i64) -> Result<(), LogError> {
        let mut log = self.inner.lock().map_err(|_| LogError::LockPoisoned)?;
        log.set_retention(retention_ns).map_err(Into::into)
    }
}

#[derive(uniffi::Object)]
pub struct BytesTail {
    inner: Mutex<core::TailIter<core::BytesPayload>>,
}

#[uniffi::export]
impl BytesTail {
    #[uniffi::constructor]
    pub fn open(root: String) -> Result<Arc<Self>, LogError> {
        let tail = core::Log::<core::BytesPayload>::tail(Path::new(&root))?;
        Ok(Arc::new(Self {
            inner: Mutex::new(tail),
        }))
    }

    pub fn try_next(&self) -> Result<Option<BytesLogEntry>, LogError> {
        let mut tail = self.inner.lock().map_err(|_| LogError::LockPoisoned)?;
        tail.try_next()
            .map(|entry| entry.map(Into::into))
            .map_err(Into::into)
    }
}

#[uniffi::export]
pub fn read_bytes_log_entries(root: String) -> Result<Vec<BytesLogEntry>, LogError> {
    let reader = core::Log::<core::BytesPayload>::read(Path::new(&root))?;
    reader
        .entries()
        .map(|entries| entries.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[uniffi::export]
pub fn read_bytes_log_manifest_json(root: String) -> Result<String, LogError> {
    let reader = core::Log::<core::BytesPayload>::read(Path::new(&root))?;
    Ok(manifest_json(reader.manifest()))
}

#[uniffi::export]
pub fn canonical_manifest_json(manifest_json: String) -> Result<String, LogError> {
    core::canonical_manifest_json_str(&manifest_json).map_err(Into::into)
}

#[uniffi::export]
pub fn encode_segment_entries_json(
    start_ns: i64,
    entries_json: String,
) -> Result<Vec<u8>, LogError> {
    let entries = core::bytes_entries_from_json(&entries_json)?;
    core::encode_segment_bytes(start_ns, &entries).map_err(Into::into)
}

#[uniffi::export]
pub fn decode_segment_entries_json(segment_bytes: Vec<u8>) -> Result<String, LogError> {
    let entries = core::decode_segment_bytes(&segment_bytes)?;
    Ok(core::bytes_entries_to_json(&entries))
}

fn parse_manifest_json(json: &str) -> Result<serde_json::Value, LogError> {
    serde_json::from_str(json).map_err(|err| LogError::Manifest {
        message: err.to_string(),
    })
}

fn manifest_json(manifest: &serde_json::Value) -> String {
    String::from_utf8(auki_jcs::canonicalize(manifest)).expect("JCS output is valid UTF-8")
}

impl From<core::Entry<core::BytesPayload>> for BytesLogEntry {
    fn from(entry: core::Entry<core::BytesPayload>) -> Self {
        Self {
            timestamp_ns: entry.timestamp_ns,
            payload: entry.payload.bytes,
        }
    }
}

impl From<core::Error> for LogError {
    fn from(err: core::Error) -> Self {
        match err {
            core::Error::Io(err) => Self::Io {
                message: err.to_string(),
            },
            core::Error::Payload(message) => Self::Payload { message },
            core::Error::Manifest(message) => Self::Manifest { message },
            core::Error::Format(message) => Self::Format { message },
        }
    }
}
