use std::path::PathBuf;

use auki_sdk_rs::{
    AukiDiscovery, AukiDiscoveryCandidate, AukiDiscoveryError, AukiDiscoverySource, AukiPeer,
    AukiPeerBootstrap, AukiPeerExit, AukiPeerLifecycle, AukiPeerProtocols, AukiPeerRoutes,
    Credentials, DdsTrackerMode, DomainDescriptor, DomainSelection,
};
use parking_lot::Mutex;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyList, PyModule},
};
use uuid::Uuid;

use crate::cleanup::{DetachedCleanup, wait_cleanup};

fn runtime_error(context: &'static str, error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(format!("{context}: {error}"))
}

fn parse_discovery_mode(mode: Option<String>) -> PyResult<Option<DdsTrackerMode>> {
    mode.map(|mode| match mode.as_str() {
        "discover_only" => Ok(DdsTrackerMode::DiscoverOnly),
        "discover_and_advertise" => Ok(DdsTrackerMode::DiscoverAndAdvertise),
        _ => Err(PyValueError::new_err(
            "discovery_mode must be 'discover_only' or 'discover_and_advertise'",
        )),
    })
    .transpose()
}

struct PeerOwner {
    peer: Mutex<Option<AukiPeer>>,
    cleanup: DetachedCleanup,
}

impl PeerOwner {
    fn new(peer: AukiPeer) -> Self {
        Self {
            peer: Mutex::new(Some(peer)),
            cleanup: DetachedCleanup::default(),
        }
    }

    fn begin_shutdown(
        &self,
    ) -> tokio::sync::watch::Receiver<Option<crate::cleanup::CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let shutdown = self.peer.lock().take().map(AukiPeer::shutdown);
            async move {
                match shutdown {
                    // The shutdown future was constructed synchronously above,
                    // so protocols are already fenced before Python can cancel.
                    Some(shutdown) => shutdown.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }
}

impl Drop for PeerOwner {
    fn drop(&mut self) {
        let Some(peer) = self.peer.get_mut().take() else {
            return;
        };
        let shutdown = peer.shutdown();
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let _ = shutdown.await;
        });
    }
}

#[pyclass(name = "AukiDomain", frozen)]
struct PyAukiDomain {
    id: String,
    name: Option<String>,
    description: Option<String>,
    organization_id: Option<String>,
}

impl From<DomainDescriptor> for PyAukiDomain {
    fn from(domain: DomainDescriptor) -> Self {
        Self {
            id: domain.id.to_string(),
            name: domain.name,
            description: domain.description,
            organization_id: domain.organization_id.map(|id| id.to_string()),
        }
    }
}

#[pymethods]
impl PyAukiDomain {
    #[getter]
    fn id(&self) -> String {
        self.id.clone()
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.name.clone()
    }

    #[getter]
    fn description(&self) -> Option<String> {
        self.description.clone()
    }

    #[getter]
    fn organization_id(&self) -> Option<String> {
        self.organization_id.clone()
    }

    fn __repr__(&self) -> String {
        let name = self
            .name
            .as_ref()
            .map(|name| format!("{name:?}"))
            .unwrap_or_else(|| "None".into());
        format!("AukiDomain(id={:?}, name={name})", self.id)
    }
}

/// One atomic snapshot of the peer's confirmed relay routes.
#[pyclass(name = "AukiPeerRoutes", frozen)]
struct PyAukiPeerRoutes {
    tcp: String,
    wss: String,
}

#[pymethods]
impl PyAukiPeerRoutes {
    #[getter]
    fn tcp(&self) -> String {
        self.tcp.clone()
    }

    #[getter]
    fn wss(&self) -> String {
        self.wss.clone()
    }

    fn __repr__(&self) -> String {
        format!("AukiPeerRoutes(tcp={:?}, wss={:?})", self.tcp, self.wss)
    }
}

/// One bounded, untrusted DDS dial candidate.
#[pyclass(name = "AukiDiscoveryCandidate", frozen)]
struct PyAukiDiscoveryCandidate {
    peer_id: String,
    routes: Vec<String>,
    served_protocols: Vec<String>,
    expires_at: String,
    source: String,
}

impl From<AukiDiscoveryCandidate> for PyAukiDiscoveryCandidate {
    fn from(candidate: AukiDiscoveryCandidate) -> Self {
        Self {
            peer_id: candidate.peer_id().to_string(),
            routes: candidate.routes().iter().map(ToString::to_string).collect(),
            served_protocols: candidate.served_protocols().to_vec(),
            expires_at: candidate.expires_at().to_rfc3339(),
            source: match candidate.source() {
                AukiDiscoverySource::DdsTracker => "dds_tracker".into(),
            },
        }
    }
}

