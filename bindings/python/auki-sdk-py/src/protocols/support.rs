#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob",
    feature = "stream"
))]
use std::sync::Arc;
#[cfg(feature = "blob")]
use std::{collections::HashMap, sync::atomic::AtomicU64, time::Duration};
#[cfg(any(feature = "blob", feature = "stream"))]
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use auki_sdk_rs::{AuthenticatedPeer, Multiaddr, PeerId};
#[cfg(any(feature = "blob", feature = "stream"))]
use parking_lot::{Mutex, RwLock};
#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob",
    feature = "stream"
))]
use pyo3::exceptions::PyTypeError;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyAny,
};
#[cfg(any(feature = "blob", feature = "stream"))]
use pyo3::{sync::GILOnceCell, types::PyModule};
#[cfg(any(feature = "blob", feature = "stream"))]
use pyo3_async_runtimes::TaskLocals;
use serde::Serialize;
#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "message",
    feature = "stream"
))]
use serde::de::DeserializeOwned;
#[cfg(any(feature = "blob", feature = "stream"))]
use tokio::sync::{oneshot, watch};

#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob",
    feature = "stream"
))]
pub(super) type PythonCallback = Arc<Py<PyAny>>;

#[cfg(any(feature = "blob", feature = "stream"))]
pub(super) type CompletionHook = Arc<dyn Fn() + Send + Sync>;

/// State for one actual `asyncio.Task` running on its captured event loop.
///
/// Keeping the real Task, rather than the outer Future returned by
/// `run_coroutine_threadsafe`, makes completion a reliable cleanup barrier:
/// the Rust receiver resolves only after Python cancellation and `finally`
/// blocks have finished.
#[cfg(any(feature = "blob", feature = "stream"))]
pub(super) struct PythonTaskState {
    event_loop: PythonCallback,
    task: RwLock<Option<PythonCallback>>,
    running: AtomicBool,
    cancel_requested: AtomicBool,
    completed: watch::Sender<bool>,
    sender: Mutex<Option<oneshot::Sender<PyResult<PyObject>>>>,
    completion_hook: Mutex<Option<CompletionHook>>,
}

#[cfg(any(feature = "blob", feature = "stream"))]
impl PythonTaskState {
    pub(super) fn python_references(&self) -> Vec<PythonCallback> {
        let mut references = vec![Arc::clone(&self.event_loop)];
        references.extend(self.task.read().iter().cloned());
        references
    }

    fn set_task(&self, task: PythonCallback) {
        self.task.write().replace(task);
        // Python 3.12's eager task factory may call `running()` from inside
        // create_task(), before `started()` can give us the Task.
        if self.running.load(Ordering::Acquire) && self.cancel_requested.load(Ordering::Acquire) {
            self.schedule_cancel();
        }
    }

    fn mark_running(&self) {
        self.running.store(true, Ordering::Release);
        if self.cancel_requested.load(Ordering::Acquire) {
            self.schedule_cancel();
        }
    }

    fn schedule_cancel(&self) {
        let Some(task) = self.task.read().clone() else {
            return;
        };
        Python::with_gil(|py| {
            let Ok(cancel) = task.bind(py).getattr("cancel") else {
                return;
            };
            let _ = self
                .event_loop
                .bind(py)
                .call_method1("call_soon_threadsafe", (cancel,));
        });
    }

    pub(super) fn cancel(&self) {
        if self.cancel_requested.swap(true, Ordering::AcqRel) {
            return;
        }
        if self.running.load(Ordering::Acquire) {
            self.schedule_cancel();
        }
    }

    fn complete(&self, result: PyResult<PyObject>) {
        self.task.write().take();
        if let Some(sender) = self.sender.lock().take() {
            let _ = sender.send(result);
        }
        self.completed.send_replace(true);
        if let Some(hook) = self.completion_hook.lock().take() {
            hook();
        }
    }

    pub(super) fn is_completed(&self) -> bool {
        *self.completed.borrow()
    }

    #[cfg(feature = "blob")]
    pub(super) async fn wait_completed(&self) {
        let mut completed = self.completed.subscribe();
        while !*completed.borrow() {
            if completed.changed().await.is_err() {
                return;
            }
        }
    }
}

#[cfg(any(feature = "blob", feature = "stream"))]
#[pyclass]
struct PythonTaskCallbacks {
    state: Weak<PythonTaskState>,
}

