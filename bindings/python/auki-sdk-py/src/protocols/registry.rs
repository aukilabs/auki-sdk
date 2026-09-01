//! Python client and provider-backed endpoint for Registry v3.

use auki_protocols::registry::{
    RegistryClient, RegistryEndpoint, RegistryProvider,
    v3::{ID, RegistryKind, RegistryRequest, RegistryResponse},
};
use auki_sdk_rs::{Multiaddr, PeerId};
use parking_lot::Mutex;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    pyclass::{PyTraverseError, PyVisit},
    types::{PyAny, PyModule},
};

use crate::{
    PyAukiPeer,
    cleanup::{CleanupResult, DetachedCleanup, wait_cleanup},
};

use super::support::{
    PythonCallback, enter_tokio_runtime, parse_peer_id, parse_python, parse_target,
    report_provider_error, requester_to_python, require_callable, runtime_error, to_python,
};

const REGISTRY_KIND_NAMES: &str = "sensor, clock, frame, detector, map, or device_model";

fn parse_registry_kind(kind: &str) -> PyResult<RegistryKind> {
    match kind {
        "sensor" => Ok(RegistryKind::Sensor),
        "clock" => Ok(RegistryKind::Clock),
        "frame" => Ok(RegistryKind::Frame),
        "detector" => Ok(RegistryKind::Detector),
        "map" => Ok(RegistryKind::Map),
        "device_model" => Ok(RegistryKind::DeviceModel),
        _ => Err(PyValueError::new_err(format!(
            "Registry kind must be one of {REGISTRY_KIND_NAMES}; got {kind:?}"
        ))),
    }
}

/// Outbound Registry v3 client backed by the portable Rust protocol.
#[pyclass(name = "AukiRegistryClient", frozen)]
#[derive(Clone)]
pub(crate) struct PyAukiRegistryClient {
    inner: RegistryClient,
}

impl PyAukiRegistryClient {
    fn from_inner(inner: RegistryClient) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyAukiRegistryClient {
    #[new]
    fn new(peer: &PyAukiPeer) -> Self {
        Self::from_inner(RegistryClient::new(peer.protocols()))
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    /// List one Registry namespace using routes configured on the peer.
    fn list<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        kind: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let remote_peer_id = parse_peer_id(&remote_peer_id)?;
        let kind = parse_registry_kind(&kind)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let entries = client
                .list(remote_peer_id, kind)
                .await
                .map_err(|error| runtime_error("list Registry entries", error))?;
            Python::with_gil(|py| to_python(py, &entries))
        })
    }

    /// List one Registry namespace through an exact advertised route.
    fn list_exact<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        route: String,
        kind: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (remote_peer_id, route) = parse_target(&remote_peer_id, &route)?;
        let kind = parse_registry_kind(&kind)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let entries = client
                .list_exact(remote_peer_id, route, kind)
                .await
                .map_err(|error| runtime_error("list Registry entries", error))?;
            Python::with_gil(|py| to_python(py, &entries))
        })
    }

    /// Fetch one hash-validated typed Registry entry using configured routes.
    fn fetch<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        kind: String,
        id: String,
        hash: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let remote_peer_id = parse_peer_id(&remote_peer_id)?;
        let kind = parse_registry_kind(&kind)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            fetch_registry_entry(client, remote_peer_id, None, kind, id, hash).await
        })
    }

    /// Fetch one hash-validated typed Registry entry through an exact route.
    fn fetch_exact<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        route: String,
        kind: String,
        id: String,
        hash: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (remote_peer_id, route) = parse_target(&remote_peer_id, &route)?;
        let kind = parse_registry_kind(&kind)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            fetch_registry_entry(client, remote_peer_id, Some(route), kind, id, hash).await
        })
    }
}

