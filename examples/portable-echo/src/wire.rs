//! Transport-neutral request/response echo protocol.
//!
//! This module owns the exact wire contract and conversation, but no transport,
//! authentication, executor, or platform lifecycle. A runtime supplies one
//! authenticated duplex stream and calls [`run_client`] or [`run_server`].

#![forbid(unsafe_code)]

use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Exact product-owned application protocol identifier.
pub const PROTOCOL_ID: &str = "/example/echo/1.0.0";

/// Maximum request or response payload size.
pub const MAX_FRAME_BYTES: usize = 1024;

/// One validated outbound echo request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EchoRequest(Vec<u8>);

impl EchoRequest {
    /// Validate and own one request payload.
    pub fn new(payload: impl Into<Vec<u8>>) -> Result<Self, EchoProtocolError> {
        Ok(Self(validate_payload(payload.into())?))
    }

    /// Borrow the exact request payload.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consume the request and return its payload.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

/// One validated inbound echo response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EchoResponse(Vec<u8>);

impl EchoResponse {
    /// Borrow the exact response payload.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consume the response and return its payload.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

/// Portable echo framing or conversation failure.
#[derive(Debug, thiserror::Error)]
pub enum EchoProtocolError {
    /// The supplied or received payload is empty.
    #[error("echo payload must not be empty")]
    EmptyFrame,
    /// A supplied or declared payload exceeds [`MAX_FRAME_BYTES`].
    #[error("echo frame is {actual} bytes; maximum is {maximum}")]
    FrameTooLarge {
        /// Supplied or declared payload size.
        actual: u64,
        /// Fixed protocol limit.
        maximum: usize,
    },
    /// The underlying stream failed.
    #[error("echo stream I/O failed: {0}")]
    Io(#[source] std::io::Error),
    /// The responder did not echo the exact request bytes.
    #[error("echo response did not match the request")]
    ResponseMismatch,
}

/// Send one request and require one byte-identical response.
pub async fn run_client<S>(
    stream: &mut S,
    request: EchoRequest,
) -> Result<EchoResponse, EchoProtocolError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_frame(stream, request.as_bytes()).await?;
    let response = EchoResponse(read_frame(stream).await?);
    if response.as_bytes() != request.as_bytes() {
        return Err(EchoProtocolError::ResponseMismatch);
    }
    Ok(response)
}

/// Read one request, echo it exactly once, and return the accepted request.
pub async fn run_server<S>(stream: &mut S) -> Result<EchoRequest, EchoProtocolError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = EchoRequest(read_frame(stream).await?);
    write_frame(stream, request.as_bytes()).await?;
    Ok(request)
}

async fn write_frame<S>(stream: &mut S, payload: &[u8]) -> Result<(), EchoProtocolError>
where
    S: AsyncWrite + Unpin,
{
    validate_payload_len(payload.len() as u64)?;
    let length = u32::try_from(payload.len()).expect("the protocol limit fits in u32");
    stream
        .write_all(&length.to_be_bytes())
        .await
        .map_err(EchoProtocolError::Io)?;
    stream
        .write_all(payload)
        .await
        .map_err(EchoProtocolError::Io)?;
    stream.flush().await.map_err(EchoProtocolError::Io)
}

async fn read_frame<S>(stream: &mut S) -> Result<Vec<u8>, EchoProtocolError>
where
    S: AsyncRead + Unpin,
{
    let mut length = [0_u8; 4];
    stream
        .read_exact(&mut length)
        .await
        .map_err(EchoProtocolError::Io)?;
    let length = u32::from_be_bytes(length);
    validate_payload_len(u64::from(length))?;

    let mut payload = vec![0_u8; length as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(EchoProtocolError::Io)?;
    Ok(payload)
}

fn validate_payload(payload: Vec<u8>) -> Result<Vec<u8>, EchoProtocolError> {
    validate_payload_len(payload.len() as u64)?;
    Ok(payload)
}

fn validate_payload_len(actual: u64) -> Result<(), EchoProtocolError> {
    if actual == 0 {
        return Err(EchoProtocolError::EmptyFrame);
    }
    if actual > MAX_FRAME_BYTES as u64 {
        return Err(EchoProtocolError::FrameTooLarge {
            actual,
            maximum: MAX_FRAME_BYTES,
        });
    }
    Ok(())
}
