//! Python client and provider-backed endpoint for participant Info v1.

use auki_protocols::info::{
    InfoClient, InfoEndpoint, InfoProvider,
    v1::{AuthenticatedParticipantInfo, ID},
};
use parking_lot::Mutex;
use pyo3::{
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

/// Outbound participant-information v1 client backed by the portable Rust protocol.
#[pyclass(name = "AukiInfoClient", frozen)]
#[derive(Clone)]
pub(crate) struct PyAukiInfoClient {
    inner: InfoClient,
}

impl PyAukiInfoClient {
    fn from_inner(inner: InfoClient) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyAukiInfoClient {
    #[new]
    fn new(peer: &PyAukiPeer) -> Self {
        Self::from_inner(InfoClient::new(peer.protocols()))
    }

    /// Immutable authenticated protocol identifier implemented by this client.
    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    /// Fetch participant metadata using routes configured on the peer.
    fn fetch<'py>(&self, py: Python<'py>, remote_peer_id: String) -> PyResult<Bound<'py, PyAny>> {
        let remote_peer_id = parse_peer_id(&remote_peer_id)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let info = client
                .fetch(remote_peer_id)
                .await
                .map_err(|error| runtime_error("fetch participant info", error))?;
            Python::with_gil(|py| to_python(py, &info))
        })
    }

    /// Fetch participant metadata through one exact advertised route.
    fn fetch_exact<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        route: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (remote_peer_id, route) = parse_target(&remote_peer_id, &route)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let info = client
                .fetch_exact(remote_peer_id, route)
                .await
                .map_err(|error| runtime_error("fetch participant info", error))?;
            Python::with_gil(|py| to_python(py, &info))
        })
    }
}

#[derive(Clone)]
struct PythonInfoProvider {
    callback: PythonCallback,
}

impl InfoProvider for PythonInfoProvider {
    fn participant_info(
        &self,
        requester: &auki_sdk_rs::AuthenticatedPeer,
    ) -> Option<AuthenticatedParticipantInfo> {
        Python::with_gil(|py| {
            let callback = self.callback.bind(py);
            let sampled = (|| {
                let requester = requester_to_python(py, requester)?;
                let value = callback.call1((requester,))?;
                if value.is_none() {
                    Ok(None)
                } else {
                    parse_python(py, &value, "participant info snapshot").map(Some)
                }
            })();
            match sampled {
                Ok(info) => info,
                Err(error) => {
                    report_provider_error(py, callback, error);
                    None
                }
            }
        })
    }
}

struct InfoEndpointOwner {
    endpoint: Mutex<Option<InfoEndpoint>>,
    callback: Mutex<Option<PythonCallback>>,
    cleanup: DetachedCleanup,
}

impl InfoEndpointOwner {
    fn new(endpoint: InfoEndpoint, callback: PythonCallback) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            callback: Mutex::new(Some(callback)),
            cleanup: DetachedCleanup::default(),
        }
    }

    fn begin_close(&self) -> tokio::sync::watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            self.callback.lock().take();
            let close = self.endpoint.lock().take().map(InfoEndpoint::close);
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

impl Drop for InfoEndpointOwner {
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

/// Mounted participant Info provider plus a cloneable outbound client.
#[pyclass(name = "AukiInfoEndpoint")]
pub(crate) struct PyAukiInfoEndpoint {
    owner: InfoEndpointOwner,
    client: InfoClient,
}

#[pymethods]
impl PyAukiInfoEndpoint {
    /// Mount Info v1 with a synchronous `provider(requester)` callback.
    ///
    /// The callback returns one canonical participant-info dict or `None` to
    /// decline the authenticated requester.
    #[staticmethod]
    fn mount(py: Python<'_>, peer: &PyAukiPeer, provider: Py<PyAny>) -> PyResult<Self> {
        let callback = require_callable(py, provider, "Info provider")?;
        let endpoint = enter_tokio_runtime(|| {
            InfoEndpoint::mount(
                peer.protocols(),
                PythonInfoProvider {
                    callback: callback.clone(),
                },
            )
        })
        .map_err(|error| runtime_error("mount participant Info", error))?;
        let client = endpoint.client();
        Ok(Self {
            owner: InfoEndpointOwner::new(endpoint, callback),
            client,
        })
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    #[getter]
    fn client(&self) -> PyAukiInfoClient {
        PyAukiInfoClient::from_inner(self.client.clone())
    }

    /// Stop accepting Info requests behind one detached, replayable barrier.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_close();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close participant Info", error))
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
    module.add_class::<PyAukiInfoClient>()?;
    module.add_class::<PyAukiInfoEndpoint>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use auki_sdk_rs::Identity;

    use super::super::support::requester;
    use super::*;

    #[test]
    fn synchronous_provider_receives_requester_and_parses_canonical_snapshot() {
        Python::with_gil(|py| {
            let requester_peer_id = Identity::generate().peer_id();
            let local_peer_id = Identity::generate().peer_id();
            let module = PyModule::from_code_bound(
                py,
                r#"
def provider(requester):
    return {
        "app": "python-test",
        "app_version": "1.0.0",
        "name": requester["peer_type"],
        "session_id": "session",
        "session_clock_id": "clock",
        "session_clock_hash": "hash",
        "session_now_ns": 7,
        "peer_id": local_peer_id,
        "app_instance": "test",
    }
"#,
                "info_provider_test.py",
                "info_provider_test",
            )
            .unwrap();
            module
                .setattr("local_peer_id", local_peer_id.to_string())
                .unwrap();
            let callback = require_callable(
                py,
                module.getattr("provider").unwrap().unbind(),
                "Info provider",
            )
            .unwrap();
            let provider = PythonInfoProvider { callback };

            let info = provider
                .participant_info(&requester(requester_peer_id))
                .unwrap();
            assert_eq!(info.peer_id, local_peer_id);
            assert_eq!(info.name, "native_app");
            assert_eq!(info.session_now_ns, 7);
        });
    }

    #[test]
    fn provider_none_declines_without_synthesizing_metadata() {
        Python::with_gil(|py| {
            let callback = PyModule::from_code_bound(
                py,
                "def provider(requester):\n    return None\n",
                "declining_info_provider_test.py",
                "declining_info_provider_test",
            )
            .unwrap()
            .getattr("provider")
            .unwrap()
            .unbind();
            let provider = PythonInfoProvider {
                callback: require_callable(py, callback, "Info provider").unwrap(),
            };
            assert!(
                provider
                    .participant_info(&requester(Identity::generate().peer_id()))
                    .is_none()
            );
        });
    }
}
