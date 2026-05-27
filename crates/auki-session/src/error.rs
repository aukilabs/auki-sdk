//! Session error type.

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid registry id: {0}")]
    InvalidId(#[from] auki_registry::RegistryIdError),
    #[error("duplicate log {source_peer_id}/{resource_id}")]
    DuplicateLog { source_peer_id: String, resource_id: String },
    #[error("materialization: {0}")]
    Materialization(#[from] crate::materialization::MaterializationError),
}

pub type Result<T> = std::result::Result<T, SessionError>;
