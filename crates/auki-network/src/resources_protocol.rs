//! `/auki/resources/0.2.0` - libp2p protocol for fetching a peer's
//! current resource catalog over the cluster's libp2p plane.
//!
//! Resources are the generalized discovery layer above one-off
//! catalogs. A peer answers "what can I provide right now?" with an
//! open set of resource rows. v0 ships three first-class rows:
//! sensor streams (`kind = "sensor_stream"`) and rigid transform
//! edges (`kind = "transform_edge"`), plus movable pose streams
//! (`kind = "pose_stream"`). Sensor streams can also carry current
//! pinhole intrinsics when the producer has them live. Future rows
//! (recordings, detection streams, calibration resources) add enum
//! variants and/or open-string kinds without changing the cluster
//! lifecycle machinery.

use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// libp2p protocol id for "what resources can this peer provide
/// right now?". Stable; bump version only on an incompatible
/// wire-shape change.
pub const RESOURCES_PROTOCOL: StreamProtocol = StreamProtocol::new("/auki/resources/0.2.0");

/// Cap on a single framed message. 1 MiB leaves room for catalogs with
/// embedded registry JSON and several transform edges while bounding
/// malformed senders.
pub const MAX_RESOURCES_FRAME_BYTES: u32 = 1024 * 1024;

/// Body of the request the initiator sends.
///
/// Default `{}` asks for every resource kind the peer is willing to
/// advertise. `kinds` is an open-string filter; unknown kinds simply
/// return no rows from peers that do not produce them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourcesRequest {
    /// Optional open-string resource kind filter. Examples:
    /// `"sensor_stream"`, `"transform_edge"`, `"pose_stream"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// Include canonical Sensor Registry JSON in sensor-stream rows
    /// when the producer can resolve `sensor_id + sensor_hash`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_sensor_entries: bool,
    /// Include canonical Frame Registry JSON in sensor-stream rows
    /// and transform-edge rows when the producer can resolve the
    /// referenced frame entries.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_frame_entries: bool,
    /// Include canonical Clock Registry JSON in pose-stream rows when
    /// the producer can resolve `clock_id + clock_hash`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_clock_entries: bool,
}

impl ResourcesRequest {
    /// All advertised resources, lightweight rows only.
    pub fn all() -> Self {
        Self::default()
    }

    /// Only sensor-stream resources.
    pub fn sensor_streams() -> Self {
        Self {
            kinds: vec![ResourceKind::SensorStream.as_str().into()],
            ..Self::default()
        }
    }

    /// Only transform-edge resources.
    pub fn transform_edges() -> Self {
        Self {
            kinds: vec![ResourceKind::TransformEdge.as_str().into()],
            ..Self::default()
        }
    }

    /// Only pose-stream resources.
    pub fn pose_streams() -> Self {
        Self {
            kinds: vec![ResourceKind::PoseStream.as_str().into()],
            ..Self::default()
        }
    }

    /// Ask for sensor, frame, and clock registry details embedded by value.
    pub fn with_registry_entries(mut self) -> Self {
        self.include_sensor_entries = true;
        self.include_frame_entries = true;
        self.include_clock_entries = true;
        self
    }

    /// Returns true when `kind` should be included in the response.
    pub fn wants_kind(&self, kind: ResourceKind) -> bool {
        self.kinds.is_empty() || self.kinds.iter().any(|k| k == kind.as_str())
    }
}

/// Body of the response the responder sends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourcesResponse {
    /// Snapshot of resources the producer can currently provide.
    /// Empty list = "I have a resource catalog and it is empty".
    pub resources: Vec<ResourceEntry>,
}

/// Canonical v0 resource kind labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    /// Live sensor stream over `/auki/stream/0.1.0`.
    SensorStream,
    /// Direct frame transform edge.
    TransformEdge,
    /// Live movable frame transform over `/auki/stream/0.1.0`.
    PoseStream,
}

