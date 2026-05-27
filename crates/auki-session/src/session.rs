//! Session — per-process declarative API.
//!
//! See `crate::Session` (and `docs/superpowers/specs/.../#section-§4`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use auki_registry::{SensorRegistryEntry, ClockRegistryEntry, FrameRegistryEntry, DetectorRegistryEntry};

use crate::registry_store::RegistryStore;
use crate::log_handles::{
    SensorLogHandle, PoseLogHandle, TimeTransformLogHandle, DetectionLogHandle,
};

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
