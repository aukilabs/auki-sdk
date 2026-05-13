//! `/auki/info/0.0.1` — libp2p protocol for fetching a peer's
//! [`ParticipantInfo`] over the cluster's libp2p plane.
//!
//! ## Why this exists
//!
//! Cluster peers need to render each other in operator UIs (Park's
//! directory, Sentinel's status board). The Hagall cluster doc
//! ([`auki-domain::ClusterMembership`](../../auki-domain)) carries
//! only `peer_id + multiaddrs + join_ts_ns + successor_token` — it
//! intentionally does NOT carry per-peer `ParticipantInfo` because
//! that's volatile (`session_now_ns` advances every call,
//! `cluster_joined_at_ns` is lazy, etc.).
//!
//! Pre-Hagall, Park resolved daemon HTTP base URLs via mDNS and
//! hit `GET /api/info` to render the directory. Hagall's
//! constraint #6 rules out mDNS / HTTP-between-cluster-peers as
//! side channels — peer-to-peer information exchange must ride on
//! libp2p so the cluster's trust boundary is the only one a peer's
//! identity flows through.
//!
//! ## Shape
//!
//! Request-response over one substream. Client opens, writes
//! [`InfoRequest`] (currently empty — future fields might carry
//! `since_session_now_ns` for delta updates), reads
//! [`InfoResponse`], closes.
//!
//! ```text
//! Initiator → Responder:  InfoRequest {}
//! Responder → Initiator:  InfoResponse { participant_info_json: ... }
//! ```
//!
//! ## Trust boundary
//!
//! Inbound substreams from peers NOT on the runtime's allow-list
//! are silently dropped at the runtime layer, identically to
//! `/auki/stream/0.1.0` and `/auki/membership/0.0.1`. Non-cluster
//! peers cannot fetch identity info — privacy by membership.
//!
//! ## Wire format
//!
//! Length-prefixed JSON, same framing as the other Hagall
//! protocols. [`MAX_INFO_FRAME_BYTES`] caps each side at 64 KiB —
//! `ParticipantInfo` is well under 1 KiB; the cap is defense
//! against malformed senders.

use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// libp2p protocol id for the `/api/info` equivalent over libp2p.
/// Stable; bump version only on an incompatible wire-shape change.
pub const INFO_PROTOCOL: &str = "/auki/info/0.0.1";

/// Cap on a single framed message. 64 KiB — `ParticipantInfo` is
/// under 1 KiB at demo scale; the cap is defense against malformed
/// senders.
pub const MAX_INFO_FRAME_BYTES: u32 = 64 * 1024;

/// Body of the request the initiator sends. Currently empty —
/// reserved for future delta-fetching fields like
/// `since_session_now_ns`. Receivers MUST tolerate unknown future
/// fields (serde JSON is permissive by default).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InfoRequest {}

/// Body of the response the responder sends. Carries a serialized
/// [`crate::ParticipantInfo`] JSON string — same shape served on
/// HTTP `/api/info`, same JSON the membership-gossip protocol
/// already passes around for the membership doc. Cross-language
/// consumers parse with their own JSON decoder.
///
/// `participant_info_json` is a [`String`] (not a typed Rust struct)
/// so this module stays independent of the [`crate::ParticipantInfo`]
/// type's exact field set — consumers can add fields without
/// touching this protocol's wire shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InfoResponse {
    /// Serialized `auki_network::ParticipantInfo` (JSON).
    pub participant_info_json: String,
}

/// Failure modes for the framed read/write helpers below.
#[derive(Debug, Error)]
pub enum InfoProtocolError {
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
    /// Length prefix exceeds [`MAX_INFO_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write an [`InfoRequest`] to `stream`, length-prefixed JSON.
pub async fn write_info_request<S>(
    stream: &mut S,
    msg: &InfoRequest,
) -> Result<(), InfoProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Write an [`InfoResponse`] to `stream`, length-prefixed JSON.
pub async fn write_info_response<S>(
    stream: &mut S,
    msg: &InfoResponse,
) -> Result<(), InfoProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Read an [`InfoRequest`] from `stream`.
pub async fn read_info_request<S>(stream: &mut S) -> Result<InfoRequest, InfoProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

/// Read an [`InfoResponse`] from `stream`.
pub async fn read_info_response<S>(stream: &mut S) -> Result<InfoResponse, InfoProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

async fn write_json<S, T>(stream: &mut S, msg: &T) -> Result<(), InfoProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(msg).map_err(InfoProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_INFO_FRAME_BYTES as u64 {
        return Err(InfoProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_INFO_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(InfoProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(InfoProtocolError::Io)?;
    stream.flush().await.map_err(InfoProtocolError::Io)?;
    Ok(())
}

async fn read_json<S, T>(stream: &mut S) -> Result<T, InfoProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(InfoProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(InfoProtocolError::EmptyFrame);
    }
    if len > MAX_INFO_FRAME_BYTES {
        return Err(InfoProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_INFO_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(InfoProtocolError::Io)?;
    serde_json::from_slice(&payload).map_err(InfoProtocolError::Decode)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_round_trips() {
        let req = InfoRequest::default();
        let mut buf = Vec::new();
        write_info_request(&mut buf, &req).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_info_request(&mut cursor).await.unwrap();
        assert_eq!(req, back);
    }

    #[tokio::test]
    async fn response_round_trips() {
        let resp = InfoResponse {
            participant_info_json: r#"{"app":"boosterapp","name":"k1-walker","peer_id":"12D3KooWA","is_manager":false,"manager_peer_id":"12D3KooWB"}"#.to_string(),
        };
        let mut buf = Vec::new();
        write_info_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_info_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    #[tokio::test]
    async fn read_rejects_oversized_length_prefix() {
        let mut buf = (MAX_INFO_FRAME_BYTES + 1).to_be_bytes().to_vec();
        buf.extend(std::iter::repeat(0u8).take(8));
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_info_response(&mut cursor).await.unwrap_err();
        assert!(matches!(err, InfoProtocolError::FrameTooLarge { .. }));
    }

    /// Pins the on-wire JSON key. Cross-language consumers (Python
    /// via `auki-domain-py`, future TS/Swift) parse by this exact name.
    #[test]
    fn wire_shape_locked_field_name() {
        let json = serde_json::to_string(&InfoResponse {
            participant_info_json: "abc".into(),
        })
        .unwrap();
        assert!(json.contains(r#""participant_info_json":"#), "{json}");
    }

    /// Future-compat: an InfoRequest with extra fields decodes
    /// cleanly into today's empty struct (serde ignores unknown
    /// fields). Lets us add `since_session_now_ns` later without a
    /// protocol-id bump.
    #[test]
    fn request_decodes_with_future_unknown_fields() {
        let forward_json = r#"{"since_session_now_ns":12345}"#;
        let back: InfoRequest = serde_json::from_str(forward_json).unwrap();
        assert_eq!(back, InfoRequest::default());
    }
}
