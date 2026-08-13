//! `/auki/registries/0.3.0` — libp2p protocol for listing and fetching a
//! peer's content-addressed registry entries over the cluster's libp2p plane.
//!
//! ## Why this exists
//!
//! Stream manifests and catalogs carry `(id, hash)` references into
//! the Sensor / Clock / Frame registries. Consumers need the matching
//! registry entry to interpret stream bytes: sensor body for payload
//! shape, clock body for timestamps, frame body for coordinate
//! convention. Pre-Hagall, apps could paper this over with daemon HTTP
//! endpoints; the SDK-owned cluster path needs the same resolution over
//! libp2p so the cluster trust boundary is the only one metadata flows
//! through.
//!
//! Device models are the same kind of object: immutable, content-addressed
//! registry JSON. Discovery is a [`RegistryRequest::List`], not a resource
//! catalog.
//!
//! ## Shape
//!
//! Request-response over one substream. Tagged request/response:
//!
//! ```text
//! Get:  RegistryRequest::Get { kind, id, hash }
//!       → RegistryResponse::Get { entry: Some(...) | None }
//!
//! List: RegistryRequest::List { kind }
//!       → RegistryResponse::List { entries: [{ id, hash }, ...] }
//! ```
//!
//! `Get` with `entry: None` means the peer understood the protocol but does
//! not have that exact entry. Transport failures, malformed frames, and
//! timeouts remain protocol/request errors at the runtime layer.
//!
//! ## Wire format
//!
//! Length-prefixed JSON, same framing as the other Hagall protocols.
//! The inner `canonical_json` is the registry entry's canonical UTF-8
//! JSON string — the bytes whose XXH3-128 hash is named by `hash`.

use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// libp2p protocol id for content-addressed registry list + get.
/// Stable; bump version only on an incompatible wire-shape change.
pub const REGISTRIES_PROTOCOL: StreamProtocol = StreamProtocol::new("/auki/registries/0.3.0");

/// Cap on a single framed message. Registry entries are tiny today
/// (~100s of bytes), but 64 KiB leaves room for future sensor bodies
/// while keeping malformed senders bounded.
pub const MAX_REGISTRIES_FRAME_BYTES: u32 = 64 * 1024;

/// Which registry namespace a request addresses.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegistryKind {
    /// Sensor Registry (`SensorRegistryEntry`).
    Sensor,
    /// Clock Registry (`ClockRegistryEntry`).
    Clock,
    /// Frame Registry (`FrameRegistryEntry`).
    Frame,
    /// Detector Registry (`DetectorRegistryEntry`).
    Detector,
    /// Map Registry (`MapRegistryEntry`).
    Map,
    /// Device Model Registry (`DeviceModelRegistryEntry`).
    DeviceModel,
}

impl RegistryKind {
    /// Stable lowercase label used in diagnostics and UI-facing
    /// errors. The wire shape is still serde's `snake_case` enum tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            RegistryKind::Sensor => "sensor",
            RegistryKind::Clock => "clock",
            RegistryKind::Frame => "frame",
            RegistryKind::Detector => "detector",
            RegistryKind::Map => "map",
            RegistryKind::DeviceModel => "device_model",
        }
    }
}

impl std::fmt::Display for RegistryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Body of the request the initiator sends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RegistryRequest {
    /// Fetch one exact content-addressed entry.
    Get {
        /// Registry namespace.
        kind: RegistryKind,
        /// Registry id (`sensor_id`, `clock_id`, `frame_id`, `detector_id`,
        /// `map_id`, or `device_model_id`).
        id: String,
        /// Expected XXH3-128 hash of the canonical JSON entry bytes.
        hash: String,
    },
    /// Enumerate `(id, hash)` pairs for one registry namespace.
    List {
        /// Registry namespace to list.
        kind: RegistryKind,
    },
}

impl RegistryRequest {
    /// Construct a Get request.
    pub fn get(kind: RegistryKind, id: impl Into<String>, hash: impl Into<String>) -> Self {
        Self::Get {
            kind,
            id: id.into(),
            hash: hash.into(),
        }
    }

    /// Construct a List request.
    pub fn list(kind: RegistryKind) -> Self {
        Self::List { kind }
    }
}

/// One row returned by [`RegistryResponse::List`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryListEntry {
    /// Registry id.
    pub id: String,
    /// XXH3-128 hash of the entry's canonical JSON bytes.
    pub hash: String,
}

/// Body of the response the responder sends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RegistryResponse {
    /// Reply to [`RegistryRequest::Get`].
    Get {
        /// `Some` when the peer has the exact entry. `None` means the
        /// peer reached the registry handler but does not have that
        /// `(kind, id, hash)` entry.
        entry: Option<RegistryEntryEnvelope>,
    },
    /// Reply to [`RegistryRequest::List`].
    List {
        /// Known `(id, hash)` pairs for the requested kind. Empty when
        /// none are published or the kind is not listed yet.
        entries: Vec<RegistryListEntry>,
    },
}

