use std::path::PathBuf;

use auki_sdk_rs::{
    AukiPeer, AukiPeerBootstrap, AukiPeerExit, AukiPeerLifecycle, AukiPeerProtocols,
    AukiPeerRoutes, Credentials, DomainDescriptor, DomainSelection,
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
    fn start_peer<'py>(
        &self,
        py: Python<'py>,
        domain_id: String,
        identity_file: PathBuf,
    ) -> PyResult<Bound<'py, PyAny>> {
        let domain_id = Uuid::parse_str(&domain_id)
            .map_err(|_| PyValueError::new_err("Domain ID must be a UUID"))?;
        let bootstrap = self.bootstrap.clone();
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
        Self {
            peer_id,
            domain_id,
            listen_addresses,
            routes: context.routes(),
            lifecycle: peer.lifecycle(),
            protocols: context.protocols(),
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

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyAukiDomain>()?;
    module.add_class::<PyAukiPeerRoutes>()?;
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
    }

    #[test]
    fn module_exposes_only_the_small_peer_facade() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_sdk").unwrap();
            register(&module).unwrap();
            assert!(module.getattr("AukiSession").is_ok());
            assert!(module.getattr("AukiDomain").is_ok());
            assert!(module.getattr("AukiPeer").is_ok());
        });
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
