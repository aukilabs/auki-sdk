//! Sensor + Clock Registry entries with content-addressed multi-version-by-hash
//! on-disk storage.
//!
//! An entry is built from typed fields, canonicalized via [`auki_jcs`], hashed
//! via [`auki_hash`], and persisted at
//! `<app_root>/registries/{sensors,clocks,frames}/<id>/<hash>.json`. Path
//! layout lives in [`auki_layout`]; this crate composes its helpers. Slashes in
//! IDs are replaced with `__` in path segments. Re-writing identical content is
//! a no-op; writing different content under the same id produces a sibling file.
//!
//! The hash *is* the version. There are no version counters.
//!
//! Registries live at the **app root**, not the session root — they're shared
//! across all sessions of the same app. Hash-keyed writes are idempotent, so
//! the same sensor entry produces the same `<hash>.json` regardless of session.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ─── Registry ID Validation ──────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryIdError {
    #[error("registry id is empty")]
    Empty,
    #[error("registry id contains disallowed character {0:?}")]
    DisallowedChar(char),
}

/// Validate that a registry id is suitable for use as a `(peer_id, id)` key.
///
/// Rejects: empty strings, `>` (collides with the `->` separator in
/// pose/time-transform resource_ids), `@` (collides with the detection
/// separator), and whitespace. Forward slashes are allowed and treated
/// as path separators downstream.
pub fn validate_registry_id(id: &str) -> std::result::Result<(), RegistryIdError> {
    if id.is_empty() {
        return Err(RegistryIdError::Empty);
    }
    for c in id.chars() {
        if c == '>' || c == '@' || c.is_whitespace() {
            return Err(RegistryIdError::DisallowedChar(c));
        }
    }
    Ok(())
}

// ─── Shared References ───────────────────────────────────────────────────────

/// Reference to a registry entry by (peer_id, id, content hash).
/// Used wherever one registry record points at another or a manifest
/// points at a registry entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryRef {
    pub peer_id: String,
    pub id: String,
    pub hash: String,
}

/// Reference to a log by (source_peer_id, resource_id). Logs are not
/// content-addressed by a single hash — their manifests may differ
/// across materializing peers — so this carries only the canonical
/// identity tuple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRef {
    pub source_peer_id: String,
    pub resource_id: String,
}

// ─── Device Model Registry ──────────────────────────────────────────────────

/// One mesh file referenced by a Device Model Registry entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshBlobRef {
    pub path: String,
    pub sha256: String,
}

/// On-wire `type` for a device model. URDF now; glTF/GLB later for phones/glasses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceModelFormat {
    Urdf {
        urdf_sha256: String,
        #[serde(default)]
        meshes: Vec<MeshBlobRef>,
    },
}

/// Immutable model metadata. Payload blobs are addressed independently so
/// meshes can be shared across otherwise distinct model revisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceModelBody {
    pub model_id: String,
    #[serde(flatten)]
    pub format: DeviceModelFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_convention: Option<String>,
}

