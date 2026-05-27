//! Single source of truth for the Auki SDK's log manifests — schemas
//! + builders for Sensor Log, Pose Log, and TimeTransform Log
//! manifests.
//!
//! Symmetric with [`auki-datatypes`](../../auki-datatypes), which
//! owns segment payload shapes. This crate owns manifest shapes —
//! the per-recording metadata that lives at the root of each
//! `auki-logs` log directory.
//!
//! Manifests are encoded as **JCS-canonical UTF-8 JSON** via
//! [`auki-jcs`](../../auki-jcs). Decision pinned 2026-05-07: per-
//! recording metadata doesn't benefit from protobuf's wire compactness,
//! and JCS gives free cross-language byte-equivalence (handy for
//! signing + content-addressed-hashing the inline producer identities
//! like [`PoseSource`]).
//!
//! ## Surface
//!
//! - [`build_sensor_log_manifest`] — Sensor Log family (covers Sensor,
//!   Point Cloud, Audio Logs; `sensor` is a [`RegistryRef`] that resolves
//!   to a `SensorRegistryEntry` whose `body` variant tells a reader which
//!   payload type the segments hold).
//! - [`build_pose_log_manifest`] — Pose Log; `source` describes the
//!   producer inline.
//! - [`build_time_transform_log_manifest`] — TimeTransform Log;
//!   four-clock-binding fields.
//! - [`build_detection_log_manifest`] — Detection Log; carries
//!   `(detector_id, detector_hash)` producer identity and copies
//!   `(input_sensor_id, input_sensor_hash)` from the input log so the
//!   detection log is self-contained.
//! - [`PoseSource`] — tagged-enum producer identity, lives inline in
//!   the Pose Log manifest under `"source"`. Carries
//!   [`PoseSource::canonical_bytes`] / [`PoseSource::hash`] for
//!   content-addressing if a future producer variant graduates to a
//!   sibling registry.

use std::time::Duration;

use auki_registry::RegistryRef;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorLogManifest {
    pub source_peer_id: String,
    pub writer_peer_id: String,
    pub app_id: String,
    pub session_id: String,
    pub sensor: RegistryRef,
    pub clock: RegistryRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<RegistryRef>,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}

