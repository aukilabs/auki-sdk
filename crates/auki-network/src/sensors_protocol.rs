//! `/auki/sensors/0.0.1` — libp2p protocol for fetching a peer's
//! currently-published sensor catalog over the cluster's libp2p plane.
//!
//! ## Why this exists
//!
//! Operator UIs (Park's session viewer, Sentinel's status board) need
//! to know which sensors each cluster peer is currently publishing so
//! the user can pick a tile to mount in the viewer. Pre-Hagall, Park
//! resolved this via mDNS plus a daemon HTTP endpoint; per Hagall
//! constraint #6 ("NO FALLBACKS") that side channel is dead — every
//! piece of cluster-internal information must ride on libp2p so the
//! cluster's trust boundary is the only one any of it flows through.
//!
//! `/auki/info/0.0.1` already covers identity (the `ParticipantInfo`
//! shape: app, name, peer_id, is_manager, …). This protocol is the
//! complementary surface for the *catalog* — "what are you currently
//! publishing on `/auki/stream/0.1.0`?". Together they unblock Park's
//! sensor-chip row (currently rendering "awaiting SDK
//! /auki/sensors/0.0.1" until this protocol lands).
//!
//! ## Shape
//!
//! Request-response over one substream. Client opens, writes
//! [`SensorsRequest`] (optionally asking the producer to embed registry
//! entries by value), reads [`SensorsResponse`], closes.
//!
//! ```text
//! Initiator → Responder:  SensorsRequest { include_registry_entries?, include_frame_entries? }
//! Responder → Initiator:  SensorsResponse { sensors: [...] }
//! ```
//!
//! ## Trust boundary
//!
//! Inbound substreams from peers NOT on the runtime's allow-list are
//! silently dropped at the runtime layer, identically to
//! `/auki/info/0.0.1`. Non-cluster peers can't probe a daemon's
//! catalog — privacy by membership.
//!
//! ## Wire format
//!
//! Length-prefixed protobuf, same framing as the other Hagall protocols.
//! [`MAX_SENSORS_FRAME_BYTES`] caps each side at 512 KiB. A plain
//! catalog of a few dozen sensors is much smaller; the larger cap
//! leaves room for optional embedded Sensor / Frame Registry JSON while
//! still defending against malformed senders.

use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::StreamProtocol;
use prost::Message;
use thiserror::Error;

pub use auki_datatypes::sensors::{SensorEntry, SensorsRequest, SensorsResponse};

/// libp2p protocol id for "what sensors are you currently
/// publishing?". Stable; bump version only on an incompatible
/// wire-shape change.
pub const SENSORS_PROTOCOL: StreamProtocol = StreamProtocol::new("/auki/sensors/0.0.1");

/// Cap on a single framed message. 512 KiB — a plain
/// `SensorsResponse` with a few dozen `SensorEntry` rows is tiny, and
/// optional embedded Sensor / Frame Registry entries still fit
/// comfortably under this cap. The limit is defense against malformed
/// senders.
pub const MAX_SENSORS_FRAME_BYTES: u32 = 512 * 1024;

/// Failure modes for the framed read/write helpers below.
#[derive(Debug, Error)]
pub enum SensorsProtocolError {
    /// Underlying I/O on the libp2p substream failed.
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    /// Protobuf encode (write side) failed.
    #[error("encode: {0}")]
    Encode(#[source] prost::EncodeError),
    /// Protobuf decode (read side) failed.
    #[error("decode: {0}")]
    Decode(#[source] prost::DecodeError),
    /// Length prefix exceeds [`MAX_SENSORS_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write a [`SensorsRequest`] to `stream`, length-prefixed protobuf.
pub async fn write_sensors_request<S>(
    stream: &mut S,
    msg: &SensorsRequest,
) -> Result<(), SensorsProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_frame(stream, msg).await
}

/// Write a [`SensorsResponse`] to `stream`, length-prefixed protobuf.
pub async fn write_sensors_response<S>(
    stream: &mut S,
    msg: &SensorsResponse,
) -> Result<(), SensorsProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_frame(stream, msg).await
}

/// Read a [`SensorsRequest`] from `stream`.
pub async fn read_sensors_request<S>(stream: &mut S) -> Result<SensorsRequest, SensorsProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_frame(stream).await
}

