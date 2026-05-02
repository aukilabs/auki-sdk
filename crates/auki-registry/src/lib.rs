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
    /// On read, the deserialized entry's `sensor_id` / `clock_id` did not
    /// match the id in the requested path. Indicates a misplaced or tampered
    /// file — content addressing is meant to make this detectable.
    IdMismatch { expected: String, found: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Json(s) => write!(f, "json: {s}"),
            Error::IdMismatch { expected, found } => {
                write!(f, "id mismatch: expected {expected:?}, found {found:?}")
            }
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
            r#"{"color_space":"BT.709","distortion_model":"plumb_bob","frame_rate_hz":20,"height":488,"intrinsics_model":"pinhole","pixel_format":"YUV_NV12","sensor_id":"K1-AABBCCDDEEFF/head_left_cam","type":"rgb_camera","width":544}"#
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
    /// entry shape, canonicalization, or hashing.
    #[test]
    fn sensor_entry_hash_is_locked() {
        assert_eq!(
            m1_sensor_entry().hash(),
            "e8cb3879fcfa7f716047aa0892b0c0c0"
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
            }),
        }
    }

    #[test]
    fn point_cloud_entry_serializes_to_canonical_bytes() {
        let bytes = m1_point_cloud_entry().canonical_bytes();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"fields":[{"count":1,"datatype":"float32","name":"x","offset":0},{"count":1,"datatype":"float32","name":"y","offset":4},{"count":1,"datatype":"float32","name":"z","offset":8}],"frame_rate_hz":10,"is_bigendian":false,"point_step":12,"sensor_id":"K1-AABBCCDDEEFF/head_depth_points","type":"point_cloud"}"#
        );
    }

    #[test]
    fn point_cloud_entry_hash_is_locked() {
        // Pin the XXH3-128 of the M1 example point cloud entry.
        // Updates to this must be coordinated with any cross-language reader.
        assert_eq!(
            m1_point_cloud_entry().hash(),
            "35b318eb6b0a70cb2202083dcd1f14a2"
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
}
