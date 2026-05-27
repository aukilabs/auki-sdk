//! `/auki/resources/0.2.0` - libp2p protocol for fetching a peer's
//! current resource catalog over the cluster's libp2p plane.
//!
//! Resources are the generalized discovery layer. A peer answers "what can
//! I provide right now?" with a flat list of [`ResourceEntry`] rows. Each
//! row is discriminated by a `variant` field (one of `sensor_log`,
//! `pose_log`, `time_transform_log`, `detection_log`). The row also
//! carries common fields (`source_peer_id`, `writer_peer_id`,
//! `resource_id`, `state`, `head` | `extent`, `available`) plus
//! variant-specific optional blocks (`sensor`, `pose`) and a typed
//! `manifest` pointer block whose shape varies by variant.

use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// libp2p protocol id for "what resources can this peer provide
/// right now?". Stable; bump version only on an incompatible
/// wire-shape change.
pub const RESOURCES_PROTOCOL: StreamProtocol = StreamProtocol::new("/auki/resources/0.2.0");

/// Cap on a single framed message. 1 MiB leaves room for catalogs with
/// embedded registry JSON and several transform edges while bounding
/// malformed senders.
pub const MAX_RESOURCES_FRAME_BYTES: u32 = 1024 * 1024;

// ── ResourcesRequest / ResourcesResponse ────────────────────────────────────

/// Body of the request the initiator sends.
///
/// `variants` is an open filter — empty means "all variants this peer
/// offers". Unknown variants simply return no rows from peers that do
/// not produce them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourcesRequest {
    /// Optional variant filter. Empty = all variants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
}

impl ResourcesRequest {
    /// All advertised resources.
    pub fn all() -> Self {
        Self::default()
    }
}

/// Body of the response the responder sends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourcesResponse {
    /// Snapshot of resources the producer can currently provide.
    /// Empty list = "I have a resource catalog and it is empty".
    pub resources: Vec<ResourceEntry>,
}

// ── Variant (discriminator) ──────────────────────────────────────────────────

/// Closed enum identifying which log variant a catalog row describes.
/// Wire values are `snake_case` strings; unknown values are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    /// Row describes a sensor log stream.
    SensorLog,
    /// Row describes a pose log stream.
    PoseLog,
    /// Row describes a time-transform log stream.
    TimeTransformLog,
    /// Row describes a detection log stream.
    DetectionLog,
}

// ── SensorKind ───────────────────────────────────────────────────────────────

/// Closed enum for the high-level sensor family.
/// Wire values are `snake_case` strings; unknown values are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    /// Optical camera (RGB, depth, IR, …).
    Camera,
    /// Distance-measuring sensor (lidar, ultrasonic, …).
    Rangefinder,
    /// Radio-frequency sensor (UWB, WiFi CSI, …).
    Rf,
    /// Microphone or acoustic sensor.
    Audio,
    /// Articulated joint encoders.
    JointEncoders,
}

// ── Head / Extent / Available ────────────────────────────────────────────────

/// Head-behavior block for live catalog rows.
///
/// `Rolling` rows retain only a sliding window of data; `Fixed` rows
/// started at a known wall-clock timestamp and keep everything since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Head {
    /// Sliding-window retention: only the most-recent `retention_ns`
    /// nanoseconds of data are available.
    Rolling {
        /// Retention window in nanoseconds.
        retention_ns: i64,
    },
    /// Fixed-start head: all data since `started_at_ns` is available.
    Fixed {
        /// Wall-clock timestamp (ns) when the producer started writing.
        started_at_ns: i64,
    },
}

/// Sealed bounds block — the time extent of data that is currently
/// on-disk and retrievable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extent {
    /// Earliest available sample timestamp (ns, inclusive).
    pub start_at_ns: i64,
    /// Latest available sample timestamp (ns, inclusive).
    pub finish_at_ns: i64,
}

/// Snapshot of currently-retrievable data volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Available {
    /// Total compressed bytes available for retrieval.
    pub bytes: u64,
    /// Total number of log entries available.
    pub entries: u64,
    /// Duration covered by available data, in nanoseconds.
    pub duration_ns: i64,
}

