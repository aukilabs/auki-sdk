//! `/auki/browser-session/0.0.1` — browser Domain control-plane
//! session between a browser leaf peer and a native Manager.
//!
//! The browser still joins through `/auki/join/0.0.1`. Once admitted,
//! it opens this long-lived stream to publish its UI-facing participant
//! state and receive Manager-pushed roster snapshots.

use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// libp2p protocol id for the browser roster/media presence session.
pub const BROWSER_SESSION_PROTOCOL: &str = "/auki/browser-session/0.0.1";

/// Cap on a single browser session frame. Mirrors the join protocol:
/// this is a small control plane, so 1 MiB is generous and defensive.
pub const MAX_BROWSER_SESSION_FRAME_BYTES: u32 = 1024 * 1024;

/// Client-to-Manager messages on the browser session stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserSessionClientMessage {
    /// First message on a fresh browser session stream.
    Hello {
        domain_name: String,
        participant: BrowserSessionParticipant,
    },
    /// Full local participant replacement after metadata/sensors/media
    /// change.
    UpdateParticipant {
        participant: BrowserSessionParticipant,
    },
    /// Local mic publication intent changed.
    SetSensorPublication { sensor_id: String, enabled: bool },
    /// Browser requested to listen to another peer's sensor.
    Subscribe { peer_id: String, sensor_id: String },
    /// Browser requested to stop listening to another peer's sensor.
    Unsubscribe { peer_id: String, sensor_id: String },
    /// Browser is leaving the Domain/session.
    Leave,
}

/// Manager-to-browser messages on the browser session stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserSessionServerMessage {
    /// Full roster snapshot for the receiving browser.
    Snapshot { snapshot: BrowserRosterSnapshot },
    /// Lightweight acknowledgement for control-plane updates that do
    /// not need to carry a fresh snapshot.
    Ack,
    /// Structured error surfaced by the Manager side.
    Error { code: String, message: String },
}

/// Park-compatible participant shape carried by the browser session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSessionParticipant {
    pub peer_id: String,
    pub app_id: String,
    pub display_name: String,
    pub is_self: bool,
    pub connected: bool,
    pub sensors: Vec<BrowserSessionSensor>,
    pub media_presence: BrowserMediaPresence,
}

/// Park-compatible sensor summary carried by a browser participant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSessionSensor {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub publishable: bool,
    pub subscribable: bool,
}

/// Park-compatible media presence carried by a browser participant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserMediaPresence {
    pub mic_available: bool,
    pub mic_publication_enabled: bool,
    pub mic_capture_healthy: bool,
    pub listening_to_peer_id: Option<String>,
    pub listening_to_sensor_id: Option<String>,
    pub playback_healthy: bool,
    pub selected_remote_stream_state: String,
    pub last_frame_unix_ms: Option<u64>,
    pub input_level: Option<u8>,
    pub output_level: Option<u8>,
}

impl Default for BrowserMediaPresence {
    fn default() -> Self {
        Self {
            mic_available: false,
            mic_publication_enabled: false,
            mic_capture_healthy: false,
            listening_to_peer_id: None,
            listening_to_sensor_id: None,
            playback_healthy: false,
            selected_remote_stream_state: "off".to_string(),
            last_frame_unix_ms: None,
            input_level: None,
            output_level: None,
        }
    }
}

/// Full roster snapshot pushed by the Manager to a browser peer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRosterSnapshot {
    pub self_peer_id: String,
    pub domain_name: String,
    pub manager_peer_id: String,
    pub election_state: String,
    pub participants: Vec<BrowserSessionParticipant>,
}

