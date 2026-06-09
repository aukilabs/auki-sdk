//! Peer — the stable, long-lived identity and registries for the Auki SDK.
//!
//! A `Peer` owns `peer_id`, `app_id`, `storage_root`, and the eternal
//! sensor / frame / detector registries. It persists across session
//! restarts; each restart starts a fresh [`crate::Session`] via
//! [`Peer::start_session`]. See #274 Phase 1.

use std::path::PathBuf;
use std::sync::Arc;

use auki_registry::{
    DetectorBody, DetectorRegistryEntry, FrameRegistryEntry, RegistryRef, SensorBody,
    SensorRegistryEntry,
};
use parking_lot::RwLock;

use crate::error::Result;
use crate::registry_store::RegistryStore;
use crate::session::Session;

// ─── FrameDef ────────────────────────────────────────────────────────────────

/// Builder-style frame preset. Created from one of the four named presets
/// (`FrameDef::ros_body()` etc.) and consumed by [`Peer::register_frame`].
/// The peer fills in `peer_id` and `frame_id` at registration time.
pub enum FrameDef {
    RosBody,
    RosOptical,
    OpenGl,
    Unity,
}

impl FrameDef {
    /// REP-103 body frame: right-handed, X forward, Y left, Z up, meters.
    pub fn ros_body() -> Self {
        Self::RosBody
    }
    /// REP-103 camera optical frame: right-handed, X right, Y down, Z forward, meters.
    pub fn ros_optical() -> Self {
        Self::RosOptical
    }
    /// OpenGL / Three.js: right-handed, X right, Y up, Z backward, meters.
    pub fn opengl() -> Self {
        Self::OpenGl
    }
    /// Unity: left-handed, X right, Y up, Z forward, meters.
    pub fn unity() -> Self {
        Self::Unity
    }

    pub(crate) fn into_entry(
        self,
        peer_id: impl Into<String>,
        frame_id: impl Into<String>,
    ) -> FrameRegistryEntry {
        let peer_id = peer_id.into();
        let frame_id = frame_id.into();
        match self {
            Self::RosBody => FrameRegistryEntry::ros_body(peer_id, frame_id),
            Self::RosOptical => FrameRegistryEntry::ros_optical(peer_id, frame_id),
            Self::OpenGl => FrameRegistryEntry::opengl(peer_id, frame_id),
            Self::Unity => FrameRegistryEntry::unity(peer_id, frame_id),
        }
    }
}

// ─── Peer ────────────────────────────────────────────────────────────────────

pub struct Peer {
    inner: Arc<RwLock<PeerInner>>,
}

/// Inner state shared between a [`Peer`] and every [`Session`] it starts.
/// A `Session` holds an `Arc<RwLock<PeerInner>>` so it reads peer identity
/// and registries live rather than copying them. See #274 (D7).
pub(crate) struct PeerInner {
    pub(crate) peer_id: String,
    pub(crate) app_id: String,
    pub(crate) storage_root: PathBuf,

    pub(crate) sensors: RegistryStore<SensorRegistryEntry>,
    pub(crate) frames: RegistryStore<FrameRegistryEntry>,
    pub(crate) detectors: RegistryStore<DetectorRegistryEntry>,
}

impl Peer {
    pub fn new(peer_id: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(PeerInner {
                peer_id: peer_id.into(),
                app_id: app_id.into(),
                storage_root: PathBuf::from("."),
                sensors: RegistryStore::default(),
                frames: RegistryStore::default(),
                detectors: RegistryStore::default(),
            })),
        }
    }

    pub fn with_storage_root(self, root: PathBuf) -> Self {
        self.inner.write().storage_root = root;
        self
    }

    /// In-place mutator equivalent of [`Peer::with_storage_root`], for FFI /
    /// binding wrappers (PyO3, UniFFI) that can't express take-by-value
    /// builders. Mutates through the inner `Arc<RwLock>`.
    pub fn set_storage_root(&self, root: PathBuf) {
        self.inner.write().storage_root = root;
    }

    /// Register a coordinate frame using a [`FrameDef`] preset, writing the
    /// entry to disk and stashing it in the in-memory store.
    ///
    /// The peer fills in `peer_id`; the caller provides `frame_id` and the
    /// preset. Validates `frame_id` (rejects `>`, `@`, whitespace) first.
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

    pub fn peer_id(&self) -> String {
        self.inner.read().peer_id.clone()
    }

    pub fn app_id(&self) -> String {
        self.inner.read().app_id.clone()
    }

    pub fn storage_root(&self) -> PathBuf {
        self.inner.read().storage_root.clone()
    }

    /// Register a sensor, writing the entry to disk and stashing it in the
    /// in-memory store. Validates `sensor_id` first. Writes are idempotent:
    /// identical content → same hash → no-op on disk.
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

    /// Register a detector, writing the entry to disk and stashing it in the
    /// in-memory store. `output_types` declares the detection `type` strings
    /// this detector emits. Validates `detector_id` first.
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

    /// Start a new session on this peer: mints a fresh `session_id` and
    /// registers the session's monotonic + UTC clocks in the corrected shape
    /// (`epoch: null` for monotonic, `unit: "ns"`, UTC `device-local`,
    /// `session_id` carried as a typed field). See #274 (D6).
    ///
    /// One live session at a time is the intended model; a restart calls
    /// `start_session` again for a fresh timeline while the peer's registries
    /// persist.
    pub fn start_session(&self) -> Result<Session> {
        Session::start(self.inner.clone())
    }

    /// A cheaply-cloneable read handle over this peer's registries, for
    /// `auki-domain` to resolve registry entries (e.g. a sensor's kind/type)
    /// while building the resource catalog. See [`PeerRegistries`].
    pub fn registries(&self) -> PeerRegistries {
        PeerRegistries {
            inner: self.inner.clone(),
        }
    }
}