impl ResourceKind {
    /// Stable open-string label carried on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            ResourceKind::SensorStream => "sensor_stream",
            ResourceKind::TransformEdge => "transform_edge",
            ResourceKind::PoseStream => "pose_stream",
        }
    }
}

/// One row in a [`ResourcesResponse`].
///
/// The serde tag is intentionally `kind` so cross-language clients
/// can route by open string even if their SDK copy predates a future
/// resource variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceEntry {
    /// Live sensor stream resource.
    SensorStream(SensorStreamResource),
    /// Direct transform edge resource.
    TransformEdge(TransformEdgeResource),
    /// Live movable transform resource.
    PoseStream(PoseStreamResource),
}

impl ResourceEntry {
    /// Open-string resource kind label.
    pub const fn kind(&self) -> ResourceKind {
        match self {
            Self::SensorStream(_) => ResourceKind::SensorStream,
            Self::TransformEdge(_) => ResourceKind::TransformEdge,
            Self::PoseStream(_) => ResourceKind::PoseStream,
        }
    }

    /// Resource identifier.
    pub fn id(&self) -> &str {
        match self {
            Self::SensorStream(r) => &r.id,
            Self::TransformEdge(r) => &r.id,
            Self::PoseStream(r) => &r.id,
        }
    }
}

/// Live sensor stream resource.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SensorStreamResource {
    /// Resource id. Defaults to `sensor_id` for the v0 shape.
    pub id: String,
    /// Producer-scoped sensor identifier.
    pub sensor_id: String,
    /// Content-addressed Sensor Registry hash.
    pub sensor_hash: String,
    /// Sensor kind - the closed SDK `SensorBody` serde tag carried
    /// through as a canonical string (`"camera"`, `"point_cloud"`,
    /// `"joint_encoders"`, or `"audio"`).
    pub sensor_kind: String,
    /// Protocol used to open the stream.
    pub stream_protocol: String,
    /// Payload type hint for consumers choosing a decoder.
    pub payload: String,
    /// Optional current pinhole intrinsics for camera-like streams.
    /// Producers that have a live calibration snapshot (for example
    /// ROS `CameraInfo`) can advertise it here; the auto-lifted
    /// sensor catalog row leaves it empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinhole_intrinsics: Option<ResourcePinholeIntrinsics>,
    /// Optional canonical Sensor Registry JSON matching
    /// `sensor_id + sensor_hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_entry_json: Option<String>,
    /// Optional canonical Frame Registry JSON for spatial sensors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_entry_json: Option<String>,
}

/// Numeric pinhole-camera intrinsics for projection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ResourcePinholeIntrinsics {
    /// Focal length in pixels along image X.
    pub fx: f64,
    /// Focal length in pixels along image Y.
    pub fy: f64,
    /// Principal point X coordinate in pixels.
    pub cx: f64,
    /// Principal point Y coordinate in pixels.
    pub cy: f64,
}

/// Direct transform edge resource.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TransformEdgeResource {
    /// Resource id. Conventionally `<from_frame_id>-><to_frame_id>`.
    pub id: String,
    /// Parent/source frame id.
    pub from_frame_id: String,
    /// Content-addressed Frame Registry hash for `from_frame_id`.
    pub from_frame_hash: String,
    /// Child/target frame id.
    pub to_frame_id: String,
    /// Content-addressed Frame Registry hash for `to_frame_id`.
    pub to_frame_hash: String,
    /// `"rigid"` for stationary edges in v0; open string for future
    /// writer modes.
    pub writer_mode: String,
    /// Producer/source identity. Mirrors PoseSource's tagged JSON
    /// shape without making auki-network depend on auki-manifests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
    /// Transform from `from_frame_id` into `to_frame_id`, following
    /// Pose Log semantics (parent/source frame to child/target frame).
    pub transform: ResourceSpatialTransform,
    /// Optional canonical Frame Registry JSON for `from_frame_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_frame_entry_json: Option<String>,
    /// Optional canonical Frame Registry JSON for `to_frame_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_frame_entry_json: Option<String>,
}

