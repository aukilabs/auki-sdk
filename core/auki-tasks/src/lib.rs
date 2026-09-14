//! Native task execution extracted from Posemesh's compute host.
//! DMS schedules tasks; this runtime owns one lease and awaits handler cleanup.
#![cfg(not(target_arch = "wasm32"))]

mod access;
mod compute;
mod machine;
mod peer;
mod robot;
mod runtime;

pub use access::TaskCredential;
pub use auki_dms::types::TaskSpec;
pub use compute::{AukiComputeCredential, ComputeConfig};
pub use machine::MachineCredential;
pub use peer::{TaskPeerFactory, TaskPeerGrant, TaskPeerSession};
pub use robot::{AukiRobotCredential, RobotConfig};
pub use runtime::{
    AukiDmsTasks, TaskContext, TaskHandler, TaskLease, TaskOutcome, TaskResult, TasksConfig,
};

/// Errors intentionally contain no response bodies, credentials or handler text.
#[derive(Debug, thiserror::Error)]
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
    #[error("machine authentication or registration failed")]
    Authentication,
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

impl TaskError {
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
