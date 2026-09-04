//! Python client and provider-backed endpoint for Blob v1.

use std::{sync::Arc, time::Duration};

use auki_protocols::blob::{
    BlobClient, BlobEndpoint, BlobFetchReceipt, BlobProvider, BlobProviderError,
    BlobProviderFuture, ProvidedBlobChunk,
    v1::{BlobRequest, ID},
};
use parking_lot::Mutex;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    pyclass::{PyTraverseError, PyVisit},
    types::{PyAny, PyBytes, PyDict, PyModule},
};
use pyo3_async_runtimes::TaskLocals;

use crate::{
    PyAukiPeer,
    cleanup::{CleanupResult, DetachedCleanup, wait_cleanup},
};

use super::support::{
    CancelablePythonAwaitable, PythonCallback, PythonTaskRegistry, enter_tokio_runtime,
    parse_peer_id, parse_target, report_provider_error, requester_to_python, require_callable,
    runtime_error,
};

const PYTHON_PROVIDER_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Outbound Blob v1 client backed by the portable Rust protocol.
#[pyclass(name = "AukiBlobClient", frozen)]
#[derive(Clone)]
pub(crate) struct PyAukiBlobClient {
    inner: BlobClient,
}

impl PyAukiBlobClient {
    fn from_inner(inner: BlobClient) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyAukiBlobClient {
    #[new]
    fn new(peer: &PyAukiPeer) -> Self {
        Self::from_inner(BlobClient::new(peer.protocols()))
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    /// Fetch and SHA-256-verify one complete blob using configured routes.
    fn fetch<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        sha256: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let remote_peer_id = parse_peer_id(&remote_peer_id)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let receipt = client
                .fetch(remote_peer_id, sha256)
                .await
                .map_err(|error| runtime_error("fetch blob", error))?;
            Python::with_gil(|py| blob_receipt_to_python(py, receipt))
        })
    }

    /// Fetch and SHA-256-verify one complete blob through an exact route.
    fn fetch_exact<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        route: String,
        sha256: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (remote_peer_id, route) = parse_target(&remote_peer_id, &route)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let receipt = client
                .fetch_exact(remote_peer_id, route, sha256)
                .await
                .map_err(|error| runtime_error("fetch blob", error))?;
            Python::with_gil(|py| blob_receipt_to_python(py, receipt))
        })
    }
}

fn blob_receipt_to_python(py: Python<'_>, receipt: BlobFetchReceipt) -> PyResult<PyObject> {
    let value = PyDict::new_bound(py);
    value.set_item("remote_peer_id", receipt.remote_peer_id.to_string())?;
    value.set_item("sha256", receipt.sha256)?;
    value.set_item("bytes", PyBytes::new_bound(py, &receipt.bytes))?;
    value.set_item("relayed", receipt.relayed)?;
    Ok(value.unbind().into_any())
}

#[derive(Clone)]
struct PythonBlobProvider {
    callback: PythonCallback,
    locals: Arc<TaskLocals>,
    tasks: PythonTaskRegistry,
}

enum PythonProviderValue {
    Ready(PyObject),
    Awaitable(CancelablePythonAwaitable),
}

impl BlobProvider for PythonBlobProvider {
    fn provide<'a>(
        &'a self,
        requester: &'a auki_sdk_rs::AuthenticatedPeer,
        request: &'a BlobRequest,
    ) -> BlobProviderFuture<'a> {
        let callback = self.callback.clone();
        let invocation =
            invoke_blob_provider(&callback, &self.locals, &self.tasks, requester, request);
        Box::pin(async move {
            let value = match invocation {
                Ok(PythonProviderValue::Ready(value)) => value,
                Ok(PythonProviderValue::Awaitable(awaitable)) => awaitable
                    .await
                    .map_err(|error| blob_callback_error(&callback, "callback rejected", error))?,
                Err(error) => {
                    return Err(blob_callback_error(
                        &callback,
                        "callback invocation failed",
                        error,
                    ));
                }
            };
            Python::with_gil(|py| provided_blob_from_python(value.bind(py)))
                .map_err(|error| blob_callback_error(&callback, "invalid callback result", error))
        })
    }
}

fn invoke_blob_provider(
    callback: &PythonCallback,
    locals: &TaskLocals,
    tasks: &PythonTaskRegistry,
    requester: &auki_sdk_rs::AuthenticatedPeer,
    request: &BlobRequest,
) -> PyResult<PythonProviderValue> {
    Python::with_gil(|py| {
        let requester = requester_to_python(py, requester)?;
        let request = blob_request_to_python(py, request)?;
        let context = locals.context(py).call_method0("copy")?;
        let value = context.call_method1("run", (callback.bind(py), requester, request))?;
        let awaitable: bool = py
            .import_bound("inspect")?
            .call_method1("isawaitable", (&value,))?
            .extract()?;
        if awaitable {
            Ok(PythonProviderValue::Awaitable(
                tasks.schedule(py, locals, value)?,
            ))
        } else {
            Ok(PythonProviderValue::Ready(value.unbind()))
        }
    })
}

