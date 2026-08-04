//! Python bindings for `auki-domain` — post-#216 schema.
//!
//! Surface (under the `auki_domain` Python module):
//!
//! - `ClusterMembership` / `ClusterMember` — value-type pyclasses
//!   mirroring the Rust types.
//! - `DaemonInfo` — value-type pyclass the daemon constructs and
//!   passes to `ClusterManager.participant_info`.
//! - `ParticipantInfo` — the SDK-provided identity wire shape,
//!   exchanged peer-to-peer over libp2p `/auki/info/0.0.1` (#293);
//!   produced by `ClusterManager.participant_info`. Has a
//!   `.to_json()` method for app-local operator/debug surfaces.
//! - `ResourceEntry` — post-#216 flat resource catalog row with
//!   `variant_content` discriminator (`sensor_log` | `pose_log` |
//!   `time_transform_log` | `detection_log`).
//! - `ReadFrom` / `StreamRequest` — post-#216 §5 stream subscription
//!   request types.
//! - `MessageEvent` / `MessageChannelReceiver` — live, receiver-owned typed
//!   messaging from Resource Catalog v0.3.
//! - `StreamManifestBuilder` — producer-side helper.
//! - `ClusterTarget` / `ClusterManager` — daemon-side cluster handle.

use auki_domain_rs::cluster_manager::ManagerRelayReservation as RustManagerRelayReservation;
use auki_domain_rs::{
    AdmitError as RustAdmitError, BootstrapError as RustBootstrapError,
    BuildStreamManifestError as RustBuildStreamManifestError, ClusterManager as RustClusterManager,
    ClusterMember as RustClusterMember, ClusterMembership as RustClusterMembership,
    ClusterTarget as RustClusterTarget, CreateClusterError as RustCreateClusterError,
    DaemonInfo as RustDaemonInfo, FetchMapCatalogError as RustFetchMapCatalogError,
    FetchRegistryEntryError as RustFetchRegistryEntryError,
    FetchResourcesCatalogError as RustFetchResourcesCatalogError,
    JoinClusterError as RustJoinClusterError, MapLogResource as RustMapLogResource,
    MessageChannelResource as RustMessageChannelResource,
    ResourceCatalogProvider as RustResourceCatalogProvider, ResourceEntry as RustResourceEntry,
    StreamManifestBuilder as RustStreamManifestBuilder,
};
use auki_identity::Wallet;
use auki_network::ParticipantInfo as RustParticipantInfo;
use auki_network::PeerIdentity;
use auki_network::discovery_client::DiscoveryError as RustDiscoveryError;
use auki_network::resources_protocol::{
    ResourcesRequest as RustResourcesRequest, Variant as RustVariant,
};
use auki_network::stream_protocol::{
    CameraFrame as RustCameraFrame, ReadFrom as RustReadFrom, StreamRequest as RustStreamRequest,
    audio::Data as RustAudioFrame, joint_encoders::Data as RustJointEncodersFrame,
    map::MapUpdate as RustMapUpdate, point_cloud::Data as RustPointCloudFrame,
    pose::SpatialTransform as RustPoseSpatialTransform,
};
use auki_network::stream_runtime::{StreamProvider, decline_all_streams};
use auki_network::swarm::{SwarmConfig, build_swarm};
use auki_network::{
    MapCatalogProvider as RustMapCatalogProvider,
    MessageChannelRegistration as RustMessageChannelRegistration,
};
use auki_network_py::PyClusterEntry;
use auki_network_py::stream_types::{
    PyStreamSubscription, STREAM_PROVIDER_CAPSULE_NAME, open_stream_error_to_pyerr,
};
use auki_registry::RegistryRef as RustRegistryRef;
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use pyo3::exceptions::{PyFileNotFoundError, PyOSError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyCapsule, PyDict};
use std::ffi::CString;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::runtime::Runtime;

// ─── Generic stream payload kind ───────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenericStreamPayloadKind {
    Camera,
    PointCloud,
    JointEncoders,
    Audio,
    Pose,
}

/// Resolve the stream payload kind for `resource_id` from the post-#216
/// flat `ResourceEntry` catalog. Matches `sensor_log` rows by looking at
/// the `sensor.kind` + `manifest` (sensor type), and `pose_log` rows.
/// When `source_peer_id` is present, the full `(source_peer_id, resource_id)`
/// identity is matched; an empty `source_peer_id` preserves the legacy
/// resource-id-only open path.
fn resolve_generic_stream_payload_kind_from_entries(
    resources: &[RustResourceEntry],
    source_peer_id: &str,
    resource_id: &str,
) -> Result<GenericStreamPayloadKind, PyErr> {
    use auki_network::resources_protocol::{SensorKind, VariantContent};
    for entry in resources {
        if entry.resource_id != resource_id {
            continue;
        }
        if !source_peer_id.is_empty() && entry.source_peer_id != source_peer_id {
            continue;
        }
        match &entry.variant_content {
            VariantContent::SensorLog { .. } => {
                if let Some(sensor) = &entry.sensor {
                    let kind = match sensor.kind {
                        SensorKind::Camera => GenericStreamPayloadKind::Camera,
                        SensorKind::Audio => GenericStreamPayloadKind::Audio,
                        SensorKind::JointEncoders => GenericStreamPayloadKind::JointEncoders,
                        // rangefinder/rf map to PointCloud as the closest stream type
                        SensorKind::Rangefinder | SensorKind::Rf => {
                            GenericStreamPayloadKind::PointCloud
                        }
                    };
                    return Ok(kind);
                }
                return Err(PyValueError::new_err(format!(
                    "sensor_log resource {resource_id:?} has no sensor block — cannot resolve stream payload"
                )));
            }
            VariantContent::PoseLog { .. } => {
                return Ok(GenericStreamPayloadKind::Pose);
            }
            _ => {
                return Err(PyValueError::new_err(format!(
                    "resource {resource_id:?} has variant {:?} which does not carry a live stream",
                    variant_tag(&entry.variant_content),
                )));
            }
        }
    }
    let scope = if source_peer_id.is_empty() {
        format!("resource {resource_id:?}")
    } else {
        format!("resource {source_peer_id:?}/{resource_id:?}")
    };
    Err(PyFileNotFoundError::new_err(format!(
        "{scope} not found in remote resource catalog"
    )))
}

fn variant_tag(vc: &auki_network::resources_protocol::VariantContent) -> &'static str {
    use auki_network::resources_protocol::VariantContent;
    match vc {
        VariantContent::SensorLog { .. } => "sensor_log",
        VariantContent::PoseLog { .. } => "pose_log",
        VariantContent::TimeTransformLog { .. } => "time_transform_log",
        VariantContent::DetectionLog { .. } => "detection_log",
    }
}

// ─── Stream provider bridge ─────────────────────────────────────────
//
// Same capsule-bridge pattern as v0.0.52: the stream provider callable
// is built inside `auki_network.so` (where the type-ids are correct)
// and shipped to us via a `PyCapsule`.

fn stream_provider_from_python(py: Python<'_>, callable: Py<PyAny>) -> PyResult<StreamProvider> {
    let cluster = py.import_bound("auki_network")?.getattr("cluster")?;
    let builder = cluster.getattr("_build_stream_provider")?;
    let result = builder.call1((callable,))?;
    let capsule = result.downcast::<PyCapsule>().map_err(|e| {
        PyRuntimeError::new_err(format!(
            "auki_network.cluster._build_stream_provider returned non-PyCapsule: {e}"
        ))
    })?;
    let expected_name =
        CString::new(STREAM_PROVIDER_CAPSULE_NAME).expect("static literal contains no nul");
    match capsule.name()? {
        Some(name) if name == expected_name.as_c_str() => {}
        Some(other) => {
            return Err(PyRuntimeError::new_err(format!(
                "stream-provider capsule has unexpected name {other:?} (want {STREAM_PROVIDER_CAPSULE_NAME:?})"
            )));
        }
        None => {
            return Err(PyRuntimeError::new_err(
                "stream-provider capsule has no name; rejecting (defense against misrouted capsules)",
            ));
        }
    }
    // SAFETY: verified capsule name; by contract the payload is a `StreamProvider`
    // (which is `Arc<dyn Fn>` and memory-layout-stable across crate boundaries
    // within this process). Both crates share the same `auki_network` rlib version.
    let provider_ref: &StreamProvider = unsafe { capsule.reference::<StreamProvider>() };
    Ok(Arc::clone(provider_ref))
}

fn stream_manifest_to_python(
    py: Python<'_>,
    manifest: auki_network::stream_protocol::StreamManifest,
) -> PyResult<Py<PyAny>> {
    let cluster = py.import_bound("auki_network")?.getattr("cluster")?;
    let cls = cluster.getattr("StreamManifest")?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("sensor_id", manifest.sensor_id)?;
    kwargs.set_item("sensor_hash", manifest.sensor_hash)?;
    kwargs.set_item("clock_id", manifest.clock_id)?;
    kwargs.set_item("clock_hash", manifest.clock_hash)?;
    kwargs.set_item("frame_id", manifest.frame_id)?;
    kwargs.set_item("frame_hash", manifest.frame_hash)?;
    Ok(cls.call((), Some(&kwargs))?.unbind())
}

fn pathlike_to_pathbuf(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &'static str,
) -> PyResult<PathBuf> {
    let path_obj = py.import_bound("os")?.call_method1("fspath", (value,))?;
    let path: String = path_obj.extract().map_err(|_| {
        PyTypeError::new_err(format!(
            "{name} must be str or os.PathLike resolving to str"
        ))
    })?;
    Ok(PathBuf::from(path))
}

// ─── Process-wide tokio runtime ────────────────────────────────────

fn shared_runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("tokio runtime starts"))
}

// ─── Live message channel pyclasses ────────────────────────────────────────

/// One live opaque application message delivered by an authenticated peer.
#[pyclass(name = "MessageEvent")]
pub struct PyMessageEvent {
    resource_id: String,
    sender_peer_id: String,
    message_type: String,
    timestamp_ns: i64,
    payload: Vec<u8>,
}

#[pymethods]
impl PyMessageEvent {
    #[getter]
    fn resource_id(&self) -> &str {
        &self.resource_id
    }

    #[getter]
    fn sender_peer_id(&self) -> &str {
        &self.sender_peer_id
    }

    #[getter(r#type)]
    fn message_type(&self) -> &str {
        &self.message_type
    }

    #[getter]
    fn timestamp_ns(&self) -> i64 {
        self.timestamp_ns
    }

    #[getter]
    fn payload(&self) -> Vec<u8> {
        self.payload.clone()
    }
}

/// Blocking Python receiver for one bounded, live-only message channel.
#[pyclass(name = "MessageChannelReceiver")]
pub struct PyMessageChannelReceiver {
    inner: Arc<Mutex<Option<RustMessageChannelRegistration>>>,
}

#[pymethods]
impl PyMessageChannelReceiver {
    /// Block until one live message arrives or the channel closes.
    fn recv(&self, py: Python<'_>) -> PyResult<Option<PyMessageEvent>> {
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let mut guard = inner.lock().expect("MessageChannelReceiver lock");
                let receiver = guard.as_mut().ok_or_else(|| {
                    PyRuntimeError::new_err("MessageChannelReceiver has been closed")
                })?;
                let resource_id = receiver.resource().resource_id.clone();
                let event = receiver.recv().await;
                Ok::<Option<PyMessageEvent>, PyErr>(event.map(|event| PyMessageEvent {
                    resource_id,
                    sender_peer_id: event.sender.to_string(),
                    message_type: event.message.r#type,
                    timestamp_ns: event.message.timestamp_ns,
                    payload: event.message.payload,
                }))
            })
        })
    }
}

// ─── ClusterMember pyclass ─────────────────────────────────────────

#[pyclass(name = "ClusterMember")]
#[derive(Clone)]
pub struct PyClusterMember {
    inner: RustClusterMember,
}

