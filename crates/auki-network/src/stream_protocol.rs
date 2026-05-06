//! `/auki/stream/1.0.0` — libp2p substream protocol carrying typed [`Stream<T>`]
//! data. [grimsby](https://www.notion.so/3575c8e965928079a955ed9573bbb398)
//! deliverable #1.
//!
//! Substream lifetime IS the stream subscription's lifetime (grimsby D1 RESOLVED
//! 2026-05-05 — single `/auki/stream/1.0.0`): the initiator opens the substream,
//! writes a [`StreamRequest`] as the first framed message, the responder either
//! declines-and-closes or starts pushing typed [`StreamMessage::Frame`] messages
//! on the same substream until close. One protocol; full-duplex; cluster.json is
//! the trust boundary (same as ansuz — peers not in the doc cannot dial).
//!
//! This module ships the **wire primitives**: protocol id, message envelope,
//! framing helpers. The `Stream<T>` Rust API (consumer / producer handles) and
//! the runtime integration (per-substream lifecycle, `stream_provider`
//! invocation) are separate grimsby deliverables (#2, #3) that build on top of
//! these primitives.
//!
//! ## Wire format
//!
//! Each substream is a sequence of length-prefixed [`StreamMessage<T>`] values:
//!
//! ```text
//!  +--------+---------+--------+---------+---
//!  |  4-byte u32 BE   |   JSON-serialized payload   |  ...
//!  +--------+---------+--------+---------+---
//! ```
//!
//! - **Length prefix.** 4-byte big-endian unsigned 32-bit length of the JSON
//!   payload that follows. Bounded by [`MAX_FRAME_BYTES`] so a peer can't drive
//!   an OOM by claiming a huge length; generous enough (16 MiB) to admit raw
//!   sensor frames if a future `Stream<T>` instantiation carries them. JPEG
//!   frames (grimsby v1) typically run 10–100 KB.
//! - **Payload.** [`serde_json`]-encoded [`StreamMessage<T>`]. JSON for
//!   consistency with the rest of the SDK's wire format; `T` deliberately rides
//!   inside the same JSON envelope so that the framing is the SAME for every
//!   `Stream<T>` instantiation (whether `T` is a `JpegFrame`, a future raw
//!   sensor frame, a point cloud, etc.).
//!
//! Note for binary payloads: `serde_json` renders `Vec<u8>` as a JSON array of
//! integers, which is wasteful (~4× bandwidth vs. raw). For grimsby v1 (LAN,
//! 1–4 robots, 30 fps, 10–100 KB JPEGs) the bandwidth budget tolerates this; if
//! it becomes a problem the encoding switches to either base64-string-encoded
//! bytes or the codec switches to CBOR. Tracked in the auki-network parking
//! lot.
//!
//! ## Message order on a healthy stream
//!
//! 1. Initiator → Responder: `Request(StreamRequest)`
//! 2. Responder → Initiator: `Accept(AcceptInfo)` *or* `Decline(DeclineReason)`
//! 3. Responder → Initiator: zero or more `Frame { ... }`
//! 4. Responder → Initiator: `EndOfStream(EndReason)` *or* substream closes
//!    without it (consumer treats "substream closed without EndOfStream" as an
//!    implicit `EndOfStream(EndReason::ConnectionLost)`-equivalent)
//!
//! Future consumer→producer control messages (pause, request keyframe, params)
//! ride the same substream as new [`StreamMessage`] variants without a wire
//! change. Substreams are full-duplex (Yamux/QUIC); the responder's "writing
//! frames" pattern doesn't preclude the initiator from interleaving control.
//!
//! ## How a consumer uses this module
//!
//! Today the module exposes the wire primitives only. The SDK consumer-facing
//! API (`Stream<T>` + `StreamRuntime` + `stream_provider`) is grimsby
//! deliverables #2 / #3. Until those land, raw consumers can:
//!
//! - Construct a `libp2p_stream::Behaviour` (via [`crate::swarm::build_swarm`]
//!   — wired into the swarm `Behaviour` struct as the `stream` field).
//! - Acquire a [`libp2p_stream::Control`] from the behaviour and bind it to
//!   [`STREAM_PROTOCOL`] via `Control::accept` (responder side) or open
//!   outbound via `Control::open_stream` (initiator side).
//! - Use [`write_message`] / [`read_message`] to frame typed [`StreamMessage<T>`]
//!   values onto the resulting `libp2p::Stream`.
//!
//! ## Trust boundary
//!
//! Cluster doc gates membership; `auki-network`'s swarm refuses dials from
//! peers not present in `cluster.json` at the Noise layer. Stream protocol does
//! not introduce a new admission decision.

