//! `/auki/join/0.0.1` — libp2p protocol for a peer to request
//! admission into a cluster.
//!
//! ## Shape
//!
//! One substream per join attempt. Joiner opens the substream, writes
//! a [`JoinRequest`], reads a [`JoinResponse`], and closes. The
//! Manager accepts the substream, reads the request, computes a
//! response (admit + serialize membership, or reject), writes the
//! response, and closes.
//!
//! ## Wire format
//!
//! Length-prefixed JSON (4-byte big-endian length, then UTF-8 JSON
//! bytes). `MAX_JOIN_FRAME_BYTES` caps either side at 1 MiB —
//! membership documents for the demo's 3-peer cluster are well under
//! 1 KiB; even a 1000-peer cluster would fit comfortably.
//!
//! ## Trust boundary
//!
//! Inbound substreams from peers NOT on the runtime's allow-list
//! never reach this protocol — libp2p's `allow_block_list` denies
//! the noise handshake. The first time a peer can speak this
//! protocol to a Manager is after the Manager has added them to the
//! allow-list, which the Manager does as part of an `admit_peer`
//! during the previous successful join. **First-peer paradox**: the
//! initial joiner of a brand-new cluster needs to bypass the
//! allow-list to reach the Manager. For Hagall v1 we accept this by
//! having `ClusterManager::join_cluster` pre-allow the Manager's
//! peer-id in the runtime's allow-list before dialing, scoped only
//! to this single inbound flow. See `auki-domain`'s
//! `cluster_manager::join_cluster` for the orchestration.

use futures::{AsyncReadExt, AsyncWriteExt};
use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// libp2p protocol id for the join handshake. Stable; bump version
/// only on an incompatible wire-shape change.
pub const JOIN_PROTOCOL: &str = "/auki/join/0.0.1";

/// Cap on a single framed JoinRequest / JoinResponse. 1 MiB — JSON
/// membership documents are well under 1 KiB at demo scale and ~10
/// KiB at a thousand peers; the cap is for defense against malformed
/// senders.
pub const MAX_JOIN_FRAME_BYTES: u32 = 1024 * 1024;

/// Body of an inbound or outbound join request. Sent by the joining
/// peer over `/auki/join/0.0.1`.
///
/// The joiner's `PeerId` is **not** in the wire body — libp2p's
/// noise handshake already authenticated it at the connection
/// layer, and the protocol picks it up from
/// `libp2p::Stream`'s metadata. Putting it in the body would create
/// two sources of truth that could disagree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JoinRequest {
    /// Dialable multiaddrs the joiner advertises. The Manager
    /// records these in the cluster membership so other peers can
    /// dial back.
    #[serde(with = "multiaddr_vec_serde")]
    pub multiaddrs: Vec<Multiaddr>,
}

/// Body of an outbound or inbound join response. Sent by the Manager
/// over `/auki/join/0.0.1`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JoinResponse {
    /// Manager accepted the join. Carries the full cluster
    /// membership document (JSON-encoded; consumers parse it with
    /// `auki_domain::ClusterMembership::from_json` or a direct
    /// `serde_json::from_str`) and an opaque successor token.
    ///
    /// The membership is sent as a JSON string to keep this module
    /// independent of `auki-domain`'s `ClusterMembership` type —
    /// `auki-network` doesn't depend on `auki-domain`. The cost is
    /// a double JSON encoding pass; the win is a clean layering.
    Accept {
        /// Serialized `ClusterMembership` (JSON).
        membership_json: String,
        /// Opaque successor token. v1: empty bytes (signature
        /// verification disabled per Discovery v1 contract).
        #[serde(with = "serde_bytes_or_empty")]
        successor_token: Vec<u8>,
    },
    /// Manager refused the join. Human-readable reason; daemons log
    /// + surface; consumers may retry or pick a different cluster.
    Reject {
        /// Free-form reason. Conventional strings include
        /// `"not the manager"`, `"already a member"`, `"cluster
        /// full"`. No machine-readable enum yet — daemons match by
        /// substring or just log and bail.
        reason: String,
    },
}

