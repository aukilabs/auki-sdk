//! Sensor + Clock Registry entries with content-addressed multi-version-by-hash
//! on-disk storage.
//!
//! An entry is built from typed fields, canonicalized via [`auki_jcs`], hashed
//! via [`auki_hash`], and persisted at
//! `<app_root>/registries/{sensors,clocks}/<id>/<hash>.json`. Path layout lives
//! in [`auki_session`]; this crate composes its helpers. Slashes in IDs are
//! replaced with `__` in path segments. Re-writing identical content is a
//! no-op; writing different content under the same id produces a sibling file.
//!
//! The hash *is* the version. There are no version counters.
//!
//! Registries live at the **app root**, not the session root — they're shared
//! across all sessions of the same app. Hash-keyed writes are idempotent, so
//! the same sensor entry produces the same `<hash>.json` regardless of session.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ─── Sensor Registry ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorRegistryEntry {
    pub sensor_id: String,
    #[serde(flatten)]
    pub body: SensorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SensorBody {
    RgbCamera(RgbCamera),
    PointCloud(PointCloud),
    Microphone(Microphone),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbCamera {
    pub width: u32,
    pub height: u32,
    pub frame_rate_hz: u32,
    pub pixel_format: String,
    pub color_space: String,
    pub intrinsics_model: String,
    pub distortion_model: String,
    /// Frame Registry id for the camera optical frame — what the pixel
    /// rays' depth axis points along, etc. Consumers look up the
    /// matching [`FrameRegistryEntry`] under
    /// `<app_root>/registries/frames/<frame_id>/`. Conventionally the
    /// REP-103 optical convention (`X right, Y down, Z forward`); the
    /// SDK does not enforce a specific convention.
    pub frame_id: String,
}

/// Static layout of a point-cloud sensor's per-point bytes. The actual point
/// data lives in the per-frame log payload (`PointCloudLogEntry`); this
/// describes how to interpret those bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointCloud {
    pub fields: Vec<PointField>,
    pub point_step: u32,
    pub is_bigendian: bool,
    pub frame_rate_hz: u32,
    /// Frame Registry id for the coordinate system the point bytes are
    /// in. ROS `PointCloud2` carries `header.frame_id`; the integrator
    /// threads it through here so consumers (Park, future Sentinel)
    /// know which Frame Registry entry tells them how to interpret the
    /// XYZ axes and units.
    pub frame_id: String,
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

/// Static identity of a microphone (or microphone array) — the bits that
/// describe how to interpret the bytes downstream consumers will see in
/// [`AudioLogEntry`].
///
/// **Multi-microphone arrays are modelled as one sensor with `channels = N`,
/// not as N independent sensors.** This is right for physically-synchronized
/// arrays where the channels share a clock and a beam-forming origin. Use
/// separate `SensorRegistryEntry` records only when mics are physically
/// independent capture devices on different chips.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Microphone {
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
}

impl SensorRegistryEntry {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonicalize(self)
    }

    pub fn hash(&self) -> String {
        auki_hash::hash_jcs_bytes(&self.canonical_bytes())
    }
}

// ─── Sensor Log payload ──────────────────────────────────────────────────────

/// Per-frame intrinsics + distortion. Pulled out of the registry-side identity
/// because intrinsics can refine at runtime (autofocus, calibration updates).
///
/// Lives in `auki-registry` so that *consumers* of a Sensor Log (renderers,
/// analysis tools) don't have to depend on a ROS adapter just to deserialize
/// the payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicIntrinsics {
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub distortion_coefficients: Vec<f64>,
}

/// The Sensor Log payload (CBOR-encoded under auki-logs framing). The frame
/// timestamp lives in the framing's `timestamp_ns`, not here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorLogEntry {
    pub dynamic_intrinsics: DynamicIntrinsics,
    /// Encoded as a CBOR byte string (major type 2) rather than an array of
    /// u8 — same on-disk semantics, ~half the byte cost for typical frames.
    #[serde(with = "serde_bytes")]
    pub frame: Vec<u8>,
}