/// Live movable transform stream resource.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PoseStreamResource {
    /// Resource id. Conventionally `<from_frame_id>-><to_frame_id>`.
    pub id: String,
    /// Parent/source frame id.
    pub from_frame_id: String,
    /// Content-addressed Frame Registry hash for `from_frame_id`.
    pub from_frame_hash: String,
    /// Child/target frame id.
    pub to_frame_id: String,
    /// Content-addressed Frame Registry hash for `to_frame_id`.
    pub to_frame_hash: String,
    /// Clock id used by `StreamEntry.timestamp_ns`.
    pub clock_id: String,
    /// Content-addressed Clock Registry hash for `clock_id`.
    pub clock_hash: String,
    /// Protocol used to open the stream.
    pub stream_protocol: String,
    /// Payload type hint for consumers choosing a decoder.
    pub payload: String,
    /// `"movable"` for live time-varying edges in v0; open string for
    /// future writer modes.
    pub writer_mode: String,
    /// Expected producer cadence in Hz. Zero means unspecified.
    pub expected_rate_hz: u32,
    /// Producer/source identity. Mirrors PoseSource's tagged JSON
    /// shape without making auki-network depend on auki-manifests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
    /// Optional canonical Frame Registry JSON for `from_frame_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_frame_entry_json: Option<String>,
    /// Optional canonical Frame Registry JSON for `to_frame_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_frame_entry_json: Option<String>,
    /// Optional canonical Clock Registry JSON for `clock_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_entry_json: Option<String>,
}

/// JSON-friendly counterpart of `auki_datatypes::pose::SpatialTransform`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ResourceSpatialTransform {
    /// Translation component.
    pub translation: ResourceVec3,
    /// Hamilton quaternion `(x, y, z, w)`.
    pub orientation: ResourceQuat,
}

/// JSON-friendly 3D vector.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ResourceVec3 {
    /// X component.
    pub x: f64,
    /// Y component.
    pub y: f64,
    /// Z component.
    pub z: f64,
}

/// JSON-friendly Hamilton quaternion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ResourceQuat {
    /// X component.
    pub x: f64,
    /// Y component.
    pub y: f64,
    /// Z component.
    pub z: f64,
    /// W component.
    pub w: f64,
}

fn is_false(value: &bool) -> bool {
    !*value
}

// ── New catalog-row building blocks (§1 of the post-#216 design) ────────────
//
// These types are purely additive — the legacy SensorStreamResource,
// TransformEdgeResource, and PoseStreamResource remain untouched.
// Task 3.4 will assemble ResourceEntry from these pieces and delete the
// legacy shapes.

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

/// Closed enum for the high-level sensor family.
/// Wire values are `snake_case` strings; unknown values are rejected.
/// Note: `point_cloud` is a sensor *type* (open string in `SensorBlock.type`),
/// not a kind — it belongs inside a `kind = "camera"` or similar row.
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

