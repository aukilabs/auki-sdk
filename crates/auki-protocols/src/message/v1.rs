//! Transport-independent framing for `/auki/auth/1/message/0.1.0` payloads.
//!
//! The authenticated Domain runtime uses this codec for persistent typed-message
//! substreams.
//! Authentication, stream ownership, queueing, and handler lifecycle remain
//! runtime concerns; this module owns only bounded wire bytes.

pub use auki_datatypes::message::Message;
use auki_registry::RegistryRef;
use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p_identity::PeerId;
use prost::Message as ProstMessage;
use thiserror::Error;

/// Exact authenticated message 0.1.0 protocol identifier.
pub const ID: &str = "/auki/auth/1/message/0.1.0";

/// Maximum encoded message frame size.
pub const MAX_MESSAGE_FRAME_BYTES: u32 = 16 * 1024 * 1024;

const OPEN_FRAME: u8 = 1;
const OPEN_RESPONSE_FRAME: u8 = 2;
const MESSAGE_FRAME: u8 = 3;
const ACK_FRAME: u8 = 4;

/// Message framing and protobuf errors.
#[derive(Debug, Error)]
pub enum MessageProtocolError {
    /// The underlying stream failed.
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    /// A message could not be encoded.
    #[error("protobuf encode: {0}")]
    Encode(#[source] prost::EncodeError),
    /// A message could not be decoded.
    #[error("protobuf decode: {0}")]
    Decode(#[source] prost::DecodeError),
    /// A zero-length frame is invalid.
    #[error("frame is empty")]
    EmptyFrame,
    /// A declared or encoded frame exceeds the fixed bound.
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
    /// The frame discriminator is not valid at this point in the exchange.
    #[error("unexpected frame kind {0:#04x}")]
    UnexpectedFrameKind(u8),
    /// The frame has an invalid internal shape.
    #[error("malformed transport frame: {0}")]
    MalformedFrame(&'static str),
}

/// Write the addressed receiver/channel open frame.
pub async fn write_open_frame<S>(
    stream: &mut S,
    owner_peer_id: PeerId,
    resource_id: &str,
    expected_clock: &RegistryRef,
) -> Result<(), MessageProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let owner = owner_peer_id.to_string();
    let mut frame = Vec::with_capacity(
        21 + owner.len()
            + resource_id.len()
            + expected_clock.peer_id.len()
            + expected_clock.id.len()
            + expected_clock.hash.len(),
    );
    frame.push(OPEN_FRAME);
    push_string(&mut frame, &owner);
    push_string(&mut frame, resource_id);
    push_string(&mut frame, &expected_clock.peer_id);
    push_string(&mut frame, &expected_clock.id);
    push_string(&mut frame, &expected_clock.hash);
    write_frame(stream, &frame).await
}

/// Decode an addressed receiver/channel open frame body.
pub fn decode_open_frame(
    frame: &[u8],
) -> Result<(PeerId, String, RegistryRef), MessageProtocolError> {
    let Some((&kind, mut rest)) = frame.split_first() else {
        return Err(MessageProtocolError::EmptyFrame);
    };
    if kind != OPEN_FRAME {
        return Err(MessageProtocolError::UnexpectedFrameKind(kind));
    }
    let owner = take_string(&mut rest)?;
    let resource_id = take_string(&mut rest)?;
    let clock_peer_id = take_string(&mut rest)?;
    let clock_id = take_string(&mut rest)?;
    let clock_hash = take_string(&mut rest)?;
    if !rest.is_empty() {
        return Err(MessageProtocolError::MalformedFrame(
            "open frame has trailing bytes",
        ));
    }
    let owner_peer_id = owner
        .parse()
        .map_err(|_| MessageProtocolError::MalformedFrame("invalid owner PeerId"))?;
    Ok((
        owner_peer_id,
        resource_id,
        RegistryRef {
            peer_id: clock_peer_id,
            id: clock_id,
            hash: clock_hash,
        },
    ))
}

/// Write whether an addressed channel was accepted.
pub async fn write_open_response<S>(
    stream: &mut S,
    accepted: bool,
) -> Result<(), MessageProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_frame(stream, &[OPEN_RESPONSE_FRAME, u8::from(accepted)]).await
}

/// Read whether an addressed channel was accepted.
pub async fn read_open_response<S>(stream: &mut S) -> Result<bool, MessageProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let frame = read_frame(stream).await?;
    match frame.as_slice() {
        [OPEN_RESPONSE_FRAME, 0] => Ok(false),
        [OPEN_RESPONSE_FRAME, 1] => Ok(true),
        [kind, ..] if *kind != OPEN_RESPONSE_FRAME => {
            Err(MessageProtocolError::UnexpectedFrameKind(*kind))
        }
        _ => Err(MessageProtocolError::MalformedFrame(
            "invalid open response",
        )),
    }
}

