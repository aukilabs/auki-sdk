//! Session — one timeline / run, born from a [`crate::Peer`].
//!
//! A `Session` is created via [`crate::Peer::start_session`]. It owns the
//! `session_id`, the session clock registry (monotonic + UTC clocks minted at
//! start), and the live logs (sensor · pose · time · detection). Peer-level
//! identity and the sensor / frame / detector registries are read live through
//! the shared [`PeerInner`] handle rather than copied.
//!
//! `Session` has no network dependencies. Authenticated network lifecycle and
//! catalog serving live in `auki-domain`'s `Domain`, which composes a `&Peer`
//! + a `&Session`. See #274 (D1, D2, D3, D7).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use auki_registry::{ClockBody, ClockMeta, ClockRegistryEntry, RegistryRef, Scope};
use parking_lot::RwLock;

use auki_manifests::{
    DetectionLogManifest, MapLogManifest, PoseLogManifest, SensorLogManifest,
    TimeTransformLogManifest,
};
use auki_registry::LogRef;

use crate::error::{Result, SessionError};
use crate::log_handles::{
    DetectionLogHandle, MapLogHandle, MaterializedLogHandle, PoseLogHandle, SensorLogHandle,
    TimeTransformLogHandle,
};
use crate::log_specs::{
    DetectionLogSpec, MapLogSpec, PoseLogSpec, SensorLogSpec, TimeTransformLogSpec,
};
use crate::materialization::MaterializationError;
use crate::peer::PeerInner;
use crate::registry_store::RegistryStore;

pub struct Session {
    /// Shared handle to the originating peer's identity and registries.
    /// Read live so a session never holds a stale copy of peer state.
    pub(crate) peer: Arc<RwLock<PeerInner>>,
    pub(crate) inner: Arc<RwLock<SessionInner>>,
}

pub(crate) struct SessionInner {
    pub(crate) session_id: String,

    /// Session-scoped clock registry. Holds the auto-minted monotonic + UTC
    /// clocks plus any the app registers via [`Session::register_clock`].
    pub(crate) clocks: RegistryStore<ClockRegistryEntry>,

    /// The session's auto-minted monotonic + UTC clocks, set at start.
    pub(crate) monotonic_clock: RegistryRef,
    pub(crate) utc_clock: RegistryRef,

    // Keyed by `resource_id`. Materialization (foreign-source logs) is
    // deferred; until then every log's source is this peer. See #274 (D8).
    pub(crate) sensor_logs: HashMap<String, Arc<SensorLogHandle>>,
    pub(crate) pose_logs: HashMap<String, Arc<PoseLogHandle>>,
    pub(crate) time_logs: HashMap<String, Arc<TimeTransformLogHandle>>,
    pub(crate) detection_logs: HashMap<String, Arc<DetectionLogHandle>>,
    pub(crate) map_logs: HashMap<String, Arc<MapLogHandle>>,
}

/// Write a clock entry to disk and store it in `clocks`, returning its ref.
/// Shared by [`Session::register_clock`] and [`Session::start`] so the
/// session-scoped clock registry has a single registration path.
pub(crate) fn write_and_store_clock(
    storage_root: &Path,
    peer_id: &str,
    session_id: &str,
    clocks: &mut RegistryStore<ClockRegistryEntry>,
    clock_id: &str,
    body: ClockBody,
) -> Result<RegistryRef> {
    ClockRegistryEntry::validate_id(clock_id)?;
    let entry = ClockRegistryEntry {
        peer_id: peer_id.to_string(),
        session_id: session_id.to_string(),
        clock_id: clock_id.to_string(),
        body,
    };
    let hash = entry.hash();
    auki_registry::write_clock(storage_root, &entry)?;
    let registry_ref = RegistryRef {
        peer_id: peer_id.to_string(),
        id: clock_id.to_string(),
        hash,
    };
    clocks.insert(clock_id, entry);
    Ok(registry_ref)
}

impl Session {
    /// Start a fresh session on `peer`: mints a `session_id`, registers the
    /// monotonic + UTC clocks in the corrected shape (#274 D6), and returns a
    /// `Session` that reads peer identity and registries live through `peer`.
    ///
    /// Called by [`crate::Peer::start_session`]; not constructed directly.
    pub(crate) fn start(peer: Arc<RwLock<PeerInner>>) -> Result<Session> {
        let session_id = ulid::Ulid::new().to_string();
        let (peer_id, storage_root) = {
            let p = peer.read();
            (p.peer_id.clone(), p.storage_root.clone())
        };

        let mut clocks = RegistryStore::default();
        let monotonic_clock = write_and_store_clock(
            &storage_root,
            &peer_id,
            &session_id,
            &mut clocks,
            &format!("{peer_id}/{session_id}/monotonic"),
            ClockBody::MonotonicClock(ClockMeta {
                unit: "ns".to_string(),
                monotonic: true,
                epoch: None,
                scope: Scope::DeviceLocal,
            }),
        )?;
        let utc_clock = write_and_store_clock(
            &storage_root,
            &peer_id,
            &session_id,
            &mut clocks,
            &format!("{peer_id}/{session_id}/utc"),
            ClockBody::UtcClock(ClockMeta {
                unit: "ns".to_string(),
                monotonic: false,
                epoch: Some("1970-01-01T00:00:00Z".to_string()),
                scope: Scope::DeviceLocal,
            }),
        )?;

        Ok(Session {
            peer,
            inner: Arc::new(RwLock::new(SessionInner {
                session_id,
                clocks,
                monotonic_clock,
                utc_clock,
                sensor_logs: HashMap::new(),
                pose_logs: HashMap::new(),
                time_logs: HashMap::new(),
                detection_logs: HashMap::new(),
                map_logs: HashMap::new(),
            })),
        })
    }

