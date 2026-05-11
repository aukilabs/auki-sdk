//! `/auki/registry/0.0.1` — libp2p stream protocol for Manager→members
//! Cluster Registry snapshot broadcast.
//!
//! Greenland T6 + T7. The Manager of a cluster publishes a fresh full
//! [`ClusterDoc`] snapshot to every member on every registry mutation
//! (join, depart, capability update, endpoint rotation). Members
//! receive the snapshot, update their local view, and use it to drive
//! their own connection state.
//!
//! ## Why direct Manager→members over libp2p, not Discovery SSE
//!
//! The earlier Greenland framing routed snapshots through Discovery's
//! existing SSE channel from
//! [PR #84](https://github.com/aukilabs/auki-sdk/pull/84). The
//! Q-disc-1 resolution (2026-05-11) inverted this: Discovery is
//! bootstrap rendezvous only; live registry state flows peer-to-peer
//! over libp2p, alongside the heartbeat liveness signal. Layering a
//! parallel Discovery-pushed snapshot stream on top of the
//! already-peer-to-peer heartbeat would create two live-state
//! surfaces with their own reconciliation problems. Single libp2p
//! surface is cleaner.
//!
//! ## Wire format
//!
//! Substream-per-snapshot. For each snapshot to send:
//!
//! 1. Manager opens a fresh substream on [`REGISTRY_PROTOCOL`] to the
//!    target member via `libp2p_stream::Control::open_stream`.
//! 2. Manager writes one [`SnapshotEnvelope`] length-prefixed onto
//!    the substream and closes the write half.
//! 3. Member reads the single envelope, applies it locally, closes
//!    the read half.
//!
//! Fire-and-forget — no acknowledgement frame. The libp2p substream
//! reaching its remote write-half-closed state is the implicit "the
//! bytes got there" signal. A snapshot that fails delivery (peer
//! disconnected, dial failed) is dropped; the next mutation produces
//! a new snapshot that gets retried implicitly. Snapshots are
//! idempotent — applying the same ClusterDoc twice is a no-op.
//!
//! ## Wire envelope
//!
//! [`SnapshotEnvelope`] is JSON-encoded; the inner `ClusterDoc` uses
//! the same shape as everywhere else in the SDK (matches Discovery's
//! `/clusters/{name}` response shape). Snapshot count is small at
//! v1's cluster sizes (≤10 peers) — JSON's verbosity is irrelevant.
//! Length-prefix uses 4-byte big-endian framing, same shape as
//! [`crate::stream_protocol`]; bounded by [`MAX_FRAME_BYTES`].
//!
//! ## How a consumer uses it
//!
//! The Manager-side broadcast loop and member-side receiver live in
//! the [`auki-domain`](../../../auki-domain) crate; this module owns
//! only the wire types, the protocol id, the envelope, and the
//! framing helpers.
//!
//! Manager opens a substream via `libp2p_stream::Control::open_stream`
//! against `REGISTRY_PROTOCOL` and calls [`write_envelope`]; member
//! accepts via `Control::accept` and calls [`read_envelope`].
//!
//! ## Lab-mode versioning
//!
//! Protocol id is `0.0.1`, not `1.0.0`. Per the SDK convention.

use crate::cluster_doc::ClusterDoc;
use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

/// libp2p protocol id for the Manager→members Cluster Registry
/// snapshot broadcast. Stable; do not change without coordinating
/// with consumers.
pub const REGISTRY_PROTOCOL: &str = "/auki/registry/0.0.1";

/// `StreamProtocol` form for use with
/// `libp2p_stream::Control::accept` / `open_stream`.
pub fn protocol() -> StreamProtocol {
    StreamProtocol::new(REGISTRY_PROTOCOL)
}

/// Maximum envelope size on the wire, in bytes. Bounded so a peer
/// cannot drive an OOM by sending an arbitrarily-large length prefix.
/// Generous enough for any plausible cluster size — at v1's ≤10 peer
/// target the envelope is well under 10 KB.
///
/// 1 MiB. Same justification as
/// [`crate::stream_protocol::MAX_FRAME_BYTES`] (16 MiB) but tighter
/// because a registry snapshot doesn't carry sensor data.
pub const MAX_FRAME_BYTES: u32 = 1024 * 1024;