// ── SensorBlock / PoseBlock ──────────────────────────────────────────────────

/// Content block for `sensor_log` variant rows.
///
/// Carries the closed `kind`, the open-string `type` (e.g. `"rgb"`,
/// `"point_cloud"`, `"depth"`), plus the content-addressed
/// `sensor_id` / `sensor_hash` pair from the Sensor Registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorBlock {
    /// Closed sensor family.
    pub kind: SensorKind,
    /// Open-string sensor type within the family (e.g. `"rgb"`, `"depth"`).
    #[serde(rename = "type")]
    pub r#type: String,
    /// Producer-scoped sensor identifier.
    pub sensor_id: String,
    /// Content-addressed Sensor Registry hash.
    pub sensor_hash: String,
}

/// Content block for `pose_log` variant rows.
///
/// Carries the writer-mode hint from the Pose Log manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoseBlock {
    /// Whether the transform is stationary (`rigid`) or time-varying (`movable`).
    pub writer_mode: auki_manifests::PoseWriterMode,
}

// ── Manifest pointer types ───────────────────────────────────────────────────

use auki_registry::{LogRef, RegistryRef};

/// Manifest pointer for `sensor_log` variant rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorManifestPointer {
    /// Clock registry reference.
    pub clock: RegistryRef,
    /// Optional frame registry reference (present when the sensor has a
    /// spatial frame).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<RegistryRef>,
}

/// Manifest pointer for `pose_log` variant rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoseManifestPointer {
    /// Parent / source frame registry reference.
    pub from_frame: RegistryRef,
    /// Child / target frame registry reference.
    pub to_frame: RegistryRef,
    /// Clock registry reference.
    pub clock: RegistryRef,
    /// Producer identity (inline).
    pub source: auki_manifests::PoseSource,
    /// Expected producer cadence in Hz. Zero means unspecified.
    pub expected_rate_hz: u32,
}

/// Manifest pointer for `time_transform_log` variant rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeTransformManifestPointer {
    /// Source clock registry reference.
    pub from_clock: RegistryRef,
    /// Target clock registry reference.
    pub to_clock: RegistryRef,
    /// Producer identity (inline).
    pub source: auki_manifests::TimeTransformSource,
}

/// Manifest pointer for `detection_log` variant rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectionManifestPointer {
    /// Detector registry reference.
    pub detector: RegistryRef,
    /// Input log reference (source_peer_id + resource_id).
    pub input_log: LogRef,
    /// Input sensor registry reference.
    pub input_sensor: RegistryRef,
    /// Clock registry reference.
    pub clock: RegistryRef,
}

// ── VariantContent enum ──────────────────────────────────────────────────────

/// Variant-specific fields flattened into [`ResourceEntry`].
///
/// The `variant` tag is flattened to the top level of the resource row
/// alongside `source_peer_id`, `writer_peer_id`, etc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "variant", rename_all = "snake_case")]
pub enum VariantContent {
    /// Sensor log row.
    SensorLog { manifest: SensorManifestPointer },
    /// Pose log row.
    PoseLog { manifest: PoseManifestPointer },
    /// Time-transform log row.
    TimeTransformLog {
        manifest: TimeTransformManifestPointer,
    },
    /// Detection log row.
    DetectionLog { manifest: DetectionManifestPointer },
}

// ── ResourceEntry ────────────────────────────────────────────────────────────

/// One row in a [`ResourcesResponse`].
///
/// `variant` (from the flattened [`VariantContent`]) discriminates the
/// row type. `sensor` is present only on `sensor_log` rows; `pose` only
/// on `pose_log` rows. `head` and `extent` are mutually exclusive — live
/// streams carry `head`, sealed archives carry `extent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceEntry {
    /// Peer whose physical sensors or logs this row describes.
    pub source_peer_id: String,
    /// Peer that holds the materialized log bytes (may differ from
    /// `source_peer_id` when Park materializes a galbot stream).
    pub writer_peer_id: String,
    /// Stable resource identifier scoped to `source_peer_id`.
    pub resource_id: String,
    /// Open-string lifecycle state; v1 values: `"live"` | `"sealed"`.
    pub state: String,
    /// Present on live rows (`state = "live"`). Describes the rolling or
    /// fixed-start retention window.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub head: Option<Head>,
    /// Present on sealed rows (`state = "sealed"`). Describes the
    /// inclusive time bounds of the archived data.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extent: Option<Extent>,
    /// Current data volume snapshot.
    pub available: Available,
    /// Sensor-family metadata. Only present on `sensor_log` rows.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sensor: Option<SensorBlock>,
    /// Pose writer-mode metadata. Only present on `pose_log` rows.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pose: Option<PoseBlock>,
    /// Variant discriminator and manifest pointer, flattened to row top level.
    #[serde(flatten)]
    pub variant_content: VariantContent,
}