fn blob_request_to_python(py: Python<'_>, request: &BlobRequest) -> PyResult<PyObject> {
    let value = PyDict::new_bound(py);
    value.set_item("sha256", &request.sha256)?;
    value.set_item("offset", request.offset)?;
    value.set_item("max_len", request.max_len)?;
    Ok(value.unbind().into_any())
}

fn provided_blob_from_python(value: &Bound<'_, PyAny>) -> PyResult<Option<ProvidedBlobChunk>> {
    if value.is_none() {
        return Ok(None);
    }
    let value = value.downcast::<PyDict>().map_err(|_| {
        PyTypeError::new_err("Blob provider must return a dict, None, or an awaitable of either")
    })?;
    let total_size = value
        .get_item("total_size")?
        .ok_or_else(|| PyTypeError::new_err("Blob provider result requires total_size"))?
        .extract::<u64>()?;
    let bytes = value
        .get_item("bytes")?
        .ok_or_else(|| PyTypeError::new_err("Blob provider result requires bytes"))?;
    let bytes = bytes
        .downcast::<PyBytes>()
        .map_err(|_| PyTypeError::new_err("Blob provider result bytes must be bytes"))?;
    Ok(Some(ProvidedBlobChunk::new(
        total_size,
        bytes.as_bytes().to_vec(),
    )))
}

fn blob_callback_error(
    callback: &PythonCallback,
    context: &'static str,
    error: PyErr,
) -> BlobProviderError {
    Python::with_gil(|py| report_provider_error(py, callback.bind(py), error));
    BlobProviderError::new(format!("{context}: Python Blob provider failed"))
}

struct BlobEndpointOwner {
    endpoint: Mutex<Option<BlobEndpoint>>,
    python_roots: Mutex<Vec<PythonCallback>>,
    tasks: PythonTaskRegistry,
    cleanup: DetachedCleanup,
}

impl BlobEndpointOwner {
    fn new(
        endpoint: BlobEndpoint,
        python_roots: Vec<PythonCallback>,
        tasks: PythonTaskRegistry,
    ) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            python_roots: Mutex::new(python_roots),
            tasks,
            cleanup: DetachedCleanup::default(),
        }
    }

    fn begin_close(&self) -> tokio::sync::watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            self.python_roots.lock().clear();
            let close = self.endpoint.lock().take().map(BlobEndpoint::close);
            let tasks = self.tasks.clone();
            async move {
                let endpoint_result = match close {
                    Some(close) => close.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                };
                let tasks_result = tasks.cancel_and_wait(PYTHON_PROVIDER_CLOSE_TIMEOUT).await;
                endpoint_result.and(tasks_result)
            }
        })
    }

    fn traverse(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        for root in self.python_roots.lock().iter() {
            visit.call(root.as_ref())?;
        }
        self.tasks.visit(visit)
    }
}

impl Drop for BlobEndpointOwner {
    fn drop(&mut self) {
        self.python_roots.get_mut().clear();
        let Some(endpoint) = self.endpoint.get_mut().take() else {
            return;
        };
        let tasks = self.tasks.clone();
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let _ = endpoint.close().await;
            let _ = tasks.cancel_and_wait(PYTHON_PROVIDER_CLOSE_TIMEOUT).await;
        });
    }
}

/// Mounted Blob provider plus a cloneable outbound client.
#[pyclass(name = "AukiBlobEndpoint")]
pub(crate) struct PyAukiBlobEndpoint {
    owner: BlobEndpointOwner,
    client: BlobClient,
}

