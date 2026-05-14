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
//!   constructor (`create_cluster`) and methods, each `block_on`s
//!   on a process-wide multi-thread tokio runtime.

use auki_domain_rs::{
    AdmitError as RustAdmitError, ClusterManager as RustClusterManager,
    ClusterMember as RustClusterMember, ClusterMembership as RustClusterMembership,
    CreateClusterError as RustCreateClusterError, DaemonInfo as RustDaemonInfo,
    JoinClusterError as RustJoinClusterError, SensorCatalogProvider as RustSensorCatalogProvider,
    SensorEntry as RustSensorEntry,
};
use auki_identity::Wallet;
use auki_network::ParticipantInfo as RustParticipantInfo;
use auki_network::PeerIdentity;
use auki_network::discovery_client::DiscoveryClient;
use auki_network::stream_protocol::{
    JointEncodersFrame as RustJointEncodersFrame, JpegFrame as RustJpegFrame,
    PointCloudFrame as RustPointCloudFrame, StreamRequest as RustStreamRequest,
};
use auki_network::stream_runtime::{StreamProvider, decline_all_streams};
use auki_network::swarm::{SwarmConfig, build_swarm};
use auki_network_py::stream_types::{
    PyStreamSubscription, STREAM_PROVIDER_CAPSULE_NAME, open_stream_error_to_pyerr,
};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyCapsule;
use std::ffi::CString;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::runtime::Runtime;

// ─── Stream provider bridge ────────────────────────────────────────────
//
// `auki-network-py` and `auki-domain-py` are separate Python extension
// modules. Although both link the `auki-network-py` rlib, each `.so`
// gets its own copy of every `#[pyclass]`'s PyType registration with a
// distinct type-id. A `PyStreamDecision` created by user code (via
// `auki_network.cluster.StreamDecision.accept(...)` → registered in
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
    let expected_name = CString::new(STREAM_PROVIDER_CAPSULE_NAME)
        .expect("static literal contains no nul");
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
        self.inner.multiaddrs.iter().map(|m| m.to_string()).collect()
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
    #[pyo3(signature = (sensor_id, sensor_hash, kind))]
    fn new(sensor_id: String, sensor_hash: String, kind: String) -> Self {
        Self {
            inner: RustSensorEntry {
                sensor_id,
                sensor_hash,
                kind,
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

    fn __repr__(&self) -> String {
        format!(
            "SensorEntry(sensor_id={:?}, sensor_hash={:?}, kind={:?})",
            self.inner.sensor_id, self.inner.sensor_hash, self.inner.kind,
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
                    eprintln!(
                        "auki-domain-py: sensor_catalog_provider callable failed: {e}"
                    );
                    Vec::new()
                }
            }
        })
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
    ///   or to advertise a relay-mediated multiaddr in v1 (until the
    ///   SDK ships a v2 relay-reservation helper). Replace-semantics:
    ///   the SDK does NOT mix these with auto-detected addresses.
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

                let discovery = DiscoveryClient::new(discovery_url);
                let manager = RustClusterManager::create_cluster(
                    cluster_name,
                    identity,
                    advertise_multiaddrs,
                    discovery,
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

                let discovery = DiscoveryClient::new(discovery_url);
                let manager = RustClusterManager::join_cluster(
                    cluster_name,
                    identity,
                    advertise_multiaddrs,
                    discovery,
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

    /// Open a JPEG stream subscription on `peer_id` for `sensor_id`.
    /// Returns a `StreamSubscription` whose `.frames()` iterator
    /// yields `ConsumerFrame(payload=JpegFrame(bytes=...))` values.
    /// Raises `auki_network.cluster.StreamDeclined` /
    /// `StreamUnreachable` / `StreamProtocolError` on failure.
    fn open_jpeg_stream(
        &self,
        py: Python<'_>,
        peer_id: &str,
        sensor_id: &str,
    ) -> PyResult<PyStreamSubscription> {
        self.open_typed_stream::<RustJpegFrame>(py, peer_id, sensor_id, |sub| {
            PyStreamSubscription::from_rust_jpeg(sub)
        })
    }

    /// Open a PointCloud stream subscription on `peer_id` for
    /// `sensor_id`. Returns a `StreamSubscription` whose `.frames()`
    /// iterator yields `ConsumerFrame(payload=PointCloudFrame(...))`.
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
    /// `sensor_id`. Returns a `StreamSubscription` whose `.frames()`
    /// iterator yields
    /// `ConsumerFrame(payload=JointEncodersFrame(angles_rad=...))`.
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
    fn fetch_participant_info(
        &self,
        py: Python<'_>,
        peer_id: &str,
    ) -> PyResult<PyParticipantInfo> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("ClusterManager has been shut down")
                })?;
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

    /// Fetch a cluster peer's current sensor catalog over the
    /// `/auki/sensors/0.0.1` libp2p protocol. `peer_id` must be a
    /// current cluster member (otherwise the runtime's allow-list
    /// refuses the substream). Returns a Python
    /// `list[SensorEntry]` — empty list if the target peer has not
    /// registered a catalog provider (NOT an error).
    fn fetch_sensors_catalog(
        &self,
        py: Python<'_>,
        peer_id: &str,
    ) -> PyResult<Vec<PySensorEntry>> {
        let peer_id_parsed = parse_peer_id(peer_id)?;
        let inner = self.inner.clone();
        py.allow_threads(|| {
            shared_runtime().block_on(async move {
                let guard = inner.lock().expect("ClusterManager lock");
                let manager = guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("ClusterManager has been shut down")
                })?;
                let resp = manager
                    .fetch_sensors_catalog(peer_id_parsed)
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

    /// Internal helper shared by `open_jpeg_stream` /
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
}

// ─── Helpers ───────────────────────────────────────────────────────

fn parse_peer_id(s: &str) -> PyResult<PeerId> {
    PeerId::from_str(s)
        .map_err(|e| PyValueError::new_err(format!("invalid peer_id {s:?}: {e}")))
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
    let wallet = Wallet::from_seed(seed);
    let identity = PeerIdentity::from_wallet(&wallet);
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
        RustJoinClusterError::NotFound(name) => PyRuntimeError::new_err(format!(
            "cluster {name:?} not found in Discovery directory"
        )),
        RustJoinClusterError::SendJoin(err) => {
            PyOSError::new_err(format!("join request: {err}"))
        }
        RustJoinClusterError::Rejected(reason) => {
            PyRuntimeError::new_err(format!("Manager rejected join: {reason}"))
        }
        RustJoinClusterError::InvalidMembership(err) => PyValueError::new_err(format!(
            "invalid membership JSON from Manager: {err}"
        )),
        RustJoinClusterError::Runtime(err) => {
            PyRuntimeError::new_err(format!("runtime: {err}"))
        }
    }
}

fn map_create_cluster_error(e: RustCreateClusterError) -> PyErr {
    match e {
        RustCreateClusterError::Discovery(err) => {
            PyOSError::new_err(format!("Discovery: {err}"))
        }
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
        RustAdmitError::Stopped => {
            PyRuntimeError::new_err("ClusterManager has been shut down")
        }
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

// ─── Module entry point ────────────────────────────────────────────

/// `auki_domain` Python module.
#[pymodule]
fn auki_domain(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyClusterMember>()?;
    m.add_class::<PyClusterMembership>()?;
    m.add_class::<PyDaemonInfo>()?;
    m.add_class::<PyParticipantInfo>()?;
    m.add_class::<PySensorEntry>()?;
    m.add_class::<PyClusterManager>()?;
    Ok(())
}