#[cfg(any(feature = "blob", feature = "stream"))]
#[pymethods]
impl PythonTaskCallbacks {
    fn started(&self, task: &Bound<'_, PyAny>) {
        if let Some(state) = self.state.upgrade() {
            state.set_task(Arc::new(task.clone().unbind()));
        } else {
            let _ = task.call_method0("cancel");
        }
    }

    fn running(&self) {
        if let Some(state) = self.state.upgrade() {
            state.mark_running();
        }
    }

    fn failed(&self, error: &Bound<'_, PyAny>) {
        if let Some(state) = self.state.upgrade() {
            state.complete(Err(PyErr::from_value_bound(error.clone())));
        }
    }

    fn __call__(&self, task: &Bound<'_, PyAny>) {
        let result = task.call_method0("result").map(Bound::unbind);
        if let Some(state) = self.state.upgrade() {
            state.complete(result);
        }
    }
}

/// A cancellation-safe Python awaitable backed by an actual `asyncio.Task`.
#[cfg(any(feature = "blob", feature = "stream"))]
pub(super) struct CancelablePythonAwaitable {
    state: Arc<PythonTaskState>,
    receiver: oneshot::Receiver<PyResult<PyObject>>,
    finished: bool,
}

#[cfg(any(feature = "blob", feature = "stream"))]
impl CancelablePythonAwaitable {
    #[cfg(feature = "stream")]
    pub(super) fn schedule(
        py: Python<'_>,
        locals: &TaskLocals,
        awaitable: Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        Self::schedule_with_completion(py, locals, awaitable, None)
    }

    pub(super) fn schedule_with_completion(
        py: Python<'_>,
        locals: &TaskLocals,
        awaitable: Bound<'_, PyAny>,
        completion_hook: Option<CompletionHook>,
    ) -> PyResult<Self> {
        static TASK_STARTER: GILOnceCell<Py<PyAny>> = GILOnceCell::new();
        let starter = TASK_STARTER.get_or_try_init(py, || {
            let module = PyModule::from_code_bound(
                py,
                concat!(
                    "import asyncio\n",
                    "async def _auki_await(value, callbacks):\n",
                    "    callbacks.running()\n",
                    "    return await value\n",
                    "def _auki_close(value):\n",
                    "    close = getattr(value, 'close', None)\n",
                    "    if close is not None:\n",
                    "        close()\n",
                    "def _auki_start(value, callbacks):\n",
                    "    wrapper = _auki_await(value, callbacks)\n",
                    "    try:\n",
                    "        task = asyncio.get_running_loop().create_task(wrapper)\n",
                    "    except BaseException as error:\n",
                    "        wrapper.close()\n",
                    "        _auki_close(value)\n",
                    "        callbacks.failed(error)\n",
                    "        return\n",
                    "    try:\n",
                    "        task.add_done_callback(callbacks)\n",
                    "    except BaseException as error:\n",
                    "        task.cancel()\n",
                    "        callbacks.failed(error)\n",
                    "        return\n",
                    "    callbacks.started(task)\n",
                ),
                "_auki_sdk_awaitable_bridge.py",
                "_auki_sdk_awaitable_bridge",
            )?;
            Ok::<_, PyErr>(module.getattr("_auki_start")?.unbind())
        })?;
        let (sender, receiver) = oneshot::channel();
        let (completed, _) = watch::channel(false);
        let state = Arc::new(PythonTaskState {
            event_loop: Arc::new(locals.event_loop(py).unbind()),
            task: RwLock::new(None),
            running: AtomicBool::new(false),
            cancel_requested: AtomicBool::new(false),
            completed,
            sender: Mutex::new(Some(sender)),
            completion_hook: Mutex::new(completion_hook),
        });
        let callbacks = Py::new(
            py,
            PythonTaskCallbacks {
                state: Arc::downgrade(&state),
            },
        )?;
        let context = locals.context(py).call_method0("copy")?;
        let context_run = context.getattr("run")?;
        if let Err(error) = locals.event_loop(py).call_method1(
            "call_soon_threadsafe",
            (context_run, starter.bind(py), awaitable.clone(), callbacks),
        ) {
            close_python_awaitable(&awaitable);
            return Err(error);
        }
        Ok(Self {
            state,
            receiver,
            finished: false,
        })
    }

    pub(super) fn state(&self) -> Arc<PythonTaskState> {
        Arc::clone(&self.state)
    }
}

#[cfg(any(feature = "blob", feature = "stream"))]
fn close_python_awaitable(awaitable: &Bound<'_, PyAny>) {
    if let Ok(close) = awaitable.getattr("close") {
        let _ = close.call0();
    }
}

