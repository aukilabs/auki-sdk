//! libp2p stream helpers for serving Subscribe requests.

use crate::protocols::subscribe_protocol;
use auki_protocol::v1::{
    frame::{self, FrameError},
    message::SpatialMessage,
    subscribe::{
        SubscribeDataError, SubscribeEnd, SubscribeEndError, SubscribeRequest,
        SubscribeRequestError, SubscribeStartResult,
    },
};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use serde_json::Value;
use std::{fmt, io};

/// Encoded Subscribe frame with its exact body length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedSubscribeFrame {
    bytes: Vec<u8>,
    body_len: usize,
}

/// Errors produced while serving one inbound Subscribe stream.
#[derive(Debug)]
pub enum SubscribeServeError {
    /// Underlying stream I/O failed.
    Io(io::Error),
    /// RFC JSON frame encoding or decoding failed.
    Frame(FrameError),
    /// Decoded frame was not a valid Subscribe request.
    Request(SubscribeRequestError),
    /// Subscribe data message failed served-stream validation.
    Data(SubscribeDataError),
    /// Subscribe end message failed construction or validation.
    End(SubscribeEndError),
    /// A served subscription was used after it ended.
    AlreadyEnded,
}

/// Accept inbound Subscribe streams on a libp2p-stream control.
pub fn accept_subscribe_streams(
    control: &mut libp2p_stream::Control,
) -> Result<libp2p_stream::IncomingStreams, libp2p_stream::AlreadyRegistered> {
    control.accept(subscribe_protocol())
}

/// Read one Subscribe request frame from an inbound stream.
pub async fn read_subscribe_request<S>(
    stream: &mut S,
    max_body_len: u64,
) -> Result<SubscribeRequest, SubscribeServeError>
where
    S: AsyncRead + Unpin,
{
    let frame = read_complete_frame(stream, max_body_len).await?;
    let (value, consumed) =
        frame::decode_json_frame(&frame, max_body_len).map_err(SubscribeServeError::Frame)?;
    debug_assert_eq!(consumed, frame.len());
    SubscribeRequest::from_value(value).map_err(SubscribeServeError::Request)
}

/// Read one Subscribe end frame from an accepted inbound stream.
pub async fn read_subscribe_end<S>(
    stream: &mut S,
    max_body_len: u64,
) -> Result<SubscribeEnd, SubscribeServeError>
where
    S: AsyncRead + Unpin,
{
    let frame = read_complete_frame(stream, max_body_len).await?;
    let (value, consumed) =
        frame::decode_json_frame(&frame, max_body_len).map_err(SubscribeServeError::Frame)?;
    debug_assert_eq!(consumed, frame.len());
    SubscribeEnd::from_value(value).map_err(SubscribeServeError::End)
}

/// Encode a Subscribe data message without writing it.
pub fn encode_subscribe_data_frame(
    message: &SpatialMessage,
    max_body_len: u64,
) -> Result<EncodedSubscribeFrame, SubscribeServeError> {
    encode_subscribe_frame(message.value(), max_body_len)
}

/// Write one Subscribe accept or reject frame.
pub async fn write_subscribe_start_result<S>(
    stream: &mut S,
    result: &SubscribeStartResult,
    max_body_len: u64,
) -> Result<(), SubscribeServeError>
where
    S: AsyncWrite + Unpin,
{
    let frame = encode_subscribe_frame(result.value(), max_body_len)?;
    write_encoded_subscribe_frame(stream, &frame).await
}

/// Write one pre-encoded Subscribe data frame.
pub async fn write_encoded_subscribe_frame<S>(
    stream: &mut S,
    frame: &EncodedSubscribeFrame,
) -> Result<(), SubscribeServeError>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(frame.bytes())
        .await
        .map_err(SubscribeServeError::Io)?;
    stream.flush().await.map_err(SubscribeServeError::Io)
}

/// Write one Subscribe end frame and close the stream.
pub async fn write_subscribe_end<S>(
    stream: &mut S,
    end: &SubscribeEnd,
    max_body_len: u64,
) -> Result<(), SubscribeServeError>
where
    S: AsyncWrite + Unpin,
{
    let frame = encode_subscribe_frame(end.value(), max_body_len)?;
    write_encoded_subscribe_frame(stream, &frame).await?;
    close_subscribe_stream(stream).await
}

/// Close a Subscribe stream.
pub async fn close_subscribe_stream<S>(stream: &mut S) -> Result<(), SubscribeServeError>
where
    S: AsyncWrite + Unpin,
{
    stream.close().await.map_err(SubscribeServeError::Io)
}

impl EncodedSubscribeFrame {
    /// Encoded frame bytes, including the length prefix.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Exact JSON frame body length.
    pub fn body_len(&self) -> usize {
        self.body_len
    }
}

fn encode_subscribe_frame(
    value: &Value,
    max_body_len: u64,
) -> Result<EncodedSubscribeFrame, SubscribeServeError> {
    let bytes =
        frame::encode_json_frame(value, max_body_len).map_err(SubscribeServeError::Frame)?;
    let (body_len, _) =
        frame::decode_length(&bytes, max_body_len).map_err(SubscribeServeError::Frame)?;
    let body_len = usize::try_from(body_len)
        .map_err(|_| SubscribeServeError::Frame(FrameError::LengthOverflow))?;
    Ok(EncodedSubscribeFrame { bytes, body_len })
}

async fn read_complete_frame<S>(
    stream: &mut S,
    max_body_len: u64,
) -> Result<Vec<u8>, SubscribeServeError>
where
    S: AsyncRead + Unpin,
{
    let mut prefix = Vec::with_capacity(frame::MAX_LEB128_U64_BYTES);

    loop {
        let mut byte = [0u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .map_err(SubscribeServeError::Io)?;
        prefix.push(byte[0]);

        match frame::decode_length(&prefix, max_body_len) {
            Ok((body_len, prefix_len)) => {
                debug_assert_eq!(prefix_len, prefix.len());
                let body_len = usize::try_from(body_len)
                    .map_err(|_| SubscribeServeError::Frame(FrameError::LengthOverflow))?;
                let mut body = vec![0u8; body_len];
                stream
                    .read_exact(&mut body)
                    .await
                    .map_err(SubscribeServeError::Io)?;

                let mut complete = prefix;
                complete.extend_from_slice(&body);
                return Ok(complete);
            }
            Err(FrameError::UnexpectedEof) if prefix.len() < frame::MAX_LEB128_U64_BYTES => {}
            Err(error) => return Err(SubscribeServeError::Frame(error)),
        }
    }
}

impl fmt::Display for SubscribeServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "subscribe stream io: {error}"),
            Self::Frame(error) => write!(f, "subscribe frame: {error}"),
            Self::Request(error) => write!(f, "subscribe request: {error}"),
            Self::Data(error) => write!(f, "subscribe data: {error}"),
            Self::End(error) => write!(f, "subscribe end: {error}"),
            Self::AlreadyEnded => write!(f, "subscribe stream already ended"),
        }
    }
}

impl std::error::Error for SubscribeServeError {}