#[pymethods]
impl PyClusterMember {
    #[new]
    #[pyo3(signature = (peer_id, multiaddrs, join_ts_ns, successor_token = None))]
    fn new(
        peer_id: &str,
        multiaddrs: Vec<String>,
        join_ts_ns: i64,
        successor_token: Option<Vec<u8>>,
    ) -> PyResult<Self> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let multiaddrs = parse_multiaddrs(&multiaddrs)?;
        Ok(Self {
            inner: RustClusterMember {
                peer_id: peer_id_parsed,
                multiaddrs,
                join_ts_ns,
                successor_token,
            },
        })
    }

    #[getter]
    fn peer_id(&self) -> String {
        self.inner.peer_id.to_string()
    }

    #[getter]
    fn multiaddrs(&self) -> Vec<String> {
        self.inner
            .multiaddrs
            .iter()
            .map(|m| m.to_string())
            .collect()
    }

    #[getter]
    fn join_ts_ns(&self) -> i64 {
        self.inner.join_ts_ns
    }

    #[getter]
    fn successor_token(&self) -> Option<Vec<u8>> {
        self.inner.successor_token.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "ClusterMember(peer_id={:?}, multiaddrs={:?}, join_ts_ns={})",
            self.inner.peer_id.to_string(),
            self.multiaddrs(),
            self.inner.join_ts_ns,
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── ClusterMembership pyclass ─────────────────────────────────────

#[pyclass(name = "ClusterMembership")]
pub struct PyClusterMembership {
    inner: RustClusterMembership,
}

#[pymethods]
impl PyClusterMembership {
    #[new]
    fn new(cluster_name: String) -> Self {
        Self {
            inner: RustClusterMembership::new(cluster_name),
        }
    }

    #[staticmethod]
    fn from_json(s: &str) -> PyResult<Self> {
        let inner: RustClusterMembership = serde_json::from_str(s)
            .map_err(|e| PyValueError::new_err(format!("invalid ClusterMembership JSON: {e}")))?;
        Ok(Self { inner })
    }

    #[getter]
    fn cluster_name(&self) -> String {
        self.inner.cluster_name.clone()
    }

    #[getter]
    fn peers(&self) -> Vec<PyClusterMember> {
        self.inner
            .peers
            .iter()
            .map(|m| PyClusterMember { inner: m.clone() })
            .collect()
    }

    #[getter]
    fn filename(&self) -> String {
        self.inner.filename()
    }

    fn admit(&mut self, member: &PyClusterMember) -> usize {
        self.inner.admit(member.inner.clone())
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner)
            .map_err(|e| PyTypeError::new_err(format!("serializing ClusterMembership: {e}")))
    }

    fn __repr__(&self) -> String {
        format!(
            "ClusterMembership(cluster_name={:?}, peers={})",
            self.inner.cluster_name,
            self.inner.peers.len(),
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── DaemonInfo pyclass ────────────────────────────────────────────

/// Daemon-side identity fields the SDK doesn't own. Passed by the
/// daemon into `ClusterManager.participant_info` alongside the
/// cluster-aware fields the SDK fills in.
#[pyclass(name = "DaemonInfo")]
#[derive(Clone)]
pub struct PyDaemonInfo {
    inner: RustDaemonInfo,
}

#[pymethods]
impl PyDaemonInfo {
    #[new]
    #[pyo3(signature = (
        app,
        name,
        session_id,
        session_clock_id,
        session_clock_hash,
        app_instance,
    ))]
    fn new(
        app: String,
        name: String,
        session_id: String,
        session_clock_id: String,
        session_clock_hash: String,
        app_instance: String,
    ) -> Self {
        Self {
            inner: RustDaemonInfo {
                app,
                name,
                session_id,
                session_clock_id,
                session_clock_hash,
                app_instance,
            },
        }
    }
}

// ─── ParticipantInfo pyclass ───────────────────────────────────────

/// SDK-provided identity wire shape, exchanged peer-to-peer over
/// libp2p `/auki/info/0.0.1` (#293). Produced by
/// `ClusterManager.participant_info`.
#[pyclass(name = "ParticipantInfo")]
pub struct PyParticipantInfo {
    inner: RustParticipantInfo,
}

#[pymethods]
impl PyParticipantInfo {
    #[getter]
    fn app(&self) -> String {
        self.inner.app.clone()
    }

    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[getter]
    fn session_id(&self) -> String {
        self.inner.session_id.clone()
    }

    #[getter]
    fn session_clock_id(&self) -> String {
        self.inner.session_clock_id.clone()
    }

    #[getter]
    fn session_clock_hash(&self) -> String {
        self.inner.session_clock_hash.clone()
    }

    #[getter]
    fn session_now_ns(&self) -> u64 {
        self.inner.session_now_ns
    }

    #[getter]
    fn cluster_joined_at_ns(&self) -> Option<u64> {
        self.inner.cluster_joined_at_ns
    }

    #[getter]
    fn peer_id(&self) -> String {
        self.inner.peer_id.to_string()
    }

    #[getter]
    fn app_instance(&self) -> String {
        self.inner.app_instance.clone()
    }

    #[getter]
    fn is_manager(&self) -> bool {
        self.inner.is_manager
    }

    #[getter]
    fn manager_peer_id(&self) -> String {
        self.inner.manager_peer_id.clone()
    }

    /// Serialize to the canonical `ParticipantInfo` JSON shape — for
    /// app-local operator/debug surfaces; the peer-facing copy is
    /// served by the SDK runtime over `/auki/info/0.0.1`.
    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner)
            .map_err(|e| PyTypeError::new_err(format!("serializing ParticipantInfo: {e}")))
    }

    fn __repr__(&self) -> String {
        format!(
            "ParticipantInfo(app={:?}, peer_id={:?}, is_manager={}, manager_peer_id={:?})",
            self.inner.app,
            self.inner.peer_id.to_string(),
            self.inner.is_manager,
            self.inner.manager_peer_id,
        )
    }
}

// ─── ResourceEntry pyclass (post-#216) ─────────────────────────────
//
// The old SensorStreamResource / TransformEdgeResource / PoseStreamResource
// pyclasses are DELETED in #216. Replaced by a single flat ResourceEntry
// with a `variant` string discriminator plus optional `sensor` / `pose`
// blocks and a `manifest` dict.
//
// Python representation: all nested structs (Head, Extent, Available,
// SensorBlock, PoseBlock, manifest pointers) are returned as plain Python
// dicts, matching the auki-session-py pattern for nested types.

/// One row in a peer's resource catalog (post-#216 §1 schema).
///
/// `variant` is `"sensor_log"` | `"pose_log"` | `"time_transform_log"` |
/// `"detection_log"`. Use `.to_json()` to get the canonical wire JSON.
#[pyclass(name = "ResourceEntry")]
#[derive(Clone)]
pub struct PyResourceEntry {
    inner: RustResourceEntry,
}

#[pymethods]
impl PyResourceEntry {
    /// Construct from a JSON string matching the /auki/resources/0.2.0 row shape.
    /// Useful for round-tripping via `to_json()` or constructing from
    /// already-serialized rows on the wire.
    #[staticmethod]
    fn from_json(s: &str) -> PyResult<Self> {
        let inner: RustResourceEntry = serde_json::from_str(s)
            .map_err(|e| PyValueError::new_err(format!("invalid ResourceEntry JSON: {e}")))?;
        Ok(Self { inner })
    }

    /// Construct from a Python dict matching the /auki/resources/0.2.0 row shape.
    /// Internally converts via `json.dumps(...)` → `serde_json::from_str`.
    /// All four variants supported (sensor_log, pose_log, time_transform_log,
    /// detection_log) — serde handles the variant discrimination via the
    /// `variant` field.
    #[staticmethod]
    fn from_dict(py: Python<'_>, d: &Bound<'_, PyAny>) -> PyResult<Self> {
        let json_mod = py.import_bound("json")?;
        let json_str: String = json_mod.call_method1("dumps", (d,))?.extract()?;
        let inner: RustResourceEntry = serde_json::from_str(&json_str)
            .map_err(|e| PyValueError::new_err(format!("invalid ResourceEntry dict: {e}")))?;
        Ok(Self { inner })
    }

    #[getter]
    fn source_peer_id(&self) -> String {
        self.inner.source_peer_id.clone()
    }

    #[getter]
    fn writer_peer_id(&self) -> String {
        self.inner.writer_peer_id.clone()
    }

    #[getter]
    fn resource_id(&self) -> String {
        self.inner.resource_id.clone()
    }

    /// Variant discriminator: `"sensor_log"` | `"pose_log"` |
    /// `"time_transform_log"` | `"detection_log"`.
    #[getter]
    fn variant(&self) -> &'static str {
        variant_tag(&self.inner.variant_content)
    }

    /// Lifecycle state: `"live"` | `"sealed"`.
    #[getter]
    fn state(&self) -> String {
        self.inner.state.clone()
    }

    /// Head block as a dict, or `None` for sealed rows.
    ///
    /// Rolling: `{"kind": "rolling", "retention_ns": N}`
    /// Fixed:   `{"kind": "fixed", "started_at_ns": N}`
    #[getter]
    fn head<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        match &self.inner.head {
            Some(head) => {
                let d = PyDict::new_bound(py);
                match head {
                    auki_network::resources_protocol::Head::Rolling { retention_ns } => {
                        d.set_item("kind", "rolling")?;
                        d.set_item("retention_ns", retention_ns)?;
                    }
                    auki_network::resources_protocol::Head::Fixed { started_at_ns } => {
                        d.set_item("kind", "fixed")?;
                        d.set_item("started_at_ns", started_at_ns)?;
                    }
                }
                Ok(Some(d))
            }
            None => Ok(None),
        }
    }

    /// Extent block as a dict, or `None` for live rows.
    ///
    /// `{"start_at_ns": N, "finish_at_ns": N}`
    #[getter]
    fn extent<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        match &self.inner.extent {
            Some(extent) => {
                let d = PyDict::new_bound(py);
                d.set_item("start_at_ns", extent.start_at_ns)?;
                d.set_item("finish_at_ns", extent.finish_at_ns)?;
                Ok(Some(d))
            }
            None => Ok(None),
        }
    }

    /// Available data snapshot as a dict.
    ///
    /// `{"bytes": N, "entries": N, "duration_ns": N}`
    #[getter]
    fn available<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        d.set_item("bytes", self.inner.available.bytes)?;
        d.set_item("entries", self.inner.available.entries)?;
        d.set_item("duration_ns", self.inner.available.duration_ns)?;
        Ok(d)
    }

    /// Sensor block as a dict, or `None` for non-sensor-log rows.
    ///
    /// `{"kind": "camera"|..., "type": "rgb"|..., "sensor_id": ..., "sensor_hash": ...}`
    #[getter]
    fn sensor<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        match &self.inner.sensor {
            Some(sensor) => {
                let d = PyDict::new_bound(py);
                let kind_str = serde_json::to_value(&sensor.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| format!("{:?}", sensor.kind).to_lowercase());
                d.set_item("kind", kind_str)?;
                d.set_item("type", &sensor.r#type)?;
                d.set_item("sensor_id", &sensor.sensor_id)?;
                d.set_item("sensor_hash", &sensor.sensor_hash)?;
                Ok(Some(d))
            }
            None => Ok(None),
        }
    }

    /// Pose block as a dict, or `None` for non-pose-log rows.
    ///
    /// `{"writer_mode": "rigid"|"movable"}`
    #[getter]
    fn pose<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        match &self.inner.pose {
            Some(pose_block) => {
                let d = PyDict::new_bound(py);
                let mode_str = serde_json::to_value(&pose_block.writer_mode)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| format!("{:?}", pose_block.writer_mode).to_lowercase());
                d.set_item("writer_mode", mode_str)?;
                Ok(Some(d))
            }
            None => Ok(None),
        }
    }

    /// Variant-specific manifest pointer as a dict. The shape depends
    /// on `variant`:
    ///
    /// - `sensor_log`:        `{"clock": {peer_id,id,hash}, "frame": {peer_id,id,hash} | None}`
    /// - `pose_log`:          `{"from_frame": ..., "to_frame": ..., "clock": ..., "source": ..., "expected_rate_hz": N}`
    /// - `time_transform_log`: `{"from_clock": ..., "to_clock": ..., "source": ...}`
    /// - `detection_log`:      `{"instance_id": ..., "detector": ..., "input_log": ..., "input_sensor": ..., "clock": ..., "cadence": ...}`
    /// - `detection_log`:     `{"detector": ..., "input_log": {source_peer_id, resource_id}, "input_sensor": ..., "clock": ...}`
    #[getter]
    fn manifest<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        use auki_network::resources_protocol::VariantContent;
        let d = PyDict::new_bound(py);
        match &self.inner.variant_content {
            VariantContent::SensorLog { manifest } => {
                d.set_item("clock", registry_ref_to_dict(py, &manifest.clock)?)?;
                match &manifest.frame {
                    Some(frame) => d.set_item("frame", registry_ref_to_dict(py, frame)?)?,
                    None => d.set_item("frame", py.None())?,
                }
            }
            VariantContent::PoseLog { manifest } => {
                d.set_item(
                    "from_frame",
                    registry_ref_to_dict(py, &manifest.from_frame)?,
                )?;
                d.set_item("to_frame", registry_ref_to_dict(py, &manifest.to_frame)?)?;
                d.set_item("clock", registry_ref_to_dict(py, &manifest.clock)?)?;
                let source_str = serde_json::to_value(&manifest.source)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| format!("{:?}", manifest.source));
                d.set_item("source", source_str)?;
                d.set_item("expected_rate_hz", manifest.expected_rate_hz)?;
            }
            VariantContent::TimeTransformLog { manifest } => {
                d.set_item(
                    "from_clock",
                    registry_ref_to_dict(py, &manifest.from_clock)?,
                )?;
                d.set_item("to_clock", registry_ref_to_dict(py, &manifest.to_clock)?)?;
                let source_str = serde_json::to_value(&manifest.source)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| format!("{:?}", manifest.source));
                d.set_item("source", source_str)?;
            }
            VariantContent::DetectionLog { manifest } => {
                d.set_item("instance_id", &manifest.instance_id)?;
                d.set_item("detector", registry_ref_to_dict(py, &manifest.detector)?)?;
                let log_ref_d = PyDict::new_bound(py);
                log_ref_d.set_item("source_peer_id", &manifest.input_log.source_peer_id)?;
                log_ref_d.set_item("resource_id", &manifest.input_log.resource_id)?;
                d.set_item("input_log", log_ref_d)?;
                d.set_item(
                    "input_sensor",
                    registry_ref_to_dict(py, &manifest.input_sensor)?,
                )?;
                d.set_item("clock", registry_ref_to_dict(py, &manifest.clock)?)?;
                let cadence = PyDict::new_bound(py);
                match manifest.cadence {
                    auki_manifests::DetectionCadence::EveryFrame => {
                        cadence.set_item("kind", "every_frame")?;
                    }
                    auki_manifests::DetectionCadence::Periodic { period_ns } => {
                        cadence.set_item("kind", "periodic")?;
                        cadence.set_item("period_ns", period_ns)?;
                    }
                }
                d.set_item("cadence", cadence)?;
            }
        }
        Ok(d)
    }

    /// Serialize to canonical JSON (serde_json). Useful for debugging
    /// and wire-level assertions.
    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner)
            .map_err(|e| PyTypeError::new_err(format!("serializing ResourceEntry: {e}")))
    }

    fn __repr__(&self) -> String {
        format!(
            "ResourceEntry(source_peer_id={:?}, resource_id={:?}, variant={:?}, state={:?})",
            self.inner.source_peer_id,
            self.inner.resource_id,
            self.variant(),
            self.inner.state,
        )
    }
}

