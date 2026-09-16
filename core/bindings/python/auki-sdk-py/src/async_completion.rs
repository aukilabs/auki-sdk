//! Drain the native completion task before resuming a Python awaiter.
//!
//! async-runtimes 0.22 publishes the Python result from an inner task before
//! its outer task has joined it and released its Python references. In
//! particular call_soon_threadsafe can release the GIL while publishing. A
//! Future being ready therefore does not make interpreter teardown safe.

use std::{cell::RefCell, future::Future, pin::Pin};

use parking_lot::Mutex;
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use pyo3_async_runtimes::{
    TaskLocals,
    generic::{ContextExt, Runtime},
};
use tokio::task::JoinHandle;

type CompletionFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

thread_local! {
    // Only the synchronous, outer spawn is intercepted. Its nested task is
    // joined by async-runtimes itself before this handle becomes ready.
    static COMPLETION: RefCell<Vec<Option<CompletionFuture>>> = const { RefCell::new(Vec::new()) };
}

// A stack preserves the outer capture if a custom event loop reenters the
// binding while creating a Future. Drop also restores it on an unwind.
struct Capture;

impl Drop for Capture {
    fn drop(&mut self) {
        COMPLETION.with(|slot| {
            slot.borrow_mut().pop();
        });
    }
}

struct CompletionRuntime;

impl Runtime for CompletionRuntime {
    type JoinError = tokio::task::JoinError;
    type JoinHandle = Pin<Box<dyn Future<Output = Result<(), Self::JoinError>> + Send>>;

    fn spawn<F>(future: F) -> Self::JoinHandle
    where
        F: Future<Output = ()> + Send + 'static,
    {
        COMPLETION.with(|slot| {
            if let Some(captured) = slot.borrow_mut().last_mut() {
                *captured = Some(Box::pin(future));
                // The upstream outer handle is discarded, never awaited.
                Box::pin(async { Ok(()) }) as Self::JoinHandle
            } else {
                Box::pin(pyo3_async_runtimes::tokio::get_runtime().spawn(future))
                    as Self::JoinHandle
            }
        })
    }
}

impl ContextExt for CompletionRuntime {
    fn scope<F, R>(locals: TaskLocals, future: F) -> Pin<Box<dyn Future<Output = R> + Send>>
    where
        F: Future<Output = R> + Send + 'static,
    {
        Box::pin(async move { pyo3_async_runtimes::tokio::scope(locals, future).await })
    }

    fn get_task_locals() -> Option<TaskLocals> {
        Python::with_gil(|py| pyo3_async_runtimes::tokio::get_current_locals(py).ok())
    }
}

#[pyclass]
struct DrainCompletion {
    task: Mutex<Option<JoinHandle<()>>>,
}

#[pymethods]
impl DrainCompletion {
    fn __call__(&self, py: Python<'_>, _future: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(task) = self.task.lock().take() {
            py.allow_threads(|| pyo3_async_runtimes::tokio::get_runtime().block_on(task))
                .map_err(|error| {
                    PyRuntimeError::new_err(format!("async completion failed: {error}"))
                })?;
        }
        Ok(())
    }
}

/// Convert an operation without letting its native completion tail outlive the
/// asyncio done callbacks. Cancellation still uses the upstream callback, which
/// is registered before our drain; detached operation ownership is unchanged.
pub fn future_into_py<F, T>(py: Python<'_>, future: F) -> PyResult<Bound<'_, PyAny>>
where
    F: Future<Output = PyResult<T>> + Send + 'static,
    T: IntoPy<PyObject>,
{
    let locals = pyo3_async_runtimes::tokio::get_current_locals(py)?;
    COMPLETION.with(|slot| slot.borrow_mut().push(None));
    let capture = Capture;
    let result = pyo3_async_runtimes::generic::future_into_py_with_locals::<CompletionRuntime, _, _>(
        py, locals, future,
    );
    let task = COMPLETION.with(|slot| slot.borrow_mut().last_mut().and_then(Option::take));
    drop(capture);
    let result = result?;
    let task = task.ok_or_else(|| PyRuntimeError::new_err("missing native completion task"))?;
    let drain = Py::new(
        py,
        DrainCompletion {
            task: Mutex::new(None),
        },
    )?;
    // Upstream cancellation is registered first. Until this registration also
    // succeeds, the captured bridge is unpolled: any error simply drops it and
    // its Python references, with no native task to detach or synchronously join.
    result.call_method1("add_done_callback", (&drain,))?;
    *drain.borrow(py).task.lock() = Some(pyo3_async_runtimes::tokio::get_runtime().spawn(task));
    Ok(result)
}
