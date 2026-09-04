//! Persistent typed-message protocols.

#[cfg(feature = "message-endpoint")]
mod endpoint;
mod resource;

#[cfg(feature = "message")]
pub mod v1;

pub use resource::{MessageChannelResource, MessageChannelResourceError};

#[cfg(feature = "message-endpoint")]
pub use endpoint::{
    MAX_INBOUND_MESSAGE_FRAME_MEMORY_BYTES, MESSAGE_CLOSE_TIMEOUT, MESSAGE_MAX_CONCURRENCY,
    MESSAGE_MAX_CONCURRENCY_PER_PEER, MESSAGE_OPEN_TIMEOUT, MESSAGE_SEND_TIMEOUT,
    MessageChannelReceiver, MessageChannelRegistrationError, MessageChannelSender, MessageClient,
    MessageEndpoint, MessageEndpointError, MessageEvent, MessageOperation, OUTBOUND_QUEUE_CAPACITY,
    OpenMessageChannelError, SendMessageError, message_protocol_spec,
};