/// Read a [`SensorsResponse`] from `stream`.
pub async fn read_sensors_response<S>(
    stream: &mut S,
) -> Result<SensorsResponse, SensorsProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_frame(stream).await
}

async fn write_frame<S, T>(stream: &mut S, msg: &T) -> Result<(), SensorsProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Message,
{
    let mut bytes = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut bytes)
        .map_err(SensorsProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_SENSORS_FRAME_BYTES as u64 {
        return Err(SensorsProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_SENSORS_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(SensorsProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(SensorsProtocolError::Io)?;
    stream.flush().await.map_err(SensorsProtocolError::Io)?;
    Ok(())
}

async fn read_frame<S, T>(stream: &mut S) -> Result<T, SensorsProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: Message + Default,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(SensorsProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_SENSORS_FRAME_BYTES {
        return Err(SensorsProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_SENSORS_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(SensorsProtocolError::Io)?;
    T::decode(&*payload).map_err(SensorsProtocolError::Decode)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_round_trips() {
        let req = SensorsRequest::default();
        let mut buf = Vec::new();
        write_sensors_request(&mut buf, &req).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_sensors_request(&mut cursor).await.unwrap();
        assert_eq!(req, back);
    }

    #[tokio::test]
    async fn sensors_response_payload_is_protobuf_not_json() {
        let resp = SensorsResponse {
            sensors: vec![SensorEntry {
                sensor_id: "audio".to_string(),
                sensor_hash: "hash".to_string(),
                kind: "audio".to_string(),
                sensor_entry_json: None,
                frame_entry_json: None,
            }],
        };
        let mut buf = Vec::new();
        write_sensors_response(&mut buf, &resp).await.unwrap();

        assert_ne!(
            buf.get(4),
            Some(&b'{'),
            "sensors protocol payload must be generated protobuf, not JSON"
        );
    }

    #[test]
    fn detail_request_encodes_requested_flags() {
        let bytes = SensorsRequest::with_frame_entries().encode_to_vec();
        assert_eq!(bytes, vec![0x08, 0x01, 0x10, 0x01]);
    }

    #[tokio::test]
    async fn response_round_trips() {
        let resp = SensorsResponse {
            sensors: vec![
                SensorEntry {
                    sensor_id: "K1-LIVE01/head_left_cam".into(),
                    sensor_hash: "abc123".into(),
                    kind: "camera".into(),
                    sensor_entry_json: None,
                    frame_entry_json: None,
                },
                SensorEntry {
                    sensor_id: "K1-LIVE01/lidar_top".into(),
                    sensor_hash: "".into(),
                    kind: "point_cloud".into(),
                    sensor_entry_json: None,
                    frame_entry_json: None,
                },
            ],
        };
        let mut buf = Vec::new();
        write_sensors_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_sensors_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    /// "I have a catalog, and right now it's empty" — a daemon that's
    /// started but hasn't mounted any sensors yet round-trips
    /// cleanly. Empty list is NOT an error.
    #[tokio::test]
    async fn empty_response_round_trips() {
        let resp = SensorsResponse { sensors: vec![] };
        let mut buf = Vec::new();
        write_sensors_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_sensors_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    #[tokio::test]
    async fn read_rejects_oversized_length_prefix() {
        let mut buf = (MAX_SENSORS_FRAME_BYTES + 1).to_be_bytes().to_vec();
        buf.extend(std::iter::repeat(0u8).take(8));
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_sensors_response(&mut cursor).await.unwrap_err();
        assert!(matches!(err, SensorsProtocolError::FrameTooLarge { .. }));
    }

    #[test]
    fn catalog_request_encodes_as_empty_protobuf_message() {
        assert!(SensorsRequest::catalog().encode_to_vec().is_empty());
    }

    #[tokio::test]
    async fn embedded_registry_json_round_trips() {
        let resp = SensorsResponse {
            sensors: vec![SensorEntry {
                sensor_id: "K1-LIVE01/lidar_top".into(),
                sensor_hash: "sensorhash".into(),
                kind: "point_cloud".into(),
                sensor_entry_json: Some(r#"{"sensor_id":"K1-LIVE01/lidar_top"}"#.into()),
                frame_entry_json: Some(r#"{"frame_id":"K1-LIVE01/base_link"}"#.into()),
            }],
        };
        let mut buf = Vec::new();
        write_sensors_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_sensors_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }
}