    /// `(peer_id, app_id, storage_root)` read live from the peer handle.
    fn peer_fields(&self) -> (String, String, PathBuf) {
        let p = self.peer.read();
        (p.peer_id.clone(), p.app_id.clone(), p.storage_root.clone())
    }

    pub fn peer_id(&self) -> String {
        self.peer.read().peer_id.clone()
    }
    pub fn app_id(&self) -> String {
        self.peer.read().app_id.clone()
    }
    pub fn session_id(&self) -> String {
        self.inner.read().session_id.clone()
    }
    pub fn storage_root(&self) -> PathBuf {
        self.peer.read().storage_root.clone()
    }

    /// Resolve an exact local Sensor Registry content address.
    pub fn sensor_registry_entry(
        &self,
        reference: &RegistryRef,
    ) -> Option<auki_registry::SensorRegistryEntry> {
        let peer = self.peer.read();
        if reference.peer_id != peer.peer_id {
            return None;
        }
        let entry = peer.sensors.get(&reference.id)?;
        (entry.hash() == reference.hash).then(|| entry.clone())
    }

    /// The session's auto-minted monotonic clock (set at start).
    pub fn monotonic_clock(&self) -> RegistryRef {
        self.inner.read().monotonic_clock.clone()
    }

    /// The session's auto-minted UTC clock (set at start).
    pub fn utc_clock(&self) -> RegistryRef {
        self.inner.read().utc_clock.clone()
    }

    /// Whether `clock` exactly identifies an entry registered in this Session.
    ///
    /// All three `RegistryRef` fields must match: peer identity, clock id, and
    /// the hash of the stored clock declaration.
    pub fn contains_clock_ref(&self, clock: &RegistryRef) -> bool {
        if clock.peer_id != self.peer_id() {
            return false;
        }
        self.inner
            .read()
            .clocks
            .get(&clock.id)
            .is_some_and(|entry| entry.hash() == clock.hash)
    }

    /// A cheaply-cloneable read handle over this session's live logs, for
    /// `auki-domain` to build the resource catalog without owning the
    /// `Session`. See [`SessionLogs`].
    pub fn logs(&self) -> SessionLogs {
        SessionLogs {
            inner: self.inner.clone(),
        }
    }

    // ─── Registry registration ────────────────────────────────────────────

    /// Register an additional session-scoped clock, writing the entry to disk
    /// and stashing it in the clock registry. The session already mints a
    /// monotonic + UTC clock at start; use this for any extra clocks.
    ///
    /// Validates `clock_id` (rejects `>`, `@`, whitespace) before writing.
    pub fn register_clock(&self, clock_id: &str, body: ClockBody) -> Result<RegistryRef> {
        let (peer_id, _app_id, storage_root) = self.peer_fields();
        let mut inner = self.inner.write();
        let session_id = inner.session_id.clone();
        write_and_store_clock(
            &storage_root,
            &peer_id,
            &session_id,
            &mut inner.clocks,
            clock_id,
            body,
        )
    }

    // ─── Log registration ─────────────────────────────────────────────────

    /// Register a sensor log, writing the manifest to disk and stashing a handle
    /// in the in-memory log map.
    ///
    /// `resource_id` is derived as `spec.sensor.id` (§6). The sensor must be
    /// registered on the peer first. Rejects duplicate `resource_id` with
    /// [`SessionError::DuplicateLog`].
    pub fn register_sensor_log(&self, spec: SensorLogSpec) -> Result<SensorLogHandle> {
        let resource_id = spec.sensor.id.clone();
        let (peer_id, app_id, storage_root, sensor_known) = {
            let p = self.peer.read();
            let known = p.sensors.get(&spec.sensor.id).is_some();
            (
                p.peer_id.clone(),
                p.app_id.clone(),
                p.storage_root.clone(),
                known,
            )
        };

        let mut inner = self.inner.write();
        if inner.sensor_logs.contains_key(&resource_id) || inner.map_logs.contains_key(&resource_id)
        {
            return Err(SessionError::DuplicateLog {
                source_peer_id: peer_id,
                resource_id,
            });
        }
        if !sensor_known {
            // Reuse DuplicateLog as a placeholder; registry lookup errors don't
            // have their own variant yet — acceptable for Phase 4.
            return Err(SessionError::DuplicateLog {
                source_peer_id: peer_id,
                resource_id: format!("sensor not registered: {}", spec.sensor.id),
            });
        }

        let head_spec = spec.head.clone();
        let root = log_root(&storage_root, &inner.session_id, &peer_id, &resource_id);
        let manifest = SensorLogManifest {
            source_peer_id: peer_id.clone(),
            writer_peer_id: peer_id.clone(),
            app_id,
            session_id: inner.session_id.clone(),
            sensor: spec.sensor,
            clock: spec.clock,
            frame: spec.frame,
            segment_duration_ns: spec.segment_duration.as_nanos().min(i64::MAX as u128) as i64,
            retention_ns: spec.retention.as_nanos().min(i64::MAX as u128) as i64,
        };

        write_manifest(&root, &manifest, "SensorLogManifest")?;

        let log_ref = LogRef {
            source_peer_id: peer_id,
            resource_id: resource_id.clone(),
        };
        let handle = SensorLogHandle {
            resource_id: resource_id.clone(),
            log_ref: log_ref.clone(),
            manifest: manifest.clone(),
            head_spec: head_spec.clone(),
            root: root.clone(),
        };
        inner.sensor_logs.insert(
            resource_id.clone(),
            Arc::new(SensorLogHandle {
                resource_id,
                log_ref,
                manifest,
                head_spec,
                root,
            }),
        );
        Ok(handle)
    }

