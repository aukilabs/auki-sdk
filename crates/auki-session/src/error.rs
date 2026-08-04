//! Session error type.

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid registry id: {0}")]
    InvalidId(#[from] auki_registry::RegistryIdError),
    #[error("registry: {0}")]
    Registry(#[from] auki_registry::Error),
    #[error("manifest: {0}")]
    Manifest(#[from] auki_manifests::ManifestValidationError),
    #[error("log: {0}")]
    Log(#[from] auki_logs::Error),
    #[error("duplicate log {source_peer_id}/{resource_id}")]
    DuplicateLog {
        source_peer_id: String,
        resource_id: String,
    },
    #[error("Map Registry entry is not registered locally: {peer_id}/{map_id}@{map_hash}")]
    MapNotRegistered {
        peer_id: String,
        map_id: String,
        map_hash: String,
    },
    #[error("clock is not registered in this Session: {peer_id}/{clock_id}@{clock_hash}")]
    ClockNotRegistered {
        peer_id: String,
        clock_id: String,
        clock_hash: String,
    },
    #[error("materialization: {0}")]
    Materialization(#[from] crate::materialization::MaterializationError),
}

pub type Result<T> = std::result::Result<T, SessionError>;
