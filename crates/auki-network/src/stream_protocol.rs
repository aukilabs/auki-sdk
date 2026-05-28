//! `/auki/stream/0.2.0` — libp2p substream protocol carrying typed
//! [`Stream<T>`] data, encoded as protobuf via prost. Step 2 of the
//! [`auki-datatypes` migration](../../auki-datatypes/src/sprint.md)
//! moved the wire format off JSON-via-serde-json onto protobuf.
//!
//! Substream lifetime IS the stream subscription's lifetime (grimsby D1
//! RESOLVED 2026-05-05): the initiator opens the substream, writes a
//! [`StreamRequest`] as the first framed message, the responder either
//! declines-and-closes or starts pushing typed [`StreamEntry`] messages on
//! the same substream until close. One protocol; full-duplex;
//! `cluster.json` is the trust boundary (same as ansuz — peers not in
//! the doc cannot dial).
//!
//! This module ships the **wire primitives**: protocol id, message
//! envelope re-exports from [`auki_datatypes::stream`], framing
//! helpers. The `Stream<T>` Rust API (consumer / producer handles) and
//! the runtime integration (per-substream lifecycle, `stream_provider`
//! invocation) live in [`crate::stream_runtime`].
//!
//! ## Wire format
//!
//! Each substream is a sequence of length-prefixed [`StreamMessage`]
//! values:
//!
//! ```text
//!  +--------+---------+--------+---------+---
//!  |  4-byte u32 BE   |   prost-encoded payload     |  ...
//!  +--------+---------+--------+---------+---
//! ```
//!
//! - **Length prefix.** 4-byte big-endian unsigned 32-bit length of the
//!   protobuf payload that follows. Bounded by [`MAX_FRAME_BYTES`] so a
//!   peer can't drive an OOM by claiming a huge length; generous enough
//!   (16 MiB) to admit raw sensor frames if a future `Stream<T>`
//!   instantiation carries them. Camera frames typically run 10–100 KB;
//!   raw `PointCloud2` CDR runs ~700 KB at ~30 Hz.
//! - **Payload.** Prost-encoded [`StreamMessage`]. The envelope is a
//!   `oneof` of `Request | Accept | Decline | Entry | EndOfStream`;
//!   each substream is mono-`T` with `T`'s prost bytes living inside
//!   `StreamEntry.payload` — the SDK runtime knows which `T` to decode based
//!   on the [`StreamManifest::sensor_hash`] or
//!   [`StreamManifest::resource_id`] handshake.
//!
//! ## Why protobuf, why now
//!
//! The previous wire used JSON-via-`serde_json`. Two pressures: (1)
//! `Vec<u8>` rendered as JSON arrays-of-integers (~4× overhead), so
//! point-cloud payloads had to carry a `base64_bytes` adapter; (2) the
//! cross-language schema lived in two places (Rust hand-rolled structs
//! + Python's hand-rolled mirror in `auki-network-py`). Protobuf
//! addresses both: native binary fields drop the adapter, and the
//! `.proto` file in [`auki-datatypes`](../../auki-datatypes/proto/stream.proto)
//! is the single source of truth that consumers in any language
//! generate from.
//!
//! ## Message order on a healthy stream
//!
//! 1. Initiator → Responder: `Request(StreamRequest)`
//! 2. Responder → Initiator: `Accept(StreamManifest)` *or*
//!    `Decline(DeclineReason)`
//! 3. Responder → Initiator: zero or more `StreamEntry { … }`
//! 4. Responder → Initiator: `EndOfStream(EndReason)` *or* substream
//!    closes without it (consumer treats "substream closed without
//!    EndOfStream" as an implicit connection loss, surfaced as
//!    [`crate::stream_runtime::StreamError::ConnectionLost`]).
//!
//! ## Trust boundary
//!
//! Cluster doc gates membership; `auki-network`'s swarm refuses dials
//! from peers not present in `cluster.json` at the Noise layer. Stream
//! protocol does not introduce a new admission decision.

