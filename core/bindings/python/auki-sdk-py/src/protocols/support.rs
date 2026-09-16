#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob",
    feature = "stream"
))]
use std::sync::Arc;
#[cfg(feature = "blob")]
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use auki_sdk_rs::{AuthenticatedPeer, Multiaddr, PeerId};
#[cfg(any(feature = "blob", feature = "stream"))]
use parking_lot::Mutex;
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

#[cfg(any(feature = "blob", feature = "stream"))]
pub(super) use crate::python_task::{CancelablePythonAwaitable, PythonTaskState};

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
        subject: uuid::Uuid::nil().to_string(),
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
            let mut requester = requester(peer_id);
            requester.subject = " User|Case-敏感 ".into();
            let value = requester_to_python(py, &requester).unwrap();
            let value = value.bind(py);
            assert_eq!(
                value
                    .get_item("subject")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                requester.subject,
            );
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
