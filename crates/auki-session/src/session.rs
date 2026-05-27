//! Session — per-process declarative API.
//!
//! See `crate::Session` (and `docs/superpowers/specs/.../#section-§4`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use auki_registry::{
    SensorRegistryEntry, SensorBody,
    ClockRegistryEntry, ClockBody,
    FrameRegistryEntry,
    DetectorRegistryEntry, DetectorBody,
    RegistryRef,
};

use auki_manifests::{
    SensorLogManifest, PoseLogManifest, TimeTransformLogManifest, DetectionLogManifest,
};
use auki_registry::LogRef;

use auki_network::resources_protocol::{
    ResourceEntry, Available, Head, SensorBlock, SensorKind, PoseBlock,
    SensorManifestPointer, PoseManifestPointer, TimeTransformManifestPointer,
    DetectionManifestPointer, VariantContent,
};

use crate::error::{Result, SessionError};
use crate::materialization::MaterializationError;
use crate::registry_store::RegistryStore;
use crate::log_handles::{
    SensorLogHandle, PoseLogHandle, TimeTransformLogHandle, DetectionLogHandle,
    MaterializedLogHandle,
};
use crate::log_specs::{HeadSpec, SensorLogSpec, PoseLogSpec, TimeTransformLogSpec, DetectionLogSpec};

// ─── FrameDef ────────────────────────────────────────────────────────────────

/// Builder-style frame preset. Created from one of the four named presets
/// (`FrameDef::ros_body()` etc.) and consumed by [`Session::register_frame`].
/// The session fills in `peer_id` and `frame_id` at registration time.
pub enum FrameDef {
    RosBody,
    RosOptical,
    OpenGl,
    Unity,
}

impl FrameDef {
    /// REP-103 body frame: right-handed, X forward, Y left, Z up, meters.
    pub fn ros_body() -> Self { Self::RosBody }
    /// REP-103 camera optical frame: right-handed, X right, Y down, Z forward, meters.
    pub fn ros_optical() -> Self { Self::RosOptical }
    /// OpenGL / Three.js: right-handed, X right, Y up, Z backward, meters.
    pub fn opengl() -> Self { Self::OpenGl }
    /// Unity: left-handed, X right, Y up, Z forward, meters.
    pub fn unity() -> Self { Self::Unity }

    fn into_entry(self, peer_id: impl Into<String>, frame_id: impl Into<String>) -> FrameRegistryEntry {
        let peer_id = peer_id.into();
        let frame_id = frame_id.into();
        match self {
            Self::RosBody    => FrameRegistryEntry::ros_body(peer_id, frame_id),
            Self::RosOptical => FrameRegistryEntry::ros_optical(peer_id, frame_id),
            Self::OpenGl     => FrameRegistryEntry::opengl(peer_id, frame_id),
            Self::Unity      => FrameRegistryEntry::unity(peer_id, frame_id),
        }
    }
}

pub struct Session {
    pub(crate) inner: Arc<RwLock<SessionInner>>,
}

pub(crate) struct SessionInner {
    pub(crate) peer_id: String,
    pub(crate) app_id: String,
    pub(crate) session_id: String,
    pub(crate) storage_root: PathBuf,

    pub(crate) sensors:   RegistryStore<SensorRegistryEntry>,
    pub(crate) clocks:    RegistryStore<ClockRegistryEntry>,
    pub(crate) frames:    RegistryStore<FrameRegistryEntry>,
    pub(crate) detectors: RegistryStore<DetectorRegistryEntry>,

    // Keyed by (source_peer_id, resource_id). For own logs source==self.peer_id;
    // for materializations source!=self.peer_id.
    pub(crate) sensor_logs:    HashMap<(String, String), Arc<SensorLogHandle>>,
    pub(crate) pose_logs:      HashMap<(String, String), Arc<PoseLogHandle>>,
    pub(crate) time_logs:      HashMap<(String, String), Arc<TimeTransformLogHandle>>,
    pub(crate) detection_logs: HashMap<(String, String), Arc<DetectionLogHandle>>,
}

