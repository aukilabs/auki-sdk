//! Python bindings for `auki-network`.
//!
//! ## Python module shape
//!
//! ```text
//! auki_network
//!   DiscoveryClient(base_url)               # HTTP client
//!   ClusterEntry / CreateClusterOutcome     # Discovery value types
//!   cluster.*                                # Stream surface (see below)
//! ```
//!
//! ### `auki_network.cluster` — the stream surface
//!
//! Producer-side types — pyclasses Boosterapp/Sentinel build inside a
//! `stream_provider` callback:
//!
//! - `StreamRequest(sensor_id=...)` — the inbound request the SDK
//!   delivers to the provider callable.
//! - `AcceptInfo(sensor_hash=..., clock_id=..., clock_hash=...)` —
//!   accept-time metadata the producer commits to.
//! - `JpegFrame(bytes)` / `PointCloudFrame(bytes)` /
//!   `JointEncodersFrame(angles_rad)` — payload `T` types.
//! - `ProducerFrame(timestamp_ns=..., payload=...)` — what the
//!   source async iterator yields.
//! - `DeclineReason.*` / `EndReason.*` — typed reason factories.
//! - `StreamDecision.accept(info, source)` /
//!   `accept_pointcloud(info, source)` /
//!   `accept_joint_encoders(info, source)` /
//!   `decline(reason)` — the value the provider callable returns.
//!
//! Consumer-side types — returned by `ClusterManager.open_*_stream`
//! (which lives in `auki-domain-py`; the pyclasses live here so all
//! stream types are in one place):
//!
//! - `StreamSubscription(info=..., frames=...)` — accept-time
//!   metadata + a single-use frame iterator.
//! - `FrameIterator` — sync blocking iterator; `__next__` raises
//!   `StreamEndOfStream(reason)` / `StreamConnectionLost` /
//!   `StreamProtocolError(detail)` as the terminator.
//! - `ConsumerFrame(timestamp_ns, seq, payload)` — what
//!   `FrameIterator.__next__` yields.
//! - Exception classes: `StreamEndOfStream`, `StreamConnectionLost`,
//!   `StreamProtocolError`, `StreamDeclined`, `StreamUnreachable`.

use auki_network_rs::discovery_client::{
    ClusterEntry as RustClusterEntry, CreateClusterOutcome as RustCreateClusterOutcome,
    DiscoveryClient as RustDiscoveryClient, DiscoveryError as RustDiscoveryError,
};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::str::FromStr;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

mod stream_bridge;
pub mod stream_types;

// ─── Process-wide tokio runtime ────────────────────────────────────

/// Multi-threaded tokio runtime shared across all Discovery calls and
/// the consumer-side `FrameIterator` blocking-bridge. Reqwest needs a
/// runtime; we own one lazily on first use so the caller doesn't need
/// an `async` Python.
///
/// Name matches the convention from `auki-network-py` v0.0.33 — other
/// in-crate modules (`stream_types`) reference it as
/// `crate::cluster_tokio_runtime()`.
pub(crate) fn cluster_tokio_runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("tokio runtime starts"))
}

// ─── Error mapping ─────────────────────────────────────────────────

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

// ─── ClusterEntry pyclass ──────────────────────────────────────────

/// One cluster's entry in Discovery's directory.
#[pyclass(name = "ClusterEntry")]
#[derive(Clone)]
pub struct PyClusterEntry {
    inner: RustClusterEntry,
}

impl PyClusterEntry {
    fn from_rust(inner: RustClusterEntry) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyClusterEntry {
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[getter]
    fn manager_peer_id(&self) -> String {
        self.inner.manager_peer_id.to_string()
    }

    #[getter]
    fn manager_multiaddrs(&self) -> Vec<String> {
        self.inner
            .manager_multiaddrs
            .iter()
            .map(|m| m.to_string())
            .collect()
    }

    #[getter]
    fn peer_count(&self) -> u32 {
        self.inner.peer_count
    }

    #[getter]
    fn created_ns(&self) -> i64 {
        self.inner.created_ns
    }

    #[getter]
    fn last_heartbeat_ns(&self) -> i64 {
        self.inner.last_heartbeat_ns
    }

    fn __repr__(&self) -> String {
        format!(
            "ClusterEntry(name={:?}, manager_peer_id={:?}, peer_count={}, created_ns={}, last_heartbeat_ns={})",
            self.inner.name,
            self.inner.manager_peer_id.to_string(),
            self.inner.peer_count,
            self.inner.created_ns,
            self.inner.last_heartbeat_ns,
        )
    }
}

// ─── CreateClusterOutcome pyclass ──────────────────────────────────

/// Outcome of `DiscoveryClient.create_cluster`. Inspect via
/// `.is_already_exists` (bool) and `.entry` (the `ClusterEntry` when
/// the caller won the race; `None` otherwise).
#[pyclass(name = "CreateClusterOutcome")]
pub struct PyCreateClusterOutcome {
    /// True if the cluster name was already taken (409). The caller
    /// should list and join the existing cluster.
    #[pyo3(get)]
    is_already_exists: bool,
    /// The new cluster entry; populated only when the caller won the
    /// race (`is_already_exists = False`).
    #[pyo3(get)]
    entry: Option<PyClusterEntry>,
}

#[pymethods]
impl PyCreateClusterOutcome {
    fn __repr__(&self) -> String {
        if self.is_already_exists {
            "CreateClusterOutcome(AlreadyExists)".to_string()
        } else {
            format!(
                "CreateClusterOutcome(Created({}))",
                self.entry
                    .as_ref()
                    .map(|e| e.inner.name.clone())
                    .unwrap_or_default()
            )
        }
    }
}

// ─── DiscoveryClient pyclass ───────────────────────────────────────

/// HTTP client for a Discovery service instance.
#[pyclass(name = "DiscoveryClient")]
pub struct PyDiscoveryClient {
    inner: RustDiscoveryClient,
}

#[pymethods]
impl PyDiscoveryClient {
    /// Construct against `base_url`, e.g. `"http://192.168.9.130:8080"`.
    /// Trailing `/` is stripped.
    #[new]
    fn new(base_url: String) -> Self {
        Self {
            inner: RustDiscoveryClient::new(base_url),
        }
    }

