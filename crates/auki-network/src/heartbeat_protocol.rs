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
//! payload that can double as a time-sync signal. The custom protocol
//! carries sender-clock timestamps plus a one-frame echo so higher
//! layers can derive NTP-style timing samples without teaching the
//! network runtime about domain clocks.
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

/// A single heartbeat write should complete in microseconds. If
/// flushing one to the carrier takes at least this long, the local
/// peer's *outbound* liveness is at risk — the remote may declare it
/// `Lost` before the next frame lands. Surfaced as
/// [`PeerLivenessEvent::HeartbeatWriteStalled`][crate::PeerLivenessEvent]
/// for field diagnosis (e.g. transport congestion starving the
/// heartbeat substream). Set to one [`HEARTBEAT_INTERVAL`]: a write
/// slower than the whole cadence has already cost a frame.
pub const HEARTBEAT_WRITE_STALL_WARN: Duration = HEARTBEAT_INTERVAL;

/// Cap on a single framed `Heartbeat`. 1 KiB — current payload is
/// ~30 bytes; the cap is for defense against malformed senders.
pub const MAX_HEARTBEAT_FRAME_BYTES: u32 = 1024;

/// Heartbeat payload. Carries a sender timestamp, the sender clock's
/// registry identity, and an optional echo of the peer's previous
/// heartbeat so higher layers can derive NTP-style timing samples.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heartbeat {
    /// Sender's wall-clock at frame-write time, unix nanoseconds.
    /// This is retained for legacy/debug provenance; time-sync
    /// consumers use `sent_at_clock_ns`.
    pub sent_at_unix_ns: i64,
    /// Clock Registry id for `sent_at_clock_ns`. The identity must
    /// name one monotonic epoch; restarting a clock at zero requires
    /// a new id or hash.
    pub clock_id: String,
    /// Content-addressed hash of the Clock Registry entry named by
    /// `clock_id`.
    pub clock_hash: String,
    /// Per-sender heartbeat counter used only to match an echo to a
    /// previous heartbeat.
    pub sequence: u64,
    /// Sender's reading of `clock_id` at frame-write time.
    pub sent_at_clock_ns: i64,
    /// Optional echo of the most recently received heartbeat from the
    /// peer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo: Option<HeartbeatEcho>,
    /// Optional current domain-clock source declaration. The network
    /// runtime only carries this metadata; `auki-time` and
    /// `auki-domain` decide how to store, validate, and use it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_clock: Option<HeartbeatDomainClock>,
}

/// Echo information for one previously received heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeartbeatEcho {
    /// The echoed peer heartbeat's `sequence`.
    pub sequence: u64,
    /// Receiver's local clock reading when it received the echoed
    /// heartbeat.
    pub received_at_clock_ns: i64,
}

/// Domain-clock source metadata carried opportunistically by
/// heartbeat frames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeartbeatDomainClock {
    /// Cluster whose stable domain clock this source describes.
    pub cluster_name: String,
    /// Stable logical domain-clock id, typically
    /// `<cluster_name>/domain-clock`.
    pub domain_clock_id: String,
    /// Deterministic hash for the domain-clock declaration.
    pub domain_clock_hash: String,
    /// Peer whose concrete clock backs the domain clock.
    pub backing_peer_id: String,
    /// Concrete Clock Registry id backing the domain clock.
    pub backing_clock_id: String,
    /// Content-addressed hash of `backing_clock_id`.
    pub backing_clock_hash: String,
    /// Offset to add to a timestamp in `backing_clock_id` to express
    /// it in `domain_clock_id`.
    pub backing_to_domain_offset_ns: i64,
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
            clock_id: "12D3KooWPeerExample/session-123/monotonic".into(),
            clock_hash: "abc123".into(),
            sequence: 7,
            sent_at_clock_ns: 10_000,
            echo: None,
            domain_clock: None,
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

    #[test]
    fn heartbeat_wire_shape_includes_ntp_echo_fields() {
        let msg = Heartbeat {
            sent_at_unix_ns: 1_715_423_400_000_000_000,
            clock_id: "12D3KooWPeerExample/session-123/monotonic".into(),
            clock_hash: "abc123".into(),
            sequence: 7,
            sent_at_clock_ns: 10_000,
            echo: Some(HeartbeatEcho {
                sequence: 6,
                received_at_clock_ns: 20_000,
            }),
            domain_clock: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"sent_at_unix_ns\":"));
        assert!(json.contains("\"clock_id\":"));
        assert!(json.contains("\"clock_hash\":"));
        assert!(json.contains("\"sequence\":"));
        assert!(json.contains("\"sent_at_clock_ns\":"));
        assert!(json.contains("\"echo\":"));
        assert!(json.contains("\"received_at_clock_ns\":"));
    }

    #[test]
    fn heartbeat_wire_shape_includes_domain_clock_fields() {
        let msg = Heartbeat {
            sent_at_unix_ns: 1_715_423_400_000_000_000,
            clock_id: "12D3KooWPeerExample/session-123/monotonic".into(),
            clock_hash: "abc123".into(),
            sequence: 7,
            sent_at_clock_ns: 10_000,
            echo: None,
            domain_clock: Some(HeartbeatDomainClock {
                cluster_name: "cluster-a".into(),
                domain_clock_id: "cluster-a/domain-clock".into(),
                domain_clock_hash: "domainhash".into(),
                backing_peer_id: "12D3KooWPeerExample".into(),
                backing_clock_id: "12D3KooWPeerExample/session-123/monotonic".into(),
                backing_clock_hash: "abc123".into(),
                backing_to_domain_offset_ns: 0,
            }),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"domain_clock\":"));
        assert!(json.contains("\"cluster_name\":"));
        assert!(json.contains("\"domain_clock_id\":"));
        assert!(json.contains("\"domain_clock_hash\":"));
        assert!(json.contains("\"backing_peer_id\":"));
        assert!(json.contains("\"backing_clock_id\":"));
        assert!(json.contains("\"backing_clock_hash\":"));
        assert!(json.contains("\"backing_to_domain_offset_ns\":"));
        assert!(!json.contains("\"generation\":"));
        assert!(!json.contains("\"source_epoch\":"));
    }
}