/// Failure modes for browser session framing.
#[derive(Debug, Error)]
pub enum BrowserSessionProtocolError {
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("encode: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("decode: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("frame is empty (length prefix is zero)")]
    EmptyFrame,
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write a browser client message to a length-prefixed JSON stream.
pub async fn write_client_message<S>(
    stream: &mut S,
    msg: &BrowserSessionClientMessage,
) -> Result<(), BrowserSessionProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Read a browser client message from a length-prefixed JSON stream.
pub async fn read_client_message<S>(
    stream: &mut S,
) -> Result<BrowserSessionClientMessage, BrowserSessionProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

/// Write a Manager server message to a length-prefixed JSON stream.
pub async fn write_server_message<S>(
    stream: &mut S,
    msg: &BrowserSessionServerMessage,
) -> Result<(), BrowserSessionProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Read a Manager server message from a length-prefixed JSON stream.
pub async fn read_server_message<S>(
    stream: &mut S,
) -> Result<BrowserSessionServerMessage, BrowserSessionProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

async fn write_json<S, T>(stream: &mut S, msg: &T) -> Result<(), BrowserSessionProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(msg).map_err(BrowserSessionProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_BROWSER_SESSION_FRAME_BYTES as u64 {
        return Err(BrowserSessionProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_BROWSER_SESSION_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(BrowserSessionProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(BrowserSessionProtocolError::Io)?;
    stream
        .flush()
        .await
        .map_err(BrowserSessionProtocolError::Io)?;
    Ok(())
}

async fn read_json<S, T>(stream: &mut S) -> Result<T, BrowserSessionProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(BrowserSessionProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(BrowserSessionProtocolError::EmptyFrame);
    }
    if len > MAX_BROWSER_SESSION_FRAME_BYTES {
        return Err(BrowserSessionProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_BROWSER_SESSION_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(BrowserSessionProtocolError::Io)?;
    serde_json::from_slice(&payload).map_err(BrowserSessionProtocolError::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_participant(peer_id: &str, is_self: bool) -> BrowserSessionParticipant {
        BrowserSessionParticipant {
            peer_id: peer_id.to_string(),
            app_id: "park".to_string(),
            display_name: peer_id.to_string(),
            is_self,
            connected: true,
            sensors: vec![BrowserSessionSensor {
                id: "audio".to_string(),
                kind: "audio".to_string(),
                label: "Microphone".to_string(),
                publishable: true,
                subscribable: false,
            }],
            media_presence: BrowserMediaPresence {
                mic_available: true,
                mic_publication_enabled: false,
                mic_capture_healthy: true,
                listening_to_peer_id: None,
                listening_to_sensor_id: None,
                playback_healthy: false,
                selected_remote_stream_state: "off".to_string(),
                last_frame_unix_ms: None,
                input_level: None,
                output_level: None,
            },
        }
    }

    #[test]
    fn protocol_id_is_stable() {
        assert_eq!(BROWSER_SESSION_PROTOCOL, "/auki/browser-session/0.0.1");
    }

    #[tokio::test]
    async fn client_and_server_messages_round_trip() {
        let participant = sample_participant("browser-a", true);
        let hello = BrowserSessionClientMessage::Hello {
            domain_name: "browser-two-peer-smoke".to_string(),
            participant: participant.clone(),
        };
        let publish = BrowserSessionClientMessage::SetSensorPublication {
            sensor_id: "audio".to_string(),
            enabled: true,
        };
        let snapshot = BrowserSessionServerMessage::Snapshot {
            snapshot: BrowserRosterSnapshot {
                self_peer_id: "browser-a".to_string(),
                domain_name: "browser-two-peer-smoke".to_string(),
                manager_peer_id: "manager".to_string(),
                election_state: "stable".to_string(),
                participants: vec![participant],
            },
        };

        let mut bytes = Vec::new();
        write_client_message(&mut bytes, &hello).await.unwrap();
        write_client_message(&mut bytes, &publish).await.unwrap();
        write_server_message(&mut bytes, &snapshot).await.unwrap();

        let mut cursor = futures::io::Cursor::new(bytes);
        assert_eq!(read_client_message(&mut cursor).await.unwrap(), hello);
        assert_eq!(read_client_message(&mut cursor).await.unwrap(), publish);
        assert_eq!(read_server_message(&mut cursor).await.unwrap(), snapshot);
    }
}