fn registry_ref_to_dict<'py>(
    py: Python<'py>,
    r: &auki_registry::RegistryRef,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    d.set_item("peer_id", &r.peer_id)?;
    d.set_item("id", &r.id)?;
    d.set_item("hash", &r.hash)?;
    Ok(d)
}

/// Adapter: wraps a Python callable returning a list of `ResourceEntry`
/// objects in a Rust `ResourceCatalogProvider`. Called from the inbound
/// `/auki/resources/0.2.0` handler task.
struct PyResourceCatalogProvider {
    callable: Py<PyAny>,
}

impl RustResourceCatalogProvider for PyResourceCatalogProvider {
    fn snapshot(&self) -> Vec<RustResourceEntry> {
        Python::with_gil(|py| {
            let result = self
                .callable
                .bind(py)
                .call0()
                .and_then(|res| extract_resource_entries(&res));
            match result {
                Ok(entries) => entries,
                Err(e) => {
                    eprintln!("auki-domain-py: resource_catalog_provider callable failed: {e}");
                    Vec::new()
                }
            }
        })
    }
}

fn extract_resource_entries(obj: &Bound<'_, PyAny>) -> PyResult<Vec<RustResourceEntry>> {
    let iter = obj.iter().map_err(|_| {
        PyTypeError::new_err("resource catalog provider must return an iterable of ResourceEntry")
    })?;
    let mut resources = Vec::new();
    for item in iter {
        let item: Bound<'_, PyAny> = item?;
        if let Ok(entry) = item.extract::<PyRef<'_, PyResourceEntry>>() {
            resources.push(entry.inner.clone());
            continue;
        }
        return Err(PyTypeError::new_err(
            "resource catalog provider returned an item that is not a ResourceEntry",
        ));
    }
    Ok(resources)
}

// ─── MapLogResource pyclass (/auki/resources/0.4.0) ────────────────

#[pyclass(name = "MapLogResource")]
#[derive(Clone)]
pub struct PyMapLogResource {
    inner: RustMapLogResource,
}

#[pymethods]
impl PyMapLogResource {
    #[staticmethod]
    fn from_json(s: &str) -> PyResult<Self> {
        let inner: RustMapLogResource = serde_json::from_str(s)
            .map_err(|e| PyValueError::new_err(format!("invalid MapLogResource JSON: {e}")))?;
        inner
            .validate()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self { inner })
    }

    #[staticmethod]
    fn from_dict(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let json = py.import_bound("json")?;
        let encoded: String = json.call_method1("dumps", (value,))?.extract()?;
        Self::from_json(&encoded)
    }

    #[getter]
    fn source_peer_id(&self) -> &str {
        &self.inner.source_peer_id
    }

    #[getter]
    fn writer_peer_id(&self) -> &str {
        &self.inner.writer_peer_id
    }

    #[getter]
    fn resource_id(&self) -> &str {
        &self.inner.resource_id
    }

    #[getter]
    fn map<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        registry_ref_to_dict(py, &self.inner.map)
    }

    #[getter]
    fn clock<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        registry_ref_to_dict(py, &self.inner.clock)
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner)
            .map_err(|e| PyTypeError::new_err(format!("serializing MapLogResource: {e}")))
    }
}

struct PyMapCatalogProvider {
    callable: Py<PyAny>,
}

impl RustMapCatalogProvider for PyMapCatalogProvider {
    fn map_catalog(&self) -> auki_network::resources_v4_protocol::ResourcesResponse {
        let resources = Python::with_gil(|py| {
            self.callable
                .bind(py)
                .call0()
                .and_then(|value| extract_map_log_resources(&value))
                .unwrap_or_else(|error| {
                    eprintln!("auki-domain-py: map_catalog_provider callable failed: {error}");
                    Vec::new()
                })
        });
        auki_network::resources_v4_protocol::ResourcesResponse { resources }
    }
}

fn extract_map_log_resources(obj: &Bound<'_, PyAny>) -> PyResult<Vec<RustMapLogResource>> {
    let mut resources = Vec::new();
    for item in obj.iter().map_err(|_| {
        PyTypeError::new_err("map catalog provider must return an iterable of MapLogResource")
    })? {
        let item = item?;
        let entry = item.extract::<PyRef<'_, PyMapLogResource>>().map_err(|_| {
            PyTypeError::new_err(
                "map catalog provider returned an item that is not a MapLogResource",
            )
        })?;
        resources.push(entry.inner.clone());
    }
    Ok(resources)
}

// ─── ReadFrom pyclass ───────────────────────────────────────────────

/// Tagged enum for stream start position (post-#216 §5).
///
/// Construct via the static factories:
/// - `ReadFrom.latest()` — tail from live end
/// - `ReadFrom.from_start()` — replay from beginning
/// - `ReadFrom.from_timestamp(ns)` — start at ≥ ns on log clock
#[pyclass(name = "ReadFrom", frozen)]
#[derive(Clone)]
pub struct PyReadFrom {
    inner: RustReadFrom,
}

#[pymethods]
impl PyReadFrom {
    /// Tail from the current live end — no historical replay.
    #[staticmethod]
    fn latest() -> Self {
        Self {
            inner: RustReadFrom::Latest,
        }
    }

    /// Replay from the very first entry in the log.
    #[staticmethod]
    fn from_start() -> Self {
        Self {
            inner: RustReadFrom::FromStart,
        }
    }

    /// Start at the first entry whose timestamp is ≥ `timestamp_ns`
    /// on the log's clock.
    #[staticmethod]
    fn from_timestamp(timestamp_ns: i64) -> Self {
        Self {
            inner: RustReadFrom::FromTimestamp(timestamp_ns),
        }
    }