    /// Register a pose log, writing the manifest to disk and stashing a handle
    /// in the in-memory log map.
    ///
    /// `resource_id` is derived as `"<from_frame.id>-><to_frame.id>"` (§6).
    /// Rejects duplicate `resource_id` with [`SessionError::DuplicateLog`].
    pub fn register_pose_log(&self, spec: PoseLogSpec) -> Result<PoseLogHandle> {
        let (peer_id, app_id, storage_root) = self.peer_fields();
        let resource_id = format!("{}->{}", spec.from_frame.id, spec.to_frame.id);
        let mut inner = self.inner.write();
        if inner.pose_logs.contains_key(&resource_id) || inner.map_logs.contains_key(&resource_id) {
            return Err(SessionError::DuplicateLog {
                source_peer_id: peer_id,
                resource_id,
            });
        }

        let head_spec = spec.head.clone();
        let writer_mode = spec.writer_mode;
        let root = log_root(&storage_root, &inner.session_id, &peer_id, &resource_id);
        let manifest = PoseLogManifest {
            source_peer_id: peer_id.clone(),
            writer_peer_id: peer_id.clone(),
            app_id,
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

        write_manifest(&root, &manifest, "PoseLogManifest")?;

        let log_ref = LogRef {
            source_peer_id: peer_id,
            resource_id: resource_id.clone(),
        };
        let handle = PoseLogHandle {
            resource_id: resource_id.clone(),
            log_ref: log_ref.clone(),
            manifest: manifest.clone(),
            head_spec: head_spec.clone(),
            writer_mode,
            root: root.clone(),
        };
        inner.pose_logs.insert(
            resource_id.clone(),
            Arc::new(PoseLogHandle {
                resource_id,
                log_ref,
                manifest,
                head_spec,
                writer_mode,
                root,
            }),
        );
        Ok(handle)
    }

    /// Register a time-transform log, writing the manifest to disk and stashing a handle
    /// in the in-memory log map.
    ///
    /// `resource_id` is derived as `"<from_clock.id>-><to_clock.id>"` (§6).
    /// Rejects duplicate `resource_id` with [`SessionError::DuplicateLog`].
    pub fn register_time_transform_log(
        &self,
        spec: TimeTransformLogSpec,
    ) -> Result<TimeTransformLogHandle> {
        let (peer_id, app_id, storage_root) = self.peer_fields();
        let resource_id = format!("{}->{}", spec.from_clock.id, spec.to_clock.id);
        let mut inner = self.inner.write();
        if inner.time_logs.contains_key(&resource_id) || inner.map_logs.contains_key(&resource_id) {
            return Err(SessionError::DuplicateLog {
                source_peer_id: peer_id,
                resource_id,
            });
        }

        let head_spec = spec.head.clone();
        let root = log_root(&storage_root, &inner.session_id, &peer_id, &resource_id);
        let manifest = TimeTransformLogManifest {
            source_peer_id: peer_id.clone(),
            writer_peer_id: peer_id.clone(),
            app_id,
            session_id: inner.session_id.clone(),
            from_clock: spec.from_clock,
            to_clock: spec.to_clock,
            source: spec.source,
            segment_duration_ns: spec.segment_duration.as_nanos().min(i64::MAX as u128) as i64,
            retention_ns: spec.retention.as_nanos().min(i64::MAX as u128) as i64,
        };

        write_manifest(&root, &manifest, "TimeTransformLogManifest")?;

        let log_ref = LogRef {
            source_peer_id: peer_id,
            resource_id: resource_id.clone(),
        };
        let handle = TimeTransformLogHandle {
            resource_id: resource_id.clone(),
            log_ref: log_ref.clone(),
            manifest: manifest.clone(),
            head_spec: head_spec.clone(),
            root: root.clone(),
        };
        inner.time_logs.insert(
            resource_id.clone(),
            Arc::new(TimeTransformLogHandle {
                resource_id,
                log_ref,
                manifest,
                head_spec,
                root,
            }),
        );
        Ok(handle)
    }

    /// Register a detection log, writing the manifest to disk and stashing a handle
    /// in the in-memory log map.
    ///
    /// `resource_id` is the application-selected Detector `instance_id`.
    /// Rejects duplicate `resource_id` with [`SessionError::DuplicateLog`].
    pub fn register_detection_log(&self, spec: DetectionLogSpec) -> Result<DetectionLogHandle> {
        let (peer_id, app_id, storage_root) = self.peer_fields();
        let resource_id = spec.instance_id.clone();
        let mut inner = self.inner.write();
        if inner.detection_logs.contains_key(&resource_id)
            || inner.map_logs.contains_key(&resource_id)
        {
            return Err(SessionError::DuplicateLog {
                source_peer_id: peer_id,
                resource_id,
            });
        }

        let head_spec = spec.head.clone();
        let root = log_root(&storage_root, &inner.session_id, &peer_id, &resource_id);
        let manifest = DetectionLogManifest {
            source_peer_id: peer_id.clone(),
            writer_peer_id: peer_id.clone(),
            app_id,
            session_id: inner.session_id.clone(),
            instance_id: spec.instance_id,
            detector: spec.detector,
            input_log: spec.input_log,
            input_sensor: spec.input_sensor,
            clock: spec.clock,
            cadence: spec.cadence,
            segment_duration_ns: spec.segment_duration.as_nanos().min(i64::MAX as u128) as i64,
            retention_ns: spec.retention.as_nanos().min(i64::MAX as u128) as i64,
        };
        manifest.validate()?;

        write_manifest(&root, &manifest, "DetectionLogManifest")?;
        let writer = Arc::new(parking_lot::Mutex::new(auki_logs::Log::open(
            &root,
            serde_json::to_value(&manifest)
                .unwrap_or_else(|_| panic!("DetectionLogManifest serializes")),
        )?));
        let (entries, _) = tokio::sync::broadcast::channel(1024);

        let log_ref = LogRef {
            source_peer_id: peer_id,
            resource_id: resource_id.clone(),
        };
        let handle = DetectionLogHandle::with_writer(
            resource_id.clone(),
            log_ref.clone(),
            manifest.clone(),
            head_spec.clone(),
            root.clone(),
            writer.clone(),
            entries.clone(),
        );
        inner.detection_logs.insert(
            resource_id.clone(),
            Arc::new(DetectionLogHandle::with_writer(
                resource_id,
                log_ref,
                manifest,
                head_spec,
                root,
                writer,
                entries,
            )),
        );
        Ok(handle)
    }

    /// Register the append-only update log for a locally-owned Map resource.
    /// The map ID is the stable resource ID, which keeps map discovery and
    /// remote materialization peer-agnostic.
    pub fn register_map_log(&self, spec: MapLogSpec) -> Result<MapLogHandle> {
        let resource_id = spec.map.id.clone();
        let clock_known = self.contains_clock_ref(&spec.clock);
        let (peer_id, app_id, storage_root, map_known) = {
            let p = self.peer.read();
            (
                p.peer_id.clone(),
                p.app_id.clone(),
                p.storage_root.clone(),
                p.maps.get(&spec.map.id).is_some_and(|entry| {
                    spec.map.peer_id == p.peer_id
                        && spec.map.id == entry.map_id
                        && spec.map.hash == entry.hash()
                }),
            )
        };
        if !map_known {
            return Err(SessionError::MapNotRegistered {
                peer_id: spec.map.peer_id,
                map_id: spec.map.id,
                map_hash: spec.map.hash,
            });
        }
        if !clock_known {
            return Err(SessionError::ClockNotRegistered {
                peer_id: spec.clock.peer_id,
                clock_id: spec.clock.id,
                clock_hash: spec.clock.hash,
            });
        }
        let mut inner = self.inner.write();
        if inner.map_logs.contains_key(&resource_id)
            || inner.sensor_logs.contains_key(&resource_id)
            || inner.pose_logs.contains_key(&resource_id)
            || inner.time_logs.contains_key(&resource_id)
            || inner.detection_logs.contains_key(&resource_id)
        {
            return Err(SessionError::DuplicateLog {
                source_peer_id: peer_id,
                resource_id,
            });
        }
        let head_spec = spec.head.clone();
        let manifest = MapLogManifest {
            source_peer_id: peer_id.clone(),
            writer_peer_id: peer_id.clone(),
            app_id,
            session_id: inner.session_id.clone(),
            map: spec.map,
            clock: spec.clock,
            segment_duration_ns: spec.segment_duration.as_nanos().min(i64::MAX as u128) as i64,
            retention_ns: spec.retention.as_nanos().min(i64::MAX as u128) as i64,
        };
        let root = storage_root.join("logs").join(&peer_id).join(&resource_id);
        let writer = Arc::new(parking_lot::Mutex::new(auki_logs::Log::open(
            &root,
            serde_json::to_value(&manifest).unwrap_or_else(|_| panic!("MapLogManifest serializes")),
        )?));
        let (updates, _) = tokio::sync::broadcast::channel(1024);
        let log_ref = LogRef {
            source_peer_id: peer_id,
            resource_id: resource_id.clone(),
        };
        let handle = MapLogHandle::with_writer(
            resource_id.clone(),
            log_ref.clone(),
            manifest.clone(),
            head_spec.clone(),
            root.clone(),
            writer.clone(),
            updates.clone(),
        );
        inner.map_logs.insert(
            resource_id.clone(),
            Arc::new(MapLogHandle::with_writer(
                resource_id,
                log_ref,
                manifest,
                head_spec,
                root,
                writer,
                updates,
            )),
        );
        Ok(handle)
    }

    // ─── Materialization ─────────────────────────────────────────────────

    /// Open a remote peer's log locally by fetching its catalog row,
    /// connecting to the serving peer, and ingesting samples.
    ///
    /// Full implementation: fetch remote catalog row, extract canonical
    /// fields, open `/auki/auth/1/stream/0.2.0` against the serving peer, write
    /// new local manifest, ingest samples. Deferred to a follow-up plan
    /// (Phase 5).
    pub async fn materialize_remote_log(
        &self,
        _log_ref: LogRef,
        _retention: Duration,
        _segment_duration: Duration,
    ) -> Result<MaterializedLogHandle> {
        Err(SessionError::Materialization(
            MaterializationError::NotImplemented,
        ))
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
        Err(SessionError::Materialization(
            MaterializationError::NotImplemented,
        ))
    }
}

/// A cheaply-cloneable read handle over a [`Session`]'s live logs.
///
/// Obtained via [`Session::logs`]. `auki-domain` holds one to serve the
/// resource catalog on inbound `/auki/auth/1/resources/*` requests without owning
/// the `Session`. Each accessor takes a brief read lock and returns a
/// snapshot of the current handles.
#[derive(Clone)]
pub struct SessionLogs {
    inner: Arc<RwLock<SessionInner>>,
}

impl SessionLogs {
    pub fn sensor_logs(&self) -> Vec<Arc<SensorLogHandle>> {
        self.inner.read().sensor_logs.values().cloned().collect()
    }
    pub fn pose_logs(&self) -> Vec<Arc<PoseLogHandle>> {
        self.inner.read().pose_logs.values().cloned().collect()
    }
    pub fn time_logs(&self) -> Vec<Arc<TimeTransformLogHandle>> {
        self.inner.read().time_logs.values().cloned().collect()
    }
    pub fn detection_logs(&self) -> Vec<Arc<DetectionLogHandle>> {
        self.inner.read().detection_logs.values().cloned().collect()
    }
    pub fn map_logs(&self) -> Vec<Arc<MapLogHandle>> {
        self.inner.read().map_logs.values().cloned().collect()
    }
}

/// Canonicalize `manifest` and write it to
/// `<storage_root>/<session_id>/logs/<peer_id>/<resource_id>/manifest.json`.
fn write_manifest<M: serde::Serialize>(root: &Path, manifest: &M, type_name: &str) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let value = serde_json::to_value(manifest).unwrap_or_else(|_| panic!("{type_name} serializes"));
    let canonical_bytes = auki_jcs::canonicalize(&value);
    std::fs::write(root.join("manifest.json"), &canonical_bytes)?;
    Ok(())
}

