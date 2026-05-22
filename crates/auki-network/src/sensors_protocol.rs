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
//! Length-prefixed JSON, same framing as the other Hagall protocols.
//! [`MAX_SENSORS_FRAME_BYTES`] caps each side at 512 KiB. A plain
//! catalog of a few dozen sensors is much smaller; the larger cap
//! leaves room for optional embedded Sensor / Frame Registry JSON while
//! still defending against malformed senders.

use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// libp2p protocol id for "what sensors are you currently
/// publishing?". Stable; bump version only on an incompatible
/// wire-shape change.
pub const SENSORS_PROTOCOL: &str = "/auki/sensors/0.0.1";

/// Cap on a single framed message. 512 KiB — a plain
/// `SensorsResponse` with a few dozen `SensorEntry` rows is tiny, and
/// optional embedded Sensor / Frame Registry entries still fit
/// comfortably under this cap. The limit is defense against malformed
/// senders.
pub const MAX_SENSORS_FRAME_BYTES: u32 = 512 * 1024;

/// Body of the request the initiator sends.
///
/// Default `{}` preserves the original catalog-only request. Set
/// `include_registry_entries` to ask the producer to attach the exact
/// Sensor Registry JSON for each row when it has it locally. Set
/// `include_frame_entries` to additionally attach the exact Frame
/// Registry JSON referenced by spatial sensor bodies (`camera` and
/// `point_cloud`). Receivers MUST tolerate unknown future fields
/// (serde JSON is permissive by default).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SensorsRequest {
    /// Include canonical Sensor Registry JSON in
    /// [`SensorEntry::sensor_entry_json`] when the producer can resolve
    /// `sensor_id + sensor_hash` locally.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_registry_entries: bool,
    /// Include canonical Frame Registry JSON in
    /// [`SensorEntry::frame_entry_json`] for spatial sensors when the
    /// producer can resolve the frame reference from the sensor entry.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_frame_entries: bool,
}

impl SensorsRequest {
    /// Catalog-only request. Serializes as `{}` for v0 compatibility.
    pub fn catalog() -> Self {
        Self::default()
    }

    /// Ask for sensor registry entries embedded by value.
    pub fn with_registry_entries() -> Self {
        Self {
            include_registry_entries: true,
            include_frame_entries: false,
        }
    }

    /// Ask for sensor registry entries and their referenced frame
    /// registry entries embedded by value.
    pub fn with_frame_entries() -> Self {
        Self {
            include_registry_entries: true,
            include_frame_entries: true,
        }
    }
}

/// Body of the response the responder sends. Carries the snapshot of
/// sensors the producer is currently publishing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SensorsResponse {
    /// The producer's current catalog. Empty list = "I have a
    /// catalog, and right now it's empty" (NOT an error — a daemon
    /// that's started but hasn't mounted any sensors yet is a valid
    /// state).
    pub sensors: Vec<SensorEntry>,
}

/// One row in a [`SensorsResponse`].
///
/// Lightweight by default: consumers that need the full
/// sensor-registry entry can fetch it separately via
/// `/auki/registries/0.0.1` using `sensor_id + sensor_hash`. Callers
/// that want fewer round trips can request embedded registry JSON by
/// value with [`SensorsRequest::with_registry_entries`] or
/// [`SensorsRequest::with_frame_entries`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SensorEntry {
    /// Producer-scoped sensor identifier (e.g.
    /// `"K1-LIVE01/head_left_cam"`). Stable for the lifetime of the
    /// producer's session. Pair with `peer_id` from
    /// `/auki/info/0.0.1` for cluster-wide uniqueness.
    pub sensor_id: String,
    /// Content-addressed hash pinning the registry entry that
    /// describes this sensor's geometry. Empty string if the
    /// producer hasn't registered it yet. Allows Park (and any
    /// consumer) to fetch the full `SensorRegistryEntry` from
    /// `auki-registry` separately.
    pub sensor_hash: String,
    /// Sensor kind — the `#[serde(tag = "type")]` value from
    /// `auki_registry::SensorBody` flowed through verbatim. Current
    /// SensorBody variants emit `"camera"`, `"point_cloud"`,
    /// `"joint_encoders"`, `"audio"`. Open string by contract:
    /// new SensorBody variants flow through without a wire bump, and
    /// consumers MUST handle unrecognised kinds gracefully (e.g. a
    /// generic-tile fallback) rather than reject. For tile-renderer
    /// dispatch in operator UIs — Park's session viewer uses it to
    /// pick a JPEG renderer vs. point-cloud renderer vs.
    /// joint-encoder gauge.
    ///
    /// Renaming a `SensorBody` variant is a coordinated registry +
    /// sensors-protocol wire break (the tag flows verbatim through
    /// this field); the four current tags above are pinned to keep
    /// such a rename loud.
    pub kind: String,
    /// Optional canonical Sensor Registry JSON matching
    /// `sensor_id + sensor_hash`. Present only when requested and
    /// available from the producer's registered app root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_entry_json: Option<String>,
    /// Optional canonical Frame Registry JSON matching the
    /// `frame_id + frame_hash` referenced by the embedded or locally
    /// resolved sensor registry entry. Present only for spatial
    /// sensors when requested and available from the producer's
    /// registered app root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_entry_json: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Failure modes for the framed read/write helpers below.
#[derive(Debug, Error)]
pub enum SensorsProtocolError {
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
    /// Length prefix exceeds [`MAX_SENSORS_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write a [`SensorsRequest`] to `stream`, length-prefixed JSON.
pub async fn write_sensors_request<S>(
    stream: &mut S,
    msg: &SensorsRequest,
) -> Result<(), SensorsProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Write a [`SensorsResponse`] to `stream`, length-prefixed JSON.
pub async fn write_sensors_response<S>(
    stream: &mut S,
    msg: &SensorsResponse,
) -> Result<(), SensorsProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Read a [`SensorsRequest`] from `stream`.
pub async fn read_sensors_request<S>(stream: &mut S) -> Result<SensorsRequest, SensorsProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

/// Read a [`SensorsResponse`] from `stream`.
pub async fn read_sensors_response<S>(
    stream: &mut S,
) -> Result<SensorsResponse, SensorsProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

async fn write_json<S, T>(stream: &mut S, msg: &T) -> Result<(), SensorsProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(msg).map_err(SensorsProtocolError::Encode)?;
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

async fn read_json<S, T>(stream: &mut S) -> Result<T, SensorsProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(SensorsProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(SensorsProtocolError::EmptyFrame);
    }
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
    serde_json::from_slice(&payload).map_err(SensorsProtocolError::Decode)
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

    #[test]
    fn default_request_serializes_as_empty_object() {
        let json = serde_json::to_string(&SensorsRequest::catalog()).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn detail_request_serializes_requested_flags() {
        let json = serde_json::to_string(&SensorsRequest::with_frame_entries()).unwrap();
        assert_eq!(
            json,
            r#"{"include_registry_entries":true,"include_frame_entries":true}"#
        );
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

    /// Future-compat: a `SensorsRequest` with extra fields decodes
    /// cleanly into today's empty struct (serde ignores unknown
    /// fields). Lets us add filter fields like `kind` later without a
    /// protocol-id bump.
    #[test]
    fn request_decodes_with_future_unknown_fields() {
        let forward_json = r#"{"kind":"camera","since_session_now_ns":12345}"#;
        let back: SensorsRequest = serde_json::from_str(forward_json).unwrap();
        assert_eq!(back, SensorsRequest::default());
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
