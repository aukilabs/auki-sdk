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
//! Length-prefixed protobuf, same framing as the other Hagall
//! protocols. [`MAX_INFO_FRAME_BYTES`] caps each side at 64 KiB —
//! `ParticipantInfo` is well under 1 KiB; the cap is defense
//! against malformed senders.

use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p_identity::PeerId;
use prost::Message;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use auki_datatypes::info::{InfoRequest, InfoResponse};

/// libp2p protocol id for the peer-to-peer `ParticipantInfo`
/// exchange — the canonical (and only) peer-facing identity surface
/// (#293). Stable; bump version only on an incompatible wire-shape
/// change.
pub const INFO_PROTOCOL: &str = "/auki/info/0.0.1";

/// Cap on a single framed message. 64 KiB — `ParticipantInfo` is
/// under 1 KiB at demo scale; the cap is defense against malformed
/// senders.
pub const MAX_INFO_FRAME_BYTES: u32 = 64 * 1024;

/// Participant metadata served by authenticated info protocol version 1.0.0.
///
/// Authentication supplies Domain membership and authority. This payload is
/// diagnostic application/session metadata only, so it intentionally has no
/// Manager, membership, authorization-role, or route fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedParticipantInfo {
    /// Application identifier, such as `boosterapp` or `sentinel`.
    pub app: String,
    /// Application version reported by the participant.
    pub app_version: String,
    /// Operator-friendly participant label.
    pub name: String,
    /// Identifier minted for this application session.
    pub session_id: String,
    /// Identifier of the session's local clock descriptor.
    pub session_clock_id: String,
    /// Content hash pinning that clock descriptor.
    pub session_clock_hash: String,
    /// Current value of the session-local clock.
    pub session_now_ns: u64,
    /// Noise-authenticated libp2p identity repeated for diagnostics.
    pub peer_id: PeerId,
    /// Stable local application-instance label, when the host has one.
    pub app_instance: String,
}

/// Failure modes for the framed read/write helpers below.
#[derive(Debug, Error)]
pub enum InfoProtocolError {
    /// Underlying I/O on the libp2p substream failed.
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    /// Protobuf encode (write side) failed.
    #[error("encode: {0}")]
    Encode(#[source] prost::EncodeError),
    /// Protobuf decode (read side) failed.
    #[error("decode: {0}")]
    Decode(#[source] prost::DecodeError),
    /// The typed authenticated participant document is not valid JSON.
    #[error("participant info json: {0}")]
    Json(#[source] serde_json::Error),
    /// Length prefix exceeds [`MAX_INFO_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write an authenticated info 1.0.0 response using the existing bounded
/// protobuf wrapper and the adapted typed JSON document.
pub async fn write_authenticated_info_response<S>(
    stream: &mut S,
    info: &AuthenticatedParticipantInfo,
) -> Result<(), InfoProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let response = InfoResponse {
        participant_info_json: serde_json::to_string(info).map_err(InfoProtocolError::Json)?,
    };
    write_info_response(stream, &response).await
}

/// Read and validate an authenticated info 1.0.0 response.
pub async fn read_authenticated_info_response<S>(
    stream: &mut S,
) -> Result<AuthenticatedParticipantInfo, InfoProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let response = read_info_response(stream).await?;
    serde_json::from_str(&response.participant_info_json).map_err(InfoProtocolError::Json)
}

/// Write an [`InfoRequest`] to `stream`, length-prefixed protobuf.
pub async fn write_info_request<S>(
    stream: &mut S,
    msg: &InfoRequest,
) -> Result<(), InfoProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_frame(stream, msg).await
}

/// Write an [`InfoResponse`] to `stream`, length-prefixed protobuf.
pub async fn write_info_response<S>(
    stream: &mut S,
    msg: &InfoResponse,
) -> Result<(), InfoProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_frame(stream, msg).await
}

/// Read an [`InfoRequest`] from `stream`.
pub async fn read_info_request<S>(stream: &mut S) -> Result<InfoRequest, InfoProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_frame(stream).await
}

/// Read an [`InfoResponse`] from `stream`.
pub async fn read_info_response<S>(stream: &mut S) -> Result<InfoResponse, InfoProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_frame(stream).await
}

async fn write_frame<S, T>(stream: &mut S, msg: &T) -> Result<(), InfoProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Message,
{
    let mut bytes = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut bytes).map_err(InfoProtocolError::Encode)?;
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

async fn read_frame<S, T>(stream: &mut S) -> Result<T, InfoProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: Message + Default,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(InfoProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
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
    T::decode(&*payload).map_err(InfoProtocolError::Decode)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn authenticated_fixture() -> AuthenticatedParticipantInfo {
        AuthenticatedParticipantInfo {
            app: "boosterapp".into(),
            app_version: "1.2.3".into(),
            name: "k1".into(),
            session_id: "s1".into(),
            session_clock_id: "clock".into(),
            session_clock_hash: "hash".into(),
            session_now_ns: 7,
            peer_id: "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw"
                .parse()
                .unwrap(),
            app_instance: "device".into(),
        }
    }

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
    async fn info_response_payload_is_protobuf_not_json() {
        let resp = InfoResponse {
            participant_info_json: r#"{"peer_id":"12D3KooWA"}"#.to_string(),
        };
        let mut buf = Vec::new();
        write_info_response(&mut buf, &resp).await.unwrap();

        assert_ne!(
            buf.get(4),
            Some(&b'{'),
            "info protocol payload must be generated protobuf, not JSON"
        );
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
    async fn authenticated_info_v1_framed_bytes_are_explicit_and_manager_free() {
        const EXPECTED_JSON: &str = r#"{"app":"boosterapp","app_version":"1.2.3","name":"k1","session_id":"s1","session_clock_id":"clock","session_clock_hash":"hash","session_now_ns":7,"peer_id":"12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw","app_instance":"device"}"#;
        let mut expected = vec![0x00, 0x00, 0x00, 0xee, 0x0a, 0xeb, 0x01];
        expected.extend_from_slice(EXPECTED_JSON.as_bytes());

        let mut bytes = Vec::new();
        write_authenticated_info_response(&mut bytes, &authenticated_fixture())
            .await
            .unwrap();
        assert_eq!(bytes, expected);

        let decoded = read_authenticated_info_response(&mut futures::io::Cursor::new(bytes))
            .await
            .unwrap();
        assert_eq!(decoded, authenticated_fixture());
        assert!(!EXPECTED_JSON.contains("manager"));
        assert!(!EXPECTED_JSON.contains("route"));
        assert!(!EXPECTED_JSON.contains("role"));
    }

    #[test]
    fn authenticated_info_v1_rejects_manager_era_fields() {
        let mut value = serde_json::to_value(authenticated_fixture()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("is_manager".into(), serde_json::Value::Bool(false));
        assert!(serde_json::from_value::<AuthenticatedParticipantInfo>(value).is_err());
    }

    #[tokio::test]
    async fn read_rejects_oversized_length_prefix() {
        let mut buf = (MAX_INFO_FRAME_BYTES + 1).to_be_bytes().to_vec();
        buf.extend(std::iter::repeat_n(0u8, 8));
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_info_response(&mut cursor).await.unwrap_err();
        assert!(matches!(err, InfoProtocolError::FrameTooLarge { .. }));
    }

    #[test]
    fn generated_response_field_number_is_locked() {
        let bytes = InfoResponse {
            participant_info_json: "abc".into(),
        }
        .encode_to_vec();
        assert_eq!(bytes, vec![0x0a, 0x03, b'a', b'b', b'c']);
    }
}