fn log_root(storage_root: &Path, session_id: &str, peer_id: &str, resource_id: &str) -> PathBuf {
    storage_root
        .join(session_id)
        .join("logs")
        .join(peer_id)
        .join(resource_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peer;
    use tempfile::tempdir;

    fn started() -> (Session, tempfile::TempDir) {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "galbot-ctrl").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        (session, tmp)
    }

    #[test]
    fn start_session_carries_peer_app_and_generates_session_id() {
        let (s, _tmp) = started();
        assert_eq!(s.peer_id(), "galbot");
        assert_eq!(s.app_id(), "galbot-ctrl");
        assert!(!s.session_id().is_empty());
        // ULIDs are 26 chars
        assert_eq!(s.session_id().len(), 26);
    }

    #[test]
    fn session_id_unique_per_session() {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("p", "a").with_storage_root(tmp.path().to_path_buf());
        let a = peer.start_session().unwrap();
        let b = peer.start_session().unwrap();
        assert_ne!(a.session_id(), b.session_id());
    }

    #[test]
    fn storage_root_reads_through_peer() {
        let (s, tmp) = started();
        assert_eq!(s.storage_root(), tmp.path());
    }

    #[test]
    fn start_session_mints_session_and_registers_both_clocks() {
        let (s, tmp) = started();
        assert_eq!(s.session_id().len(), 26, "session_id should be a ULID");
        let mono = s.monotonic_clock();
        let utc = s.utc_clock();
        assert!(mono.id.ends_with("/monotonic"), "mono id: {}", mono.id);
        assert!(utc.id.ends_with("/utc"), "utc id: {}", utc.id);
        // Both clock entries written to disk under the peer's storage root.
        assert!(
            tmp.path().join("registries/clocks/galbot").exists(),
            "clock entries not registered on disk"
        );
    }

    #[test]
    fn contains_clock_ref_requires_exact_registered_peer_id_and_hash() {
        let (session, _tmp) = started();
        let exact = session.monotonic_clock();
        assert!(session.contains_clock_ref(&exact));

        let mut unknown = exact.clone();
        unknown.id.push_str("/unknown");
        assert!(!session.contains_clock_ref(&unknown));

        let mut wrong_peer = exact.clone();
        wrong_peer.peer_id = "other-peer".into();
        assert!(!session.contains_clock_ref(&wrong_peer));

        let mut wrong_hash = exact;
        wrong_hash.hash = "wrong-hash".into();
        assert!(!session.contains_clock_ref(&wrong_hash));
    }
}