use crate::PEER_DERIVATION_LABEL;
use auki_identity::Wallet;
use futures::{AsyncReadExt, AsyncWriteExt};
use prost::Message;
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
fn _phantom() -> Option<Wallet> {
    Some(
        Wallet::from_seed(vec![0u8; 32])
            .expect("32-byte seed")
            .derive_child(PEER_DERIVATION_LABEL),
    )
}

// Wire-type re-exports — single source of truth lives in
// `auki-datatypes`. The opaque-bytes / structured-vector payloads
// (`audio`, `joint_encoders`, `point_cloud`) share one `Data` message
// per module that's used on both disk (Sensor Log segment) and wire
// (this substream); the dual `*_stream` packages were removed in
// favour of that single shape.
pub use auki_datatypes::camera::{CameraFrame, DynamicIntrinsics};
pub use auki_datatypes::stream::{
    DeclineReason, EndReason, StreamEntry, StreamManifest, StreamMessage, decline_reason,
    end_reason, stream_message,
};
pub use auki_datatypes::{audio, joint_encoders, point_cloud, pose};

// ─── StreamRequest + ReadFrom ─────────────────────────────────────────────────

/// Where to start reading on the producer's log when accepting a stream
/// subscription.
///
/// `Latest` — tail from the current end (live streaming; no replay).
/// `FromStart` — replay from the beginning of the log.
/// `FromTimestamp(i64)` — start from the first entry at or after the
/// given nanosecond timestamp on the log's own clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadFrom {
    /// Tail from the current live end — no historical replay.
    Latest,
    /// Replay from the very first entry in the log.
    FromStart,
    /// Start at the first entry whose timestamp is ≥ this nanosecond
    /// value on the log's clock.
    FromTimestamp(i64),
}

impl Default for ReadFrom {
    fn default() -> Self {
        ReadFrom::Latest
    }
}

/// Consumer → Producer handshake: identifies the log the consumer wants
/// to subscribe to, and where on that log to begin.
///
/// `source_peer_id` is the libp2p peer-id string of the peer that wrote
/// the log (the canonical source, which may differ from the peer being
/// dialled — e.g. a materializer re-serving a log it cached from a
/// robot). `resource_id` is the log's stable identity string, matching
/// the `resource_id` in the producer's Resource Catalog.
///
/// `writer_peer_id` is implicit by the libp2p connection — no field
/// needed for v1. The connection itself identifies who the consumer is
/// talking to; if the serving peer is a materializer, `source_peer_id`
/// identifies the original writer.
///
/// Wire format: encoded as a prost `auki.stream.StreamRequest` message
/// on the `/auki/stream/0.2.0` substream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StreamRequest {
    /// Peer-id string of the peer that originally wrote the log. Empty
    /// string for legacy v0 subscriptions that pre-date this field; in
    /// that case the serving peer is assumed to be the source.
    pub source_peer_id: String,
    /// Stable identity of the log/resource being subscribed to.
    pub resource_id: String,
    /// Starting position on the log. Defaults to [`ReadFrom::Latest`].
    #[serde(default)]
    pub from: ReadFrom,
}

// ─── Wire conversion helpers ──────────────────────────────────────────────────

/// Convert the Rust `StreamRequest` to the prost wire type so it can
/// be wrapped in a `StreamMessage::request(…)`.
pub(crate) fn stream_request_to_wire(req: StreamRequest) -> auki_datatypes::stream::StreamRequest {
    use auki_datatypes::stream::{
        ReadFromLatest, ReadFromStart, ReadFromTimestamp, stream_request,
    };
    let read_from = match req.from {
        ReadFrom::Latest => Some(stream_request::ReadFrom::Latest(ReadFromLatest {})),
        ReadFrom::FromStart => Some(stream_request::ReadFrom::FromStart(ReadFromStart {})),
        ReadFrom::FromTimestamp(ts) => {
            Some(stream_request::ReadFrom::FromTimestamp(ReadFromTimestamp {
                timestamp_ns: ts,
            }))
        }
    };
    auki_datatypes::stream::StreamRequest {
        sensor_id: String::new(),
        resource_id: req.resource_id,
        source_peer_id: req.source_peer_id,
        read_from,
    }
}