impl Session {
    pub fn new(peer_id: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(SessionInner {
                peer_id: peer_id.into(),
                app_id: app_id.into(),
                session_id: ulid::Ulid::new().to_string(),
                storage_root: PathBuf::from("."),
                sensors:   RegistryStore::default(),
                clocks:    RegistryStore::default(),
                frames:    RegistryStore::default(),
                detectors: RegistryStore::default(),
                sensor_logs:    HashMap::new(),
                pose_logs:      HashMap::new(),
                time_logs:      HashMap::new(),
                detection_logs: HashMap::new(),
            })),
        }
    }

    pub fn with_storage_root(self, root: PathBuf) -> Self {
        self.inner.write().storage_root = root;
        self
    }

    pub fn peer_id(&self) -> String { self.inner.read().peer_id.clone() }
    pub fn app_id(&self) -> String { self.inner.read().app_id.clone() }
    pub fn session_id(&self) -> String { self.inner.read().session_id.clone() }
    pub fn storage_root(&self) -> PathBuf { self.inner.read().storage_root.clone() }

    // ─── Registry registration ────────────────────────────────────────────

    /// Register a sensor, writing the entry to disk and stashing it
    /// in the in-memory store.
    ///
    /// Validates `sensor_id` (rejects `>`, `@`, whitespace) before writing.
    /// Writes are idempotent: identical content → same hash → no-op on disk.
    pub fn register_sensor(&self, sensor_id: &str, body: SensorBody) -> Result<RegistryRef> {
        SensorRegistryEntry::validate_id(sensor_id)?;
        let mut inner = self.inner.write();
        let entry = SensorRegistryEntry {
            peer_id: inner.peer_id.clone(),
            sensor_id: sensor_id.to_string(),
            body,
        };
        let hash = entry.hash();
        auki_registry::write_sensor(&inner.storage_root, &entry)?;
        let registry_ref = RegistryRef {
            peer_id: inner.peer_id.clone(),
            id: sensor_id.to_string(),
            hash,
        };
        inner.sensors.insert(sensor_id, entry);
        Ok(registry_ref)
    }

    /// Register a clock, writing the entry to disk and stashing it
    /// in the in-memory store.
    ///
    /// Validates `clock_id` (rejects `>`, `@`, whitespace) before writing.
    pub fn register_clock(&self, clock_id: &str, body: ClockBody) -> Result<RegistryRef> {
        ClockRegistryEntry::validate_id(clock_id)?;
        let mut inner = self.inner.write();
        let entry = ClockRegistryEntry {
            peer_id: inner.peer_id.clone(),
            clock_id: clock_id.to_string(),
            body,
        };
        let hash = entry.hash();
        auki_registry::write_clock(&inner.storage_root, &entry)?;
        let registry_ref = RegistryRef {
            peer_id: inner.peer_id.clone(),
            id: clock_id.to_string(),
            hash,
        };
        inner.clocks.insert(clock_id, entry);
        Ok(registry_ref)
    }

    /// Register a coordinate frame using a [`FrameDef`] preset.
    ///
    /// The session fills in `peer_id`; the caller provides `frame_id` and
    /// the preset that describes the convention.
    ///
    /// Validates `frame_id` (rejects `>`, `@`, whitespace) before writing.
    pub fn register_frame(&self, frame_id: &str, def: FrameDef) -> Result<RegistryRef> {
        FrameRegistryEntry::validate_id(frame_id)?;
        let mut inner = self.inner.write();
        let entry = def.into_entry(inner.peer_id.clone(), frame_id);
        let hash = entry.hash();
        auki_registry::write_frame(&inner.storage_root, &entry)?;
        let registry_ref = RegistryRef {
            peer_id: inner.peer_id.clone(),
            id: frame_id.to_string(),
            hash,
        };
        inner.frames.insert(frame_id, entry);
        Ok(registry_ref)
    }

    /// Register a detector, writing the entry to disk and stashing it
    /// in the in-memory store.
    ///
    /// `output_types` declares the detection `type` strings this detector
    /// emits (e.g. `["aruco"]`).
    ///
    /// Validates `detector_id` (rejects `>`, `@`, whitespace) before writing.
    pub fn register_detector(
        &self,
        detector_id: &str,
        body: DetectorBody,
        output_types: Vec<String>,
    ) -> Result<RegistryRef> {
        DetectorRegistryEntry::validate_id(detector_id)?;
        let mut inner = self.inner.write();
        let entry = DetectorRegistryEntry {
            peer_id: inner.peer_id.clone(),
            detector_id: detector_id.to_string(),
            body,
            output_types,
        };
        let hash = entry.hash();
        auki_registry::write_detector(&inner.storage_root, &entry)?;
        let registry_ref = RegistryRef {
            peer_id: inner.peer_id.clone(),
            id: detector_id.to_string(),
            hash,
        };
        inner.detectors.insert(detector_id, entry);
        Ok(registry_ref)
    }

    // ─── Log registration ─────────────────────────────────────────────────

    /// Register a sensor log, writing the manifest to disk and stashing a handle
    /// in the in-memory log map.
    ///
    /// `resource_id` is derived as `spec.sensor.id` (§6).
    /// Rejects duplicate `(source_peer_id, resource_id)` with
    /// [`SessionError::DuplicateLog`].
    pub fn register_sensor_log(&self, spec: SensorLogSpec) -> Result<SensorLogHandle> {
        let mut inner = self.inner.write();
        let resource_id = spec.sensor.id.clone();
        let key = (inner.peer_id.clone(), resource_id.clone());
        if inner.sensor_logs.contains_key(&key) {
            return Err(SessionError::DuplicateLog {
                source_peer_id: inner.peer_id.clone(),
                resource_id,
            });
        }

        // Derive kind+type from the registered SensorRegistryEntry.
        let sensor_entry = inner.sensors.get(&spec.sensor.id)
            .ok_or_else(|| SessionError::DuplicateLog {
                // Reuse DuplicateLog as a placeholder; registry lookup errors
                // don't have their own variant yet — acceptable for Phase 4.
                source_peer_id: inner.peer_id.clone(),
                resource_id: format!("sensor not registered: {}", spec.sensor.id),
            })?;
        let (sensor_kind, sensor_type) = match &sensor_entry.body {
            SensorBody::Camera(b)       => (SensorKind::Camera, b.r#type.clone()),
            SensorBody::Rangefinder(b)  => (SensorKind::Rangefinder, b.r#type.clone()),
            SensorBody::Rf(b)           => (SensorKind::Rf, b.r#type.clone()),
            SensorBody::Audio(b)        => (SensorKind::Audio, b.r#type.clone()),
            SensorBody::JointEncoders(b) => (SensorKind::JointEncoders, b.r#type.clone()),
        };

        let head_spec = spec.head.clone();
        let manifest = SensorLogManifest {
            source_peer_id: inner.peer_id.clone(),
            writer_peer_id: inner.peer_id.clone(),
            app_id: inner.app_id.clone(),
            session_id: inner.session_id.clone(),
            sensor: spec.sensor,
            clock: spec.clock,
            frame: spec.frame,
            segment_duration_ns: spec.segment_duration.as_nanos().min(i64::MAX as u128) as i64,
            retention_ns: spec.retention.as_nanos().min(i64::MAX as u128) as i64,
        };

        let manifest_dir = inner.storage_root
            .join("logs")
            .join(&inner.peer_id)
            .join(&resource_id);
        std::fs::create_dir_all(&manifest_dir)?;
        let canonical_bytes = auki_jcs::canonicalize(&serde_json::to_value(&manifest)
            .expect("SensorLogManifest serializes"));
        std::fs::write(manifest_dir.join("manifest.json"), &canonical_bytes)?;

        let log_ref = LogRef {
            source_peer_id: inner.peer_id.clone(),
            resource_id: resource_id.clone(),
        };
        let handle = SensorLogHandle {
            resource_id: resource_id.clone(),
            log_ref: log_ref.clone(),
            manifest: manifest.clone(),
            head_spec: head_spec.clone(),
            sensor_kind,
            sensor_type: sensor_type.clone(),
        };
        inner.sensor_logs.insert(key, std::sync::Arc::new(SensorLogHandle {
            resource_id,
            log_ref,
            manifest,
            head_spec,
            sensor_kind,
            sensor_type,
        }));
        Ok(handle)
    }

    /// Register a pose log, writing the manifest to disk and stashing a handle
    /// in the in-memory log map.
    ///
    /// `resource_id` is derived as `"<from_frame.id>-><to_frame.id>"` (§6).
    /// Rejects duplicate `(source_peer_id, resource_id)` with
    /// [`SessionError::DuplicateLog`].
    pub fn register_pose_log(&self, spec: PoseLogSpec) -> Result<PoseLogHandle> {
        let mut inner = self.inner.write();
        let resource_id = format!("{}->{}", spec.from_frame.id, spec.to_frame.id);
        let key = (inner.peer_id.clone(), resource_id.clone());
        if inner.pose_logs.contains_key(&key) {
            return Err(SessionError::DuplicateLog {
                source_peer_id: inner.peer_id.clone(),
                resource_id,
            });
        }

        let head_spec = spec.head.clone();
        let writer_mode = spec.writer_mode.clone();
        let manifest = PoseLogManifest {
            source_peer_id: inner.peer_id.clone(),
            writer_peer_id: inner.peer_id.clone(),
            app_id: inner.app_id.clone(),
            session_id: inner.session_id.clone(),
            from_frame: spec.from_frame,
            to_frame: spec.to_frame,
            clock: spec.clock,
            source: spec.source,
            writer_mode: spec.writer_mode,
            expected_rate_hz: spec.expected_rate_hz,
            segment_duration_ns: spec.segment_duration.as_nanos().min(i64::MAX as u128) as i64,
            retention_ns: spec.retention.as_nanos().min(i64::MAX as u128) as i64,
        };

        let manifest_dir = inner.storage_root
            .join("logs")
            .join(&inner.peer_id)
            .join(&resource_id);
        std::fs::create_dir_all(&manifest_dir)?;
        let canonical_bytes = auki_jcs::canonicalize(&serde_json::to_value(&manifest)
            .expect("PoseLogManifest serializes"));
        std::fs::write(manifest_dir.join("manifest.json"), &canonical_bytes)?;

        let log_ref = LogRef {
            source_peer_id: inner.peer_id.clone(),
            resource_id: resource_id.clone(),
        };
        let handle = PoseLogHandle {
            resource_id: resource_id.clone(),
            log_ref: log_ref.clone(),
            manifest: manifest.clone(),
            head_spec: head_spec.clone(),
            writer_mode: writer_mode.clone(),
        };
        inner.pose_logs.insert(key, std::sync::Arc::new(PoseLogHandle {
            resource_id,
            log_ref,
            manifest,
            head_spec,
            writer_mode,
        }));
        Ok(handle)
    }

    /// Register a time-transform log, writing the manifest to disk and stashing a handle
    /// in the in-memory log map.
    ///
    /// `resource_id` is derived as `"<from_clock.id>-><to_clock.id>"` (§6).
    /// Rejects duplicate `(source_peer_id, resource_id)` with
    /// [`SessionError::DuplicateLog`].
    pub fn register_time_transform_log(&self, spec: TimeTransformLogSpec) -> Result<TimeTransformLogHandle> {
        let mut inner = self.inner.write();
        let resource_id = format!("{}->{}", spec.from_clock.id, spec.to_clock.id);
        let key = (inner.peer_id.clone(), resource_id.clone());
        if inner.time_logs.contains_key(&key) {
            return Err(SessionError::DuplicateLog {
                source_peer_id: inner.peer_id.clone(),
                resource_id,
            });
        }

        let head_spec = spec.head.clone();
        let manifest = TimeTransformLogManifest {
            source_peer_id: inner.peer_id.clone(),
            writer_peer_id: inner.peer_id.clone(),
            app_id: inner.app_id.clone(),
            session_id: inner.session_id.clone(),
            from_clock: spec.from_clock,
            to_clock: spec.to_clock,
            source: spec.source,
            segment_duration_ns: spec.segment_duration.as_nanos().min(i64::MAX as u128) as i64,
            retention_ns: spec.retention.as_nanos().min(i64::MAX as u128) as i64,
        };

        let manifest_dir = inner.storage_root
            .join("logs")
            .join(&inner.peer_id)
            .join(&resource_id);
        std::fs::create_dir_all(&manifest_dir)?;
        let canonical_bytes = auki_jcs::canonicalize(&serde_json::to_value(&manifest)
            .expect("TimeTransformLogManifest serializes"));
        std::fs::write(manifest_dir.join("manifest.json"), &canonical_bytes)?;

        let log_ref = LogRef {
            source_peer_id: inner.peer_id.clone(),
            resource_id: resource_id.clone(),
        };
        let handle = TimeTransformLogHandle {
            resource_id: resource_id.clone(),
            log_ref: log_ref.clone(),
            manifest: manifest.clone(),
            head_spec: head_spec.clone(),
        };
        inner.time_logs.insert(key, std::sync::Arc::new(TimeTransformLogHandle {
            resource_id,
            log_ref,
            manifest,
            head_spec,
        }));
        Ok(handle)
    }

    /// Register a detection log, writing the manifest to disk and stashing a handle
    /// in the in-memory log map.
    ///
    /// `resource_id` is derived as `"<detector.id>@<input_sensor.id>"` (§6).
    /// Rejects duplicate `(source_peer_id, resource_id)` with
    /// [`SessionError::DuplicateLog`].
    pub fn register_detection_log(&self, spec: DetectionLogSpec) -> Result<DetectionLogHandle> {
        let mut inner = self.inner.write();
        let resource_id = format!("{}@{}", spec.detector.id, spec.input_sensor.id);
        let key = (inner.peer_id.clone(), resource_id.clone());
        if inner.detection_logs.contains_key(&key) {
            return Err(SessionError::DuplicateLog {
                source_peer_id: inner.peer_id.clone(),
                resource_id,
            });
        }

        let head_spec = spec.head.clone();
        let manifest = DetectionLogManifest {
            source_peer_id: inner.peer_id.clone(),
            writer_peer_id: inner.peer_id.clone(),
            app_id: inner.app_id.clone(),
            session_id: inner.session_id.clone(),
            detector: spec.detector,
            input_log: spec.input_log,
            input_sensor: spec.input_sensor,
            clock: spec.clock,
            segment_duration_ns: spec.segment_duration.as_nanos().min(i64::MAX as u128) as i64,
            retention_ns: spec.retention.as_nanos().min(i64::MAX as u128) as i64,
        };

        let manifest_dir = inner.storage_root
            .join("logs")
            .join(&inner.peer_id)
            .join(&resource_id);
        std::fs::create_dir_all(&manifest_dir)?;
        let canonical_bytes = auki_jcs::canonicalize(&serde_json::to_value(&manifest)
            .expect("DetectionLogManifest serializes"));
        std::fs::write(manifest_dir.join("manifest.json"), &canonical_bytes)?;

        let log_ref = LogRef {
            source_peer_id: inner.peer_id.clone(),
            resource_id: resource_id.clone(),
        };
        let handle = DetectionLogHandle {
            resource_id: resource_id.clone(),
            log_ref: log_ref.clone(),
            manifest: manifest.clone(),
            head_spec: head_spec.clone(),
        };
        inner.detection_logs.insert(key, std::sync::Arc::new(DetectionLogHandle {
            resource_id,
            log_ref,
            manifest,
            head_spec,
        }));
        Ok(handle)
    }

    // ─── Catalog ─────────────────────────────────────────────────────────

    /// Return a catalog row for every registered log (own + materialized).
    ///
    /// Phase 4: handles carry only identity info; the `available` block
    /// is stubbed to 0/0/0 until the backing `Log<T>` hookup lands in a
    /// later phase.
    pub fn catalog(&self) -> Vec<ResourceEntry> {
        let inner = self.inner.read();
        let mut out = Vec::new();
        for handle in inner.sensor_logs.values()    { out.push(sensor_log_row(handle)); }
        for handle in inner.pose_logs.values()      { out.push(pose_log_row(handle)); }
        for handle in inner.time_logs.values()      { out.push(time_transform_row(handle)); }
        for handle in inner.detection_logs.values() { out.push(detection_log_row(handle)); }
        out
    }

    // ─── Materialization ─────────────────────────────────────────────────

    /// Open a remote peer's log locally by fetching its catalog row,
    /// connecting to the serving peer, and ingesting samples.
    ///
    /// Full implementation: fetch remote catalog row, extract canonical
    /// fields, open `/auki/stream/0.2.0` against serving peer, write
    /// new local manifest, ingest samples. Deferred to a follow-up plan
    /// (Phase 5).
    pub async fn materialize_remote_log(
        &self,
        _log_ref: LogRef,
        _retention: Duration,
        _segment_duration: Duration,
    ) -> Result<MaterializedLogHandle> {
        Err(SessionError::Materialization(MaterializationError::NotImplemented))
    }

    /// Open the sealed one-sample pose log identified by `log_ref` and
    /// read its single sample. Used to migrate consumers that previously
    /// read rigid transforms from `TransformEdgeResource.transform` inline.
    ///
    /// Full implementation requires a working remote-stream path (Phase 5).
    /// Returns `NotImplemented` until then; the surface lets app code
    /// be written against it now.
    pub async fn resolve_static_transform(
        &self,
        _log_ref: LogRef,
    ) -> Result<auki_datatypes::pose::SpatialTransform> {
        Err(SessionError::Materialization(MaterializationError::NotImplemented))
    }
}

// ─── Catalog row helper functions ────────────────────────────────────────────

fn head_from_spec(spec: &HeadSpec) -> Option<Head> {
    match spec {
        HeadSpec::Rolling { retention_ns } => Some(Head::Rolling { retention_ns: *retention_ns }),
        HeadSpec::Fixed => Some(Head::Fixed { started_at_ns: 0 }), // stub; real timestamp when backing Log<T> is wired
    }
}

fn sensor_log_row(handle: &SensorLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available { bytes: 0, entries: 0, duration_ns: 0 },
        sensor: Some(SensorBlock {
            kind: handle.sensor_kind,
            r#type: handle.sensor_type.clone(),
            sensor_id: handle.manifest.sensor.id.clone(),
            sensor_hash: handle.manifest.sensor.hash.clone(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: handle.manifest.clock.clone(),
                frame: handle.manifest.frame.clone(),
            },
        },
    }
}

fn pose_log_row(handle: &PoseLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available { bytes: 0, entries: 0, duration_ns: 0 },
        sensor: None,
        pose: Some(PoseBlock { writer_mode: handle.writer_mode.clone() }),
        variant_content: VariantContent::PoseLog {
            manifest: PoseManifestPointer {
                from_frame: handle.manifest.from_frame.clone(),
                to_frame: handle.manifest.to_frame.clone(),
                clock: handle.manifest.clock.clone(),
                source: handle.manifest.source.clone(),
                expected_rate_hz: handle.manifest.expected_rate_hz,
            },
        },
    }
}

fn time_transform_row(handle: &TimeTransformLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available { bytes: 0, entries: 0, duration_ns: 0 },
        sensor: None,
        pose: None,
        variant_content: VariantContent::TimeTransformLog {
            manifest: TimeTransformManifestPointer {
                from_clock: handle.manifest.from_clock.clone(),
                to_clock: handle.manifest.to_clock.clone(),
                source: handle.manifest.source.clone(),
            },
        },
    }
}

fn detection_log_row(handle: &DetectionLogHandle) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: handle.manifest.source_peer_id.clone(),
        writer_peer_id: handle.manifest.writer_peer_id.clone(),
        resource_id: handle.resource_id.clone(),
        state: "live".to_string(),
        head: head_from_spec(&handle.head_spec),
        extent: None,
        available: Available { bytes: 0, entries: 0, duration_ns: 0 },
        sensor: None,
        pose: None,
        variant_content: VariantContent::DetectionLog {
            manifest: DetectionManifestPointer {
                detector: handle.manifest.detector.clone(),
                input_log: handle.manifest.input_log.clone(),
                input_sensor: handle.manifest.input_sensor.clone(),
                clock: handle.manifest.clock.clone(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn new_carries_peer_app_and_generates_session_id() {
        let s = Session::new("galbot", "galbot-ctrl");
        assert_eq!(s.peer_id(), "galbot");
        assert_eq!(s.app_id(), "galbot-ctrl");
        assert!(!s.session_id().is_empty());
        // ULIDs are 26 chars
        assert_eq!(s.session_id().len(), 26);
    }

    #[test]
    fn session_id_unique_per_session() {
        let a = Session::new("p", "a");
        let b = Session::new("p", "a");
        assert_ne!(a.session_id(), b.session_id());
    }

    #[test]
    fn with_storage_root_sets_root() {
        let tmp = tempdir().unwrap();
        let s = Session::new("p", "a").with_storage_root(tmp.path().to_path_buf());
        assert_eq!(s.storage_root(), tmp.path());
    }

    #[test]
    fn storage_root_defaults_to_dot() {
        let s = Session::new("p", "a");
        assert_eq!(s.storage_root(), PathBuf::from("."));
    }
}

#[cfg(test)]
mod register_tests {
    use super::*;
    use tempfile::tempdir;
    use auki_registry::{
        Camera, ClockMeta, Scope,
        DetectorBody, ObjectDetection,
    };
    use crate::SessionError;

    // ─── helpers ─────────────────────────────────────────────────────────────

    /// A RegistryRef that will be used as the camera's frame reference.
    /// We pre-register the frame so write_sensor doesn't reject it.
    fn register_optical_frame(s: &Session) -> RegistryRef {
        s.register_frame("head_left_camera_optical", FrameDef::ros_optical()).unwrap()
    }

    fn camera_body(frame: RegistryRef) -> SensorBody {
        SensorBody::Camera(Camera {
            r#type: "rgb".to_string(),
            width: 1920,
            height: 1200,
            frame_rate_hz: 30,
            pixel_format: "rgb8".to_string(),
            color_space: "srgb".to_string(),
            intrinsics_model: "pinhole".to_string(),
            distortion_model: "brown_conrady".to_string(),
            frame,
        })
    }

    // ─── sensor ──────────────────────────────────────────────────────────────

    #[test]
    fn register_sensor_returns_registry_ref_with_self_peer_id() {
        let tmp = tempdir().unwrap();
        let s = Session::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let frame_ref = register_optical_frame(&s);
        let r = s.register_sensor("head_left_rgb", camera_body(frame_ref.clone())).unwrap();
        assert_eq!(r.peer_id, "galbot");
        assert_eq!(r.id, "head_left_rgb");
        assert!(!r.hash.is_empty());
        assert_eq!(r.hash.len(), 32); // XXH3-128 → 32 hex chars

        // Idempotent — same body → same hash
        let r2 = s.register_sensor("head_left_rgb", camera_body(frame_ref)).unwrap();
        assert_eq!(r.hash, r2.hash);

        // Disk: the entry file exists at peer_id-qualified path
        // auki-layout translates '/' in IDs to '__'
        let path = tmp.path()
            .join("registries")
            .join("sensors")
            .join("galbot")
            .join("head_left_rgb")
            .join(format!("{}.json", r.hash));
        assert!(path.exists(), "expected entry at {}", path.display());
    }

    #[test]
    fn register_sensor_rejects_invalid_id() {
        let tmp = tempdir().unwrap();
        let s = Session::new("p", "a").with_storage_root(tmp.path().to_path_buf());
        let frame_ref = register_optical_frame(&s);
        let result = s.register_sensor("bad>id", camera_body(frame_ref));
        assert!(matches!(result, Err(SessionError::InvalidId(_))));
    }

    #[test]
    fn register_sensor_in_memory_store() {
        let tmp = tempdir().unwrap();
        let s = Session::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let frame_ref = register_optical_frame(&s);
        s.register_sensor("head_left_rgb", camera_body(frame_ref)).unwrap();
        let inner = s.inner.read();
        assert!(inner.sensors.get("head_left_rgb").is_some());
    }

    // ─── clock ───────────────────────────────────────────────────────────────

    #[test]
    fn register_clock_returns_registry_ref_with_self_peer_id() {
        let tmp = tempdir().unwrap();
        let s = Session::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let body = ClockBody::MonotonicClock(ClockMeta {
            unit: "ns".to_string(),
            monotonic: true,
            epoch: None,
            scope: Scope::DeviceLocal,
        });
        let r = s.register_clock("session/sdk_clock", body).unwrap();
        assert_eq!(r.peer_id, "galbot");
        assert_eq!(r.id, "session/sdk_clock");
        assert!(!r.hash.is_empty());
    }

    #[test]
    fn register_clock_rejects_invalid_id() {
        let tmp = tempdir().unwrap();
        let s = Session::new("p", "a").with_storage_root(tmp.path().to_path_buf());
        let body = ClockBody::MonotonicClock(ClockMeta {
            unit: "ns".to_string(),
            monotonic: true,
            epoch: None,
            scope: Scope::DeviceLocal,
        });
        let result = s.register_clock("bad@id", body);
        assert!(matches!(result, Err(SessionError::InvalidId(_))));
    }

    // ─── frame ───────────────────────────────────────────────────────────────

    #[test]
    fn register_frame_with_ros_optical_preset() {
        let tmp = tempdir().unwrap();
        let s = Session::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let r = s.register_frame("head_left_camera_optical", FrameDef::ros_optical()).unwrap();
        assert_eq!(r.peer_id, "galbot");
        assert_eq!(r.id, "head_left_camera_optical");
        assert!(!r.hash.is_empty());
    }

    #[test]
    fn register_frame_all_presets_produce_distinct_hashes() {
        let tmp = tempdir().unwrap();
        let s = Session::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let rb = s.register_frame("a", FrameDef::ros_body()).unwrap();
        let ro = s.register_frame("b", FrameDef::ros_optical()).unwrap();
        let gl = s.register_frame("c", FrameDef::opengl()).unwrap();
        let u  = s.register_frame("d", FrameDef::unity()).unwrap();
        // All four presets are distinct axis conventions → distinct hashes
        let hashes = [rb.hash, ro.hash, gl.hash, u.hash];
        let unique: std::collections::HashSet<_> = hashes.iter().collect();
        assert_eq!(unique.len(), 4, "all four frame presets must have distinct hashes");
    }

    #[test]
    fn register_frame_rejects_invalid_id() {
        let tmp = tempdir().unwrap();
        let s = Session::new("p", "a").with_storage_root(tmp.path().to_path_buf());
        let result = s.register_frame("bad id", FrameDef::ros_body());
        assert!(matches!(result, Err(SessionError::InvalidId(_))));
    }

    // ─── detector ────────────────────────────────────────────────────────────

    #[test]
    fn register_detector_returns_registry_ref() {
        let tmp = tempdir().unwrap();
        let s = Session::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let body = DetectorBody::ObjectDetection(ObjectDetection {
            model: "yolo_v8n".to_string(),
        });
        let r = s.register_detector("yolo_v8", body, vec!["object".to_string()]).unwrap();
        assert_eq!(r.peer_id, "galbot");
        assert_eq!(r.id, "yolo_v8");
        assert!(!r.hash.is_empty());
    }

    #[test]
    fn register_detector_rejects_invalid_id() {
        let tmp = tempdir().unwrap();
        let s = Session::new("p", "a").with_storage_root(tmp.path().to_path_buf());
        let body = DetectorBody::ObjectDetection(ObjectDetection {
            model: "yolo_v8n".to_string(),
        });
        let result = s.register_detector("bad>id", body, vec![]);
        assert!(matches!(result, Err(SessionError::InvalidId(_))));
    }
}

#[cfg(test)]
mod register_log_tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;
    use auki_registry::{Camera, ClockMeta, Scope, DetectorBody, ObjectDetection, LogRef};
    use auki_manifests::{PoseSource, PoseWriterMode, TimeTransformSource};
    use crate::log_specs::{SensorLogSpec, PoseLogSpec, TimeTransformLogSpec, DetectionLogSpec, HeadSpec};

    fn fixture_session() -> (Session, tempfile::TempDir) {
        let tmp = tempdir().unwrap();
        let s = Session::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        (s, tmp)
    }

    fn camera_body(frame: RegistryRef) -> SensorBody {
        SensorBody::Camera(Camera {
            r#type: "rgb".to_string(),
            width: 1920, height: 1200, frame_rate_hz: 30,
            pixel_format: "rgb8".to_string(),
            color_space: "srgb".to_string(),
            intrinsics_model: "pinhole".to_string(),
            distortion_model: "brown_conrady".to_string(),
            frame,
        })
    }

    fn monotonic_clock_body() -> ClockBody {
        ClockBody::MonotonicClock(ClockMeta {
            unit: "ns".to_string(),
            monotonic: true,
            epoch: None,
            scope: Scope::DeviceLocal,
        })
    }

    fn utc_clock_body() -> ClockBody {
        ClockBody::UtcClock(ClockMeta {
            unit: "ns".to_string(),
            monotonic: false,
            epoch: Some("1970-01-01T00:00:00Z".to_string()),
            scope: Scope::Global,
        })
    }

    fn fixture_registries(s: &Session) -> (RegistryRef, RegistryRef, RegistryRef) {
        let frame = s.register_frame("head_left_camera_optical", FrameDef::ros_optical()).unwrap();
        let sensor = s.register_sensor("head_left_rgb", camera_body(frame.clone())).unwrap();
        let clock  = s.register_clock("session/sdk_clock", monotonic_clock_body()).unwrap();
        (sensor, clock, frame)
    }

    #[test]
    fn register_sensor_log_resource_id_is_sensor_id() {
        let (s, tmp) = fixture_session();
        let (sensor, clock, frame) = fixture_registries(&s);

        let handle = s.register_sensor_log(SensorLogSpec {
            sensor: sensor.clone(),
            clock,
            frame: Some(frame),
            head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        }).unwrap();

        assert_eq!(handle.resource_id(), "head_left_rgb");
        assert_eq!(handle.log_ref().source_peer_id, "galbot");
        assert_eq!(handle.log_ref().resource_id, "head_left_rgb");

        let manifest_path = tmp.path().join("logs/galbot/head_left_rgb/manifest.json");
        assert!(manifest_path.exists(), "manifest.json missing at {}", manifest_path.display());
    }

    #[test]
    fn register_sensor_log_rejects_duplicate() {
        let (s, _tmp) = fixture_session();
        let (sensor, clock, frame) = fixture_registries(&s);
        let spec = SensorLogSpec {
            sensor: sensor.clone(),
            clock: clock.clone(),
            frame: Some(frame.clone()),
            head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        };
        s.register_sensor_log(spec.clone()).unwrap();
        let result = s.register_sensor_log(spec);
        assert!(matches!(result, Err(SessionError::DuplicateLog { .. })));
    }

    #[test]
    fn register_pose_log_resource_id_is_from_arrow_to() {
        let (s, _tmp) = fixture_session();
        let (_sensor, clock, _frame) = fixture_registries(&s);
        let world = s.register_frame("world", FrameDef::ros_body()).unwrap();
        let base_link = s.register_frame("base_link", FrameDef::ros_body()).unwrap();

        let handle = s.register_pose_log(PoseLogSpec {
            from_frame: world,
            to_frame: base_link,
            clock,
            source: PoseSource::Manual,
            writer_mode: PoseWriterMode::Movable,
            expected_rate_hz: 30,
            head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        }).unwrap();

        assert_eq!(handle.resource_id(), "world->base_link");
    }

    #[test]
    fn register_time_transform_log_resource_id_format() {
        let (s, _tmp) = fixture_session();
        let clock = s.register_clock("session/sdk_clock", monotonic_clock_body()).unwrap();
        let wall = s.register_clock("wall_clock", utc_clock_body()).unwrap();

        let handle = s.register_time_transform_log(TimeTransformLogSpec {
            from_clock: clock,
            to_clock: wall,
            source: TimeTransformSource::LocalClockRead,
            head: HeadSpec::Rolling { retention_ns: 60_000_000_000 },
            segment_duration: Duration::from_secs(60),
            retention: Duration::from_secs(3600),
        }).unwrap();

        assert_eq!(handle.resource_id(), "session/sdk_clock->wall_clock");
    }

    #[test]
    fn register_detection_log_resource_id_format() {
        let (s, _tmp) = fixture_session();
        let (sensor, clock, _frame) = fixture_registries(&s);
        let detector = s.register_detector(
            "yolo_v8",
            DetectorBody::ObjectDetection(ObjectDetection { model: "yolo_v8n".to_string() }),
            vec!["bounding_box".to_string()],
        ).unwrap();
        let input_log = LogRef {
            source_peer_id: "galbot".to_string(),
            resource_id: "head_left_rgb".to_string(),
        };
        let handle = s.register_detection_log(DetectionLogSpec {
            detector,
            input_log,
            input_sensor: sensor,
            clock,
            head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        }).unwrap();

        assert_eq!(handle.resource_id(), "yolo_v8@head_left_rgb");
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;
    use auki_registry::{Camera, ClockMeta, Scope, LogRef};
    use auki_network::resources_protocol::{Head, SensorKind, VariantContent};
    use crate::log_specs::{SensorLogSpec, HeadSpec};
    use crate::materialization::MaterializationError;

    fn fixture_session() -> (Session, tempfile::TempDir) {
        let tmp = tempdir().unwrap();
        let s = Session::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        (s, tmp)
    }

    fn fixture_registries(s: &Session) -> (RegistryRef, RegistryRef, RegistryRef) {
        let frame = s.register_frame("head_left_camera_optical", FrameDef::ros_optical()).unwrap();
        let sensor = s.register_sensor("head_left_rgb", SensorBody::Camera(Camera {
            r#type: "rgb".to_string(),
            width: 1920, height: 1200, frame_rate_hz: 30,
            pixel_format: "rgb8".to_string(),
            color_space: "srgb".to_string(),
            intrinsics_model: "pinhole".to_string(),
            distortion_model: "brown_conrady".to_string(),
            frame: frame.clone(),
        })).unwrap();
        let clock = s.register_clock("session/sdk_clock", ClockBody::MonotonicClock(ClockMeta {
            unit: "ns".to_string(),
            monotonic: true,
            epoch: None,
            scope: Scope::DeviceLocal,
        })).unwrap();
        (sensor, clock, frame)
    }

    #[test]
    fn catalog_returns_a_row_per_registered_log() {
        let (s, _tmp) = fixture_session();
        let (sensor, clock, frame) = fixture_registries(&s);
        let _h = s.register_sensor_log(SensorLogSpec {
            sensor: sensor.clone(),
            clock: clock.clone(),
            frame: Some(frame),
            head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        }).unwrap();

        let rows = s.catalog();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.source_peer_id, "galbot");
        assert_eq!(row.writer_peer_id, "galbot");
        assert_eq!(row.resource_id, "head_left_rgb");
        assert_eq!(row.state, "live");
        assert!(matches!(row.head, Some(Head::Rolling { retention_ns: 5_000_000_000 })));
        let sensor_block = row.sensor.as_ref().unwrap();
        assert_eq!(sensor_block.kind, SensorKind::Camera);
        assert_eq!(sensor_block.r#type, "rgb");
        assert!(row.pose.is_none());
        assert!(matches!(row.variant_content, VariantContent::SensorLog { .. }));
    }

    #[tokio::test]
    async fn materialize_remote_log_surface_returns_not_implemented() {
        let (s, _tmp) = fixture_session();
        let result = s.materialize_remote_log(
            LogRef { source_peer_id: "galbot".to_string(), resource_id: "head_left_rgb".to_string() },
            Duration::from_secs(300),
            Duration::from_secs(10),
        ).await;
        assert!(matches!(result, Err(SessionError::Materialization(MaterializationError::NotImplemented))));
    }

    #[tokio::test]
    async fn resolve_static_transform_surface_returns_not_implemented() {
        let (s, _tmp) = fixture_session();
        let result = s.resolve_static_transform(
            LogRef { source_peer_id: "park".to_string(), resource_id: "world->base_link".to_string() }
        ).await;
        assert!(matches!(result, Err(SessionError::Materialization(MaterializationError::NotImplemented))));
    }
}
