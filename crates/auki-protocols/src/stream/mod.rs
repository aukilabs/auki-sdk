//! Typed-stream protocols.

#[cfg(feature = "stream-endpoint")]
mod endpoint;

pub mod v2;

#[cfg(feature = "stream-endpoint")]
pub use endpoint::{
    CLOSE_TIMEOUT, HANDSHAKE_TIMEOUT, LIVE_WRITE_TIMEOUT,
    MAX_CONCURRENCY as ENDPOINT_MAX_CONCURRENCY, SourceStream, StreamClient, StreamDispatch,
    StreamEndpoint, StreamEndpointError, StreamEntry, StreamError, StreamItem, StreamOperation,
    StreamPayload, StreamProvider, StreamSubscription, SubscriptionEntries, decline_all_streams,
    protocol_spec,
};
