//! Participant information protocols.

#[cfg(feature = "info-endpoint")]
mod endpoint;

pub mod v1;

#[cfg(feature = "info-endpoint")]
pub use endpoint::{
    INFO_MAX_CONCURRENCY, INFO_OPERATION_TIMEOUT, InfoClient, InfoEndpoint, InfoEndpointError,
    InfoOperation, InfoProvider, info_protocol_spec,
};