impl DeviceModelBody {
    pub fn as_urdf(&self) -> Option<(&str, &[MeshBlobRef])> {
        match &self.format {
            DeviceModelFormat::Urdf { urdf_sha256, meshes } => {
                Some((urdf_sha256.as_str(), meshes.as_slice()))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceModelRegistryEntry {
    pub peer_id: String,
    pub device_model_id: String,
    #[serde(flatten)]
    pub body: DeviceModelBody,
}

impl DeviceModelRegistryEntry {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonicalize(self)
    }

    pub fn hash(&self) -> String {
        auki_hash::hash_jcs_bytes(&self.canonical_bytes())
    }

    pub fn registry_ref(&self) -> RegistryRef {
        RegistryRef {
            peer_id: self.peer_id.clone(),
            id: self.device_model_id.clone(),
            hash: self.hash(),
        }
    }

    pub fn validate_id(id: &str) -> std::result::Result<(), RegistryIdError> {
        validate_registry_id(id)
    }

    pub fn validate(&self) -> Result<()> {
        if self.peer_id.is_empty() {
            return Err(Error::InvalidDeviceModel("peer_id must not be empty".into()));
        }
        validate_registry_id(&self.device_model_id)
            .map_err(|error| Error::InvalidDeviceModel(format!("invalid device_model_id: {error}")))?;
        if self.body.model_id.is_empty() {
            return Err(Error::InvalidDeviceModel("model_id must not be empty".into()));
        }
        match &self.body.format {
            DeviceModelFormat::Urdf { urdf_sha256, meshes } => {
                if !is_sha256_hex(urdf_sha256) {
                    return Err(Error::InvalidDeviceModel(
                        "urdf_sha256 must be 64 lowercase hex characters".into(),
                    ));
                }
                let mut paths = std::collections::BTreeSet::new();
                for mesh in meshes {
                    if mesh.path.is_empty() || !paths.insert(&mesh.path) {
                        return Err(Error::InvalidDeviceModel(
                            "mesh paths must be non-empty and unique".into(),
                        ));
                    }
                    if !is_sha256_hex(&mesh.sha256) {
                        return Err(Error::InvalidDeviceModel(
                            "mesh sha256 must be 64 lowercase hex characters".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

// ─── Map Registry ───────────────────────────────────────────────────────────

/// Stable, content-addressed identity and interpretation contract for a map.
/// A map belongs to its owning peer, but its update log may be consumed and
/// materialized by any peer. The registry fixes every property necessary to
/// interpret voxel indices without depending on a Mapper implementation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapRegistryEntry {
    pub peer_id: String,
    pub map_id: String,
    #[serde(flatten)]
    pub body: MapBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MapBody {
    Voxel(VoxelMap),
}

/// A sparse, unbounded voxel map. Voxel coordinates are integer indices in
/// `frame`; `voxel_size_m` converts each index step into metres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoxelMap {
    pub frame: RegistryRef,
    pub voxel_size_m: FiniteF64,
    pub chunk_dimension: u32,
    pub value_model: VoxelValueModel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_model: Option<VoxelColorModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_classes: Vec<String>,
}

/// Defines how optional per-voxel color observations are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoxelColorModel {
    /// Source sRGB samples are converted to linear light and accumulated as
    /// weighted channel sums plus an additive weight.
    AdditiveLinearRgbEvidence,
}

/// JSON-safe finite floating point value used in content-addressed registries.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FiniteF64(pub f64);

/// Defines how MapUpdate values are combined. This initial model stores
/// additive, unclamped occupancy evidence so independently-produced updates
/// commute and replay deterministically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoxelValueModel {
    AdditiveOccupancyEvidence,
}

impl MapRegistryEntry {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonicalize(self)
    }

    pub fn hash(&self) -> String {
        auki_hash::hash_jcs_bytes(&self.canonical_bytes())
    }

    /// Exact content-addressed identity of this registry entry.
    pub fn registry_ref(&self) -> RegistryRef {
        RegistryRef {
            peer_id: self.peer_id.clone(),
            id: self.map_id.clone(),
            hash: self.hash(),
        }
    }

    pub fn validate_id(id: &str) -> std::result::Result<(), RegistryIdError> {
        validate_registry_id(id)
    }

    pub fn validate(&self) -> Result<()> {
        if self.peer_id.is_empty() {
            return Err(Error::InvalidMap("peer_id must not be empty".into()));
        }
        validate_registry_id(&self.map_id)
            .map_err(|error| Error::InvalidMap(format!("invalid map_id: {error}")))?;
        match &self.body {
            MapBody::Voxel(map) => {
                if !map.voxel_size_m.0.is_finite()
                    || map.voxel_size_m.0 <= 0.0
                    || map.chunk_dimension == 0
                {
                    return Err(Error::InvalidMap(
                        "voxel_size_m must be finite and greater than zero; chunk_dimension must be greater than zero"
                            .into(),
                    ));
                }
                if map.frame.peer_id.is_empty()
                    || map.frame.id.is_empty()
                    || map.frame.hash.is_empty()
                {
                    return Err(Error::InvalidMap(
                        "frame must contain peer_id, id, and hash".into(),
                    ));
                }
                let unique_classes = map
                    .semantic_classes
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>();
                if map.semantic_classes.iter().any(String::is_empty)
                    || unique_classes.len() != map.semantic_classes.len()
                {
                    return Err(Error::InvalidMap(
                        "semantic class labels must be non-empty and unique".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

// ─── Sensor Registry ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorRegistryEntry {
    pub peer_id: String,
    pub sensor_id: String,
    #[serde(flatten)]
    pub body: SensorBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SensorBody {
    Camera(Camera),
    Rangefinder(Rangefinder),
    Rf(Rf),
    Audio(Audio),
    JointEncoders(JointEncoders),
    Scalar(Scalar),
}

/// Static identity of a non-spatial scalar measurement such as battery
/// charge, voltage, temperature, or CPU utilization. The open strings name
/// the measured quantity and unit; each matching Sensor Log or live stream
/// carries [`auki_datatypes::scalar::Data`] samples.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scalar {
    /// Open-string measured quantity, e.g. `"battery_charge"`, `"voltage"`,
    /// or `"temperature"`.
    pub r#type: String,
    /// Open-string unit shared by every sample, e.g. `"percent"`, `"volt"`,
    /// or `"celsius"`.
    pub unit: String,
    /// Expected publication cadence. This is a sizing/discovery hint rather
    /// than a guarantee; timestamps remain authoritative.
    pub expected_rate_hz: u32,
}

impl Scalar {
    pub fn validate(&self) -> Result<()> {
        if self.r#type.is_empty() || self.unit.is_empty() || self.expected_rate_hz == 0 {
            return Err(Error::InvalidScalar(
                "type and unit must be non-empty and expected_rate_hz must be positive".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// Open-string sensor type, e.g. `"rgb"` | `"depth"` | `"ir"` | `"mono"` | `"multispectral"`.
    pub r#type: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate_hz: u32,
    /// Byte encoding of every `CameraFrame.frame` payload in the matching
    /// Sensor Log, for example `"raw"` or `"jpeg"`.
    pub image_encoding: String,
    pub pixel_format: String,
    /// Number of bytes between successive image rows for a raw single-plane
    /// image. Zero is only valid for compressed encodings.
    pub row_stride_bytes: u32,
    pub color_space: String,
    pub intrinsics_model: String,
    pub distortion_model: String,
    /// Numeric calibration for the immutable output image geometry. Cameras
    /// remain useful for non-geometric consumers when this is absent, while
    /// PnP and other metric consumers must require either this calibration or
    /// a per-frame dynamic override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<CameraCalibration>,
    /// Frame Registry reference for the camera optical frame. Replaces
    /// the former `(frame_id, frame_hash)` pair.
    pub frame: RegistryRef,
}

/// Static pinhole intrinsics and lens-distortion coefficients for every frame
/// in the Sensor Log. Values describe the published `width` × `height` image
/// after any producer-side crop, resize, rotation, or rectification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraCalibration {
    pub fx: FiniteF64,
    pub fy: FiniteF64,
    pub cx: FiniteF64,
    pub cy: FiniteF64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub distortion_coefficients: Vec<FiniteF64>,
}

impl Camera {
    /// Validate the immutable byte-layout contract shared by every frame in
    /// the Sensor Log that pins this registry entry.
    pub fn validate_image_layout(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(Error::InvalidImageLayout(
                "width and height must be positive".into(),
            ));
        }
        if self.image_encoding.is_empty() || self.pixel_format.is_empty() {
            return Err(Error::InvalidImageLayout(
                "image_encoding and pixel_format must be non-empty".into(),
            ));
        }
        match self.image_encoding.as_str() {
            "raw" if self.row_stride_bytes == 0 => Err(Error::InvalidImageLayout(
                "raw images require a positive row_stride_bytes".into(),
            )),
            "raw" => Ok(()),
            _ if self.row_stride_bytes != 0 => Err(Error::InvalidImageLayout(
                "compressed images must set row_stride_bytes to zero".into(),
            )),
            _ => Ok(()),
        }
    }

    /// Validate optional metric calibration without requiring it for image-
    /// space-only consumers such as previews and QR decoders.
    pub fn validate_calibration(&self) -> Result<()> {
        let Some(calibration) = &self.calibration else {
            return Ok(());
        };
        if !calibration.fx.0.is_finite()
            || calibration.fx.0 <= 0.0
            || !calibration.fy.0.is_finite()
            || calibration.fy.0 <= 0.0
            || !calibration.cx.0.is_finite()
            || !calibration.cy.0.is_finite()
            || calibration
                .distortion_coefficients
                .iter()
                .any(|coefficient| !coefficient.0.is_finite())
        {
            return Err(Error::InvalidCameraCalibration(
                "fx and fy must be finite and positive; cx, cy, and all distortion coefficients must be finite"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// Static layout of a rangefinder sensor's per-point bytes. The actual point
/// data lives in the per-frame log payload
/// ([`auki_datatypes::point_cloud::PointCloudLogEntry`]); this describes how
/// to interpret those bytes.
///
/// Renamed from `PointCloud`; `point_cloud` becomes a `sensor.type` value
/// under this variant (see §1 sensor kind/type taxonomy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rangefinder {
    /// Open-string sensor type, e.g. `"point_cloud"` | `"2d_lidar"` | `"3d_lidar"` |
    /// `"ultrasonic"` | `"radar"`.
    pub r#type: String,
    pub fields: Vec<PointField>,
    pub point_step: u32,
    pub is_bigendian: bool,
    pub frame_rate_hz: u32,
    /// Frame Registry reference for the coordinate system the point bytes are
    /// in. Replaces the former `(frame_id, frame_hash)` pair.
    pub frame: RegistryRef,
}

/// Minimal RF sensor body. v1 ships the variant so catalog rows can declare
/// `sensor.kind = "rf"` without a registry-shape mismatch. Production-quality
/// RF fields (channel map, tx power, etc.) land via a follow-up card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rf {
    /// Open-string sensor type, e.g. `"wifi"` | `"bluetooth"` | `"uwb"`.
    pub r#type: String,
    pub frame: RegistryRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointField {
    pub name: String,
    pub offset: u32,
    pub datatype: PointFieldDataType,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointFieldDataType {
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
}

impl PointFieldDataType {
    /// Width of one element of this datatype in bytes.
    pub const fn byte_width(self) -> u32 {
        match self {
            PointFieldDataType::Int8 | PointFieldDataType::Uint8 => 1,
            PointFieldDataType::Int16 | PointFieldDataType::Uint16 => 2,
            PointFieldDataType::Int32
            | PointFieldDataType::Uint32
            | PointFieldDataType::Float32 => 4,
            PointFieldDataType::Float64 => 8,
        }
    }
}

/// Static identity of an audio sensor (microphone or microphone array) — the
/// bits that describe how to interpret the bytes downstream consumers will
/// see in [`auki_datatypes::audio::AudioLogEntry`].
///
/// **Multi-microphone arrays are modelled as one sensor with `channels = N`,
/// not as N independent sensors.** This is right for physically-synchronized
/// arrays where the channels share a clock and a beam-forming origin. Use
/// separate `SensorRegistryEntry` records only when mics are physically
/// independent capture devices on different chips.
///
/// Named `Audio` (signal-type) rather than `Microphone` (instrument) for
/// consistency with the other sensor bodies (`Rangefinder`, `JointEncoders`)
/// and the `SensorRegistryEntry` body kinds in `auki-registry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Audio {
    /// Open-string sensor type, e.g. `"pcm"` | `"opus"`.
    pub r#type: String,
    /// Samples per channel per second (e.g. 48000).
    pub sample_rate_hz: u32,
    /// Number of channels per sample (1 mono, 2 stereo, N for arrays).
    pub channels: u32,
    /// Encoding of one sample. v1 supports raw PCM only:
    ///   `pcm_s16le` | `pcm_s24le` | `pcm_s32le` | `pcm_f32le` | `pcm_f64le`.
    /// Compressed formats (FLAC, Opus) get added as additional values when
    /// they earn it; readers should treat unknown values as opaque.
    pub sample_format: String,
    /// Channel layout label. Cross-language readers use this to know which
    /// channel index means what:
    ///   `mono` | `stereo` | `5.1` | `7.1` | `ambisonic_b` | `n_channel`.
    /// `n_channel` means "N independent channels, no specific layout" —
    /// appropriate for generic mic arrays where the consumer does its own
    /// beam-forming.
    pub channel_layout: String,
    /// Frame Registry reference for the acoustic reference point.
    pub frame: RegistryRef,
}

/// Static identity of a joint-encoder bank — the bits that describe how
/// to interpret the per-frame angle vector consumers will see in
/// [`auki_datatypes::joint_encoders::JointEncodersLogEntry`] (on disk)
/// and [`auki_datatypes::joint_encoders_stream::JointEncodersFrame`]
/// (on the libp2p stream wire).
///
/// **Joint angles are encoder readings — measurements of joint
/// positions, before any kinematic interpretation.** Forward kinematics
/// (joint space → cartesian TF) is a consumer-side derivation; the
/// URDF that drives that derivation lives with the consumer (Park,
/// future analyses), not the producer. The producer ships angle floats
/// and just enough deserialization metadata (`joint_count`) for the
/// consumer to read the bytes correctly. Mirrors the layering of
/// [`Camera`] / [`Rangefinder`] / [`Audio`]: producer ships
/// raw measurements, consumer holds the schema-for-interpretation.
///
/// Joint ordering is producer-defined and immutable per log; mapping
/// joint indices to URDF links is a consumer-side concern, agreed by
/// hand-coordination at integration time.
///
/// Deliberately excluded fields and their parking-lot rationale:
/// - **No `joint_names: Vec<String>`** — URDF lives on the consumer.
///   See `parking_lot.md` "`joint_names` placement".
/// - **No `urdf_id` / `urdf_hash`** — speculative. Park is K1-monoculture
///   today. See `parking_lot.md` "`SensorBody::JointEncoders` minimalism".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JointEncoders {
    /// Open-string sensor type, e.g. `"absolute"` | `"incremental"`.
    pub r#type: String,
    /// Number of joints in each per-frame angle vector. Sanity-check
    /// invariant for deserialization — the per-frame payload's
    /// `angles_rad` length MUST equal this. Equivalent in spirit to
    /// [`Audio::channels`].
    pub joint_count: u32,
    /// Expected publish rate in Hz, observed at sensor bootstrap.
    /// Sizing hint for segment duration / consumer buffers; not part
    /// of identity logic. Same role as [`Camera::frame_rate_hz`]
    /// and [`Rangefinder::frame_rate_hz`].
    pub frame_rate_hz: u32,
    /// Frame Registry reference for the joint-encoders sensor's reference
    /// frame. Joint encoders are in joint space, not a cartesian frame;
    /// this ref points at the kinematic root frame for the encoder bank.
    pub frame: RegistryRef,
}

impl SensorRegistryEntry {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonicalize(self)
    }

    pub fn hash(&self) -> String {
        auki_hash::hash_jcs_bytes(&self.canonical_bytes())
    }

    pub fn validate_id(id: &str) -> std::result::Result<(), RegistryIdError> {
        validate_registry_id(id)
    }
}

// ─── Sensor Log payload ──────────────────────────────────────────────────────

// `SensorLogEntry` (renamed `CameraFrame`) and `DynamicIntrinsics`
// moved to [`auki-datatypes`](../../auki-datatypes) under the `auki.camera`
// `.proto` package in Step 1 of the migration. Encoding switched from CBOR
// to protobuf; segment payload bytes are no longer self-describing
// (consumers resolve the schema via `(sensor_id, sensor_hash)` pointing at a
// `SensorRegistryEntry` whose body kind tells them which `.proto` to use).

// Pose Log payload moved to [`auki_datatypes::pose`] (Step 5 of the
// auki-datatypes migration, 2026-05-08): flat `SpatialTransform` per
// segment entry — no `PoseLogEntry { transforms: Vec<TransformSample> }`
// wrapper, no per-sample `parent_frame`/`child_frame`. Frame identity
// lives in the manifest (`from_frame_id` / `from_frame_hash` /
// `to_frame_id` / `to_frame_hash`); each Pose Log holds one
// `(from, to)` pair.
//
// `PoseSource` moved to [`auki-manifests`] at Step 0;
// `build_pose_log_manifest` lives there too (rewritten at Step 5 for
// the new identity).

// ─── Clock Registry ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockRegistryEntry {
    pub peer_id: String,
    /// The session (boot) this clock belongs to. A monotonic clock's zero is
    /// one process lifetime; a typed field so consumers resolve the session
    /// without parsing it out of `clock_id`. See #274 (D6/D7).
    pub session_id: String,
    pub clock_id: String,
    #[serde(flatten)]
    pub body: ClockBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClockBody {
    MonotonicClock(ClockMeta),
    UtcClock(ClockMeta),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockMeta {
    pub unit: String,
    pub monotonic: bool,
    /// Always serialized — `null` is meaningful (e.g. monotonic clocks have no
    /// epoch). Do *not* add `skip_serializing_if`.
    pub epoch: Option<String>,
    pub scope: Scope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    DeviceLocal,
    DomainLocal,
    Global,
}

impl ClockRegistryEntry {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonicalize(self)
    }

    pub fn hash(&self) -> String {
        auki_hash::hash_jcs_bytes(&self.canonical_bytes())
    }

    pub fn validate_id(id: &str) -> std::result::Result<(), RegistryIdError> {
        validate_registry_id(id)
    }
}

// ─── Frame Registry ──────────────────────────────────────────────────────────

/// A named coordinate system. Tells a consumer how to interpret position
/// and rotation data tagged with this frame: handedness, what each axis
/// points toward, and the length unit. Same content-addressed
/// multi-version storage shape as [`SensorRegistryEntry`] and
/// [`ClockRegistryEntry`].
///
/// **Tree structure lives elsewhere.** A FrameRegistryEntry declares
/// what one frame *is in isolation*. Edges between frames (the TF tree)
/// live in the Pose Log as `auki_datatypes::pose::SpatialTransform`
/// segment entries; the `(from, to)` frame pair is pinned in the log's
/// manifest, not on each sample.
///
/// **Rotation representation** is fixed at the `SpatialTransform`
/// layer (Hamilton quaternion `(x, y, z, w)`); not per-frame.
///
/// Construct via the field-explicit struct literal or via one of the
/// `ros_body` / `ros_optical` / `opengl` / `unity` preset constructors —
/// the on-disk JSON is fully spelled out either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRegistryEntry {
    pub peer_id: String,
    /// Stable human ID, e.g. `"K1-AABBCCDDEEFF/head_left_cam_optical"`.
    /// Same naming convention as `sensor_id` / `clock_id`.
    pub frame_id: String,
    pub handedness: Handedness,
    pub axes: AxisConvention,
    pub units: LengthUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handedness {
    Right,
    Left,
}

/// What each axis of the frame points toward semantically. The triplet
/// must be drawn from three different axis-pairs (forward/backward,
/// left/right, up/down); [`FrameRegistryEntry::validate`] enforces this.
/// Handedness is declared independently — the SDK does not cross-check
/// the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisConvention {
    pub x: AxisDirection,
    pub y: AxisDirection,
    pub z: AxisDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisDirection {
    Forward,
    Backward,
    Up,
    Down,
    Left,
    Right,
}

impl AxisDirection {
    /// Which axis-pair this direction belongs to. Two directions share a
    /// pair iff they're parallel; a valid [`AxisConvention`] picks one
    /// direction from each of the three pairs.
    fn axis_pair(self) -> u8 {
        match self {
            AxisDirection::Forward | AxisDirection::Backward => 0,
            AxisDirection::Left | AxisDirection::Right => 1,
            AxisDirection::Up | AxisDirection::Down => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Meters,
    Millimeters,
    Centimeters,
}

impl FrameRegistryEntry {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonicalize(self)
    }

    pub fn hash(&self) -> String {
        auki_hash::hash_jcs_bytes(&self.canonical_bytes())
    }

    pub fn validate_id(id: &str) -> std::result::Result<(), RegistryIdError> {
        validate_registry_id(id)
    }

    /// Validate that the [`AxisConvention`] is internally orthogonal —
    /// the three axis directions must come from three distinct
    /// axis-pairs (forward/backward, left/right, up/down). Returns
    /// `Err(Error::InvalidAxes)` if any two share a pair.
    ///
    /// Handedness consistency vs. the chosen axes is **not** validated;
    /// the integrator declares both fields and the SDK trusts the
    /// declaration.
    pub fn validate(&self) -> Result<()> {
        let xp = self.axes.x.axis_pair();
        let yp = self.axes.y.axis_pair();
        let zp = self.axes.z.axis_pair();
        if xp == yp || yp == zp || xp == zp {
            return Err(Error::InvalidAxes(format!(
                "axes must be orthogonal (drawn from three distinct \
                 axis-pairs); got x={:?} y={:?} z={:?}",
                self.axes.x, self.axes.y, self.axes.z
            )));
        }
        Ok(())
    }

    // ─── Presets ────────────────────────────────────────────────────────────
    //
    // Ergonomic constructors for the four conventions that cover almost
    // every real-world frame. The on-disk JSON is fully spelled-out
    // either way — these are pure builders, no shorthand on the wire.

    /// REP-103 body frame: right-handed, X forward, Y left, Z up, meters.
    /// Used for robot bases (`base_link`), sensor bodies, and any frame
    /// with a clear "forward direction of motion."
    pub fn ros_body(peer_id: impl Into<String>, frame_id: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            frame_id: frame_id.into(),
            handedness: Handedness::Right,
            axes: AxisConvention {
                x: AxisDirection::Forward,
                y: AxisDirection::Left,
                z: AxisDirection::Up,
            },
            units: LengthUnit::Meters,
        }
    }

    /// REP-103 camera optical frame: right-handed, X right, Y down,
    /// Z forward, meters. Used for camera optical centers; pixel-space
    /// reasoning lines up with this directly.
    pub fn ros_optical(peer_id: impl Into<String>, frame_id: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            frame_id: frame_id.into(),
            handedness: Handedness::Right,
            axes: AxisConvention {
                x: AxisDirection::Right,
                y: AxisDirection::Down,
                z: AxisDirection::Forward,
            },
            units: LengthUnit::Meters,
        }
    }

    /// OpenGL / Three.js: right-handed, X right, Y up, Z backward, meters.
    /// Used for browser-side visualizers (Park) and OpenGL renderers.
    /// "Z backward" because the camera in OpenGL convention looks down
    /// the negative-Z axis; +Z points away from the scene.
    pub fn opengl(peer_id: impl Into<String>, frame_id: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            frame_id: frame_id.into(),
            handedness: Handedness::Right,
            axes: AxisConvention {
                x: AxisDirection::Right,
                y: AxisDirection::Up,
                z: AxisDirection::Backward,
            },
            units: LengthUnit::Meters,
        }
    }

    /// Unity: left-handed, X right, Y up, Z forward, meters. Some
    /// pipelines still target Unity; included so producers in that
    /// ecosystem can declare without spelling fields out by hand.
    pub fn unity(peer_id: impl Into<String>, frame_id: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            frame_id: frame_id.into(),
            handedness: Handedness::Left,
            axes: AxisConvention {
                x: AxisDirection::Right,
                y: AxisDirection::Up,
                z: AxisDirection::Forward,
            },
            units: LengthUnit::Meters,
        }
    }
}

// ─── Detector Registry ───────────────────────────────────────────────────────

/// One Detector's identity in the Detector Registry. Closes Cuba **T4**.
///
/// Mirrors [`SensorRegistryEntry`]: stable `detector_id`, a typed
/// `body` describing what the detector *is* (e.g. an ArUco detector
/// configured for a specific dictionary), and — per Cuba **T16** — an
/// `output_types` list declaring *what it emits*.
///
/// Two axes coexist:
///
/// * **`detector_id` + content-addressed hash** → provenance, stable
///   identity, "I want exactly this configured detector."
/// * **`output_types`** → capability discovery, "who on the cluster
///   emits `aruco`?" The Notion Detector concept doc's directive —
///   *advertise what you detect, not which implementation you're
///   running* — lives on this field.
///
/// A detector that emits one logical detection type fills a single-
/// element vector (`["aruco"]`). A detector that emits several (e.g.
/// the QR_Reader that emits both `portal` and `portal_corner`) lists
/// them all. Each `type` value should match what the detector sets on
/// `DetectionFrame.type` (Cuba T12) for the entries it produces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorRegistryEntry {
    pub peer_id: String,
    pub detector_id: String,
    #[serde(flatten)]
    pub body: DetectorBody,
    /// Sensor contracts accepted as inputs by this detector. A detector can
    /// advertise several alternatives; matching any one makes a stream
    /// compatible.
    pub input_types: Vec<DetectorInput>,
    /// Detection `type` strings this detector emits. Cuba T16. Order is
    /// preserved on disk; consumers should treat the list as a set.
    pub output_types: Vec<String>,
}

/// Discoverable compatibility requirement for one Detector input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorInput {
    /// Sensor body kind, for example `"camera"` or `"rangefinder"`.
    pub sensor_kind: String,
    /// Optional open-string sensor modality, for example `"rgb"` or `"mono"`.
    pub sensor_type: Option<String>,
    /// Required Camera image encoding when applicable.
    pub image_encoding: Option<String>,
    /// Required Camera pixel format when applicable.
    pub pixel_format: Option<String>,
}

impl DetectorInput {
    /// Whether this requirement accepts the supplied immutable sensor body.
    pub fn matches(&self, sensor: &SensorBody) -> bool {
        let (kind, sensor_type) = match sensor {
            SensorBody::Camera(camera) => ("camera", camera.r#type.as_str()),
            SensorBody::Rangefinder(rangefinder) => ("rangefinder", rangefinder.r#type.as_str()),
            SensorBody::Rf(rf) => ("rf", rf.r#type.as_str()),
            SensorBody::Audio(audio) => ("audio", audio.r#type.as_str()),
            SensorBody::JointEncoders(encoders) => ("joint_encoders", encoders.r#type.as_str()),
            SensorBody::Scalar(scalar) => ("scalar", scalar.r#type.as_str()),
        };
        if self.sensor_kind != kind
            || self
                .sensor_type
                .as_deref()
                .is_some_and(|required| required != sensor_type)
        {
            return false;
        }
        match sensor {
            SensorBody::Camera(camera) => {
                self.image_encoding
                    .as_deref()
                    .is_none_or(|required| required == camera.image_encoding)
                    && self
                        .pixel_format
                        .as_deref()
                        .is_none_or(|required| required == camera.pixel_format)
            }
            _ => self.image_encoding.is_none() && self.pixel_format.is_none(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DetectorBody {
    Aruco(Aruco),
    Qr(Qr),
    Esl(Esl),
    ObjectDetection(ObjectDetection),
    /// Developer-defined detector kind and content-addressed configuration.
    Custom(CustomDetector),
}

/// Open extension point for bring-your-own detector implementations.
///
/// The SDK treats `kind` as an open, reverse-DNS-style identifier and does not
/// interpret `configuration`; both participate in the registry hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomDetector {
    pub kind: String,
    #[serde(default)]
    pub configuration: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aruco {
    /// One of OpenCV's predefined ArUco dictionary names, lowercased
    /// with an underscore between family and size — e.g. `"5x5_50"`,
    /// `"apriltag_36h11"`. Matches the CLI vocabulary in
    /// `detector-aruco`'s `--dict` flag.
    pub dictionary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Qr {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Esl {}

/// Generic ML-based object detection detector body. Carries the model
/// name as the primary identity field; the output types list on
/// [`DetectorRegistryEntry`] declares what detection labels it emits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectDetection {
    /// Model identifier, e.g. `"yolo_v8n"`, `"yolo_v8s"`.
    pub model: String,
}

impl DetectorRegistryEntry {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonicalize(self)
    }
    pub fn hash(&self) -> String {
        auki_hash::hash_jcs_bytes(&self.canonical_bytes())
    }

    /// Whether at least one advertised input contract accepts this immutable
    /// Sensor Registry body.
    pub fn accepts_input(&self, sensor: &SensorBody) -> bool {
        self.input_types.iter().any(|input| input.matches(sensor))
    }

    pub fn validate_id(id: &str) -> std::result::Result<(), RegistryIdError> {
        validate_registry_id(id)
    }
}

// ─── Storage ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// File did not exist; created at `<id>/<hash>.json`.
    Created(String),
    /// A file at `<id>/<hash>.json` already existed; treated as no-op.
    AlreadyExists(String),
}

impl WriteOutcome {
    pub fn hash(&self) -> &str {
        match self {
            WriteOutcome::Created(h) | WriteOutcome::AlreadyExists(h) => h,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Json(String),
    /// On read, the deserialized entry's `sensor_id` / `clock_id` /
    /// `frame_id` / `detector_id` did not match the id in the requested
    /// path. Indicates a misplaced or tampered file — content addressing
    /// is meant to make this detectable.
    IdMismatch {
        expected: String,
        found: String,
    },
    /// On write of a [`FrameRegistryEntry`], the [`AxisConvention`]
    /// triplet was not orthogonal — i.e. two of `x`/`y`/`z` came from
    /// the same axis-pair (forward/backward, left/right, or up/down).
    InvalidAxes(String),
    /// A Camera registry entry does not define a coherent immutable frame
    /// byte layout.
    InvalidImageLayout(String),
    /// A Camera registry entry contains invalid numeric calibration.
    InvalidCameraCalibration(String),
    /// A Scalar registry entry does not identify a quantity, unit, and
    /// positive expected cadence.
    InvalidScalar(String),
    /// On write of a frame-bearing [`SensorRegistryEntry`], the
    /// referenced `(frame_id, frame_hash)` did not resolve to an
    /// existing [`FrameRegistryEntry`] on disk.
    FrameReferenceMissing {
        sensor_id: String,
        frame_id: String,
        frame_hash: String,
    },
    /// A map registry entry declares an invalid voxel-grid contract.
    InvalidMap(String),
    /// A device model entry has invalid metadata or blob references.
    InvalidDeviceModel(String),
    /// A blob SHA-256 is invalid or its bytes do not match its address.
    InvalidBlob(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Json(s) => write!(f, "json: {s}"),
            Error::IdMismatch { expected, found } => {
                write!(f, "id mismatch: expected {expected:?}, found {found:?}")
            }
            Error::InvalidAxes(msg) => write!(f, "invalid axes: {msg}"),
            Error::InvalidImageLayout(msg) => write!(f, "invalid image layout: {msg}"),
            Error::InvalidCameraCalibration(msg) => {
                write!(f, "invalid camera calibration: {msg}")
            }
            Error::InvalidScalar(msg) => write!(f, "invalid scalar sensor: {msg}"),
            Error::FrameReferenceMissing {
                sensor_id,
                frame_id,
                frame_hash,
            } => write!(
                f,
                "sensor {sensor_id:?} references missing frame ({frame_id:?}, {frame_hash:?})"
            ),
            Error::InvalidMap(msg) => write!(f, "invalid map: {msg}"),
            Error::InvalidDeviceModel(msg) => write!(f, "invalid device model: {msg}"),
            Error::InvalidBlob(msg) => write!(f, "invalid blob: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Write a sensor registry entry under `<app_root>/registries/sensors/...`.
/// Idempotent on hash: writing identical content is a no-op.
pub fn write_sensor(app_root: &Path, entry: &SensorRegistryEntry) -> Result<WriteOutcome> {
    if let SensorBody::Camera(camera) = &entry.body {
        camera.validate_image_layout()?;
        camera.validate_calibration()?;
    }
    if let SensorBody::Scalar(scalar) = &entry.body {
        scalar.validate()?;
    }
    validate_sensor_frame_reference(app_root, entry)?;
    let bytes = entry.canonical_bytes();
    let hash = auki_hash::hash_jcs_bytes(&bytes);
    let path = auki_layout::sensor_entry_path(app_root, &entry.peer_id, &entry.sensor_id, &hash);
    write_entry_at(&path, hash, &bytes)
}

/// Write a clock registry entry under `<app_root>/registries/clocks/...`.
pub fn write_clock(app_root: &Path, entry: &ClockRegistryEntry) -> Result<WriteOutcome> {
    let bytes = entry.canonical_bytes();
    let hash = auki_hash::hash_jcs_bytes(&bytes);
    let path = auki_layout::clock_entry_path(app_root, &entry.peer_id, &entry.clock_id, &hash);
    write_entry_at(&path, hash, &bytes)
}

/// Read a sensor registry entry by `(peer_id, sensor_id, hash)`. Returns `Ok(None)` when
/// the file doesn't exist; `Err(IdMismatch)` if the on-disk entry's
/// `sensor_id` differs from the requested id.
pub fn read_sensor(
    app_root: &Path,
    peer_id: &str,
    sensor_id: &str,
    hash: &str,
) -> Result<Option<SensorRegistryEntry>> {
    let path = auki_layout::sensor_entry_path(app_root, peer_id, sensor_id, hash);
    let Some(bytes) = read_at(&path)? else {
        return Ok(None);
    };
    let entry: SensorRegistryEntry =
        serde_json::from_slice(&bytes).map_err(|e| Error::Json(e.to_string()))?;
    if entry.sensor_id != sensor_id {
        return Err(Error::IdMismatch {
            expected: sensor_id.to_string(),
            found: entry.sensor_id,
        });
    }
    Ok(Some(entry))
}

/// Read a clock registry entry by `(peer_id, clock_id, hash)`.
pub fn read_clock(
    app_root: &Path,
    peer_id: &str,
    clock_id: &str,
    hash: &str,
) -> Result<Option<ClockRegistryEntry>> {
    let path = auki_layout::clock_entry_path(app_root, peer_id, clock_id, hash);
    let Some(bytes) = read_at(&path)? else {
        return Ok(None);
    };
    let entry: ClockRegistryEntry =
        serde_json::from_slice(&bytes).map_err(|e| Error::Json(e.to_string()))?;
    if entry.clock_id != clock_id {
        return Err(Error::IdMismatch {
            expected: clock_id.to_string(),
            found: entry.clock_id,
        });
    }
    Ok(Some(entry))
}

/// Write a frame registry entry under `<app_root>/registries/frames/...`.
/// Validates the [`AxisConvention`] before hashing — a non-orthogonal
/// triplet is rejected with [`Error::InvalidAxes`] without touching disk.
/// Idempotent on hash: writing identical content is a no-op.
pub fn write_frame(app_root: &Path, entry: &FrameRegistryEntry) -> Result<WriteOutcome> {
    entry.validate()?;
    let bytes = entry.canonical_bytes();
    let hash = auki_hash::hash_jcs_bytes(&bytes);
    let path = auki_layout::frame_entry_path(app_root, &entry.peer_id, &entry.frame_id, &hash);
    write_entry_at(&path, hash, &bytes)
}

/// Read a frame registry entry by `(peer_id, frame_id, hash)`.
pub fn read_frame(
    app_root: &Path,
    peer_id: &str,
    frame_id: &str,
    hash: &str,
) -> Result<Option<FrameRegistryEntry>> {
    let path = auki_layout::frame_entry_path(app_root, peer_id, frame_id, hash);
    let Some(bytes) = read_at(&path)? else {
        return Ok(None);
    };
    let entry: FrameRegistryEntry =
        serde_json::from_slice(&bytes).map_err(|e| Error::Json(e.to_string()))?;
    if entry.frame_id != frame_id {
        return Err(Error::IdMismatch {
            expected: frame_id.to_string(),
            found: entry.frame_id,
        });
    }
    Ok(Some(entry))
}

/// Write a detector registry entry under `<app_root>/registries/detectors/...`.
/// Idempotent on hash: writing identical content is a no-op. Cuba T4.
pub fn write_detector(app_root: &Path, entry: &DetectorRegistryEntry) -> Result<WriteOutcome> {
    let bytes = entry.canonical_bytes();
    let hash = auki_hash::hash_jcs_bytes(&bytes);
    let path =
        auki_layout::detector_entry_path(app_root, &entry.peer_id, &entry.detector_id, &hash);
    write_entry_at(&path, hash, &bytes)
}

/// Read a detector registry entry by `(peer_id, detector_id, hash)`. Returns
/// `Ok(None)` when the file doesn't exist; `Err(IdMismatch)` if the
/// on-disk entry's `detector_id` differs from the requested id. Cuba T4.
pub fn read_detector(
    app_root: &Path,
    peer_id: &str,
    detector_id: &str,
    hash: &str,
) -> Result<Option<DetectorRegistryEntry>> {
    let path = auki_layout::detector_entry_path(app_root, peer_id, detector_id, hash);
    let Some(bytes) = read_at(&path)? else {
        return Ok(None);
    };
    let entry: DetectorRegistryEntry =
        serde_json::from_slice(&bytes).map_err(|e| Error::Json(e.to_string()))?;
    if entry.detector_id != detector_id {
        return Err(Error::IdMismatch {
            expected: detector_id.to_string(),
            found: entry.detector_id,
        });
    }
    Ok(Some(entry))
}

/// Write a map registry entry under `<app_root>/registries/maps/...`.
pub fn write_map(app_root: &Path, entry: &MapRegistryEntry) -> Result<WriteOutcome> {
    entry.validate()?;
    let bytes = entry.canonical_bytes();
    let hash = auki_hash::hash_jcs_bytes(&bytes);
    let path = auki_layout::map_entry_path(app_root, &entry.peer_id, &entry.map_id, &hash);
    write_entry_at(&path, hash, &bytes)
}

/// Read a map registry entry by `(peer_id, map_id, hash)`.
pub fn read_map(
    app_root: &Path,
    peer_id: &str,
    map_id: &str,
    hash: &str,
) -> Result<Option<MapRegistryEntry>> {
    let path = auki_layout::map_entry_path(app_root, peer_id, map_id, hash);
    let Some(bytes) = read_at(&path)? else {
        return Ok(None);
    };
    let entry: MapRegistryEntry =
        serde_json::from_slice(&bytes).map_err(|e| Error::Json(e.to_string()))?;
    if entry.peer_id != peer_id {
        return Err(Error::IdMismatch {
            expected: peer_id.to_string(),
            found: entry.peer_id,
        });
    }
    if entry.map_id != map_id {
        return Err(Error::IdMismatch {
            expected: map_id.to_string(),
            found: entry.map_id,
        });
    }
    entry.validate()?;
    Ok(Some(entry))
}

/// Write a Device Model Registry entry under
/// `<app_root>/registries/device_models/...`.
pub fn write_device_model(
    app_root: &Path,
    entry: &DeviceModelRegistryEntry,
) -> Result<WriteOutcome> {
    entry.validate()?;
    let bytes = entry.canonical_bytes();
    let hash = auki_hash::hash_jcs_bytes(&bytes);
    let path = auki_layout::device_model_entry_path(
        app_root,
        &entry.peer_id,
        &entry.device_model_id,
        &hash,
    );
    write_entry_at(&path, hash, &bytes)
}

/// Read a Device Model Registry entry by `(peer_id, device_model_id, hash)`.
pub fn read_device_model(
    app_root: &Path,
    peer_id: &str,
    device_model_id: &str,
    hash: &str,
) -> Result<Option<DeviceModelRegistryEntry>> {
    let path = auki_layout::device_model_entry_path(app_root, peer_id, device_model_id, hash);
    let Some(bytes) = read_at(&path)? else {
        return Ok(None);
    };
    let entry: DeviceModelRegistryEntry =
        serde_json::from_slice(&bytes).map_err(|e| Error::Json(e.to_string()))?;
    if entry.peer_id != peer_id {
        return Err(Error::IdMismatch {
            expected: peer_id.to_owned(),
            found: entry.peer_id,
        });
    }
    if entry.device_model_id != device_model_id {
        return Err(Error::IdMismatch {
            expected: device_model_id.to_owned(),
            found: entry.device_model_id,
        });
    }
    entry.validate()?;
    Ok(Some(entry))
}

/// One `(id, hash)` row from [`list_device_models`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceModelListEntry {
    /// `device_model_id` from the registry entry.
    pub id: String,
    /// XXH3-128 of the entry's canonical JSON bytes.
    pub hash: String,
}

/// Enumerate device-model registry entries for `peer_id` under `app_root`.
///
/// Walks `<app_root>/registries/device_models/<peer>/…` and returns every
/// valid `(device_model_id, hash)` pair. Missing peer dirs yield an empty
/// list. Malformed files are skipped with no error (same soft-fail spirit
/// as an empty List response on the wire).
pub fn list_device_models(app_root: &Path, peer_id: &str) -> Result<Vec<DeviceModelListEntry>> {
    let peer_dir = auki_layout::device_models_peer_dir(app_root, peer_id);
    let Ok(model_dirs) = fs::read_dir(&peer_dir) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for model_dir in model_dirs {
        let model_dir = model_dir?;
        if !model_dir.file_type()?.is_dir() {
            continue;
        }
        let Ok(files) = fs::read_dir(model_dir.path()) else {
            continue;
        };
        for file in files {
            let file = file?;
            let path = file.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(bytes) = read_at(&path)? else {
                continue;
            };
            let Ok(entry) = serde_json::from_slice::<DeviceModelRegistryEntry>(&bytes) else {
                continue;
            };
            if entry.peer_id != peer_id {
                continue;
            }
            if entry.validate().is_err() {
                continue;
            }
            let hash = entry.hash();
            let file_stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default();
            if file_stem != hash {
                continue;
            }
            entries.push(DeviceModelListEntry {
                id: entry.device_model_id,
                hash,
            });
        }
    }
    entries.sort_by(|a, b| (&a.id, &a.hash).cmp(&(&b.id, &b.hash)));
    Ok(entries)
}

/// SHA-256, encoded as lowercase hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Persist a content-addressed blob and return its SHA-256 address.
pub fn put_blob(app_root: &Path, bytes: &[u8]) -> Result<String> {
    let sha256 = sha256_hex(bytes);
    let path = auki_layout::blob_path(app_root, &sha256);
    if !path.exists() {
        let dir = path.parent().expect("blob path has parent");
        fs::create_dir_all(dir)?;
        atomic_write(&path, bytes)?;
    }
    Ok(sha256)
}

/// Fetch a blob only when the requested address is a valid lowercase SHA-256.
pub fn get_blob(app_root: &Path, sha256: &str) -> Result<Option<Vec<u8>>> {
    if !is_sha256_hex(sha256) {
        return Err(Error::InvalidBlob("sha256 must be 64 lowercase hex characters".into()));
    }
    let Some(bytes) = read_at(&auki_layout::blob_path(app_root, sha256))? else {
        return Ok(None);
    };
    if sha256_hex(&bytes) != sha256 {
        return Err(Error::InvalidBlob("on-disk bytes do not match SHA-256 address".into()));
    }
    Ok(Some(bytes))
}

/// Whether an addressable blob exists locally.
pub fn blob_exists(app_root: &Path, sha256: &str) -> Result<bool> {
    Ok(get_blob(app_root, sha256)?.is_some())
}

/// Result of [`put_urdf_package`]: rewritten URDF + mesh blobs on disk and
/// a ready-to-register [`DeviceModelBody`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutUrdfPackage {
    /// Stable id taken from the URDF `<robot name>` (lowercased), or the
    /// package directory name when the attribute is missing.
    pub device_model_id: String,
    /// Body suitable for [`write_device_model`] / session `register_device_model`.
    pub body: DeviceModelBody,
}

/// Rewrite mesh `filename`s to package-relative paths, `put_blob` the URDF
/// and every referenced mesh, and return a [`DeviceModelBody`].
///
/// Fails if the URDF references any mesh that cannot be resolved beside the
/// URDF package directory. Rewrite happens once at publish time so consumers
/// do not need a second path rewrite after dig.
pub fn put_urdf_package(
    app_root: &Path,
    urdf_path: &Path,
    root_convention: Option<String>,
) -> Result<PutUrdfPackage> {
    let urdf_bytes = fs::read(urdf_path).map_err(Error::Io)?;
    let urdf_text = String::from_utf8(urdf_bytes).map_err(|error| {
        Error::InvalidDeviceModel(format!("URDF is not UTF-8: {error}"))
    })?;
    let device_model_id = urdf_robot_name(&urdf_text)
        .or_else(|| {
            urdf_path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(|name| name.to_ascii_lowercase())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "robot".into());

    let mesh_refs = collect_urdf_mesh_filenames(&urdf_text);
    let package_dir = urdf_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let mut rewritten = urdf_text;
    let mut meshes = Vec::new();
    let mut seen_paths = std::collections::BTreeSet::new();
    // Replace from the end so earlier offsets stay valid.
    for (start, end, original) in mesh_refs.into_iter().rev() {
        let rel = normalize_mesh_rel_path(&original);
        if rel.is_empty() {
            return Err(Error::InvalidDeviceModel(
                "URDF mesh filename resolved to an empty path".into(),
            ));
        }
        rewritten.replace_range(start..end, &rel);
        if !seen_paths.insert(rel.clone()) {
            continue;
        }
        let resolved = resolve_urdf_mesh(&package_dir, &original, &rel);
        let Some(resolved) = resolved else {
            return Err(Error::InvalidDeviceModel(format!(
                "URDF references mesh {original:?} but none resolved beside {}",
                package_dir.display()
            )));
        };
        let mesh_bytes = fs::read(&resolved).map_err(Error::Io)?;
        let sha256 = put_blob(app_root, &mesh_bytes)?;
        meshes.push(MeshBlobRef {
            path: rel,
            sha256,
        });
    }
    meshes.sort_by(|a, b| a.path.cmp(&b.path));

    if rewritten.contains("filename=\"package://") || rewritten.contains("filename='package://") {
        return Err(Error::InvalidDeviceModel(
            "URDF still contains package:// mesh refs after rewrite".into(),
        ));
    }

    let urdf_sha256 = put_blob(app_root, rewritten.as_bytes())?;
    let body = DeviceModelBody {
        model_id: device_model_id.clone(),
        format: DeviceModelFormat::Urdf {
            urdf_sha256,
            meshes,
        },
        root_convention,
    };
    Ok(PutUrdfPackage {
        device_model_id,
        body,
    })
}

fn urdf_robot_name(urdf: &str) -> Option<String> {
    let marker = "<robot";
    let start = urdf.find(marker)?;
    let after = &urdf[start..];
    let end = after.find('>')?;
    let tag = &after[..end];
    for quote in ['"', '\''] {
        let key = format!("name={quote}");
        if let Some(name_start) = tag.find(&key) {
            let value_start = name_start + key.len();
            let rest = &tag[value_start..];
            if let Some(value_end) = rest.find(quote) {
                let name = rest[..value_end].trim().to_ascii_lowercase();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

/// `(value_start, value_end, original_value)` for every `filename="…"` / `filename='…'`.
fn collect_urdf_mesh_filenames(urdf: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let bytes = urdf.as_bytes();
    let mut i = 0;
    while i + 10 < bytes.len() {
        if !urdf[i..].starts_with("filename=") {
            i += 1;
            continue;
        }
        let quote_idx = i + "filename=".len();
        let quote = match bytes.get(quote_idx).copied() {
            Some(b'"') | Some(b'\'') => bytes[quote_idx] as char,
            _ => {
                i += 1;
                continue;
            }
        };
        let value_start = quote_idx + 1;
        let Some(rel) = urdf[value_start..].find(quote) else {
            i += 1;
            continue;
        };
        let value_end = value_start + rel;
        out.push((
            value_start,
            value_end,
            urdf[value_start..value_end].to_string(),
        ));
        i = value_end + 1;
    }
    out
}

/// Strip ROS `package://pkg/` so mesh paths are package-relative.
pub fn normalize_mesh_rel_path(path: &str) -> String {
    let path = path.trim().trim_start_matches('/');
    if let Some(rest) = path.strip_prefix("package://") {
        if let Some((_, relative)) = rest.split_once('/') {
            return relative.trim_start_matches('/').to_string();
        }
        return rest.to_string();
    }
    path.to_string()
}

fn resolve_urdf_mesh(package_dir: &Path, original: &str, rel: &str) -> Option<PathBuf> {
    let basename = Path::new(rel).file_name()?;
    let mut candidates = vec![
        package_dir.join(rel),
        package_dir.join("meshes").join(basename),
        package_dir
            .parent()
            .unwrap_or(package_dir)
            .join(rel),
        package_dir
            .parent()
            .unwrap_or(package_dir)
            .join("meshes")
            .join(basename),
        // Booster K1 vendor tree (L16b).
        PathBuf::from("/opt/booster/Gait/configs/K1/meshes").join(basename),
    ];
    if let Some(rest) = original.strip_prefix("package://") {
        let (pkg, relative) = rest
            .split_once('/')
            .map(|(pkg, relative)| (pkg, relative))
            .unwrap_or((rest, ""));
        candidates.push(package_dir.join(rest));
        candidates.push(package_dir.join(pkg).join(relative));
        if let Some(parent) = package_dir.parent() {
            candidates.push(parent.join(rest));
            candidates.push(parent.join(pkg).join(relative));
        }
        for env_key in ["AMENT_PREFIX_PATH", "ROS_PACKAGE_PATH"] {
            let Ok(value) = std::env::var(env_key) else {
                continue;
            };
            for prefix in value.split(':').filter(|prefix| !prefix.is_empty()) {
                let root = PathBuf::from(prefix);
                if env_key == "AMENT_PREFIX_PATH" {
                    candidates.push(root.join("share").join(pkg).join(relative));
                    candidates.push(
                        root.join("share")
                            .join(pkg)
                            .join("meshes")
                            .join(basename),
                    );
                } else {
                    candidates.push(root.join(pkg).join(relative));
                    candidates.push(root.join(pkg).join("meshes").join(basename));
                }
            }
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

// `build_sensor_log_manifest` moved to [`auki-manifests`] in Step 0 of the
// auki-datatypes migration.

// ─── Internals ───────────────────────────────────────────────────────────────

fn canonicalize<T: Serialize>(value: &T) -> Vec<u8> {
    // Registry entry types are plain structs of strings/ints/bools/options;
    // serializing to a Value cannot fail in practice.
    let v = serde_json::to_value(value).expect("registry entry serializes to a JSON value");
    auki_jcs::canonicalize(&v)
}

fn write_entry_at(path: &Path, hash: String, bytes: &[u8]) -> Result<WriteOutcome> {
    if path.exists() {
        return Ok(WriteOutcome::AlreadyExists(hash));
    }
    let dir = path.parent().expect("entry path has a parent");
    fs::create_dir_all(dir)?;
    atomic_write(path, bytes)?;
    Ok(WriteOutcome::Created(hash))
}

fn validate_sensor_frame_reference(app_root: &Path, entry: &SensorRegistryEntry) -> Result<()> {
    let Some(frame_ref) = sensor_frame_reference(&entry.body) else {
        return Ok(());
    };

    if frame_ref.id.is_empty()
        || frame_ref.hash.is_empty()
        || read_frame(app_root, &frame_ref.peer_id, &frame_ref.id, &frame_ref.hash)?.is_none()
    {
        return Err(Error::FrameReferenceMissing {
            sensor_id: entry.sensor_id.clone(),
            frame_id: frame_ref.id.clone(),
            frame_hash: frame_ref.hash.clone(),
        });
    }

    Ok(())
}

fn sensor_frame_reference(body: &SensorBody) -> Option<&RegistryRef> {
    match body {
        SensorBody::Camera(b) => Some(&b.frame),
        SensorBody::Rangefinder(b) => Some(&b.frame),
        SensorBody::Rf(b) => Some(&b.frame),
        SensorBody::Audio(b) => Some(&b.frame),
        SensorBody::JointEncoders(b) => Some(&b.frame),
        SensorBody::Scalar(_) => None,
    }
}

fn read_at(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(e)),
    }
}

pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| io::Error::other("target has no parent"))?;
    let file_name = target
        .file_name()
        .ok_or_else(|| io::Error::other("target has no file name"))?;
    let tmp = dir.join(format!(".{}.tmp", file_name.to_string_lossy()));
    {
        let mut f: File = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, target)?;
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const M1_OPTICAL_FRAME_HASH: &str = "03b86f32827ec6a25a5e619b2f36478b";

    fn m1_sensor_entry() -> SensorRegistryEntry {
        SensorRegistryEntry {
            peer_id: "test-peer".into(),
            sensor_id: "K1-AABBCCDDEEFF/head_left_cam".into(),
            body: SensorBody::Camera(Camera {
                r#type: "rgb".into(),
                width: 544,
                height: 488,
                frame_rate_hz: 20,
                image_encoding: "raw".into(),
                pixel_format: "YUV_NV12".into(),
                row_stride_bytes: 544,
                color_space: "BT.709".into(),
                intrinsics_model: "pinhole".into(),
                distortion_model: "plumb_bob".into(),
                calibration: None,
                frame: RegistryRef {
                    peer_id: "test-peer".into(),
                    id: "K1-AABBCCDDEEFF/head_left_cam_optical".into(),
                    hash: M1_OPTICAL_FRAME_HASH.into(),
                },
            }),
        }
    }

    fn m1_optical_frame_entry() -> FrameRegistryEntry {
        FrameRegistryEntry::ros_optical("test-peer", "K1-AABBCCDDEEFF/head_left_cam_optical")
    }

    fn write_m1_optical_frame(app_root: &Path) {
        let outcome = write_frame(app_root, &m1_optical_frame_entry()).unwrap();
        assert_eq!(outcome.hash(), M1_OPTICAL_FRAME_HASH);
    }

    fn m1_monotonic_entry() -> ClockRegistryEntry {
        ClockRegistryEntry {
            peer_id: "galbot".into(),
            session_id: "sess-m1".into(),
            clock_id: "K1-AABBCCDDEEFF/monotonic".into(),
            body: ClockBody::MonotonicClock(ClockMeta {
                unit: "milliseconds".into(),
                monotonic: true,
                epoch: None,
                scope: Scope::DeviceLocal,
            }),
        }
    }

    fn m1_utc_entry() -> ClockRegistryEntry {
        ClockRegistryEntry {
            peer_id: "galbot".into(),
            session_id: "sess-m1".into(),
            clock_id: "K1-AABBCCDDEEFF/utc".into(),
            body: ClockBody::UtcClock(ClockMeta {
                unit: "milliseconds".into(),
                monotonic: false,
                epoch: Some("1970-01-01T00:00:00Z".into()),
                scope: Scope::Global,
            }),
        }
    }

    /// Locks the JCS canonical bytes for the M1 example sensor entry.
    /// Catches drift in entry shape OR canonicalization.
    #[test]
    fn sensor_entry_serializes_to_canonical_bytes_matching_m1_example() {
        let bytes = m1_sensor_entry().canonical_bytes();
        let s = std::str::from_utf8(&bytes).unwrap();
        // Keys sorted by RFC 8785 §3.2.3 (lexicographic UTF-16 code units).
        // peer_id added; frame_id+frame_hash replaced by nested frame object;
        // variant discriminator renamed from "type" to "kind"; open-string
        // sensor.type lives as "type" key inside the body.
        assert_eq!(
            s,
            r#"{"color_space":"BT.709","distortion_model":"plumb_bob","frame":{"hash":"03b86f32827ec6a25a5e619b2f36478b","id":"K1-AABBCCDDEEFF/head_left_cam_optical","peer_id":"test-peer"},"frame_rate_hz":20,"height":488,"image_encoding":"raw","intrinsics_model":"pinhole","kind":"camera","peer_id":"test-peer","pixel_format":"YUV_NV12","row_stride_bytes":544,"sensor_id":"K1-AABBCCDDEEFF/head_left_cam","type":"rgb","width":544}"#
        );
    }

    #[test]
    fn monotonic_clock_canonical_bytes_match_m1_example() {
        let bytes = m1_monotonic_entry().canonical_bytes();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"clock_id":"K1-AABBCCDDEEFF/monotonic","epoch":null,"monotonic":true,"peer_id":"galbot","scope":"device-local","session_id":"sess-m1","type":"monotonic_clock","unit":"milliseconds"}"#
        );
    }

    #[test]
    fn utc_clock_canonical_bytes_match_m1_example() {
        let bytes = m1_utc_entry().canonical_bytes();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"clock_id":"K1-AABBCCDDEEFF/utc","epoch":"1970-01-01T00:00:00Z","monotonic":false,"peer_id":"galbot","scope":"global","session_id":"sess-m1","type":"utc_clock","unit":"milliseconds"}"#
        );
    }

    /// Locks the XXH3-128 hex of the M1 sensor entry. Catches drift in
    /// entry shape, canonicalization, or hashing. Recomputed for #216 rev 2:
    /// peer_id added, frame_id+frame_hash → RegistryRef, kind tag renamed,
    /// sensor.type field added.
    #[test]
    fn sensor_entry_hash_is_locked() {
        assert_eq!(m1_sensor_entry().hash(), "9306d67f99d38ced7c186c0f63734421");
    }

    #[test]
    fn monotonic_clock_hash_is_locked() {
        assert_eq!(
            m1_monotonic_entry().hash(),
            "107238adc0441893cbfd35c41b5ec989"
        );
    }

    #[test]
    fn utc_clock_hash_is_locked() {
        assert_eq!(m1_utc_entry().hash(), "79eb38239c937eaa63863d25f822947a");
    }

    #[test]
    fn clock_entry_carries_session_id() {
        let e = ClockRegistryEntry {
            peer_id: "galbot".into(),
            session_id: "sess-7f3a".into(),
            clock_id: "galbot/sess-7f3a/monotonic".into(),
            body: ClockBody::MonotonicClock(ClockMeta {
                unit: "ns".into(),
                monotonic: true,
                epoch: None,
                scope: Scope::DeviceLocal,
            }),
        };
        let s = String::from_utf8(e.canonical_bytes()).unwrap();
        assert!(
            s.contains(r#""session_id":"sess-7f3a""#),
            "session_id missing from canonical bytes: {s}"
        );
    }

    #[test]
    fn write_then_read_sensor_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        write_m1_optical_frame(dir.path());
        let entry = m1_sensor_entry();
        let outcome = write_sensor(dir.path(), &entry).unwrap();
        let hash = match outcome {
            WriteOutcome::Created(h) => h,
            other => panic!("expected Created, got {other:?}"),
        };
        let read = read_sensor(dir.path(), &entry.peer_id, &entry.sensor_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
    }

    #[test]
    fn write_then_read_clock_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_utc_entry();
        let outcome = write_clock(dir.path(), &entry).unwrap();
        let hash = outcome.hash().to_string();
        let read = read_clock(dir.path(), &entry.peer_id, &entry.clock_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
    }

    #[test]
    fn multi_version_same_content_is_no_op() {
        let dir = tempfile::tempdir().unwrap();
        write_m1_optical_frame(dir.path());
        let entry = m1_sensor_entry();

        let first = write_sensor(dir.path(), &entry).unwrap();
        assert!(matches!(first, WriteOutcome::Created(_)));

        let second = write_sensor(dir.path(), &entry).unwrap();
        assert!(matches!(second, WriteOutcome::AlreadyExists(_)));
        assert_eq!(first.hash(), second.hash());

        let entry_dir = dir
            .path()
            .join("registries")
            .join("sensors")
            .join("test-peer")
            .join("K1-AABBCCDDEEFF__head_left_cam");
        let json_count = fs::read_dir(&entry_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .count();
        assert_eq!(json_count, 1);
    }

    #[test]
    fn multi_version_different_content_writes_alongside() {
        let dir = tempfile::tempdir().unwrap();
        write_m1_optical_frame(dir.path());
        let mut entry = m1_sensor_entry();
        let first_hash = entry.hash();
        write_sensor(dir.path(), &entry).unwrap();

        // Mutate a static field — produces a new content hash and a sibling file.
        // Match (not `if let`): exhaustiveness means a future SensorBody variant
        // becomes a compile error pointing the author here.
        match &mut entry.body {
            SensorBody::Camera(cam) => {
                cam.width = 1920;
                cam.height = 1080;
            }
            SensorBody::Rangefinder(_)
            | SensorBody::Rf(_)
            | SensorBody::Audio(_)
            | SensorBody::JointEncoders(_)
            | SensorBody::Scalar(_) => {
                panic!("test was set up for Camera")
            }
        }
        let second_hash = entry.hash();
        assert_ne!(first_hash, second_hash);
        write_sensor(dir.path(), &entry).unwrap();

        let entry_dir = dir
            .path()
            .join("registries")
            .join("sensors")
            .join("test-peer")
            .join("K1-AABBCCDDEEFF__head_left_cam");
        let json_count = fs::read_dir(&entry_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .count();
        assert_eq!(json_count, 2);

        // Both resolvable by their respective hashes.
        assert!(
            read_sensor(dir.path(), &entry.peer_id, &entry.sensor_id, &first_hash)
                .unwrap()
                .is_some()
        );
        let resolved_second =
            read_sensor(dir.path(), &entry.peer_id, &entry.sensor_id, &second_hash).unwrap();
        assert_eq!(resolved_second, Some(entry));
    }

    #[test]
    fn slash_in_id_becomes_double_underscore() {
        let dir = tempfile::tempdir().unwrap();
        write_m1_optical_frame(dir.path());
        let entry = m1_sensor_entry();
        write_sensor(dir.path(), &entry).unwrap();

        let expected_dir = dir
            .path()
            .join("registries")
            .join("sensors")
            .join("test-peer")
            .join("K1-AABBCCDDEEFF__head_left_cam");
        assert!(expected_dir.is_dir(), "expected {expected_dir:?} to exist");

        // Defensive: literal `head_left_cam` subdir under a `K1-AABBCCDDEEFF`
        // dir must NOT exist (would mean we forgot the substitution).
        let bad = dir
            .path()
            .join("registries")
            .join("sensors")
            .join("test-peer")
            .join("K1-AABBCCDDEEFF")
            .join("head_left_cam");
        assert!(!bad.exists(), "did not expect nested dirs: {bad:?}");
    }

    #[test]
    fn read_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let result = read_sensor(
            dir.path(),
            "galbot",
            "K1-AABBCCDDEEFF/never_written",
            "00000000000000000000000000000000",
        )
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_with_id_mismatch_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_m1_optical_frame(dir.path());
        let entry = m1_sensor_entry();
        let hash = match write_sensor(dir.path(), &entry).unwrap() {
            WriteOutcome::Created(h) => h,
            other => panic!("unexpected: {other:?}"),
        };

        // Manually copy the on-disk file under a *different* sensor_id's
        // directory at the same hash. This simulates a misplaced or tampered
        // file — content addressing is meant to make the mismatch detectable.
        let real = dir
            .path()
            .join("registries")
            .join("sensors")
            .join(&entry.peer_id)
            .join("K1-AABBCCDDEEFF__head_left_cam")
            .join(format!("{hash}.json"));
        let bogus_dir = dir
            .path()
            .join("registries")
            .join("sensors")
            .join(&entry.peer_id)
            .join("K1-AABBCCDDEEFF__other_cam");
        fs::create_dir_all(&bogus_dir).unwrap();
        fs::copy(&real, bogus_dir.join(format!("{hash}.json"))).unwrap();

        let err = read_sensor(
            dir.path(),
            &entry.peer_id,
            "K1-AABBCCDDEEFF/other_cam",
            &hash,
        );
        assert!(matches!(err, Err(Error::IdMismatch { .. })), "got {err:?}");
    }

    #[test]
    fn write_outcome_hash_accessor() {
        let h = "deadbeef".to_string();
        assert_eq!(WriteOutcome::Created(h.clone()).hash(), "deadbeef");
        assert_eq!(WriteOutcome::AlreadyExists(h).hash(), "deadbeef");
    }

    // ─── Point cloud tests ──────────────────────────────────────────────────

    fn m1_point_cloud_entry() -> SensorRegistryEntry {
        SensorRegistryEntry {
            peer_id: "test-peer".into(),
            sensor_id: "K1-AABBCCDDEEFF/head_depth_points".into(),
            body: SensorBody::Rangefinder(Rangefinder {
                r#type: "point_cloud".into(),
                fields: vec![
                    PointField {
                        name: "x".into(),
                        offset: 0,
                        datatype: PointFieldDataType::Float32,
                        count: 1,
                    },
                    PointField {
                        name: "y".into(),
                        offset: 4,
                        datatype: PointFieldDataType::Float32,
                        count: 1,
                    },
                    PointField {
                        name: "z".into(),
                        offset: 8,
                        datatype: PointFieldDataType::Float32,
                        count: 1,
                    },
                ],
                point_step: 12,
                is_bigendian: false,
                frame_rate_hz: 10,
                frame: RegistryRef {
                    peer_id: "test-peer".into(),
                    id: "K1-AABBCCDDEEFF/head_left_cam_optical".into(),
                    hash: M1_OPTICAL_FRAME_HASH.into(),
                },
            }),
        }
    }

    #[test]
    fn point_cloud_entry_serializes_to_canonical_bytes() {
        let bytes = m1_point_cloud_entry().canonical_bytes();
        // Keys in JCS order; frame_id+frame_hash replaced by nested frame object;
        // variant discriminator is now "kind":"rangefinder"; open-string type is "type":"point_cloud";
        // peer_id added at top level.
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"fields":[{"count":1,"datatype":"float32","name":"x","offset":0},{"count":1,"datatype":"float32","name":"y","offset":4},{"count":1,"datatype":"float32","name":"z","offset":8}],"frame":{"hash":"03b86f32827ec6a25a5e619b2f36478b","id":"K1-AABBCCDDEEFF/head_left_cam_optical","peer_id":"test-peer"},"frame_rate_hz":10,"is_bigendian":false,"kind":"rangefinder","peer_id":"test-peer","point_step":12,"sensor_id":"K1-AABBCCDDEEFF/head_depth_points","type":"point_cloud"}"#
        );
    }

    #[test]
    fn point_cloud_entry_hash_is_locked() {
        // Pin the XXH3-128 of the M1 example rangefinder (formerly point_cloud) entry.
        // Updates to this must be coordinated with any cross-language reader.
        // Recomputed for #216 rev 2: peer_id added, PointCloud→Rangefinder,
        // frame_id+frame_hash→RegistryRef, kind tag renamed.
        // Updated for Task 1.4: frame peer_id changed from "galbot" to "test-peer".
        assert_eq!(
            m1_point_cloud_entry().hash(),
            "9522242bd92110b03c024e512e0274cd"
        );
    }

    #[test]
    fn write_then_read_point_cloud_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        write_m1_optical_frame(dir.path());
        let entry = m1_point_cloud_entry();
        let outcome = write_sensor(dir.path(), &entry).unwrap();
        let hash = outcome.hash().to_string();
        let read = read_sensor(dir.path(), &entry.peer_id, &entry.sensor_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
    }

    #[test]
    fn write_sensor_rejects_missing_frame_reference() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_sensor_entry();
        let err = write_sensor(dir.path(), &entry).unwrap_err();
        assert!(
            matches!(err, Error::FrameReferenceMissing { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn write_sensor_rejects_empty_frame_hash() {
        let dir = tempfile::tempdir().unwrap();
        write_m1_optical_frame(dir.path());
        let mut entry = m1_sensor_entry();
        match &mut entry.body {
            SensorBody::Camera(cam) => cam.frame.hash.clear(),
            SensorBody::Rangefinder(_)
            | SensorBody::Rf(_)
            | SensorBody::Audio(_)
            | SensorBody::JointEncoders(_)
            | SensorBody::Scalar(_) => {
                panic!("test was set up for Camera")
            }
        }
        let err = write_sensor(dir.path(), &entry).unwrap_err();
        assert!(
            matches!(err, Error::FrameReferenceMissing { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn camera_registry_rejects_raw_frames_without_a_stride() {
        let dir = tempfile::tempdir().unwrap();
        let mut entry = m1_sensor_entry();
        let SensorBody::Camera(camera) = &mut entry.body else {
            unreachable!()
        };
        camera.row_stride_bytes = 0;
        assert!(matches!(
            write_sensor(dir.path(), &entry),
            Err(Error::InvalidImageLayout(_))
        ));
    }

    #[test]
    fn camera_registry_rejects_stride_on_compressed_frames() {
        let dir = tempfile::tempdir().unwrap();
        let mut entry = m1_sensor_entry();
        let SensorBody::Camera(camera) = &mut entry.body else {
            unreachable!()
        };
        camera.image_encoding = "jpeg".into();
        assert!(matches!(
            write_sensor(dir.path(), &entry),
            Err(Error::InvalidImageLayout(_))
        ));
    }

    fn test_camera_calibration() -> CameraCalibration {
        CameraCalibration {
            fx: FiniteF64(400.0),
            fy: FiniteF64(401.0),
            cx: FiniteF64(272.5),
            cy: FiniteF64(244.5),
            distortion_coefficients: vec![
                FiniteF64(-0.1),
                FiniteF64(0.05),
                FiniteF64(0.0),
                FiniteF64(0.0),
                FiniteF64(0.0),
            ],
        }
    }

    #[test]
    fn camera_calibration_is_optional_but_content_addressed_when_present() {
        let uncalibrated = m1_sensor_entry();
        let mut calibrated = uncalibrated.clone();
        let SensorBody::Camera(camera) = &mut calibrated.body else {
            unreachable!()
        };
        camera.calibration = Some(test_camera_calibration());

        assert_ne!(uncalibrated.hash(), calibrated.hash());
        let json = String::from_utf8(calibrated.canonical_bytes()).unwrap();
        assert!(json.contains(
            r#""calibration":{"cx":272.5,"cy":244.5,"distortion_coefficients":[-0.1,0.05,0,0,0],"fx":400,"fy":401}"#
        ));
    }

    #[test]
    fn camera_registry_rejects_non_positive_focal_lengths() {
        let dir = tempfile::tempdir().unwrap();
        let mut entry = m1_sensor_entry();
        let SensorBody::Camera(camera) = &mut entry.body else {
            unreachable!()
        };
        let mut calibration = test_camera_calibration();
        calibration.fx = FiniteF64(0.0);
        camera.calibration = Some(calibration);

        assert!(matches!(
            write_sensor(dir.path(), &entry),
            Err(Error::InvalidCameraCalibration(_))
        ));
    }

    #[test]
    fn camera_registry_rejects_non_finite_calibration_values() {
        let dir = tempfile::tempdir().unwrap();
        let mut entry = m1_sensor_entry();
        let SensorBody::Camera(camera) = &mut entry.body else {
            unreachable!()
        };
        let mut calibration = test_camera_calibration();
        calibration.distortion_coefficients[0] = FiniteF64(f64::NAN);
        camera.calibration = Some(calibration);

        assert!(matches!(
            write_sensor(dir.path(), &entry),
            Err(Error::InvalidCameraCalibration(_))
        ));
    }

    #[test]
    fn point_field_datatype_byte_widths() {
        assert_eq!(PointFieldDataType::Int8.byte_width(), 1);
        assert_eq!(PointFieldDataType::Uint8.byte_width(), 1);
        assert_eq!(PointFieldDataType::Int16.byte_width(), 2);
        assert_eq!(PointFieldDataType::Uint16.byte_width(), 2);
        assert_eq!(PointFieldDataType::Int32.byte_width(), 4);
        assert_eq!(PointFieldDataType::Uint32.byte_width(), 4);
        assert_eq!(PointFieldDataType::Float32.byte_width(), 4);
        assert_eq!(PointFieldDataType::Float64.byte_width(), 8);
    }

    // ─── Audio tests ───────────────────────────────────────────────────────

    fn m1_audio_entry() -> SensorRegistryEntry {
        SensorRegistryEntry {
            peer_id: "test-peer".into(),
            sensor_id: "K1-AABBCCDDEEFF/head_array_4mic".into(),
            body: SensorBody::Audio(Audio {
                r#type: "pcm".into(),
                sample_rate_hz: 48_000,
                channels: 4,
                sample_format: "pcm_s16le".into(),
                channel_layout: "n_channel".into(),
                // Re-use the optical frame for test convenience; any valid
                // FrameRegistryEntry is fine — the registry only checks existence.
                frame: RegistryRef {
                    peer_id: "test-peer".into(),
                    id: "K1-AABBCCDDEEFF/head_left_cam_optical".into(),
                    hash: M1_OPTICAL_FRAME_HASH.into(),
                },
            }),
        }
    }

    #[test]
    fn audio_entry_serializes_to_canonical_bytes() {
        let bytes = m1_audio_entry().canonical_bytes();
        // Keys in JCS order; peer_id added; frame ref added; kind discriminator
        // renamed from "type" to "kind"; open-string sensor.type is now "type":"pcm".
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"channel_layout":"n_channel","channels":4,"frame":{"hash":"03b86f32827ec6a25a5e619b2f36478b","id":"K1-AABBCCDDEEFF/head_left_cam_optical","peer_id":"test-peer"},"kind":"audio","peer_id":"test-peer","sample_format":"pcm_s16le","sample_rate_hz":48000,"sensor_id":"K1-AABBCCDDEEFF/head_array_4mic","type":"pcm"}"#
        );
    }

    #[test]
    fn audio_entry_hash_is_locked() {
        // Pin the XXH3-128 of the M1 example audio entry.
        // Updates to this must be coordinated with any cross-language reader.
        // Recomputed for #216 rev 2: peer_id added, frame ref added,
        // kind tag renamed, sensor.type field added.
        // Updated for Task 1.4: frame peer_id changed from "galbot" to "test-peer".
        assert_eq!(m1_audio_entry().hash(), "3cfe29be8c3382753655f3e693068d88");
    }

    #[test]
    fn write_then_read_audio_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        write_m1_optical_frame(dir.path()); // audio entry refs the same optical frame
        let entry = m1_audio_entry();
        let outcome = write_sensor(dir.path(), &entry).unwrap();
        let hash = outcome.hash().to_string();
        let read = read_sensor(dir.path(), &entry.peer_id, &entry.sensor_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
    }

    // ─── JointEncoders tests ───────────────────────────────────────────────

    const M1_BASE_LINK_FRAME_HASH: &str = "476d36916dd2c96f09ea57304d0da334";

    /// Six-DOF arm fixture — `K1` upper-arm shape, plausible publish
    /// rate. Joint count, frame rate, type, and frame ref are the fields
    /// the registry body carries; URDF / joint names live with the consumer.
    fn m1_joint_encoders_entry() -> SensorRegistryEntry {
        SensorRegistryEntry {
            peer_id: "test-peer".into(),
            sensor_id: "K1-AABBCCDDEEFF/right_arm_joints".into(),
            body: SensorBody::JointEncoders(JointEncoders {
                r#type: "absolute".into(),
                joint_count: 6,
                frame_rate_hz: 100,
                // base_link is the kinematic root frame for the joint bank.
                frame: RegistryRef {
                    peer_id: "test-peer".into(),
                    id: "K1-AABBCCDDEEFF/base_link".into(),
                    hash: M1_BASE_LINK_FRAME_HASH.into(),
                },
            }),
        }
    }

    fn write_m1_base_link_frame(app_root: &Path) {
        let outcome = write_frame(app_root, &m1_frame_entry()).unwrap();
        assert_eq!(outcome.hash(), M1_BASE_LINK_FRAME_HASH);
    }

    /// Locks the JCS canonical bytes for the M1 example joint-encoders
    /// entry. Catches drift in entry shape OR canonicalization. Joins
    /// the workspace's cross-language locked-vector set.
    #[test]
    fn joint_encoders_entry_serializes_to_canonical_bytes() {
        let bytes = m1_joint_encoders_entry().canonical_bytes();
        // Keys in JCS order; peer_id added; frame ref added; kind discriminator
        // renamed; open-string sensor.type is "type":"absolute".
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"frame":{"hash":"476d36916dd2c96f09ea57304d0da334","id":"K1-AABBCCDDEEFF/base_link","peer_id":"test-peer"},"frame_rate_hz":100,"joint_count":6,"kind":"joint_encoders","peer_id":"test-peer","sensor_id":"K1-AABBCCDDEEFF/right_arm_joints","type":"absolute"}"#
        );
    }

    /// Locks the XXH3-128 of the canonical bytes. Trips if any of
    /// `auki-jcs`, `auki-hash`, or this crate's serde shape drifts.
    #[test]
    fn joint_encoders_entry_hash_is_locked() {
        // Hash recomputed for #216 rev 2: peer_id, type, and frame fields added.
        // Updated for Task 1.4 when frame peer_id changed from "galbot" to "test-peer".
        assert_eq!(
            m1_joint_encoders_entry().hash(),
            "3098545a72004674e0f5e2eb4f86ee0e"
        );
    }

    #[test]
    fn write_then_read_joint_encoders_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        write_m1_base_link_frame(dir.path());
        let entry = m1_joint_encoders_entry();
        let outcome = write_sensor(dir.path(), &entry).unwrap();
        let hash = outcome.hash().to_string();
        let read = read_sensor(dir.path(), &entry.peer_id, &entry.sensor_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
    }

    // Sensor Log manifest builder + locked PoseSource vectors moved to
    // [`auki-manifests`] in Step 0 of the auki-datatypes migration. The Pose
    // Log payload (`PoseLogEntry` / `TransformSample`) moved to
    // [`auki_datatypes::pose::SpatialTransform`] at Step 5 of the migration —
    // the flat `SpatialTransform` segment entry replaced the
    // `PoseLogEntry { transforms: Vec<...> }` wrapper, and the round-trip
    // tests live in `auki-datatypes::tests` now.

    // ─── Frame Registry tests ──────────────────────────────────────────────

    fn m1_frame_entry() -> FrameRegistryEntry {
        FrameRegistryEntry::ros_body("test-peer", "K1-AABBCCDDEEFF/base_link")
    }

    /// Locks the JCS canonical bytes for the locked Frame Registry vector.
    /// Cross-language readers (Park's browser side, future Sentinel) MUST
    /// produce these exact bytes for the same input. Joins the
    /// `auki-hash` / `auki-identity` / `auki-network` cross-language
    /// conformance set.
    #[test]
    fn frame_entry_serializes_to_canonical_bytes_matching_locked_vector() {
        let bytes = m1_frame_entry().canonical_bytes();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(
            s,
            r#"{"axes":{"x":"forward","y":"left","z":"up"},"frame_id":"K1-AABBCCDDEEFF/base_link","handedness":"right","peer_id":"test-peer","units":"meters"}"#,
        );
    }

    /// Locks the XXH3-128 hex of the locked Frame Registry vector.
    /// Trips if any of `auki-jcs`, `auki-hash`, or this crate's serde
    /// shape drifts.
    #[test]
    fn frame_entry_hash_is_locked() {
        assert_eq!(m1_frame_entry().hash(), "476d36916dd2c96f09ea57304d0da334");
    }

    #[test]
    fn ros_body_preset_matches_explicit_construction() {
        let preset = FrameRegistryEntry::ros_body("galbot", "frame/x");
        let explicit = FrameRegistryEntry {
            peer_id: "galbot".into(),
            frame_id: "frame/x".into(),
            handedness: Handedness::Right,
            axes: AxisConvention {
                x: AxisDirection::Forward,
                y: AxisDirection::Left,
                z: AxisDirection::Up,
            },
            units: LengthUnit::Meters,
        };
        assert_eq!(preset, explicit);
    }

    #[test]
    fn ros_optical_preset_matches_explicit_construction() {
        let preset = FrameRegistryEntry::ros_optical("galbot", "frame/x");
        let explicit = FrameRegistryEntry {
            peer_id: "galbot".into(),
            frame_id: "frame/x".into(),
            handedness: Handedness::Right,
            axes: AxisConvention {
                x: AxisDirection::Right,
                y: AxisDirection::Down,
                z: AxisDirection::Forward,
            },
            units: LengthUnit::Meters,
        };
        assert_eq!(preset, explicit);
    }

    #[test]
    fn opengl_preset_matches_explicit_construction() {
        let preset = FrameRegistryEntry::opengl("galbot", "frame/x");
        let explicit = FrameRegistryEntry {
            peer_id: "galbot".into(),
            frame_id: "frame/x".into(),
            handedness: Handedness::Right,
            axes: AxisConvention {
                x: AxisDirection::Right,
                y: AxisDirection::Up,
                z: AxisDirection::Backward,
            },
            units: LengthUnit::Meters,
        };
        assert_eq!(preset, explicit);
    }

    #[test]
    fn unity_preset_matches_explicit_construction() {
        let preset = FrameRegistryEntry::unity("galbot", "frame/x");
        let explicit = FrameRegistryEntry {
            peer_id: "galbot".into(),
            frame_id: "frame/x".into(),
            handedness: Handedness::Left,
            axes: AxisConvention {
                x: AxisDirection::Right,
                y: AxisDirection::Up,
                z: AxisDirection::Forward,
            },
            units: LengthUnit::Meters,
        };
        assert_eq!(preset, explicit);
    }

    #[test]
    fn validate_rejects_non_orthogonal_axes() {
        let entry = FrameRegistryEntry {
            peer_id: String::new(),
            frame_id: "frame/x".into(),
            handedness: Handedness::Right,
            axes: AxisConvention {
                // x and y both on the forward/backward pair → not orthogonal.
                x: AxisDirection::Forward,
                y: AxisDirection::Backward,
                z: AxisDirection::Up,
            },
            units: LengthUnit::Meters,
        };
        match entry.validate() {
            Err(Error::InvalidAxes(msg)) => assert!(
                msg.contains("orthogonal"),
                "error should mention orthogonality: {msg}"
            ),
            other => panic!("expected InvalidAxes; got {other:?}"),
        }
    }

    #[test]
    fn validate_accepts_all_four_presets() {
        FrameRegistryEntry::ros_body("galbot", "a")
            .validate()
            .unwrap();
        FrameRegistryEntry::ros_optical("galbot", "b")
            .validate()
            .unwrap();
        FrameRegistryEntry::opengl("galbot", "c")
            .validate()
            .unwrap();
        FrameRegistryEntry::unity("galbot", "d").validate().unwrap();
    }

    #[test]
    fn frame_entry_presets_carry_peer_id() {
        let entry = FrameRegistryEntry::ros_body("galbot", "base_link");
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains(r#""peer_id":"galbot""#));
        assert!(json.contains(r#""frame_id":"base_link""#));

        let entry = FrameRegistryEntry::ros_optical("galbot", "head_left_camera_optical");
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains(r#""peer_id":"galbot""#));
        assert!(json.contains(r#""frame_id":"head_left_camera_optical""#));

        let entry = FrameRegistryEntry::opengl("park", "world");
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains(r#""peer_id":"park""#));

        let entry = FrameRegistryEntry::unity("park", "world");
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains(r#""peer_id":"park""#));
    }

    #[test]
    fn write_frame_rejects_non_orthogonal_axes_without_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let entry = FrameRegistryEntry {
            peer_id: String::new(),
            frame_id: "frame/bad".into(),
            handedness: Handedness::Right,
            axes: AxisConvention {
                x: AxisDirection::Up,
                y: AxisDirection::Down,
                z: AxisDirection::Forward,
            },
            units: LengthUnit::Meters,
        };
        match write_frame(dir.path(), &entry) {
            Err(Error::InvalidAxes(_)) => {}
            other => panic!("expected InvalidAxes; got {other:?}"),
        }
        // Verify nothing was written under registries/frames/.
        let frames_root = dir.path().join("registries").join("frames");
        assert!(
            !frames_root.exists(),
            "no on-disk write on validation failure"
        );
    }

    #[test]
    fn write_then_read_frame_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_frame_entry();
        let outcome = write_frame(dir.path(), &entry).unwrap();
        let hash = outcome.hash().to_string();
        let read = read_frame(dir.path(), &entry.peer_id, &entry.frame_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
    }

    #[test]
    fn write_frame_is_idempotent_on_identical_content() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_frame_entry();
        let first = write_frame(dir.path(), &entry).unwrap();
        let second = write_frame(dir.path(), &entry).unwrap();
        assert!(matches!(first, WriteOutcome::Created(_)));
        assert!(matches!(second, WriteOutcome::AlreadyExists(_)));
        assert_eq!(first.hash(), second.hash());
    }

    #[test]
    fn read_frame_returns_none_for_missing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let entry = read_frame(dir.path(), "galbot", "frame/missing", "deadbeef").unwrap();
        assert_eq!(entry, None);
    }

    // `pose_log_manifest_opens_a_log_round_trip` moved to [`auki-manifests`]
    // in Step 0 of the auki-datatypes migration — `build_pose_log_manifest`
    // and `PoseSource` live there now. The PoseLogEntry CBOR round-trip
    // tests above cover this crate's payload-encoding contract.

    // ─── Detector Registry tests (Cuba T4 + T16) ────────────────────────────

    fn cuba_aruco_detector_entry() -> DetectorRegistryEntry {
        DetectorRegistryEntry {
            peer_id: "galbot".into(),
            detector_id: "aukilabs/aruco/v1".into(),
            body: DetectorBody::Aruco(Aruco {
                dictionary: "5x5_50".into(),
            }),
            input_types: vec![],
            output_types: vec!["aruco".into()],
        }
    }

    #[test]
    fn detector_entry_canonical_bytes_lock_the_aruco_shape() {
        let bytes = cuba_aruco_detector_entry().canonical_bytes();
        let s = std::str::from_utf8(&bytes).unwrap();
        // Keys sorted lexicographically per RFC 8785 §3.2.3.
        assert_eq!(
            s,
            r#"{"detector_id":"aukilabs/aruco/v1","dictionary":"5x5_50","input_types":[],"output_types":["aruco"],"peer_id":"galbot","type":"aruco"}"#
        );
    }

    #[test]
    fn detector_entry_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let entry = cuba_aruco_detector_entry();
        let outcome = write_detector(dir.path(), &entry).unwrap();
        let hash = match outcome {
            WriteOutcome::Created(h) => h,
            other => panic!("unexpected: {other:?}"),
        };
        let read = read_detector(dir.path(), &entry.peer_id, &entry.detector_id, &hash)
            .unwrap()
            .expect("entry must read back");
        assert_eq!(read, entry);
    }

    #[test]
    fn detector_entry_write_is_idempotent_on_hash() {
        let dir = tempfile::tempdir().unwrap();
        let entry = cuba_aruco_detector_entry();
        let first = write_detector(dir.path(), &entry).unwrap();
        let second = write_detector(dir.path(), &entry).unwrap();
        assert!(matches!(first, WriteOutcome::Created(_)));
        assert!(matches!(second, WriteOutcome::AlreadyExists(_)));
        assert_eq!(first.hash(), second.hash());
    }

    #[test]
    fn detector_entry_two_dictionaries_get_distinct_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let five = cuba_aruco_detector_entry();
        let four = DetectorRegistryEntry {
            body: DetectorBody::Aruco(Aruco {
                dictionary: "4x4_50".into(),
            }),
            peer_id: "galbot".into(),
            ..cuba_aruco_detector_entry()
        };
        let h1 = write_detector(dir.path(), &five).unwrap();
        let h2 = write_detector(dir.path(), &four).unwrap();
        assert_ne!(h1.hash(), h2.hash());
    }

    #[test]
    fn custom_detector_configuration_is_serialized_and_content_addressed() {
        let entry = DetectorRegistryEntry {
            peer_id: "robot".into(),
            detector_id: "developer-detector".into(),
            body: DetectorBody::Custom(CustomDetector {
                kind: "com.example.detector".into(),
                configuration: serde_json::json!({"threshold": 0.7}),
            }),
            input_types: vec![],
            output_types: vec!["example".into()],
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["type"], "custom");
        assert_eq!(json["kind"], "com.example.detector");
        assert_eq!(json["configuration"]["threshold"], 0.7);

        let mut changed = entry.clone();
        let DetectorBody::Custom(custom) = &mut changed.body else {
            unreachable!();
        };
        custom.configuration = serde_json::json!({"threshold": 0.8});
        assert_ne!(entry.hash(), changed.hash());
    }

    #[test]
    fn detector_entry_slash_in_id_becomes_double_underscore() {
        let dir = tempfile::tempdir().unwrap();
        let entry = cuba_aruco_detector_entry();
        write_detector(dir.path(), &entry).unwrap();
        let expected_dir = dir
            .path()
            .join("registries")
            .join("detectors")
            .join(&entry.peer_id)
            .join("aukilabs__aruco__v1");
        assert!(expected_dir.is_dir(), "expected {expected_dir:?} to exist");
    }

    #[test]
    fn detector_entry_supports_multiple_output_types() {
        let entry = DetectorRegistryEntry {
            peer_id: "galbot".into(),
            detector_id: "aukilabs/qr/v1".into(),
            body: DetectorBody::Qr(Qr {}),
            input_types: vec![DetectorInput {
                sensor_kind: "camera".into(),
                sensor_type: None,
                image_encoding: Some("raw".into()),
                pixel_format: Some("luma8".into()),
            }],
            output_types: vec!["portal".into(), "portal_corner".into()],
        };
        let s = std::str::from_utf8(&entry.canonical_bytes())
            .unwrap()
            .to_string();
        assert!(s.contains(r#""output_types":["portal","portal_corner"]"#));
        assert!(s.contains(r#""type":"qr""#));
    }

    #[test]
    fn detector_input_matches_exact_camera_byte_contract() {
        let input = DetectorInput {
            sensor_kind: "camera".into(),
            sensor_type: None,
            image_encoding: Some("raw".into()),
            pixel_format: Some("luma8".into()),
        };
        let mut sensor = m1_sensor_entry().body;
        assert!(!input.matches(&sensor));
        let SensorBody::Camera(camera) = &mut sensor else {
            unreachable!()
        };
        camera.pixel_format = "luma8".into();
        assert!(input.matches(&sensor));
    }

    #[test]
    fn detector_entry_accepts_any_matching_input_alternative() {
        let entry = DetectorRegistryEntry {
            peer_id: "robot".into(),
            detector_id: "qr".into(),
            body: DetectorBody::Qr(Qr {}),
            input_types: vec![DetectorInput {
                sensor_kind: "camera".into(),
                sensor_type: None,
                image_encoding: Some("raw".into()),
                pixel_format: Some("luma8".into()),
            }],
            output_types: vec!["qr".into()],
        };
        let mut sensor = m1_sensor_entry().body;
        let SensorBody::Camera(camera) = &mut sensor else {
            unreachable!()
        };
        camera.pixel_format = "luma8".into();
        assert!(entry.accepts_input(&sensor));
    }

    // ─── New-shape canonical JSON test (#216 rev 2 TDD anchor) ─────────────

    /// TDD anchor: asserts the new Camera + Rangefinder + Rf canonical JSON
    /// shape after #216 rev 2 restructure. Written first (red), then the
    /// struct changes made it green. The assertions capture:
    ///   - `peer_id` at top level
    ///   - variant discriminator as `"kind"` (not `"type"`)
    ///   - open-string `"type"` field inside each body
    ///   - `"frame"` nested object replacing `frame_id`+`frame_hash`
    #[test]
    fn new_shape_camera_rangefinder_rf_canonical_json() {
        let frame_ref = RegistryRef {
            peer_id: "galbot".into(),
            id: "head_optical".into(),
            hash: "abc123".into(),
        };

        let camera = SensorRegistryEntry {
            peer_id: "galbot".into(),
            sensor_id: "head_rgb".into(),
            body: SensorBody::Camera(Camera {
                r#type: "rgb".into(),
                width: 1920,
                height: 1200,
                frame_rate_hz: 30,
                image_encoding: "raw".into(),
                pixel_format: "rgb8".into(),
                row_stride_bytes: 1920 * 3,
                color_space: "srgb".into(),
                intrinsics_model: "pinhole".into(),
                distortion_model: "brown_conrady".into(),
                calibration: None,
                frame: frame_ref.clone(),
            }),
        };
        let camera_json = std::str::from_utf8(&camera.canonical_bytes())
            .unwrap()
            .to_string();
        // Must contain kind:camera, type:rgb, peer_id, nested frame
        assert!(
            camera_json.contains(r#""kind":"camera""#),
            "camera: {camera_json}"
        );
        assert!(
            camera_json.contains(r#""type":"rgb""#),
            "camera type: {camera_json}"
        );
        assert!(
            camera_json.contains(r#""peer_id":"galbot""#),
            "camera peer_id: {camera_json}"
        );
        assert!(
            camera_json
                .contains(r#""frame":{"hash":"abc123","id":"head_optical","peer_id":"galbot"}"#),
            "camera frame: {camera_json}"
        );

        let rangefinder = SensorRegistryEntry {
            peer_id: "galbot".into(),
            sensor_id: "head_lidar".into(),
            body: SensorBody::Rangefinder(Rangefinder {
                r#type: "3d_lidar".into(),
                fields: vec![],
                point_step: 0,
                is_bigendian: false,
                frame_rate_hz: 10,
                frame: frame_ref.clone(),
            }),
        };
        let rf_json = std::str::from_utf8(&rangefinder.canonical_bytes())
            .unwrap()
            .to_string();
        assert!(
            rf_json.contains(r#""kind":"rangefinder""#),
            "rangefinder kind: {rf_json}"
        );
        assert!(
            rf_json.contains(r#""type":"3d_lidar""#),
            "rangefinder type: {rf_json}"
        );

        let rf = SensorRegistryEntry {
            peer_id: "galbot".into(),
            sensor_id: "ble_beacon".into(),
            body: SensorBody::Rf(Rf {
                r#type: "bluetooth".into(),
                frame: frame_ref.clone(),
            }),
        };
        let ble_json = std::str::from_utf8(&rf.canonical_bytes())
            .unwrap()
            .to_string();
        assert!(ble_json.contains(r#""kind":"rf""#), "rf kind: {ble_json}");
        assert!(
            ble_json.contains(r#""type":"bluetooth""#),
            "rf type: {ble_json}"
        );
    }
}

#[cfg(test)]
mod id_charset_tests {
    use super::*;

    #[test]
    fn rejects_disallowed_chars() {
        let bad_ids = ["foo>bar", "foo@bar", "foo bar", "foo\tbar", "foo\nbar"];
        for bad in bad_ids {
            let result = validate_registry_id(bad);
            assert!(
                matches!(result, Err(RegistryIdError::DisallowedChar(_))),
                "id {bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_empty_id() {
        let result = validate_registry_id("");
        assert_eq!(result, Err(RegistryIdError::Empty));
    }

    #[test]
    fn allows_slash_underscore_dash_dot() {
        for good in [
            "foo/bar",
            "foo_bar",
            "foo-bar",
            "a.b.c",
            "a/b/c",
            "head_left_rgb",
            "session/sdk_clock",
        ] {
            assert!(
                validate_registry_id(good).is_ok(),
                "id {good:?} should be allowed"
            );
        }
    }

    #[test]
    fn each_entry_type_has_validate_id() {
        // Just smoke — confirm each entry type exposes a validate_id function delegating to the helper.
        assert!(SensorRegistryEntry::validate_id("head_left_rgb").is_ok());
        assert!(ClockRegistryEntry::validate_id("session/sdk_clock").is_ok());
        assert!(FrameRegistryEntry::validate_id("base_link").is_ok());
        assert!(DetectorRegistryEntry::validate_id("yolo_v8").is_ok());

        assert!(SensorRegistryEntry::validate_id("bad>id").is_err());
        assert!(ClockRegistryEntry::validate_id("bad@id").is_err());
        assert!(FrameRegistryEntry::validate_id("bad id").is_err());
        assert!(DetectorRegistryEntry::validate_id("").is_err());
        assert!(MapRegistryEntry::validate_id("voxel/world").is_ok());
    }

    #[test]
    fn voxel_map_registry_entry_is_content_addressed_and_rejects_invalid_grid() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = MapRegistryEntry {
            peer_id: "galbot".into(),
            map_id: "world".into(),
            body: MapBody::Voxel(VoxelMap {
                frame: RegistryRef {
                    peer_id: "galbot".into(),
                    id: "world".into(),
                    hash: "frame-hash".into(),
                },
                voxel_size_m: FiniteF64(0.05),
                chunk_dimension: 64,
                value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                color_model: None,
                semantic_classes: vec!["floor".into()],
            }),
        };
        write_map(tmp.path(), &entry).unwrap();
        assert_eq!(
            read_map(tmp.path(), "galbot", "world", &entry.hash()).unwrap(),
            Some(entry)
        );

        let invalid = MapRegistryEntry {
            peer_id: "galbot".into(),
            map_id: "bad".into(),
            body: MapBody::Voxel(VoxelMap {
                frame: RegistryRef {
                    peer_id: "galbot".into(),
                    id: "world".into(),
                    hash: "frame-hash".into(),
                },
                voxel_size_m: FiniteF64(0.0),
                chunk_dimension: 0,
                value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                color_model: None,
                semantic_classes: vec![],
            }),
        };
        assert!(matches!(
            write_map(tmp.path(), &invalid),
            Err(Error::InvalidMap(_))
        ));
    }

    #[test]
    fn device_model_and_blobs_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let urdf = put_blob(tmp.path(), b"<robot name='test'/>").unwrap();
        let mesh = put_blob(tmp.path(), b"mesh bytes").unwrap();
        let entry = DeviceModelRegistryEntry {
            peer_id: "galbot".into(),
            device_model_id: "unitree/g1".into(),
            body: DeviceModelBody {
                model_id: "unitree_g1".into(),
                format: DeviceModelFormat::Urdf {
                    urdf_sha256: urdf.clone(),
                    meshes: vec![MeshBlobRef {
                        path: "meshes/body.glb".into(),
                        sha256: mesh.clone(),
                    }],
                },
                root_convention: Some("ros_rep_103".into()),
            },
        };
        let bytes = entry.canonical_bytes();
        let json = std::str::from_utf8(&bytes).unwrap();
        assert!(json.contains(r#""type":"urdf""#));
        let outcome = write_device_model(tmp.path(), &entry).unwrap();
        assert_eq!(
            read_device_model(tmp.path(), "galbot", "unitree/g1", outcome.hash()).unwrap(),
            Some(entry)
        );
        assert_eq!(get_blob(tmp.path(), &mesh).unwrap(), Some(b"mesh bytes".to_vec()));
        assert!(blob_exists(tmp.path(), &urdf).unwrap());
        let listed = list_device_models(tmp.path(), "galbot").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "unitree/g1");
        assert_eq!(listed[0].hash, outcome.hash());
    }

    #[test]
    fn put_urdf_package_rewrites_and_blobs() {
        let pkg = tempfile::tempdir().unwrap();
        let meshes = pkg.path().join("meshes");
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("body.stl"), b"mesh").unwrap();
        let urdf_path = pkg.path().join("robot.urdf");
        fs::write(
            &urdf_path,
            r#"<robot name="K1">
                <link name="base">
                  <visual><geometry><mesh filename="package://K1_URDF_Serial/meshes/body.stl"/></geometry></visual>
                </link>
            </robot>"#,
        )
        .unwrap();
        let app = tempfile::tempdir().unwrap();
        let package = put_urdf_package(app.path(), &urdf_path, Some("ros_body".into())).unwrap();
        assert_eq!(package.device_model_id, "k1");
        let (urdf_sha, meshes) = package.body.as_urdf().unwrap();
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].path, "meshes/body.stl");
        let urdf = get_blob(app.path(), urdf_sha).unwrap().unwrap();
        let text = String::from_utf8(urdf).unwrap();
        assert!(text.contains(r#"filename="meshes/body.stl""#));
        assert!(!text.contains("package://"));
    }

    #[test]
    fn scalar_sensor_is_non_spatial_and_content_addressed() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = SensorRegistryEntry {
            peer_id: "bracketbot".into(),
            sensor_id: "battery_charge".into(),
            body: SensorBody::Scalar(Scalar {
                r#type: "battery_charge".into(),
                unit: "percent".into(),
                expected_rate_hz: 1,
            }),
        };

        assert_eq!(
            std::str::from_utf8(&entry.canonical_bytes()).unwrap(),
            r#"{"expected_rate_hz":1,"kind":"scalar","peer_id":"bracketbot","sensor_id":"battery_charge","type":"battery_charge","unit":"percent"}"#
        );
        let outcome = write_sensor(tmp.path(), &entry).unwrap();
        assert_eq!(
            read_sensor(tmp.path(), &entry.peer_id, &entry.sensor_id, outcome.hash()).unwrap(),
            Some(entry)
        );
    }

    #[test]
    fn scalar_sensor_rejects_incomplete_contracts() {
        for body in [
            Scalar {
                r#type: String::new(),
                unit: "percent".into(),
                expected_rate_hz: 1,
            },
            Scalar {
                r#type: "battery_charge".into(),
                unit: String::new(),
                expected_rate_hz: 1,
            },
            Scalar {
                r#type: "battery_charge".into(),
                unit: "percent".into(),
                expected_rate_hz: 0,
            },
        ] {
            let entry = SensorRegistryEntry {
                peer_id: "bracketbot".into(),
                sensor_id: "battery_charge".into(),
                body: SensorBody::Scalar(body),
            };
            assert!(matches!(
                write_sensor(tempfile::tempdir().unwrap().path(), &entry),
                Err(Error::InvalidScalar(_))
            ));
        }
    }
}

#[cfg(test)]
mod ref_tests {
    use super::*;

    #[test]
    fn registry_ref_round_trips_canonical_json() {
        let peer_id_str = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
        let r = RegistryRef {
            peer_id: peer_id_str.to_string(),
            id: "head_left_rgb".to_string(),
            hash: "abc123".to_string(),
        };
        let value = serde_json::to_value(&r).unwrap();
        let json_bytes = auki_jcs::canonicalize(&value);
        let json = String::from_utf8(json_bytes).unwrap();
        assert_eq!(
            json,
            r#"{"hash":"abc123","id":"head_left_rgb","peer_id":"12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"}"#
        );
        let r2: RegistryRef = serde_json::from_str(&json).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn log_ref_round_trips_canonical_json() {
        let peer_id_str = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
        let r = LogRef {
            source_peer_id: peer_id_str.to_string(),
            resource_id: "head_left_rgb".to_string(),
        };
        let value = serde_json::to_value(&r).unwrap();
        let json_bytes = auki_jcs::canonicalize(&value);
        let json = String::from_utf8(json_bytes).unwrap();
        assert_eq!(
            json,
            r#"{"resource_id":"head_left_rgb","source_peer_id":"12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"}"#
        );
        let r2: LogRef = serde_json::from_str(&json).unwrap();
        assert_eq!(r, r2);
    }
}