/// Returned registry entry payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryEntryEnvelope {
    /// Registry namespace of the returned entry.
    pub kind: RegistryKind,
    /// Registry id carried by the decoded entry.
    pub id: String,
    /// XXH3-128 hash of `canonical_json.as_bytes()`.
    pub hash: String,
    /// Canonical UTF-8 JSON for the typed registry entry. Consumers
    /// hash these bytes first, then decode into `SensorRegistryEntry`,
    /// `ClockRegistryEntry`, or `FrameRegistryEntry`.
    pub canonical_json: String,
}

/// Failure modes for the framed read/write helpers below.
#[derive(Debug, Error)]
pub enum RegistriesProtocolError {
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
    /// Length prefix exceeds [`MAX_REGISTRIES_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

/// Write a [`RegistryRequest`] to `stream`, length-prefixed JSON.
pub async fn write_registry_request<S>(
    stream: &mut S,
    msg: &RegistryRequest,
) -> Result<(), RegistriesProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Write a [`RegistryResponse`] to `stream`, length-prefixed JSON.
pub async fn write_registry_response<S>(
    stream: &mut S,
    msg: &RegistryResponse,
) -> Result<(), RegistriesProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Read a [`RegistryRequest`] from `stream`.
pub async fn read_registry_request<S>(
    stream: &mut S,
) -> Result<RegistryRequest, RegistriesProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

/// Read a [`RegistryResponse`] from `stream`.
pub async fn read_registry_response<S>(
    stream: &mut S,
) -> Result<RegistryResponse, RegistriesProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

async fn write_json<S, T>(stream: &mut S, msg: &T) -> Result<(), RegistriesProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(msg).map_err(RegistriesProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_REGISTRIES_FRAME_BYTES as u64 {
        return Err(RegistriesProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_REGISTRIES_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(RegistriesProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(RegistriesProtocolError::Io)?;
    stream.flush().await.map_err(RegistriesProtocolError::Io)?;
    Ok(())
}

async fn read_json<S, T>(stream: &mut S) -> Result<T, RegistriesProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(RegistriesProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(RegistriesProtocolError::EmptyFrame);
    }
    if len > MAX_REGISTRIES_FRAME_BYTES {
        return Err(RegistriesProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_REGISTRIES_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(RegistriesProtocolError::Io)?;
    serde_json::from_slice(&payload).map_err(RegistriesProtocolError::Decode)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_request_round_trips() {
        let req = RegistryRequest::get(RegistryKind::Frame, "K1-LIVE01/head_cam_points", "abc123");
        let mut buf = Vec::new();
        write_registry_request(&mut buf, &req).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_registry_request(&mut cursor).await.unwrap();
        assert_eq!(req, back);
    }

    #[tokio::test]
    async fn list_request_round_trips() {
        let req = RegistryRequest::list(RegistryKind::DeviceModel);
        let mut buf = Vec::new();
        write_registry_request(&mut buf, &req).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_registry_request(&mut cursor).await.unwrap();
        assert_eq!(req, back);
    }

    #[tokio::test]
    async fn get_response_round_trips() {
        let resp = RegistryResponse::Get {
            entry: Some(RegistryEntryEnvelope {
                kind: RegistryKind::Frame,
                id: "K1-LIVE01/head_cam_points".into(),
                hash: "abc123".into(),
                canonical_json: r#"{"frame_id":"K1-LIVE01/head_cam_points"}"#.into(),
            }),
        };
        let mut buf = Vec::new();
        write_registry_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_registry_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    #[tokio::test]
    async fn list_response_round_trips() {
        let resp = RegistryResponse::List {
            entries: vec![RegistryListEntry {
                id: "k1".into(),
                hash: "deadbeef".into(),
            }],
        };
        let mut buf = Vec::new();
        write_registry_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_registry_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    #[tokio::test]
    async fn missing_get_response_round_trips() {
        let resp = RegistryResponse::Get { entry: None };
        let mut buf = Vec::new();
        write_registry_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_registry_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
    }

    #[tokio::test]
    async fn read_rejects_oversized_length_prefix() {
        let mut buf = (MAX_REGISTRIES_FRAME_BYTES + 1).to_be_bytes().to_vec();
        buf.extend(std::iter::repeat(0u8).take(8));
        let mut cursor = futures::io::Cursor::new(buf);
        let err = read_registry_response(&mut cursor).await.unwrap_err();
        assert!(matches!(err, RegistriesProtocolError::FrameTooLarge { .. }));
    }

    /// Pins the envelope field names for cross-language consumers.
    #[test]
    fn wire_shape_locked_field_names() {
        let json = serde_json::to_string(&RegistryRequest::get(
            RegistryKind::Frame,
            "frame",
            "hash",
        ))
        .unwrap();
        assert!(json.contains(r#""op":"get""#), "{json}");
        assert!(json.contains(r#""kind":"frame""#), "{json}");
        assert!(json.contains(r#""id":"frame""#), "{json}");
        assert!(json.contains(r#""hash":"hash""#), "{json}");

        let list = serde_json::to_string(&RegistryRequest::list(RegistryKind::DeviceModel)).unwrap();
        assert!(list.contains(r#""op":"list""#), "{list}");
        assert!(list.contains(r#""kind":"device_model""#), "{list}");
    }
}
