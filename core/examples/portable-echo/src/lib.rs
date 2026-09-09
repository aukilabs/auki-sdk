//! Portable echo wire contract and cross-platform Auki peer endpoint.
//!
//! The private modules keep transport-neutral framing separate from SDK
//! lifecycle mechanics. Their public authoring surface is re-exported here so
//! native and Web hosts depend on one application-protocol crate.

#![forbid(unsafe_code)]

mod endpoint;
mod wire;

pub use endpoint::{
    EchoClient, EchoEndpoint, EchoError, EchoEventReceiver, EchoOperation, EchoSendReceipt,
    EchoServeEvent, EchoServeReceipt, MAX_CONCURRENCY, OPERATION_TIMEOUT, protocol_spec,
};
pub use wire::{
    EchoProtocolError, EchoRequest, EchoResponse, MAX_FRAME_BYTES, PROTOCOL_ID, run_client,
    run_server,
};