async fn fetch_registry_entry(
    client: RegistryClient,
    remote_peer_id: PeerId,
    route: Option<Multiaddr>,
    kind: RegistryKind,
    id: String,
    hash: String,
) -> PyResult<PyObject> {
    match kind {
        RegistryKind::Sensor => {
            let entry = match route {
                Some(route) => {
                    client
                        .fetch_sensor_exact(remote_peer_id, route, id, hash)
                        .await
                }
                None => client.fetch_sensor(remote_peer_id, id, hash).await,
            }
            .map_err(|error| runtime_error("fetch Sensor Registry entry", error))?;
            Python::with_gil(|py| to_python(py, &entry))
        }
        RegistryKind::Clock => {
            let entry = match route {
                Some(route) => {
                    client
                        .fetch_clock_exact(remote_peer_id, route, id, hash)
                        .await
                }
                None => client.fetch_clock(remote_peer_id, id, hash).await,
            }
            .map_err(|error| runtime_error("fetch Clock Registry entry", error))?;
            Python::with_gil(|py| to_python(py, &entry))
        }
        RegistryKind::Frame => {
            let entry = match route {
                Some(route) => {
                    client
                        .fetch_frame_exact(remote_peer_id, route, id, hash)
                        .await
                }
                None => client.fetch_frame(remote_peer_id, id, hash).await,
            }
            .map_err(|error| runtime_error("fetch Frame Registry entry", error))?;
            Python::with_gil(|py| to_python(py, &entry))
        }
        RegistryKind::Detector => {
            let entry = match route {
                Some(route) => {
                    client
                        .fetch_detector_exact(remote_peer_id, route, id, hash)
                        .await
                }
                None => client.fetch_detector(remote_peer_id, id, hash).await,
            }
            .map_err(|error| runtime_error("fetch Detector Registry entry", error))?;
            Python::with_gil(|py| to_python(py, &entry))
        }
        RegistryKind::Map => {
            let entry = match route {
                Some(route) => {
                    client
                        .fetch_map_exact(remote_peer_id, route, id, hash)
                        .await
                }
                None => client.fetch_map(remote_peer_id, id, hash).await,
            }
            .map_err(|error| runtime_error("fetch Map Registry entry", error))?;
            Python::with_gil(|py| to_python(py, &entry))
        }
        RegistryKind::DeviceModel => {
            let entry = match route {
                Some(route) => {
                    client
                        .fetch_device_model_exact(remote_peer_id, route, id, hash)
                        .await
                }
                None => client.fetch_device_model(remote_peer_id, id, hash).await,
            }
            .map_err(|error| runtime_error("fetch Device Model Registry entry", error))?;
            Python::with_gil(|py| to_python(py, &entry))
        }
    }
}

#[derive(Clone)]
struct PythonRegistryProvider {
    callback: PythonCallback,
}

impl RegistryProvider for PythonRegistryProvider {
    fn respond(
        &self,
        requester: &auki_sdk_rs::AuthenticatedPeer,
        request: &RegistryRequest,
    ) -> RegistryResponse {
        Python::with_gil(|py| {
            let callback = self.callback.bind(py);
            let response = (|| {
                let requester = requester_to_python(py, requester)?;
                let request = to_python(py, request)?;
                let value = callback.call1((requester, request))?;
                parse_python(py, &value, "Registry provider response")
            })();
            response.unwrap_or_else(|error| {
                report_provider_error(py, callback, error);
                RegistryResponse::Error {
                    reason: "Registry provider callback failed".into(),
                }
            })
        })
    }
}

struct RegistryEndpointOwner {
    endpoint: Mutex<Option<RegistryEndpoint>>,
    callback: Mutex<Option<PythonCallback>>,
    cleanup: DetachedCleanup,
}

impl RegistryEndpointOwner {
    fn new(endpoint: RegistryEndpoint, callback: PythonCallback) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            callback: Mutex::new(Some(callback)),
            cleanup: DetachedCleanup::default(),
        }
    }

    fn begin_close(&self) -> tokio::sync::watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            self.callback.lock().take();
            let close = self.endpoint.lock().take().map(RegistryEndpoint::close);
            async move {
                match close {
                    Some(close) => close.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }

    fn traverse(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Some(callback) = self.callback.lock().as_ref() {
            visit.call(callback.as_ref())?;
        }
        Ok(())
    }
}