    /// Discriminator string: `"latest"` | `"from_start"` | `"from_timestamp"`.
    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner {
            RustReadFrom::Latest => "latest",
            RustReadFrom::FromStart => "from_start",
            RustReadFrom::FromTimestamp(_) => "from_timestamp",
        }
    }

    /// Timestamp value for `from_timestamp` variants; `None` otherwise.
    #[getter]
    fn timestamp_ns(&self) -> Option<i64> {
        match self.inner {
            RustReadFrom::FromTimestamp(ts) => Some(ts),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        match self.inner {
            RustReadFrom::Latest => "ReadFrom.latest()".into(),
            RustReadFrom::FromStart => "ReadFrom.from_start()".into(),
            RustReadFrom::FromTimestamp(ts) => format!("ReadFrom.from_timestamp({ts})"),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── StreamRequest pyclass ──────────────────────────────────────────

/// Consumer → Producer subscription handshake (post-#216 §5).
///
/// Identifies the log to subscribe to (`source_peer_id`, `resource_id`)
/// and where to begin (`from`).
#[pyclass(name = "StreamRequest")]
#[derive(Clone)]
pub struct PyStreamRequest {
    inner: RustStreamRequest,
}

#[pymethods]
impl PyStreamRequest {
    #[new]
    #[pyo3(signature = (resource_id, source_peer_id = String::new(), from_ = None))]
    fn new(
        resource_id: String,
        source_peer_id: String,
        from_: Option<PyRef<'_, PyReadFrom>>,
    ) -> Self {
        let from = from_.map(|r| r.inner).unwrap_or_default();
        Self {
            inner: RustStreamRequest {
                source_peer_id,
                resource_id,
                from,
            },
        }
    }

    #[getter]
    fn source_peer_id(&self) -> String {
        self.inner.source_peer_id.clone()
    }

    #[getter]
    fn resource_id(&self) -> String {
        self.inner.resource_id.clone()
    }

    #[getter]
    fn from_(&self) -> PyReadFrom {
        PyReadFrom {
            inner: self.inner.from,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "StreamRequest(source_peer_id={:?}, resource_id={:?}, from_={:?})",
            self.inner.source_peer_id,
            self.inner.resource_id,
            PyReadFrom {
                inner: self.inner.from
            }
            .__repr__(),
        )
    }
}

// ─── StreamManifestBuilder pyclass ─────────────────────────────────

/// Producer-side helper for building accept-time stream manifests from
/// the local registry.
#[pyclass(name = "StreamManifestBuilder")]
pub struct PyStreamManifestBuilder;

#[pymethods]
impl PyStreamManifestBuilder {
    /// Build an `auki_network.cluster.StreamManifest` from local
    /// Sensor / Frame registry entries.
    ///
    /// `sensor_peer_id` is the peer-id string of the peer that wrote the
    /// sensor registry entries (used for sub-directory lookup inside
    /// `app_root/registries/sensors/<sensor_peer_id>/...`).
    #[staticmethod]
    #[pyo3(signature = (app_root, sensor_peer_id, sensor_id, sensor_hash, clock_id, clock_hash))]
    fn from_registry(
        py: Python<'_>,
        app_root: &Bound<'_, PyAny>,
        sensor_peer_id: &str,
        sensor_id: &str,
        sensor_hash: &str,
        clock_id: &str,
        clock_hash: &str,
    ) -> PyResult<Py<PyAny>> {
        let app_root = pathlike_to_pathbuf(py, app_root, "app_root")?;
        let sensor_peer_id = sensor_peer_id.to_string();
        let sensor_id = sensor_id.to_string();
        let sensor_hash = sensor_hash.to_string();
        let clock_id = clock_id.to_string();
        let clock_hash = clock_hash.to_string();

        let manifest = py
            .allow_threads(|| {
                RustStreamManifestBuilder::from_registry(
                    &app_root,
                    &sensor_peer_id,
                    sensor_id,
                    sensor_hash,
                    clock_id,
                    clock_hash,
                )
            })
            .map_err(map_build_stream_manifest_error)?;

        stream_manifest_to_python(py, manifest)
    }
}

// ─── ClusterTarget ─────────────────────────────────────────────────

/// Policy declaration for [`ClusterManager.bootstrap`]. Construct via
/// the static factories; the bare variant is opaque to Python.
#[pyclass(name = "ClusterTarget", frozen)]
#[derive(Clone)]
pub struct PyClusterTarget {
    pub(crate) inner: RustClusterTarget,
}

#[pymethods]
impl PyClusterTarget {
    /// Create a new cluster named `name`. Errors if the name is taken.
    #[staticmethod]
    fn create(name: &str) -> Self {
        Self {
            inner: RustClusterTarget::create(name),
        }
    }

    /// Join an existing cluster named `name`.
    #[staticmethod]
    fn join(name: &str) -> Self {
        Self {
            inner: RustClusterTarget::join(name),
        }
    }

    /// Join `name` if it exists; otherwise create it.
    #[staticmethod]
    fn join_or_create(name: &str) -> Self {
        Self {
            inner: RustClusterTarget::join_or_create(name),
        }
    }

    /// Join the most-recently-created cluster; if none, create with
    /// `fallback_name`.
    #[staticmethod]
    fn most_recent_or_create(fallback_name: &str) -> Self {
        Self {
            inner: RustClusterTarget::most_recent_or_create(fallback_name),
        }
    }

    /// Discriminator: `"create"`, `"join"`, `"join_or_create"`, or
    /// `"most_recent_or_create"`.
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            RustClusterTarget::Create { .. } => "create",
            RustClusterTarget::Join { .. } => "join",
            RustClusterTarget::JoinOrCreate { .. } => "join_or_create",
            RustClusterTarget::MostRecentOrCreate { .. } => "most_recent_or_create",
        }
    }

    /// The cluster name carried by the variant.
    #[getter]
    fn name(&self) -> &str {
        match &self.inner {
            RustClusterTarget::Create { name } => name,
            RustClusterTarget::Join { name } => name,
            RustClusterTarget::JoinOrCreate { name } => name,
            RustClusterTarget::MostRecentOrCreate { fallback_name } => fallback_name,
        }
    }

    fn __repr__(&self) -> String {
        format!("ClusterTarget.{}({:?})", self.kind(), self.name())
    }
}

// ─── ClusterManager pyclass ────────────────────────────────────────

#[pyclass(name = "ClusterManager")]
pub struct PyClusterManager {
    // Wrapped in Option<Mutex<...>> so `shutdown()` can take the
    // inner ClusterManager out by value (it consumes self). The
    // pyclass itself remains usable but every method on it returns
    // `RuntimeError("ClusterManager has been shut down")` after.
    inner: Arc<Mutex<Option<RustClusterManager>>>,
}

#[pymethods]
impl PyClusterManager {
    /// Snapshot Discovery's cluster directory.
    ///
    /// Returns the list sorted by `created_ns` desc (newest first).
    #[staticmethod]
    fn list_clusters(py: Python<'_>, discovery_url: &str) -> PyResult<Vec<PyClusterEntry>> {
        let url = discovery_url.to_string();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                RustClusterManager::list_clusters(url)
                    .await
                    .map(|entries| entries.into_iter().map(PyClusterEntry::from_rust).collect())
                    .map_err(map_discovery_error)
            })
        })
    }

    /// Policy-driven cluster bootstrap. The single entry point for
    /// headless daemons (Boosterapp, Sentinel).
    #[staticmethod]
    #[pyo3(signature = (
        target,
        wallet_seed,
        discovery_url,
        listen_addresses,
        agent_version,
        daemon_info,
        stream_provider = None,
        external_addresses = None,
    ))]
    fn bootstrap(
        py: Python<'_>,
        target: PyClusterTarget,
        wallet_seed: Vec<u8>,
        discovery_url: &str,
        listen_addresses: Vec<String>,
        agent_version: &str,
        daemon_info: &PyDaemonInfo,
        stream_provider: Option<Py<PyAny>>,
        external_addresses: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let seed: [u8; 32] = wallet_seed
            .try_into()
            .map_err(|_| PyValueError::new_err("wallet_seed must be 32 bytes"))?;
        let discovery_url = discovery_url.to_string();
        let agent_version = agent_version.to_string();
        let listen_multiaddrs = parse_multiaddrs(&listen_addresses)?;
        let external_multiaddrs = match external_addresses {
            Some(addrs) => Some(parse_multiaddrs(&addrs)?),
            None => None,
        };
        let daemon = daemon_info.inner.clone();
        let provider: StreamProvider = match stream_provider {
            Some(callable) => stream_provider_from_python(py, callable)?,
            None => decline_all_streams(),
        };
        let rust_target = target.inner;

        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let (identity, swarm, advertise_multiaddrs) = build_identity_and_swarm(
                    &seed,
                    listen_multiaddrs,
                    agent_version,
                    external_multiaddrs.as_deref(),
                )
                .await?;

                let manager = RustClusterManager::bootstrap(
                    rust_target,
                    identity,
                    advertise_multiaddrs,
                    discovery_url,
                    swarm,
                    provider,
                    daemon,
                )
                .await
                .map_err(map_bootstrap_error)?;

                Ok::<_, PyErr>(Self {
                    inner: Arc::new(Mutex::new(Some(manager))),
                })
            })
        })
    }

    /// Create a new cluster, becoming its initial Manager.
    #[staticmethod]
    #[pyo3(signature = (
        wallet_seed,
        cluster_name,
        discovery_url,
        listen_addresses,
        agent_version,
        daemon_info,
        stream_provider = None,
        external_addresses = None,
    ))]
    fn create_cluster(
        py: Python<'_>,
        wallet_seed: Vec<u8>,
        cluster_name: &str,
        discovery_url: &str,
        listen_addresses: Vec<String>,
        agent_version: &str,
        daemon_info: &PyDaemonInfo,
        stream_provider: Option<Py<PyAny>>,
        external_addresses: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let seed: [u8; 32] = wallet_seed
            .try_into()
            .map_err(|_| PyValueError::new_err("wallet_seed must be 32 bytes"))?;
        let cluster_name = cluster_name.to_string();
        let discovery_url = discovery_url.to_string();
        let agent_version = agent_version.to_string();
        let listen_multiaddrs = parse_multiaddrs(&listen_addresses)?;
        let external_multiaddrs = match external_addresses {
            Some(addrs) => Some(parse_multiaddrs(&addrs)?),
            None => None,
        };
        let daemon = daemon_info.inner.clone();
        let provider: StreamProvider = match stream_provider {
            Some(callable) => stream_provider_from_python(py, callable)?,
            None => decline_all_streams(),
        };

        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let (identity, swarm, advertise_multiaddrs) = build_identity_and_swarm(
                    &seed,
                    listen_multiaddrs,
                    agent_version,
                    external_multiaddrs.as_deref(),
                )
                .await?;

                let manager = RustClusterManager::create_cluster(
                    cluster_name,
                    identity,
                    advertise_multiaddrs,
                    discovery_url,
                    swarm,
                    provider,
                    daemon,
                )
                .await
                .map_err(map_create_cluster_error)?;

                Ok::<_, PyErr>(Self {
                    inner: Arc::new(Mutex::new(Some(manager))),
                })
            })
        })
    }

    /// Create a new cluster and include browser-compatible Relay hints.
    #[staticmethod]
    #[pyo3(signature = (
        wallet_seed,
        cluster_name,
        discovery_url,
        listen_addresses,
        relay_multiaddrs,
        agent_version,
        daemon_info,
        stream_provider = None,
        external_addresses = None,
    ))]
    fn create_cluster_with_relay_multiaddrs(
        py: Python<'_>,
        wallet_seed: Vec<u8>,
        cluster_name: &str,
        discovery_url: &str,
        listen_addresses: Vec<String>,
        relay_multiaddrs: Vec<String>,
        agent_version: &str,
        daemon_info: &PyDaemonInfo,
        stream_provider: Option<Py<PyAny>>,
        external_addresses: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let seed: [u8; 32] = wallet_seed
            .try_into()
            .map_err(|_| PyValueError::new_err("wallet_seed must be 32 bytes"))?;
        let cluster_name = cluster_name.to_string();
        let discovery_url = discovery_url.to_string();
        let agent_version = agent_version.to_string();
        let listen_multiaddrs = parse_multiaddrs(&listen_addresses)?;
        let relay_multiaddrs = parse_multiaddrs(&relay_multiaddrs)?;
        let external_multiaddrs = match external_addresses {
            Some(addrs) => Some(parse_multiaddrs(&addrs)?),
            None => None,
        };
        let daemon = daemon_info.inner.clone();
        let provider: StreamProvider = match stream_provider {
            Some(callable) => stream_provider_from_python(py, callable)?,
            None => decline_all_streams(),
        };

        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let (identity, swarm, advertise_multiaddrs) = build_identity_and_swarm(
                    &seed,
                    listen_multiaddrs,
                    agent_version,
                    external_multiaddrs.as_deref(),
                )
                .await?;

                let manager = RustClusterManager::create_cluster_with_relay_multiaddrs(
                    cluster_name,
                    identity,
                    advertise_multiaddrs,
                    relay_multiaddrs,
                    discovery_url,
                    swarm,
                    provider,
                    daemon,
                )
                .await
                .map_err(map_create_cluster_error)?;

                Ok::<_, PyErr>(Self {
                    inner: Arc::new(Mutex::new(Some(manager))),
                })
            })
        })
    }

    /// Create a new cluster after reserving a relay-mediated Manager address.
    #[staticmethod]
    #[pyo3(signature = (
        wallet_seed,
        cluster_name,
        discovery_url,
        listen_addresses,
        relay_dial_multiaddr,
        relay_advertise_multiaddr,
        agent_version,
        daemon_info,
        stream_provider = None,
        external_addresses = None,
        relay_reservation_timeout_ms = 10000,
    ))]
    fn create_cluster_with_relay_reservation(
        py: Python<'_>,
        wallet_seed: Vec<u8>,
        cluster_name: &str,
        discovery_url: &str,
        listen_addresses: Vec<String>,
        relay_dial_multiaddr: &str,
        relay_advertise_multiaddr: &str,
        agent_version: &str,
        daemon_info: &PyDaemonInfo,
        stream_provider: Option<Py<PyAny>>,
        external_addresses: Option<Vec<String>>,
        relay_reservation_timeout_ms: u64,
    ) -> PyResult<Self> {
        let seed: [u8; 32] = wallet_seed
            .try_into()
            .map_err(|_| PyValueError::new_err("wallet_seed must be 32 bytes"))?;
        let cluster_name = cluster_name.to_string();
        let discovery_url = discovery_url.to_string();
        let agent_version = agent_version.to_string();
        let listen_multiaddrs = parse_multiaddrs(&listen_addresses)?;
        let relay_dial_multiaddr = Multiaddr::from_str(relay_dial_multiaddr).map_err(|e| {
            PyValueError::new_err(format!(
                "invalid relay_dial_multiaddr {relay_dial_multiaddr:?}: {e}"
            ))
        })?;
        let relay_advertise_multiaddr =
            Multiaddr::from_str(relay_advertise_multiaddr).map_err(|e| {
                PyValueError::new_err(format!(
                    "invalid relay_advertise_multiaddr {relay_advertise_multiaddr:?}: {e}"
                ))
            })?;
        let external_multiaddrs = match external_addresses {
            Some(addrs) => Some(parse_multiaddrs(&addrs)?),
            None => None,
        };
        let daemon = daemon_info.inner.clone();
        let provider: StreamProvider = match stream_provider {
            Some(callable) => stream_provider_from_python(py, callable)?,
            None => decline_all_streams(),
        };
        let relay_reservation = RustManagerRelayReservation {
            relay_dial_multiaddr,
            relay_advertise_multiaddr,
            timeout: std::time::Duration::from_millis(relay_reservation_timeout_ms),
        };

        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let (identity, swarm, advertise_multiaddrs) = build_identity_and_swarm(
                    &seed,
                    listen_multiaddrs,
                    agent_version,
                    external_multiaddrs.as_deref(),
                )
                .await?;

                let manager = RustClusterManager::create_cluster_with_relay_reservation(
                    cluster_name,
                    identity,
                    advertise_multiaddrs,
                    relay_reservation,
                    discovery_url,
                    swarm,
                    provider,
                    daemon,
                )
                .await
                .map_err(map_create_cluster_error)?;

                Ok::<_, PyErr>(Self {
                    inner: Arc::new(Mutex::new(Some(manager))),
                })
            })
        })
    }

    /// Join an existing cluster by talking to its Manager.
    #[staticmethod]
    #[pyo3(signature = (
        wallet_seed,
        cluster_name,
        discovery_url,
        listen_addresses,
        agent_version,
        daemon_info,
        stream_provider = None,
        external_addresses = None,
    ))]
    fn join_cluster(
        py: Python<'_>,
        wallet_seed: Vec<u8>,
        cluster_name: &str,
        discovery_url: &str,
        listen_addresses: Vec<String>,
        agent_version: &str,
        daemon_info: &PyDaemonInfo,
        stream_provider: Option<Py<PyAny>>,
        external_addresses: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let seed: [u8; 32] = wallet_seed
            .try_into()
            .map_err(|_| PyValueError::new_err("wallet_seed must be 32 bytes"))?;
        let cluster_name = cluster_name.to_string();
        let discovery_url = discovery_url.to_string();
        let agent_version = agent_version.to_string();
        let listen_multiaddrs = parse_multiaddrs(&listen_addresses)?;
        let external_multiaddrs = match external_addresses {
            Some(addrs) => Some(parse_multiaddrs(&addrs)?),
            None => None,
        };
        let daemon = daemon_info.inner.clone();
        let provider: StreamProvider = match stream_provider {
            Some(callable) => stream_provider_from_python(py, callable)?,
            None => decline_all_streams(),
        };

        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let (identity, swarm, advertise_multiaddrs) = build_identity_and_swarm(
                    &seed,
                    listen_multiaddrs,
                    agent_version,
                    external_multiaddrs.as_deref(),
                )
                .await?;

                let manager = RustClusterManager::join_cluster(
                    cluster_name,
                    identity,
                    advertise_multiaddrs,
                    discovery_url,
                    swarm,
                    provider,
                    daemon,
                )
                .await
                .map_err(map_join_cluster_error)?;

                Ok::<_, PyErr>(Self {
                    inner: Arc::new(Mutex::new(Some(manager))),
                })
            })
        })
    }

    #[getter]
    fn cluster_name(&self) -> PyResult<String> {
        self.with_inner(|m| Ok(m.cluster_name().to_string()))
    }

    #[getter]
    fn local_peer_id(&self) -> PyResult<String> {
        self.with_inner(|m| Ok(m.local_peer_id().to_string()))
    }

    #[getter]
    fn is_manager(&self) -> PyResult<bool> {
        self.with_inner(|m| Ok(m.is_manager()))
    }

    #[getter]
    fn manager_peer_id(&self) -> PyResult<String> {
        self.with_inner(|m| Ok(m.manager_peer_id().to_string()))
    }

    #[getter]
    fn peer_count(&self) -> PyResult<usize> {
        self.with_inner(|m| Ok(m.peer_count()))
    }

    /// Snapshot of cluster membership.
    fn membership(&self) -> PyResult<PyClusterMembership> {
        self.with_inner(|m| {
            Ok(PyClusterMembership {
                inner: m.membership(),
            })
        })
    }

    /// Admit a new peer to the cluster (Manager-only).
    fn admit_peer(
        &self,
        py: Python<'_>,
        peer_id: &str,
        multiaddrs: Vec<String>,
    ) -> PyResult<PyClusterMember> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let multiaddrs = parse_multiaddrs(&multiaddrs)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let member = manager
                    .admit_peer(peer_id_parsed, multiaddrs)
                    .await
                    .map_err(map_admit_error)?;
                Ok(PyClusterMember { inner: member })
            })
        })
    }

    /// Open a camera stream subscription on `peer_id` for `resource_id`.
    fn open_camera_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        resource_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustCameraFrame>(py, peer_id, resource_id, String::new(), |sub| {
            PyStreamSubscription::from_rust_camera(sub)
        })
    }

    /// Open a PointCloud stream subscription on `peer_id` for `resource_id`.
    fn open_pointcloud_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        resource_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustPointCloudFrame>(
            py,
            peer_id,
            resource_id,
            String::new(),
            |sub| PyStreamSubscription::from_rust_pointcloud(sub),
        )
    }

    /// Open a JointEncoders stream subscription on `peer_id` for `resource_id`.
    fn open_joint_encoders_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        resource_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustJointEncodersFrame>(
            py,
            peer_id,
            resource_id,
            String::new(),
            |sub| PyStreamSubscription::from_rust_joint_encoders(sub),
        )
    }

    /// Open an Audio stream subscription on `peer_id` for `resource_id`.
    fn open_audio_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        resource_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustAudioFrame>(py, peer_id, resource_id, String::new(), |sub| {
            PyStreamSubscription::from_rust_audio(sub)
        })
    }

    /// Open a live pose stream subscription on `peer_id` for `resource_id`.
    fn open_pose_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        resource_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustPoseSpatialTransform>(
            py,
            peer_id,
            resource_id,
            String::new(),
            |sub| PyStreamSubscription::from_rust_pose(sub),
        )
    }

    /// Open an exact discovered Map Log with replay plus live updates.
    #[pyo3(signature = (peer_id, resource, from_=None))]
    fn open_map_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        resource: &PyMapLogResource,
        from_: Option<PyRef<'_, PyReadFrom>>,
    ) -> PyResult<PyStreamSubscription> {
        let target = parse_peer_id(peer_id)?;
        let expected = resource.inner.clone();
        expected
            .validate()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        if expected.writer_peer_id != peer_id {
            return Err(PyValueError::new_err(format!(
                "map writer mismatch: target={peer_id}, writer_peer_id={}",
                expected.writer_peer_id
            )));
        }
        let request = RustStreamRequest {
            source_peer_id: expected.source_peer_id.clone(),
            resource_id: expected.resource_id.clone(),
            from: from_.map(|value| value.inner).unwrap_or_default(),
        };
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let subscription = manager
                    .open_stream::<RustMapUpdate>(target, request)
                    .await
                    .map_err(|error| {
                        Python::with_gil(|py| open_stream_error_to_pyerr(py, error))
                    })?;
                let manifest = &subscription.manifest;
                let matches = manifest.resource_id == expected.resource_id
                    && manifest.payload == "map_update"
                    && manifest.map_peer_id == expected.map.peer_id
                    && manifest.map_id == expected.map.id
                    && manifest.map_hash == expected.map.hash
                    && manifest.clock_peer_id == expected.clock.peer_id
                    && manifest.clock_id == expected.clock.id
                    && manifest.clock_hash == expected.clock.hash;
                if !matches {
                    return Err(PyValueError::new_err(
                        "Map stream manifest does not match the discovered MapLogResource",
                    ));
                }
                Ok(PyStreamSubscription::from_rust_map(subscription))
            })
        })
    }

    /// Open a stream via a full `StreamRequest` (post-#216 §5).
    ///
    /// Accepts a `StreamRequest` pyclass (use `StreamRequest(resource_id=...,
    /// source_peer_id=..., from_=ReadFrom.latest())`) and `peer_id` as the
    /// libp2p peer to dial.
    fn open_stream_with_request(
        &self,
        py: Python<'_>,
        peer_id: &str,
        request: &PyStreamRequest,
    ) -> PyResult<PyStreamSubscription> {
        // Resolve the payload kind by fetching the resource catalog first.
        let resource_id = request.inner.resource_id.clone();
        let source_peer_id = request.inner.source_peer_id.clone();
        let kind = self.resolve_stream_payload_kind(py, peer_id, &source_peer_id, &resource_id)?;
        let rust_request = request.inner.clone();

        match kind {
            GenericStreamPayloadKind::Camera => self
                .open_typed_stream_with_request::<RustCameraFrame>(
                    py,
                    peer_id,
                    rust_request,
                    |sub| PyStreamSubscription::from_rust_camera(sub),
                ),
            GenericStreamPayloadKind::PointCloud => self
                .open_typed_stream_with_request::<RustPointCloudFrame>(
                    py,
                    peer_id,
                    rust_request,
                    |sub| PyStreamSubscription::from_rust_pointcloud(sub),
                ),
            GenericStreamPayloadKind::JointEncoders => self
                .open_typed_stream_with_request::<RustJointEncodersFrame>(
                    py,
                    peer_id,
                    rust_request,
                    |sub| PyStreamSubscription::from_rust_joint_encoders(sub),
                ),
            GenericStreamPayloadKind::Audio => self
                .open_typed_stream_with_request::<RustAudioFrame>(
                    py,
                    peer_id,
                    rust_request,
                    |sub| PyStreamSubscription::from_rust_audio(sub),
                ),
            GenericStreamPayloadKind::Pose => self
                .open_typed_stream_with_request::<RustPoseSpatialTransform>(
                    py,
                    peer_id,
                    rust_request,
                    |sub| PyStreamSubscription::from_rust_pose(sub),
                ),
        }
    }

    /// Open a stream subscription without requiring the Python caller to
    /// choose a payload-specific opener. The SDK fetches the peer's resource
    /// catalog, resolves the matching row's variant + sensor kind, then
    /// delegates to the typed Rust subscription. Returned entries still yield
    /// typed payload pyclasses (`CameraFrame`, `PointCloudFrame`, etc.).
    ///
    /// `peer_id` is the libp2p peer to dial; `resource_id` matches the
    /// `resource_id` field in the peer's resource catalog.
    fn open_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        resource_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        match self.resolve_stream_payload_kind(py, peer_id, "", resource_id)? {
            GenericStreamPayloadKind::Camera => self.open_typed_stream::<RustCameraFrame>(
                py,
                peer_id,
                resource_id,
                String::new(),
                |sub| PyStreamSubscription::from_rust_camera(sub),
            ),
            GenericStreamPayloadKind::PointCloud => self.open_typed_stream::<RustPointCloudFrame>(
                py,
                peer_id,
                resource_id,
                String::new(),
                |sub| PyStreamSubscription::from_rust_pointcloud(sub),
            ),
            GenericStreamPayloadKind::JointEncoders => self
                .open_typed_stream::<RustJointEncodersFrame>(
                    py,
                    peer_id,
                    resource_id,
                    String::new(),
                    |sub| PyStreamSubscription::from_rust_joint_encoders(sub),
                ),
            GenericStreamPayloadKind::Audio => self.open_typed_stream::<RustAudioFrame>(
                py,
                peer_id,
                resource_id,
                String::new(),
                |sub| PyStreamSubscription::from_rust_audio(sub),
            ),
            GenericStreamPayloadKind::Pose => self.open_pose_stream(py, peer_id, resource_id),
        }
    }

    /// Build a fresh `ParticipantInfo` snapshot.
    fn participant_info(&self) -> PyResult<PyParticipantInfo> {
        self.with_inner(|m| {
            Ok(PyParticipantInfo {
                inner: m.participant_info(),
            })
        })
    }

    /// Fetch a cluster peer's `ParticipantInfo` over `/auki/info/0.0.1`.
    fn fetch_participant_info(&self, py: Python<'_>, peer_id: &str) -> PyResult<PyParticipantInfo> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let info = manager
                    .fetch_participant_info(peer_id_parsed)
                    .await
                    .map_err(map_fetch_participant_info_error)?;
                Ok(PyParticipantInfo { inner: info })
            })
        })
    }

    /// Register the resource catalog provider callable. `callable` must
    /// return a list of `ResourceEntry` objects and is invoked for each
    /// inbound `/auki/resources/0.2.0` fetch.
    fn set_resource_catalog_provider(&self, callable: Py<PyAny>) -> PyResult<()> {
        let provider = Arc::new(PyResourceCatalogProvider { callable });
        self.with_inner(|m| {
            m.set_resource_catalog_provider(provider);
            Ok(())
        })
    }

    /// Register the SDK Map Log catalog served over `/auki/resources/0.4.0`.
    fn set_map_catalog_provider(&self, callable: Py<PyAny>) -> PyResult<()> {
        let provider = Arc::new(PyMapCatalogProvider { callable });
        self.with_inner(|manager| {
            manager.set_map_catalog_provider(provider);
            Ok(())
        })
    }

    /// Register (or replace) the app root for hash-pinned registry entries.
    fn set_registry_app_root(&self, py: Python<'_>, app_root: &Bound<'_, PyAny>) -> PyResult<()> {
        let path = pathlike_to_pathbuf(py, app_root, "app_root")?;
        self.with_inner(|m| {
            m.set_registry_app_root(path);
            Ok(())
        })
    }

    /// Register a bounded, receiver-owned live message channel.
    ///
    /// The channel uses this manager's SDK-declared session clock. The returned
    /// receiver keeps the channel advertised until it is dropped or the
    /// ClusterManager shuts down.
    #[pyo3(signature = (resource_id, capacity = 64))]
    fn register_message_channel(
        &self,
        resource_id: String,
        capacity: usize,
    ) -> PyResult<PyMessageChannelReceiver> {
        let registration = self.with_inner(|manager| {
            let owner_peer_id = manager.local_peer_id();
            let participant = manager.participant_info();
            manager
                .register_message_channel(
                    RustMessageChannelResource {
                        owner_peer_id,
                        resource_id,
                        clock: RustRegistryRef {
                            peer_id: participant.peer_id.to_string(),
                            id: participant.session_clock_id,
                            hash: participant.session_clock_hash,
                        },
                    },
                    capacity,
                )
                .map_err(|error| PyValueError::new_err(error.to_string()))
        })?;
        Ok(PyMessageChannelReceiver {
            inner: Arc::new(Mutex::new(Some(registration))),
        })
    }

    /// Fetch a cluster peer's current resource catalog over `/auki/resources/0.2.0`.
    ///
    /// Returns a `list[ResourceEntry]`. Use `variants` to filter by
    /// variant string (e.g. `["sensor_log"]`, `["pose_log"]`). Empty
    /// means all variants.
    #[pyo3(signature = (peer_id, variants = None))]
    fn fetch_resources_catalog(
        &self,
        py: Python<'_>,
        peer_id: &str,
        variants: Option<Vec<String>>,
    ) -> PyResult<Vec<PyResourceEntry>> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let rust_variants = parse_variants(variants)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let resp = manager
                    .fetch_resources_catalog_with(
                        peer_id_parsed,
                        RustResourcesRequest {
                            variants: rust_variants,
                        },
                    )
                    .await
                    .map_err(map_fetch_resources_catalog_error)?;
                Ok(resp
                    .resources
                    .into_iter()
                    .map(|inner| PyResourceEntry { inner })
                    .collect())
            })
        })
    }

    /// Fetch a peer's Map Log catalog over `/auki/resources/0.4.0`.
    fn fetch_map_catalog(&self, py: Python<'_>, peer_id: &str) -> PyResult<Vec<PyMapLogResource>> {
        let peer_id = parse_peer_id(peer_id)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let response = manager
                    .fetch_map_catalog(peer_id)
                    .await
                    .map_err(map_fetch_map_catalog_error)?;
                Ok(response
                    .resources
                    .into_iter()
                    .map(|inner| PyMapLogResource { inner })
                    .collect())
            })
        })
    }

    /// Fetch and verify a peer's Sensor Registry entry.
    fn fetch_sensor_entry(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: String,
        sensor_hash: String,
    ) -> PyResult<String> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let entry = manager
                    .fetch_sensor_entry(peer_id_parsed, sensor_id, sensor_hash)
                    .await
                    .map_err(map_fetch_registry_entry_error)?;
                Ok(canonical_json_to_py(entry.canonical_bytes()))
            })
        })
    }

    /// Fetch and verify a peer's Clock Registry entry.
    fn fetch_clock_entry(
        &self,
        py: Python<'_>,
        peer_id: &str,
        clock_id: String,
        clock_hash: String,
    ) -> PyResult<String> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let entry = manager
                    .fetch_clock_entry(peer_id_parsed, clock_id, clock_hash)
                    .await
                    .map_err(map_fetch_registry_entry_error)?;
                Ok(canonical_json_to_py(entry.canonical_bytes()))
            })
        })
    }

    /// Fetch and verify a peer's Frame Registry entry.
    fn fetch_frame_entry(
        &self,
        py: Python<'_>,
        peer_id: &str,
        frame_id: String,
        frame_hash: String,
    ) -> PyResult<String> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let entry = manager
                    .fetch_frame_entry(peer_id_parsed, frame_id, frame_hash)
                    .await
                    .map_err(map_fetch_registry_entry_error)?;
                Ok(canonical_json_to_py(entry.canonical_bytes()))
            })
        })
    }

    /// Shutdown — cancels the Manager heartbeat tick, deregisters the
    /// cluster from Discovery (if we're the Manager), and shuts down
    /// the runtime.
    fn shutdown(&self, py: Python<'_>) -> PyResult<()> {
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let manager = {
                    let mut guard = inner.lock().expect("ClusterManager lock");
                    guard.take()
                };
                let manager = manager
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                manager.shutdown().await.map_err(|e| {
                    PyOSError::new_err(format!("Discovery deregister failed during shutdown: {e}"))
                })
            })
        })
    }
}