impl SensorLogManifest {
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        validate_non_empty("source_peer_id", &self.source_peer_id)?;
        validate_non_empty("writer_peer_id", &self.writer_peer_id)?;
        validate_non_empty("app_id", &self.app_id)?;
        validate_non_empty("session_id", &self.session_id)?;
        validate_non_empty("sensor.peer_id", &self.sensor.peer_id)?;
        validate_non_empty("sensor.id", &self.sensor.id)?;
        validate_non_empty("sensor.hash", &self.sensor.hash)?;
        validate_non_empty("clock.peer_id", &self.clock.peer_id)?;
        validate_non_empty("clock.id", &self.clock.id)?;
        validate_non_empty("clock.hash", &self.clock.hash)?;
        if let Some(ref f) = self.frame {
            validate_non_empty("frame.peer_id", &f.peer_id)?;
            validate_non_empty("frame.id", &f.id)?;
            validate_non_empty("frame.hash", &f.hash)?;
        }
        validate_durations(self.segment_duration_ns, self.retention_ns)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoseLogManifest {
    pub source_peer_id: String,
    pub writer_peer_id: String,
    pub app_id: String,
    pub session_id: String,
    pub from_frame: RegistryRef,
    pub to_frame: RegistryRef,
    pub clock: RegistryRef,
    pub source: PoseSource,
    pub writer_mode: PoseWriterMode,
    pub expected_rate_hz: u32,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}

impl PoseLogManifest {
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        validate_non_empty("source_peer_id", &self.source_peer_id)?;
        validate_non_empty("writer_peer_id", &self.writer_peer_id)?;
        validate_non_empty("app_id", &self.app_id)?;
        validate_non_empty("session_id", &self.session_id)?;
        validate_non_empty("from_frame.peer_id", &self.from_frame.peer_id)?;
        validate_non_empty("from_frame.id", &self.from_frame.id)?;
        validate_non_empty("from_frame.hash", &self.from_frame.hash)?;
        validate_non_empty("to_frame.peer_id", &self.to_frame.peer_id)?;
        validate_non_empty("to_frame.id", &self.to_frame.id)?;
        validate_non_empty("to_frame.hash", &self.to_frame.hash)?;
        validate_non_empty("clock.peer_id", &self.clock.peer_id)?;
        validate_non_empty("clock.id", &self.clock.id)?;
        validate_non_empty("clock.hash", &self.clock.hash)?;
        validate_durations(self.segment_duration_ns, self.retention_ns)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeTransformLogManifest {
    pub app_id: String,
    pub session_id: String,
    pub from_clock_id: String,
    pub from_clock_hash: String,
    pub to_clock_id: String,
    pub to_clock_hash: String,
    pub source: TimeTransformSource,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}

impl TimeTransformLogManifest {
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        validate_non_empty("app_id", &self.app_id)?;
        validate_non_empty("session_id", &self.session_id)?;
        validate_non_empty("from_clock_id", &self.from_clock_id)?;
        validate_non_empty("from_clock_hash", &self.from_clock_hash)?;
        validate_non_empty("to_clock_id", &self.to_clock_id)?;
        validate_non_empty("to_clock_hash", &self.to_clock_hash)?;
        validate_durations(self.segment_duration_ns, self.retention_ns)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectionLogManifest {
    pub app_id: String,
    pub session_id: String,
    pub detector_id: String,
    pub detector_hash: String,
    pub input_log_id: String,
    pub input_sensor_id: String,
    pub input_sensor_hash: String,
    pub clock_id: String,
    pub clock_hash: String,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}

impl DetectionLogManifest {
    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        validate_non_empty("app_id", &self.app_id)?;
        validate_non_empty("session_id", &self.session_id)?;
        validate_non_empty("detector_id", &self.detector_id)?;
        validate_non_empty("detector_hash", &self.detector_hash)?;
        validate_non_empty("input_log_id", &self.input_log_id)?;
        validate_non_empty("input_sensor_id", &self.input_sensor_id)?;
        validate_non_empty("input_sensor_hash", &self.input_sensor_hash)?;
        validate_non_empty("clock_id", &self.clock_id)?;
        validate_non_empty("clock_hash", &self.clock_hash)?;
        validate_durations(self.segment_duration_ns, self.retention_ns)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestValidationError {
    pub field: &'static str,
    pub reason: &'static str,
}

impl std::fmt::Display for ManifestValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid manifest field `{}`: {}",
            self.field, self.reason
        )
    }
}

impl std::error::Error for ManifestValidationError {}

/// Build a Sensor Log family manifest with `source_peer_id` / `writer_peer_id`,
/// the run-identifying `app_id` / `session_id`, the sensor/clock/frame
/// [`RegistryRef`] bindings, and auki-logs's required
/// `segment_duration_ns` / `retention_ns`.
///
/// Same shape for Sensor Log, Point Cloud Log, and Audio Log — the `sensor`
/// [`RegistryRef`] resolves to a `SensorRegistryEntry` whose `body` variant
/// tells a reader which payload type the segments hold.
///
/// `source_peer_id` is the peer that produced the sensor data.
/// `writer_peer_id` is the peer that wrote this manifest file (may differ
/// when a remote peer materializes the log, e.g. Park re-materializing
/// Galbot's sensor log).
/// `app_id` is the application identifier (same string as the daemon's
/// `/api/info` `app` field; e.g. `"boosterapp"`, `"sentinel"`).
/// `session_id` is the integrator-minted UUIDv4 for the current daemon run
/// (same value as the parent session directory name).
pub fn build_sensor_log_manifest(
    source_peer_id: &str,
    writer_peer_id: &str,
    app_id: &str,
    session_id: &str,
    sensor: RegistryRef,
    clock: RegistryRef,
    frame: Option<RegistryRef>,
    segment_duration: Duration,
    retention: Duration,
) -> serde_json::Value {
    serde_json::to_value(SensorLogManifest {
        source_peer_id: source_peer_id.to_string(),
        writer_peer_id: writer_peer_id.to_string(),
        app_id: app_id.to_string(),
        session_id: session_id.to_string(),
        sensor,
        clock,
        frame,
        segment_duration_ns: duration_as_i64_ns(segment_duration),
        retention_ns: duration_as_i64_ns(retention),
    })
    .expect("manifest serialization")
}

/// Build a Pose Log manifest for the new `(from, to)`-keyed Pose Log
/// shape (Step 5 of the [`auki-datatypes` migration],
/// 2026-05-08). One Pose Log holds samples for exactly one
/// `(from_frame, to_frame)` pair; segment entries are flat
/// `auki_datatypes::pose::SpatialTransform`. A producer that observes a
/// multi-pair ROS `TFMessage` is responsible for fanning the message
/// into N parallel pose logs.
///
/// Carries:
/// - `source_peer_id` — the peer that produced the pose data.
/// - `writer_peer_id` — the peer that wrote this manifest file (may
///   differ when a remote peer materializes the log).
/// - `app_id` / `session_id` — run identity (same shape as siblings).
/// - `from_frame` / `to_frame` — [`RegistryRef`] bindings to the two
///   `FrameRegistryEntry` records, each carrying `(peer_id, id, hash)`.
///   Mirrors how `build_sensor_log_manifest` pins `sensor` / `clock`
///   via `RegistryRef`.
/// - `clock` — [`RegistryRef`] binding to the clock the framing's
///   `timestamp_ns` is on.
/// - `source` — inline [`PoseSource`] tagged-enum producer identity.
/// - `writer_mode` — `"rigid"` (transform doesn't change between
///   samples; the log captures one observation that reads back at any
///   query time) or `"movable"` (transform varies over time; readers
///   interpolate or step-look-up). Per the synthesis decided
///   2026-05-07.
/// - `expected_rate_hz` — the producer's nominal sample rate. Hint for
///   readers (e.g. for choosing interpolation step size); not enforced
///   by the SDK.
/// - `segment_duration_ns` / `retention_ns` — auki-logs framing.
///
/// `source_peer_id` is the peer that produced the pose data.
/// `writer_peer_id` is the peer that wrote this manifest file (may
/// differ when a remote peer materializes the log, e.g. Park
/// re-materializing Galbot's pose log).
/// `app_id` is the application identifier (same string as the daemon's
/// `/api/info` `app` field; e.g. `"boosterapp"`, `"sentinel"`).
/// `session_id` is the integrator-minted UUIDv4 for the current daemon
/// run (same value as the parent session directory name).
#[allow(clippy::too_many_arguments)]
pub fn build_pose_log_manifest(
    source_peer_id: &str,
    writer_peer_id: &str,
    app_id: &str,
    session_id: &str,
    from_frame: RegistryRef,
    to_frame: RegistryRef,
    clock: RegistryRef,
    source: &PoseSource,
    writer_mode: PoseWriterMode,
    expected_rate_hz: u32,
    segment_duration: Duration,
    retention: Duration,
) -> serde_json::Value {
    serde_json::to_value(PoseLogManifest {
        source_peer_id: source_peer_id.into(),
        writer_peer_id: writer_peer_id.into(),
        app_id: app_id.into(),
        session_id: session_id.into(),
        from_frame,
        to_frame,
        clock,
        source: source.clone(),
        writer_mode,
        expected_rate_hz,
        segment_duration_ns: duration_as_i64_ns(segment_duration),
        retention_ns: duration_as_i64_ns(retention),
    })
    .expect("PoseLogManifest serializes")
}

/// Build a Detection Log manifest. One Detection Log per
/// `(detector, input sensor log)` pair within a session.
///
/// Closes blocker #2 of [`detectors`](https://github.com/aukilabs/detectors)
/// phase 2 — the read side ([`auki-logs::Log<T>::tail`](../../auki-logs))
/// and the segment payload type ([`auki_datatypes::detection::DetectionFrame`](../../auki-datatypes))
/// landed in sibling PRs. The detector loop the integrator writes is
/// `for entry in tail(input_path)? { detector.process(...); output.append(...); }`,
/// where `output` is the `Log<DetectionFrame>` opened with this
/// manifest at [`auki_layout::detection_log_path`](../../auki-layout).
///
/// Carries:
/// - `app_id` / `session_id` — run identity (same shape as siblings).
/// - `detector_id` + `detector_hash` — content-addressed producer
///   identity. Mirrors `(sensor_id, sensor_hash)` for sensors. The
///   `detector_id` is namespaced and human-readable (`"aukilabs/qr/v1"`,
///   `"aukilabs/esl/v1"`); the `detector_hash` content-binds the
///   producer (e.g. `hash(commit-SHA + config)` for code-only
///   detectors, `hash(commit-SHA + weights + config)` for ML
///   detectors). The `DetectorRegistryEntry` shape that pins exactly
///   what's hashed is **deferred** to a sibling PR — for v1 the
///   manifest carries both as opaque strings and the SDK doesn't
///   validate them.
/// - `input_log_id` — the `sensor_log_id` of the input log being
///   tailed (the directory name under `sensorlogs/`); pins WHICH
///   instance of the sensor produced the inputs. Mirrors the
///   `(detector_id, input_log_id)` dedup-identity lean from
///   [the keystone's detection-log lifecycle entry](../../parking_lot.md).
/// - `input_sensor_id` + `input_sensor_hash` — copied from the input
///   log's manifest so the detection log is self-contained: a reader
///   that holds only the detection log can still know what sensor
///   produced its inputs, even after the sensor log is evicted by
///   retention.
/// - `clock_id` + `clock_hash` — the clock the framing's
///   `timestamp_ns` is on. Same clock as the input log (entries
///   are timestamp-aligned with the frame they were derived from).
/// - `segment_duration_ns` / `retention_ns` — auki-logs framing.
///
/// `app_id` is the application identifier (same string as the daemon's
/// `/api/info` `app` field). `session_id` is the integrator-minted
/// UUIDv4 for the current daemon run.
///
/// **No `intent` field.** Per PR #72 the keystone's `buffer | intent_recording`
/// dimension applies to every log, but adding it uniformly across the
/// existing manifest builders is a separate PR — match the existing
/// log behavior here, file the uniform update as a follow-up.
#[allow(clippy::too_many_arguments)]
pub fn build_detection_log_manifest(
    app_id: &str,
    session_id: &str,
    detector_id: &str,
    detector_hash: &str,
    input_log_id: &str,
    input_sensor_id: &str,
    input_sensor_hash: &str,
    clock_id: &str,
    clock_hash: &str,
    segment_duration: Duration,
    retention: Duration,
) -> serde_json::Value {
    serde_json::to_value(DetectionLogManifest {
        app_id: app_id.into(),
        session_id: session_id.into(),
        detector_id: detector_id.into(),
        detector_hash: detector_hash.into(),
        input_log_id: input_log_id.into(),
        input_sensor_id: input_sensor_id.into(),
        input_sensor_hash: input_sensor_hash.into(),
        clock_id: clock_id.into(),
        clock_hash: clock_hash.into(),
        segment_duration_ns: duration_as_i64_ns(segment_duration),
        retention_ns: duration_as_i64_ns(retention),
    })
    .expect("DetectionLogManifest serializes")
}

/// Build a TimeTransform Log manifest with the four required clock-binding
/// fields, the run-identifying `app_id` / `session_id`, the inline
/// producer identity, and auki-logs's required `segment_duration_ns` /
/// `retention_ns`.
///
/// Step 6 of the [`auki-datatypes` migration] (2026-05-08) added the
/// `source: &TimeTransformSource` argument: per-sample `source` on
/// `TimeTransformEntry` moved to per-log `source` on the manifest,
/// matching how Pose Log carries `PoseSource` inline.
///
/// `app_id` is the application identifier (same string as the daemon's
/// `/api/info` `app` field; e.g. `"boosterapp"`, `"sentinel"`).
/// `session_id` is the integrator-minted UUIDv4 for the current daemon
/// run (same value as the parent session directory name).
#[allow(clippy::too_many_arguments)]
pub fn build_time_transform_log_manifest(
    app_id: &str,
    session_id: &str,
    from_clock_id: &str,
    from_clock_hash: &str,
    to_clock_id: &str,
    to_clock_hash: &str,
    source: &TimeTransformSource,
    segment_duration: Duration,
    retention: Duration,
) -> serde_json::Value {
    serde_json::to_value(TimeTransformLogManifest {
        app_id: app_id.into(),
        session_id: session_id.into(),
        from_clock_id: from_clock_id.into(),
        from_clock_hash: from_clock_hash.into(),
        to_clock_id: to_clock_id.into(),
        to_clock_hash: to_clock_hash.into(),
        source: source.clone(),
        segment_duration_ns: duration_as_i64_ns(segment_duration),
        retention_ns: duration_as_i64_ns(retention),
    })
    .expect("TimeTransformLogManifest serializes")
}

/// Identifies the producer of the offsets in a TimeTransform Log.
/// Lives **inline** in the log's manifest under the `"source"` key —
/// TimeTransform Log has no separate registry because the segment
/// payload is fully self-describing (`offset_ns` + `uncertainty_ns`);
/// source identity is provenance, not a decoder. Tagged-enum body
/// mirrors [`PoseSource`]'s shape for future producer variants.
///
/// Step 6 of the [`auki-datatypes` migration] (2026-05-08) moved this
/// type from its pre-migration home in [`auki-time`](../../auki-time)
/// (where it was a per-sample field on `TimeTransformEntry`) to here.
/// One variant ships today (`LocalClockRead` — the 1 Hz sampler in
/// [`auki-time`](../../auki-time) reading
/// `CLOCK_MONOTONIC` and `CLOCK_REALTIME` via `clock_gettime`); the
/// extension point is the same as `PoseSource`'s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimeTransformSource {
    /// The 1 Hz `clock_gettime`-based sampler in
    /// [`auki-time`](../../auki-time). The only
    /// producer that ships today.
    LocalClockRead,
    // future: NtpSynced { server }, SyncedTo { peer_id }, ...
}