fn take_string(bytes: &mut &[u8]) -> Result<String, MessageProtocolError> {
    if bytes.len() < 4 {
        return Err(MessageProtocolError::MalformedFrame(
            "missing string length",
        ));
    }
    let len = u32::from_be_bytes(bytes[..4].try_into().expect("checked length")) as usize;
    *bytes = &bytes[4..];
    if bytes.len() < len {
        return Err(MessageProtocolError::MalformedFrame("truncated string"));
    }
    let value = std::str::from_utf8(&bytes[..len])
        .map_err(|_| MessageProtocolError::MalformedFrame("string is not UTF-8"))?
        .to_owned();
    *bytes = &bytes[len..];
    Ok(value)
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

/// Write one sequence-numbered typed message.
pub async fn write_message_frame<S>(
    stream: &mut S,
    sequence: u64,
    message: &Message,
) -> Result<(), MessageProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let mut payload = Vec::with_capacity(9 + message.encoded_len());
    payload.push(MESSAGE_FRAME);
    payload.extend_from_slice(&sequence.to_be_bytes());
    message
        .encode(&mut payload)
        .map_err(MessageProtocolError::Encode)?;
    write_frame(stream, &payload).await
}

/// Read and decode one sequence-numbered typed message.
pub async fn read_message_frame<S>(stream: &mut S) -> Result<(u64, Message), MessageProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let frame_len = read_frame_length(stream).await?;
    let payload = read_frame_body(stream, frame_len).await?;
    decode_message_frame(&payload)
}

/// Decode one message frame body after its length prefix was consumed.
pub fn decode_message_frame(payload: &[u8]) -> Result<(u64, Message), MessageProtocolError> {
    let Some((&kind, rest)) = payload.split_first() else {
        return Err(MessageProtocolError::EmptyFrame);
    };
    if kind != MESSAGE_FRAME {
        return Err(MessageProtocolError::UnexpectedFrameKind(kind));
    }
    if rest.len() < 8 {
        return Err(MessageProtocolError::MalformedFrame(
            "message frame is missing sequence",
        ));
    }
    let sequence = u64::from_be_bytes(rest[..8].try_into().expect("checked length"));
    let message = Message::decode(&rest[8..]).map_err(MessageProtocolError::Decode)?;
    Ok((sequence, message))
}

/// Write a transport acceptance acknowledgement.
pub async fn write_ack_frame<S>(stream: &mut S, sequence: u64) -> Result<(), MessageProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let mut payload = [0; 9];
    payload[0] = ACK_FRAME;
    payload[1..].copy_from_slice(&sequence.to_be_bytes());
    write_frame(stream, &payload).await
}

/// Read a transport acceptance acknowledgement.
pub async fn read_ack_frame<S>(stream: &mut S) -> Result<u64, MessageProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let payload = read_frame(stream).await?;
    let Some((&kind, rest)) = payload.split_first() else {
        return Err(MessageProtocolError::EmptyFrame);
    };
    if kind != ACK_FRAME {
        return Err(MessageProtocolError::UnexpectedFrameKind(kind));
    }
    if rest.len() != 8 {
        return Err(MessageProtocolError::MalformedFrame(
            "ack frame must contain exactly one sequence",
        ));
    }
    Ok(u64::from_be_bytes(rest.try_into().expect("checked length")))
}

/// Write one bounded, length-prefixed frame body.
pub async fn write_frame<S>(stream: &mut S, payload: &[u8]) -> Result<(), MessageProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    if payload.len() as u64 > MAX_MESSAGE_FRAME_BYTES as u64 {
        return Err(MessageProtocolError::FrameTooLarge {
            actual: payload.len() as u64,
            max: MAX_MESSAGE_FRAME_BYTES as u64,
        });
    }
    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .map_err(MessageProtocolError::Io)?;
    stream
        .write_all(payload)
        .await
        .map_err(MessageProtocolError::Io)?;
    stream.flush().await.map_err(MessageProtocolError::Io)
}

