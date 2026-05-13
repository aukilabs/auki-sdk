//! `/auki/membership/0.0.1` — libp2p protocol for the Manager to push
//! cluster membership updates to peers.
//!
//! ## Shape
//!
//! One substream per push. Manager opens the substream, writes a
//! single [`MembershipUpdate`] message (length-prefixed JSON, same
//! framing as [`crate::join_protocol`]), closes. No response — this
//! is fire-and-forget gossip; the receiver's accept-and-apply
//! semantics make this safe.
//!
//! ## Triggers (Manager-side)
//!
//! The Manager (an [`auki-domain`](../../auki-domain) `ClusterManager`
//! whose `is_manager == true`) calls
//! [`crate::network_runtime::NetworkRuntime::broadcast_membership`] in
//! three cases:
//!
//! 1. After [`admit_peer`](../../auki-domain/src/cluster_manager.rs)
//!    succeeds — broadcast the freshly-extended membership to all
//!    currently-allow-listed peers EXCEPT the new joiner (who got the
//!    same JSON in `JoinResponse::Accept`).
//! 2. When the Manager evicts a peer (heartbeat-loss path from
//!    `spawn_liveness_handler`) — broadcast the shrunken membership.
//! 3. On Manager promotion (election winner in `spawn_liveness_handler`)
//!    — broadcast the new Manager's view so survivors converge on the
//!    new Manager identity.
//!
//! ## Receive-side application
//!
//! On inbound, the receiver's `ClusterManager` parses the JSON,
//! atomically swaps its local [`ClusterMembership`](../../auki-domain),
//! and pushes the updated allow-list to its runtime via
//! [`crate::network_runtime::NetworkRuntime::set_allowed_peers`]. The
//! `manager_peer_id` field is honored — if the gossip carries a
//! different Manager than the receiver currently believes in, the
//! receiver updates (handoff convergence).
//!
//! ## Trust boundary
//!
//! Inbound substreams from peers NOT on the runtime's allow-list are
//! silently dropped at the runtime layer, identically to
//! `/auki/stream/0.1.0`. Non-cluster peers cannot inject membership
//! updates.
//!
//! ## Wire format
//!
//! Length-prefixed JSON (4-byte big-endian length, then UTF-8 JSON
//! bytes). [`MAX_MEMBERSHIP_FRAME_BYTES`] caps the frame at 1 MiB —
//! membership documents for the demo's 3-peer cluster are well under
//! 1 KiB; even a 1000-peer cluster fits comfortably.
//!
//! ## Last-write-wins convergence
//!
//! There is no version vector, no sequence number, no causal
//! ordering. Receivers apply gossip in arrival order — last write
//! wins. With a single Manager broadcasting, this is correct by
//! construction (the Manager is the source of truth). During
//! Manager-handoff windows two peers may briefly both broadcast as
//! Manager; the survivors converge on whichever broadcast arrived
//! last, and Discovery's directory entry breaks the tie for any
//! newcomer. Per SDK-Q5's "long-term direction: cluster doc as a
//! DHT" — v1 keeps the simple shape.

use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// libp2p protocol id for the membership-gossip stream. Stable; bump
/// version only on an incompatible wire-shape change.
pub const MEMBERSHIP_PROTOCOL: &str = "/auki/membership/0.0.1";

/// Cap on a single framed [`MembershipUpdate`]. 1 MiB — JSON
/// membership documents are well under 1 KiB at demo scale; the cap
/// is defense against malformed senders.
pub const MAX_MEMBERSHIP_FRAME_BYTES: u32 = 1024 * 1024;

/// Body of a membership broadcast. The Manager fills `membership_json`
/// with the serialized
/// [`auki_domain::ClusterMembership`](../../auki-domain) — the same
/// JSON shape `JoinResponse::Accept` already carries.
///
/// `membership_json` is a [`String`] (not a typed Rust struct) so
/// this module stays independent of `auki-domain`'s
/// `ClusterMembership` type. Cost: a double JSON encoding pass. Win:
/// clean layering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MembershipUpdate {
    /// Serialized `auki_domain::ClusterMembership` (JSON).
    pub membership_json: String,
}

/// Failure modes for [`read_membership_update`] /
/// [`write_membership_update`].
#[derive(Debug, Error)]
pub enum MembershipProtocolError {
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
    /// Length prefix exceeds [`MAX_MEMBERSHIP_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write a [`MembershipUpdate`] to `stream`, length-prefixed JSON.
pub async fn write_membership_update<S>(
    stream: &mut S,
    msg: &MembershipUpdate,
) -> Result<(), MembershipProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let bytes = serde_json::to_vec(msg).map_err(MembershipProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_MEMBERSHIP_FRAME_BYTES as u64 {
        return Err(MembershipProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_MEMBERSHIP_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(MembershipProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(MembershipProtocolError::Io)?;
    stream
        .flush()
        .await
        .map_err(MembershipProtocolError::Io)?;
    Ok(())
}

/// Read a [`MembershipUpdate`] from `stream`. End-of-stream surfaces
/// as `Err(MembershipProtocolError::Io(_))` with `kind() ==
/// UnexpectedEof`.
pub async fn read_membership_update<S>(
    stream: &mut S,
) -> Result<MembershipUpdate, MembershipProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(MembershipProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(MembershipProtocolError::EmptyFrame);
    }
    if len > MAX_MEMBERSHIP_FRAME_BYTES {
        return Err(MembershipProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_MEMBERSHIP_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(MembershipProtocolError::Io)?;
    serde_json::from_slice(&payload).map_err(MembershipProtocolError::Decode)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_update() -> MembershipUpdate {
        MembershipUpdate {
            membership_json: r#"{"cluster_name":"foo","peers":[{"peer_id":"12D3KooWA","multiaddrs":["/ip4/127.0.0.1/tcp/4001"],"join_ts_ns":1000,"successor_token":[]}]}"#.to_string(),
        }
    }

    #[tokio::test]
    async fn round_trips_through_framed_stream() {
        let msg = sample_update();
        let mut buf = Vec::new();
        write_membership_update(&mut buf, &msg).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_membership_update(&mut cursor).await.unwrap();
        assert_eq!(msg, back);
    }

    #[tokio::test]
    async fn read_rejects_zero_length() {
        let buf = vec![0u8; 4]; // length prefix == 0
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_membership_update(&mut cursor).await.unwrap_err();
        assert!(matches!(err, MembershipProtocolError::EmptyFrame));
    }

    #[tokio::test]
    async fn read_rejects_oversized_length_prefix() {
        let mut buf = (MAX_MEMBERSHIP_FRAME_BYTES + 1).to_be_bytes().to_vec();
        buf.extend(std::iter::repeat(0u8).take(8));
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_membership_update(&mut cursor).await.unwrap_err();
        assert!(matches!(err, MembershipProtocolError::FrameTooLarge { .. }));
    }

    /// Pins the on-wire JSON shape against rename. Cross-language
    /// consumers (Python via `auki-domain-py`, future TS/Swift)
    /// parse by this exact key.
    #[test]
    fn wire_shape_locked_field_name() {
        let json = serde_json::to_string(&sample_update()).unwrap();
        assert!(json.contains(r#""membership_json":"#), "{json}");
    }

    /// An empty membership_json string serializes through cleanly.
    /// Edge case for the Manager-handoff broadcast when a Manager
    /// promotes itself in a one-peer (just-itself) cluster.
    #[test]
    fn empty_membership_json_round_trips() {
        let msg = MembershipUpdate {
            membership_json: String::new(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: MembershipUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }
}
