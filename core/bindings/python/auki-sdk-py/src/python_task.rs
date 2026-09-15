//! Shared asyncio Task bridge. Cancellation waits for the actual task's cleanup.
use parking_lot::{Mutex, RwLock};
use pyo3::{exceptions::PyRuntimeError, prelude::*, sync::GILOnceCell, types::PyModule};
use pyo3_async_runtimes::TaskLocals;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::sync::{oneshot, watch};
type PythonCallback = Arc<Py<PyAny>>;
pub(crate) type CompletionHook = Arc<dyn Fn() + Send + Sync>;

/// State for one actual `asyncio.Task` running on its captured event loop.
///
/// Keeping the real Task, rather than the outer Future returned by
/// `run_coroutine_threadsafe`, makes completion a reliable cleanup barrier:
/// the Rust receiver resolves only after Python cancellation and `finally`
/// blocks have finished.
pub(crate) struct PythonTaskState {
    event_loop: PythonCallback,
    task: RwLock<Option<PythonCallback>>,
    running: AtomicBool,
    cancel_requested: AtomicBool,
    completed: watch::Sender<bool>,
    sender: Mutex<Option<oneshot::Sender<PyResult<PyObject>>>>,
    completion_hook: Mutex<Option<CompletionHook>>,
}

impl PythonTaskState {
    #[cfg(feature = "blob")]
    pub(crate) fn python_references(&self) -> Vec<PythonCallback> {
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

    pub(crate) fn cancel(&self) {
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

    #[cfg(feature = "blob")]
    pub(crate) fn is_completed(&self) -> bool {
        *self.completed.borrow()
    }

    #[cfg(feature = "blob")]
    pub(crate) async fn wait_completed(&self) {
        let mut completed = self.completed.subscribe();
        while !*completed.borrow() {
            if completed.changed().await.is_err() {
                return;
            }
        }
    }
}

#[pyclass]
struct PythonTaskCallbacks {
    state: Weak<PythonTaskState>,
}

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
pub(crate) struct CancelablePythonAwaitable {
    state: Arc<PythonTaskState>,
    receiver: oneshot::Receiver<PyResult<PyObject>>,
    finished: bool,
}

impl CancelablePythonAwaitable {
    pub(crate) fn schedule(
        py: Python<'_>,
        locals: &TaskLocals,
        awaitable: Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        Self::schedule_with_completion(py, locals, awaitable, None)
    }

    pub(crate) fn schedule_with_completion(
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

    pub(crate) fn state(&self) -> Arc<PythonTaskState> {
        Arc::clone(&self.state)
    }
}

fn close_python_awaitable(awaitable: &Bound<'_, PyAny>) {
    if let Ok(close) = awaitable.getattr("close") {
        let _ = close.call0();
    }
}

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

impl Drop for CancelablePythonAwaitable {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.state.cancel();
    }
}