#[cfg(test)]
mod register_clock_tests {
    use super::*;
    use crate::{Peer, SessionError};
    use auki_registry::{ClockMeta, Scope};
    use tempfile::tempdir;

    fn started() -> (Session, tempfile::TempDir) {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        (peer.start_session().unwrap(), tmp)
    }

    #[test]
    fn register_clock_returns_registry_ref_with_self_peer_id() {
        let (s, _tmp) = started();
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
        let (s, _tmp) = started();
        let body = ClockBody::MonotonicClock(ClockMeta {
            unit: "ns".to_string(),
            monotonic: true,
            epoch: None,
            scope: Scope::DeviceLocal,
        });
        let result = s.register_clock("bad@id", body);
        assert!(matches!(result, Err(SessionError::InvalidId(_))));
    }
}

#[cfg(test)]
mod register_log_tests {
    use super::*;
    use crate::log_specs::{
        DetectionLogSpec, HeadSpec, PoseLogSpec, SensorLogSpec, TimeTransformLogSpec,
    };
    use crate::{FrameDef, Peer};
    use auki_manifests::{PoseSource, PoseWriterMode, TimeTransformSource};
    use auki_registry::{
        Camera, ClockMeta, DetectorBody, LogRef, ObjectDetection, Scope, SensorBody,
    };
    use std::time::Duration;
    use tempfile::tempdir;