impl Drop for RegistryEndpointOwner {
    fn drop(&mut self) {
        self.callback.get_mut().take();
        let Some(endpoint) = self.endpoint.get_mut().take() else {
            return;
        };
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let _ = endpoint.close().await;
        });
    }
}

/// Mounted Registry provider plus a cloneable outbound client.
#[pyclass(name = "AukiRegistryEndpoint")]
pub(crate) struct PyAukiRegistryEndpoint {
    owner: RegistryEndpointOwner,
    client: RegistryClient,
}

#[pymethods]
impl PyAukiRegistryEndpoint {
    /// Mount Registry v3 with a synchronous
    /// `provider(requester, request) -> response` callback.
    #[staticmethod]
    fn mount(py: Python<'_>, peer: &PyAukiPeer, provider: Py<PyAny>) -> PyResult<Self> {
        let callback = require_callable(py, provider, "Registry provider")?;
        let endpoint = enter_tokio_runtime(|| {
            RegistryEndpoint::mount(
                peer.protocols(),
                PythonRegistryProvider {
                    callback: callback.clone(),
                },
            )
        })
        .map_err(|error| runtime_error("mount Registry", error))?;
        let client = endpoint.client();
        Ok(Self {
            owner: RegistryEndpointOwner::new(endpoint, callback),
            client,
        })
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    #[getter]
    fn client(&self) -> PyAukiRegistryClient {
        PyAukiRegistryClient::from_inner(self.client.clone())
    }

    /// Stop accepting Registry requests behind one detached, replayable barrier.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_close();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close Registry", error))
        })
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        self.owner.traverse(&visit)
    }

    fn __clear__(&mut self) {
        self.owner.begin_close();
    }
}

pub(super) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyAukiRegistryClient>()?;
    module.add_class::<PyAukiRegistryEndpoint>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use auki_sdk_rs::Identity;

    use super::super::support::{requester, require_callable};
    use super::*;

    #[test]
    fn registry_kinds_use_only_the_six_canonical_names() {
        assert_eq!(parse_registry_kind("sensor").unwrap(), RegistryKind::Sensor);
        assert_eq!(
            parse_registry_kind("device_model").unwrap(),
            RegistryKind::DeviceModel
        );
        for invalid in ["Sensor", "deviceModel", "device-model", "unknown"] {
            assert!(parse_registry_kind(invalid).is_err());
        }
    }

    #[test]
    fn synchronous_provider_receives_and_returns_canonical_records() {
        Python::with_gil(|py| {
            let module = PyModule::from_code_bound(
                py,
                r#"
def provider(requester, request):
    assert requester["peer_type"] == "native_app"
    if request["op"] == "list":
        return {"op": "list", "entries": [{"id": "camera", "hash": "a" * 32}]}
    return {"op": "get", "entry": None}
"#,
                "registry_provider_test.py",
                "registry_provider_test",
            )
            .unwrap();
            let provider = PythonRegistryProvider {
                callback: require_callable(
                    py,
                    module.getattr("provider").unwrap().unbind(),
                    "Registry provider",
                )
                .unwrap(),
            };
            let requester = requester(Identity::generate().peer_id());

            let listed = provider.respond(&requester, &RegistryRequest::list(RegistryKind::Sensor));
            assert_eq!(
                listed,
                RegistryResponse::List {
                    entries: vec![auki_protocols::registry::v3::RegistryListEntry {
                        id: "camera".into(),
                        hash: "a".repeat(32),
                    }],
                }
            );
            assert_eq!(
                provider.respond(
                    &requester,
                    &RegistryRequest::get(RegistryKind::Sensor, "camera", "a".repeat(32)),
                ),
                RegistryResponse::Get { entry: None }
            );
        });
    }
}