#[pymethods]
impl PyAukiBlobEndpoint {
    /// Mount Blob v1 with a
    /// `provider(requester, request) -> value | awaitable` callback.
    ///
    /// Mounting must happen inside the running asyncio loop that will own the
    /// provider's awaitables. A result is `None` or a dict containing
    /// `total_size: int` and `bytes: bytes`.
    #[staticmethod]
    fn mount(py: Python<'_>, peer: &PyAukiPeer, provider: Py<PyAny>) -> PyResult<Self> {
        let callback = require_callable(py, provider, "Blob provider")?;
        let locals = pyo3_async_runtimes::tokio::get_current_locals(py)
            .map_err(|error| runtime_error("capture Blob provider asyncio loop", error))?;
        let python_roots = vec![
            callback.clone(),
            Arc::new(locals.event_loop(py).unbind()),
            Arc::new(locals.context(py).unbind()),
        ];
        let tasks = PythonTaskRegistry::default();
        let endpoint = enter_tokio_runtime(|| {
            BlobEndpoint::mount(
                peer.protocols(),
                PythonBlobProvider {
                    callback: callback.clone(),
                    locals: Arc::new(locals),
                    tasks: tasks.clone(),
                },
            )
        })
        .map_err(|error| runtime_error("mount Blob", error))?;
        let client = endpoint.client();
        Ok(Self {
            owner: BlobEndpointOwner::new(endpoint, python_roots, tasks),
            client,
        })
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    #[getter]
    fn client(&self) -> PyAukiBlobClient {
        PyAukiBlobClient::from_inner(self.client.clone())
    }

    /// Stop accepting Blob streams behind one detached, replayable barrier.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_close();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close Blob", error))
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
    module.add_class::<PyAukiBlobClient>()?;
    module.add_class::<PyAukiBlobEndpoint>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
        time::{Duration, Instant},
    };

    use auki_sdk_rs::Identity;
    use pyo3::exceptions::PyRuntimeError;

    use super::super::support::{requester, require_callable};
    use super::*;

    fn callback(py: Python<'_>, module: &Bound<'_, PyModule>, name: &str) -> PythonCallback {
        require_callable(py, module.getattr(name).unwrap().unbind(), "Blob provider").unwrap()
    }

    #[test]
    fn blob_dicts_preserve_integer_ranges_and_exact_bytes() {
        Python::with_gil(|py| {
            let request = blob_request_to_python(
                py,
                &BlobRequest {
                    sha256: "0".repeat(64),
                    offset: u64::MAX,
                    max_len: 1024,
                },
            )
            .unwrap();
            let request = request.bind(py).downcast::<PyDict>().unwrap();
            assert_eq!(
                request
                    .get_item("offset")
                    .unwrap()
                    .unwrap()
                    .extract::<u64>()
                    .unwrap(),
                u64::MAX
            );

            let value = PyDict::new_bound(py);
            value.set_item("total_size", 4_u64).unwrap();
            value
                .set_item("bytes", PyBytes::new_bound(py, b"data"))
                .unwrap();
            assert_eq!(
                provided_blob_from_python(value.as_any()).unwrap(),
                Some(ProvidedBlobChunk::new(4, b"data".to_vec()))
            );
            assert!(
                provided_blob_from_python(py.None().bind(py))
                    .unwrap()
                    .is_none()
            );
        });
    }

    #[test]
    fn provider_accepts_immediate_values_and_python_awaitables() {
        Python::with_gil(|py| {
            let event_loop = py
                .import_bound("asyncio")
                .unwrap()
                .call_method0("new_event_loop")
                .unwrap();
            let locals = Arc::new(
                TaskLocals::new(event_loop.clone())
                    .copy_context(py)
                    .unwrap(),
            );
            let module = PyModule::from_code_bound(
                py,
                r#"
import asyncio

def immediate(requester, request):
    assert requester["peer_type"] == "native_app"
    return {"total_size": 4, "bytes": b"sync"}

async def awaited(requester, request):
    await asyncio.sleep(0)
    return {"total_size": 5, "bytes": b"async"}
"#,
                "blob_provider_test.py",
                "blob_provider_test",
            )
            .unwrap();
            let immediate = PythonBlobProvider {
                callback: callback(py, &module, "immediate"),
                locals: Arc::clone(&locals),
                tasks: PythonTaskRegistry::default(),
            };
            let awaited = PythonBlobProvider {
                callback: callback(py, &module, "awaited"),
                locals,
                tasks: PythonTaskRegistry::default(),
            };

            pyo3_async_runtimes::tokio::run_until_complete(event_loop, async move {
                let requester = requester(Identity::generate().peer_id());
                let request = BlobRequest {
                    sha256: "0".repeat(64),
                    offset: 0,
                    max_len: 1024,
                };
                let immediate = immediate
                    .provide(&requester, &request)
                    .await
                    .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
                let awaited = awaited
                    .provide(&requester, &request)
                    .await
                    .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
                assert_eq!(immediate, Some(ProvidedBlobChunk::new(4, b"sync".to_vec())));
                assert_eq!(awaited, Some(ProvidedBlobChunk::new(5, b"async".to_vec())));
                Ok(())
            })
            .unwrap();
        });
    }

