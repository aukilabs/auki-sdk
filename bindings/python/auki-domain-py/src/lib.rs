//! Python bindings for `auki-domain`.
//!
//! Surface (under the `auki_domain` Python module):
//!
//! - `ClusterMembership` / `ClusterMember` — value-type pyclasses
//!   mirroring the Rust types.
//! - `DaemonInfo` — value-type pyclass the daemon constructs and
//!   passes to `ClusterManager.participant_info`.
//! - `ParticipantInfo` — the SDK-provided `/api/info` wire shape;
//!   produced by `ClusterManager.participant_info`. Has a
//!   `.to_json()` method daemons serve verbatim on their HTTP
//!   surface.
//! - `ClusterManager(...)` — the daemon-side cluster handle. Sync
//!   constructors (`create_cluster`, `create_cluster_with_relay_multiaddrs`)
//!   and methods, each `block_on`s on a process-wide multi-thread tokio
//!   runtime.

use auki_domain_rs::{
    AdmitError as RustAdmitError, BootstrapError as RustBootstrapError,
    BuildStreamManifestError as RustBuildStreamManifestError, ClusterManager as RustClusterManager,
    ClusterMember as RustClusterMember, ClusterMembership as RustClusterMembership,
    ClusterTarget as RustClusterTarget, CreateClusterError as RustCreateClusterError,
    DaemonInfo as RustDaemonInfo, FetchRegistryEntryError as RustFetchRegistryEntryError,
    FetchResourcesCatalogError as RustFetchResourcesCatalogError,
    JoinClusterError as RustJoinClusterError,
    ResourceCatalogProvider as RustResourceCatalogProvider, ResourceEntry as RustResourceEntry,
    ResourcePinholeIntrinsics as RustResourcePinholeIntrinsics, ResourceQuat as RustResourceQuat,
    ResourceSpatialTransform as RustResourceSpatialTransform, ResourceVec3 as RustResourceVec3,
    ResourcesRequest as RustResourcesRequest, SensorCatalogProvider as RustSensorCatalogProvider,
    SensorEntry as RustSensorEntry, SensorStreamResource as RustSensorStreamResource,
    SensorsRequest as RustSensorsRequest, StreamManifestBuilder as RustStreamManifestBuilder,
    TransformEdgeResource as RustTransformEdgeResource,
};
use auki_identity::Wallet;
use auki_network::ParticipantInfo as RustParticipantInfo;
use auki_network::PeerIdentity;
use auki_network::discovery_client::DiscoveryError as RustDiscoveryError;
use auki_network::stream_protocol::{
    CameraFrame as RustCameraFrame, StreamManifest as RustStreamManifest,
    StreamRequest as RustStreamRequest, audio::Data as RustAudioFrame,
    joint_encoders::Data as RustJointEncodersFrame, point_cloud::Data as RustPointCloudFrame,
};
use auki_network::stream_runtime::{StreamProvider, decline_all_streams};
use auki_network::swarm::{SwarmConfig, build_swarm};
use auki_network_py::PyClusterEntry;
use auki_network_py::stream_types::{
    PyStreamSubscription, STREAM_PROVIDER_CAPSULE_NAME, open_stream_error_to_pyerr,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenericStreamPayloadKind {
    Camera,
    PointCloud,
    JointEncoders,
    Audio,
}

fn resolve_generic_stream_payload_kind(
    resources: &[RustResourceEntry],
    sensor_id: &str,
) -> Result<GenericStreamPayloadKind, PyErr> {
    let Some(resource) = resources.iter().find_map(|resource| match resource {
        RustResourceEntry::SensorStream(stream) if stream.sensor_id == sensor_id => Some(stream),
        _ => None,
    }) else {
        return Err(PyFileNotFoundError::new_err(format!(
            "sensor stream {sensor_id:?} not found in remote resource catalog"
        )));
    };

    generic_stream_payload_kind_for_resource(resource).ok_or_else(|| {
        PyValueError::new_err(format!(
            "sensor stream {sensor_id:?} advertises unsupported payload {:?} for sensor kind {:?}",
            resource.payload, resource.sensor_kind
        ))
    })
}

fn generic_stream_payload_kind_for_resource(
    resource: &RustSensorStreamResource,
) -> Option<GenericStreamPayloadKind> {
    match resource.payload.as_str() {
        "camera_frame" => Some(GenericStreamPayloadKind::Camera),
        "point_cloud_frame" => Some(GenericStreamPayloadKind::PointCloud),
        "joint_encoders_frame" => Some(GenericStreamPayloadKind::JointEncoders),
        "audio_frame" => Some(GenericStreamPayloadKind::Audio),
        _ => None,
    }
}

// ─── Stream provider bridge ────────────────────────────────────────────
//
// `auki-network-py` and `auki-domain-py` are separate Python extension
// modules. Although both link the `auki-network-py` rlib, each `.so`
// gets its own copy of every `#[pyclass]`'s PyType registration with a
// distinct type-id. A `PyStreamDecision` created by user code (via
// `auki_network.cluster.StreamDecision.accept_camera(...)` → registered in
// `auki_network.so`) is therefore NOT extractable as
// `PyRef<PyStreamDecision>` from inside `auki_domain.so` — the
// type-ids mismatch with the misleading error
// `'StreamDecision' object cannot be converted to 'StreamDecision'`.
//
// To dodge that, `auki_domain.so` doesn't call the rlib's
// `build_stream_provider` directly. It imports the public Python
// helper `auki_network.cluster._build_stream_provider`, which runs
// inside `auki_network.so` (where the type-ids match), and ships back
// the resulting `Arc<StreamProvider>` via a `PyCapsule`. We unbox the
// Arc here, clone it, and hand it to the cluster runtime as before.

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
    // SAFETY: we just verified the capsule's name; by contract the
    // payload is a `StreamProvider` (which is `Arc<dyn Fn>` and so
    // memory-layout-stable across crate boundaries within this
    // process). Both crates share the same `auki_network` rlib
    // version, so the trait object's vtable is consistent. Cloning
    // the Arc bumps its refcount; the capsule retains its own
    // reference until Python GC drops it.
    let provider_ref: &StreamProvider = unsafe { capsule.reference::<StreamProvider>() };
    Ok(Arc::clone(provider_ref))
}