impl TimeTransformSource {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let v = serde_json::to_value(self).expect("TimeTransformSource serializes to a JSON value");
        auki_jcs::canonicalize(&v)
    }

    pub fn hash(&self) -> String {
        auki_hash::hash_jcs_bytes(&self.canonical_bytes())
    }
}

/// Writer-mode hint for a Pose Log — one of `"rigid"` (transform is
/// stationary; the log captures a single observation that reads back
/// at any query time) or `"movable"` (transform varies over time;
/// readers interpolate or step-look-up). Per the synthesis decided
/// 2026-05-07; lives in the manifest, not on segment entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoseWriterMode {
    /// Stationary `(from, to)` transform — e.g. a static camera mount,
    /// a calibrated robot link. Producers SHOULD still write samples
    /// at the manifest's `expected_rate_hz` to give downstream a
    /// liveness signal; readers MAY treat any sample as authoritative
    /// for the whole log lifetime.
    Rigid,
    /// Time-varying `(from, to)` transform — e.g. SLAM odometry,
    /// articulated joints. Readers interpolate or step-look-up at a
    /// query timestamp.
    Movable,
}

/// Identifies the producer of the transforms in a Pose Log. Lives **inline**
/// in the log's manifest under the `source` key — Pose Log does not have a
/// separate registry like Sensor Log, because provenance is the only thing
/// `source` describes. Tagged-enum body is the extension point for future
/// producers (SLAM, odometry, manual fixtures, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PoseSource {
    /// ROS 2 `/tf` (and `/tf_static`, merged on capture). `publishers` is the
    /// sorted list of ROS node names contributing to the topic. Frame pairs
    /// are *not* part of identity — they can change at runtime; consult the
    /// segments for what was actually observed.
    Ros2Tf {
        /// Sorted; ROS node names contributing to `/tf`.
        publishers: Vec<String>,
    },
    /// Manually specified or calibration-fixture transform. No automated
    /// producer; the transform was hand-entered or produced by an
    /// offline calibration tool and written directly into the log.
    Manual,
    // future: Slam { algorithm, map_id, ... }, Odometry { ... }, ...
}

