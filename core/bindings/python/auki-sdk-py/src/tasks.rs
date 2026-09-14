//! Python handlers run on the caller's event loop; Rust owns the lease lifecycle.
use std::{collections::BTreeMap, future::Future, sync::Arc, time::Duration};

use async_trait::async_trait;
use auki_sdk_rs::{
    AukiComputeCredential, AukiDmsTasks, ComputeConfig, SecretString, TaskContext, TaskError,
    TaskHandler, TaskOutcome, TaskResult, TasksConfig,
};
use parking_lot::Mutex;
use pyo3::{
    exceptions::{PyRuntimeError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyModule},
};
use pyo3_async_runtimes::TaskLocals;
use tokio_util::sync::CancellationToken;

use crate::{data::PyData, python_task::CancelablePythonAwaitable};

#[allow(unexpected_cfgs)]
mod exception {
    pyo3::create_exception!(auki_sdk, TaskRuntimeError, pyo3::exceptions::PyRuntimeError);
}

fn error(error: TaskError) -> PyErr {
    Python::with_gil(|py| {
        let kind = match error {
            TaskError::Configuration(_) => "configuration",
            TaskError::Closed => "closed",
            TaskError::Busy => "busy",
            TaskError::Cancelled => "cancelled",
            TaskError::LeaseLost => "lease_lost",
            TaskError::Authority(_) => "authority",
            TaskError::Authentication => "authentication",
            TaskError::Service(_) => "service",
            TaskError::HttpStatus { .. } => "http",
            TaskError::Handler => "handler",
            TaskError::Data => "data",
        };
        let result = exception::TaskRuntimeError::new_err(error.to_string());
        let _ = result.value_bound(py).setattr("kind", kind);
        let status = match error {
            TaskError::HttpStatus { status, .. } => Some(status),
            _ => None,
        };
        let _ = result.value_bound(py).setattr("status", status);
        result
    })
}

fn json(py: Python<'_>, value: &impl serde::Serialize) -> PyResult<PyObject> {
    let value =
        serde_json::to_string(value).map_err(|_| PyValueError::new_err("invalid task metadata"))?;
    Ok(PyModule::import_bound(py, "json")?
        .call_method1("loads", (value,))?
        .unbind())
}

fn parse<T: serde::de::DeserializeOwned>(value: &Bound<'_, PyAny>) -> PyResult<T> {
    let encoded: String = PyModule::import_bound(value.py(), "json")?
        .call_method1("dumps", (value,))?
        .extract()?;
    if encoded.len() > 64 * 1024 {
        return Err(PyValueError::new_err("task metadata exceeds 64 KiB"));
    }
    serde_json::from_str(&encoded)
        .map_err(|_| PyValueError::new_err("invalid task metadata/result"))
}

fn owned<'py, F, Fut>(py: Python<'py>, operation: F) -> PyResult<Bound<'py, PyAny>>
where
    F: FnOnce(CancellationToken) -> Fut,
    Fut: Future<Output = PyResult<PyObject>> + Send + 'static,
{
    let cancellation = CancellationToken::new();
    let guard = cancellation.clone().drop_guard();
    let task = pyo3_async_runtimes::tokio::get_runtime().spawn(operation(cancellation));
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let _guard = guard;
        task.await
            .map_err(|_| PyRuntimeError::new_err("task runtime stopped unexpectedly"))?
    })
}

#[pyclass(name = "AukiComputeCredential")]
struct PyCompute {
    inner: AukiComputeCredential,
}