fn stream_manifest_to_python(py: Python<'_>, manifest: RustStreamManifest) -> PyResult<Py<PyAny>> {
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

/// SDK-provided `/api/info` wire shape. Produced by
/// `ClusterManager.participant_info`. Serve verbatim on the
/// daemon's Control API.
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

    /// Serialize to the canonical `/api/info` JSON shape. Daemons
    /// return this string verbatim from their HTTP handler.
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

// ─── SensorEntry pyclass ───────────────────────────────────────────

/// One row in a peer's sensor catalog. Produced by
/// `ClusterManager.fetch_sensors_catalog(peer_id)` (consumer side)
/// and supplied by the daemon's catalog provider callable
/// (producer side, via `ClusterManager.set_sensor_catalog_provider`).
#[pyclass(name = "SensorEntry")]
#[derive(Clone)]
pub struct PySensorEntry {
    inner: RustSensorEntry,
}

#[pymethods]
impl PySensorEntry {
    #[new]
    #[pyo3(signature = (sensor_id, sensor_hash, kind, sensor_entry_json = None, frame_entry_json = None))]
    fn new(
        sensor_id: String,
        sensor_hash: String,
        kind: String,
        sensor_entry_json: Option<String>,
        frame_entry_json: Option<String>,
    ) -> Self {
        Self {
            inner: RustSensorEntry {
                sensor_id,
                sensor_hash,
                kind,
                sensor_entry_json,
                frame_entry_json,
            },
        }
    }

    #[getter]
    fn sensor_id(&self) -> String {
        self.inner.sensor_id.clone()
    }

    #[getter]
    fn sensor_hash(&self) -> String {
        self.inner.sensor_hash.clone()
    }

    #[getter]
    fn kind(&self) -> String {
        self.inner.kind.clone()
    }

    #[getter]
    fn sensor_entry_json(&self) -> Option<String> {
        self.inner.sensor_entry_json.clone()
    }

    #[getter]
    fn frame_entry_json(&self) -> Option<String> {
        self.inner.frame_entry_json.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "SensorEntry(sensor_id={:?}, sensor_hash={:?}, kind={:?}, sensor_entry_json={}, frame_entry_json={})",
            self.inner.sensor_id,
            self.inner.sensor_hash,
            self.inner.kind,
            self.inner.sensor_entry_json.is_some(),
            self.inner.frame_entry_json.is_some(),
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Adapter: wraps a Python callable returning `list[SensorEntry]` in
/// a Rust `SensorCatalogProvider`. Called from the inbound
/// `/auki/sensors/0.0.1` handler task — re-acquires the GIL on each
/// snapshot.
struct PySensorCatalogProvider {
    callable: Py<PyAny>,
}

impl RustSensorCatalogProvider for PySensorCatalogProvider {
    fn snapshot(&self) -> Vec<RustSensorEntry> {
        Python::with_gil(|py| {
            let result = self
                .callable
                .bind(py)
                .call0()
                .and_then(|res| res.extract::<Vec<PyRef<PySensorEntry>>>());
            match result {
                Ok(entries) => entries.into_iter().map(|e| e.inner.clone()).collect(),
                Err(e) => {
                    eprintln!("auki-domain-py: sensor_catalog_provider callable failed: {e}");
                    Vec::new()
                }
            }
        })
    }
}

// ─── Resource catalog pyclasses ─────────────────────────────────────

/// Numeric pinhole camera intrinsics for projection.
#[pyclass(name = "ResourcePinholeIntrinsics")]
#[derive(Clone)]
pub struct PyResourcePinholeIntrinsics {
    inner: RustResourcePinholeIntrinsics,
}

#[pymethods]
impl PyResourcePinholeIntrinsics {
    #[new]
    fn new(fx: f64, fy: f64, cx: f64, cy: f64) -> Self {
        Self {
            inner: RustResourcePinholeIntrinsics { fx, fy, cx, cy },
        }
    }

    #[getter]
    fn fx(&self) -> f64 {
        self.inner.fx
    }

    #[getter]
    fn fy(&self) -> f64 {
        self.inner.fy
    }

    #[getter]
    fn cx(&self) -> f64 {
        self.inner.cx
    }

    #[getter]
    fn cy(&self) -> f64 {
        self.inner.cy
    }

    fn __repr__(&self) -> String {
        format!(
            "ResourcePinholeIntrinsics(fx={}, fy={}, cx={}, cy={})",
            self.inner.fx, self.inner.fy, self.inner.cx, self.inner.cy
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// 3D vector used by resource transform rows.
#[pyclass(name = "ResourceVec3")]
#[derive(Clone)]
pub struct PyResourceVec3 {
    inner: RustResourceVec3,
}

#[pymethods]
impl PyResourceVec3 {
    #[new]
    fn new(x: f64, y: f64, z: f64) -> Self {
        Self {
            inner: RustResourceVec3 { x, y, z },
        }
    }

    #[getter]
    fn x(&self) -> f64 {
        self.inner.x
    }

    #[getter]
    fn y(&self) -> f64 {
        self.inner.y
    }

    #[getter]
    fn z(&self) -> f64 {
        self.inner.z
    }

    fn __repr__(&self) -> String {
        format!(
            "ResourceVec3(x={}, y={}, z={})",
            self.inner.x, self.inner.y, self.inner.z
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Hamilton quaternion used by resource transform rows.
#[pyclass(name = "ResourceQuat")]
#[derive(Clone)]
pub struct PyResourceQuat {
    inner: RustResourceQuat,
}

#[pymethods]
impl PyResourceQuat {
    #[new]
    fn new(x: f64, y: f64, z: f64, w: f64) -> Self {
        Self {
            inner: RustResourceQuat { x, y, z, w },
        }
    }

    #[getter]
    fn x(&self) -> f64 {
        self.inner.x
    }

    #[getter]
    fn y(&self) -> f64 {
        self.inner.y
    }

    #[getter]
    fn z(&self) -> f64 {
        self.inner.z
    }

    #[getter]
    fn w(&self) -> f64 {
        self.inner.w
    }

    fn __repr__(&self) -> String {
        format!(
            "ResourceQuat(x={}, y={}, z={}, w={})",
            self.inner.x, self.inner.y, self.inner.z, self.inner.w
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Rigid transform payload carried by `TransformEdgeResource`.
#[pyclass(name = "ResourceSpatialTransform")]
#[derive(Clone)]
pub struct PyResourceSpatialTransform {
    inner: RustResourceSpatialTransform,
}

#[pymethods]
impl PyResourceSpatialTransform {
    #[new]
    fn new(translation: PyRef<'_, PyResourceVec3>, orientation: PyRef<'_, PyResourceQuat>) -> Self {
        Self {
            inner: RustResourceSpatialTransform {
                translation: translation.inner,
                orientation: orientation.inner,
            },
        }
    }

    #[getter]
    fn translation(&self) -> PyResourceVec3 {
        PyResourceVec3 {
            inner: self.inner.translation,
        }
    }

    #[getter]
    fn orientation(&self) -> PyResourceQuat {
        PyResourceQuat {
            inner: self.inner.orientation,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ResourceSpatialTransform(translation={:?}, orientation={:?})",
            self.inner.translation, self.inner.orientation
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Live sensor stream resource row from `/auki/resources/0.0.1`.
#[pyclass(name = "SensorStreamResource")]
#[derive(Clone)]
pub struct PySensorStreamResource {
    inner: RustSensorStreamResource,
}

#[pymethods]
impl PySensorStreamResource {
    #[new]
    #[pyo3(signature = (
        id,
        sensor_id,
        sensor_hash,
        sensor_kind,
        stream_protocol,
        payload,
        pinhole_intrinsics = None,
        sensor_entry_json = None,
        frame_entry_json = None
    ))]
    fn new(
        id: String,
        sensor_id: String,
        sensor_hash: String,
        sensor_kind: String,
        stream_protocol: String,
        payload: String,
        pinhole_intrinsics: Option<&Bound<'_, PyAny>>,
        sensor_entry_json: Option<String>,
        frame_entry_json: Option<String>,
    ) -> PyResult<Self> {
        let pinhole_intrinsics = match pinhole_intrinsics {
            Some(value) => Some(
                value
                    .extract::<PyRef<'_, PyResourcePinholeIntrinsics>>()?
                    .inner,
            ),
            None => None,
        };
        Ok(Self {
            inner: RustSensorStreamResource {
                id,
                sensor_id,
                sensor_hash,
                sensor_kind,
                stream_protocol,
                payload,
                pinhole_intrinsics,
                sensor_entry_json,
                frame_entry_json,
            },
        })
    }

    #[getter]
    fn kind(&self) -> &'static str {
        "sensor_stream"
    }

    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    #[getter]
    fn sensor_id(&self) -> String {
        self.inner.sensor_id.clone()
    }

    #[getter]
    fn sensor_hash(&self) -> String {
        self.inner.sensor_hash.clone()
    }

    #[getter]
    fn sensor_kind(&self) -> String {
        self.inner.sensor_kind.clone()
    }

    #[getter]
    fn stream_protocol(&self) -> String {
        self.inner.stream_protocol.clone()
    }

    #[getter]
    fn payload(&self) -> String {
        self.inner.payload.clone()
    }

    #[getter]
    fn pinhole_intrinsics(&self) -> Option<PyResourcePinholeIntrinsics> {
        self.inner
            .pinhole_intrinsics
            .map(|inner| PyResourcePinholeIntrinsics { inner })
    }

    #[getter]
    fn sensor_entry_json(&self) -> Option<String> {
        self.inner.sensor_entry_json.clone()
    }

    #[getter]
    fn frame_entry_json(&self) -> Option<String> {
        self.inner.frame_entry_json.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "SensorStreamResource(id={:?}, sensor_id={:?}, sensor_hash={:?}, sensor_kind={:?}, payload={:?}, pinhole_intrinsics={})",
            self.inner.id,
            self.inner.sensor_id,
            self.inner.sensor_hash,
            self.inner.sensor_kind,
            self.inner.payload,
            self.inner.pinhole_intrinsics.is_some(),
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Direct rigid transform edge resource row from `/auki/resources/0.0.1`.
#[pyclass(name = "TransformEdgeResource")]
#[derive(Clone)]
pub struct PyTransformEdgeResource {
    inner: RustTransformEdgeResource,
}

#[pymethods]
impl PyTransformEdgeResource {
    #[new]
    #[pyo3(signature = (
        id,
        from_frame_id,
        from_frame_hash,
        to_frame_id,
        to_frame_hash,
        writer_mode,
        transform,
        source_json = None,
        from_frame_entry_json = None,
        to_frame_entry_json = None
    ))]
    fn new(
        id: String,
        from_frame_id: String,
        from_frame_hash: String,
        to_frame_id: String,
        to_frame_hash: String,
        writer_mode: String,
        transform: PyRef<'_, PyResourceSpatialTransform>,
        source_json: Option<String>,
        from_frame_entry_json: Option<String>,
        to_frame_entry_json: Option<String>,
    ) -> PyResult<Self> {
        let source = match source_json {
            Some(json) => Some(serde_json::from_str(&json).map_err(|e| {
                PyValueError::new_err(format!("source_json must be valid JSON: {e}"))
            })?),
            None => None,
        };
        Ok(Self {
            inner: RustTransformEdgeResource {
                id,
                from_frame_id,
                from_frame_hash,
                to_frame_id,
                to_frame_hash,
                writer_mode,
                source,
                transform: transform.inner,
                from_frame_entry_json,
                to_frame_entry_json,
            },
        })
    }

    #[getter]
    fn kind(&self) -> &'static str {
        "transform_edge"
    }

    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    #[getter]
    fn from_frame_id(&self) -> String {
        self.inner.from_frame_id.clone()
    }

    #[getter]
    fn from_frame_hash(&self) -> String {
        self.inner.from_frame_hash.clone()
    }

    #[getter]
    fn to_frame_id(&self) -> String {
        self.inner.to_frame_id.clone()
    }

    #[getter]
    fn to_frame_hash(&self) -> String {
        self.inner.to_frame_hash.clone()
    }

    #[getter]
    fn writer_mode(&self) -> String {
        self.inner.writer_mode.clone()
    }

    #[getter]
    fn source_json(&self) -> Option<String> {
        self.inner
            .source
            .as_ref()
            .map(|value| serde_json::to_string(value).expect("serde_json::Value serializes"))
    }

    #[getter]
    fn transform(&self) -> PyResourceSpatialTransform {
        PyResourceSpatialTransform {
            inner: self.inner.transform,
        }
    }

    #[getter]
    fn from_frame_entry_json(&self) -> Option<String> {
        self.inner.from_frame_entry_json.clone()
    }

    #[getter]
    fn to_frame_entry_json(&self) -> Option<String> {
        self.inner.to_frame_entry_json.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "TransformEdgeResource(id={:?}, from_frame_id={:?}, to_frame_id={:?}, writer_mode={:?})",
            self.inner.id, self.inner.from_frame_id, self.inner.to_frame_id, self.inner.writer_mode
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Adapter: wraps a Python callable returning a list containing
/// `SensorStreamResource` and/or `TransformEdgeResource` objects in a
/// Rust `ResourceCatalogProvider`.
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
        PyTypeError::new_err(
            "resource catalog provider must return an iterable of SensorStreamResource or TransformEdgeResource",
        )
    })?;
    let mut resources = Vec::new();
    for item in iter {
        let item: Bound<'_, PyAny> = item?;
        if let Ok(sensor) = item.extract::<PyRef<'_, PySensorStreamResource>>() {
            resources.push(RustResourceEntry::SensorStream(sensor.inner.clone()));
            continue;
        }
        if let Ok(edge) = item.extract::<PyRef<'_, PyTransformEdgeResource>>() {
            resources.push(RustResourceEntry::TransformEdge(edge.inner.clone()));
            continue;
        }
        return Err(PyTypeError::new_err(
            "resource catalog provider returned an item that is not SensorStreamResource or TransformEdgeResource",
        ));
    }
    Ok(resources)
}

fn resource_entry_to_py(py: Python<'_>, entry: RustResourceEntry) -> PyResult<PyObject> {
    match entry {
        RustResourceEntry::SensorStream(inner) => {
            Ok(Py::new(py, PySensorStreamResource { inner })?.into_py(py))
        }
        RustResourceEntry::TransformEdge(inner) => {
            Ok(Py::new(py, PyTransformEdgeResource { inner })?.into_py(py))
        }
        RustResourceEntry::PoseStream(_) => Err(PyTypeError::new_err(
            "PoseStreamResource is not exposed by auki-domain-py yet",
        )),
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
    /// Spatial sensor bodies (`Camera`, `PointCloud`) must carry a
    /// non-empty `frame_id` + `frame_hash`, and the exact frame entry
    /// must exist on disk. Non-spatial bodies (`Audio`,
    /// `JointEncoders`) return empty frame fields.
    #[staticmethod]
    #[pyo3(signature = (app_root, sensor_id, sensor_hash, clock_id, clock_hash))]
    fn from_registry(
        py: Python<'_>,
        app_root: &Bound<'_, PyAny>,
        sensor_id: &str,
        sensor_hash: &str,
        clock_id: &str,
        clock_hash: &str,
    ) -> PyResult<Py<PyAny>> {
        let app_root = pathlike_to_pathbuf(py, app_root, "app_root")?;
        let sensor_id = sensor_id.to_string();
        let sensor_hash = sensor_hash.to_string();
        let clock_id = clock_id.to_string();
        let clock_hash = clock_hash.to_string();

        let manifest = py
            .allow_threads(|| {
                RustStreamManifestBuilder::from_registry(
                    &app_root,
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

// ─── ClusterManager pyclass ────────────────────────────────────────

/// Daemon-side cluster handle. The SDK owns the libp2p swarm, the
/// `NetworkRuntime`, the Discovery client, the cluster membership
/// document, and the Manager-side Discovery heartbeat tick — the
/// daemon constructs one of these and treats it as a single object.
///
/// Construct with `create_cluster(...)` (you become the initial
/// Manager). Join-existing-cluster lands in a follow-up commit once
/// the libp2p join protocol ships.
// ─── ClusterTarget ──────────────────────────────────────────────────
//
// Tagged-union mirror of `auki_domain::ClusterTarget`. Static factory
// methods so Python construction reads like the Rust constructors:
// `ClusterTarget.most_recent_or_create("hagall")` etc.

/// Policy declaration for [`ClusterManager.bootstrap`]. Construct via
/// the static factories; the bare variant is opaque to Python.
#[pyclass(name = "ClusterTarget", frozen)]
#[derive(Clone)]
pub struct PyClusterTarget {
    pub(crate) inner: RustClusterTarget,
}

#[pymethods]
impl PyClusterTarget {
    /// Create a new cluster named `name`. Errors with
    /// `RuntimeError("cluster '...' already exists ...")` if the name
    /// is taken in Discovery's directory.
    #[staticmethod]
    fn create(name: &str) -> Self {
        Self {
            inner: RustClusterTarget::create(name),
        }
    }

    /// Join an existing cluster named `name`. Errors with
    /// `RuntimeError("cluster '...' not in Discovery directory")` if
    /// the cluster is missing.
    #[staticmethod]
    fn join(name: &str) -> Self {
        Self {
            inner: RustClusterTarget::join(name),
        }
    }

    /// Join `name` if it exists in Discovery's directory; otherwise
    /// create it. Headless daemons with a specific name configured
    /// (e.g. `--cluster-name foo`) use this.
    #[staticmethod]
    fn join_or_create(name: &str) -> Self {
        Self {
            inner: RustClusterTarget::join_or_create(name),
        }
    }

    /// Join the most-recently-created cluster from Discovery's
    /// directory; if the directory is empty, create with
    /// `fallback_name`. Headless daemons with no specific name
    /// configured (e.g. Boosterapp without `--cluster-name`) use this.
    #[staticmethod]
    fn most_recent_or_create(fallback_name: &str) -> Self {
        Self {
            inner: RustClusterTarget::most_recent_or_create(fallback_name),
        }
    }

    /// Discriminator: `"create"`, `"join"`, `"join_or_create"`, or
    /// `"most_recent_or_create"`. Read-only inspection.
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            RustClusterTarget::Create { .. } => "create",
            RustClusterTarget::Join { .. } => "join",
            RustClusterTarget::JoinOrCreate { .. } => "join_or_create",
            RustClusterTarget::MostRecentOrCreate { .. } => "most_recent_or_create",
        }
    }

    /// The cluster name carried by the variant: the cluster to act
    /// on for `create` / `join` / `join_or_create`; the fallback name
    /// for `most_recent_or_create`.
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
    /// Snapshot Discovery's cluster directory. **The SDK-fronted way
    /// for app daemons to read Discovery state** — apps should not
    /// construct their own `DiscoveryClient` (per Hagall constraint
    /// #5, the SDK owns Discovery-talking).
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

    /// Policy-driven cluster bootstrap. **The single entry point for
    /// headless daemons** (Boosterapp, Sentinel) — declares intent via
    /// `target` and the SDK does list + decide + create-or-join
    /// internally.
    ///
    /// Park-style UIs with explicit Create / Join buttons can still
    /// call `create_cluster(...)` / `join_cluster(...)` directly when
    /// operator intent is unambiguous.
    ///
    /// `target`: a `ClusterTarget` constructed via
    /// `ClusterTarget.create(name)` / `.join(name)` /
    /// `.join_or_create(name)` / `.most_recent_or_create(fallback_name)`.
    /// See the [Rust `ClusterTarget` docs](https://docs.rs/auki-domain)
    /// for the dispatch semantics.
    ///
    /// All other kwargs match `create_cluster` / `join_cluster`.
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

    /// Create a new cluster, becoming its initial Manager. Returns a
    /// `ClusterManager` you can read membership / role state from
    /// and feed into `participant_info` for `/api/info`.
    ///
    /// - `wallet_seed`: 32 bytes; deterministically derives the libp2p
    ///   peer identity for this daemon.
    /// - `cluster_name`: the cluster's name. Discovery accepts
    ///   `^[A-Za-z0-9_-]{1,64}$`.
    /// - `discovery_url`: base URL of the Discovery service (e.g.
    ///   `http://192.168.9.130:8080`).
    /// - `listen_addresses`: libp2p multiaddrs the swarm will listen
    ///   on (e.g. `["/ip4/0.0.0.0/tcp/0"]`). The runtime listens
    ///   here; unless `external_addresses` is set, the daemon
    ///   advertises the auto-detected routable subset to Discovery.
    /// - `agent_version`: `agent_version` string in libp2p
    ///   `identify` exchanges. Convention `"<app>/<version>"`.
    /// - `external_addresses`: optional operator override. If
    ///   provided and non-empty, the daemon advertises EXACTLY these
    ///   addresses to Discovery, skipping auto-detection. Use to
    ///   resolve multi-NIC ambiguity, container / VM host mappings,
    ///   or to advertise an explicitly managed address. Replace-semantics:
    ///   the SDK does NOT mix these with auto-detected addresses. Use
    ///   `create_cluster_with_relay_multiaddrs(...)` when the Manager
    ///   should also publish separate browser-compatible Relay hints.
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

    /// Create a new cluster and include separate browser-compatible
    /// Relay hints in Discovery's cluster entry.
    ///
    /// All args match `create_cluster(...)`, with the additional
    /// `relay_multiaddrs` list. These relay addresses are not local
    /// Manager listen addresses; they are browser-dialable relay
    /// multiaddrs suffixed with `/p2p/<relay-peer-id>`.
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

    /// Join an existing cluster by talking to its Manager. Looks
    /// the cluster up in Discovery, opens a libp2p
    /// `/auki/join/0.0.1` substream to the Manager, sends a join
    /// request, parses the Manager's gossiped membership, and
    /// returns a `ClusterManager` with `is_manager = False`.
    ///
    /// Same kwargs as `create_cluster` (including the
    /// `external_addresses` operator override). The cluster MUST
    /// already exist on Discovery; otherwise raises `RuntimeError`.
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

    /// Snapshot of cluster membership. Returns a `ClusterMembership`
    /// pyclass.
    fn membership(&self) -> PyResult<PyClusterMembership> {
        self.with_inner(|m| {
            Ok(PyClusterMembership {
                inner: m.membership(),
            })
        })
    }

    /// Admit a new peer to the cluster (Manager-only). The runtime's
    /// allow-list is extended; the new entry is returned.
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

    /// Open a camera stream subscription on `peer_id` for `sensor_id`.
    /// Returns a `StreamSubscription` whose `.entries()` iterator
    /// yields `StreamEntry(payload=CameraFrame(frame=...))` values.
    /// Raises `auki_network.cluster.StreamDeclined` /
    /// `StreamUnreachable` / `StreamProtocolError` on failure.
    fn open_camera_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustCameraFrame>(py, peer_id, sensor_id, |sub| {
            PyStreamSubscription::from_rust_camera(sub)
        })
    }

    /// Open a PointCloud stream subscription on `peer_id` for
    /// `sensor_id`. Returns a `StreamSubscription` whose `.entries()`
    /// iterator yields `StreamEntry(payload=PointCloudFrame(...))`.
    fn open_pointcloud_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustPointCloudFrame>(py, peer_id, sensor_id, |sub| {
            PyStreamSubscription::from_rust_pointcloud(sub)
        })
    }

    /// Open a JointEncoders stream subscription on `peer_id` for
    /// `sensor_id`. Returns a `StreamSubscription` whose `.entries()`
    /// iterator yields
    /// `StreamEntry(payload=JointEncodersFrame(angles_rad=...))`.
    fn open_joint_encoders_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustJointEncodersFrame>(py, peer_id, sensor_id, |sub| {
            PyStreamSubscription::from_rust_joint_encoders(sub)
        })
    }

    /// Open an Audio stream subscription on `peer_id` for `sensor_id`
    /// (Dialogue Batch 1). Returns a `StreamSubscription` whose
    /// `.entries()` iterator yields
    /// `StreamEntry(payload=AudioFrame(data=...))`. Sample format,
    /// channels, sample rate, and channel layout for the PCM bytes are
    /// resolved out-of-band via `(sensor_id, sensor_hash) →
    /// SensorBody::Audio` at handshake; the wire payload is
    /// opaque-bytes.
    fn open_audio_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustAudioFrame>(py, peer_id, sensor_id, |sub| {
            PyStreamSubscription::from_rust_audio(sub)
        })
    }

    /// Open a stream subscription on `peer_id` for `sensor_id` without
    /// requiring the Python caller to choose a payload-specific opener.
    ///
    /// The SDK fetches the peer's resource catalog, resolves the
    /// matching `sensor_stream` row to the advertised stream payload,
    /// then delegates internally to the typed Rust subscription. The
    /// returned `StreamSubscription.entries()` iterator still yields the
    /// existing typed payload pyclasses (`CameraFrame`,
    /// `PointCloudFrame`, `JointEncodersFrame`, or `AudioFrame`).
    fn open_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        match self.resolve_stream_payload_kind(py, peer_id, sensor_id)? {
            GenericStreamPayloadKind::Camera => {
                self.open_typed_stream::<RustCameraFrame>(py, peer_id, sensor_id, |sub| {
                    PyStreamSubscription::from_rust_camera(sub)
                })
            }
            GenericStreamPayloadKind::PointCloud => {
                self.open_typed_stream::<RustPointCloudFrame>(py, peer_id, sensor_id, |sub| {
                    PyStreamSubscription::from_rust_pointcloud(sub)
                })
            }
            GenericStreamPayloadKind::JointEncoders => self
                .open_typed_stream::<RustJointEncodersFrame>(py, peer_id, sensor_id, |sub| {
                    PyStreamSubscription::from_rust_joint_encoders(sub)
                }),
            GenericStreamPayloadKind::Audio => {
                self.open_typed_stream::<RustAudioFrame>(py, peer_id, sensor_id, |sub| {
                    PyStreamSubscription::from_rust_audio(sub)
                })
            }
        }
    }

    /// Build a fresh `ParticipantInfo` snapshot. Combines the
    /// stored daemon-side identity (passed at construction via
    /// `DaemonInfo`) with SDK-tracked dynamic fields. Daemons serve
    /// this verbatim on their Control API's `GET /api/info`.
    fn participant_info(&self) -> PyResult<PyParticipantInfo> {
        self.with_inner(|m| {
            Ok(PyParticipantInfo {
                inner: m.participant_info(),
            })
        })
    }

    /// Fetch a cluster peer's `ParticipantInfo` over the
    /// `/auki/info/0.0.1` libp2p protocol. `peer_id` must be a
    /// current cluster member (otherwise the runtime's allow-list
    /// refuses the substream). Returns a Python `ParticipantInfo`
    /// equivalent to what that peer's own `participant_info()`
    /// would return.
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

    /// Register (or replace) the application-supplied sensor
    /// catalog provider. `callable` must be a zero-argument Python
    /// callable returning a `list[SensorEntry]`. Called by the SDK
    /// once per inbound `/auki/sensors/0.0.1` request from a
    /// cluster peer.
    fn set_sensor_catalog_provider(&self, callable: Py<PyAny>) -> PyResult<()> {
        let provider = Arc::new(PySensorCatalogProvider { callable });
        self.with_inner(|m| {
            m.set_sensor_catalog_provider(provider);
            Ok(())
        })
    }

    /// Register (or replace) the application-supplied resource
    /// catalog provider. `callable` must be a zero-argument Python
    /// callable returning a list containing `SensorStreamResource`
    /// and/or `TransformEdgeResource` objects. Called by the SDK once
    /// per inbound `/auki/resources/0.0.1` request from a cluster
    /// peer.
    fn set_resource_catalog_provider(&self, callable: Py<PyAny>) -> PyResult<()> {
        let provider = Arc::new(PyResourceCatalogProvider { callable });
        self.with_inner(|m| {
            m.set_resource_catalog_provider(provider);
            Ok(())
        })
    }

    /// Register (or replace) the app root used to serve hash-pinned
    /// registry entries over `/auki/registries/0.0.1`.
    ///
    /// `app_root` accepts `str` or any `os.PathLike` resolving to a
    /// string. Producer daemons should call this after construction so
    /// peers can fetch existing entries from
    /// `<app_root>/registries/{sensors,clocks,frames}/...`.
    fn set_registry_app_root(&self, py: Python<'_>, app_root: &Bound<'_, PyAny>) -> PyResult<()> {
        let path = pathlike_to_pathbuf(py, app_root, "app_root")?;
        self.with_inner(|m| {
            m.set_registry_app_root(path);
            Ok(())
        })
    }

    /// Fetch a cluster peer's current sensor catalog over the
    /// `/auki/sensors/0.0.1` libp2p protocol. `peer_id` must be a
    /// current cluster member (otherwise the runtime's allow-list
    /// refuses the substream). Returns a Python
    /// `list[SensorEntry]` — empty list if the target peer has not
    /// registered a catalog provider (NOT an error).
    #[pyo3(signature = (peer_id, include_registry_entries = false, include_frame_entries = false))]
    fn fetch_sensors_catalog(
        &self,
        py: Python<'_>,
        peer_id: &str,
        include_registry_entries: bool,
        include_frame_entries: bool,
    ) -> PyResult<Vec<PySensorEntry>> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
                let resp = manager
                    .fetch_sensors_catalog_with(
                        peer_id_parsed,
                        RustSensorsRequest {
                            include_registry_entries,
                            include_frame_entries,
                        },
                    )
                    .await
                    .map_err(map_fetch_sensors_catalog_error)?;
                Ok(resp
                    .sensors
                    .into_iter()
                    .map(|inner| PySensorEntry { inner })
                    .collect())
            })
        })
    }

    /// Fetch a cluster peer's current resource catalog over
    /// `/auki/resources/0.0.1`. Returns a Python list containing
    /// `SensorStreamResource` and/or `TransformEdgeResource` objects.
    ///
    /// `kinds` can filter by open-string kind, e.g.
    /// `["sensor_stream"]` or `["transform_edge"]`.
    #[pyo3(signature = (
        peer_id,
        kinds = None,
        include_sensor_entries = false,
        include_frame_entries = false
    ))]
    fn fetch_resources_catalog(
        &self,
        py: Python<'_>,
        peer_id: &str,
        kinds: Option<Vec<String>>,
        include_sensor_entries: bool,
        include_frame_entries: bool,
    ) -> PyResult<Vec<PyObject>> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
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
                            kinds: kinds.unwrap_or_default(),
                            include_sensor_entries,
                            include_frame_entries,
                            include_clock_entries: false,
                        },
                    )
                    .await
                    .map_err(map_fetch_resources_catalog_error)?;
                Python::with_gil(|py| {
                    resp.resources
                        .into_iter()
                        .map(|entry| resource_entry_to_py(py, entry))
                        .collect()
                })
            })
        })
    }

    /// Fetch and verify a peer's Sensor Registry entry over
    /// `/auki/registries/0.0.1`. Returns canonical JSON for the exact
    /// `sensor_id + sensor_hash` entry.
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

    /// Fetch and verify a peer's Clock Registry entry over
    /// `/auki/registries/0.0.1`. Returns canonical JSON for the exact
    /// `clock_id + clock_hash` entry.
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

    /// Fetch and verify a peer's Frame Registry entry over
    /// `/auki/registries/0.0.1`. Returns canonical JSON for the exact
    /// `frame_id + frame_hash` entry.
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

    /// Shutdown — cancels the Manager heartbeat tick, deregisters
    /// the cluster from Discovery (if we're the Manager), and shuts
    /// down the runtime. Idempotent; subsequent calls return an
    /// error indicating the manager has already been shut down.
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

    /// Internal helper shared by `open_camera_stream` /
    /// `open_pointcloud_stream` / `open_joint_encoders_stream`.
    /// Each typed wrapper supplies the matching `T` plus a closure
    /// that builds a `PyStreamSubscription` from the corresponding
    /// Rust `StreamSubscription<T>`.
    ///
    /// Holds the `ClusterManager` Mutex for the full open round-trip
    /// (up to `OPEN_STREAM_TIMEOUT` = 30s). Same pattern as
    /// `admit_peer`. Concurrent ClusterManager calls block during
    /// this window — acceptable because `open_stream` is per-tile-
    /// mount, not on a hot path.
    fn open_typed_stream<T>(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: &str,
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
        let request = RustStreamRequest {
            sensor_id: sensor_id.to_string(),
            ..Default::default()
        };
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
        sensor_id: &str,
    ) -> PyResult<GenericStreamPayloadKind> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let sensor_id = sensor_id.to_string();
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
                        RustResourcesRequest::sensor_streams(),
                    )
                    .await
                    .map_err(map_fetch_resources_catalog_error)?;
                resolve_generic_stream_payload_kind(&resp.resources, &sensor_id)
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
    // `Wallet::from_seed` now takes `Vec<u8>` and returns `Result<Arc<Wallet>, _>`
    // (UniFFI 0.31 constraint — no Lift impl for `[u8; 32]`). `seed` is a
    // `&[u8; 32]` here, so the length precondition is structural; the
    // expect is unreachable at runtime.
    let wallet = Wallet::from_seed(seed.to_vec()).expect("caller passes a 32-byte seed");
    let identity = PeerIdentity::from_wallet(wallet);
    let cfg = SwarmConfig {
        listen_addresses: listen_multiaddrs,
        agent_version,
        enable_relay_server: false,
    };
    let mut swarm = build_swarm(&identity, cfg)
        .map_err(|e| PyOSError::new_err(format!("build_swarm failed: {e}")))?;
    // Single SDK helper that subsumes both auto-detection and
    // operator-override resolution. If the caller passed
    // `external_addresses`, those go to Discovery verbatim and the swarm
    // event loop is skipped. Otherwise, drive the swarm for ~2 s to
    // collect every routable NewListenAddr libp2p emits (loopback /
    // link-local / unspecified filtered).
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