use crate::PEER_DERIVATION_LABEL;
use auki_identity::Wallet;
use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

// `Wallet`/`PEER_DERIVATION_LABEL` are unused by this module's runtime code;
// they're imported so the doctest below resolves. Strip if Rust complains.
#[allow(dead_code)]
fn _phantom() -> Option<Wallet> {
    Some(Wallet::from_seed(&[0u8; 32]).derive_child(PEER_DERIVATION_LABEL))
}

/// libp2p protocol id for grimsby's typed-byte-stream protocol. Stable; do not
/// change without coordinating with consumers (Boosterapp, Sentinel, Park) and
/// any cross-language reimplementation.
pub const STREAM_PROTOCOL: &str = "/auki/stream/1.0.0";

/// Maximum framed-message size on the wire, in bytes. Bounded so a peer cannot
/// drive an OOM by sending an arbitrarily-large length prefix; generous enough
/// to admit any reasonable single sensor frame.
///
/// 16 MiB. JPEG frames (grimsby v1) typically run 10–100 KB; raw NV12 frames
/// (future Stream<SensorLogEntry>) at K1 resolutions run ~400 KB; point cloud
/// frames after server-side decimation are usually well under 1 MB.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

// ─── Wire types ──────────────────────────────────────────────────────────────

/// First message on a fresh substream. The initiator names the sensor it's
/// asking for; the responder's `stream_provider` decides accept / decline.
///
/// (grimsby D2 RESOLVED 2026-05-05): `sensor_id` is authoritative for v1.
/// Topic-based addressing (capability labels like `rgb/head_left`) is parallel
/// work tracked in the SDK root parking lot under the Layer 2 capability
/// protocol; can be added as an optional sibling field on this struct without a
/// wire-version bump (additive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamRequest {
    /// Producer-scoped sensor identifier — same string the consumer learns
    /// from the producer's `/api/state.recordings[].sensor_id` plus the
    /// matching `/api/registries/sensors/<id>/<hash>` registry lookup.
    /// Example: `"K1-AABBCCDDEEFF/head_left_cam"`.
    pub sensor_id: String,
}