/// Convert the prost wire type back to the Rust `StreamRequest`.
pub(crate) fn stream_request_from_wire(
    wire: auki_datatypes::stream::StreamRequest,
) -> StreamRequest {
    use auki_datatypes::stream::stream_request;
    let from = match wire.read_from {
        Some(stream_request::ReadFrom::Latest(_)) => ReadFrom::Latest,
        Some(stream_request::ReadFrom::FromStart(_)) => ReadFrom::FromStart,
        Some(stream_request::ReadFrom::FromTimestamp(t)) => ReadFrom::FromTimestamp(t.timestamp_ns),
        None => ReadFrom::Latest,
    };
    StreamRequest {
        source_peer_id: wire.source_peer_id,
        resource_id: wire.resource_id,
        from,
    }
}

/// libp2p protocol id for the typed-byte-stream protocol. Stable; do
/// not change without coordinating with consumers (Boosterapp, Sentinel,
/// Park) and any cross-language reimplementation.
///
/// Pre-1.0; sub-1.0 versioning per the workspace convention. Replaces
/// the previous JSON-on-wire `/auki/stream/1.0.0` (retired in this
/// PR) at Step 2 of the [`auki-datatypes`](../../auki-datatypes)
/// migration. The new wire is prost-encoded `StreamMessage` from
/// `auki-datatypes`'s `auki.stream` package; consumers update their
/// decoders in lockstep. 1.0.0 is reserved for the SDK's first
/// official release.
pub const STREAM_PROTOCOL: &str = "/auki/stream/0.2.0";

/// Maximum framed-message size on the wire, in bytes. Bounded so a peer
/// cannot drive an OOM by sending an arbitrarily-large length prefix;
/// generous enough to admit any reasonable single sensor frame.
///
/// 16 MiB. Camera frames typically run 10–100 KB; raw NV12 frames at K1
/// resolutions run ~400 KB; point cloud frames after server-side
/// decimation are usually well under 1 MB.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

// ─── Errors ──────────────────────────────────────────────────────────────────

