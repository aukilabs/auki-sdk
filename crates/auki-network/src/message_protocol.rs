//! `/auki/message/0.0.1` — generic peer-to-peer message protocol.
//!
//! One substream carries one length-prefixed protobuf
//! [`MessageEnvelope`] and, when the receiver chooses to acknowledge it,
//! one length-prefixed [`MessageAck`]. The envelope shape lives in
//! `proto/auki/message.proto` and is generated into `auki-proto`, so
//! browser JavaScript, native Swift, Python, and Rust all share the same
//! data contract.
//!
//! The network layer treats `MessageEnvelope.body` as opaque bytes. The
//! application names the payload contract with `type_url` and owns the
//! schema behind those bytes.

use futures::{AsyncReadExt, AsyncWriteExt};
use prost::Message;
use thiserror::Error;

pub use auki_proto::message::{MessageAck, MessageEnvelope};

/// libp2p protocol id for generic peer messages. Stable; bump version
/// only on an incompatible wire-shape change.
pub const MESSAGE_PROTOCOL: &str = "/auki/message/0.0.1";

/// Cap on a single message frame. Message payloads are control-plane
/// data, not bulk sensor or media transfer.
pub const MAX_MESSAGE_FRAME_BYTES: u32 = 1024 * 1024;

/// Failure modes for message-protocol framing.
#[derive(Debug, Error)]
pub enum MessageProtocolError {
    /// Underlying I/O on the libp2p substream failed.
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    /// Protobuf encoding failed.
    #[error("encode: {0}")]
    Encode(#[source] prost::EncodeError),
    /// Protobuf decoding failed.
    #[error("decode: {0}")]
    Decode(#[source] prost::DecodeError),
    /// Length prefix is zero.
    #[error("frame is empty (length prefix is zero)")]
    EmptyFrame,
    /// Length prefix exceeds [`MAX_MESSAGE_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write a [`MessageEnvelope`] to `stream`, length-prefixed protobuf.
pub async fn write_message_envelope<S>(
    stream: &mut S,
    msg: &MessageEnvelope,
) -> Result<(), MessageProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_proto(stream, msg).await
}

/// Read a [`MessageEnvelope`] from `stream`.
pub async fn read_message_envelope<S>(
    stream: &mut S,
) -> Result<MessageEnvelope, MessageProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_proto(stream).await
}

/// Write a [`MessageAck`] to `stream`, length-prefixed protobuf.
pub async fn write_message_ack<S>(
    stream: &mut S,
    msg: &MessageAck,
) -> Result<(), MessageProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_proto(stream, msg).await
}

/// Read a [`MessageAck`] from `stream`.
pub async fn read_message_ack<S>(stream: &mut S) -> Result<MessageAck, MessageProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_proto(stream).await
}

async fn write_proto<S, T>(stream: &mut S, msg: &T) -> Result<(), MessageProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Message,
{
    let mut bytes = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut bytes)
        .map_err(MessageProtocolError::Encode)?;
    if bytes.is_empty() {
        return Err(MessageProtocolError::EmptyFrame);
    }
    if bytes.len() as u64 > MAX_MESSAGE_FRAME_BYTES as u64 {
        return Err(MessageProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_MESSAGE_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(MessageProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(MessageProtocolError::Io)?;
    stream.flush().await.map_err(MessageProtocolError::Io)?;
    Ok(())
}

async fn read_proto<S, T>(stream: &mut S) -> Result<T, MessageProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: Message + Default,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(MessageProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(MessageProtocolError::EmptyFrame);
    }
    if len > MAX_MESSAGE_FRAME_BYTES {
        return Err(MessageProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_MESSAGE_FRAME_BYTES as u64,
        });
    }

    let mut bytes = vec![0u8; len as usize];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(MessageProtocolError::Io)?;
    T::decode(&*bytes).map_err(MessageProtocolError::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::Cursor;

    fn envelope_fixture() -> MessageEnvelope {
        MessageEnvelope {
            type_url: "auki.test/hello".to_string(),
            body: vec![1, 2, 3],
            request_id: "req-1".to_string(),
        }
    }

    fn ack_fixture() -> MessageAck {
        MessageAck {
            request_id: "req-1".to_string(),
            accepted: true,
            detail: "ok".to_string(),
        }
    }

    #[test]
    fn protocol_id_is_locked() {
        assert_eq!(MESSAGE_PROTOCOL, "/auki/message/0.0.1");
    }

    #[tokio::test]
    async fn message_envelope_frame_round_trips() {
        let envelope = envelope_fixture();
        let mut buf: Vec<u8> = Vec::new();
        write_message_envelope(&mut buf, &envelope).await.unwrap();
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(len as usize, buf.len() - 4);

        let mut cursor = Cursor::new(buf);
        let decoded = read_message_envelope(&mut cursor).await.unwrap();
        assert_eq!(decoded, envelope);
    }

    #[tokio::test]
    async fn message_ack_frame_round_trips() {
        let ack = ack_fixture();
        let mut buf: Vec<u8> = Vec::new();
        write_message_ack(&mut buf, &ack).await.unwrap();

        let mut cursor = Cursor::new(buf);
        let decoded = read_message_ack(&mut cursor).await.unwrap();
        assert_eq!(decoded, ack);
    }

    #[tokio::test]
    async fn read_rejects_empty_frame() {
        let mut cursor = Cursor::new(0u32.to_be_bytes().to_vec());
        let err = read_message_envelope(&mut cursor).await;
        assert!(matches!(err, Err(MessageProtocolError::EmptyFrame)));
    }

    #[tokio::test]
    async fn read_rejects_oversized_frame_via_length_prefix() {
        let too_big = MAX_MESSAGE_FRAME_BYTES as u64 + 1;
        let len = (too_big as u32).to_be_bytes();
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&len);
        let mut cursor = Cursor::new(buf);
        let err = read_message_envelope(&mut cursor).await;
        assert!(matches!(
            err,
            Err(MessageProtocolError::FrameTooLarge { actual, max })
                if actual == too_big && max == MAX_MESSAGE_FRAME_BYTES as u64
        ));
    }
}
