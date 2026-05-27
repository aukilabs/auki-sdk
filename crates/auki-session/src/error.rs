//! Session error type.

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid registry id: {0}")]
    InvalidId(#[from] auki_registry::RegistryIdError),
    #[error("registry: {0}")]
    Registry(#[from] auki_registry::Error),
    #[error("duplicate log {source_peer_id}/{resource_id}")]
    DuplicateLog { source_peer_id: String, resource_id: String },
    #[error("materialization: {0}")]
    Materialization(#[from] crate::materialization::MaterializationError),
    /// Returned by [`Session::join_domain`] when the cluster bootstrap fails.
    #[error("domain bootstrap: {0}")]
    DomainBootstrap(#[from] auki_domain::BootstrapError),
    /// Returned by [`Session::leave_domain`] when Discovery deregistration fails.
    #[error("domain shutdown: {0}")]
    DomainShutdown(auki_domain::DiscoveryClientError),
}

pub type Result<T> = std::result::Result<T, SessionError>;