/// Per-substream message envelope. Generic over payload `T`; grimsby v1
/// instantiates with [`JpegFrame`].
///
/// `#[serde(tag = "kind", rename_all = "snake_case")]` puts the variant
/// discriminator in a `kind` field, making the wire shape easy to inspect:
///
/// ```text
/// {"kind":"request","sensor_id":"K1-.../head_left_cam"}
/// {"kind":"accept","sensor_hash":"…","clock_id":"…","clock_hash":"…"}
/// {"kind":"decline","reason":{"kind":"sensor_not_found"}}
/// {"kind":"frame","timestamp_ns":12345,"seq":0,"payload":{ … }}
/// {"kind":"end_of_stream","reason":{"kind":"source_ended"}}
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamMessage<T> {
    /// Initiator's request descriptor. First message on the substream.
    Request(StreamRequest),
    /// Responder's accept reply with the registry hashes the consumer needs to
    /// interpret subsequent frames. For self-describing payloads (JPEG, per
    /// grimsby D4 v1) `sensor_hash` is informational; for typed Log<T>
    /// payloads (raw NV12, point clouds, poses) it's load-bearing.
    Accept(AcceptInfo),
    /// Responder declines the request with a typed reason. Substream closes
    /// after this message; consumer's app-level policy decides whether to
    /// re-request later (grimsby D5c — SDK never silently retries).
    Decline { reason: DeclineReason },
    /// Typed payload frame. `timestamp_ns` lives on the producer's `clock_id`
    /// (carried in the preceding `Accept`). `seq` enables consumer-side drop
    /// detection without depending on regular frame intervals.
    Frame {
        /// Producer-side timestamp on the clock identified by
        /// `AcceptInfo.clock_id`. Monotonically nondecreasing on a healthy
        /// producer.
        timestamp_ns: i64,
        /// Producer-side sequence number, starting at 0 for the first frame
        /// after `Accept` and incrementing by 1 per frame. Consumer compares
        /// the gap between successive `seq` values to detect drops.
        seq: u64,
        /// Typed frame payload. For grimsby v1: [`JpegFrame`].
        payload: T,
    },
    /// Responder cleanly ends the stream. Substream closes after this message.
    /// "Substream closed without EndOfStream" is also a valid end signal —
    /// consumer treats it as an implicit `ConnectionLost`-equivalent (grimsby
    /// D5b — implicit via libp2p disconnect).
    EndOfStream { reason: EndReason },
}

/// Registry hashes the consumer uses to interpret subsequent frames. For
/// grimsby v1 (`T = JpegFrame`, self-describing) `sensor_hash` is informational
/// and used by the consumer's UI to label which physical sensor the preview is
/// of; for future Stream<T> instantiations carrying typed Log<T> payloads
/// (`SensorLogEntry`, `PointCloudLogEntry`, `PoseLogEntry`) it's load-bearing
/// because the per-frame bytes are uninterpretable without the SensorRegistry
/// entry it pins. `clock_id` + `clock_hash` are load-bearing in v1 — they tell
/// the consumer what clock the per-frame `timestamp_ns` lives on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptInfo {
    /// Hash of the Sensor Registry entry that describes the sensor producing
    /// these frames. Same hash the consumer would resolve via the producer's
    /// `/api/registries/sensors/<id>/<hash>` HTTP endpoint.
    pub sensor_hash: String,
    /// Identifier of the clock on which `Frame.timestamp_ns` is stamped. Same
    /// id space as the producer's `/api/info.session_clock_id` and the manifest
    /// `clock_id` field of the producer's Sensor Logs.
    pub clock_id: String,
    /// Hash of the matching Clock Registry entry.
    pub clock_hash: String,
}

/// Typed reasons the responder can decline a stream request. App-level policy
/// (per grimsby D3) — the SDK transports the variant; the responder's
/// `stream_provider` chooses which to return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeclineReason {
    /// Producer doesn't expose the named sensor.
    SensorNotFound,
    /// Producer recognizes the sensor but is currently not producing frames
    /// (capture paused, sensor unhealthy).
    SensorUnavailable,
    /// Producer is shutting down; consumer should avoid immediate retry.
    ProducerShuttingDown,
    /// Producer-supplied free-form reason. Use sparingly — the named variants
    /// above are easier for consumers to act on programmatically.
    Other { detail: String },
}

/// Typed reasons the responder ends a stream cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EndReason {
    /// The producer's source-Stream returned `None` — frames are no longer
    /// available on this sensor for this session. Most common cause: the
    /// recording the consumer was tailing was stopped; or the producer chose
    /// to end the subscription deliberately.
    SourceEnded,
    /// The producer is shutting down (daemon quit). Consumer should not retry
    /// against this peer until it observes a fresh `session_id`.
    ProducerShuttingDown,
    /// The producer's `session_id` changed mid-stream (daemon restart from the
    /// consumer's perspective). Per grimsby D5b, a session change invalidates
    /// every open stream; consumer can re-request after observing the new
    /// session_id via `ParticipantInfo`.
    SessionEnded,
    /// Producer-side internal error. Detail is for debugging; consumers should
    /// treat this like any other end-of-stream and decide whether to retry.
    ProducerError { detail: String },
}

