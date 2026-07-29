//! `/auki/leave/0.0.1` — libp2p protocol for a peer to gracefully leave
//! a cluster.
//!
//! ## Shape
//!
//! One substream per leave. Leaver opens the substream, writes a
//! [`LeaveRequest`], reads a [`LeaveResponse`] Ack, and closes. The
//! Manager accepts, updates membership, Acks, then drops the peer from
//! the allow-list (Ack must land before the connection is torn down).
//!
//! Peer identity is **not** in the wire body — libp2p noise already
//! authenticated it. Empty [`LeaveResponse`] means Ack. There is no
//! Reject variant: leave is best-effort and idempotent.
//!
//! ## Wire format
//!
//! Length-prefixed protobuf (4-byte big-endian length, then prost body).
//! [`MAX_LEAVE_FRAME_BYTES`] caps either side.

use futures::{AsyncReadExt, AsyncWriteExt};
use prost::Message;
use thiserror::Error;

/// libp2p protocol id for the leave handshake.
pub const LEAVE_PROTOCOL: &str = "/auki/leave/0.0.1";

/// Cap on a single framed LeaveRequest / LeaveResponse.
pub const MAX_LEAVE_FRAME_BYTES: u32 = 64 * 1024;

/// Body of a leave request. Empty — peer comes from the connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeaveRequest;

/// Body of a leave response. Empty = Ack.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeaveResponse;

/// Failure modes for leave framing helpers.
#[derive(Debug, Error)]
pub enum LeaveProtocolError {
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("encode: {0}")]
    Encode(#[source] prost::EncodeError),
    #[error("decode: {0}")]
    Decode(#[source] prost::DecodeError),
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

pub async fn write_leave_request<S>(
    stream: &mut S,
    _msg: &LeaveRequest,
) -> Result<(), LeaveProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_frame(stream, &auki_datatypes::leave::LeaveRequest::default()).await
}

pub async fn write_leave_response<S>(
    stream: &mut S,
    _msg: &LeaveResponse,
) -> Result<(), LeaveProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_frame(stream, &auki_datatypes::leave::LeaveResponse::default()).await
}

pub async fn read_leave_request<S>(stream: &mut S) -> Result<LeaveRequest, LeaveProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let _: auki_datatypes::leave::LeaveRequest = read_frame(stream).await?;
    Ok(LeaveRequest)
}

pub async fn read_leave_response<S>(stream: &mut S) -> Result<LeaveResponse, LeaveProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let _: auki_datatypes::leave::LeaveResponse = read_frame(stream).await?;
    Ok(LeaveResponse)
}

async fn write_frame<S, T>(stream: &mut S, msg: &T) -> Result<(), LeaveProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Message,
{
    let mut bytes = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut bytes).map_err(LeaveProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_LEAVE_FRAME_BYTES as u64 {
        return Err(LeaveProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_LEAVE_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(LeaveProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(LeaveProtocolError::Io)?;
    stream.flush().await.map_err(LeaveProtocolError::Io)?;
    Ok(())
}

async fn read_frame<S, T>(stream: &mut S) -> Result<T, LeaveProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: Message + Default,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(LeaveProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_LEAVE_FRAME_BYTES {
        return Err(LeaveProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_LEAVE_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(LeaveProtocolError::Io)?;
    T::decode(&*payload).map_err(LeaveProtocolError::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn leave_request_round_trips() {
        let mut buf = Vec::new();
        write_leave_request(&mut buf, &LeaveRequest).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_leave_request(&mut cursor).await.unwrap();
        assert_eq!(back, LeaveRequest);
    }

    #[tokio::test]
    async fn leave_response_ack_round_trips() {
        let mut buf = Vec::new();
        write_leave_response(&mut buf, &LeaveResponse)
            .await
            .unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_leave_response(&mut cursor).await.unwrap();
        assert_eq!(back, LeaveResponse);
    }

    #[tokio::test]
    async fn zero_length_request_decodes() {
        let buf = vec![0u8; 4];
        let mut cursor = futures::io::Cursor::new(buf);
        assert_eq!(
            read_leave_request(&mut cursor).await.unwrap(),
            LeaveRequest
        );
    }
}
