//! Native task execution extracted from Posemesh's compute host.
//! DMS schedules tasks; this runtime owns one lease and awaits handler cleanup.
#![cfg(not(target_arch = "wasm32"))]

mod access;
mod compute;
mod machine;
mod peer;
mod robot;
mod robot_peer;
mod runtime;

pub use access::{TaskAccessToken, TaskCredential};
pub use auki_dms::types::TaskSpec;
pub use compute::{AukiComputeCredential, ComputeConfig};
pub use machine::MachineCredential;
pub use peer::{TaskPeerFactory, TaskPeerGrant, TaskPeerSession};
pub use robot::{AukiRobotCredential, RobotConfig};
pub use runtime::{
    AukiDmsTasks, TaskContext, TaskHandler, TaskLease, TaskOutcome, TaskResult, TasksConfig,
};

/// Errors intentionally contain no response bodies, credentials or handler text.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum TaskError {
    #[error("invalid task configuration: {0}")]
    Configuration(&'static str),
    #[error("task runtime is closed")]
    Closed,
    #[error("a task operation is already active")]
    Busy,
    #[error("task operation cancelled")]
    Cancelled,
    #[error("task authority was lost or expired")]
    LeaseLost,
    #[error("invalid task authority: {0}")]
    Authority(&'static str),
    /// DDS rejected a robot presence post or access-token refresh, or a compute
    /// registration failed. `source` is `presence`, `token`, or empty.
    #[error(transparent)]
    Authentication(AuthenticationFailure),
    /// DDS rejected the robot peer-token exchange. A transport blip uses
    /// `Authority("robot P2P exchange unavailable")` and is retried.
    #[error("robot P2P exchange was rejected (HTTP {status})")]
    PeerExchangeRejected { status: u16 },
    #[error("DMS {0} failed; its outcome may be unknown")]
    Service(&'static str),
    #[error("DMS {operation} returned HTTP {status}")]
    HttpStatus {
        operation: &'static str,
        status: u16,
    },
    #[error("task handler failed")]
    Handler,
    #[error("task data client could not be created")]
    Data,
    #[error("task peer shutdown failed")]
    PeerCleanup,
}

pub type Result<T> = std::result::Result<T, TaskError>;

/// Which robot refresh DDS rejected, plus the HTTP status when the response had one.
#[derive(Debug, Clone, Copy)]
pub struct AuthenticationFailure {
    pub source: &'static str,
    pub status: Option<u16>,
}

impl std::fmt::Display for AuthenticationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("machine authentication or registration failed")?;
        if !self.source.is_empty() {
            write!(f, ": {} refresh was rejected", self.source)?;
        }
        if let Some(status) = self.status {
            write!(f, " (HTTP {status})")?;
        }
        Ok(())
    }
}

impl std::error::Error for AuthenticationFailure {}

impl TaskError {
    pub(crate) fn authentication(source: &'static str, status: Option<u16>) -> Self {
        Self::Authentication(AuthenticationFailure { source, status })
    }

    pub(crate) fn dms(operation: &'static str, error: anyhow::Error) -> Self {
        match error.downcast_ref::<auki_dms::client::DmsHttpError>() {
            Some(error) => Self::HttpStatus {
                operation,
                status: error.status,
            },
            None => Self::Service(operation),
        }
    }
}