impl PyClusterManager {
    fn with_inner<R>(&self, f: impl FnOnce(&RustClusterManager) -> PyResult<R>) -> PyResult<R> {
        let guard = self.inner.lock().expect("ClusterManager lock");
        let manager = guard
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
        f(manager)
    }

    /// Internal helper for typed stream opening. Builds a post-#216
    /// `StreamRequest` from `(resource_id, source_peer_id)` with
    /// `ReadFrom::Latest` (live streaming default).
    fn open_typed_stream<T>(
        &self,
        py: Python<'_>,
        peer_id: &str,
        resource_id: &str,
        source_peer_id: String,
        to_py_sub: impl FnOnce(
            auki_network::stream_runtime::StreamSubscription<T>,
        ) -> PyStreamSubscription
        + Send
        + 'static,
    ) -> PyResult<PyStreamSubscription>
    where
        T: prost::Message + Default + Send + 'static,
    {
        self.open_typed_stream_with_request(
            py,
            peer_id,
            RustStreamRequest {
                resource_id: resource_id.to_string(),
                source_peer_id,
                from: RustReadFrom::Latest,
            },
            to_py_sub,
        )
    }

    fn open_typed_stream_with_request<T>(
        &self,
        py: Python<'_>,
        peer_id: &str,
        request: RustStreamRequest,
        to_py_sub: impl FnOnce(
            auki_network::stream_runtime::StreamSubscription<T>,
        ) -> PyStreamSubscription
        + Send
        + 'static,
    ) -> PyResult<PyStreamSubscription>
    where
        T: prost::Message + Default + Send + 'static,
    {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let rust_sub = manager
                    .open_stream::<T>(peer_id_parsed, request)
                    .await
                    .map_err(|e| Python::with_gil(|py| open_stream_error_to_pyerr(py, e)))?;
                Ok(to_py_sub(rust_sub))
            })
        })
    }

    fn resolve_stream_payload_kind(
        &self,
        py: Python<'_>,
        peer_id: &str,
        source_peer_id: &str,
        resource_id: &str,
    ) -> PyResult<GenericStreamPayloadKind> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let source_peer_id = source_peer_id.to_string();
        let resource_id = resource_id.to_string();
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let resp = manager
                    .fetch_resources_catalog_with(
                        peer_id_parsed,
                        RustResourcesRequest {
                            variants: vec![RustVariant::SensorLog, RustVariant::PoseLog],
                        },
                    )
                    .await
                    .map_err(map_fetch_resources_catalog_error)?;
                resolve_generic_stream_payload_kind_from_entries(
                    &resp.resources,
                    &source_peer_id,
                    &resource_id,
                )
            })
        })
    }
}

