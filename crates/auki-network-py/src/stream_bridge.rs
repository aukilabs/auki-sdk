//! Python async iterator → Rust `futures::Stream` bridge for grimsby's
//! `stream_provider` Python binding (deliverable #4 / Path 1, locked).
//!
//! Shape (per the [grimsby doc](https://www.notion.so/3575c8e965928079a955ed9573bbb398)
//! Lane B status log entry 2026-05-05):
//!
//! 1. The wrapper boots a **dedicated Python thread** running an asyncio
//!    event loop forever, on first use. The loop's `TaskLocals` are
//!    cached in a process-wide `OnceLock`.
//! 2. When the SDK invokes the Python `stream_provider` and the producer
//!    returns an async iterator, the wrapper hands the SDK a
//!    [`futures::Stream`] backed by [`PyAsyncIterStream`]. Each call to
//!    `__anext__` runs as a coroutine **on the dedicated asyncio loop**
//!    (via [`pyo3_async_runtimes::into_future_with_locals`] — the locals
//!    point at the loop, not at the caller's task locals).
//!
//! Pattern A per the BoosterApp Claude flag (status log 2026-05-05):
//! the SDK owns the Python event loop internally; the caller's process
//! stays sync-shaped (`BaseHTTPServer`, `rclpy` spinner, `threading.Lock`
//! control flow) and never has to host an asyncio loop itself. The
//! provider's `async def` generator runs on the SDK's loop, with
//! `finally` blocks firing naturally when the SDK drops the iterator on
//! consumer disconnect.
//!
//! Synchronisation note: `loop.run_forever()` is called from the
//! dedicated Python thread; `into_future_with_locals` schedules
//! coroutines onto the loop via `loop.call_soon_threadsafe` under the
//! hood. No GIL contention with the caller's main Python thread beyond
//! the brief acquire on each scheduling.

use pyo3::exceptions::PyStopAsyncIteration;
use pyo3::prelude::*;
use pyo3_async_runtimes::TaskLocals;
use std::sync::OnceLock;

/// Shared `TaskLocals` pointing at the wrapper's dedicated asyncio
/// event loop. Lazily initialized on first call to [`asyncio_locals`];
/// the loop runs forever on a daemon Python thread for the rest of the
/// process lifetime.
///
/// Returning `&'static TaskLocals` rather than cloning per call so
/// callers can borrow without re-acquiring the GIL on each invocation.
pub(crate) fn asyncio_locals() -> &'static TaskLocals {
    static LOCALS: OnceLock<TaskLocals> = OnceLock::new();
    LOCALS.get_or_init(|| {
        Python::with_gil(|py| -> PyResult<TaskLocals> {
            // Build a fresh asyncio event loop and run it on a daemon
            // thread forever. `daemon=True` so the thread doesn't keep
            // the Python interpreter alive on shutdown.
            let asyncio = py.import_bound("asyncio")?;
            let event_loop = asyncio.call_method0("new_event_loop")?;

            let threading = py.import_bound("threading")?;
            let kwargs = pyo3::types::PyDict::new_bound(py);
            kwargs.set_item("target", event_loop.getattr("run_forever")?)?;
            kwargs.set_item("daemon", true)?;
            let thread = threading.call_method("Thread", (), Some(&kwargs))?;
            thread.call_method0("start")?;

            // `copy_context` snapshots the *current* contextvars so
            // coroutines running on this loop inherit them (matters for
            // `tracing` / logging context-propagation if a downstream
            // consumer relies on contextvars).
            TaskLocals::new(event_loop).copy_context(py)
        })
        .expect("initialize wrapper asyncio event loop")
    })
}

/// Wraps a Python async iterator (any object with `__anext__`) and drives
/// it from Rust via `pyo3-async-runtimes`'s tokio bridge, against the
/// wrapper's shared asyncio loop.
///
/// One `next().await` = one `__anext__()` round trip on the Python side.
pub(crate) struct PyAsyncIterStream {
    aiter: Py<PyAny>,
}