// ── End of new catalog-row building blocks ──────────────────────────────────

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

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_round_trips() {
        let req = ResourcesRequest::transform_edges().with_registry_entries();
        let mut buf = Vec::new();
        write_resources_request(&mut buf, &req).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_resources_request(&mut cursor).await.unwrap();
        assert_eq!(req, back);

        let req = ResourcesRequest::pose_streams().with_registry_entries();
        let mut buf = Vec::new();
        write_resources_request(&mut buf, &req).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_resources_request(&mut cursor).await.unwrap();
        assert_eq!(back.kinds, vec!["pose_stream"]);
        assert!(back.include_frame_entries);
        assert!(back.include_clock_entries);
    }

    #[tokio::test]
    async fn response_round_trips_with_sensor_and_transform() {
        let resp = ResourcesResponse {
            resources: vec![
                ResourceEntry::SensorStream(SensorStreamResource {
                    id: "K1-LIVE01/head_left_cam".into(),
                    sensor_id: "K1-LIVE01/head_left_cam".into(),
                    sensor_hash: "sensorhash".into(),
                    sensor_kind: "camera".into(),
                    stream_protocol: "/auki/stream/0.1.0".into(),
                    payload: "camera_frame".into(),
                    pinhole_intrinsics: Some(ResourcePinholeIntrinsics {
                        fx: 400.0,
                        fy: 401.0,
                        cx: 272.5,
                        cy: 244.5,
                    }),
                    sensor_entry_json: Some(r#"{"sensor_id":"K1-LIVE01/head_left_cam"}"#.into()),
                    frame_entry_json: None,
                }),
                ResourceEntry::TransformEdge(TransformEdgeResource {
                    id: "K1-LIVE01/camera_link->K1-LIVE01/head_left_cam_optical".into(),
                    from_frame_id: "K1-LIVE01/camera_link".into(),
                    from_frame_hash: "fromhash".into(),
                    to_frame_id: "K1-LIVE01/head_left_cam_optical".into(),
                    to_frame_hash: "tohash".into(),
                    writer_mode: "rigid".into(),
                    source: Some(serde_json::json!({
                        "kind": "ros2_tf",
                        "publishers": ["robot_state_publisher"]
                    })),
                    transform: ResourceSpatialTransform {
                        translation: ResourceVec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        },
                        orientation: ResourceQuat {
                            x: 0.5,
                            y: -0.5,
                            z: 0.5,
                            w: -0.5,
                        },
                    },
                    from_frame_entry_json: None,
                    to_frame_entry_json: None,
                }),
                ResourceEntry::PoseStream(PoseStreamResource {
                    id: "K1-LIVE01/base_link->K1-LIVE01/head_left_rgb_optical".into(),
                    from_frame_id: "K1-LIVE01/base_link".into(),
                    from_frame_hash: "basehash".into(),
                    to_frame_id: "K1-LIVE01/head_left_rgb_optical".into(),
                    to_frame_hash: "headhash".into(),
                    clock_id: "K1-LIVE01/monotonic".into(),
                    clock_hash: "clockhash".into(),
                    stream_protocol: "/auki/stream/0.1.0".into(),
                    payload: "spatial_transform".into(),
                    writer_mode: "movable".into(),
                    expected_rate_hz: 30,
                    source: Some(serde_json::json!({
                        "kind": "ros2_tf",
                        "publishers": ["robot_state_publisher"]
                    })),
                    from_frame_entry_json: None,
                    to_frame_entry_json: None,
                    clock_entry_json: None,
                }),
            ],
        };
        let mut buf = Vec::new();
        write_resources_response(&mut buf, &resp).await.unwrap();
        let mut cursor = futures::io::Cursor::new(buf);
        let back = read_resources_response(&mut cursor).await.unwrap();
        assert_eq!(resp, back);
        let ResourceEntry::SensorStream(sensor) = &back.resources[0] else {
            panic!("expected sensor stream");
        };
        assert_eq!(sensor.pinhole_intrinsics.as_ref().unwrap().fx, 400.0);
    }

    #[test]
    fn resource_kinds_are_stable() {
        assert_eq!(ResourceKind::SensorStream.as_str(), "sensor_stream");
        assert_eq!(ResourceKind::TransformEdge.as_str(), "transform_edge");
        assert_eq!(ResourceKind::PoseStream.as_str(), "pose_stream");
    }
}

#[cfg(test)]
mod new_blocks_tests {
    use super::*;

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
        assert!(bad.is_err()); // point_cloud is now a sensor.type, not a kind
    }

    #[test]
    fn head_rolling_canonical() {
        let h = Head::Rolling {
            retention_ns: 5_000_000_000,
        };
        assert_eq!(
            canon(&h),
            r#"{"kind":"rolling","retention_ns":5000000000}"#
        );
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
        use auki_manifests::PoseWriterMode;
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