#[cfg(any(feature = "blob", feature = "stream"))]
impl Future for CancelablePythonAwaitable {
    type Output = PyResult<PyObject>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.receiver).poll(context) {
            Poll::Ready(Ok(result)) => {
                self.finished = true;
                Poll::Ready(result)
            }
            Poll::Ready(Err(_)) => {
                self.finished = true;
                Poll::Ready(Err(PyRuntimeError::new_err(
                    "Python awaitable ended without reporting a result",
                )))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(any(feature = "blob", feature = "stream"))]
impl Drop for CancelablePythonAwaitable {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.state.cancel();
    }
}

#[cfg(feature = "blob")]
struct PythonTaskRegistryInner {
    next_id: AtomicU64,
    tasks: Mutex<HashMap<u64, Arc<PythonTaskState>>>,
}

/// Active-only registry used when an endpoint close must await Python cleanup.
#[cfg(feature = "blob")]
#[derive(Clone)]
pub(super) struct PythonTaskRegistry {
    inner: Arc<PythonTaskRegistryInner>,
}

#[cfg(feature = "blob")]
impl Default for PythonTaskRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(PythonTaskRegistryInner {
                next_id: AtomicU64::new(0),
                tasks: Mutex::new(HashMap::new()),
            }),
        }
    }
}

#[cfg(feature = "blob")]
impl PythonTaskRegistry {
    pub(super) fn schedule(
        &self,
        py: Python<'_>,
        locals: &TaskLocals,
        awaitable: Bound<'_, PyAny>,
    ) -> PyResult<CancelablePythonAwaitable> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let weak = Arc::downgrade(&self.inner);
        let completion_hook: CompletionHook = Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.tasks.lock().remove(&id);
            }
        });
        let scheduled = CancelablePythonAwaitable::schedule_with_completion(
            py,
            locals,
            awaitable,
            Some(completion_hook),
        )?;
        let state = scheduled.state();
        self.inner.tasks.lock().insert(id, Arc::clone(&state));
        // The event loop may complete the Task between scheduling and insert.
        if state.is_completed() {
            self.inner.tasks.lock().remove(&id);
        }
        Ok(scheduled)
    }

    pub(super) fn visit(
        &self,
        visit: &pyo3::pyclass::PyVisit<'_>,
    ) -> Result<(), pyo3::pyclass::PyTraverseError> {
        let tasks = self
            .inner
            .tasks
            .lock()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for task in tasks {
            for reference in task.python_references() {
                visit.call(reference.as_ref())?;
            }
        }
        Ok(())
    }

    pub(super) async fn cancel_and_wait(&self, timeout: Duration) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let tasks = self
                .inner
                .tasks
                .lock()
                .values()
                .cloned()
                .collect::<Vec<_>>();
            if tasks.is_empty() {
                return Ok(());
            }
            for task in &tasks {
                task.cancel();
            }
            for task in tasks {
                if tokio::time::timeout_at(deadline, task.wait_completed())
                    .await
                    .is_err()
                {
                    return Err("timed out waiting for Python provider cleanup".into());
                }
            }
        }
    }
}

pub(super) fn runtime_error(context: &'static str, error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(format!("{context}: {error}"))
}

/// Run synchronous native setup with the binding's Tokio reactor installed.
///
/// Python calls endpoint `mount()` on its asyncio thread, while libp2p service
/// registration starts Tokio tasks synchronously. Keeping this boundary here
/// lets every protocol retain a small synchronous mount API without relying on
/// whichever thread happened to invoke Python.
pub(super) fn enter_tokio_runtime<T>(operation: impl FnOnce() -> T) -> T {
    let _runtime = pyo3_async_runtimes::tokio::get_runtime().enter();
    operation()
}

pub(super) fn parse_peer_id(raw: &str) -> PyResult<PeerId> {
    raw.parse::<PeerId>()
        .map_err(|error| PyValueError::new_err(format!("invalid remote Peer ID: {error}")))
}

pub(super) fn parse_target(raw_peer_id: &str, raw_route: &str) -> PyResult<(PeerId, Multiaddr)> {
    let peer_id = parse_peer_id(raw_peer_id)?;
    let route = raw_route
        .parse::<Multiaddr>()
        .map_err(|error| PyValueError::new_err(format!("invalid remote route: {error}")))?;
    Ok((peer_id, route))
}