impl PyAsyncIterStream {
    /// Wrap an existing Python async iterator. The argument is whatever
    /// `aiter()` would have returned in Python — i.e. an async generator
    /// instance, or any object exposing `__anext__`. (The `__aiter__`
    /// step is the caller's job; this struct only drives `__anext__`.)
    pub(crate) fn new(aiter: Py<PyAny>) -> Self {
        Self { aiter }
    }

    /// One round trip: call `__anext__()` on the iterator, await the
    /// resulting coroutine on the wrapper's asyncio loop, return the
    /// next item.
    ///
    /// - `Ok(Some(value))` — Python yielded `value`.
    /// - `Ok(None)` — Python raised `StopAsyncIteration`. Iterator done.
    /// - `Err(e)` — any other Python exception, including raised-from
    ///   inside the generator. Caller should treat the iterator as
    ///   exhausted; calling `next` again is undefined.
    pub(crate) async fn next(&self) -> PyResult<Option<PyObject>> {
        let locals = asyncio_locals();

        // Step 1 (under GIL): call `__anext__()` to obtain a coroutine,
        // then convert it into a Rust future bound to the wrapper's
        // asyncio loop via the explicit `TaskLocals`. Going through
        // `into_future_with_locals` (rather than `into_future`) means
        // we don't depend on whatever tokio task happens to be polling
        // us having locals set — the locals are explicit.
        let fut = Python::with_gil(|py| -> PyResult<_> {
            let coro = self.aiter.bind(py).call_method0("__anext__")?;
            pyo3_async_runtimes::into_future_with_locals(locals, coro)
        })?;

        // Step 2 (no GIL): await the coroutine. The asyncio loop on the
        // dedicated Python thread drives it; pyo3-async-runtimes hands
        // the result back through a oneshot channel.
        match fut.await {
            Ok(value) => Ok(Some(value)),
            Err(e) => {
                let is_stop = Python::with_gil(|py| e.is_instance_of::<PyStopAsyncIteration>(py));
                if is_stop {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Best-effort shutdown of the Python async iterator. Calls
    /// `aiter.aclose()` if present — async generators expose this as
    /// the entry point to drive a `GeneratorExit` exception through the
    /// generator, which fires the generator's `finally` blocks (the
    /// natural cleanup hook BoosterApp wants per the doc).
    ///
    /// Errors from `aclose` are intentionally swallowed: this fires on
    /// the Drop path of a `SourceStream` whose consumer disconnected,
    /// and "the iterator already errored" or "no `aclose`" is fine.
    pub(crate) async fn aclose(&self) {
        let locals = asyncio_locals();
        let fut = Python::with_gil(|py| -> Option<_> {
            let bound = self.aiter.bind(py);
            // Plain iterators (anything with `__anext__` but not a
            // generator) won't have `aclose`. Treat as no-op.
            let aclose = bound.getattr("aclose").ok()?;
            let coro = aclose.call0().ok()?;
            pyo3_async_runtimes::into_future_with_locals(locals, coro).ok()
        });
        if let Some(fut) = fut {
            let _ = fut.await;
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────
//
// Run serially: `cargo test -p auki-network-py --lib stream_bridge:: --
// --test-threads=1`. The dedicated-loop init fires once per process and
// `pyo3::prepare_freethreaded_python` doesn't tolerate concurrent
// initialization from multiple test threads.

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a Python async generator yielding 0..n and return the live
    /// async iterator (already past the `__aiter__` step). The returned
    /// `Py<PyAny>` is GIL-detached so it can cross `Python::with_gil`
    /// boundaries.
    fn build_async_iter_yielding(py: Python<'_>, n: i64) -> PyResult<Py<PyAny>> {
        let code = "
async def _gen(n):
    for i in range(n):
        yield i
";
        let module = PyModule::from_code_bound(py, code, "test_gen.py", "test_gen")?;
        let gen_fn = module.getattr("_gen")?;
        let aiter = gen_fn.call1((n,))?;
        Ok(aiter.unbind())
    }

    /// Drive a future on the same asyncio loop the production wrapper
    /// uses — exercises the dedicated-thread loop init path under test.
    fn block_on_wrapper_loop<F, T>(fut: F) -> PyResult<T>
    where
        F: std::future::Future<Output = PyResult<T>> + Send + 'static,
        T: Send + Sync + 'static,
    {
        // Build a tokio runtime for the test (we deliberately don't share
        // the wrapper's `cluster_tokio_runtime()` here — that's lib.rs's
        // singleton and we keep this module isolated).
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("test tokio runtime");
        rt.block_on(fut)
    }

    /// Smoke test: a 3-yield Python async generator drives all the way
    /// through the bridge — three values + StopAsyncIteration → None.
    #[test]
    fn three_yields_then_stop_async_iteration() {
        pyo3::prepare_freethreaded_python();
        // Force loop init under GIL before we start the tokio runtime.
        let _ = asyncio_locals();

        block_on_wrapper_loop(async {
            let aiter = Python::with_gil(|py| build_async_iter_yielding(py, 3))?;
            let stream = PyAsyncIterStream::new(aiter);

            for expected in 0..3 {
                let got = stream.next().await?;
                let value = got.expect("iterator should yield a value");
                let n = Python::with_gil(|py| value.extract::<i64>(py))?;
                assert_eq!(n, expected, "wrong yield order");
            }

            let end = stream.next().await?;
            assert!(end.is_none(), "iterator should be exhausted after 3 yields");

            Ok(())
        })
        .expect("path 1 round trip");
    }

    /// A Python generator that raises a non-stop exception mid-iteration.
    /// Bridge surfaces it as `Err(e)` (not `Ok(None)`); the discriminator
    /// between "exhausted" and "errored" is the `StopAsyncIteration`
    /// check.
    #[test]
    fn propagates_non_stop_exception() {
        pyo3::prepare_freethreaded_python();
        let _ = asyncio_locals();

        block_on_wrapper_loop(async {
            let aiter = Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                let code = "
async def _gen():
    yield 7
    raise RuntimeError('boom')
";
                let module = PyModule::from_code_bound(py, code, "test_gen_err.py", "test_gen_err")?;
                Ok(module.getattr("_gen")?.call0()?.unbind())
            })?;
            let stream = PyAsyncIterStream::new(aiter);

            let first = stream.next().await?.expect("first yield");
            let n = Python::with_gil(|py| first.extract::<i64>(py))?;
            assert_eq!(n, 7);

            let err = stream
                .next()
                .await
                .expect_err("non-stop exception must surface as Err");
            let msg = err.to_string();
            assert!(msg.contains("boom"), "error should carry the message: {msg}");

            Ok(())
        })
        .expect("path 1 propagates exceptions");
    }

    /// `aclose` drives `GeneratorExit` through the iterator's `finally`
    /// — the cleanup hook BoosterApp's preview-fanout subscriber relies
    /// on. Asserts the generator's `finally` runs.
    #[test]
    fn aclose_runs_finally_block() {
        pyo3::prepare_freethreaded_python();
        let _ = asyncio_locals();

        block_on_wrapper_loop(async {
            // Build a generator that sets a module-level flag in its
            // `finally`, so we can verify the block ran.
            let (aiter, module): (Py<PyAny>, Py<PyModule>) = Python::with_gil(|py| -> PyResult<_> {
                let code = "
finally_ran = False
async def _gen():
    global finally_ran
    try:
        yield 1
        yield 2
    finally:
        finally_ran = True
";
                let module = PyModule::from_code_bound(py, code, "test_close.py", "test_close")?;
                let aiter = module.getattr("_gen")?.call0()?.unbind();
                Ok((aiter, module.unbind()))
            })?;

            let stream = PyAsyncIterStream::new(aiter);
            // Pull one value to enter the generator's body.
            let first = stream.next().await?.expect("first yield");
            let n = Python::with_gil(|py| first.extract::<i64>(py))?;
            assert_eq!(n, 1);

            // Close before exhausting. `finally` should run.
            stream.aclose().await;

            Python::with_gil(|py| {
                let ran: bool = module.bind(py).getattr("finally_ran")?.extract()?;
                assert!(ran, "finally block must run after aclose");
                Ok::<(), PyErr>(())
            })?;

            Ok(())
        })
        .expect("aclose drives finally");
    }
}