// ── Protocol error ───────────────────────────────────────────────────────────

/// Failure modes for the framed read/write helpers below.
#[derive(Debug, Error)]
pub enum ResourcesProtocolError {
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
    /// Length prefix exceeds [`MAX_RESOURCES_FRAME_BYTES`].
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

// ── Wire helpers ─────────────────────────────────────────────────────────────

/// Write a [`ResourcesRequest`] to `stream`, length-prefixed JSON.
pub async fn write_resources_request<S>(
    stream: &mut S,
    msg: &ResourcesRequest,
) -> Result<(), ResourcesProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Write a [`ResourcesResponse`] to `stream`, length-prefixed JSON.
pub async fn write_resources_response<S>(
    stream: &mut S,
    msg: &ResourcesResponse,
) -> Result<(), ResourcesProtocolError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, msg).await
}

/// Read a [`ResourcesRequest`] from `stream`.
pub async fn read_resources_request<S>(
    stream: &mut S,
) -> Result<ResourcesRequest, ResourcesProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

/// Read a [`ResourcesResponse`] from `stream`.
pub async fn read_resources_response<S>(
    stream: &mut S,
) -> Result<ResourcesResponse, ResourcesProtocolError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

async fn write_json<S, T>(stream: &mut S, msg: &T) -> Result<(), ResourcesProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(msg).map_err(ResourcesProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_RESOURCES_FRAME_BYTES as u64 {
        return Err(ResourcesProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_RESOURCES_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(ResourcesProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(ResourcesProtocolError::Io)?;
    stream.flush().await.map_err(ResourcesProtocolError::Io)?;
    Ok(())
}

async fn read_json<S, T>(stream: &mut S) -> Result<T, ResourcesProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(ResourcesProtocolError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(ResourcesProtocolError::EmptyFrame);
    }
    if len > MAX_RESOURCES_FRAME_BYTES {
        return Err(ResourcesProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_RESOURCES_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(ResourcesProtocolError::Io)?;
    serde_json::from_slice(&payload).map_err(ResourcesProtocolError::Decode)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use auki_manifests::{PoseSource, PoseWriterMode, TimeTransformSource};
    use auki_registry::{LogRef, RegistryRef};

    // ── New ResourceEntry shape (post-#216 §1) ──────────────────────────

    #[test]
    fn sensor_log_row_canonical() {
        let row = ResourceEntry {
            source_peer_id: "galbot".to_string(),
            writer_peer_id: "galbot".to_string(),
            resource_id: "head_left_rgb".to_string(),
            state: "live".to_string(),
            head: Some(Head::Rolling {
                retention_ns: 5_000_000_000,
            }),
            extent: None,
            available: Available {
                bytes: 3_000_000_000,
                entries: 900,
                duration_ns: 5_000_000_000,
            },
            sensor: Some(SensorBlock {
                kind: SensorKind::Camera,
                r#type: "rgb".to_string(),
                sensor_id: "head_left_rgb".to_string(),
                sensor_hash: "sh".to_string(),
            }),
            pose: None,
            variant_content: VariantContent::SensorLog {
                manifest: SensorManifestPointer {
                    clock: RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "session/sdk_clock".to_string(),
                        hash: "ch".to_string(),
                    },
                    frame: Some(RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "head_left_camera_optical".to_string(),
                        hash: "fh".to_string(),
                    }),
                },
            },
        };
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(value["variant"], "sensor_log");
        assert_eq!(value["source_peer_id"], "galbot");
        assert_eq!(value["sensor"]["kind"], "camera");
        assert_eq!(value["sensor"]["type"], "rgb");
        assert_eq!(value["manifest"]["clock"]["id"], "session/sdk_clock");
        assert!(value.get("pose").is_none() || value["pose"].is_null());
    }

    #[test]
    fn pose_log_rigid_row_canonical() {
        let row = ResourceEntry {
            source_peer_id: "galbot".to_string(),
            writer_peer_id: "galbot".to_string(),
            resource_id: "world->base_link".to_string(),
            state: "sealed".to_string(),
            head: None,
            extent: Some(Extent {
                start_at_ns: 100,
                finish_at_ns: 100,
            }),
            available: Available {
                bytes: 80,
                entries: 1,
                duration_ns: 0,
            },
            sensor: None,
            pose: Some(PoseBlock {
                writer_mode: PoseWriterMode::Rigid,
            }),
            variant_content: VariantContent::PoseLog {
                manifest: PoseManifestPointer {
                    from_frame: RegistryRef {
                        peer_id: "park".to_string(),
                        id: "world".to_string(),
                        hash: "fh".to_string(),
                    },
                    to_frame: RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "base_link".to_string(),
                        hash: "th".to_string(),
                    },
                    clock: RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "session/sdk_clock".to_string(),
                        hash: "ch".to_string(),
                    },
                    source: PoseSource::Manual,
                    expected_rate_hz: 0,
                },
            },
        };
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(value["variant"], "pose_log");
        assert_eq!(value["pose"]["writer_mode"], "rigid");
        assert_eq!(value["state"], "sealed");
        assert!(value.get("head").is_none() || value["head"].is_null());
        assert_eq!(value["extent"]["start_at_ns"], 100);
        assert_eq!(value["manifest"]["from_frame"]["peer_id"], "park");
    }

    #[test]
    fn time_transform_log_row_minimal() {
        let row = ResourceEntry {
            source_peer_id: "galbot".to_string(),
            writer_peer_id: "galbot".to_string(),
            resource_id: "session/sdk_clock->wall_clock".to_string(),
            state: "live".to_string(),
            head: Some(Head::Rolling {
                retention_ns: 60_000_000_000,
            }),
            extent: None,
            available: Available {
                bytes: 4096,
                entries: 60,
                duration_ns: 60_000_000_000,
            },
            sensor: None,
            pose: None,
            variant_content: VariantContent::TimeTransformLog {
                manifest: TimeTransformManifestPointer {
                    from_clock: RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "session/sdk_clock".to_string(),
                        hash: "fh".to_string(),
                    },
                    to_clock: RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "wall_clock".to_string(),
                        hash: "th".to_string(),
                    },
                    source: TimeTransformSource::LocalClockRead,
                },
            },
        };
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(value["variant"], "time_transform_log");
        assert!(value.get("sensor").is_none() || value["sensor"].is_null());
        assert!(value.get("pose").is_none() || value["pose"].is_null());
    }

    #[test]
    fn detection_log_row_minimal() {
        let row = ResourceEntry {
            source_peer_id: "galbot".to_string(),
            writer_peer_id: "galbot".to_string(),
            resource_id: "yolo_v8@head_left_rgb".to_string(),
            state: "live".to_string(),
            head: Some(Head::Rolling {
                retention_ns: 5_000_000_000,
            }),
            extent: None,
            available: Available {
                bytes: 250000,
                entries: 150,
                duration_ns: 5_000_000_000,
            },
            sensor: None,
            pose: None,
            variant_content: VariantContent::DetectionLog {
                manifest: DetectionManifestPointer {
                    detector: RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "yolo_v8".to_string(),
                        hash: "dh".to_string(),
                    },
                    input_log: LogRef {
                        source_peer_id: "galbot".to_string(),
                        resource_id: "head_left_rgb".to_string(),
                    },
                    input_sensor: RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "head_left_rgb".to_string(),
                        hash: "sh".to_string(),
                    },
                    clock: RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "session/sdk_clock".to_string(),
                        hash: "ch".to_string(),
                    },
                },
            },
        };
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(value["variant"], "detection_log");
        assert_eq!(
            value["manifest"]["input_log"]["resource_id"],
            "head_left_rgb"
        );
    }

    #[test]
    fn materialization_row_source_writer_differ() {
        let row = ResourceEntry {
            source_peer_id: "galbot".to_string(),
            writer_peer_id: "park".to_string(),
            resource_id: "head_left_rgb".to_string(),
            state: "live".to_string(),
            head: Some(Head::Rolling {
                retention_ns: 300_000_000_000,
            }),
            extent: None,
            available: Available {
                bytes: 12_000_000_000,
                entries: 9000,
                duration_ns: 300_000_000_000,
            },
            sensor: Some(SensorBlock {
                kind: SensorKind::Camera,
                r#type: "rgb".to_string(),
                sensor_id: "head_left_rgb".to_string(),
                sensor_hash: "sh".to_string(),
            }),
            pose: None,
            variant_content: VariantContent::SensorLog {
                manifest: SensorManifestPointer {
                    clock: RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "session/sdk_clock".to_string(),
                        hash: "ch".to_string(),
                    },
                    frame: Some(RegistryRef {
                        peer_id: "galbot".to_string(),
                        id: "head_left_camera_optical".to_string(),
                        hash: "fh".to_string(),
                    }),
                },
            },
        };
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(value["source_peer_id"], "galbot");
        assert_eq!(value["writer_peer_id"], "park");
        assert_eq!(value["head"]["retention_ns"], 300_000_000_000i64);
    }

    // ── Wire round-trip ─────────────────────────────────────────────────

    #[tokio::test]
    async fn request_round_trips() {
        let req = ResourcesRequest {
            variants: vec![Variant::SensorLog, Variant::PoseLog],
        };
        let mut buf = Vec::new();
        write_resources_request(&mut buf, &req).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_resources_request(&mut cursor).await.unwrap();
        assert_eq!(req, back);

        // Empty request (all variants)
        let req_all = ResourcesRequest::all();
        let mut buf = Vec::new();
        write_resources_request(&mut buf, &req_all).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_resources_request(&mut cursor).await.unwrap();
        assert_eq!(back.variants, vec![]);
    }

    #[tokio::test]
    async fn response_round_trips_sensor_and_pose() {
        let resp = ResourcesResponse {
            resources: vec![
                ResourceEntry {
                    source_peer_id: "galbot".to_string(),
                    writer_peer_id: "galbot".to_string(),
                    resource_id: "head_left_rgb".to_string(),
                    state: "live".to_string(),
                    head: Some(Head::Rolling {
                        retention_ns: 5_000_000_000,
                    }),
                    extent: None,
                    available: Available {
                        bytes: 1024,
                        entries: 10,
                        duration_ns: 5_000_000_000,
                    },
                    sensor: Some(SensorBlock {
                        kind: SensorKind::Camera,
                        r#type: "rgb".to_string(),
                        sensor_id: "head_left_rgb".to_string(),
                        sensor_hash: "sh".to_string(),
                    }),
                    pose: None,
                    variant_content: VariantContent::SensorLog {
                        manifest: SensorManifestPointer {
                            clock: RegistryRef {
                                peer_id: "galbot".to_string(),
                                id: "session/sdk_clock".to_string(),
                                hash: "ch".to_string(),
                            },
                            frame: None,
                        },
                    },
                },
                ResourceEntry {
                    source_peer_id: "galbot".to_string(),
                    writer_peer_id: "galbot".to_string(),
                    resource_id: "world->base_link".to_string(),
                    state: "live".to_string(),
                    head: Some(Head::Rolling {
                        retention_ns: 60_000_000_000,
                    }),
                    extent: None,
                    available: Available {
                        bytes: 512,
                        entries: 5,
                        duration_ns: 60_000_000_000,
                    },
                    sensor: None,
                    pose: Some(PoseBlock {
                        writer_mode: PoseWriterMode::Movable,
                    }),
                    variant_content: VariantContent::PoseLog {
                        manifest: PoseManifestPointer {
                            from_frame: RegistryRef {
                                peer_id: "park".to_string(),
                                id: "world".to_string(),
                                hash: "fh".to_string(),
                            },
                            to_frame: RegistryRef {
                                peer_id: "galbot".to_string(),
                                id: "base_link".to_string(),
                                hash: "th".to_string(),
                            },
                            clock: RegistryRef {
                                peer_id: "galbot".to_string(),
                                id: "session/sdk_clock".to_string(),
                                hash: "ch".to_string(),
                            },
                            source: PoseSource::Manual,
                            expected_rate_hz: 30,
                        },
                    },
                },
            ],
        };
        let mut buf = Vec::new();
        write_resources_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_resources_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
        assert_eq!(back.resources.len(), 2);
    }

    // ── Building-block canonicalization (retained from Task 3.2+3.3) ────

    fn canon<T: serde::Serialize>(v: &T) -> String {
        let value = serde_json::to_value(v).unwrap();
        let bytes = auki_jcs::canonicalize(&value);
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn variant_serializes_as_snake_case_strings() {
        assert_eq!(canon(&Variant::SensorLog), r#""sensor_log""#);
        assert_eq!(canon(&Variant::PoseLog), r#""pose_log""#);
        assert_eq!(canon(&Variant::TimeTransformLog), r#""time_transform_log""#);
        assert_eq!(canon(&Variant::DetectionLog), r#""detection_log""#);
    }

    #[test]
    fn variant_rejects_unknown() {
        let bad: Result<Variant, _> = serde_json::from_str(r#""foo_log""#);
        assert!(bad.is_err());
    }

    #[test]
    fn sensor_kind_closed_set() {
        assert_eq!(canon(&SensorKind::Camera), r#""camera""#);
        assert_eq!(canon(&SensorKind::Rangefinder), r#""rangefinder""#);
        assert_eq!(canon(&SensorKind::Rf), r#""rf""#);
        assert_eq!(canon(&SensorKind::Audio), r#""audio""#);
        assert_eq!(canon(&SensorKind::JointEncoders), r#""joint_encoders""#);
        let bad: Result<SensorKind, _> = serde_json::from_str(r#""point_cloud""#);
        assert!(bad.is_err());
    }

    #[test]
    fn head_rolling_canonical() {
        let h = Head::Rolling {
            retention_ns: 5_000_000_000,
        };
        assert_eq!(canon(&h), r#"{"kind":"rolling","retention_ns":5000000000}"#);
    }

    #[test]
    fn head_fixed_canonical() {
        let h = Head::Fixed {
            started_at_ns: 1733836800000000000,
        };
        assert_eq!(
            canon(&h),
            r#"{"kind":"fixed","started_at_ns":1733836800000000000}"#
        );
    }

    #[test]
    fn extent_canonical() {
        let e = Extent {
            start_at_ns: 100,
            finish_at_ns: 200,
        };
        assert_eq!(canon(&e), r#"{"finish_at_ns":200,"start_at_ns":100}"#);
    }

    #[test]
    fn available_canonical() {
        let a = Available {
            bytes: 3_000_000_000,
            entries: 900,
            duration_ns: 5_000_000_000,
        };
        assert_eq!(
            canon(&a),
            r#"{"bytes":3000000000,"duration_ns":5000000000,"entries":900}"#
        );
    }

    #[test]
    fn sensor_block_canonical() {
        let b = SensorBlock {
            kind: SensorKind::Camera,
            r#type: "rgb".to_string(),
            sensor_id: "head_left_rgb".to_string(),
            sensor_hash: "abc123".to_string(),
        };
        assert_eq!(
            canon(&b),
            r#"{"kind":"camera","sensor_hash":"abc123","sensor_id":"head_left_rgb","type":"rgb"}"#
        );
    }

    #[test]
    fn pose_block_canonical() {
        let b = PoseBlock {
            writer_mode: PoseWriterMode::Rigid,
        };
        assert_eq!(canon(&b), r#"{"writer_mode":"rigid"}"#);

        let b = PoseBlock {
            writer_mode: PoseWriterMode::Movable,
        };
        assert_eq!(canon(&b), r#"{"writer_mode":"movable"}"#);
    }
}