#[pymethods]
impl PyAukiDiscoveryCandidate {
    #[getter]
    fn peer_id(&self) -> String {
        self.peer_id.clone()
    }

    #[getter]
    fn routes(&self) -> Vec<String> {
        self.routes.clone()
    }

    #[getter]
    fn served_protocols(&self) -> Vec<String> {
        self.served_protocols.clone()
    }

    #[getter]
    fn expires_at(&self) -> String {
        self.expires_at.clone()
    }

    #[getter]
    fn source(&self) -> String {
        self.source.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "AukiDiscoveryCandidate(peer_id={:?}, protocols={})",
            self.peer_id,
            self.served_protocols.len()
        )
    }
}

#[pyclass(name = "AukiSession")]
struct PyAukiSession {
    bootstrap: AukiPeerBootstrap,
}

#[pymethods]
impl PyAukiSession {
    /// Authenticate a User against the shared development environment.
    #[staticmethod]
    fn login_dev<'py>(
        py: Python<'py>,
        email: String,
        password: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let bootstrap = AukiPeerBootstrap::dev(Credentials::user_password(email, password))
                .await
                .map_err(|error| runtime_error("authenticate Auki User", error))?;
            Python::with_gil(|py| Py::new(py, Self { bootstrap }))
        })
    }

    /// Authenticate a trusted native App against the development environment.
    #[staticmethod]
    fn login_app_dev<'py>(
        py: Python<'py>,
        access_key: String,
        secret: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let bootstrap = AukiPeerBootstrap::dev(Credentials::app(access_key, secret))
                .await
                .map_err(|error| runtime_error("authenticate Auki App", error))?;
            Python::with_gil(|py| Py::new(py, Self { bootstrap }))
        })
    }

    /// List every Domain this principal may explicitly select.
    fn accessible_domains<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let bootstrap = self.bootstrap.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let domains = bootstrap
                .accessible_domains()
                .await
                .map_err(|error| runtime_error("list accessible Auki Domains", error))?;
            Python::with_gil(|py| {
                let values = PyList::empty_bound(py);
                for choice in domains {
                    values.append(Py::new(py, PyAukiDomain::from(choice.domain))?)?;
                }
                Ok(values.unbind().into_any())
            })
        })
    }

    /// Start one persistent, relay-backed peer in an explicitly selected Domain.
    #[pyo3(signature = (domain_id, identity_file, discovery_mode=None))]
    fn start_peer<'py>(
        &self,
        py: Python<'py>,
        domain_id: String,
        identity_file: PathBuf,
        discovery_mode: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let domain_id = Uuid::parse_str(&domain_id)
            .map_err(|_| PyValueError::new_err("Domain ID must be a UUID"))?;
        let discovery_mode = parse_discovery_mode(discovery_mode)?;
        let bootstrap = match discovery_mode {
            Some(mode) => self.bootstrap.clone().with_dds_tracker(mode),
            None => self.bootstrap.clone(),
        };
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let peer = bootstrap
                .start_persistent_peer(DomainSelection::new(domain_id), identity_file)
                .await
                .map_err(|error| runtime_error("start Auki peer", error))?;
            Python::with_gil(|py| Py::new(py, PyAukiPeer::new(peer)))
        })
    }
}

/// One persistent relay-backed native peer.
#[pyclass(name = "AukiPeer")]
pub struct PyAukiPeer {
    owner: PeerOwner,
    peer_id: String,
    domain_id: String,
    listen_addresses: Vec<String>,
    routes: AukiPeerRoutes,
    lifecycle: AukiPeerLifecycle,
    protocols: AukiPeerProtocols,
    discovery: Option<AukiDiscovery>,
}

impl PyAukiPeer {
    fn new(peer: AukiPeer) -> Self {
        let peer_id = peer.peer_id().to_string();
        let domain_id = peer.domain_id().to_string();
        let listen_addresses = peer
            .listen_addresses()
            .iter()
            .map(ToString::to_string)
            .collect();
        let context = peer.protocol_context();
        let discovery = peer.discovery_handle().ok();
        Self {
            peer_id,
            domain_id,
            listen_addresses,
            routes: context.routes(),
            lifecycle: peer.lifecycle(),
            protocols: context.protocols(),
            discovery,
            owner: PeerOwner::new(peer),
        }
    }

    /// Rust-only protocol handle for adapters compiled into this extension.
    pub fn protocols(&self) -> AukiPeerProtocols {
        self.protocols.clone()
    }

    #[cfg(all(
        test,
        feature = "info",
        feature = "blob",
        feature = "message",
        feature = "stream"
    ))]
    pub(crate) fn from_test_peer(peer: AukiPeer) -> Self {
        Self::new(peer)
    }
}

#[pymethods]
impl PyAukiPeer {
    #[getter]
    fn peer_id(&self) -> String {
        self.peer_id.clone()
    }