/// The Point Cloud Log payload (CBOR-encoded under auki-logs framing). The
/// frame timestamp lives in the framing's `timestamp_ns`, not here. The byte
/// layout of `data` is described by the corresponding `SensorBody::PointCloud`
/// registry entry referenced by the log's manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointCloudLogEntry {
    /// Organized: cols. Unorganized: total point count.
    pub width: u32,
    /// Organized: rows. Unorganized: 1.
    pub height: u32,
    /// True if `data` contains no invalid (NaN/Inf) points.
    pub is_dense: bool,
    /// `data.len()` MUST equal `point_step × width × height` where `point_step`
    /// comes from the registry entry. Encoded as a CBOR byte string.
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

/// The Audio Log payload (CBOR-encoded under auki-logs framing). Each entry
/// is one chunk of audio samples; the framing's `timestamp_ns` is the
/// chunk's start time. The byte layout of `data` is described by the
/// corresponding `SensorBody::Microphone` registry entry referenced by the
/// log's manifest.
///
/// Samples are **interleaved**: for `channels = N`, the byte stream is
/// `[s0_c0, s0_c1, ..., s0_cN-1, s1_c0, s1_c1, ..., s1_cN-1, ...]`. Each
/// sample's encoding is the registry entry's `sample_format`.
///
/// Chunk size (samples per entry) is the integrator's choice; the SDK does
/// not impose a value. Typical: 10–100 ms of samples per chunk at 48 kHz.
/// Sample count per chunk is `data.len() / (sample_byte_width × channels)`
/// where `sample_byte_width` is determined by `sample_format`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioLogEntry {
    /// Interleaved samples per the registry's `sample_format` and `channels`.
    /// Encoded as a CBOR byte string.
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

// ─── Pose Log ────────────────────────────────────────────────────────────────

// `PoseSource` (the inline producer-identity tagged enum that lives in the
// Pose Log manifest) moved to [`auki-manifests`] in Step 0 of the
// auki-datatypes migration. The Pose Log payload types below stay here
// until Step 5 of the migration; see ../auki-datatypes/src/sprint.md.

/// One Pose Log entry = one batch of transforms (one ROS `TFMessage`, or
/// the equivalent for other producers). Empty `transforms` is permitted —
/// some producers occasionally publish empty messages.
///
/// The auki-logs framing's `timestamp_ns` is the message arrival time on
/// the daemon's clock. Per-`TransformSample` timestamps from the wire
/// format are not preserved in v1; if downstream needs them, add them
/// alongside without breaking the existing shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoseLogEntry {
    pub transforms: Vec<TransformSample>,
}

/// One `parent → child` transform sample. Translation is `[x, y, z]` in
/// the frame's units (typically meters). Rotation is a unit quaternion in
/// `[x, y, z, w]` order (Hamilton convention; matches ROS `geometry_msgs`).
/// `f64` matches the underlying ROS `geometry_msgs` types and preserves
/// precision at large-map scales.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformSample {
    pub parent_frame: String,
    pub child_frame: String,
    pub translation: [f64; 3],
    pub rotation_quat: [f64; 4],
}

// `build_pose_log_manifest` moved to [`auki-manifests`] in Step 0 of the
// auki-datatypes migration.