// ─── Payload (grimsby v1) ────────────────────────────────────────────────────

/// JPEG-encoded preview frame — the payload `T` instantiated for grimsby v1
/// (D4 RESOLVED 2026-05-05). Byte-identical to what `GET /api/preview/latest.jpg`
/// serves today over HTTP; consumer renders via `<img>` / Blob URL with no
/// SDK-side decode required.
///
/// Other Stream<T> instantiations later: `T = SensorLogEntry` for raw NV12,
/// `T = PointCloudLogEntry`, `T = PoseLogEntry`. Each is a different `T`
/// instantiated from the same [`StreamMessage`] envelope; the framing on the
/// wire is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JpegFrame {
    /// Raw JPEG bytes (the same byte sequence the producer would have served
    /// on `GET /api/preview/latest.jpg`).
    pub bytes: Vec<u8>,
}

/// Pointcloud payload for `T = PointCloudFrame` substreams ([Dagaz](https://www.notion.so/3585c8e96592805b8d83c89f849d3577) D2
/// = raw CDR). The producer forwards a CDR-encoded `PointCloud2` ROS 2
/// message verbatim; the consumer (Park, future Sentinel) parses CDR on
/// its side. The SDK doesn't decode or interpret these bytes — the
/// shape mirrors `JpegFrame { bytes }` for the same reason.
///
/// **Per-frame layout fields** (`row_step`, `height`, `width`, `is_dense`)
/// ride inside the CDR payload itself; the consumer pulls them from the
/// wire frame as it decodes. **Sensor bootstrap-time fields**
/// (`fields[]`, `point_step`, `is_bigendian`, `frame_rate_hz`) live in
/// the producer's `SensorBody::PointCloud` registry entry — see
/// `bootstrap_pointcloud` in [boosterapp's auki_capture.py](https://github.com/aukilabs/boosterapp/blob/develop/scripts/auki_capture.py).
///
/// ## Wire-side encoding
///
/// `bytes` is base64-encoded inside the JSON envelope via the
/// [`base64_bytes`] serde adapter. Without the adapter, `serde_json`
/// renders `Vec<u8>` as a JSON array of integers — about 4× the raw
/// byte size on the wire. Base64 is ~33% overhead vs raw, so a 736 KB
/// `PointCloud2` frame at ~30 Hz lands at ~30 MB/s on the wire instead
/// of ~80 MB/s. JpegFrame keeps the array-of-integers form — JPEGs are
/// 1000× smaller, the 4× tax there is tolerable, and changing JpegFrame
/// would break grimsby v1 wire compat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointCloudFrame {
    /// Raw CDR-encoded `PointCloud2` message bytes. Base64-encoded
    /// inside the JSON envelope by the serde adapter — see [`base64_bytes`].
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

/// Serde adapter: serialize `Vec<u8>` as a base64-string field inside
/// JSON, parse it back on deserialize. Used by [`PointCloudFrame::bytes`]
/// to dodge `serde_json`'s array-of-integers encoding for `Vec<u8>`
/// (~4× wire overhead). Base64 round-trip is lossless and ~33% overhead
/// vs raw bytes — the smallest of the three exits the auki-network
/// parking lot pre-spec'd for the JSON-of-binary tax.
mod base64_bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as B64;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&B64.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        B64.decode(&s).map_err(serde::de::Error::custom)
    }
}

// ─── Errors ──────────────────────────────────────────────────────────────────

