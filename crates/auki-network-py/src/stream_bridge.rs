//! Path 1 prototype: bridge a Python async iterator → Rust `futures::Stream`.
//!
//! [Grimsby](https://www.notion.so/3575c8e965928079a955ed9573bbb398) Lane B
//! deliverable #4 evaluates two paths for the Python `stream_provider`'s
//! source-Stream wire shape:
//!
//! - **Path 1 (this file)**: provider returns a Python `async def` generator
//!   (an object supporting `__aiter__` / `__anext__`); the Rust binding
//!   converts each `__anext__()` coroutine into a tokio-driven future via
//!   [`pyo3_async_runtimes::tokio::into_future`]. Preferred by BoosterApp
//!   because the generator's `finally` block is a natural cleanup hook
//!   when the SDK drops the iterator on consumer disconnect.
//! - **Path 2 (fallback)**: provider returns a sync object exposing
//!   `async def next() -> Optional[T]`; same wire semantics but Python
//!   loses the `finally`-on-Drop affordance.
//!
//! This prototype is here to evaluate Path 1 honestly. If the bridging
//! turns out to be meaningfully more PyO3 plumbing than Path 2, it goes
//! into the grimsby status log and we offer Path 2 to BoosterApp.
//!
//! ## Bridge shape
//!
//! [`PyAsyncIterStream`] wraps a Python async iterator (`Py<PyAny>`) and
//! exposes an async [`next`][PyAsyncIterStream::next] that drives one
//! `__anext__()` round trip. `Some(value)` for a yielded item, `None`
//! when the iterator raises `StopAsyncIteration`, `Err(e)` for any
//! other Python exception.
//!
//! Wrapping into a [`futures::Stream`] is straightforward (e.g.
//! [`futures::stream::unfold`]); kept out of this prototype to keep the
//! surface minimal and the failure mode obvious.
//!
//! ## Why dev-only
//!
//! The production `stream_provider` wrapper (deliverable #6) waits on
//! the design call for Path 1 vs Path 2. Until then the bridge lives as
//! a `#[cfg(test)]`-gated experiment so we can feel the friction without
//! committing the crate's `[dependencies]` to `pyo3-async-runtimes`.

#![cfg(test)]

use pyo3::exceptions::PyStopAsyncIteration;
use pyo3::prelude::*;

/// Wraps a Python async iterator (any object with `__anext__`) and
/// drives it from Rust via `pyo3-async-runtimes`'s tokio bridge.
///
/// One `next().await` = one `__anext__()` round trip on the Python side.
/// The asyncio event loop must be running on a tokio task locals
/// context (set up by [`pyo3_async_runtimes::tokio::run`] or the
/// `#[pyo3_async_runtimes::tokio::main]` attribute).
pub struct PyAsyncIterStream {
    aiter: Py<PyAny>,
}

impl PyAsyncIterStream {
    /// Wrap an existing Python async iterator. The argument is whatever
    /// `aiter()` would have returned in Python — i.e. an async generator
    /// instance, or any object exposing `__anext__`. (The `__aiter__`
    /// step is the caller's job; this struct only drives `__anext__`.)
    pub fn new(aiter: Py<PyAny>) -> Self {
        Self { aiter }
    }

    /// One round trip: call `__anext__()` on the iterator, await the
    /// resulting coroutine, return the next item.
    ///
    /// - `Ok(Some(value))` — Python yielded `value`.
    /// - `Ok(None)` — Python raised `StopAsyncIteration`. Iterator done.
    /// - `Err(e)` — any other Python exception, including raised-from
    ///   inside the generator. Caller should treat the iterator as
    ///   exhausted; calling `next` again is undefined.
    pub async fn next(&self) -> PyResult<Option<PyObject>> {
        // Step 1 (under GIL): call `__anext__()` to obtain a coroutine,
        // then convert it into a tokio-driven Rust future. The
        // conversion has to happen here because `pyo3_async_runtimes`
        // captures the current asyncio task locals at conversion time.
        let fut = Python::with_gil(|py| -> PyResult<_> {
            let coro = self.aiter.bind(py).call_method0("__anext__")?;
            pyo3_async_runtimes::tokio::into_future(coro)
        })?;

        // Step 2 (no GIL): await the coroutine. The asyncio loop drives
        // it on whatever thread `pyo3_async_runtimes::tokio::run`
        // bound; pyo3-async-runtimes hands the result back through a
        // oneshot channel.
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
}

// ─── Tests ───────────────────────────────────────────────────────────────────
//
// Run serially: `cargo test -p auki-network-py --lib stream_bridge:: --
// --test-threads=1`. `pyo3_async_runtimes::tokio::run` shares a
// process-wide tokio runtime + per-call asyncio event loop; parallel
// test threads contend on those globals and deadlock. Serial passes
// in <1s.

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

    /// Smoke test: a 3-yield Python async generator drives all the way
    /// through the bridge — three values + StopAsyncIteration → None.
    #[test]
    fn path_1_three_yields_then_stop() {
        pyo3::prepare_freethreaded_python();

        let result = Python::with_gil(|py| -> PyResult<()> {
            pyo3_async_runtimes::tokio::run(py, async {
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
        });

        result.expect("path 1 prototype round trip");
    }

    /// A Python generator that raises a non-stop exception mid-iteration.
    /// Bridge surfaces it as `Err(e)` (not `Ok(None)`); the discriminator
    /// between "exhausted" and "errored" is the `StopAsyncIteration`
    /// check.
    #[test]
    fn path_1_propagates_non_stop_exception() {
        pyo3::prepare_freethreaded_python();

        let result = Python::with_gil(|py| -> PyResult<()> {
            pyo3_async_runtimes::tokio::run(py, async {
                let aiter = Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let code = "
async def _gen():
    yield 7
    raise RuntimeError('boom')
";
                    let module = PyModule::from_code_bound(
                        py,
                        code,
                        "test_gen_err.py",
                        "test_gen_err",
                    )?;
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
        });

        result.expect("path 1 propagates exceptions");
    }
}