/// A cheaply-cloneable read handle over a [`Peer`]'s registries.
///
/// Obtained via [`Peer::registries`]. `auki-domain` holds one to look up
/// registry entries when building catalog rows (the eternal capabilities a
/// session's logs reference). Each accessor takes a brief read lock and
/// returns an owned clone.
#[derive(Clone)]
pub struct PeerRegistries {
    inner: Arc<RwLock<PeerInner>>,
}

impl PeerRegistries {
    pub fn sensor(&self, sensor_id: &str) -> Option<SensorRegistryEntry> {
        self.inner.read().sensors.get(sensor_id).cloned()
    }
    pub fn frame(&self, frame_id: &str) -> Option<FrameRegistryEntry> {
        self.inner.read().frames.get(frame_id).cloned()
    }
    pub fn detector(&self, detector_id: &str) -> Option<DetectorRegistryEntry> {
        self.inner.read().detectors.get(detector_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_registry::{Camera, SensorBody};
    use tempfile::tempdir;

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

    #[test]
    fn peer_carries_identity_and_storage_root() {
        let tmp = tempdir().unwrap();
        let p = Peer::new("galbot", "galbot-ctrl").with_storage_root(tmp.path().to_path_buf());
        assert_eq!(p.peer_id(), "galbot");
        assert_eq!(p.app_id(), "galbot-ctrl");
        assert_eq!(p.storage_root(), tmp.path());
    }

    #[test]
    fn register_frame_returns_ref_and_writes_disk() {
        let tmp = tempdir().unwrap();
        let p = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let r = p
            .register_frame("head_left_camera_optical", FrameDef::ros_optical())
            .unwrap();
        assert_eq!(r.peer_id, "galbot");
        assert_eq!(r.id, "head_left_camera_optical");
        assert!(!r.hash.is_empty());
        let path = tmp
            .path()
            .join("registries/frames/galbot/head_left_camera_optical")
            .join(format!("{}.json", r.hash));
        assert!(path.exists(), "frame entry missing at {}", path.display());
    }

    #[test]
    fn register_sensor_returns_ref_and_writes_disk() {
        let tmp = tempdir().unwrap();
        let p = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let frame = p
            .register_frame("head_left_camera_optical", FrameDef::ros_optical())
            .unwrap();
        let r = p
            .register_sensor("head_left_rgb", camera_body(frame))
            .unwrap();
        assert_eq!(r.peer_id, "galbot");
        assert_eq!(r.id, "head_left_rgb");
        assert_eq!(r.hash.len(), 32);
        let path = tmp
            .path()
            .join("registries/sensors/galbot/head_left_rgb")
            .join(format!("{}.json", r.hash));
        assert!(path.exists(), "sensor entry missing at {}", path.display());
    }

    #[test]
    fn register_detector_returns_ref() {
        use auki_registry::{DetectorBody, ObjectDetection};
        let tmp = tempdir().unwrap();
        let p = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let r = p
            .register_detector(
                "yolo_v8",
                DetectorBody::ObjectDetection(ObjectDetection {
                    model: "yolo_v8n".to_string(),
                }),
                vec!["object".to_string()],
            )
            .unwrap();
        assert_eq!(r.peer_id, "galbot");
        assert_eq!(r.id, "yolo_v8");
        assert!(!r.hash.is_empty());
    }

    #[test]
    fn register_frame_rejects_invalid_id() {
        let tmp = tempdir().unwrap();
        let p = Peer::new("p", "a").with_storage_root(tmp.path().to_path_buf());
        let result = p.register_frame("bad id", FrameDef::ros_body());
        assert!(matches!(result, Err(crate::SessionError::InvalidId(_))));
    }

    #[test]
    fn register_sensor_rejects_invalid_id() {
        let tmp = tempdir().unwrap();
        let p = Peer::new("p", "a").with_storage_root(tmp.path().to_path_buf());
        let frame = p
            .register_frame("head_left_camera_optical", FrameDef::ros_optical())
            .unwrap();
        let result = p.register_sensor("bad>id", camera_body(frame));
        assert!(matches!(result, Err(crate::SessionError::InvalidId(_))));
    }

    #[test]
    fn register_detector_rejects_invalid_id() {
        use auki_registry::{DetectorBody, ObjectDetection};
        let tmp = tempdir().unwrap();
        let p = Peer::new("p", "a").with_storage_root(tmp.path().to_path_buf());
        let result = p.register_detector(
            "bad>id",
            DetectorBody::ObjectDetection(ObjectDetection {
                model: "yolo_v8n".to_string(),
            }),
            vec![],
        );
        assert!(matches!(result, Err(crate::SessionError::InvalidId(_))));
    }
}