impl PoseSource {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        // PoseSource is a plain tagged-enum of strings; serializing to a Value
        // cannot fail in practice.
        let v = serde_json::to_value(self).expect("PoseSource serializes to a JSON value");
        auki_jcs::canonicalize(&v)
    }

    pub fn hash(&self) -> String {
        auki_hash::hash_jcs_bytes(&self.canonical_bytes())
    }
}

fn duration_as_i64_ns(d: Duration) -> i64 {
    d.as_nanos().min(i64::MAX as u128) as i64
}

fn validate_non_empty(field: &'static str, value: &str) -> Result<(), ManifestValidationError> {
    if value.is_empty() {
        Err(ManifestValidationError {
            field,
            reason: "must not be empty",
        })
    } else {
        Ok(())
    }
}

fn validate_optional_pair(
    left_field: &'static str,
    left: Option<&str>,
    right_field: &'static str,
    right: Option<&str>,
) -> Result<(), ManifestValidationError> {
    match (left, right) {
        (None, None) => Ok(()),
        (Some(left), Some(right)) => {
            validate_non_empty(left_field, left)?;
            validate_non_empty(right_field, right)
        }
        (None, Some(_)) => Err(ManifestValidationError {
            field: left_field,
            reason: "must be set when paired field is set",
        }),
        (Some(_), None) => Err(ManifestValidationError {
            field: right_field,
            reason: "must be set when paired field is set",
        }),
    }
}

