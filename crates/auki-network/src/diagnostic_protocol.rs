//! `/auki/diagnostic/0.0.1` — best-effort app diagnostic messages.
//!
//! One substream carries one length-prefixed JSON [`DiagnosticMessage`].
//! The network layer only knows the topic and opaque JSON payload; each
//! application owns its topic schema.

use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// libp2p protocol id for generic diagnostic messages.
pub const DIAGNOSTIC_PROTOCOL: &str = "/auki/diagnostic/0.0.1";

/// Cap on a single diagnostic frame. Diagnostic payloads should be tiny.
pub const MAX_DIAGNOSTIC_FRAME_BYTES: u32 = 64 * 1024;

#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticMessage {
    pub topic: String,
    pub payload_json: String,
}

#[derive(Debug, Error)]
pub enum DiagnosticProtocolError {
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("json: {0}")]
    Json(#[source] serde_json::Error),
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
    #[error("frame is empty")]
    EmptyFrame,
}

pub async fn write_diagnostic_message<S>(
    stream: &mut S,
    message: &DiagnosticMessage,
) -> Result<(), DiagnosticProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let bytes = serde_json::to_vec(message).map_err(DiagnosticProtocolError::Json)?;
    if bytes.len() as u64 > MAX_DIAGNOSTIC_FRAME_BYTES as u64 {
        return Err(DiagnosticProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_DIAGNOSTIC_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(DiagnosticProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(DiagnosticProtocolError::Io)?;
    stream.flush().await.map_err(DiagnosticProtocolError::Io)?;
    Ok(())
}

pub async fn read_diagnostic_message<S>(
    stream: &mut S,
) -> Result<DiagnosticMessage, DiagnosticProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(DiagnosticProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(DiagnosticProtocolError::EmptyFrame);
    }
    if len > MAX_DIAGNOSTIC_FRAME_BYTES {
        return Err(DiagnosticProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_DIAGNOSTIC_FRAME_BYTES as u64,
        });
    }

    let mut bytes = vec![0u8; len as usize];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(DiagnosticProtocolError::Io)?;
    serde_json::from_slice(&bytes).map_err(DiagnosticProtocolError::Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn diagnostic_message_round_trips() {
        let message = DiagnosticMessage {
            topic: "diagnostic.tick-report".into(),
            payload_json: r#"{"tick_id":7}"#.into(),
        };
        let mut bytes = Vec::new();

        write_diagnostic_message(&mut bytes, &message)
            .await
            .expect("write");
        let decoded = read_diagnostic_message(&mut bytes.as_slice())
            .await
            .expect("read");

        assert_eq!(decoded, message);
    }
}