    #[test]
    fn concurrent_callbacks_enter_independent_context_copies() {
        pyo3::prepare_freethreaded_python();
        let (callback, locals) = Python::with_gil(|py| {
            let event_loop = py
                .import_bound("asyncio")
                .unwrap()
                .call_method0("new_event_loop")
                .unwrap();
            let locals = Arc::new(TaskLocals::new(event_loop).copy_context(py).unwrap());
            let callback = PyModule::from_code_bound(
                py,
                r#"
import time

def provider(requester, request):
    time.sleep(0.05)
    return {"total_size": 4, "bytes": b"data"}
"#,
                "concurrent_blob_provider_test.py",
                "concurrent_blob_provider_test",
            )
            .unwrap()
            .getattr("provider")
            .unwrap()
            .unbind();
            (
                require_callable(py, callback, "Blob provider").unwrap(),
                locals,
            )
        });
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let callback = Arc::clone(&callback);
                let locals = Arc::clone(&locals);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let requester = requester(Identity::generate().peer_id());
                    let request = BlobRequest {
                        sha256: "0".repeat(64),
                        offset: 0,
                        max_len: 4,
                    };
                    barrier.wait();
                    assert!(matches!(
                        invoke_blob_provider(
                            &callback,
                            &locals,
                            &PythonTaskRegistry::default(),
                            &requester,
                            &request,
                        )
                        .unwrap(),
                        PythonProviderValue::Ready(_)
                    ));
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn task_registry_waits_for_cancelled_provider_finally() {
        pyo3::prepare_freethreaded_python();
        let (event_loop, provider, tasks, started, finished) = Python::with_gil(|py| {
            let event_loop = py
                .import_bound("asyncio")
                .unwrap()
                .call_method0("new_event_loop")
                .unwrap();
            let locals = Arc::new(
                TaskLocals::new(event_loop.clone())
                    .copy_context(py)
                    .unwrap(),
            );
            let module = PyModule::from_code_bound(
                py,
                r#"
import asyncio
import threading

started = threading.Event()
finished = threading.Event()

async def provider(requester, request):
    started.set()
    try:
        await asyncio.Event().wait()
    finally:
        await asyncio.sleep(0.05)
        finished.set()
"#,
                "cancelled_blob_provider_test.py",
                "cancelled_blob_provider_test",
            )
            .unwrap();
            let callback = callback(py, &module, "provider");
            let tasks = PythonTaskRegistry::default();
            (
                event_loop.unbind(),
                PythonBlobProvider {
                    callback,
                    locals,
                    tasks: tasks.clone(),
                },
                tasks,
                module.getattr("started").unwrap().unbind(),
                module.getattr("finished").unwrap().unbind(),
            )
        });

        Python::with_gil(|py| {
            pyo3_async_runtimes::tokio::run_until_complete(
                event_loop.bind(py).clone(),
                async move {
                    let requester = requester(Identity::generate().peer_id());
                    let request = BlobRequest {
                        sha256: "0".repeat(64),
                        offset: 0,
                        max_len: 4,
                    };
                    let pending = provider.provide(&requester, &request);
                    wait_for_python_flag(&started).await;
                    drop(pending);
                    tasks
                        .cancel_and_wait(PYTHON_PROVIDER_CLOSE_TIMEOUT)
                        .await
                        .map_err(PyRuntimeError::new_err)?;
                    let finished = Python::with_gil(|py| {
                        finished.bind(py).call_method0("is_set")?.extract::<bool>()
                    })?;
                    assert!(finished, "cleanup barrier returned before Python finally");
                    Ok(())
                },
            )
        })
        .unwrap();
    }

    async fn wait_for_python_flag(flag: &Py<PyAny>) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let set = Python::with_gil(|py| {
                flag.bind(py)
                    .call_method0("is_set")
                    .unwrap()
                    .extract::<bool>()
                    .unwrap()
            });
            if set {
                return;
            }
            assert!(Instant::now() < deadline, "Python task did not finish");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[test]
    fn blob_receipt_uses_bytes_not_a_list_of_integers() {
        Python::with_gil(|py| {
            let peer_id = Identity::generate().peer_id();
            let value = blob_receipt_to_python(
                py,
                BlobFetchReceipt {
                    remote_peer_id: peer_id,
                    sha256: "0".repeat(64),
                    bytes: b"blob".to_vec(),
                    relayed: true,
                },
            )
            .unwrap();
            let value = value.bind(py).downcast::<PyDict>().unwrap();
            assert_eq!(
                value
                    .get_item("bytes")
                    .unwrap()
                    .unwrap()
                    .downcast::<PyBytes>()
                    .unwrap()
                    .as_bytes(),
                b"blob"
            );
        });
    }
}