// ─── Helpers ───────────────────────────────────────────────────────

fn parse_peer_id(s: &str) -> PyResult<PeerId> {
    PeerId::from_str(s).map_err(|e| PyValueError::new_err(format!("invalid peer_id {s:?}: {e}")))
}

fn parse_multiaddrs(ss: &[String]) -> PyResult<Vec<Multiaddr>> {
    ss.iter()
        .map(|s| {
            Multiaddr::from_str(s)
                .map_err(|e| PyValueError::new_err(format!("invalid multiaddr {s:?}: {e}")))
        })
        .collect()
}

/// Parse optional variant filter strings to `Variant` enum values.
fn parse_variants(variants: Option<Vec<String>>) -> PyResult<Vec<RustVariant>> {
    let Some(vs) = variants else {
        return Ok(vec![]);
    };
    vs.iter()
        .map(|s| match s.as_str() {
            "sensor_log" => Ok(RustVariant::SensorLog),
            "pose_log" => Ok(RustVariant::PoseLog),
            "time_transform_log" => Ok(RustVariant::TimeTransformLog),
            "detection_log" => Ok(RustVariant::DetectionLog),
            other => Err(PyValueError::new_err(format!(
                "unknown variant {other:?}; expected one of: sensor_log, pose_log, time_transform_log, detection_log"
            ))),
        })
        .collect()
}

async fn build_identity_and_swarm(
    seed: &[u8; 32],
    listen_multiaddrs: Vec<Multiaddr>,
    agent_version: String,
    operator_override: Option<&[Multiaddr]>,
) -> PyResult<(
    PeerIdentity,
    auki_network::Swarm<auki_network::swarm::Behaviour>,
    Vec<Multiaddr>,
)> {
    use auki_network::swarm::resolve_advertise_multiaddrs;
    use std::time::Duration;
    let wallet = Wallet::from_seed(seed.to_vec()).expect("caller passes a 32-byte seed");
    let identity = PeerIdentity::from_wallet(wallet);
    let cfg = SwarmConfig {
        listen_addresses: listen_multiaddrs,
        agent_version,
        enable_relay_server: false,
    };
    let mut swarm = build_swarm(&identity, cfg)
        .map_err(|e| PyOSError::new_err(format!("build_swarm failed: {e}")))?;
    let advertise =
        resolve_advertise_multiaddrs(&mut swarm, operator_override, Duration::from_secs(2)).await;
    if advertise.is_empty() {
        return Err(PyOSError::new_err(
            "no advertise multiaddrs resolved — pass `external_addresses=[...]` \
             explicitly, or bind to /ip4/0.0.0.0/... on a host with at least one \
             non-loopback interface",
        ));
    }
    Ok((identity, swarm, advertise))
}

fn map_join_cluster_error(e: RustJoinClusterError) -> PyErr {
    match e {
        RustJoinClusterError::Discovery(err) => PyOSError::new_err(format!("Discovery: {err}")),
        RustJoinClusterError::NotFound(name) => {
            PyRuntimeError::new_err(format!("cluster {name:?} not found in Discovery directory"))
        }
        RustJoinClusterError::SendJoin(err) => PyOSError::new_err(format!("join request: {err}")),
        RustJoinClusterError::Rejected(reason) => {
            PyRuntimeError::new_err(format!("Manager rejected join: {reason}"))
        }
        RustJoinClusterError::InvalidMembership(err) => {
            PyValueError::new_err(format!("invalid membership JSON from Manager: {err}"))
        }
        RustJoinClusterError::Runtime(err) => PyRuntimeError::new_err(format!("runtime: {err}")),
    }
}