/// Read one bounded, length-prefixed frame body.
pub async fn read_frame<S>(stream: &mut S) -> Result<Vec<u8>, MessageProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let len = read_frame_length(stream).await?;
    read_frame_body(stream, len).await
}

/// Read and validate a frame length before reserving its body memory.
pub async fn read_frame_length<S>(stream: &mut S) -> Result<u32, MessageProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let mut len = [0; 4];
    stream
        .read_exact(&mut len)
        .await
        .map_err(MessageProtocolError::Io)?;
    let len = u32::from_be_bytes(len);
    if len == 0 {
        return Err(MessageProtocolError::EmptyFrame);
    }
    if len > MAX_MESSAGE_FRAME_BYTES {
        return Err(MessageProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_MESSAGE_FRAME_BYTES as u64,
        });
    }
    Ok(len)
}

/// Read a frame body whose length was already validated.
pub async fn read_frame_body<S>(
    stream: &mut S,
    validated_len: u32,
) -> Result<Vec<u8>, MessageProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    if validated_len == 0 {
        return Err(MessageProtocolError::EmptyFrame);
    }
    if validated_len > MAX_MESSAGE_FRAME_BYTES {
        return Err(MessageProtocolError::FrameTooLarge {
            actual: validated_len as u64,
            max: MAX_MESSAGE_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0; validated_len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(MessageProtocolError::Io)?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use auki_datatypes::message::Message;
    use auki_registry::RegistryRef;
    use futures::io::Cursor;
    use libp2p_identity::PeerId;

    use super::*;

    fn owner() -> PeerId {
        "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw"
            .parse()
            .unwrap()
    }

    #[tokio::test]
    async fn open_message_and_ack_bytes_are_locked_without_transport() {
        let clock = RegistryRef {
            peer_id: "peer".into(),
            id: "clock".into(),
            hash: "hash".into(),
        };
        let mut open = Vec::new();
        write_open_frame(&mut open, owner(), "events", &clock)
            .await
            .unwrap();
        let mut expected_open = vec![0x00, 0x00, 0x00, 0x5c, 0x01, 0x00, 0x00, 0x00, 0x34];
        expected_open.extend_from_slice(owner().to_string().as_bytes());
        expected_open.extend_from_slice(&[
            0x00, 0x00, 0x00, 0x06, b'e', b'v', b'e', b'n', b't', b's', 0x00, 0x00, 0x00, 0x04,
            b'p', b'e', b'e', b'r', 0x00, 0x00, 0x00, 0x05, b'c', b'l', b'o', b'c', b'k', 0x00,
            0x00, 0x00, 0x04, b'h', b'a', b's', b'h',
        ]);
        assert_eq!(open, expected_open);

        let message = Message {
            r#type: "type-b".into(),
            timestamp_ns: -9,
            payload: vec![1, 2, 3, 4],
        };
        let mut framed_message = Vec::new();
        write_message_frame(&mut framed_message, 44, &message)
            .await
            .unwrap();
        assert_eq!(
            framed_message,
            vec![
                0x00, 0x00, 0x00, 0x22, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x0a,
                0x06, b't', b'y', b'p', b'e', b'-', b'b', 0x10, 0xf7, 0xff, 0xff, 0xff, 0xff, 0xff,
                0xff, 0xff, 0xff, 0x01, 0x1a, 0x04, 0x01, 0x02, 0x03, 0x04,
            ]
        );
        assert_eq!(
            read_message_frame(&mut Cursor::new(framed_message))
                .await
                .unwrap(),
            (44, message)
        );

        let mut ack = Vec::new();
        write_ack_frame(&mut ack, 44).await.unwrap();
        assert_eq!(
            ack,
            vec![
                0x00, 0x00, 0x00, 0x09, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2c,
            ]
        );
    }

    #[tokio::test]
    async fn separately_read_body_revalidates_the_memory_bound() {
        assert!(matches!(
            read_frame_body(
                &mut Cursor::new(Vec::<u8>::new()),
                MAX_MESSAGE_FRAME_BYTES + 1
            )
            .await,
            Err(MessageProtocolError::FrameTooLarge { .. })
        ));
    }
}