/// One Manager-authored snapshot broadcast. Wire envelope; the
/// `mutation_ns` field lets receivers detect retransmits / stale
/// snapshots without consulting their own clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEnvelope {
    /// The Manager's monotonic timestamp (ns) at the moment the
    /// mutation that produced this snapshot was committed locally.
    /// Source clock is the Manager's session-monotonic clock.
    /// Receivers use this as a stale-snapshot detector: if they've
    /// already applied a snapshot with a later `mutation_ns` from
    /// the same Manager, this one is discarded.
    pub mutation_ns: u64,

    /// The full Cluster Registry snapshot the Manager wants every
    /// member to see. Identical shape to Discovery's
    /// `/clusters/{name}` response — same JSON, same
    /// `serde(with = "multiaddr_vec_serde")` adapter for peer
    /// addresses. A daemon that knows how to parse Discovery's
    /// response already knows how to parse this.
    pub doc: ClusterDoc,
}

/// Failure modes for [`read_envelope`] / [`write_envelope`].
#[derive(Debug, thiserror::Error)]
pub enum RegistryProtocolError {
    /// Underlying I/O on the libp2p substream failed.
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    /// JSON encode failed. Almost always a bug — `SnapshotEnvelope`
    /// is designed to round-trip.
    #[error("encode: {0}")]
    Encode(#[source] serde_json::Error),
    /// JSON decode failed. Peer sent malformed bytes.
    #[error("decode: {0}")]
    Decode(#[source] serde_json::Error),
    /// Length prefix exceeds [`MAX_FRAME_BYTES`].
    #[error("envelope too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
    /// Length prefix is zero. Defined out — every well-formed
    /// envelope has at least the JSON object braces.
    #[error("envelope is empty (length prefix is zero)")]
    EmptyFrame,
}

/// Write a single [`SnapshotEnvelope`] to `stream`, length-prefixed.
///
/// Caller is expected to close the write half after this returns so
/// the receiver sees EOF and stops reading.
pub async fn write_envelope<S>(
    stream: &mut S,
    env: &SnapshotEnvelope,
) -> Result<(), RegistryProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let bytes = serde_json::to_vec(env).map_err(RegistryProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_FRAME_BYTES as u64 {
        return Err(RegistryProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(RegistryProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(RegistryProtocolError::Io)?;
    stream.flush().await.map_err(RegistryProtocolError::Io)?;
    Ok(())
}

/// Read a single [`SnapshotEnvelope`] from `stream`. End-of-stream
/// from the peer before the full envelope arrives surfaces as
/// `Err(RegistryProtocolError::Io(e))` with
/// `e.kind() == UnexpectedEof`.
pub async fn read_envelope<S>(stream: &mut S) -> Result<SnapshotEnvelope, RegistryProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(RegistryProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf) as u64;
    if len == 0 {
        return Err(RegistryProtocolError::EmptyFrame);
    }
    if len > MAX_FRAME_BYTES as u64 {
        return Err(RegistryProtocolError::FrameTooLarge {
            actual: len,
            max: MAX_FRAME_BYTES as u64,
        });
    }
    let mut buf = vec![0u8; len as usize];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(RegistryProtocolError::Io)?;
    serde_json::from_slice(&buf).map_err(RegistryProtocolError::Decode)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster_doc::{ClusterDoc, ClusterPeer};
    use futures::io::Cursor;
    use libp2p::PeerId;
    use libp2p_identity::ed25519;
    use multiaddr::Multiaddr;

    fn fixture_peer_id(seed: u8) -> PeerId {
        let mut s = [seed; 32];
        let sk = ed25519::SecretKey::try_from_bytes(&mut s).unwrap();
        let kp = ed25519::Keypair::from(sk);
        let kp = libp2p_identity::Keypair::from(kp);
        kp.public().to_peer_id()
    }

    fn fixture_envelope() -> SnapshotEnvelope {
        let p1 = fixture_peer_id(1);
        let p2 = fixture_peer_id(2);
        let addr1: Multiaddr = "/ip4/192.168.1.10/tcp/4001".parse().unwrap();
        let addr2: Multiaddr = "/ip4/192.168.1.11/tcp/4001".parse().unwrap();
        SnapshotEnvelope {
            mutation_ns: 1_234_567_890,
            doc: ClusterDoc {
                version: 1,
                cluster_name: "demo-domain".into(),
                created_ns: 0,
                current_manager_peer_id: None,
                peers: vec![
                    ClusterPeer {
                        peer_id: p1,
                        addresses: vec![addr1],
                        expected_app_id: Some("park".into()),
                        note: None,
                    },
                    ClusterPeer {
                        peer_id: p2,
                        addresses: vec![addr2],
                        expected_app_id: Some("boosterapp".into()),
                        note: None,
                    },
                ],
            },
        }
    }

    #[test]
    fn protocol_id_is_locked() {
        // Wire format. If this test fails, you're looking at a
        // breaking change.
        assert_eq!(REGISTRY_PROTOCOL, "/auki/registry/0.0.1");
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let env = fixture_envelope();
        let json = serde_json::to_string(&env).unwrap();
        let parsed: SnapshotEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(env, parsed);
    }

    #[tokio::test]
    async fn write_then_read_round_trips() {
        let env = fixture_envelope();
        let mut buf = Vec::new();
        {
            let mut cursor = Cursor::new(&mut buf);
            write_envelope(&mut cursor, &env).await.unwrap();
        }
        let mut cursor = Cursor::new(buf);
        let read = read_envelope(&mut cursor).await.unwrap();
        assert_eq!(env, read);
    }

    #[tokio::test]
    async fn read_rejects_oversized_frame_via_length_prefix() {
        // Construct a length prefix that exceeds MAX_FRAME_BYTES.
        let mut buf = vec![];
        buf.extend_from_slice(&((MAX_FRAME_BYTES as u32 + 1).to_be_bytes()));
        // Don't append the body — read_exact would block on it; the
        // length-prefix check fires first.
        let mut cursor = Cursor::new(buf);
        let err = read_envelope(&mut cursor).await.unwrap_err();
        match err {
            RegistryProtocolError::FrameTooLarge { actual, max } => {
                assert_eq!(actual, (MAX_FRAME_BYTES as u64) + 1);
                assert_eq!(max, MAX_FRAME_BYTES as u64);
            }
            other => panic!("expected FrameTooLarge, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn read_rejects_zero_length_frame() {
        let buf = vec![0u8; 4]; // length prefix = 0
        let mut cursor = Cursor::new(buf);
        let err = read_envelope(&mut cursor).await.unwrap_err();
        assert!(matches!(err, RegistryProtocolError::EmptyFrame));
    }

    #[tokio::test]
    async fn read_surfaces_eof_as_io_error() {
        let buf: Vec<u8> = vec![]; // empty — first read_exact hits EOF
        let mut cursor = Cursor::new(buf);
        let err = read_envelope(&mut cursor).await.unwrap_err();
        match err {
            RegistryProtocolError::Io(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
            }
            other => panic!("expected Io(UnexpectedEof), got {:?}", other),
        }
    }

    /// Locked cross-language conformance vector — pins the wire-byte
    /// representation of a fixed minimal envelope. Any cross-language
    /// reimplementation must produce these exact bytes from the same
    /// input.
    ///
    /// Empty `peers` list, `cluster_name = "x"`, `mutation_ns = 0`,
    /// `version = 1`, `created_ns = 0`, no `current_manager_peer_id`.
    /// Locks the JSON serialization shape so a future change to field
    /// order, default-elision, etc. is caught immediately. The
    /// `created_ns` field always serialises (required-shaped on the
    /// wire); `current_manager_peer_id` is Option-skipped when None.
    #[test]
    fn envelope_locked_minimal_json() {
        let env = SnapshotEnvelope {
            mutation_ns: 0,
            doc: ClusterDoc {
                version: 1,
                cluster_name: "x".into(),
                created_ns: 0,
                current_manager_peer_id: None,
                peers: vec![],
            },
        };
        let json = serde_json::to_string(&env).unwrap();
        assert_eq!(
            json,
            r#"{"mutation_ns":0,"doc":{"version":1,"cluster_name":"x","created_ns":0,"peers":[]}}"#
        );
    }

    /// Locked vector for the post-failover shape: `current_manager_peer_id`
    /// populated, `created_ns` populated. Pins the wire shape every
    /// cross-language reimplementation must produce when both Greenland
    /// fields are present.
    #[test]
    fn envelope_locked_with_manager_and_created_ns() {
        let manager = fixture_peer_id(7);
        let env = SnapshotEnvelope {
            mutation_ns: 0,
            doc: ClusterDoc {
                version: 1,
                cluster_name: "x".into(),
                created_ns: 1_715_423_400_000_000_000,
                current_manager_peer_id: Some(manager),
                peers: vec![],
            },
        };
        let json = serde_json::to_string(&env).unwrap();
        // libp2p-identity serialises PeerId as its canonical base58
        // string; format!() the expected JSON around the runtime-derived
        // value so the assertion stays brittle on shape but not on
        // PeerId text (which can vary with crate version).
        let expected_manager = manager.to_string();
        let expected = format!(
            r#"{{"mutation_ns":0,"doc":{{"version":1,"cluster_name":"x","created_ns":1715423400000000000,"current_manager_peer_id":"{expected_manager}","peers":[]}}}}"#
        );
        assert_eq!(json, expected);
    }
}