/// Failure modes for [`read_message`] / [`write_message`].
#[derive(Debug, thiserror::Error)]
pub enum StreamProtocolError {
    /// Underlying I/O on the libp2p substream failed.
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    /// JSON serialization failed (write side; almost always a bug — types in
    /// this module are designed to round-trip).
    #[error("serialize: {0}")]
    Serialize(#[source] serde_json::Error),
    /// JSON deserialization failed (read side; peer sent malformed bytes or a
    /// version-incompatible payload).
    #[error("deserialize: {0}")]
    Deserialize(#[source] serde_json::Error),
    /// Length prefix exceeds [`MAX_FRAME_BYTES`]. Peer is either malformed,
    /// malicious, or speaks a future protocol version with a wider cap.
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
    /// Length prefix is zero. Defined out — the smallest valid JSON payload
    /// is 4 bytes (`null`).
    #[error("frame is empty (length prefix is zero)")]
    EmptyFrame,
}

// ─── Framing helpers ─────────────────────────────────────────────────────────

/// Write a single [`StreamMessage<T>`] to `stream`, length-prefixed.
///
/// Performs three I/O writes (4-byte length, payload, flush) so a partially-
/// written frame can't sit half-buffered while the peer waits. On error the
/// stream's state is undefined — callers should drop the substream rather than
/// reuse it.
pub async fn write_message<T, S>(
    stream: &mut S,
    msg: &StreamMessage<T>,
) -> Result<(), StreamProtocolError>
where
    T: Serialize,
    S: AsyncWriteExt + Unpin,
{
    let bytes = serde_json::to_vec(msg).map_err(StreamProtocolError::Serialize)?;
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

/// Read a single [`StreamMessage<T>`] from `stream`. Reads exactly one frame:
/// 4-byte length prefix, then `len` bytes of JSON, then deserializes.
///
/// On error the stream's state is undefined — callers should drop the substream
/// rather than reuse it. End-of-stream from the peer surfaces as
/// `Err(StreamProtocolError::Io(e))` with `e.kind() == UnexpectedEof`.
pub async fn read_message<T, S>(stream: &mut S) -> Result<StreamMessage<T>, StreamProtocolError>
where
    T: DeserializeOwned,
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
    serde_json::from_slice(&payload).map_err(StreamProtocolError::Deserialize)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::Cursor;

    #[test]
    fn protocol_id_is_locked() {
        // Wire format. If this test fails, you're looking at a breaking
        // change — coordinate with Boosterapp, Sentinel, Park, and any
        // cross-language reimplementation before touching it.
        assert_eq!(STREAM_PROTOCOL, "/auki/stream/1.0.0");
    }

    #[test]
    fn max_frame_bytes_is_locked() {
        // 16 MiB. Documented in the module docstring + AcceptInfo layout.
        // A bump is a wire-compat decision: a producer running a higher cap
        // can still talk to a consumer running a lower cap (the consumer
        // will fail closed on oversized frames), but a consumer running a
        // higher cap may receive valid frames an older consumer rejects.
        assert_eq!(MAX_FRAME_BYTES, 16 * 1024 * 1024);
    }

    #[test]
    fn request_message_round_trips_through_json() {
        let msg: StreamMessage<JpegFrame> = StreamMessage::Request(StreamRequest {
            sensor_id: "K1-AABBCCDDEEFF/head_left_cam".into(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        // Locking the wire shape — these strings are part of the protocol.
        assert!(json.contains(r#""kind":"request""#));
        assert!(json.contains(r#""sensor_id":"K1-AABBCCDDEEFF/head_left_cam""#));
        let back: StreamMessage<JpegFrame> = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn accept_message_round_trips_through_json() {
        let msg: StreamMessage<JpegFrame> = StreamMessage::Accept(AcceptInfo {
            sensor_hash: "abcdef".into(),
            clock_id: "K1-AABBCCDDEEFF/utc".into(),
            clock_hash: "deadbeef".into(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""kind":"accept""#));
        assert!(json.contains(r#""sensor_hash":"abcdef""#));
        let back: StreamMessage<JpegFrame> = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn decline_message_round_trips_through_json() {
        let msg: StreamMessage<JpegFrame> = StreamMessage::Decline {
            reason: DeclineReason::SensorNotFound,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""kind":"decline""#));
        assert!(json.contains(r#""reason":{"kind":"sensor_not_found"}"#));
        let back: StreamMessage<JpegFrame> = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn frame_message_round_trips_through_json() {
        let msg: StreamMessage<JpegFrame> = StreamMessage::Frame {
            timestamp_ns: 12_345_678_900,
            seq: 7,
            payload: JpegFrame {
                bytes: vec![0xff, 0xd8, 0xff, 0xe0, 0x00],
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""kind":"frame""#));
        assert!(json.contains(r#""seq":7"#));
        let back: StreamMessage<JpegFrame> = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn end_of_stream_message_round_trips_through_json() {
        let msg: StreamMessage<JpegFrame> = StreamMessage::EndOfStream {
            reason: EndReason::ProducerError {
                detail: "encoder died".into(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""kind":"end_of_stream""#));
        assert!(json.contains(r#""kind":"producer_error""#));
        assert!(json.contains(r#""detail":"encoder died""#));
        let back: StreamMessage<JpegFrame> = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[tokio::test]
    async fn write_then_read_round_trips_a_request() {
        // Smoke test: a single message survives the framing helpers in a
        // round-trip through an in-memory cursor.
        let msg: StreamMessage<JpegFrame> = StreamMessage::Request(StreamRequest {
            sensor_id: "test/sensor".into(),
        });

        let mut buf: Vec<u8> = Vec::new();
        write_message(&mut buf, &msg).await.unwrap();

        // Length prefix is the first 4 bytes; the encoded JSON length should
        // match.
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(len as usize, buf.len() - 4);

        let mut cursor = Cursor::new(buf);
        let back: StreamMessage<JpegFrame> = read_message(&mut cursor).await.unwrap();
        assert_eq!(back, msg);
    }

    #[tokio::test]
    async fn write_then_read_round_trips_a_full_session() {
        // A realistic message order: Request → Accept → Frame × 3 →
        // EndOfStream. Verify they all survive in the same buffer in order.
        let messages: Vec<StreamMessage<JpegFrame>> = vec![
            StreamMessage::Request(StreamRequest {
                sensor_id: "K1/cam_a".into(),
            }),
            StreamMessage::Accept(AcceptInfo {
                sensor_hash: "h1".into(),
                clock_id: "c1".into(),
                clock_hash: "h2".into(),
            }),
            StreamMessage::Frame {
                timestamp_ns: 1,
                seq: 0,
                payload: JpegFrame { bytes: vec![1, 2, 3] },
            },
            StreamMessage::Frame {
                timestamp_ns: 2,
                seq: 1,
                payload: JpegFrame { bytes: vec![4, 5, 6] },
            },
            StreamMessage::Frame {
                timestamp_ns: 3,
                seq: 2,
                payload: JpegFrame { bytes: vec![7, 8, 9] },
            },
            StreamMessage::EndOfStream {
                reason: EndReason::SourceEnded,
            },
        ];

        let mut buf: Vec<u8> = Vec::new();
        for msg in &messages {
            write_message(&mut buf, msg).await.unwrap();
        }

        let mut cursor = Cursor::new(buf);
        let mut received: Vec<StreamMessage<JpegFrame>> = Vec::new();
        for _ in 0..messages.len() {
            received.push(read_message(&mut cursor).await.unwrap());
        }
        assert_eq!(received, messages);
    }

    #[tokio::test]
    async fn read_rejects_oversized_frame_via_length_prefix() {
        // Construct a buffer claiming MAX_FRAME_BYTES + 1; the read should
        // refuse before allocating.
        let too_big = MAX_FRAME_BYTES as u64 + 1;
        let len = (too_big as u32).to_be_bytes();
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&len);
        // No payload; the read should error before trying to read the body.
        let mut cursor = Cursor::new(buf);
        let err: Result<StreamMessage<JpegFrame>, _> = read_message(&mut cursor).await;
        assert!(matches!(
            err,
            Err(StreamProtocolError::FrameTooLarge { actual, max })
                if actual == too_big && max == MAX_FRAME_BYTES as u64
        ));
    }

    #[tokio::test]
    async fn read_rejects_empty_frame() {
        let mut cursor = Cursor::new(0u32.to_be_bytes().to_vec());
        let err: Result<StreamMessage<JpegFrame>, _> = read_message(&mut cursor).await;
        assert!(matches!(err, Err(StreamProtocolError::EmptyFrame)));
    }

    #[tokio::test]
    async fn read_surfaces_eof_as_io_error() {
        // Empty buffer — read_exact on the length prefix fails with
        // UnexpectedEof.
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let err: Result<StreamMessage<JpegFrame>, _> = read_message(&mut cursor).await;
        match err {
            Err(StreamProtocolError::Io(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
            }
            other => panic!("expected Io(UnexpectedEof), got {other:?}"),
        }
    }

    #[test]
    fn write_rejects_oversized_payload_before_io() {
        // serde_json size estimate: a JpegFrame with N bytes serializes as
        // approximately 4N (each byte becomes "123,"), so we need a payload
        // big enough that the JSON exceeds MAX_FRAME_BYTES. 16 MiB / 4 = 4
        // MiB of bytes is the rough floor; use 8 MiB to be safe.
        let huge = vec![0u8; 8 * 1024 * 1024];
        let msg: StreamMessage<JpegFrame> = StreamMessage::Frame {
            timestamp_ns: 0,
            seq: 0,
            payload: JpegFrame { bytes: huge },
        };
        let result = futures::executor::block_on(async {
            let mut buf: Vec<u8> = Vec::new();
            write_message(&mut buf, &msg).await
        });
        assert!(matches!(
            result,
            Err(StreamProtocolError::FrameTooLarge { .. })
        ));
    }

    // ─── PointCloudFrame (Dagaz Batch 1) ─────────────────────────────────

    /// `PointCloudFrame.bytes` serializes via the [`base64_bytes`]
    /// adapter as a JSON string (not an array of integers like
    /// `JpegFrame.bytes`). Pin the on-the-wire shape so a refactor of
    /// the adapter doesn't silently change the format.
    #[test]
    fn point_cloud_frame_serializes_bytes_as_base64_string() {
        let frame = PointCloudFrame {
            bytes: vec![0x00, 0x01, 0x02, 0x03, 0xff, 0xfe],
        };
        let json = serde_json::to_string(&frame).unwrap();
        // base64 of 0x00,0x01,0x02,0x03,0xff,0xfe is "AAECA//+".
        assert_eq!(json, r#"{"bytes":"AAECA//+"}"#);

        // And it round-trips losslessly.
        let back: PointCloudFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(back, frame);
    }

    /// Lossless round-trip on a representative-size CDR-shaped payload
    /// (1 KB of pseudo-random bytes). Confirms the base64 path doesn't
    /// truncate / pad / mis-align on non-trivial inputs.
    #[test]
    fn point_cloud_frame_round_trips_a_kilobyte_payload() {
        let bytes: Vec<u8> = (0..1024u32).map(|i| (i * 37 + 11) as u8).collect();
        let frame = PointCloudFrame {
            bytes: bytes.clone(),
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: PointCloudFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(back.bytes, bytes);
    }

    /// Wire-compat pin against the JSON-of-binary tax. For a 1 KB
    /// payload, base64 encoding produces ~1.37 KB of JSON, while
    /// `serde_json`'s array-of-integers encoding for `Vec<u8>` would
    /// produce ~3.4 KB (each byte averaging 3.5 ASCII chars + comma).
    /// If this assertion ever flips, someone removed the
    /// `#[serde(with = "base64_bytes")]` adapter and Dagaz's bandwidth
    /// reasoning is silently broken.
    #[test]
    fn point_cloud_frame_wire_size_dodges_the_array_of_integers_tax() {
        let bytes = vec![0xAB; 1024];
        let frame = PointCloudFrame { bytes };
        let json = serde_json::to_string(&frame).unwrap();
        // Base64 path: ~1.37 KB.
        assert!(
            json.len() < 1500,
            "base64 path should produce < 1.5 KB for 1 KB input; got {}",
            json.len()
        );
        // Sanity: definitely smaller than the array-of-integers path
        // (~3.4 KB for the same input — `[171,171,171,...]`).
        assert!(json.len() < 3000, "must be smaller than array-of-ints");
    }

    /// Locked cross-language conformance vector for the
    /// `PointCloudFrame` wire shape. A cross-language reimplementation
    /// (Park's browser-side decoder, future Sentinel-as-consumer in
    /// another runtime) is correct only if it produces these exact JSON
    /// bytes from the same `(timestamp_ns, seq, bytes)` tuple, and
    /// parses these exact bytes back to the same tuple.
    ///
    /// Pinned values:
    /// - `timestamp_ns: 1_700_000_000_000_000_000`
    /// - `seq: 42`
    /// - `bytes`: the 8 bytes `[0x42, 0x43, 0x44, 0x45, 0x00, 0x01, 0xfe, 0xff]`
    /// - base64(bytes) = "QkNERQAB/v8="
    #[test]
    fn locked_point_cloud_frame_wire_shape_vector() {
        let payload = PointCloudFrame {
            bytes: vec![0x42, 0x43, 0x44, 0x45, 0x00, 0x01, 0xfe, 0xff],
        };
        let msg: StreamMessage<PointCloudFrame> = StreamMessage::Frame {
            timestamp_ns: 1_700_000_000_000_000_000,
            seq: 42,
            payload: payload.clone(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let expected = r#"{"kind":"frame","timestamp_ns":1700000000000000000,"seq":42,"payload":{"bytes":"QkNERQAB/v8="}}"#;
        assert_eq!(
            json, expected,
            "PointCloudFrame wire shape drifted — see crate docs for the locked recipe",
        );

        // And the reverse — these exact bytes parse back to the same
        // typed message. A cross-language consumer that emits the same
        // string from the same input is correct on both directions.
        let back: StreamMessage<PointCloudFrame> = serde_json::from_str(expected).unwrap();
        match back {
            StreamMessage::Frame {
                timestamp_ns,
                seq,
                payload: parsed,
            } => {
                assert_eq!(timestamp_ns, 1_700_000_000_000_000_000);
                assert_eq!(seq, 42);
                assert_eq!(parsed, payload);
            }
            _ => panic!("expected Frame variant"),
        }
    }

    /// Through the framing helpers (length prefix + JSON body), a
    /// `PointCloudFrame` round-trips. Different from the JSON-only
    /// round-trip above because this exercises the wire-level
    /// `write_message` / `read_message` path that the runtime actually
    /// uses for transport.
    #[test]
    fn point_cloud_frame_round_trips_through_framing_helpers() {
        let payload = PointCloudFrame {
            bytes: (0..256u32).map(|i| (i * 31 + 7) as u8).collect(),
        };
        let original: StreamMessage<PointCloudFrame> = StreamMessage::Frame {
            timestamp_ns: 12_345_678_900,
            seq: 7,
            payload: payload.clone(),
        };

        let mut buf: Vec<u8> = Vec::new();
        futures::executor::block_on(async { write_message(&mut buf, &original).await }).unwrap();

        let mut reader = Cursor::new(buf);
        let parsed: StreamMessage<PointCloudFrame> =
            futures::executor::block_on(async { read_message(&mut reader).await }).unwrap();
        match parsed {
            StreamMessage::Frame {
                timestamp_ns,
                seq,
                payload: p,
            } => {
                assert_eq!(timestamp_ns, 12_345_678_900);
                assert_eq!(seq, 7);
                assert_eq!(p, payload);
            }
            _ => panic!("expected Frame variant"),
        }
    }
}