fn map_discovery_error(e: RustDiscoveryError) -> PyErr {
    match e {
        RustDiscoveryError::Transport(err) => {
            PyOSError::new_err(format!("Discovery transport: {err}"))
        }
        RustDiscoveryError::Status { status, body } => {
            PyRuntimeError::new_err(format!("Discovery HTTP {status}: {body}"))
        }
        RustDiscoveryError::InvalidPeerId(s) => {
            PyValueError::new_err(format!("invalid peer-id in Discovery response: {s}"))
        }
        RustDiscoveryError::InvalidMultiaddr(s) => {
            PyValueError::new_err(format!("invalid multiaddr in Discovery response: {s}"))
        }
    }
}

fn map_bootstrap_error(e: RustBootstrapError) -> PyErr {
    match e {
        RustBootstrapError::Discovery(err) => map_discovery_error(err),
        RustBootstrapError::AlreadyExists(name) => PyRuntimeError::new_err(format!(
            "cluster {name:?} already exists in Discovery directory"
        )),
        RustBootstrapError::NotFound(name) => {
            PyRuntimeError::new_err(format!("cluster {name:?} not in Discovery directory"))
        }
        RustBootstrapError::SendJoin(err) => PyOSError::new_err(format!("join request: {err}")),
        RustBootstrapError::Rejected(reason) => {
            PyRuntimeError::new_err(format!("Manager rejected join: {reason}"))
        }
        RustBootstrapError::InvalidMembership(err) => {
            PyValueError::new_err(format!("invalid membership JSON from Manager: {err}"))
        }
        RustBootstrapError::Runtime(err) => {
            PyRuntimeError::new_err(format!("runtime spawn failed: {err}"))
        }
        RustBootstrapError::RelayReservation(err) => {
            PyOSError::new_err(format!("relay reservation failed: {err}"))
        }
    }
}

fn map_create_cluster_error(e: RustCreateClusterError) -> PyErr {
    match e {
        RustCreateClusterError::Discovery(err) => PyOSError::new_err(format!("Discovery: {err}")),
        RustCreateClusterError::AlreadyExists(name) => PyRuntimeError::new_err(format!(
            "cluster {name:?} already exists; list and join instead"
        )),
        RustCreateClusterError::Runtime(err) => {
            PyRuntimeError::new_err(format!("runtime spawn failed: {err}"))
        }
        RustCreateClusterError::RelayReservation(err) => {
            PyOSError::new_err(format!("relay reservation failed: {err}"))
        }
    }
}

fn map_admit_error(e: RustAdmitError) -> PyErr {
    match e {
        RustAdmitError::NotManager { cluster, manager } => PyRuntimeError::new_err(format!(
            "not the Manager of cluster {cluster:?}; manager_peer_id={manager}"
        )),
        RustAdmitError::AlreadyMember(pid) => {
            PyValueError::new_err(format!("peer {pid} is already a cluster member"))
        }
        RustAdmitError::Runtime(err) => PyRuntimeError::new_err(format!("runtime: {err}")),
        RustAdmitError::Stopped => PyRuntimeError::new_err("ClusterManager has been shut down"),
    }
}

fn map_fetch_participant_info_error(e: auki_domain_rs::FetchParticipantInfoError) -> PyErr {
    match e {
        auki_domain_rs::FetchParticipantInfoError::Request(err) => {
            PyOSError::new_err(format!("fetch_participant_info: {err}"))
        }
        auki_domain_rs::FetchParticipantInfoError::InvalidJson(err) => {
            PyValueError::new_err(format!("invalid ParticipantInfo JSON from peer: {err}"))
        }
        auki_domain_rs::FetchParticipantInfoError::Stopped => {
            PyRuntimeError::new_err("ClusterManager has been shut down")
        }
    }
}

fn map_fetch_resources_catalog_error(e: RustFetchResourcesCatalogError) -> PyErr {
    match e {
        RustFetchResourcesCatalogError::Request(err) => {
            PyOSError::new_err(format!("fetch_resources_catalog: {err}"))
        }
    }
}

fn map_fetch_map_catalog_error(e: RustFetchMapCatalogError) -> PyErr {
    match e {
        RustFetchMapCatalogError::UnsupportedProtocol => {
            PyRuntimeError::new_err("remote peer does not support Map discovery")
        }
        RustFetchMapCatalogError::Request(error) => {
            PyOSError::new_err(format!("fetch_map_catalog: {error}"))
        }
    }
}

fn map_fetch_registry_entry_error(e: RustFetchRegistryEntryError) -> PyErr {
    match e {
        RustFetchRegistryEntryError::Request(err) => {
            PyOSError::new_err(format!("fetch_registry_entry: {err}"))
        }
        RustFetchRegistryEntryError::NotFound { kind, id, hash } => PyFileNotFoundError::new_err(
            format!("registry entry not found: kind={kind} id={id:?} hash={hash}"),
        ),
        RustFetchRegistryEntryError::InvalidEnvelope(err) => {
            PyValueError::new_err(format!("invalid registry envelope: {err}"))
        }
        RustFetchRegistryEntryError::HashMismatch { expected, actual } => PyValueError::new_err(
            format!("registry hash mismatch: expected {expected}, got {actual}"),
        ),
        RustFetchRegistryEntryError::InvalidJson(err) => {
            PyValueError::new_err(format!("invalid registry JSON from peer: {err}"))
        }
        RustFetchRegistryEntryError::Stopped => {
            PyRuntimeError::new_err("ClusterManager has been shut down")
        }
    }
}

fn canonical_json_to_py(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("JCS output is UTF-8 JSON")
}

fn map_build_stream_manifest_error(e: RustBuildStreamManifestError) -> PyErr {
    match e {
        RustBuildStreamManifestError::SensorEntryMissing { .. }
        | RustBuildStreamManifestError::FrameEntryMissing { .. } => {
            PyFileNotFoundError::new_err(e.to_string())
        }
        RustBuildStreamManifestError::FrameIdMissing { .. }
        | RustBuildStreamManifestError::FrameHashMissing { .. }
        | RustBuildStreamManifestError::Registry(_) => PyValueError::new_err(e.to_string()),
        RustBuildStreamManifestError::Io(err) => PyOSError::new_err(err.to_string()),
    }
}

// ─── Module entry point ────────────────────────────────────────────