fn validate_durations(
    segment_duration_ns: i64,
    retention_ns: i64,
) -> Result<(), ManifestValidationError> {
    if segment_duration_ns <= 0 {
        return Err(ManifestValidationError {
            field: "segment_duration_ns",
            reason: "must be greater than zero",
        });
    }
    if retention_ns < 0 {
        return Err(ManifestValidationError {
            field: "retention_ns",
            reason: "must be greater than or equal to zero",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// Trivial body type for the auki-logs round-trip tests — manifests are
    /// independent of the payload `T`, so a placeholder struct is enough.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestEntry {
        value: i64,
    }

    impl auki_logs::LogPayload for TestEntry {
        fn encode(&self) -> Vec<u8> {
            let mut buf = Vec::new();
            ciborium::into_writer(self, &mut buf).expect("ciborium encode of TestEntry");
            buf
        }
        fn decode(bytes: &[u8]) -> std::result::Result<Self, String> {
            ciborium::from_reader(bytes).map_err(|e| e.to_string())
        }
    }

    // ─── Sensor Log manifest ────────────────────────────────────────────────

    fn m1_sensor_ref() -> RegistryRef {
        RegistryRef {
            peer_id: "galbot".into(),
            id: "K1-AABBCCDDEEFF/head_left_cam".into(),
            hash: "e8cb3879fcfa7f716047aa0892b0c0c0".into(),
        }
    }

    fn m1_clock_ref() -> RegistryRef {
        RegistryRef {
            peer_id: "galbot".into(),
            id: "K1-AABBCCDDEEFF/utc".into(),
            hash: "89f84f4c2e09bef81d385b2af1d17e6c".into(),
        }
    }

    fn m1_frame_ref() -> RegistryRef {
        RegistryRef {
            peer_id: "galbot".into(),
            id: "K1-AABBCCDDEEFF/head_left_cam_optical".into(),
            hash: "fd0dc3789e898b71b5e16ee122a81a44".into(),
        }
    }

    #[test]
    fn build_sensor_log_manifest_contains_all_required_fields() {
        let m = build_sensor_log_manifest(
            "galbot",
            "galbot",
            "boosterapp",
            "550e8400-e29b-41d4-a716-446655440000",
            m1_sensor_ref(),
            m1_clock_ref(),
            Some(m1_frame_ref()),
            Duration::from_secs(1),
            Duration::from_secs(30),
        );
        assert_eq!(m["source_peer_id"], "galbot");
        assert_eq!(m["writer_peer_id"], "galbot");
        assert_eq!(m["app_id"], "boosterapp");
        assert_eq!(m["session_id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(m["sensor"]["peer_id"], "galbot");
        assert_eq!(m["sensor"]["id"], "K1-AABBCCDDEEFF/head_left_cam");
        assert_eq!(m["sensor"]["hash"], "e8cb3879fcfa7f716047aa0892b0c0c0");
        assert_eq!(m["clock"]["peer_id"], "galbot");
        assert_eq!(m["clock"]["id"], "K1-AABBCCDDEEFF/utc");
        assert_eq!(m["clock"]["hash"], "89f84f4c2e09bef81d385b2af1d17e6c");
        assert_eq!(m["frame"]["peer_id"], "galbot");
        assert_eq!(m["frame"]["id"], "K1-AABBCCDDEEFF/head_left_cam_optical");
        assert_eq!(m["frame"]["hash"], "fd0dc3789e898b71b5e16ee122a81a44");
        assert_eq!(m["segment_duration_ns"], 1_000_000_000i64);
        assert_eq!(m["retention_ns"], 30_000_000_000i64);
    }

    #[test]
    fn sensor_log_manifest_opens_a_log_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = build_sensor_log_manifest(
            "galbot",
            "galbot",
            "boosterapp",
            "550e8400-e29b-41d4-a716-446655440000",
            m1_sensor_ref(),
            m1_clock_ref(),
            None,
            Duration::from_secs(1),
            Duration::from_secs(30),
        );
        let _log = auki_logs::Log::<TestEntry>::open(dir.path(), manifest).unwrap();
        let reader = auki_logs::Log::<TestEntry>::read(dir.path()).unwrap();
        assert_eq!(reader.manifest()["app_id"], "boosterapp");
        assert_eq!(
            reader.manifest()["session_id"],
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn sensor_log_manifest_deserializes_and_validates() {
        let m = build_sensor_log_manifest(
            "galbot",
            "galbot",
            "boosterapp",
            "550e8400-e29b-41d4-a716-446655440000",
            m1_sensor_ref(),
            m1_clock_ref(),
            Some(m1_frame_ref()),
            Duration::from_secs(1),
            Duration::from_secs(30),
        );
        let typed: SensorLogManifest = serde_json::from_value(m).unwrap();
        typed.validate().unwrap();
        assert_eq!(
            typed.frame.as_ref().map(|f| f.id.as_str()),
            Some("K1-AABBCCDDEEFF/head_left_cam_optical")
        );
    }

    #[test]
    fn sensor_log_manifest_rejects_empty_sensor_id() {
        let typed = SensorLogManifest {
            source_peer_id: "galbot".into(),
            writer_peer_id: "galbot".into(),
            app_id: "boosterapp".into(),
            session_id: "session".into(),
            sensor: RegistryRef {
                peer_id: "galbot".into(),
                id: "".into(), // empty — should fail
                hash: "sensor_hash".into(),
            },
            clock: RegistryRef {
                peer_id: "galbot".into(),
                id: "clock".into(),
                hash: "clock_hash".into(),
            },
            frame: None,
            segment_duration_ns: 1,
            retention_ns: 0,
        };
        let err = typed.validate().unwrap_err();
        assert_eq!(err.field, "sensor.id");
    }

    // ─── Pose Log + PoseSource ──────────────────────────────────────────────

    fn m1_ros2_tf_source() -> PoseSource {
        PoseSource::Ros2Tf {
            publishers: vec![
                "amcl".into(),
                "robot_state_publisher".into(),
                "tf_broadcaster".into(),
            ],
        }
    }

    /// Locks the JCS canonical bytes for the M1 example ROS 2 TF source.
    /// Catches drift in the tagged-enum shape OR canonicalization.
    #[test]
    fn ros2_tf_source_serializes_to_canonical_bytes() {
        let bytes = m1_ros2_tf_source().canonical_bytes();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"kind":"ros2_tf","publishers":["amcl","robot_state_publisher","tf_broadcaster"]}"#
        );
    }

    /// Locks the XXH3-128 hex of the M1 example ROS 2 TF source. Cross-cutting
    /// guard: trips if any of `auki-jcs`, `auki-hash`, or this crate's serde
    /// shape drifts.
    #[test]
    fn ros2_tf_source_hash_is_locked() {
        assert_eq!(
            m1_ros2_tf_source().hash(),
            "f3d296341347589c72297a0cc7c81cd8"
        );
    }

    #[test]
    fn build_pose_log_manifest_contains_all_required_fields() {
        let m = build_pose_log_manifest(
            "galbot",
            "galbot",
            "boosterapp",
            "550e8400-e29b-41d4-a716-446655440000",
            RegistryRef {
                peer_id: "galbot".into(),
                id: "K1-AABBCCDDEEFF/base_link".into(),
                hash: "fd0dc3789e898b71b5e16ee122a81a44".into(),
            },
            RegistryRef {
                peer_id: "galbot".into(),
                id: "K1-AABBCCDDEEFF/head_left_cam_optical".into(),
                hash: "11223344556677889900aabbccddeeff".into(),
            },
            RegistryRef {
                peer_id: "galbot".into(),
                id: "K1-AABBCCDDEEFF/utc".into(),
                hash: "89f84f4c2e09bef81d385b2af1d17e6c".into(),
            },
            &m1_ros2_tf_source(),
            PoseWriterMode::Movable,
            100,
            Duration::from_secs(1),
            Duration::from_secs(30),
        );
        assert_eq!(m["source_peer_id"], "galbot");
        assert_eq!(m["writer_peer_id"], "galbot");
        assert_eq!(m["app_id"], "boosterapp");
        assert_eq!(m["session_id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(m["from_frame"]["peer_id"], "galbot");
        assert_eq!(m["from_frame"]["id"], "K1-AABBCCDDEEFF/base_link");
        assert_eq!(m["from_frame"]["hash"], "fd0dc3789e898b71b5e16ee122a81a44");
        assert_eq!(m["to_frame"]["peer_id"], "galbot");
        assert_eq!(m["to_frame"]["id"], "K1-AABBCCDDEEFF/head_left_cam_optical");
        assert_eq!(m["to_frame"]["hash"], "11223344556677889900aabbccddeeff");
        assert_eq!(m["clock"]["peer_id"], "galbot");
        assert_eq!(m["clock"]["id"], "K1-AABBCCDDEEFF/utc");
        assert_eq!(m["clock"]["hash"], "89f84f4c2e09bef81d385b2af1d17e6c");
        assert_eq!(m["source"]["kind"], "ros2_tf");
        assert_eq!(m["source"]["publishers"][0], "amcl");
        assert_eq!(m["writer_mode"], "movable");
        assert_eq!(m["expected_rate_hz"], 100);
        assert_eq!(m["segment_duration_ns"], 1_000_000_000i64);
        assert_eq!(m["retention_ns"], 30_000_000_000i64);
    }

    #[test]
    fn build_pose_log_manifest_serializes_writer_mode_as_snake_case() {
        let m = build_pose_log_manifest(
            "test",
            "test",
            "test-app",
            "s",
            RegistryRef {
                peer_id: "p".into(),
                id: "from".into(),
                hash: "fh".into(),
            },
            RegistryRef {
                peer_id: "p".into(),
                id: "to".into(),
                hash: "th".into(),
            },
            RegistryRef {
                peer_id: "p".into(),
                id: "c".into(),
                hash: "ch".into(),
            },
            &m1_ros2_tf_source(),
            PoseWriterMode::Rigid,
            0,
            Duration::from_secs(1),
            Duration::from_secs(30),
        );
        assert_eq!(m["writer_mode"], "rigid");
    }

    // ─── TimeTransform Log manifest ─────────────────────────────────────────

    #[test]
    fn build_time_transform_log_manifest_contains_required_fields() {
        let m = build_time_transform_log_manifest(
            "boosterapp",
            "550e8400-e29b-41d4-a716-446655440000",
            "K1-AABBCCDDEEFF/monotonic",
            "deadbeefcafefeed",
            "K1-AABBCCDDEEFF/utc",
            "1234567890abcdef",
            &TimeTransformSource::LocalClockRead,
            Duration::from_secs(1),
            Duration::from_secs(60),
        );
        assert_eq!(m["app_id"], "boosterapp");
        assert_eq!(m["session_id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(m["from_clock_id"], "K1-AABBCCDDEEFF/monotonic");
        assert_eq!(m["from_clock_hash"], "deadbeefcafefeed");
        assert_eq!(m["to_clock_id"], "K1-AABBCCDDEEFF/utc");
        assert_eq!(m["to_clock_hash"], "1234567890abcdef");
        assert_eq!(m["source"]["kind"], "local_clock_read");
        assert_eq!(m["segment_duration_ns"], 1_000_000_000i64);
        assert_eq!(m["retention_ns"], 60_000_000_000i64);
    }

    /// Locks the JCS canonical bytes for the only `TimeTransformSource`
    /// variant that ships today. Catches drift in tagged-enum serde
    /// shape OR canonicalization. Mirrors `ros2_tf_source_serializes_to_canonical_bytes`.
    #[test]
    fn local_clock_read_source_serializes_to_canonical_bytes() {
        let bytes = TimeTransformSource::LocalClockRead.canonical_bytes();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"kind":"local_clock_read"}"#
        );
    }

    /// Locked XXH3-128 hex of the canonical bytes above. Trips if any
    /// of `auki-jcs`, `auki-hash`, or this crate's serde shape drifts.
    #[test]
    fn local_clock_read_source_hash_is_locked() {
        assert_eq!(
            TimeTransformSource::LocalClockRead.hash(),
            "8dcea0b9b0b2219d651e0856f112cd65"
        );
    }

    // ─── Detection Log manifest ─────────────────────────────────────────────

    fn m1_detection_log_manifest() -> serde_json::Value {
        build_detection_log_manifest(
            "boosterapp",
            "550e8400-e29b-41d4-a716-446655440000",
            "aukilabs/qr/v1",
            "abc123def4567890abc123def4567890",
            "rec-456",
            "K1-AABBCCDDEEFF/head_left_cam",
            "e8cb3879fcfa7f716047aa0892b0c0c0",
            "K1-AABBCCDDEEFF/utc",
            "89f84f4c2e09bef81d385b2af1d17e6c",
            Duration::from_secs(1),
            Duration::from_secs(30),
        )
    }

    #[test]
    fn build_detection_log_manifest_contains_all_required_fields() {
        let m = m1_detection_log_manifest();
        assert_eq!(m["app_id"], "boosterapp");
        assert_eq!(m["session_id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(m["detector_id"], "aukilabs/qr/v1");
        assert_eq!(m["detector_hash"], "abc123def4567890abc123def4567890");
        assert_eq!(m["input_log_id"], "rec-456");
        assert_eq!(m["input_sensor_id"], "K1-AABBCCDDEEFF/head_left_cam");
        assert_eq!(m["input_sensor_hash"], "e8cb3879fcfa7f716047aa0892b0c0c0");
        assert_eq!(m["clock_id"], "K1-AABBCCDDEEFF/utc");
        assert_eq!(m["clock_hash"], "89f84f4c2e09bef81d385b2af1d17e6c");
        assert_eq!(m["segment_duration_ns"], 1_000_000_000i64);
        assert_eq!(m["retention_ns"], 30_000_000_000i64);
    }

    #[test]
    fn build_detection_log_manifest_omits_intent_field() {
        // Per the keystone, intent (`buffer | intent_recording`) applies
        // to every log — but the existing log builders don't carry it
        // yet, so this builder matches them. A follow-on PR adds intent
        // uniformly across every manifest builder. Asserting absence
        // here so the contract is explicit and the follow-on PR has a
        // failing test to update when it lands.
        let m = m1_detection_log_manifest();
        assert!(
            m.get("intent").is_none(),
            "intent field should be absent until uniform rollout"
        );
    }

    // ─── SensorLogManifest source/writer split (Task 2.1) ───────────────────

    #[test]
    fn sensor_log_manifest_origin_canonical_has_source_writer_peer_id() {
        let m = SensorLogManifest {
            source_peer_id: "galbot".to_string(),
            writer_peer_id: "galbot".to_string(),
            app_id: "galbot-control-plane".to_string(),
            session_id: "01HV-galbot-session".to_string(),
            sensor: RegistryRef {
                peer_id: "galbot".to_string(),
                id: "head_left_rgb".to_string(),
                hash: "sensorhash".to_string(),
            },
            clock: RegistryRef {
                peer_id: "galbot".to_string(),
                id: "session/sdk_clock".to_string(),
                hash: "clockhash".to_string(),
            },
            frame: Some(RegistryRef {
                peer_id: "galbot".to_string(),
                id: "head_left_camera_optical".to_string(),
                hash: "framehash".to_string(),
            }),
            segment_duration_ns: 1_000_000_000,
            retention_ns: 5_000_000_000,
        };
        let v = serde_json::to_value(&m).unwrap();
        let bytes = auki_jcs::canonicalize(&v);
        let json = std::str::from_utf8(&bytes).unwrap();
        assert!(json.contains(r#""source_peer_id":"galbot""#));
        assert!(json.contains(r#""writer_peer_id":"galbot""#));
        assert!(json.contains(r#""sensor":{"hash":"sensorhash","id":"head_left_rgb","peer_id":"galbot"}"#));
        assert!(json.contains(r#""segment_duration_ns":1000000000"#));
        assert!(json.contains(r#""retention_ns":5000000000"#));
    }

    #[test]
    fn sensor_log_manifest_materialized_keeps_source_changes_writer() {
        let m = SensorLogManifest {
            source_peer_id: "galbot".to_string(),
            writer_peer_id: "park".to_string(), // Park materialized Galbot's log
            app_id: "park-vis".to_string(),
            session_id: "01HV-park-session".to_string(),
            sensor: RegistryRef {
                peer_id: "galbot".to_string(), // Sensor still owned by Galbot
                id: "head_left_rgb".to_string(),
                hash: "sensorhash".to_string(),
            },
            clock: RegistryRef {
                peer_id: "galbot".to_string(),
                id: "session/sdk_clock".to_string(),
                hash: "clockhash".to_string(),
            },
            frame: None,
            segment_duration_ns: 10_000_000_000, // Park picked larger segments
            retention_ns: 300_000_000_000,       // Park keeps 5min
        };
        let v = serde_json::to_value(&m).unwrap();
        let bytes = auki_jcs::canonicalize(&v);
        let json = std::str::from_utf8(&bytes).unwrap();
        assert!(json.contains(r#""source_peer_id":"galbot""#));
        assert!(json.contains(r#""writer_peer_id":"park""#));
        assert!(json.contains(r#""app_id":"park-vis""#));
    }

    // ─── PoseLogManifest source/writer split + RegistryRef (Task 2.2) ──────

    #[test]
    fn pose_log_manifest_movable_canonical() {
        use auki_registry::RegistryRef;

        let m = PoseLogManifest {
            source_peer_id: "galbot".to_string(),
            writer_peer_id: "galbot".to_string(),
            app_id: "galbot-control-plane".to_string(),
            session_id: "01HV".to_string(),
            from_frame: RegistryRef {
                peer_id: "park".to_string(),
                id: "world".to_string(),
                hash: "fromhash".to_string(),
            },
            to_frame: RegistryRef {
                peer_id: "galbot".to_string(),
                id: "base_link".to_string(),
                hash: "tohash".to_string(),
            },
            clock: RegistryRef {
                peer_id: "galbot".to_string(),
                id: "session/sdk_clock".to_string(),
                hash: "clockhash".to_string(),
            },
            source: PoseSource::Manual,
            writer_mode: PoseWriterMode::Movable,
            expected_rate_hz: 30,
            segment_duration_ns: 1_000_000_000,
            retention_ns: 60_000_000_000,
        };
        let bytes = auki_jcs::canonicalize(&serde_json::to_value(&m).unwrap());
        let json = std::str::from_utf8(&bytes).unwrap();
        assert!(json.contains(r#""source_peer_id":"galbot""#));
        assert!(json.contains(r#""writer_mode":"movable""#));
        assert!(json.contains(r#""from_frame":{"hash":"fromhash","id":"world","peer_id":"park"}"#));
        assert!(json.contains(r#""to_frame":{"hash":"tohash","id":"base_link","peer_id":"galbot"}"#));
    }

    #[test]
    fn pose_log_manifest_rigid_one_sample_canonical() {
        use auki_registry::RegistryRef;

        let m = PoseLogManifest {
            source_peer_id: "galbot".to_string(),
            writer_peer_id: "galbot".to_string(),
            app_id: "galbot-control-plane".to_string(),
            session_id: "01HV".to_string(),
            from_frame: RegistryRef {
                peer_id: "park".to_string(),
                id: "world".to_string(),
                hash: "fromhash".to_string(),
            },
            to_frame: RegistryRef {
                peer_id: "galbot".to_string(),
                id: "base_link".to_string(),
                hash: "tohash".to_string(),
            },
            clock: RegistryRef {
                peer_id: "galbot".to_string(),
                id: "session/sdk_clock".to_string(),
                hash: "clockhash".to_string(),
            },
            source: PoseSource::Manual,
            writer_mode: PoseWriterMode::Rigid,
            expected_rate_hz: 0,
            segment_duration_ns: 1_000_000_000,
            retention_ns: 0,
        };
        let bytes = auki_jcs::canonicalize(&serde_json::to_value(&m).unwrap());
        let json = std::str::from_utf8(&bytes).unwrap();
        assert!(json.contains(r#""writer_mode":"rigid""#));
    }

    #[test]
    fn detection_log_manifest_opens_a_log_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = m1_detection_log_manifest();
        let _log = auki_logs::Log::<TestEntry>::open(dir.path(), manifest).unwrap();
        let reader = auki_logs::Log::<TestEntry>::read(dir.path()).unwrap();
        assert_eq!(reader.manifest()["detector_id"], "aukilabs/qr/v1");
        assert_eq!(
            reader.manifest()["input_sensor_id"],
            "K1-AABBCCDDEEFF/head_left_cam"
        );
        // Self-containedness check: the detection log alone surfaces
        // both producer and input identities, even after the input
        // sensor log might have been evicted.
        assert!(reader.manifest()["detector_hash"].as_str().is_some());
        assert!(reader.manifest()["input_sensor_hash"].as_str().is_some());
    }
}