/// Failure modes for [`read_join_request`] / [`read_join_response`] /
/// [`write_join_request`] / [`write_join_response`].
#[derive(Debug, Error)]
pub enum JoinProtocolError {
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
    /// Length prefix exceeds [`MAX_JOIN_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write a [`JoinRequest`] to `stream`, length-prefixed JSON.
pub async fn write_join_request<S>(
    stream: &mut S,
    msg: &JoinRequest,
) -> Result<(), JoinProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Write a [`JoinResponse`] to `stream`, length-prefixed JSON.
pub async fn write_join_response<S>(
    stream: &mut S,
    msg: &JoinResponse,
) -> Result<(), JoinProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Read a [`JoinRequest`] from `stream`. End-of-stream surfaces as
/// `Err(JoinProtocolError::Io(_))` with `kind() == UnexpectedEof`.
pub async fn read_join_request<S>(stream: &mut S) -> Result<JoinRequest, JoinProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

/// Read a [`JoinResponse`] from `stream`.
pub async fn read_join_response<S>(stream: &mut S) -> Result<JoinResponse, JoinProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

async fn write_json<S, T>(stream: &mut S, msg: &T) -> Result<(), JoinProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(msg).map_err(JoinProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_JOIN_FRAME_BYTES as u64 {
        return Err(JoinProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_JOIN_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(JoinProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(JoinProtocolError::Io)?;
    stream.flush().await.map_err(JoinProtocolError::Io)?;
    Ok(())
}

async fn read_json<S, T>(stream: &mut S) -> Result<T, JoinProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(JoinProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(JoinProtocolError::EmptyFrame);
    }
    if len > MAX_JOIN_FRAME_BYTES {
        return Err(JoinProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_JOIN_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(JoinProtocolError::Io)?;
    serde_json::from_slice(&payload).map_err(JoinProtocolError::Decode)
}

mod multiaddr_vec_serde {
    use multiaddr::Multiaddr;
    use serde::{Deserialize, Deserializer, Serializer, ser::SerializeSeq};
    use std::str::FromStr;

    pub fn serialize<S: Serializer>(addrs: &Vec<Multiaddr>, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(addrs.len()))?;
        for a in addrs {
            seq.serialize_element(&a.to_string())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Multiaddr>, D::Error> {
        let strs: Vec<String> = Vec::deserialize(d)?;
        strs.into_iter()
            .map(|s| {
                Multiaddr::from_str(&s)
                    .map_err(|e| serde::de::Error::custom(format!("multiaddr: parse {s:?}: {e}")))
            })
            .collect()
    }
}

/// Serialize `Vec<u8>` as an array of u8 (JSON's array-of-numbers).
/// Empty vec round-trips as `[]`. v2 will swap this for a typed token
/// shape (per SDK-Q3); for v1 the bytes are opaque and the array form
/// is the simplest cross-language wire.
mod serde_bytes_or_empty {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(bytes.iter())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        Vec::<u8>::deserialize(d)
    }
}

// Used only for ergonomic construction below.
#[allow(dead_code)]
fn _ensure_multiaddr_str_compat() {
    let _ = Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").expect("smoke");
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> JoinRequest {
        JoinRequest {
            multiaddrs: vec![
                "/ip4/192.168.1.10/tcp/4001".parse().unwrap(),
                "/ip4/192.168.1.10/udp/4001/quic-v1".parse().unwrap(),
            ],
        }
    }

    fn sample_accept_response() -> JoinResponse {
        JoinResponse::Accept {
            membership_json: r#"{"cluster_name":"foo","peers":[]}"#.to_string(),
            successor_token: vec![0xde, 0xad, 0xbe, 0xef],
        }
    }

    fn sample_reject_response() -> JoinResponse {
        JoinResponse::Reject {
            reason: "not the manager".to_string(),
        }
    }

    #[tokio::test]
    async fn join_request_round_trips_through_framed_stream() {
        let req = sample_request();
        let mut buf = Vec::new();
        write_json(&mut buf, &req).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back: JoinRequest = read_json(&mut cursor).await.unwrap();
        assert_eq!(req, back);
    }

    #[tokio::test]
    async fn join_response_accept_round_trips() {
        let resp = sample_accept_response();
        let mut buf = Vec::new();
        write_json(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back: JoinResponse = read_json(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    #[tokio::test]
    async fn join_response_reject_round_trips() {
        let resp = sample_reject_response();
        let mut buf = Vec::new();
        write_json(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back: JoinResponse = read_json(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    #[tokio::test]
    async fn read_rejects_zero_length() {
        let buf = vec![0u8; 4]; // length prefix == 0
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_json::<_, JoinRequest>(&mut cursor).await.unwrap_err();
        assert!(matches!(err, JoinProtocolError::EmptyFrame));
    }

    #[tokio::test]
    async fn read_rejects_oversized_length_prefix() {
        let mut buf = (MAX_JOIN_FRAME_BYTES + 1).to_be_bytes().to_vec();
        buf.extend(std::iter::repeat(0u8).take(8));
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_json::<_, JoinRequest>(&mut cursor).await.unwrap_err();
        assert!(matches!(err, JoinProtocolError::FrameTooLarge { .. }));
    }

    /// Pins the on-wire JSON shape of `JoinResponse` against rename.
    /// Cross-language consumers (Python via `auki-domain-py`, future
    /// TS/Swift clients) parse responses by these exact keys.
    #[test]
    fn wire_shape_locked_field_names() {
        let accept = serde_json::to_string(&sample_accept_response()).unwrap();
        let reject = serde_json::to_string(&sample_reject_response()).unwrap();
        // Accept variant
        assert!(accept.contains(r#""kind":"accept""#), "{accept}");
        assert!(accept.contains(r#""membership_json":"#), "{accept}");
        assert!(accept.contains(r#""successor_token":"#), "{accept}");
        // Reject variant
        assert!(reject.contains(r#""kind":"reject""#), "{reject}");
        assert!(reject.contains(r#""reason":"#), "{reject}");
    }
}
