//! Materialization of remote logs.

use auki_registry::LogRef;

#[derive(Debug, thiserror::Error)]
pub enum MaterializationError {
    #[error("remote catalog row not found: {0:?}")]
    NotFound(LogRef),
    #[error("connection: {0}")]
    Connection(String),
    #[error("not implemented: full implementation deferred to Phase 5")]
    NotImplemented,
}
