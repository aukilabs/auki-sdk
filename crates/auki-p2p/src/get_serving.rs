//! libp2p stream helpers for serving Get requests.

use crate::protocols::get_protocol;
use auki_protocol::v1::{
    frame::{self, FrameError},
    get::{GetRequest, GetRequestError, GetResponse},
};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::{fmt, io};

/// Errors produced while serving one inbound Get request.
#[derive(Debug)]
pub enum GetServeError {
    /// Underlying stream I/O failed.
    Io(io::Error),
    /// RFC JSON frame encoding or decoding failed.
    Frame(FrameError),
    /// Decoded frame was not a valid Get request.
    Request(GetRequestError),
}

/// Accept inbound Get streams on a libp2p-stream control.
pub fn accept_get_streams(
    control: &mut libp2p_stream::Control,
) -> Result<libp2p_stream::IncomingStreams, libp2p_stream::AlreadyRegistered> {
    control.accept(get_protocol())
}

/// Read one Get request frame from an inbound stream.
pub async fn read_get_request<S>(
    stream: &mut S,
    max_body_len: u64,
) -> Result<GetRequest, GetServeError>
where
    S: AsyncRead + Unpin,
{
    let frame = read_complete_frame(stream, max_body_len).await?;
    let (value, consumed) =
        frame::decode_json_frame(&frame, max_body_len).map_err(GetServeError::Frame)?;
    debug_assert_eq!(consumed, frame.len());
    GetRequest::from_value(value).map_err(GetServeError::Request)
}

/// Write one Get response frame and close the stream.
pub async fn write_get_response<S>(
    stream: &mut S,
    response: &GetResponse,
    max_body_len: u64,
) -> Result<(), GetServeError>
where
    S: AsyncWrite + Unpin,
{
    let frame =
        frame::encode_json_frame(response.value(), max_body_len).map_err(GetServeError::Frame)?;
    stream.write_all(&frame).await.map_err(GetServeError::Io)?;
    stream.flush().await.map_err(GetServeError::Io)?;
    stream.close().await.map_err(GetServeError::Io)
}

async fn read_complete_frame<S>(stream: &mut S, max_body_len: u64) -> Result<Vec<u8>, GetServeError>
where
    S: AsyncRead + Unpin,
{
    let mut prefix = Vec::with_capacity(frame::MAX_LEB128_U64_BYTES);

    loop {
        let mut byte = [0u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .map_err(GetServeError::Io)?;
        prefix.push(byte[0]);

        match frame::decode_length(&prefix, max_body_len) {
            Ok((body_len, prefix_len)) => {
                debug_assert_eq!(prefix_len, prefix.len());
                let body_len = usize::try_from(body_len)
                    .map_err(|_| GetServeError::Frame(FrameError::LengthOverflow))?;
                let mut body = vec![0u8; body_len];
                stream
                    .read_exact(&mut body)
                    .await
                    .map_err(GetServeError::Io)?;

                let mut complete = prefix;
                complete.extend_from_slice(&body);
                return Ok(complete);
            }
            Err(FrameError::UnexpectedEof) if prefix.len() < frame::MAX_LEB128_U64_BYTES => {}
            Err(error) => return Err(GetServeError::Frame(error)),
        }
    }
}

impl fmt::Display for GetServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "get stream io: {error}"),
            Self::Frame(error) => write!(f, "get frame: {error}"),
            Self::Request(error) => write!(f, "get request: {error}"),
        }
    }
}

impl std::error::Error for GetServeError {}