/// `auki_domain` Python module — post-#216 schema.
#[pymodule]
fn auki_domain(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyClusterMember>()?;
    m.add_class::<PyClusterMembership>()?;
    m.add_class::<PyDaemonInfo>()?;
    m.add_class::<PyParticipantInfo>()?;
    m.add_class::<PyResourceEntry>()?;
    m.add_class::<PyMapLogResource>()?;
    m.add_class::<PyMessageEvent>()?;
    m.add_class::<PyMessageChannelReceiver>()?;
    m.add_class::<PyReadFrom>()?;
    m.add_class::<PyStreamRequest>()?;
    m.add_class::<PyStreamManifestBuilder>()?;
    m.add_class::<PyClusterTarget>()?;
    m.add_class::<PyClusterManager>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_network::resources_protocol::{
        Available, Head, ResourceEntry, SensorBlock, SensorKind, SensorManifestPointer,
        VariantContent,
    };
    use auki_registry::{
        FrameRegistryEntry, PointField, PointFieldDataType, Rangefinder, RegistryRef, SensorBody,
        SensorRegistryEntry, write_frame, write_sensor,
    };
    use pyo3::types::{PyModule, PyString};

    const FRAME_ID: &str = "K1-AABBCCDDEEFF/head_left_cam_optical";
    const PEER_ID: &str = "K1-AABBCCDDEEFF";

    fn install_auki_network_module(py: Python<'_>) -> PyResult<()> {
        let module = PyModule::new_bound(py, "auki_network")?;
        auki_network_py::populate_module(&module)?;
        py.import_bound("sys")?
            .getattr("modules")?
            .set_item("auki_network", &module)?;
        Ok(())
    }

    fn write_spatial_registry_fixture(app_root: &std::path::Path) -> (String, String, String) {
        let frame = FrameRegistryEntry::ros_optical(PEER_ID, FRAME_ID);
        let frame_hash = write_frame(app_root, &frame).unwrap().hash().to_string();
        let entry = SensorRegistryEntry {
            peer_id: PEER_ID.into(),
            sensor_id: "K1-AABBCCDDEEFF/head_depth_points".into(),
            body: SensorBody::Rangefinder(Rangefinder {
                r#type: "point_cloud".into(),
                fields: vec![PointField {
                    name: "x".into(),
                    offset: 0,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                }],
                point_step: 4,
                is_bigendian: false,
                frame_rate_hz: 10,
                frame: RegistryRef {
                    peer_id: PEER_ID.into(),
                    id: FRAME_ID.into(),
                    hash: frame_hash.clone(),
                },
            }),
        };
        let sensor_hash = write_sensor(app_root, &entry).unwrap().hash().to_string();
        (entry.sensor_id, sensor_hash, frame_hash)
    }

    #[test]
    fn module_exposes_post_216_surface() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_domain").unwrap();
            auki_domain(py, &module).unwrap();

            assert!(module.getattr("ClusterMember").is_ok());
            assert!(module.getattr("ClusterMembership").is_ok());
            assert!(module.getattr("DaemonInfo").is_ok());
            assert!(module.getattr("ParticipantInfo").is_ok());
            assert!(module.getattr("ResourceEntry").is_ok());
            assert!(module.getattr("MapLogResource").is_ok());
            assert!(module.getattr("ReadFrom").is_ok());
            assert!(module.getattr("StreamRequest").is_ok());
            assert!(module.getattr("StreamManifestBuilder").is_ok());
            assert!(module.getattr("ClusterTarget").is_ok());
            assert!(
                module
                    .getattr("ClusterManager")
                    .unwrap()
                    .getattr("open_pose_stream")
                    .is_ok()
            );
            assert!(
                module
                    .getattr("ClusterManager")
                    .unwrap()
                    .getattr("open_stream")
                    .is_ok()
            );
            assert!(
                module
                    .getattr("ClusterManager")
                    .unwrap()
                    .getattr("fetch_map_catalog")
                    .is_ok()
            );
            assert!(
                module
                    .getattr("ClusterManager")
                    .unwrap()
                    .getattr("open_map_stream")
                    .is_ok()
            );
        });
    }

    #[test]
    fn resource_entry_getters_sensor_log() {
        Python::with_gil(|py| {
            let rust_entry = ResourceEntry {
                source_peer_id: "galbot".into(),
                writer_peer_id: "galbot".into(),
                resource_id: "head_left_rgb".into(),
                state: "live".into(),
                head: Some(Head::Rolling {
                    retention_ns: 5_000_000_000,
                }),
                extent: None,
                available: Available {
                    bytes: 3_000_000,
                    entries: 900,
                    duration_ns: 5_000_000_000,
                },
                sensor: Some(SensorBlock {
                    kind: SensorKind::Camera,
                    r#type: "rgb".into(),
                    sensor_id: "head_left_rgb".into(),
                    sensor_hash: "sh".into(),
                }),
                pose: None,
                variant_content: VariantContent::SensorLog {
                    manifest: SensorManifestPointer {
                        clock: RegistryRef {
                            peer_id: "galbot".into(),
                            id: "session/sdk_clock".into(),
                            hash: "ch".into(),
                        },
                        frame: None,
                    },
                },
            };
            let entry = PyResourceEntry { inner: rust_entry };
            assert_eq!(entry.source_peer_id(), "galbot");
            assert_eq!(entry.resource_id(), "head_left_rgb");
            assert_eq!(entry.variant(), "sensor_log");
            assert_eq!(entry.state(), "live");

            let head = entry.head(py).unwrap().unwrap();
            assert_eq!(
                head.get_item("kind")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "rolling"
            );

            let sensor = entry.sensor(py).unwrap().unwrap();
            assert_eq!(
                sensor
                    .get_item("kind")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "camera"
            );
            assert_eq!(
                sensor
                    .get_item("type")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "rgb"
            );

            let manifest = entry.manifest(py).unwrap();
            let clock = manifest
                .get_item("clock")
                .unwrap()
                .unwrap()
                .downcast::<PyDict>()
                .unwrap()
                .clone();
            assert_eq!(
                clock
                    .get_item("id")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "session/sdk_clock"
            );
        });
    }

    #[test]
    fn resource_entry_to_json_roundtrip() {
        Python::with_gil(|_py| {
            use auki_manifests::PoseSource;
            use auki_manifests::PoseWriterMode;
            use auki_network::resources_protocol::{PoseBlock, PoseManifestPointer};

            let rust_entry = ResourceEntry {
                source_peer_id: "galbot".into(),
                writer_peer_id: "galbot".into(),
                resource_id: "world->base_link".into(),
                state: "sealed".into(),
                head: None,
                extent: Some(auki_network::resources_protocol::Extent {
                    start_at_ns: 100,
                    finish_at_ns: 200,
                }),
                available: Available {
                    bytes: 512,
                    entries: 5,
                    duration_ns: 100,
                },
                sensor: None,
                pose: Some(PoseBlock {
                    writer_mode: PoseWriterMode::Rigid,
                }),
                variant_content: VariantContent::PoseLog {
                    manifest: PoseManifestPointer {
                        from_frame: RegistryRef {
                            peer_id: "park".into(),
                            id: "world".into(),
                            hash: "fh".into(),
                        },
                        to_frame: RegistryRef {
                            peer_id: "galbot".into(),
                            id: "base_link".into(),
                            hash: "th".into(),
                        },
                        clock: RegistryRef {
                            peer_id: "galbot".into(),
                            id: "session/sdk_clock".into(),
                            hash: "ch".into(),
                        },
                        source: PoseSource::Manual,
                        expected_rate_hz: 30,
                    },
                },
            };
            let entry = PyResourceEntry { inner: rust_entry };
            let json_str = entry.to_json().unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
            assert_eq!(parsed["variant"], "pose_log");
            assert_eq!(parsed["state"], "sealed");
            assert_eq!(parsed["pose"]["writer_mode"], "rigid");
            assert_eq!(parsed["extent"]["start_at_ns"], 100);
        });
    }

    #[test]
    fn read_from_factories_and_getters() {
        let latest = PyReadFrom::latest();
        assert_eq!(latest.kind(), "latest");
        assert_eq!(latest.timestamp_ns(), None);

        let from_start = PyReadFrom::from_start();
        assert_eq!(from_start.kind(), "from_start");

        let ts = PyReadFrom::from_timestamp(1_733_836_800_000_000_000);
        assert_eq!(ts.kind(), "from_timestamp");
        assert_eq!(ts.timestamp_ns(), Some(1_733_836_800_000_000_000));

        assert_eq!(PyReadFrom::latest().__repr__(), "ReadFrom.latest()");
        assert_eq!(
            PyReadFrom::from_timestamp(42).__repr__(),
            "ReadFrom.from_timestamp(42)"
        );
    }

    #[test]
    fn cluster_manager_exposes_relay_aware_create() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_domain").unwrap();
            auki_domain(py, &module).unwrap();

            let manager = module.getattr("ClusterManager").unwrap();
            assert!(
                manager
                    .getattr("create_cluster_with_relay_multiaddrs")
                    .is_ok()
            );
            assert!(
                manager
                    .getattr("create_cluster_with_relay_reservation")
                    .is_ok()
            );
        });
    }

    #[test]
    fn relay_reservation_create_validates_relay_multiaddrs_before_network() {
        Python::with_gil(|py| {
            let daemon = PyDaemonInfo::new(
                "test-app".into(),
                "test-daemon".into(),
                "session".into(),
                "clock".into(),
                "clock-hash".into(),
                "instance".into(),
            );

            let err = match PyClusterManager::create_cluster_with_relay_reservation(
                py,
                vec![7; 32],
                "relay-validation",
                "http://127.0.0.1:0",
                vec!["/ip4/0.0.0.0/tcp/0".into()],
                "not-a-multiaddr",
                "/ip4/127.0.0.1/tcp/4002/ws/p2p/12D3KooWJfVjn3XAFv5XnuACSMsPB3Uh8nCqC7zkKMNpJkgjKZBW",
                "test/0.0.0",
                &daemon,
                None,
                Some(vec!["/ip4/127.0.0.1/tcp/4001".into()]),
                1,
            ) {
                Ok(_) => panic!("invalid relay dial multiaddr should fail before network work"),
                Err(err) => err,
            };

            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(err.to_string().contains("relay_dial_multiaddr"));
        });
    }

    #[test]
    fn generic_stream_resolver_uses_sensor_log_variant() {
        let resources = vec![
            ResourceEntry {
                source_peer_id: "galbot".into(),
                writer_peer_id: "galbot".into(),
                resource_id: "head_left_rgb".into(),
                state: "live".into(),
                head: Some(Head::Rolling {
                    retention_ns: 5_000_000_000,
                }),
                extent: None,
                available: Available {
                    bytes: 0,
                    entries: 0,
                    duration_ns: 0,
                },
                sensor: Some(SensorBlock {
                    kind: SensorKind::Camera,
                    r#type: "rgb".into(),
                    sensor_id: "head_left_rgb".into(),
                    sensor_hash: "sh".into(),
                }),
                pose: None,
                variant_content: VariantContent::SensorLog {
                    manifest: SensorManifestPointer {
                        clock: RegistryRef {
                            peer_id: "g".into(),
                            id: "clk".into(),
                            hash: "ch".into(),
                        },
                        frame: None,
                    },
                },
            },
            ResourceEntry {
                source_peer_id: "galbot".into(),
                writer_peer_id: "galbot".into(),
                resource_id: "head_array_4mic".into(),
                state: "live".into(),
                head: Some(Head::Rolling {
                    retention_ns: 5_000_000_000,
                }),
                extent: None,
                available: Available {
                    bytes: 0,
                    entries: 0,
                    duration_ns: 0,
                },
                sensor: Some(SensorBlock {
                    kind: SensorKind::Audio,
                    r#type: "pcm".into(),
                    sensor_id: "head_array_4mic".into(),
                    sensor_hash: "sh".into(),
                }),
                pose: None,
                variant_content: VariantContent::SensorLog {
                    manifest: SensorManifestPointer {
                        clock: RegistryRef {
                            peer_id: "g".into(),
                            id: "clk".into(),
                            hash: "ch".into(),
                        },
                        frame: None,
                    },
                },
            },
        ];

        assert_eq!(
            resolve_generic_stream_payload_kind_from_entries(&resources, "galbot", "head_left_rgb")
                .unwrap(),
            GenericStreamPayloadKind::Camera
        );
        assert_eq!(
            resolve_generic_stream_payload_kind_from_entries(
                &resources,
                "galbot",
                "head_array_4mic",
            )
            .unwrap(),
            GenericStreamPayloadKind::Audio
        );
    }

    #[test]
    fn generic_stream_resolver_matches_source_peer_id_before_resource_id() {
        let resources = vec![
            ResourceEntry {
                source_peer_id: "galbot-a".into(),
                writer_peer_id: "park".into(),
                resource_id: "head_left_rgb".into(),
                state: "live".into(),
                head: Some(Head::Rolling {
                    retention_ns: 5_000_000_000,
                }),
                extent: None,
                available: Available {
                    bytes: 0,
                    entries: 0,
                    duration_ns: 0,
                },
                sensor: Some(SensorBlock {
                    kind: SensorKind::Camera,
                    r#type: "rgb".into(),
                    sensor_id: "head_left_rgb".into(),
                    sensor_hash: "sh".into(),
                }),
                pose: None,
                variant_content: VariantContent::SensorLog {
                    manifest: SensorManifestPointer {
                        clock: RegistryRef {
                            peer_id: "a".into(),
                            id: "clk".into(),
                            hash: "ch".into(),
                        },
                        frame: None,
                    },
                },
            },
            ResourceEntry {
                source_peer_id: "galbot-b".into(),
                writer_peer_id: "park".into(),
                resource_id: "head_left_rgb".into(),
                state: "live".into(),
                head: Some(Head::Rolling {
                    retention_ns: 5_000_000_000,
                }),
                extent: None,
                available: Available {
                    bytes: 0,
                    entries: 0,
                    duration_ns: 0,
                },
                sensor: Some(SensorBlock {
                    kind: SensorKind::Audio,
                    r#type: "pcm".into(),
                    sensor_id: "head_array_4mic".into(),
                    sensor_hash: "sh".into(),
                }),
                pose: None,
                variant_content: VariantContent::SensorLog {
                    manifest: SensorManifestPointer {
                        clock: RegistryRef {
                            peer_id: "b".into(),
                            id: "clk".into(),
                            hash: "ch".into(),
                        },
                        frame: None,
                    },
                },
            },
        ];

        assert_eq!(
            resolve_generic_stream_payload_kind_from_entries(
                &resources,
                "galbot-b",
                "head_left_rgb",
            )
            .unwrap(),
            GenericStreamPayloadKind::Audio
        );
    }

    #[test]
    fn generic_stream_resolver_uses_pose_log_variant() {
        use auki_manifests::{PoseSource, PoseWriterMode};
        use auki_network::resources_protocol::{PoseBlock, PoseManifestPointer};

        let resources = vec![ResourceEntry {
            source_peer_id: "galbot".into(),
            writer_peer_id: "galbot".into(),
            resource_id: "world->base_link".into(),
            state: "live".into(),
            head: Some(Head::Rolling {
                retention_ns: 60_000_000_000,
            }),
            extent: None,
            available: Available {
                bytes: 0,
                entries: 0,
                duration_ns: 0,
            },
            sensor: None,
            pose: Some(PoseBlock {
                writer_mode: PoseWriterMode::Movable,
            }),
            variant_content: VariantContent::PoseLog {
                manifest: PoseManifestPointer {
                    from_frame: RegistryRef {
                        peer_id: "p".into(),
                        id: "world".into(),
                        hash: "fh".into(),
                    },
                    to_frame: RegistryRef {
                        peer_id: "g".into(),
                        id: "base_link".into(),
                        hash: "th".into(),
                    },
                    clock: RegistryRef {
                        peer_id: "g".into(),
                        id: "clk".into(),
                        hash: "ch".into(),
                    },
                    source: PoseSource::Manual,
                    expected_rate_hz: 30,
                },
            },
        }];

        assert_eq!(
            resolve_generic_stream_payload_kind_from_entries(
                &resources,
                "galbot",
                "world->base_link",
            )
            .unwrap(),
            GenericStreamPayloadKind::Pose
        );
    }

    #[test]
    fn stream_manifest_builder_returns_auki_network_manifest() {
        Python::with_gil(|py| {
            install_auki_network_module(py).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let (sensor_id, sensor_hash, frame_hash) = write_spatial_registry_fixture(dir.path());
            let app_root = PyString::new_bound(py, dir.path().to_str().unwrap());

            let manifest = PyStreamManifestBuilder::from_registry(
                py,
                app_root.as_any(),
                PEER_ID,
                &sensor_id,
                &sensor_hash,
                "K1-AABBCCDDEEFF/monotonic",
                "clock-hash",
            )
            .unwrap();
            let manifest = manifest.bind(py);

            assert_eq!(
                manifest
                    .getattr("sensor_id")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                sensor_id
            );
            assert_eq!(
                manifest
                    .getattr("frame_id")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                FRAME_ID
            );
            assert_eq!(
                manifest
                    .getattr("frame_hash")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                frame_hash
            );
        });
    }

    #[test]
    fn stream_manifest_builder_missing_sensor_maps_to_file_not_found() {
        Python::with_gil(|py| {
            let dir = tempfile::tempdir().unwrap();
            let app_root = PyString::new_bound(py, dir.path().to_str().unwrap());

            let err = PyStreamManifestBuilder::from_registry(
                py,
                app_root.as_any(),
                "missing-peer",
                "missing/sensor",
                "missing-hash",
                "clock",
                "clock-hash",
            )
            .unwrap_err();

            assert!(err.is_instance_of::<PyFileNotFoundError>(py));
        });
    }
}
