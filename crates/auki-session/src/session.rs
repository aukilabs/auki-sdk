//! Session — per-process declarative API.
//!
//! See `crate::Session` (and `docs/superpowers/specs/.../#section-§4`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use auki_registry::{
    SensorRegistryEntry, SensorBody,
    ClockRegistryEntry, ClockBody,
    FrameRegistryEntry,
    DetectorRegistryEntry, DetectorBody,
    RegistryRef,
};

use crate::error::Result;
use crate::registry_store::RegistryStore;
use crate::log_handles::{
    SensorLogHandle, PoseLogHandle, TimeTransformLogHandle, DetectionLogHandle,
};

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
