//! Python handlers run on the caller's event loop; Rust owns the lease lifecycle.
use std::{collections::BTreeMap, future::Future, sync::Arc, time::Duration};

use async_trait::async_trait;
use auki_sdk_rs::{
    AukiComputeCredential, AukiDmsTasks, AukiPeerConfig, AukiRobotCredential, AukiTaskPeerConfig,
    ComputeConfig, Identity, MachineCredential, RobotConfig, SecretString, TaskAccessToken,
    TaskContext, TaskError, TaskHandler, TaskOutcome, TaskPeerContext, TaskResult, TasksConfig,
};
use parking_lot::Mutex;
use pyo3::{
    exceptions::{PyRuntimeError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyModule},
};
use pyo3_async_runtimes::TaskLocals;
use tokio_util::sync::CancellationToken;

use crate::facade::{PyAukiPeer, PyAukiPeerConfig};
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
            TaskError::PeerCleanup => "cleanup",
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
    crate::async_completion::future_into_py(py, async move {
        let _guard = guard;
        task.await
            .map_err(|_| PyRuntimeError::new_err("task runtime stopped unexpectedly"))?
    })
}

#[pyclass(name = "AukiComputeCredential")]
struct PyCompute {
    inner: AukiComputeCredential,
    peer: Option<Arc<AukiTaskPeerConfig>>,
}