    #[getter]
    fn base_url(&self) -> &str {
        self.inner.base_url()
    }

    /// Snapshot of Discovery's directory, sorted by `created_ns` desc.
    fn list_clusters(&self, py: Python<'_>) -> PyResult<Vec<PyClusterEntry>> {
        let client = self.inner.clone();
        py.allow_threads(|| {
            cluster_tokio_runtime()
                .block_on(client.list_clusters())
                .map(|entries| entries.into_iter().map(PyClusterEntry::from_rust).collect())
                .map_err(map_discovery_error)
        })
    }

    /// Atomically create a cluster. The caller becomes its initial
    /// Manager. On 409 returns `CreateClusterOutcome` with
    /// `is_already_exists = True`.
    fn create_cluster(
        &self,
        py: Python<'_>,
        name: &str,
        manager_peer_id: &str,
        manager_multiaddrs: Vec<String>,
    ) -> PyResult<PyCreateClusterOutcome> {
        let peer_id = parse_peer_id(manager_peer_id)?;
        let multiaddrs = parse_multiaddrs(&manager_multiaddrs)?;
        let client = self.inner.clone();
        let name = name.to_string();
        py.allow_threads(|| {
            let outcome = cluster_tokio_runtime()
                .block_on(client.create_cluster(&name, &peer_id, &multiaddrs))
                .map_err(map_discovery_error)?;
            Ok(match outcome {
                RustCreateClusterOutcome::Created(e) => PyCreateClusterOutcome {
                    is_already_exists: false,
                    entry: Some(PyClusterEntry::from_rust(e)),
                },
                RustCreateClusterOutcome::AlreadyExists => PyCreateClusterOutcome {
                    is_already_exists: true,
                    entry: None,
                },
            })
        })
    }

    /// Manager push: report aggregate `peer_count`.
    fn heartbeat(&self, py: Python<'_>, name: &str, peer_count: u32) -> PyResult<PyClusterEntry> {
        let client = self.inner.clone();
        let name = name.to_string();
        py.allow_threads(|| {
            cluster_tokio_runtime()
                .block_on(client.heartbeat(&name, peer_count))
                .map(PyClusterEntry::from_rust)
                .map_err(map_discovery_error)
        })
    }

    /// Rotate the Manager hint. Called by a newly-elected Manager
    /// after a successor election.
    fn rotate_manager(
        &self,
        py: Python<'_>,
        name: &str,
        manager_peer_id: &str,
        manager_multiaddrs: Vec<String>,
    ) -> PyResult<PyClusterEntry> {
        let peer_id = parse_peer_id(manager_peer_id)?;
        let multiaddrs = parse_multiaddrs(&manager_multiaddrs)?;
        let client = self.inner.clone();
        let name = name.to_string();
        py.allow_threads(|| {
            cluster_tokio_runtime()
                .block_on(client.rotate_manager(&name, &peer_id, &multiaddrs))
                .map(PyClusterEntry::from_rust)
                .map_err(map_discovery_error)
        })
    }

    /// Graceful deregistration.
    fn deregister(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        let client = self.inner.clone();
        let name = name.to_string();
        py.allow_threads(|| {
            cluster_tokio_runtime()
                .block_on(client.deregister(&name))
                .map_err(map_discovery_error)
        })
    }
}

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

// ─── Module population ─────────────────────────────────────────────

/// Populate `m` with the full `auki_network` Python surface:
///
/// - Root-level: `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`.
/// - `cluster` submodule: every stream type (`StreamRequest`,
///   `AcceptInfo`, `JpegFrame`, `PointCloudFrame`, `JointEncodersFrame`,
///   `ProducerFrame`, `ConsumerFrame`, `DeclineReason`, `EndReason`,
///   `StreamDecision`, `StreamSubscription`, `FrameIterator`, plus the
///   five `Stream*` exception classes).
///
/// Exposed as `pub` so cross-crate consumers in the workspace
/// (notably `auki-domain-py`'s test helpers) can build a populated
/// module for fixture purposes.
pub fn populate_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyClusterEntry>()?;
    m.add_class::<PyCreateClusterOutcome>()?;
    m.add_class::<PyDiscoveryClient>()?;

    let py = m.py();
    let cluster = PyModule::new_bound(py, "cluster")?;
    stream_types::register(py, &cluster)?;
    m.add_submodule(&cluster)?;
    Ok(())
}

/// `auki_network` Python module.
#[pymodule]
fn auki_network(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    populate_module(m)
}