#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob",
    feature = "stream"
))]
pub(super) fn require_callable(
    py: Python<'_>,
    callback: Py<PyAny>,
    name: &'static str,
) -> PyResult<PythonCallback> {
    if callback.bind(py).is_callable() {
        Ok(Arc::new(callback))
    } else {
        Err(PyTypeError::new_err(format!("{name} must be callable")))
    }
}

#[cfg(feature = "catalog")]
pub(super) fn optional_callable(
    py: Python<'_>,
    callback: Option<Py<PyAny>>,
    name: &'static str,
) -> PyResult<Option<PythonCallback>> {
    callback
        .map(|callback| require_callable(py, callback, name))
        .transpose()
}

#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "message",
    feature = "stream"
))]
pub(super) fn parse_python<T>(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &'static str,
) -> PyResult<T>
where
    T: DeserializeOwned,
{
    let json = py.import_bound("json")?;
    let encoded: String = json.call_method1("dumps", (value,))?.extract()?;
    serde_json::from_str(&encoded)
        .map_err(|error| PyValueError::new_err(format!("invalid {name}: {error}")))
}

pub(super) fn to_python<T>(py: Python<'_>, value: &T) -> PyResult<PyObject>
where
    T: Serialize,
{
    let encoded = serde_json::to_string(value)
        .map_err(|error| runtime_error("serialize protocol value", error))?;
    Ok(py
        .import_bound("json")?
        .call_method1("loads", (encoded,))?
        .unbind())
}

pub(super) fn requester_to_python(
    py: Python<'_>,
    requester: &AuthenticatedPeer,
) -> PyResult<PyObject> {
    let application = requester.application.as_ref().map(|application| {
        serde_json::json!({
            "name": application.name,
            "version": application.version,
        })
    });
    to_python(
        py,
        &serde_json::json!({
            "peer_id": requester.peer_id.to_string(),
            "subject": requester.subject.to_string(),
            "peer_type": requester.peer_type,
            "domain_ids": requester
                .domain_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            "scopes": requester.scopes,
            "application": application,
            "verified_until": requester.verified_until.to_rfc3339(),
        }),
    )
}

/// A synchronous provider has no Python caller to receive an exception.
/// Associate it with the callback so `sys.unraisablehook` preserves the
/// traceback, then let the native provider's decline/empty fallback apply.
#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob",
    feature = "stream"
))]
pub(super) fn report_provider_error(py: Python<'_>, callback: &Bound<'_, PyAny>, error: PyErr) {
    error.write_unraisable_bound(py, Some(callback));
}