    #[getter]
    fn domain_id(&self) -> String {
        self.domain_id.clone()
    }

    #[getter]
    fn listen_addresses(&self) -> Vec<String> {
        self.listen_addresses.clone()
    }

    /// One atomic snapshot of the required TCP/WSS routes from one relay slot.
    #[getter]
    fn routes(&self, py: Python<'_>) -> PyResult<Py<PyAukiPeerRoutes>> {
        let route = self
            .routes
            .snapshot()
            .map_err(|error| runtime_error("read Auki peer routes", error))?
            .relay_routes
            .into_iter()
            .next()
            .ok_or_else(|| PyRuntimeError::new_err("Auki peer has no confirmed relay route"))?;
        Py::new(
            py,
            PyAukiPeerRoutes {
                tcp: route.routes.tcp().to_string(),
                wss: route.routes.wss().to_string(),
            },
        )
    }

    /// Fetch every fresh same-Domain DDS candidate.
    fn discover<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.discover_inner(py, None)
    }

    /// Fetch fresh candidates advertising one exact protocol ID.
    fn discover_protocol<'py>(
        &self,
        py: Python<'py>,
        protocol_id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.discover_inner(py, Some(protocol_id))
    }

    /// Resolve after requested shutdown or raise after unexpected terminal failure.
    fn wait_stopped<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let lifecycle = self.lifecycle.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match lifecycle.wait_stopped().await {
                AukiPeerExit::Stopped => Ok(()),
                AukiPeerExit::Failed(failure) => Err(runtime_error(
                    "Auki peer stopped unexpectedly",
                    format!("{failure:?}"),
                )),
            }
        })
    }

    /// Fence immediately, then await one detached, replayable ordered cleanup.
    fn shutdown<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_shutdown();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("shut down Auki peer", error))
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "AukiPeer(peer_id={:?}, domain_id={:?})",
            self.peer_id, self.domain_id
        )
    }
}

impl PyAukiPeer {
    fn discover_inner<'py>(
        &self,
        py: Python<'py>,
        protocol_id: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let discovery = self.discovery.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let discovery = discovery.ok_or_else(|| {
                runtime_error("discover Auki peers", AukiDiscoveryError::Disabled)
            })?;
            let candidates = match protocol_id {
                Some(protocol_id) => discovery.discover_protocol(protocol_id).await,
                None => discovery.discover().await,
            }
            .map_err(|error| runtime_error("discover Auki peers", error))?;
            Python::with_gil(|py| {
                let values = PyList::empty_bound(py);
                for candidate in candidates {
                    values.append(Py::new(py, PyAukiDiscoveryCandidate::from(candidate))?)?;
                }
                Ok(values.unbind().into_any())
            })
        })
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyAukiDomain>()?;
    module.add_class::<PyAukiPeerRoutes>()?;
    module.add_class::<PyAukiDiscoveryCandidate>()?;
    module.add_class::<PyAukiSession>()?;
    module.add_class::<PyAukiPeer>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}
    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn native_facade_types_fit_the_multithreaded_python_runtime() {
        assert_send_sync::<AukiPeerBootstrap>();
        assert_send::<AukiPeer>();
        assert_send_sync::<AukiPeerProtocols>();
        assert_send_sync::<AukiPeerRoutes>();
        assert_send_sync::<AukiDiscovery>();
    }

    #[test]
    fn module_exposes_only_the_small_peer_facade() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_sdk").unwrap();
            register(&module).unwrap();
            assert!(module.getattr("AukiSession").is_ok());
            assert!(module.getattr("AukiDomain").is_ok());
            assert!(module.getattr("AukiPeer").is_ok());
            assert!(module.getattr("AukiDiscoveryCandidate").is_ok());
        });
    }

    #[test]
    fn discovery_mode_is_explicit_and_bounded() {
        assert_eq!(
            parse_discovery_mode(Some("discover_only".into())).unwrap(),
            Some(DdsTrackerMode::DiscoverOnly)
        );
        assert_eq!(
            parse_discovery_mode(Some("discover_and_advertise".into())).unwrap(),
            Some(DdsTrackerMode::DiscoverAndAdvertise)
        );
        assert!(parse_discovery_mode(Some("automatic".into())).is_err());
        assert_eq!(parse_discovery_mode(None).unwrap(), None);
    }

    #[test]
    fn domain_value_repr_is_non_secret_and_stable() {
        let domain = PyAukiDomain {
            id: "00000000-0000-0000-0000-000000000001".into(),
            name: Some("Lab".into()),
            description: None,
            organization_id: None,
        };
        assert_eq!(
            domain.__repr__(),
            "AukiDomain(id=\"00000000-0000-0000-0000-000000000001\", name=\"Lab\")"
        );
    }
}
