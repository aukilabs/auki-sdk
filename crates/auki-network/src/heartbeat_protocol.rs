//! `/auki/heartbeat/0.0.1` — libp2p carrier protocol for sub-second
//! cluster heartbeat frames.
//!
//! ## Shape
//!
//! Bidirectional, long-lived. The domain layer decides which peers
//! should maintain a heartbeat carrier; the libp2p runtime opens or
//! accepts the substream and both sides write a [`Heartbeat`] frame
//! every [`HEARTBEAT_INTERVAL`].
//!
//! In today's Hagall cluster behavior, `auki-domain::ClusterManager`
//! uses a Manager-star topology: the Manager opens carriers to peers,
//! non-Managers watch the Manager, and missed frames within
//! [`HEARTBEAT_TIMEOUT`] trigger election or peer eviction. That
//! topology/timer policy is intentionally above this wire module so a
//! future non-libp2p transport can carry the same domain heartbeat.
//!
//! The runtime reports received frames and carrier closure upward; the
//! domain layer owns last-seen timestamps and the consequences of loss.
//!
//! ## Why custom (vs libp2p `ping`)
//!
//! libp2p's built-in `ping` ticks every 15 s by default, far too slow
//! for the sub-second cadence Hagall wants. It also doesn't carry a
//! payload that can later double as a time-sync signal. The custom
//! protocol lets the payload evolve: today a single unix-ns
//! timestamp; tomorrow a TimeTransform-shaped payload for peer-clock
//! agreement.
//!
//! ## Wire format
//!
//! Length-prefixed JSON (4-byte BE length + UTF-8 JSON, 1 KiB cap).
//! `Heartbeat` is small enough that JSON is fine; future
//! TimeTransform-shaped payloads may want prost.

use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// libp2p protocol id for the peer-side heartbeat. Stable; bump
/// version only on an incompatible wire-shape change.
pub const HEARTBEAT_PROTOCOL: &str = "/auki/heartbeat/0.0.1";

/// Cadence at which each side of a heartbeat-pair writes a
/// `Heartbeat` frame. 500 ms is fast enough for sub-second death
/// detection (~1.5 s window) and slow enough that the load is
/// negligible (~2 frames/sec/peer-pair).
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);

/// How long a peer waits without receiving a heartbeat before
/// marking the other side dead. 1500 ms = 3 missed heartbeats at
/// the 500 ms cadence; matches the "sub-second connection-drop
/// detection" the Hagall spec calls for.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_millis(1500);

/// Cap on a single framed `Heartbeat`. 1 KiB — current payload is
/// ~30 bytes; the cap is for defense against malformed senders.
pub const MAX_HEARTBEAT_FRAME_BYTES: u32 = 1024;

/// Heartbeat payload. Carries a unix-nanosecond timestamp the
/// receiver can use as a liveness signal today and as a time-sync
/// hint tomorrow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heartbeat {
    /// Sender's wall-clock at frame-write time, unix nanoseconds.
    pub sent_at_unix_ns: i64,
}

/// Failure modes for [`read_heartbeat`] / [`write_heartbeat`].
#[derive(Debug, Error)]
pub enum HeartbeatProtocolError {
    /// Underlying I/O on the libp2p substream failed.
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    /// JSON encode (write side) failed.
    #[error("encode: {0}")]
    Encode(#[source] serde_json::Error),
    /// JSON decode (read side) failed.
    #[error("decode: {0}")]
    Decode(#[source] serde_json::Error),
    /// Length prefix is zero.
    #[error("frame is empty (length prefix is zero)")]
    EmptyFrame,
    /// Length prefix exceeds [`MAX_HEARTBEAT_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write a [`Heartbeat`] to `stream`, length-prefixed JSON.
pub async fn write_heartbeat<S>(
    stream: &mut S,
    msg: &Heartbeat,
) -> Result<(), HeartbeatProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let bytes = serde_json::to_vec(msg).map_err(HeartbeatProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_HEARTBEAT_FRAME_BYTES as u64 {
        return Err(HeartbeatProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_HEARTBEAT_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(HeartbeatProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(HeartbeatProtocolError::Io)?;
    stream.flush().await.map_err(HeartbeatProtocolError::Io)?;
    Ok(())
}

/// Read a [`Heartbeat`] from `stream`. End-of-stream surfaces as
/// `Err(HeartbeatProtocolError::Io(_))` with `kind() ==
/// UnexpectedEof`.
pub async fn read_heartbeat<S>(stream: &mut S) -> Result<Heartbeat, HeartbeatProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(HeartbeatProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(HeartbeatProtocolError::EmptyFrame);
    }
    if len > MAX_HEARTBEAT_FRAME_BYTES {
        return Err(HeartbeatProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_HEARTBEAT_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(HeartbeatProtocolError::Io)?;
    serde_json::from_slice(&payload).map_err(HeartbeatProtocolError::Decode)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Heartbeat {
        Heartbeat {
            sent_at_unix_ns: 1_715_423_400_000_000_000,
        }
    }

    #[tokio::test]
    async fn heartbeat_round_trips_through_framed_stream() {
        let msg = sample();
        let mut buf = Vec::new();
        write_heartbeat(&mut buf, &msg).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_heartbeat(&mut cursor).await.unwrap();
        assert_eq!(msg, back);
    }

    #[tokio::test]
    async fn read_rejects_zero_length() {
        let buf = vec![0u8; 4]; // length prefix == 0
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_heartbeat(&mut cursor).await.unwrap_err();
        assert!(matches!(err, HeartbeatProtocolError::EmptyFrame));
    }

    #[tokio::test]
    async fn read_rejects_oversized_length_prefix() {
        let mut buf = (MAX_HEARTBEAT_FRAME_BYTES + 1).to_be_bytes().to_vec();
        buf.extend(std::iter::repeat(0u8).take(8));
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_heartbeat(&mut cursor).await.unwrap_err();
        assert!(matches!(err, HeartbeatProtocolError::FrameTooLarge { .. }));
    }

    #[test]
    fn cadence_matches_spec() {
        // Sub-second death detection — 500 ms cadence, 1500 ms
        // timeout (3 missed heartbeats).
        assert_eq!(HEARTBEAT_INTERVAL, Duration::from_millis(500));
        assert_eq!(HEARTBEAT_TIMEOUT, Duration::from_millis(1500));
    }

    /// Pins the wire shape against rename. Cross-language consumers
    /// rely on these JSON field names.
    #[test]
    fn wire_shape_locked_field_names() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(
            json.contains("\"sent_at_unix_ns\":"),
            "missing wire key sent_at_unix_ns in {json}"
        );
    }
}