#[pymethods]
impl PyCompute {
    #[new]
    #[pyo3(signature = (*, dds_url, dms_url, registration, wallet_key, version, client_id, request_timeout=30.0, registration_interval=120.0))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        dds_url: &str,
        dms_url: &str,
        registration: String,
        wallet_key: String,
        version: &str,
        client_id: &str,
        request_timeout: f64,
        registration_interval: f64,
    ) -> PyResult<Self> {
        let mut config = ComputeConfig::new(
            dds_url,
            dms_url,
            SecretString::new(registration),
            SecretString::new(wallet_key),
            version,
            client_id,
        )
        .map_err(error)?;
        config.request_timeout = duration(request_timeout)?;
        config.registration_interval = duration(registration_interval)?;
        Ok(Self {
            inner: AukiComputeCredential::new(config).map_err(error)?,
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        owned(py, |_| async move {
            inner.close().await;
            Ok(Python::with_gil(|py| py.None()))
        })
    }
}

fn duration(seconds: f64) -> PyResult<Duration> {
    Duration::try_from_secs_f64(seconds).map_err(|_| PyValueError::new_err("invalid duration"))
}

#[pyclass(name = "AukiTask")]
struct PyTask {
    inner: TaskContext,
}

#[pymethods]
impl PyTask {
    #[getter]
    fn id(&self) -> String {
        self.inner.task.id.to_string()
    }
    #[getter]
    fn domain_id(&self) -> String {
        self.inner.credential.domain_id().to_string()
    }
    #[getter]
    fn capability(&self) -> &str {
        &self.inner.task.capability
    }
    #[getter]
    fn meta(&self, py: Python<'_>) -> PyResult<PyObject> {
        json(py, &self.inner.task.meta)
    }
    #[getter]
    fn inputs_cids(&self) -> Vec<String> {
        self.inner.task.inputs_cids.clone()
    }
    #[getter]
    fn outputs_prefix(&self) -> Option<String> {
        self.inner.task.outputs_prefix.clone()
    }
    fn data(&self) -> PyData {
        PyData {
            inner: self.inner.data(),
        }
    }
    fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
    fn cancelled<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.cancelled().await;
            Ok(())
        })
    }
    fn progress<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let value = parse(value)?;
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.progress(value).map_err(error)
        })
    }
}

struct PythonHandler {
    callback: Arc<Py<PyAny>>,
    locals: TaskLocals,
    failure: Arc<Mutex<Option<PyErr>>>,
}

#[async_trait]
impl TaskHandler for PythonHandler {
    async fn run(&self, context: TaskContext) -> std::result::Result<TaskResult, TaskError> {
        let operation = async {
            let mut scheduled = Python::with_gil(|py| {
                static START: pyo3::sync::GILOnceCell<Py<PyAny>> = pyo3::sync::GILOnceCell::new();
                let start = START.get_or_try_init(py, || {
                    let module = PyModule::from_code_bound(
                        py,
                        "async def run(handler, task):\n    return await handler(task)\n",
                        "_auki_task_handler.py",
                        "_auki_task_handler",
                    )?;
                    Ok::<_, PyErr>(module.getattr("run")?.unbind())
                })?;
                let task = Py::new(
                    py,
                    PyTask {
                        inner: context.clone(),
                    },
                )?;
                let awaitable = start.bind(py).call1((self.callback.bind(py), task))?;
                CancelablePythonAwaitable::schedule(py, &self.locals, awaitable)
            })?;
            let result = tokio::select! {
                biased;
                _ = context.cancelled() => { scheduled.state().cancel(); scheduled.await },
                result = &mut scheduled => result,
            }?;
            Python::with_gil(|py| {
                if result.is_none(py) {
                    Ok(TaskResult::default())
                } else {
                    parse(result.bind(py))
                }
            })
        }
        .await;
        match operation {
            Ok(result) => Ok(result),
            Err(error) => {
                if context.is_cancelled() {
                    return Err(TaskError::Cancelled);
                }
                *self.failure.lock() = Some(error);
                Err(TaskError::Handler)
            }
        }
    }
}

#[pyclass(name = "AukiDmsTasks")]
struct PyTasks {
    inner: AukiDmsTasks,
    handlers: BTreeMap<String, Arc<Py<PyAny>>>,
}

impl Drop for PyTasks {
    fn drop(&mut self) {
        self.inner.request_shutdown();
    }
}