    fn fixture() -> (Peer, Session, tempfile::TempDir) {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        (peer, session, tmp)
    }

    fn camera_body(frame: RegistryRef) -> SensorBody {
        SensorBody::Camera(Camera {
            r#type: "rgb".to_string(),
            width: 1920,
            height: 1200,
            frame_rate_hz: 30,
            image_encoding: "raw".to_string(),
            pixel_format: "rgb8".to_string(),
            row_stride_bytes: 1920 * 3,
            color_space: "srgb".to_string(),
            intrinsics_model: "pinhole".to_string(),
            distortion_model: "brown_conrady".to_string(),
            calibration: None,
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

    fn fixture_registries(peer: &Peer, s: &Session) -> (RegistryRef, RegistryRef, RegistryRef) {
        let frame = peer
            .register_frame("head_left_camera_optical", FrameDef::ros_optical())
            .unwrap();
        let sensor = peer
            .register_sensor("head_left_rgb", camera_body(frame.clone()))
            .unwrap();
        let clock = s
            .register_clock("session/sdk_clock", monotonic_clock_body())
            .unwrap();
        (sensor, clock, frame)
    }

    #[test]
    fn register_sensor_log_resource_id_is_sensor_id() {
        let (peer, s, _tmp) = fixture();
        let (sensor, clock, frame) = fixture_registries(&peer, &s);

        let handle = s
            .register_sensor_log(SensorLogSpec {
                sensor: sensor.clone(),
                clock,
                frame: Some(frame),
                head: HeadSpec::Rolling {
                    retention_ns: 5_000_000_000,
                },
                segment_duration: Duration::from_secs(1),
                retention: Duration::from_secs(5),
            })
            .unwrap();

        assert_eq!(handle.resource_id(), "head_left_rgb");
        assert_eq!(handle.log_ref().source_peer_id, "galbot");
        assert_eq!(handle.log_ref().resource_id, "head_left_rgb");

        let manifest_path = handle.root().join("manifest.json");
        assert!(
            manifest_path.exists(),
            "manifest.json missing at {}",
            manifest_path.display()
        );
    }

    #[test]
    fn register_sensor_log_rejects_unregistered_sensor() {
        let (_peer, s, _tmp) = fixture();
        // sensor never registered on the peer
        let bogus = RegistryRef {
            peer_id: "galbot".to_string(),
            id: "ghost_cam".to_string(),
            hash: "deadbeef".to_string(),
        };
        let clock = s
            .register_clock("session/sdk_clock", monotonic_clock_body())
            .unwrap();
        let result = s.register_sensor_log(SensorLogSpec {
            sensor: bogus,
            clock,
            frame: None,
            head: HeadSpec::Rolling {
                retention_ns: 5_000_000_000,
            },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        });
        assert!(matches!(result, Err(SessionError::DuplicateLog { .. })));
    }

    #[test]
    fn register_sensor_log_rejects_duplicate() {
        let (peer, s, _tmp) = fixture();
        let (sensor, clock, frame) = fixture_registries(&peer, &s);
        let spec = SensorLogSpec {
            sensor: sensor.clone(),
            clock: clock.clone(),
            frame: Some(frame.clone()),
            head: HeadSpec::Rolling {
                retention_ns: 5_000_000_000,
            },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        };
        s.register_sensor_log(spec.clone()).unwrap();
        let result = s.register_sensor_log(spec);
        assert!(matches!(result, Err(SessionError::DuplicateLog { .. })));
    }

    #[test]
    fn register_pose_log_resource_id_is_from_arrow_to() {
        let (peer, s, _tmp) = fixture();
        let (_sensor, clock, _frame) = fixture_registries(&peer, &s);
        let world = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let base_link = peer
            .register_frame("base_link", FrameDef::ros_body())
            .unwrap();

        let handle = s
            .register_pose_log(PoseLogSpec {
                from_frame: world,
                to_frame: base_link,
                clock,
                source: PoseSource::Manual,
                writer_mode: PoseWriterMode::Movable,
                expected_rate_hz: 30,
                head: HeadSpec::Rolling {
                    retention_ns: 5_000_000_000,
                },
                segment_duration: Duration::from_secs(1),
                retention: Duration::from_secs(5),
            })
            .unwrap();

        assert_eq!(handle.resource_id(), "world->base_link");
    }

    #[test]
    fn register_time_transform_log_resource_id_format() {
        let (_peer, s, _tmp) = fixture();
        let clock = s
            .register_clock("session/sdk_clock", monotonic_clock_body())
            .unwrap();
        let wall = s.register_clock("wall_clock", utc_clock_body()).unwrap();

        let handle = s
            .register_time_transform_log(TimeTransformLogSpec {
                from_clock: clock,
                to_clock: wall,
                source: TimeTransformSource::LocalClockRead,
                head: HeadSpec::Rolling {
                    retention_ns: 60_000_000_000,
                },
                segment_duration: Duration::from_secs(60),
                retention: Duration::from_secs(3600),
            })
            .unwrap();

        assert_eq!(handle.resource_id(), "session/sdk_clock->wall_clock");
    }

    #[test]
    fn register_detection_log_resource_id_format() {
        let (peer, s, _tmp) = fixture();
        let (sensor, clock, _frame) = fixture_registries(&peer, &s);
        let detector = peer
            .register_detector(
                "yolo_v8",
                DetectorBody::ObjectDetection(ObjectDetection {
                    model: "yolo_v8n".to_string(),
                }),
                vec!["bounding_box".to_string()],
            )
            .unwrap();
        let input_log = LogRef {
            source_peer_id: "galbot".to_string(),
            resource_id: "head_left_rgb".to_string(),
        };
        let handle = s
            .register_detection_log(DetectionLogSpec {
                instance_id: "yolo-head-left".into(),
                detector,
                input_log,
                input_sensor: sensor,
                clock,
                cadence: auki_manifests::DetectionCadence::EveryFrame,
                head: HeadSpec::Rolling {
                    retention_ns: 5_000_000_000,
                },
                segment_duration: Duration::from_secs(1),
                retention: Duration::from_secs(5),
            })
            .unwrap();

        assert_eq!(handle.resource_id(), "yolo-head-left");
    }
}

#[cfg(test)]
mod session_logs_tests {
    use super::*;
    use crate::log_specs::{HeadSpec, SensorLogSpec};
    use crate::materialization::MaterializationError;
    use crate::{FrameDef, Peer};
    use auki_registry::{Camera, LogRef, SensorBody};
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn logs_view_reflects_registered_sensor_log() {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let s = peer.start_session().unwrap();
        let frame = peer
            .register_frame("head_left_camera_optical", FrameDef::ros_optical())
            .unwrap();
        let sensor = peer
            .register_sensor(
                "head_left_rgb",
                SensorBody::Camera(Camera {
                    r#type: "rgb".to_string(),
                    width: 1920,
                    height: 1200,
                    frame_rate_hz: 30,
                    image_encoding: "raw".to_string(),
                    pixel_format: "rgb8".to_string(),
                    row_stride_bytes: 1920 * 3,
                    color_space: "srgb".to_string(),
                    intrinsics_model: "pinhole".to_string(),
                    distortion_model: "brown_conrady".to_string(),
                    calibration: None,
                    frame: frame.clone(),
                }),
            )
            .unwrap();
        let clock = s.monotonic_clock();

        let logs = s.logs();
        assert_eq!(logs.sensor_logs().len(), 0);

        s.register_sensor_log(SensorLogSpec {
            sensor,
            clock,
            frame: Some(frame),
            head: HeadSpec::Rolling {
                retention_ns: 5_000_000_000,
            },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        })
        .unwrap();

        let sensor_logs = logs.sensor_logs();
        assert_eq!(sensor_logs.len(), 1);
        assert_eq!(sensor_logs[0].resource_id(), "head_left_rgb");
        assert_eq!(sensor_logs[0].manifest.source_peer_id, "galbot");
    }

    #[test]
    fn logs_view_reflects_registered_map_log() {
        use crate::MapLogSpec;
        use auki_registry::{FiniteF64, MapBody, VoxelMap, VoxelValueModel};
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let map = peer
            .register_map(
                "occupancy",
                MapBody::Voxel(VoxelMap {
                    frame,
                    voxel_size_m: FiniteF64(0.05),
                    chunk_dimension: 64,
                    value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                    color_model: None,
                    semantic_classes: vec![],
                }),
            )
            .unwrap();
        let handle = session
            .register_map_log(MapLogSpec {
                map: map.clone(),
                clock: session.monotonic_clock(),
                head: HeadSpec::Fixed,
                segment_duration: Duration::from_secs(1),
                retention: Duration::ZERO,
            })
            .unwrap();
        handle
            .append(
                42,
                &auki_datatypes::map::MapUpdate {
                    voxel_chunks: vec![auki_datatypes::map::VoxelChunkUpdate {
                        chunk_x: -1,
                        chunk_y: 0,
                        chunk_z: 2,
                        voxels: vec![auki_datatypes::map::VoxelDelta {
                            x: 3,
                            y: 4,
                            z: 5,
                            occupancy_delta: 0.8,
                            semantics: vec![],
                            color: None,
                        }],
                    }],
                    checkpoint: None,
                },
            )
            .unwrap();
        assert_eq!(handle.entries().unwrap().len(), 1);
        assert!(handle.persisted_bytes().unwrap() > 0);
        assert_eq!(handle.resource_id(), "occupancy");
        assert_eq!(session.logs().map_logs().len(), 1);
    }

    #[test]
    fn register_map_log_rejects_a_stale_map_registry_reference() {
        use crate::MapLogSpec;
        use auki_registry::{FiniteF64, MapBody, VoxelMap, VoxelValueModel};
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let mut map = peer
            .register_map(
                "occupancy",
                MapBody::Voxel(VoxelMap {
                    frame,
                    voxel_size_m: FiniteF64(0.05),
                    chunk_dimension: 64,
                    value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                    color_model: None,
                    semantic_classes: vec![],
                }),
            )
            .unwrap();
        map.hash = "stale".into();
        assert!(matches!(
            session.register_map_log(MapLogSpec {
                map,
                clock: session.monotonic_clock(),
                head: HeadSpec::Fixed,
                segment_duration: Duration::from_secs(1),
                retention: Duration::ZERO,
            }),
            Err(SessionError::MapNotRegistered { .. })
        ));
    }

    #[test]
    fn voxelizer_update_persists_and_replays_through_a_map_log() {
        use crate::MapLogSpec;
        use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
        use auki_mappers::Voxelizer;
        use auki_registry::{FiniteF64, MapBody, VoxelMap, VoxelValueModel};
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let map_contract = VoxelMap {
            frame,
            voxel_size_m: FiniteF64(1.0),
            chunk_dimension: 64,
            value_model: VoxelValueModel::AdditiveOccupancyEvidence,
            color_model: None,
            semantic_classes: vec![],
        };
        let map = peer
            .register_map("occupancy", MapBody::Voxel(map_contract.clone()))
            .unwrap();
        let log = session
            .register_map_log(MapLogSpec {
                map: map.clone(),
                clock: session.monotonic_clock(),
                head: HeadSpec::Fixed,
                segment_duration: Duration::from_secs(1),
                retention: Duration::ZERO,
            })
            .unwrap();
        let pose = SpatialTransform {
            translation: Some(Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            orientation: Some(Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            }),
        };
        let update = Voxelizer::new(1.0, 64)
            .unwrap()
            .map_sensor_rays(
                [Vec3 {
                    x: 2.0,
                    y: 0.0,
                    z: 0.0,
                }],
                &pose,
                -0.2,
                0.8,
            )
            .unwrap();
        log.append(100, &update).unwrap();
        let entries = log.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].payload.voxel_chunks[0]
                .voxels
                .iter()
                .any(|v| v.occupancy_delta > 0.0)
        );
        assert!(
            entries[0].payload.voxel_chunks[0]
                .voxels
                .iter()
                .any(|v| v.occupancy_delta < 0.0)
        );

        let mut accumulated =
            auki_maps::VoxelMapAccumulator::new(map.clone(), map_contract).unwrap();
        let mut applied = None;
        for entry in entries {
            applied = Some(accumulated.apply(&entry.payload).unwrap());
        }
        let viewer = accumulated.viewer_snapshot(0.0).unwrap();
        assert_eq!(viewer.map, map);
        assert_eq!(viewer.frame.id, "world");
        assert_eq!(viewer.chunks.len(), 1);
        assert_eq!(viewer.chunks[0].voxels[0].center_m, [2.5, 0.5, 0.5]);
        let adapter = auki_maps::VoxelViewerAdapter::new(Default::default()).unwrap();
        let render_updates = adapter
            .changed_chunks(&accumulated, &applied.unwrap())
            .unwrap();
        let auki_maps::ChunkRenderUpdate::Replace { instances, .. } = &render_updates[0] else {
            panic!("occupied Galbot voxel must produce a chunk instance buffer")
        };
        assert_eq!(instances[0].center_m, [2.5, 0.5, 0.5]);
        assert_eq!(instances[0].edge_length_m, 0.92);
    }

    #[tokio::test]
    async fn materialize_remote_log_surface_returns_not_implemented() {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "ctrl").with_storage_root(tmp.path().to_path_buf());
        let s = peer.start_session().unwrap();
        let result = s
            .materialize_remote_log(
                LogRef {
                    source_peer_id: "galbot".to_string(),
                    resource_id: "head_left_rgb".to_string(),
                },
                Duration::from_secs(300),
                Duration::from_secs(10),
            )
            .await;
        assert!(matches!(
            result,
            Err(SessionError::Materialization(
                MaterializationError::NotImplemented
            ))
        ));
    }

    #[tokio::test]
    async fn resolve_static_transform_surface_returns_not_implemented() {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("park", "vis").with_storage_root(tmp.path().to_path_buf());
        let s = peer.start_session().unwrap();
        let result = s
            .resolve_static_transform(LogRef {
                source_peer_id: "park".to_string(),
                resource_id: "world->base_link".to_string(),
            })
            .await;
        assert!(matches!(
            result,
            Err(SessionError::Materialization(
                MaterializationError::NotImplemented
            ))
        ));
    }
}