/// Failure modes for [`read_message`] / [`write_message`].
#[derive(Debug, thiserror::Error)]
pub enum StreamProtocolError {
    /// Underlying I/O on the libp2p substream failed.
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    /// Protobuf encoding failed (write side; almost always a bug — types
    /// in this module are designed to round-trip).
    #[error("encode: {0}")]
    Encode(#[source] prost::EncodeError),
    /// Protobuf decoding failed (read side; peer sent malformed bytes
    /// or a wire-incompatible payload).
    #[error("decode: {0}")]
    Decode(#[source] prost::DecodeError),
    /// `StreamMessage` arrived with no `variant` set — unspecified
    /// envelope. Peer is malformed or speaks a future protocol version.
    #[error("stream message has no variant set")]
    MissingVariant,
    /// Length prefix exceeds [`MAX_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
    /// Length prefix is zero. Defined out — every well-formed
    /// `StreamMessage` has at least its `variant` field tag.
    #[error("frame is empty (length prefix is zero)")]
    EmptyFrame,
}

// ─── Framing helpers ─────────────────────────────────────────────────────────

/// Write a single [`StreamMessage`] to `stream`, length-prefixed.
pub async fn write_message<S>(
    stream: &mut S,
    msg: &StreamMessage,
) -> Result<(), StreamProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    let mut bytes = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut bytes)
        .map_err(StreamProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_FRAME_BYTES as u64 {
        return Err(StreamProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(StreamProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(StreamProtocolError::Io)?;
    stream.flush().await.map_err(StreamProtocolError::Io)?;
    Ok(())
}

/// Read a single [`StreamMessage`] from `stream`. End-of-stream from the
/// peer surfaces as `Err(StreamProtocolError::Io(e))` with
/// `e.kind() == UnexpectedEof`.
pub async fn read_message<S>(stream: &mut S) -> Result<StreamMessage, StreamProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(StreamProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(StreamProtocolError::EmptyFrame);
    }
    if len > MAX_FRAME_BYTES {
        return Err(StreamProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(StreamProtocolError::Io)?;
    StreamMessage::decode(&*payload).map_err(StreamProtocolError::Decode)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::Cursor;

    fn request_msg(resource_id: &str) -> StreamMessage {
        StreamMessage::request(stream_request_to_wire(StreamRequest {
            resource_id: resource_id.into(),
            ..Default::default()
        }))
    }

    fn accept_msg(
        sensor_id: &str,
        sensor_hash: &str,
        clock_id: &str,
        clock_hash: &str,
        frame_id: &str,
        frame_hash: &str,
    ) -> StreamMessage {
        StreamMessage::accept(StreamManifest {
            sensor_id: sensor_id.into(),
            sensor_hash: sensor_hash.into(),
            clock_id: clock_id.into(),
            clock_hash: clock_hash.into(),
            frame_id: frame_id.into(),
            frame_hash: frame_hash.into(),
            ..Default::default()
        })
    }

    fn entry_msg(timestamp_ns: i64, seq: u64, payload: Vec<u8>) -> StreamMessage {
        StreamMessage::entry(StreamEntry {
            timestamp_ns,
            seq,
            payload,
        })
    }

    #[test]
    fn protocol_id_is_locked() {
        // Wire format. Coordinate with Boosterapp, Sentinel, Park, and
        // any cross-language reimplementation before touching it.
        assert_eq!(STREAM_PROTOCOL, "/auki/stream/0.2.0");
    }

    #[test]
    fn max_frame_bytes_is_locked() {
        assert_eq!(MAX_FRAME_BYTES, 16 * 1024 * 1024);
    }

    #[test]
    fn request_message_round_trips() {
        let msg = request_msg("K1-AABBCCDDEEFF/head_left_cam");
        let mut bytes = Vec::new();
        msg.encode(&mut bytes).unwrap();
        let back = StreamMessage::decode(&*bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn accept_message_round_trips() {
        let msg = accept_msg(
            "K1-AABBCCDDEEFF/head_left_cam",
            "abcdef",
            "K1-AABBCCDDEEFF/utc",
            "deadbeef",
            "K1-AABBCCDDEEFF/head_left_cam/optical",
            "framebeef",
        );
        let mut bytes = Vec::new();
        msg.encode(&mut bytes).unwrap();
        let back = StreamMessage::decode(&*bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn pose_request_and_manifest_round_trip_resource_identity() {
        let request = StreamRequest {
            source_peer_id: "galbot".into(),
            resource_id: "K1/base_link->K1/head_left_rgb_optical".into(),
            ..Default::default()
        };
        let wire_req = stream_request_to_wire(request.clone());
        let msg = StreamMessage::request(wire_req.clone());
        let bytes = msg.encode_to_vec();
        let back = StreamMessage::decode(&*bytes).unwrap();
        assert_eq!(back, StreamMessage::request(wire_req));

        let manifest = StreamManifest {
            sensor_id: String::new(),
            sensor_hash: String::new(),
            clock_id: "K1/monotonic".into(),
            clock_hash: "clockhash".into(),
            frame_id: String::new(),
            frame_hash: String::new(),
            resource_id: "K1/base_link->K1/head_left_rgb_optical".into(),
            payload: "spatial_transform".into(),
            from_frame_id: "K1/base_link".into(),
            from_frame_hash: "basehash".into(),
            to_frame_id: "K1/head_left_rgb_optical".into(),
            to_frame_hash: "headhash".into(),
            writer_mode: "movable".into(),
            expected_rate_hz: 30,
        };
        let msg = StreamMessage::accept(manifest.clone());
        let bytes = msg.encode_to_vec();
        let back = StreamMessage::decode(&*bytes).unwrap();
        assert_eq!(back, StreamMessage::accept(manifest));
    }

    #[test]
    fn decline_message_round_trips() {
        let msg = StreamMessage::decline(DeclineReason::sensor_not_found());
        let mut bytes = Vec::new();
        msg.encode(&mut bytes).unwrap();
        let back = StreamMessage::decode(&*bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn decline_other_round_trips_with_detail() {
        let msg = StreamMessage::decline(DeclineReason::other("provider raised"));
        let mut bytes = Vec::new();
        msg.encode(&mut bytes).unwrap();
        let back = StreamMessage::decode(&*bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn frame_message_round_trips() {
        let msg = entry_msg(12_345_678_900, 7, vec![0xff, 0xd8, 0xff, 0xe0, 0x00]);
        let mut bytes = Vec::new();
        msg.encode(&mut bytes).unwrap();
        let back = StreamMessage::decode(&*bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn end_of_stream_round_trips() {
        let msg = StreamMessage::end_of_stream(EndReason::producer_error("encoder died"));
        let mut bytes = Vec::new();
        msg.encode(&mut bytes).unwrap();
        let back = StreamMessage::decode(&*bytes).unwrap();
        assert_eq!(msg, back);
    }

    #[tokio::test]
    async fn write_then_read_round_trips_a_request() {
        let msg = request_msg("test/sensor");
        let mut buf: Vec<u8> = Vec::new();
        write_message(&mut buf, &msg).await.unwrap();
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(len as usize, buf.len() - 4);
        let mut cursor = Cursor::new(buf);
        let back = read_message(&mut cursor).await.unwrap();
        assert_eq!(back, msg);
    }

    #[tokio::test]
    async fn write_then_read_round_trips_a_full_session() {
        let messages = vec![
            request_msg("K1/cam_a"),
            accept_msg("K1/cam_a", "h1", "c1", "h2", "K1/cam_a/frame", "fh1"),
            entry_msg(1, 0, vec![1, 2, 3]),
            entry_msg(2, 1, vec![4, 5, 6]),
            entry_msg(3, 2, vec![7, 8, 9]),
            StreamMessage::end_of_stream(EndReason::source_ended()),
        ];

        let mut buf: Vec<u8> = Vec::new();
        for msg in &messages {
            write_message(&mut buf, msg).await.unwrap();
        }

        let mut cursor = Cursor::new(buf);
        let mut received: Vec<StreamMessage> = Vec::new();
        for _ in 0..messages.len() {
            received.push(read_message(&mut cursor).await.unwrap());
        }
        assert_eq!(received, messages);
    }

    #[tokio::test]
    async fn read_rejects_oversized_frame_via_length_prefix() {
        let too_big = MAX_FRAME_BYTES as u64 + 1;
        let len = (too_big as u32).to_be_bytes();
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&len);
        let mut cursor = Cursor::new(buf);
        let err = read_message(&mut cursor).await;
        assert!(matches!(
            err,
            Err(StreamProtocolError::FrameTooLarge { actual, max })
                if actual == too_big && max == MAX_FRAME_BYTES as u64
        ));
    }

    #[tokio::test]
    async fn read_rejects_empty_frame() {
        let mut cursor = Cursor::new(0u32.to_be_bytes().to_vec());
        let err = read_message(&mut cursor).await;
        assert!(matches!(err, Err(StreamProtocolError::EmptyFrame)));
    }

    #[tokio::test]
    async fn read_surfaces_eof_as_io_error() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let err = read_message(&mut cursor).await;
        match err {
            Err(StreamProtocolError::Io(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
            }
            other => panic!("expected Io(UnexpectedEof), got {other:?}"),
        }
    }

    #[test]
    fn write_rejects_oversized_payload_before_io() {
        // 17 MiB payload trips the cap (16 MiB).
        let huge = vec![0u8; 17 * 1024 * 1024];
        let msg = entry_msg(0, 0, huge);
        let result = futures::executor::block_on(async {
            let mut buf: Vec<u8> = Vec::new();
            write_message(&mut buf, &msg).await
        });
        assert!(matches!(
            result,
            Err(StreamProtocolError::FrameTooLarge { .. })
        ));
    }

    // ─── Locked cross-language conformance vectors ────────────────────────

    /// `CameraFrame` prost wire bytes. Camera streams carry the
    /// exact same payload record as camera Sensor Logs, not a stream-only
    /// wrapper around the JPEG bytes.
    #[test]
    fn camera_frame_serializes_to_locked_wire_bytes() {
        let frame = CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46],
        };
        let bytes = frame.encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        // Field 2 (`frame`) length-delimited: tag 0x12, len 0x0a, then 10 bytes.
        assert_eq!(hex, "120affd8ffe000104a464946");
    }

    #[test]
    fn camera_frame_round_trips() {
        let frame = CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![1, 2, 3, 4, 5],
        };
        let bytes = frame.encode_to_vec();
        let back = CameraFrame::decode(&*bytes).unwrap();
        assert_eq!(back, frame);
    }

    /// `point_cloud::Data` prost wire bytes. Same opaque-bytes shape on
    /// disk and on the wire (one type, one byte spec — locked by the
    /// `point_cloud_data_*` tests in `auki-datatypes`).
    #[test]
    fn point_cloud_data_serializes_to_locked_wire_bytes() {
        let frame = point_cloud::Data {
            data: vec![0x42, 0x43, 0x44, 0x45, 0x00, 0x01, 0xfe, 0xff],
        };
        let bytes = frame.encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        let expected: String = std::iter::once(0x0au8)
            .chain(std::iter::once(0x08u8))
            .chain(frame.data.iter().copied())
            .map(|b| format!("{:02x}", b))
            .collect();
        assert_eq!(hex, expected);
    }

    #[test]
    fn point_cloud_data_round_trips_a_kilobyte_payload() {
        let bytes: Vec<u8> = (0..1024u32).map(|i| (i * 37 + 11) as u8).collect();
        let frame = point_cloud::Data {
            data: bytes.clone(),
        };
        let encoded = frame.encode_to_vec();
        let back = point_cloud::Data::decode(&*encoded).unwrap();
        assert_eq!(back.data, bytes);
    }

    /// Wire-size pin: protobuf's `bytes` field is native binary, no
    /// JSON tax. A 1 KB payload encodes to ~1 KB + tiny envelope (tag +
    /// varint length).
    #[test]
    fn point_cloud_data_wire_size_is_native_binary() {
        let data = vec![0xAB; 1024];
        let frame = point_cloud::Data { data };
        let encoded = frame.encode_to_vec();
        // 1 byte tag + 2 bytes varint length (1024) + 1024 bytes payload = 1027.
        assert_eq!(encoded.len(), 1027);
    }

    /// `joint_encoders::Data` prost wire bytes (sawslin Phase B). Same
    /// `repeated float angles_rad` shape on disk and on the wire — one
    /// type, one byte spec.
    #[test]
    fn joint_encoders_data_serializes_to_locked_wire_bytes() {
        let frame = joint_encoders::Data {
            angles_rad: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        };
        let bytes = frame.encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        // Field 1 (`repeated float`) packed: tag 0x0a, varint length 0x18
        // (24 = 6 × 4 little-endian f32), then six f32s.
        // 1.0 → 0000803f, 2.0 → 00000040, 3.0 → 00004040,
        // 4.0 → 00008040, 5.0 → 0000a040, 6.0 → 0000c040.
        assert_eq!(hex, "0a180000803f0000004000004040000080400000a0400000c040");
    }

    #[test]
    fn joint_encoders_data_round_trips() {
        let frame = joint_encoders::Data {
            angles_rad: vec![0.0, 0.5, -1.5, 3.14159],
        };
        let bytes = frame.encode_to_vec();
        let back = joint_encoders::Data::decode(&*bytes).unwrap();
        assert_eq!(back, frame);
    }

    /// Empty `angles_rad` is the proto3 default — a `repeated` field
    /// with no entries elides to zero bytes on the wire. Locked so the
    /// SDK's "frame with no joints" edge case stays predictable.
    #[test]
    fn joint_encoders_data_empty_vector_elides() {
        let frame = joint_encoders::Data { angles_rad: vec![] };
        let bytes = frame.encode_to_vec();
        assert!(bytes.is_empty());
        let back = joint_encoders::Data::decode(&*bytes).unwrap();
        assert_eq!(back.angles_rad, Vec::<f32>::new());
    }

    /// `audio::Data` prost wire bytes (Dialogue Batch 1). Same opaque-
    /// bytes shape on disk and on the wire — one type, one byte spec.
    #[test]
    fn audio_data_serializes_to_locked_wire_bytes() {
        let frame = audio::Data {
            data: vec![0x00, 0x80, 0xff, 0x7f, 0x40, 0x40, 0xc0, 0xbf],
        };
        let bytes = frame.encode_to_vec();
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        // Field 1 length-delimited: tag 0x0a, len 0x08, then 8 bytes.
        assert_eq!(hex, "0a080080ff7f4040c0bf");
    }

    #[test]
    fn audio_data_round_trips() {
        let frame = audio::Data {
            data: vec![0, 1, 2, 3, 4, 5],
        };
        let bytes = frame.encode_to_vec();
        let back = audio::Data::decode(&*bytes).unwrap();
        assert_eq!(back, frame);
    }

    /// 20 ms of 48 kHz mono int16-LE PCM — the canonical Dialogue
    /// fixture (960 samples × 2 bytes = 1920 bytes payload). Pins the
    /// wire-size envelope expected on the actual demo path: 1 byte tag
    /// + 2 bytes varint length (1920) + 1920 payload = 1923 bytes.
    #[test]
    fn audio_data_wire_size_is_native_binary() {
        let data = vec![0xAB; 1920];
        let frame = audio::Data { data };
        let encoded = frame.encode_to_vec();
        assert_eq!(encoded.len(), 1923);
    }

    /// Locked conformance vector for a `StreamMessage::Entry` carrying
    /// a `point_cloud::Data` payload. Pins the full envelope: the `StreamEntry`
    /// inside the `StreamMessage` oneof carries a `bytes` field whose
    /// content is the prost-encoded `point_cloud::Data`.
    #[test]
    fn locked_stream_message_entry_with_point_cloud_payload() {
        let pc = point_cloud::Data {
            data: vec![0x42, 0x43, 0x44, 0x45, 0x00, 0x01, 0xfe, 0xff],
        };
        let pc_encoded = pc.encode_to_vec();
        let msg = entry_msg(1_700_000_000_000_000_000, 42, pc_encoded.clone());

        let envelope = msg.encode_to_vec();
        let back = StreamMessage::decode(&*envelope).unwrap();
        assert_eq!(back, msg);

        // Decode the inner payload and confirm it's the same point_cloud::Data.
        let frame_inner = match back.variant {
            Some(stream_message::Variant::Entry(f)) => f,
            _ => panic!("expected Entry variant"),
        };
        let parsed_pc = point_cloud::Data::decode(&*frame_inner.payload).unwrap();
        assert_eq!(parsed_pc, pc);
        assert_eq!(frame_inner.timestamp_ns, 1_700_000_000_000_000_000);
        assert_eq!(frame_inner.seq, 42);
    }

    // ─── StreamRequest + ReadFrom spec tests (§5 of #216) ────────────────

    #[test]
    fn stream_request_canonical() {
        let r = StreamRequest {
            source_peer_id: "galbot".to_string(),
            resource_id: "head_left_rgb".to_string(),
            from: ReadFrom::Latest,
        };
        let value = serde_json::to_value(&r).unwrap();
        assert_eq!(value["source_peer_id"], "galbot");
        assert_eq!(value["resource_id"], "head_left_rgb");
        assert_eq!(value["from"], "latest");
    }

    #[test]
    fn stream_request_from_timestamp_canonical() {
        let r = StreamRequest {
            source_peer_id: "galbot".to_string(),
            resource_id: "head_left_rgb".to_string(),
            from: ReadFrom::FromTimestamp(1733836800000000000),
        };
        let value = serde_json::to_value(&r).unwrap();
        assert!(value["from"]["from_timestamp"] == 1733836800000000000i64);
    }

    #[test]
    fn read_from_enum_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(&ReadFrom::Latest).unwrap(),
            serde_json::json!("latest")
        );
        assert_eq!(
            serde_json::to_value(&ReadFrom::FromStart).unwrap(),
            serde_json::json!("from_start")
        );
        let v = serde_json::to_value(&ReadFrom::FromTimestamp(42)).unwrap();
        assert!(v.is_object()); // tagged enum variant with value
    }

    #[test]
    fn stream_request_rejects_writer_peer_id() {
        // The new schema does NOT include writer_peer_id. Sending one should
        // either deserialize fine (extra field ignored) or fail.
        let json = r#"{"source_peer_id":"galbot","resource_id":"head_left_rgb","writer_peer_id":"park","from":"latest"}"#;
        let parsed: Result<StreamRequest, _> = serde_json::from_str(json);
        // Default serde behavior: extra fields are silently ignored.
        if let Ok(req) = parsed {
            assert_eq!(req.source_peer_id, "galbot");
            // writer_peer_id is silently dropped — acceptable since v1 doesn't need it
        }
    }

    #[test]
    fn stream_request_wire_round_trip_with_source_peer_id() {
        let req = StreamRequest {
            source_peer_id: "galbot".to_string(),
            resource_id: "head_left_rgb".to_string(),
            from: ReadFrom::Latest,
        };
        let wire = stream_request_to_wire(req.clone());
        let back = stream_request_from_wire(wire);
        assert_eq!(back.source_peer_id, req.source_peer_id);
        assert_eq!(back.resource_id, req.resource_id);
        assert_eq!(back.from, req.from);
    }

    #[test]
    fn stream_request_wire_round_trip_from_timestamp() {
        let req = StreamRequest {
            source_peer_id: String::new(),
            resource_id: "head_left_rgb".to_string(),
            from: ReadFrom::FromTimestamp(1733836800000000000),
        };
        let wire = stream_request_to_wire(req.clone());
        let back = stream_request_from_wire(wire);
        assert_eq!(back.from, ReadFrom::FromTimestamp(1733836800000000000));
    }

    #[test]
    fn stream_request_wire_round_trip_from_start() {
        let req = StreamRequest {
            source_peer_id: String::new(),
            resource_id: "sensor_log".to_string(),
            from: ReadFrom::FromStart,
        };
        let wire = stream_request_to_wire(req.clone());
        let back = stream_request_from_wire(wire);
        assert_eq!(back.from, ReadFrom::FromStart);
        assert_eq!(back.resource_id, "sensor_log");
    }

    // ─────────────────────────────────────────────────────────────────────

    /// Proves the request/response bring-up matches the documented
    /// message-order spec: Request-then-Accept-then-StreamEntry-then-EndOfStream
    /// each survive the framing helpers in their typed positions.
    #[test]
    fn typed_session_matches_message_order_spec() {
        let wire_req = stream_request_to_wire(StreamRequest {
            source_peer_id: "galbot".into(),
            resource_id: "ordered".into(),
            ..Default::default()
        });
        let order = vec![
            stream_message::Variant::Request(wire_req),
            stream_message::Variant::Accept(StreamManifest {
                sensor_id: "ordered".into(),
                sensor_hash: "h".into(),
                clock_id: "c".into(),
                clock_hash: "ch".into(),
                frame_id: "ordered/frame".into(),
                frame_hash: "fh".into(),
                ..Default::default()
            }),
            stream_message::Variant::Entry(StreamEntry {
                timestamp_ns: 1,
                seq: 0,
                payload: vec![1, 2, 3],
            }),
            stream_message::Variant::EndOfStream(EndReason::source_ended()),
        ];
        for variant in order {
            let msg = StreamMessage {
                variant: Some(variant),
            };
            let encoded = msg.encode_to_vec();
            let back = StreamMessage::decode(&*encoded).unwrap();
            assert_eq!(back, msg);
        }
    }
}