fn map_fetch_sensors_catalog_error(e: auki_domain_rs::FetchSensorsCatalogError) -> PyErr {
    match e {
        auki_domain_rs::FetchSensorsCatalogError::Request(err) => {
            PyOSError::new_err(format!("fetch_sensors_catalog: {err}"))
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

/// `auki_domain` Python module.
#[pymodule]
fn auki_domain(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyClusterMember>()?;
    m.add_class::<PyClusterMembership>()?;
    m.add_class::<PyDaemonInfo>()?;
    m.add_class::<PyParticipantInfo>()?;
    m.add_class::<PySensorEntry>()?;
    m.add_class::<PyResourcePinholeIntrinsics>()?;
    m.add_class::<PyResourceVec3>()?;
    m.add_class::<PyResourceQuat>()?;
    m.add_class::<PyResourceSpatialTransform>()?;
    m.add_class::<PySensorStreamResource>()?;
    m.add_class::<PyTransformEdgeResource>()?;
    m.add_class::<PyStreamManifestBuilder>()?;
    m.add_class::<PyClusterTarget>()?;
    m.add_class::<PyClusterManager>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_registry::{
        FrameRegistryEntry, PointCloud, PointField, PointFieldDataType, SensorBody,
        SensorRegistryEntry, write_frame, write_sensor,
    };
    use pyo3::types::{PyModule, PyString};

    const FRAME_ID: &str = "K1-AABBCCDDEEFF/head_left_cam_optical";

    fn install_auki_network_module(py: Python<'_>) -> PyResult<()> {
        let module = PyModule::new_bound(py, "auki_network")?;
        auki_network_py::populate_module(&module)?;
        py.import_bound("sys")?
            .getattr("modules")?
            .set_item("auki_network", &module)?;
        Ok(())
    }

    fn write_spatial_registry_fixture(app_root: &std::path::Path) -> (String, String, String) {
        let frame = FrameRegistryEntry::ros_optical(FRAME_ID);
        let frame_hash = write_frame(app_root, &frame).unwrap().hash().to_string();
        let entry = SensorRegistryEntry {
            sensor_id: "K1-AABBCCDDEEFF/head_depth_points".into(),
            body: SensorBody::PointCloud(PointCloud {
                fields: vec![PointField {
                    name: "x".into(),
                    offset: 0,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                }],
                point_step: 4,
                is_bigendian: false,
                frame_rate_hz: 10,
                frame_id: FRAME_ID.into(),
                frame_hash: frame_hash.clone(),
            }),
        };
        let sensor_hash = write_sensor(app_root, &entry).unwrap().hash().to_string();
        (entry.sensor_id, sensor_hash, frame_hash)
    }

    #[test]
    fn module_exposes_stream_manifest_builder() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_domain").unwrap();
            auki_domain(py, &module).unwrap();

            assert!(module.getattr("StreamManifestBuilder").is_ok());
            assert!(module.getattr("ResourcePinholeIntrinsics").is_ok());
            assert!(module.getattr("ResourceVec3").is_ok());
            assert!(module.getattr("ResourceQuat").is_ok());
            assert!(module.getattr("ResourceSpatialTransform").is_ok());
            assert!(module.getattr("SensorStreamResource").is_ok());
            assert!(module.getattr("TransformEdgeResource").is_ok());
        });
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
        });
    }

    #[test]
    fn generic_stream_resolver_uses_resource_payload_metadata() {
        let resources = vec![
            RustResourceEntry::SensorStream(RustSensorStreamResource {
                id: "camera".into(),
                sensor_id: "K1-AABBCCDDEEFF/head_left_cam".into(),
                sensor_hash: "camera-hash".into(),
                sensor_kind: "camera".into(),
                stream_protocol: "/auki/stream/0.1.0".into(),
                payload: "camera_frame".into(),
                pinhole_intrinsics: None,
                sensor_entry_json: None,
                frame_entry_json: None,
            }),
            RustResourceEntry::SensorStream(RustSensorStreamResource {
                id: "audio".into(),
                sensor_id: "K1-AABBCCDDEEFF/head_array_4mic".into(),
                sensor_hash: "audio-hash".into(),
                sensor_kind: "audio".into(),
                stream_protocol: "/auki/stream/0.1.0".into(),
                payload: "audio_frame".into(),
                pinhole_intrinsics: None,
                sensor_entry_json: None,
                frame_entry_json: None,
            }),
        ];

        assert_eq!(
            resolve_generic_stream_payload_kind(&resources, "K1-AABBCCDDEEFF/head_left_cam")
                .unwrap(),
            GenericStreamPayloadKind::Camera
        );
        assert_eq!(
            resolve_generic_stream_payload_kind(&resources, "K1-AABBCCDDEEFF/head_array_4mic")
                .unwrap(),
            GenericStreamPayloadKind::Audio
        );
    }

    #[test]
    fn generic_stream_resolver_rejects_unknown_payloads_in_sdk() {
        let resources = vec![
            RustResourceEntry::SensorStream(RustSensorStreamResource {
                id: "custom".into(),
                sensor_id: "robot/custom".into(),
                sensor_hash: "custom-hash".into(),
                sensor_kind: "custom_sensor".into(),
                stream_protocol: "/auki/stream/0.1.0".into(),
                payload: "custom_frame".into(),
                pinhole_intrinsics: None,
                sensor_entry_json: None,
                frame_entry_json: None,
            }),
            RustResourceEntry::SensorStream(RustSensorStreamResource {
                id: "camera-bad-payload".into(),
                sensor_id: "robot/camera".into(),
                sensor_hash: "camera-hash".into(),
                sensor_kind: "camera".into(),
                stream_protocol: "/auki/stream/0.1.0".into(),
                payload: "custom_camera_frame".into(),
                pinhole_intrinsics: None,
                sensor_entry_json: None,
                frame_entry_json: None,
            }),
        ];

        let err = resolve_generic_stream_payload_kind(&resources, "robot/custom").unwrap_err();
        Python::with_gil(|py| {
            assert!(err.is_instance_of::<PyValueError>(py));
        });

        let err = resolve_generic_stream_payload_kind(&resources, "robot/camera").unwrap_err();
        Python::with_gil(|py| {
            assert!(err.is_instance_of::<PyValueError>(py));
        });
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

            // Proves we constructed the real `auki_network.cluster.StreamManifest`
            // type, not a duplicate PyO3 class registered by auki_domain.
            let cluster = py
                .import_bound("auki_network")
                .unwrap()
                .getattr("cluster")
                .unwrap();
            let decision_cls = cluster.getattr("StreamDecision").unwrap();
            let kwargs = PyDict::new_bound(py);
            kwargs.set_item("manifest", manifest).unwrap();
            kwargs.set_item("source", py.None()).unwrap();
            let decision = decision_cls
                .getattr("accept_pointcloud")
                .unwrap()
                .call((), Some(&kwargs))
                .unwrap();
            assert_eq!(
                decision
                    .getattr("kind")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "accept_pointcloud"
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