#[cfg(test)]
pub(super) fn requester(peer_id: PeerId) -> AuthenticatedPeer {
    AuthenticatedPeer {
        peer_id,
        subject: uuid::Uuid::nil(),
        peer_type: Some("native_app".into()),
        domain_ids: vec![uuid::Uuid::nil()],
        scopes: vec!["protocol:test".into()],
        application: None,
        verified_until: "2030-01-01T00:00:00Z".parse().unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use auki_sdk_rs::Identity;

    use super::*;

    #[test]
    fn exact_target_rejects_each_invalid_component() {
        let peer_id = Identity::generate().peer_id().to_string();
        assert!(parse_target("not-a-peer", "/ip4/127.0.0.1/tcp/1").is_err());
        assert!(parse_target(&peer_id, "not-a-route").is_err());
        assert!(parse_target(&peer_id, "/ip4/127.0.0.1/tcp/1").is_ok());
    }

    #[test]
    fn requester_record_contains_authenticated_identity_and_authority() {
        Python::with_gil(|py| {
            let peer_id = Identity::generate().peer_id();
            let value = requester_to_python(py, &requester(peer_id)).unwrap();
            let value = value.bind(py);
            assert_eq!(
                value
                    .get_item("peer_id")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                peer_id.to_string()
            );
            assert_eq!(
                value
                    .get_item("peer_type")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "native_app"
            );
            assert_eq!(
                value
                    .get_item("verified_until")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "2030-01-01T00:00:00+00:00"
            );
        });
    }

    #[cfg(any(feature = "blob", feature = "stream"))]
    #[test]
    fn task_creation_failure_completes_rust_and_closes_the_coroutine() {
        pyo3::prepare_freethreaded_python();
        let (event_loop, locals, module) = Python::with_gil(|py| {
            let asyncio = py.import_bound("asyncio").unwrap();
            let event_loop = asyncio.call_method0("new_event_loop").unwrap();
            let locals = TaskLocals::new(event_loop.clone())
                .copy_context(py)
                .unwrap();
            let module = PyModule::from_code_bound(
                py,
                r#"
import asyncio

async def value():
    await asyncio.sleep(0)

def fail_task_factory(loop, coroutine, **kwargs):
    raise RuntimeError("synthetic task factory failure")
"#,
                "task_creation_failure_test.py",
                "task_creation_failure_test",
            )
            .unwrap();
            (event_loop.unbind(), locals, module.unbind())
        });

        Python::with_gil(|py| {
            let loop_for_test = event_loop.clone_ref(py);
            pyo3_async_runtimes::tokio::run_until_complete(
                event_loop.bind(py).clone(),
                async move {
                    let (scheduled, coroutine_root) = Python::with_gil(|py| {
                        loop_for_test.bind(py).call_method1(
                            "set_task_factory",
                            (module.bind(py).getattr("fail_task_factory")?,),
                        )?;
                        let coroutine = module.bind(py).getattr("value")?.call0()?;
                        let coroutine_root = coroutine.clone().unbind();
                        let scheduled = CancelablePythonAwaitable::schedule_with_completion(
                            py, &locals, coroutine, None,
                        )?;
                        Ok::<_, PyErr>((scheduled, coroutine_root))
                    })?;
                    let result = tokio::time::timeout(Duration::from_secs(1), scheduled)
                        .await
                        .expect("task creation failure must complete the Rust receiver");
                    Python::with_gil(|py| {
                        loop_for_test
                            .bind(py)
                            .call_method1("set_task_factory", (py.None(),))?;
                        let state: String = py
                            .import_bound("inspect")?
                            .call_method1("getcoroutinestate", (coroutine_root.bind(py),))?
                            .extract()?;
                        assert_eq!(state, "CORO_CLOSED");
                        Ok::<_, PyErr>(())
                    })?;
                    let error = result.expect_err("synthetic task factory must fail");
                    assert!(error.to_string().contains("synthetic task factory failure"));
                    Ok(())
                },
            )
            .unwrap();
        });
    }

    #[cfg(any(feature = "blob", feature = "stream"))]
    #[test]
    fn eager_task_cancel_before_started_still_runs_python_finally() {
        pyo3::prepare_freethreaded_python();
        let Some((event_loop, locals, module, eager_task_factory)) = Python::with_gil(|py| {
            let asyncio = py.import_bound("asyncio").unwrap();
            let Ok(eager_task_factory) = asyncio.getattr("eager_task_factory") else {
                // The eager factory is a Python 3.12+ behavior.
                return None;
            };
            let event_loop = asyncio.call_method0("new_event_loop").unwrap();
            let locals = TaskLocals::new(event_loop.clone())
                .copy_context(py)
                .unwrap();
            let module = PyModule::from_code_bound(
                py,
                r#"
import asyncio
import threading

finished = threading.Event()

async def value():
    try:
        await asyncio.Event().wait()
    finally:
        finished.set()
"#,
                "eager_task_cancellation_test.py",
                "eager_task_cancellation_test",
            )
            .unwrap();
            Some((
                event_loop.unbind(),
                locals,
                module.unbind(),
                eager_task_factory.unbind(),
            ))
        }) else {
            return;
        };

        Python::with_gil(|py| {
            let loop_for_test = event_loop.clone_ref(py);
            pyo3_async_runtimes::tokio::run_until_complete(
                event_loop.bind(py).clone(),
                async move {
                    let (scheduled, finished) = Python::with_gil(|py| {
                        loop_for_test
                            .bind(py)
                            .call_method1("set_task_factory", (eager_task_factory.bind(py),))?;
                        let coroutine = module.bind(py).getattr("value")?.call0()?;
                        let scheduled = CancelablePythonAwaitable::schedule_with_completion(
                            py, &locals, coroutine, None,
                        )?;
                        scheduled.state().cancel();
                        Ok::<_, PyErr>((scheduled, module.bind(py).getattr("finished")?.unbind()))
                    })?;
                    let result = tokio::time::timeout(Duration::from_secs(1), scheduled)
                        .await
                        .expect("eager cancellation must complete the Rust receiver");
                    Python::with_gil(|py| {
                        loop_for_test
                            .bind(py)
                            .call_method1("set_task_factory", (py.None(),))?;
                        let finished: bool = finished.bind(py).call_method0("is_set")?.extract()?;
                        assert!(finished, "Task completion preceded Python finally");
                        Ok::<_, PyErr>(())
                    })?;
                    assert!(result.is_err(), "the eager Python Task must be cancelled");
                    Ok(())
                },
            )
            .unwrap();
        });
    }
}
