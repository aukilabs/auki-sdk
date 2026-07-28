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
//! Length-prefixed protobuf (4-byte big-endian length, then the
//! prost-encoded message). `MAX_JOIN_FRAME_BYTES` caps either side at 1 MiB —
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
use prost::Message;
use std::str::FromStr;
use thiserror::Error;

/// libp2p protocol id for the join handshake. Stable; bump version
/// only on an incompatible wire-shape change.
pub const JOIN_PROTOCOL: &str = "/auki/join/0.0.1";

/// Cap on a single framed JoinRequest / JoinResponse. 1 MiB — protobuf
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequest {
    /// Dialable multiaddrs the joiner advertises. The Manager
    /// records these in the cluster membership so other peers can
    /// dial back.
    pub multiaddrs: Vec<Multiaddr>,
    /// Full HTTP `Authorization` header value (e.g. `Bearer <token>`).
    /// Empty string means absent.
    pub authorization: String,
}

/// Body of an outbound or inbound join response. Sent by the Manager
/// over `/auki/join/0.0.1`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Protobuf encode (write side) failed.
    #[error("encode: {0}")]
    Encode(#[source] prost::EncodeError),
    /// Protobuf decode (read side) failed.
    #[error("decode: {0}")]
    Decode(#[source] prost::DecodeError),
    /// Peer sent a multiaddr string that does not parse.
    #[error("invalid multiaddr {value:?}: {error}")]
    InvalidMultiaddr { value: String, error: String },
    /// Peer sent a `JoinResponse` with no accept/reject variant.
    #[error("join response has no kind set")]
    MissingVariant,
    /// Length prefix exceeds [`MAX_JOIN_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write a [`JoinRequest`] to `stream`, length-prefixed protobuf.
pub async fn write_join_request<S>(
    stream: &mut S,
    msg: &JoinRequest,
) -> Result<(), JoinProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_frame(stream, &proto_from_join_request(msg)).await
}

/// Write a [`JoinResponse`] to `stream`, length-prefixed protobuf.
pub async fn write_join_response<S>(
    stream: &mut S,
    msg: &JoinResponse,
) -> Result<(), JoinProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_frame(stream, &proto_from_join_response(msg)).await
}

/// Read a [`JoinRequest`] from `stream`. End-of-stream surfaces as
/// `Err(JoinProtocolError::Io(_))` with `kind() == UnexpectedEof`.
pub async fn read_join_request<S>(stream: &mut S) -> Result<JoinRequest, JoinProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let proto: auki_datatypes::join::JoinRequest = read_frame(stream).await?;
    join_request_from_proto(proto)
}

/// Read a [`JoinResponse`] from `stream`.
pub async fn read_join_response<S>(stream: &mut S) -> Result<JoinResponse, JoinProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let proto: auki_datatypes::join::JoinResponse = read_frame(stream).await?;
    join_response_from_proto(proto)
}

fn proto_from_join_request(msg: &JoinRequest) -> auki_datatypes::join::JoinRequest {
    auki_datatypes::join::JoinRequest {
        multiaddrs: msg.multiaddrs.iter().map(ToString::to_string).collect(),
        authorization: msg.authorization.clone(),
    }
}

fn proto_from_join_response(msg: &JoinResponse) -> auki_datatypes::join::JoinResponse {
    match msg {
        JoinResponse::Accept {
            membership_json,
            successor_token,
        } => auki_datatypes::join::JoinResponse::accept(
            membership_json.clone(),
            successor_token.clone(),
        ),
        JoinResponse::Reject { reason } => auki_datatypes::join::JoinResponse::reject(reason),
    }
}

fn join_request_from_proto(
    proto: auki_datatypes::join::JoinRequest,
) -> Result<JoinRequest, JoinProtocolError> {
    let multiaddrs = proto
        .multiaddrs
        .into_iter()
        .map(|value| {
            Multiaddr::from_str(&value).map_err(|error| JoinProtocolError::InvalidMultiaddr {
                value,
                error: error.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(JoinRequest {
        multiaddrs,
        authorization: proto.authorization,
    })
}

fn join_response_from_proto(
    proto: auki_datatypes::join::JoinResponse,
) -> Result<JoinResponse, JoinProtocolError> {
    match proto.kind.ok_or(JoinProtocolError::MissingVariant)? {
        auki_datatypes::join::join_response::Kind::Accept(accept) => Ok(JoinResponse::Accept {
            membership_json: accept.membership_json,
            successor_token: accept.successor_token,
        }),
        auki_datatypes::join::join_response::Kind::Reject(reject) => Ok(JoinResponse::Reject {
            reason: reject.reason,
        }),
    }
}

async fn write_frame<S, T>(stream: &mut S, msg: &T) -> Result<(), JoinProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Message,
{
    let mut bytes = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut bytes).map_err(JoinProtocolError::Encode)?;
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

async fn read_frame<S, T>(stream: &mut S) -> Result<T, JoinProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: Message + Default,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(JoinProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
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
    T::decode(&*payload).map_err(JoinProtocolError::Decode)
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
            authorization: "Bearer test-token".into(),
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
        write_join_request(&mut buf, &req).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_join_request(&mut cursor).await.unwrap();
        assert_eq!(req, back);
    }

    #[tokio::test]
    async fn join_request_payload_is_protobuf_not_json() {
        let req = sample_request();
        let mut buf = Vec::new();
        write_join_request(&mut buf, &req).await.unwrap();

        assert_ne!(
            buf.get(4),
            Some(&b'{'),
            "join protocol payload must be generated protobuf, not JSON"
        );
    }

    #[tokio::test]
    async fn join_response_accept_round_trips() {
        let resp = sample_accept_response();
        let mut buf = Vec::new();
        write_join_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_join_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    #[tokio::test]
    async fn join_response_reject_round_trips() {
        let resp = sample_reject_response();
        let mut buf = Vec::new();
        write_join_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_join_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    #[tokio::test]
    async fn zero_length_request_decodes_as_empty_protobuf_message() {
        let buf = vec![0u8; 4]; // length prefix == 0
        let mut cursor = futures::io::Cursor::new(buf);
        let request = read_join_request(&mut cursor).await.unwrap();
        assert!(request.multiaddrs.is_empty());
    }

    #[tokio::test]
    async fn read_rejects_oversized_length_prefix() {
        let mut buf = (MAX_JOIN_FRAME_BYTES + 1).to_be_bytes().to_vec();
        buf.extend(std::iter::repeat(0u8).take(8));
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_join_request(&mut cursor).await.unwrap_err();
        assert!(matches!(err, JoinProtocolError::FrameTooLarge { .. }));
    }

    #[test]
    fn generated_accept_response_uses_expected_fields() {
        let proto = proto_from_join_response(&sample_accept_response());
        let auki_datatypes::join::join_response::Kind::Accept(accept) =
            proto.kind.expect("accept variant")
        else {
            panic!("expected accept variant");
        };
        assert_eq!(
            accept.membership_json,
            r#"{"cluster_name":"foo","peers":[]}"#
        );
        assert_eq!(accept.successor_token, vec![0xde, 0xad, 0xbe, 0xef]);
    }
}
