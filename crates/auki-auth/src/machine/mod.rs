//! Native machine authentication operations extracted from the compute runner.
//!
//! Callers provide credentials and registration metadata explicitly. The token
//! manager starts background refresh only when `start_bg` is called and must be
//! stopped with `stop_bg`. No peer or DMS task loop is started here.

pub mod robot;
pub mod token_manager;
mod types;

pub use types::{AccessBundle, Result, SiweError};