#[pymethods]
impl PyCompute {
    #[new]
    #[pyo3(signature = (*, dds_url, dms_url, registration, wallet_key, version, client_id, request_timeout=30.0, registration_interval=120.0, peer_identity_file=None, peer_config=None))]
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
        peer_identity_file: Option<std::path::PathBuf>,
        peer_config: Option<&PyAukiPeerConfig>,
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
        let peer = task_peer(dds_url, dms_url, peer_identity_file, peer_config)?;
        config.peer_identity = peer.as_ref().map(|p| p.identity_proof());
        Ok(Self {
            inner: AukiComputeCredential::new(config).map_err(error)?,
            peer,
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

fn task_peer(
    dds_url: &str,
    dms_url: &str,
    file: Option<std::path::PathBuf>,
    config: Option<&PyAukiPeerConfig>,
) -> PyResult<Option<Arc<AukiTaskPeerConfig>>> {
    let Some(file) = file else {
        if config.is_some() {
            return Err(PyValueError::new_err(
                "peer_config requires peer_identity_file",
            ));
        }
        return Ok(None);
    };
    let identity = Identity::load_or_create(file)
        .map_err(|_| PyValueError::new_err("cannot load persistent task peer identity"))?;
    let config = match config {
        Some(config) => config.inner.clone(),
        None => AukiPeerConfig::new(dms_url)
            .map_err(|_| PyValueError::new_err("invalid peer DMS URL"))?,
    };
    Ok(Some(Arc::new(
        AukiTaskPeerConfig::new(identity, dds_url, config).map_err(error)?,
    )))
}

#[pyclass(name = "AukiRobotCredential")]
struct PyRobot {
    inner: AukiRobotCredential,
    peer: Option<Arc<AukiTaskPeerConfig>>,
}
#[pymethods]
impl PyRobot {
    #[new]
    #[pyo3(signature = (*, dds_url, dms_url, registration, version, client_id, audience=None, capabilities, request_timeout=30.0, registration_interval=120.0, peer_identity_file=None, peer_config=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        dds_url: &str,
        dms_url: &str,
        registration: String,
        version: &str,
        client_id: &str,
        audience: Option<&str>,
        capabilities: Vec<String>,
        request_timeout: f64,
        registration_interval: f64,
        peer_identity_file: Option<std::path::PathBuf>,
        peer_config: Option<&PyAukiPeerConfig>,
    ) -> PyResult<Self> {
        let mut config = RobotConfig::new(
            dds_url,
            dms_url,
            SecretString::new(registration),
            version,
            client_id,
            audience,
            capabilities,
        )
        .map_err(error)?;
        config.request_timeout = duration(request_timeout)?;
        config.registration_interval = duration(registration_interval)?;
        let peer = task_peer(dds_url, dms_url, peer_identity_file, peer_config)?;
        config.peer_identity = peer.as_ref().map(|p| p.identity_proof());
        Ok(Self {
            inner: AukiRobotCredential::new(config).map_err(error)?,
            peer,
        })
    }
    fn assigned_domain_id<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        owned(py, |cancel| async move {
            let domain = inner
                .assigned_domain_id(&cancel)
                .await
                .map_err(error)?
                .map(|id| id.to_string());
            Ok(Python::with_gil(|py| domain.into_py(py)))
        })
    }
    fn data(&self, domain_id: &str) -> PyResult<PyData> {
        let domain = uuid::Uuid::parse_str(domain_id)
            .map_err(|_| PyValueError::new_err("Domain ID must be a UUID"))?;
        Ok(PyData {
            inner: self.inner.data(domain).map_err(error)?,
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

/// A retained handle reads the current task bearer and fails when its lease ends.
#[pyclass(name = "TaskAccessToken")]
struct PyTaskAccessToken {
    inner: TaskAccessToken,
}

#[pymethods]
impl PyTaskAccessToken {
    fn get(&self) -> PyResult<String> {
        Ok(self.inner.get().map_err(error)?.expose_secret().to_owned())
    }

    fn __repr__(&self) -> &'static str {
        "TaskAccessToken([REDACTED])"
    }
}

#[pymethods]
impl PyTask {
    #[getter]
    fn access_token(&self) -> PyTaskAccessToken {
        PyTaskAccessToken {
            inner: self.inner.access_token.clone(),
        }
    }
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
    fn peer(&self) -> Option<PyAukiPeer> {
        self.inner.peer().map(PyAukiPeer::from_task_peer)
    }
    fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
    fn cancelled<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::async_completion::future_into_py(py, async move {
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
        crate::async_completion::future_into_py(
            py,
            async move { inner.progress(value).map_err(error) },
        )
    }

    fn log_event<'py>(
        &self,
        py: Python<'py>,
        value: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let value = parse(value)?;
        let inner = self.inner.clone();
        crate::async_completion::future_into_py(
            py,
            async move { inner.log_event(value).map_err(error) },
        )
    }

    #[pyo3(signature = (reason, details=None))]
    fn set_failure<'py>(
        &self,
        py: Python<'py>,
        reason: String,
        details: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let details = details
            .map(parse)
            .transpose()?
            .unwrap_or(serde_json::Value::Null);
        let inner = self.inner.clone();
        crate::async_completion::future_into_py(py, async move {
            inner.set_failure(reason, details).map_err(error)
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
        credential: &Bound<'_, PyAny>,
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
        let (machine, peer) = if let Ok(credential) = credential.extract::<PyRef<'_, PyCompute>>() {
            (
                MachineCredential::from(credential.inner.clone()),
                credential.peer.clone(),
            )
        } else if let Ok(credential) = credential.extract::<PyRef<'_, PyRobot>>() {
            (
                MachineCredential::from(credential.inner.clone()),
                credential.peer.clone(),
            )
        } else {
            return Err(PyTypeError::new_err(
                "expected a compute or robot credential",
            ));
        };
        let capabilities = callbacks.keys().cloned().collect();
        let inner = match peer {
            Some(peer) => AukiDmsTasks::new_with_peer(machine, capabilities, config, peer),
            None => AukiDmsTasks::new(machine, capabilities, config),
        }
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

    /// Register and start optional robot networking without claiming a task.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        owned(py, |cancellation| async move {
            inner.start(&cancellation).await.map_err(error)?;
            Ok(Python::with_gil(|py| py.None()))
        })
    }

    /// The assigned robot's persistent peer, available after start/run starts it.
    fn peer(&self) -> Option<PyAukiPeer> {
        self.inner.peer().map(PyAukiPeer::from_task_peer)
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
            inner.close().await.map_err(error)?;
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
    module.add_class::<PyRobot>()?;
    module.add_class::<PyTasks>()?;
    module.add_class::<PyTask>()?;
    module.add_class::<PyTaskAccessToken>()?;
    module.add(
        "TaskRuntimeError",
        module.py().get_type_bound::<exception::TaskRuntimeError>(),
    )?;
    Ok(())
}