// ─── Clock Registry ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockRegistryEntry {
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
/// live in the Pose Log as [`TransformSample`]s with `parent_frame` /
/// `child_frame` strings naming entries in this registry.
///
/// **Rotation representation** is fixed at the [`TransformSample`]
/// layer (Hamilton convention, `[x, y, z, w]`); not per-frame.
///
/// Construct via the field-explicit struct literal or via one of the
/// `ros_body` / `ros_optical` / `opengl` / `unity` preset constructors —
/// the on-disk JSON is fully spelled out either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRegistryEntry {
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
    pub fn ros_body(frame_id: impl Into<String>) -> Self {
        Self {
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
    pub fn ros_optical(frame_id: impl Into<String>) -> Self {
        Self {
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
    pub fn opengl(frame_id: impl Into<String>) -> Self {
        Self {
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
    pub fn unity(frame_id: impl Into<String>) -> Self {
        Self {
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
    /// `frame_id` did not match the id in the requested path. Indicates
    /// a misplaced or tampered file — content addressing is meant to
    /// make this detectable.
    IdMismatch { expected: String, found: String },
    /// On write of a [`FrameRegistryEntry`], the [`AxisConvention`]
    /// triplet was not orthogonal — i.e. two of `x`/`y`/`z` came from
    /// the same axis-pair (forward/backward, left/right, or up/down).
    InvalidAxes(String),
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
    let bytes = entry.canonical_bytes();
    let hash = auki_hash::hash_jcs_bytes(&bytes);
    let path = auki_session::sensor_entry_path(app_root, &entry.sensor_id, &hash);
    write_entry_at(&path, hash, &bytes)
}

/// Write a clock registry entry under `<app_root>/registries/clocks/...`.
pub fn write_clock(app_root: &Path, entry: &ClockRegistryEntry) -> Result<WriteOutcome> {
    let bytes = entry.canonical_bytes();
    let hash = auki_hash::hash_jcs_bytes(&bytes);
    let path = auki_session::clock_entry_path(app_root, &entry.clock_id, &hash);
    write_entry_at(&path, hash, &bytes)
}

/// Read a sensor registry entry by `(sensor_id, hash)`. Returns `Ok(None)` when
/// the file doesn't exist; `Err(IdMismatch)` if the on-disk entry's
/// `sensor_id` differs from the requested id.
pub fn read_sensor(
    app_root: &Path,
    sensor_id: &str,
    hash: &str,
) -> Result<Option<SensorRegistryEntry>> {
    let path = auki_session::sensor_entry_path(app_root, sensor_id, hash);
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

/// Read a clock registry entry by `(clock_id, hash)`.
pub fn read_clock(
    app_root: &Path,
    clock_id: &str,
    hash: &str,
) -> Result<Option<ClockRegistryEntry>> {
    let path = auki_session::clock_entry_path(app_root, clock_id, hash);
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
    let path = auki_session::frame_entry_path(app_root, &entry.frame_id, &hash);
    write_entry_at(&path, hash, &bytes)
}

/// Read a frame registry entry by `(frame_id, hash)`.
pub fn read_frame(
    app_root: &Path,
    frame_id: &str,
    hash: &str,
) -> Result<Option<FrameRegistryEntry>> {
    let path = auki_session::frame_entry_path(app_root, frame_id, hash);
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

fn read_at(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(e)),
    }
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

    fn m1_sensor_entry() -> SensorRegistryEntry {
        SensorRegistryEntry {
            sensor_id: "K1-AABBCCDDEEFF/head_left_cam".into(),
            body: SensorBody::RgbCamera(RgbCamera {
                width: 544,
                height: 488,
                frame_rate_hz: 20,
                pixel_format: "YUV_NV12".into(),
                color_space: "BT.709".into(),
                intrinsics_model: "pinhole".into(),
                distortion_model: "plumb_bob".into(),
                frame_id: "K1-AABBCCDDEEFF/head_left_cam_optical".into(),
            }),
        }
    }

    fn m1_monotonic_entry() -> ClockRegistryEntry {
        ClockRegistryEntry {
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
        assert_eq!(
            s,
            r#"{"color_space":"BT.709","distortion_model":"plumb_bob","frame_id":"K1-AABBCCDDEEFF/head_left_cam_optical","frame_rate_hz":20,"height":488,"intrinsics_model":"pinhole","pixel_format":"YUV_NV12","sensor_id":"K1-AABBCCDDEEFF/head_left_cam","type":"rgb_camera","width":544}"#
        );
    }

    #[test]
    fn monotonic_clock_canonical_bytes_match_m1_example() {
        let bytes = m1_monotonic_entry().canonical_bytes();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"clock_id":"K1-AABBCCDDEEFF/monotonic","epoch":null,"monotonic":true,"scope":"device-local","type":"monotonic_clock","unit":"milliseconds"}"#
        );
    }

    #[test]
    fn utc_clock_canonical_bytes_match_m1_example() {
        let bytes = m1_utc_entry().canonical_bytes();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"clock_id":"K1-AABBCCDDEEFF/utc","epoch":"1970-01-01T00:00:00Z","monotonic":false,"scope":"global","type":"utc_clock","unit":"milliseconds"}"#
        );
    }

    /// Locks the XXH3-128 hex of the M1 sensor entry. Catches drift in
    /// entry shape, canonicalization, or hashing. Recomputed at v0.0.22
    /// when `frame_id` was added to RgbCamera per the Frame Registry
    /// rollout.
    #[test]
    fn sensor_entry_hash_is_locked() {
        assert_eq!(
            m1_sensor_entry().hash(),
            "d798fa879c80a5b00cabc1ce47ca4f7a"
        );
    }

    #[test]
    fn monotonic_clock_hash_is_locked() {
        assert_eq!(
            m1_monotonic_entry().hash(),
            "1f2176888b1a6621315033f22659b9f3"
        );
    }

    #[test]
    fn utc_clock_hash_is_locked() {
        assert_eq!(
            m1_utc_entry().hash(),
            "89f84f4c2e09bef81d385b2af1d17e6c"
        );
    }

    #[test]
    fn write_then_read_sensor_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_sensor_entry();
        let outcome = write_sensor(dir.path(), &entry).unwrap();
        let hash = match outcome {
            WriteOutcome::Created(h) => h,
            other => panic!("expected Created, got {other:?}"),
        };
        let read = read_sensor(dir.path(), &entry.sensor_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
    }

    #[test]
    fn write_then_read_clock_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_utc_entry();
        let outcome = write_clock(dir.path(), &entry).unwrap();
        let hash = outcome.hash().to_string();
        let read = read_clock(dir.path(), &entry.clock_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
    }

    #[test]
    fn multi_version_same_content_is_no_op() {
        let dir = tempfile::tempdir().unwrap();
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
        let mut entry = m1_sensor_entry();
        let first_hash = entry.hash();
        write_sensor(dir.path(), &entry).unwrap();

        // Mutate a static field — produces a new content hash and a sibling file.
        // Match (not `if let`): exhaustiveness means a future SensorBody variant
        // becomes a compile error pointing the author here.
        match &mut entry.body {
            SensorBody::RgbCamera(cam) => {
                cam.width = 1920;
                cam.height = 1080;
            }
            SensorBody::PointCloud(_) | SensorBody::Microphone(_) => {
                panic!("test was set up for RgbCamera")
            }
        }
        let second_hash = entry.hash();
        assert_ne!(first_hash, second_hash);
        write_sensor(dir.path(), &entry).unwrap();

        let entry_dir = dir
            .path()
            .join("registries")
            .join("sensors")
            .join("K1-AABBCCDDEEFF__head_left_cam");
        let json_count = fs::read_dir(&entry_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .count();
        assert_eq!(json_count, 2);

        // Both resolvable by their respective hashes.
        assert!(read_sensor(dir.path(), &entry.sensor_id, &first_hash)
            .unwrap()
            .is_some());
        let resolved_second =
            read_sensor(dir.path(), &entry.sensor_id, &second_hash).unwrap();
        assert_eq!(resolved_second, Some(entry));
    }

    #[test]
    fn slash_in_id_becomes_double_underscore() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_sensor_entry();
        write_sensor(dir.path(), &entry).unwrap();

        let expected_dir = dir
            .path()
            .join("registries")
            .join("sensors")
            .join("K1-AABBCCDDEEFF__head_left_cam");
        assert!(expected_dir.is_dir(), "expected {expected_dir:?} to exist");

        // Defensive: literal `head_left_cam` subdir under a `K1-AABBCCDDEEFF`
        // dir must NOT exist (would mean we forgot the substitution).
        let bad = dir
            .path()
            .join("registries")
            .join("sensors")
            .join("K1-AABBCCDDEEFF")
            .join("head_left_cam");
        assert!(!bad.exists(), "did not expect nested dirs: {bad:?}");
    }

    #[test]
    fn read_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let result = read_sensor(
            dir.path(),
            "K1-AABBCCDDEEFF/never_written",
            "00000000000000000000000000000000",
        )
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_with_id_mismatch_errors() {
        let dir = tempfile::tempdir().unwrap();
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
            .join("K1-AABBCCDDEEFF__head_left_cam")
            .join(format!("{hash}.json"));
        let bogus_dir = dir
            .path()
            .join("registries")
            .join("sensors")
            .join("K1-AABBCCDDEEFF__other_cam");
        fs::create_dir_all(&bogus_dir).unwrap();
        fs::copy(&real, bogus_dir.join(format!("{hash}.json"))).unwrap();

        let err = read_sensor(dir.path(), "K1-AABBCCDDEEFF/other_cam", &hash);
        assert!(
            matches!(err, Err(Error::IdMismatch { .. })),
            "got {err:?}"
        );
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
            sensor_id: "K1-AABBCCDDEEFF/head_depth_points".into(),
            body: SensorBody::PointCloud(PointCloud {
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
                frame_id: "K1-AABBCCDDEEFF/head_left_cam_optical".into(),
            }),
        }
    }

    #[test]
    fn point_cloud_entry_serializes_to_canonical_bytes() {
        let bytes = m1_point_cloud_entry().canonical_bytes();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"fields":[{"count":1,"datatype":"float32","name":"x","offset":0},{"count":1,"datatype":"float32","name":"y","offset":4},{"count":1,"datatype":"float32","name":"z","offset":8}],"frame_id":"K1-AABBCCDDEEFF/head_left_cam_optical","frame_rate_hz":10,"is_bigendian":false,"point_step":12,"sensor_id":"K1-AABBCCDDEEFF/head_depth_points","type":"point_cloud"}"#
        );
    }

    #[test]
    fn point_cloud_entry_hash_is_locked() {
        // Pin the XXH3-128 of the M1 example point cloud entry.
        // Updates to this must be coordinated with any cross-language reader.
        // Recomputed at v0.0.22 when `frame_id` was added per the Frame
        // Registry rollout. If this trips, either (a) the canonical
        // bytes assertion above also tripped — see that for the cause —
        // or (b) `auki-jcs` / `auki-hash` drifted; investigate before
        // updating.
        assert_eq!(
            m1_point_cloud_entry().hash(),
            "79b58e4e1743d238f93fc27f1a6a5ebf"
        );
    }

    #[test]
    fn write_then_read_point_cloud_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_point_cloud_entry();
        let outcome = write_sensor(dir.path(), &entry).unwrap();
        let hash = outcome.hash().to_string();
        let read = read_sensor(dir.path(), &entry.sensor_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
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

    // ─── Microphone tests ──────────────────────────────────────────────────

    fn m1_microphone_entry() -> SensorRegistryEntry {
        SensorRegistryEntry {
            sensor_id: "K1-AABBCCDDEEFF/head_array_4mic".into(),
            body: SensorBody::Microphone(Microphone {
                sample_rate_hz: 48_000,
                channels: 4,
                sample_format: "pcm_s16le".into(),
                channel_layout: "n_channel".into(),
            }),
        }
    }

    #[test]
    fn microphone_entry_serializes_to_canonical_bytes() {
        let bytes = m1_microphone_entry().canonical_bytes();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"channel_layout":"n_channel","channels":4,"sample_format":"pcm_s16le","sample_rate_hz":48000,"sensor_id":"K1-AABBCCDDEEFF/head_array_4mic","type":"microphone"}"#
        );
    }

    #[test]
    fn microphone_entry_hash_is_locked() {
        // Pin the XXH3-128 of the M1 example microphone entry.
        // Updates to this must be coordinated with any cross-language reader.
        assert_eq!(
            m1_microphone_entry().hash(),
            "6e0a195364866f18834d2db8e2a0699f"
        );
    }

    #[test]
    fn write_then_read_microphone_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_microphone_entry();
        let outcome = write_sensor(dir.path(), &entry).unwrap();
        let hash = outcome.hash().to_string();
        let read = read_sensor(dir.path(), &entry.sensor_id, &hash).unwrap();
        assert_eq!(read, Some(entry));
    }

    // Sensor Log manifest builder + locked PoseSource vectors moved to
    // [`auki-manifests`] in Step 0 of the auki-datatypes migration. The Pose
    // Log payload round-trip tests below stay here until Step 5 of the
    // migration moves the payload types themselves.

    // ─── Pose Log ───────────────────────────────────────────────────────────

    #[test]
    fn pose_log_entry_round_trips_through_cbor() {
        let entry = PoseLogEntry {
            transforms: vec![
                TransformSample {
                    parent_frame: "base_link".into(),
                    child_frame: "head_link".into(),
                    translation: [0.0, 0.0, 1.5],
                    rotation_quat: [0.0, 0.0, 0.0, 1.0],
                },
                TransformSample {
                    parent_frame: "head_link".into(),
                    child_frame: "head_left_cam".into(),
                    translation: [0.05, 0.02, 0.0],
                    rotation_quat: [0.5, -0.5, 0.5, -0.5],
                },
            ],
        };
        let mut buf = Vec::new();
        ciborium::into_writer(&entry, &mut buf).unwrap();
        let back: PoseLogEntry = ciborium::from_reader(&buf[..]).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn pose_log_entry_with_empty_transforms_round_trips() {
        let entry = PoseLogEntry {
            transforms: Vec::new(),
        };
        let mut buf = Vec::new();
        ciborium::into_writer(&entry, &mut buf).unwrap();
        let back: PoseLogEntry = ciborium::from_reader(&buf[..]).unwrap();
        assert_eq!(back, entry);
    }

    // ─── Frame Registry tests ──────────────────────────────────────────────

    fn m1_frame_entry() -> FrameRegistryEntry {
        FrameRegistryEntry::ros_body("K1-AABBCCDDEEFF/base_link")
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
            r#"{"axes":{"x":"forward","y":"left","z":"up"},"frame_id":"K1-AABBCCDDEEFF/base_link","handedness":"right","units":"meters"}"#,
        );
    }

    /// Locks the XXH3-128 hex of the locked Frame Registry vector.
    /// Trips if any of `auki-jcs`, `auki-hash`, or this crate's serde
    /// shape drifts.
    #[test]
    fn frame_entry_hash_is_locked() {
        assert_eq!(
            m1_frame_entry().hash(),
            "fd0dc3789e898b71b5e16ee122a81a44"
        );
    }

    #[test]
    fn ros_body_preset_matches_explicit_construction() {
        let preset = FrameRegistryEntry::ros_body("frame/x");
        let explicit = FrameRegistryEntry {
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
        let preset = FrameRegistryEntry::ros_optical("frame/x");
        let explicit = FrameRegistryEntry {
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
        let preset = FrameRegistryEntry::opengl("frame/x");
        let explicit = FrameRegistryEntry {
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
        let preset = FrameRegistryEntry::unity("frame/x");
        let explicit = FrameRegistryEntry {
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
        FrameRegistryEntry::ros_body("a").validate().unwrap();
        FrameRegistryEntry::ros_optical("b").validate().unwrap();
        FrameRegistryEntry::opengl("c").validate().unwrap();
        FrameRegistryEntry::unity("d").validate().unwrap();
    }

    #[test]
    fn write_frame_rejects_non_orthogonal_axes_without_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let entry = FrameRegistryEntry {
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
        assert!(!frames_root.exists(), "no on-disk write on validation failure");
    }

    #[test]
    fn write_then_read_frame_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let entry = m1_frame_entry();
        let outcome = write_frame(dir.path(), &entry).unwrap();
        let hash = outcome.hash().to_string();
        let read = read_frame(dir.path(), &entry.frame_id, &hash).unwrap();
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
        let entry = read_frame(dir.path(), "frame/missing", "deadbeef").unwrap();
        assert_eq!(entry, None);
    }

    // `pose_log_manifest_opens_a_log_round_trip` moved to [`auki-manifests`]
    // in Step 0 of the auki-datatypes migration — `build_pose_log_manifest`
    // and `PoseSource` live there now. The PoseLogEntry CBOR round-trip
    // tests above cover this crate's payload-encoding contract.
}