#[pymethods]
impl PyTasks {
    #[new]
    #[pyo3(signature = (credential, handlers, *, poll_interval=1.0, heartbeat_interval=30.0, request_timeout=30.0))]
    fn new(
        credential: &PyCompute,
        handlers: &Bound<'_, PyDict>,
        poll_interval: f64,
        heartbeat_interval: f64,
        request_timeout: f64,
    ) -> PyResult<Self> {
        let mut callbacks = BTreeMap::new();
        for (key, value) in handlers.iter() {
            if !value.is_callable() {
                return Err(PyTypeError::new_err(
                    "task handlers must be async callables",
                ));
            }
            callbacks.insert(key.extract::<String>()?, Arc::new(value.unbind()));
        }
        let config = TasksConfig {
            poll_interval: duration(poll_interval)?,
            heartbeat_interval: duration(heartbeat_interval)?,
            request_timeout: duration(request_timeout)?,
        };
        let inner = AukiDmsTasks::new(
            credential.inner.clone(),
            callbacks.keys().cloned().collect(),
            config,
        )
        .map_err(error)?;
        Ok(Self {
            inner,
            handlers: callbacks,
        })
    }

    fn __traverse__(
        &self,
        visit: pyo3::pyclass::PyVisit<'_>,
    ) -> std::result::Result<(), pyo3::pyclass::PyTraverseError> {
        for handler in self.handlers.values() {
            visit.call(handler.as_ref())?;
        }
        Ok(())
    }
    fn __clear__(&mut self) {
        self.inner.request_shutdown();
        self.handlers.clear();
    }

    fn run<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (handlers, failure) = self.prepare(py)?;
        let inner = self.inner.clone();
        owned(py, |cancellation| async move {
            inner
                .run(&handlers, &cancellation)
                .await
                .map_err(|e| handler_error(e, &failure))?;
            Ok(Python::with_gil(|py| py.None()))
        })
    }

    #[pyo3(signature = (capability=None))]
    fn run_once<'py>(
        &self,
        py: Python<'py>,
        capability: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let capability = match capability {
            Some(value) => value,
            None if self.handlers.len() == 1 => self.handlers.keys().next().unwrap().clone(),
            None => {
                return Err(PyValueError::new_err(
                    "select a capability when multiple handlers are registered",
                ));
            }
        };
        let (handlers, failure) = self.prepare(py)?;
        let handler = handlers
            .get(&capability)
            .cloned()
            .ok_or_else(|| PyValueError::new_err("unregistered capability"))?;
        let inner = self.inner.clone();
        owned(py, |cancellation| async move {
            let outcome = inner
                .run_once(&capability, handler.as_ref(), &cancellation)
                .await
                .map_err(|e| handler_error(e, &failure))?;
            Ok(Python::with_gil(|py| {
                match outcome {
                    TaskOutcome::NoWork => "no_work",
                    TaskOutcome::Busy => "busy",
                    TaskOutcome::Completed => "completed",
                }
                .into_py(py)
            }))
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.inner.request_shutdown();
        let inner = self.inner.clone();
        owned(py, |_| async move {
            inner.close().await;
            Ok(Python::with_gil(|py| py.None()))
        })
    }
}

type Handlers = BTreeMap<String, Arc<dyn TaskHandler>>;
type Failure = Arc<Mutex<Option<PyErr>>>;

impl PyTasks {
    fn prepare(&self, py: Python<'_>) -> PyResult<(Handlers, Failure)> {
        let locals = pyo3_async_runtimes::tokio::get_current_locals(py)?.copy_context(py)?;
        let failure = Arc::new(Mutex::new(None));
        let handlers = self
            .handlers
            .iter()
            .map(|(capability, callback)| {
                let handler: Arc<dyn TaskHandler> = Arc::new(PythonHandler {
                    callback: callback.clone(),
                    locals: locals.clone_ref(py),
                    failure: failure.clone(),
                });
                (capability.clone(), handler)
            })
            .collect();
        Ok((handlers, failure))
    }
}

fn handler_error(error_value: TaskError, failure: &Failure) -> PyErr {
    if matches!(error_value, TaskError::Handler)
        && let Some(error) = failure.lock().take()
    {
        return error;
    }
    error(error_value)
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyCompute>()?;
    module.add_class::<PyTasks>()?;
    module.add_class::<PyTask>()?;
    module.add(
        "TaskRuntimeError",
        module.py().get_type_bound::<exception::TaskRuntimeError>(),
    )?;
    Ok(())
}
