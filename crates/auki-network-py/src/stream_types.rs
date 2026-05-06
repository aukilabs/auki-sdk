//! Python wrappers for grimsby's `Stream<T>` Rust API surface — wire
//! types, [`StreamDecision`], the [`PyStreamProvider`] adapter, and
//! [`StreamSubscription`] / [`FrameIterator`].
//!
//! Lock-state per the [grimsby doc](https://www.notion.so/3575c8e965928079a955ed9573bbb398):
//!
//! - **Producer side**: `stream_provider` is a *sync* Python callable
//!   `(StreamRequest) -> StreamDecision`; on `Accept`, the source is a
//!   Python async iterator (typically an `async def` generator). The
//!   wrapper drains it through [`crate::stream_bridge::PyAsyncIterStream`]
//!   on the wrapper's dedicated asyncio loop. (BoosterApp's preferred
//!   shape — `finally`-on-Drop cleanup runs naturally via `aclose`.)
//! - **Consumer side**: `runtime.open_stream(peer_id, sensor_id)` is
//!   synchronous-blocking. The returned [`StreamSubscription`] exposes
//!   accept-time metadata via `.info`; `.frames()` returns a sync
//!   iterator that blocks on each `__next__()` until the next frame
//!   arrives. Stream-end signals surface as Python exceptions raised
//!   from `__next__()`.
//!
//! Sync everywhere on the *callable surface* (Pattern A, per the
//! status log 2026-05-05) — the asyncio plumbing is internal to the
//! SDK's tokio worker. Caller processes (BoosterApp's `BaseHTTPServer`
//! sidecar; future Sentinel-as-consumer) stay sync-shaped.

use auki_network_rs::stream_protocol::{
    AcceptInfo as RustAcceptInfo, DeclineReason as RustDeclineReason,
    EndReason as RustEndReason, JpegFrame as RustJpegFrame, StreamRequest as RustStreamRequest,
};
use auki_network_rs::stream_runtime::{
    ConsumerFrame as RustConsumerFrame, OpenStreamError as RustOpenStreamError,
    ProducerFrame as RustProducerFrame, SourceStream, StreamDispatch as RustStreamDispatch,
    StreamError as RustStreamError, StreamProvider, StreamSubscription as RustStreamSubscription,
};
use futures::{Stream, StreamExt};
use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyStopIteration, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::stream_bridge::PyAsyncIterStream;

// ─── StreamRequest ───────────────────────────────────────────────────────────

/// Inbound request the SDK delivers to the Python `stream_provider`.
/// Today carries a single `sensor_id`; future additive fields land here
/// without a wire-version bump (per grimsby D2).
#[pyclass(name = "StreamRequest", frozen)]
#[derive(Clone, Debug)]
pub struct PyStreamRequest {
    pub(crate) inner: RustStreamRequest,
}

#[pymethods]
impl PyStreamRequest {
    #[new]
    #[pyo3(signature = (*, sensor_id))]
    fn new(sensor_id: String) -> Self {
        Self {
            inner: RustStreamRequest { sensor_id },
        }
    }

    #[getter]
    fn sensor_id(&self) -> &str {
        &self.inner.sensor_id
    }

    fn __repr__(&self) -> String {
        format!("StreamRequest(sensor_id={:?})", self.inner.sensor_id)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── AcceptInfo ──────────────────────────────────────────────────────────────

/// Accept-time metadata the producer commits to for the lifetime of the
/// subscription — `sensor_hash` (UI-labelling for v1 JPEG payload),
/// `clock_id` (load-bearing for `timestamp_ns` interpretation),
/// `clock_hash`.
#[pyclass(name = "AcceptInfo", frozen)]
#[derive(Clone, Debug)]
pub struct PyAcceptInfo {
    pub(crate) inner: RustAcceptInfo,
}

#[pymethods]
impl PyAcceptInfo {
    #[new]
    #[pyo3(signature = (*, sensor_hash, clock_id, clock_hash))]
    fn new(sensor_hash: String, clock_id: String, clock_hash: String) -> Self {
        Self {
            inner: RustAcceptInfo {
                sensor_hash,
                clock_id,
                clock_hash,
            },
        }
    }

    #[getter]
    fn sensor_hash(&self) -> &str {
        &self.inner.sensor_hash
    }

    #[getter]
    fn clock_id(&self) -> &str {
        &self.inner.clock_id
    }

    #[getter]
    fn clock_hash(&self) -> &str {
        &self.inner.clock_hash
    }

    fn __repr__(&self) -> String {
        format!(
            "AcceptInfo(sensor_hash={:?}, clock_id={:?}, clock_hash={:?})",
            self.inner.sensor_hash, self.inner.clock_id, self.inner.clock_hash,
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── JpegFrame ───────────────────────────────────────────────────────────────

/// Grimsby v1 payload `T` — JPEG bytes (per D4). Byte-identical to what
/// `GET /api/preview/latest.jpg` serves today over HTTP.
#[pyclass(name = "JpegFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyJpegFrame {
    pub(crate) inner: RustJpegFrame,
}

#[pymethods]
impl PyJpegFrame {
    #[new]
    #[pyo3(signature = (bytes, /))]
    fn new(bytes: Bound<'_, PyBytes>) -> Self {
        Self {
            inner: RustJpegFrame {
                bytes: bytes.as_bytes().to_vec(),
            },
        }
    }

    /// Raw JPEG bytes. Returns a fresh `bytes` copy each call.
    #[getter]
    fn bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.bytes)
    }

    fn __len__(&self) -> usize {
        self.inner.bytes.len()
    }

    fn __repr__(&self) -> String {
        format!("JpegFrame(<{} bytes>)", self.inner.bytes.len())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── DeclineReason ───────────────────────────────────────────────────────────

/// Tagged union mirroring [`RustDeclineReason`]. Construct via the
/// `static` factories; read the variant via `.kind` (snake-case string)
/// and `.detail` (`None` except for the `Other` variant).
#[pyclass(name = "DeclineReason", frozen)]
#[derive(Clone, Debug)]
pub struct PyDeclineReason {
    pub(crate) inner: RustDeclineReason,
}

#[pymethods]
impl PyDeclineReason {
    #[staticmethod]
    fn sensor_not_found() -> Self {
        Self {
            inner: RustDeclineReason::SensorNotFound,
        }
    }

    #[staticmethod]
    fn sensor_unavailable() -> Self {
        Self {
            inner: RustDeclineReason::SensorUnavailable,
        }
    }

    #[staticmethod]
    fn producer_shutting_down() -> Self {
        Self {
            inner: RustDeclineReason::ProducerShuttingDown,
        }
    }

    #[staticmethod]
    #[pyo3(signature = (*, detail))]
    fn other(detail: String) -> Self {
        Self {
            inner: RustDeclineReason::Other { detail },
        }
    }

    /// snake-case discriminator: `"sensor_not_found"`, `"sensor_unavailable"`,
    /// `"producer_shutting_down"`, or `"other"`. Stable across SDK versions
    /// (matches the Rust `serde(rename_all = "snake_case")` tag).
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            RustDeclineReason::SensorNotFound => "sensor_not_found",
            RustDeclineReason::SensorUnavailable => "sensor_unavailable",
            RustDeclineReason::ProducerShuttingDown => "producer_shutting_down",
            RustDeclineReason::Other { .. } => "other",
        }
    }

    /// Free-form detail string. Populated only for the `other` variant;
    /// `None` for the named variants.
    #[getter]
    fn detail(&self) -> Option<&str> {
        match &self.inner {
            RustDeclineReason::Other { detail } => Some(detail.as_str()),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            RustDeclineReason::Other { detail } => {
                format!("DeclineReason.other(detail={detail:?})")
            }
            RustDeclineReason::SensorNotFound => "DeclineReason.sensor_not_found()".to_string(),
            RustDeclineReason::SensorUnavailable => {
                "DeclineReason.sensor_unavailable()".to_string()
            }
            RustDeclineReason::ProducerShuttingDown => {
                "DeclineReason.producer_shutting_down()".to_string()
            }
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── EndReason ───────────────────────────────────────────────────────────────

/// Tagged union mirroring [`RustEndReason`]. Same factory + getter shape
/// as [`PyDeclineReason`].
#[pyclass(name = "EndReason", frozen)]
#[derive(Clone, Debug)]
pub struct PyEndReason {
    pub(crate) inner: RustEndReason,
}

#[pymethods]
impl PyEndReason {
    #[staticmethod]
    fn source_ended() -> Self {
        Self {
            inner: RustEndReason::SourceEnded,
        }
    }

    #[staticmethod]
    fn producer_shutting_down() -> Self {
        Self {
            inner: RustEndReason::ProducerShuttingDown,
        }
    }

    #[staticmethod]
    fn session_ended() -> Self {
        Self {
            inner: RustEndReason::SessionEnded,
        }
    }

    #[staticmethod]
    #[pyo3(signature = (*, detail))]
    fn producer_error(detail: String) -> Self {
        Self {
            inner: RustEndReason::ProducerError { detail },
        }
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            RustEndReason::SourceEnded => "source_ended",
            RustEndReason::ProducerShuttingDown => "producer_shutting_down",
            RustEndReason::SessionEnded => "session_ended",
            RustEndReason::ProducerError { .. } => "producer_error",
        }
    }

    #[getter]
    fn detail(&self) -> Option<&str> {
        match &self.inner {
            RustEndReason::ProducerError { detail } => Some(detail.as_str()),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            RustEndReason::ProducerError { detail } => {
                format!("EndReason.producer_error(detail={detail:?})")
            }
            RustEndReason::SourceEnded => "EndReason.source_ended()".to_string(),
            RustEndReason::ProducerShuttingDown => {
                "EndReason.producer_shutting_down()".to_string()
            }
            RustEndReason::SessionEnded => "EndReason.session_ended()".to_string(),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── ProducerFrame ───────────────────────────────────────────────────────────

/// What the producer's source-iterator yields. `seq` is stamped by the
/// SDK at send time; producers only set `timestamp_ns` + `payload`.
#[pyclass(name = "ProducerFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyProducerFrame {
    pub(crate) timestamp_ns: i64,
    pub(crate) payload: PyJpegFrame,
}

#[pymethods]
impl PyProducerFrame {
    #[new]
    #[pyo3(signature = (*, timestamp_ns, payload))]
    fn new(timestamp_ns: i64, payload: PyJpegFrame) -> Self {
        Self {
            timestamp_ns,
            payload,
        }
    }

    #[getter]
    fn timestamp_ns(&self) -> i64 {
        self.timestamp_ns
    }

    #[getter]
    fn payload(&self) -> PyJpegFrame {
        self.payload.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "ProducerFrame(timestamp_ns={}, payload={})",
            self.timestamp_ns,
            self.payload.__repr__(),
        )
    }
}

impl PyProducerFrame {
    pub(crate) fn to_rust_jpeg(&self) -> RustProducerFrame<RustJpegFrame> {
        RustProducerFrame {
            timestamp_ns: self.timestamp_ns,
            payload: self.payload.inner.clone(),
        }
    }
}

// ─── ConsumerFrame ───────────────────────────────────────────────────────────

/// What the consumer reads off `StreamSubscription.frames()`. Same as
/// [`PyProducerFrame`] but with the SDK-stamped `seq` exposed.
#[pyclass(name = "ConsumerFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyConsumerFrame {
    timestamp_ns: i64,
    seq: u64,
    payload: PyJpegFrame,
}

#[pymethods]
impl PyConsumerFrame {
    #[getter]
    fn timestamp_ns(&self) -> i64 {
        self.timestamp_ns
    }

    #[getter]
    fn seq(&self) -> u64 {
        self.seq
    }

    #[getter]
    fn payload(&self) -> PyJpegFrame {
        self.payload.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "ConsumerFrame(timestamp_ns={}, seq={}, payload={})",
            self.timestamp_ns,
            self.seq,
            self.payload.__repr__(),
        )
    }
}

impl PyConsumerFrame {
    fn from_rust(frame: RustConsumerFrame<RustJpegFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: PyJpegFrame {
                inner: frame.payload,
            },
        }
    }
}

// ─── StreamDecision ──────────────────────────────────────────────────────────

/// Provider's accept/decline decision. Construct via the static factories
/// `accept(info, source)` / `decline(reason)` — there is no public
/// constructor.
///
/// `source` (on `accept`) is **a Python async iterator yielding
/// [`PyProducerFrame`] values**. Typically an `async def` generator;
/// any object with `__aiter__` / `__anext__` works. The SDK drains it
/// on the wrapper's asyncio loop; `finally` blocks fire when the SDK
/// drops the iterator (consumer disconnect → `aclose` driven through).
#[pyclass(name = "StreamDecision", frozen)]
pub struct PyStreamDecision {
    pub(crate) inner: Mutex<Option<DecisionInner>>,
}

pub(crate) enum DecisionInner {
    Accept {
        info: PyAcceptInfo,
        source: Py<PyAny>,
    },
    Decline {
        reason: PyDeclineReason,
    },
}

#[pymethods]
impl PyStreamDecision {
    #[staticmethod]
    #[pyo3(signature = (*, info, source))]
    fn accept(info: PyAcceptInfo, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::Accept { info, source })),
        }
    }

    #[staticmethod]
    #[pyo3(signature = (reason, /))]
    fn decline(reason: PyDeclineReason) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::Decline { reason })),
        }
    }

    /// Discriminator: `"accept"` or `"decline"`. Read-only inspection;
    /// the actual fields aren't exposed because the source iterator is
    /// consumed by the SDK exactly once.
    #[getter]
    fn kind(&self) -> &'static str {
        let guard = self.inner.lock().expect("PyStreamDecision mutex poisoned");
        match guard.as_ref() {
            Some(DecisionInner::Accept { .. }) => "accept",
            Some(DecisionInner::Decline { .. }) => "decline",
            None => "consumed",
        }
    }

    fn __repr__(&self) -> String {
        format!("StreamDecision.{}()", self.kind())
    }
}

impl PyStreamDecision {
    /// Take the inner decision out of the mutex. Returns `None` if the
    /// SDK already drained it (which should never happen — the wrapper
    /// constructs the decision once per inbound request).
    pub(crate) fn take(&self) -> Option<DecisionInner> {
        self.inner
            .lock()
            .expect("PyStreamDecision mutex poisoned")
            .take()
    }
}

// ─── PyStreamProvider ────────────────────────────────────────────────────────

/// Build a Rust [`StreamProvider`] from a Python callable matching
/// `Callable[[StreamRequest], StreamDecision]`. Used by `cluster.spawn`
/// when the consumer passes `stream_provider=...`.
///
/// **Python surface today is JPEG-only.** Dagaz Batch 1 lifted the Rust
/// [`StreamProvider`] to a closed [`RustStreamDispatch`] enum over the
/// SDK-supported `T`s (`AcceptJpeg`, `AcceptPointCloud`, `Decline`).
/// The Python `StreamDecision` PyClass currently only constructs JPEG
/// sources, so this adapter always returns either
/// `RustStreamDispatch::AcceptJpeg` (on Accept) or
/// `RustStreamDispatch::Decline`. PointCloud support on the Python
/// surface lands in Dagaz Batch 2 (the `auki-network-py` extension for
/// the new `T` shape) — when that lands, the `DecisionInner` enum
/// extends with a `PointCloud` variant and this match grows another
/// arm.
///
/// Behaviour on Python exception / non-`StreamDecision` return:
/// the wrapper logs the offence via `tracing::warn!` and synthesizes a
/// `Decline { reason: Other { detail: <error string> } }` so the
/// requester sees a typed failure rather than a hung substream.
pub(crate) fn build_stream_provider(callable: Py<PyAny>) -> StreamProvider {
    // `RustJpegFrame` is no longer referenced through the generic
    // `StreamProvider<T>` signature; keep it imported because the
    // PyJpegFrame path still touches `RustJpegFrame` lower in this
    // file.
    let _ = std::marker::PhantomData::<RustJpegFrame>;
    Arc::new(move |request: RustStreamRequest| {
        let py_request = PyStreamRequest { inner: request };

        // Step 1 (under GIL): call the Python provider, extract a
        // PyStreamDecision (or normalize errors to a Decline).
        let decision_or_err: Result<DecisionInner, String> = Python::with_gil(|py| {
            let result = match callable.call1(py, (py_request.clone(),)) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "stream_provider raised; declining");
                    return Err(format!("provider raised: {e}"));
                }
            };
            // Bind & extract a PyStreamDecision PyRef so we can call .take().
            let bound = result.bind(py);
            match bound.extract::<PyRef<PyStreamDecision>>() {
                Ok(decision_ref) => match decision_ref.take() {
                    Some(inner) => Ok(inner),
                    None => Err("provider returned an already-consumed StreamDecision".into()),
                },
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "stream_provider returned non-StreamDecision; declining"
                    );
                    Err(format!("provider returned non-StreamDecision: {e}"))
                }
            }
        });

        // Step 2 (no GIL needed for the type-shape match): map onto a
        // Rust StreamDispatch variant. On error, synthesize a Decline
        // carrying the error string. Python surface is JPEG-only today,
        // so Accept maps to AcceptJpeg.
        match decision_or_err {
            Err(detail) => RustStreamDispatch::Decline {
                reason: RustDeclineReason::Other { detail },
            },
            Ok(DecisionInner::Decline { reason }) => RustStreamDispatch::Decline {
                reason: reason.inner,
            },
            Ok(DecisionInner::Accept { info, source }) => {
                let source_stream = python_iter_into_source_stream(source);
                RustStreamDispatch::AcceptJpeg {
                    info: info.inner,
                    source: source_stream,
                }
            }
        }
    })
}

/// Convert a Python async iterator (yielding `PyProducerFrame`) into a
/// Rust [`SourceStream<JpegFrame>`] the SDK can drain.
///
/// Type contract: each yielded Python value must extract as
/// [`PyProducerFrame`]. Anything else maps to
/// `Some(Err("..."))` which the SDK converts into
/// [`auki_network::stream_protocol::EndReason::ProducerError`] on the
/// wire and ends the stream.
///
/// Lifetime / cleanup: the bridge is held inside [`SourceStreamGuard`].
/// On natural end (`StopAsyncIteration` or first error) we explicitly
/// fire `aclose` on the iterator before yielding `None`. On unnatural
/// drop (consumer disconnect mid-stream → SDK drops the `SourceStream`),
/// `Drop` on [`SourceStreamGuard`] schedules a fire-and-forget `aclose`
/// task on the wrapper's tokio runtime so the generator's `finally`
/// block fires promptly rather than waiting for asyncio's gc hooks.
fn python_iter_into_source_stream(aiter: Py<PyAny>) -> SourceStream<RustJpegFrame> {
    let bridge = PyAsyncIterStream::new(aiter);
    let state = SourceStreamGuard {
        bridge: Some(bridge),
    };

    let stream = futures::stream::unfold(state, |mut state| async move {
        let bridge = state.bridge.as_ref()?;
        match bridge.next().await {
            Ok(Some(value)) => {
                // Type-check the yielded item under GIL; convert to
                // ProducerFrame<JpegFrame>.
                let result = Python::with_gil(
                    |py| -> Result<RustProducerFrame<RustJpegFrame>, String> {
                        let bound = value.bind(py);
                        match bound.extract::<PyRef<PyProducerFrame>>() {
                            Ok(pf) => Ok(pf.to_rust_jpeg()),
                            Err(_) => Err(format!(
                                "stream_provider source must yield ProducerFrame; got {}",
                                bound
                                    .repr()
                                    .map(|r| r.to_string())
                                    .unwrap_or_else(|_| "<unrepr>".into())
                            )),
                        }
                    },
                );
                match result {
                    Ok(frame) => Some((Ok(frame), state)),
                    Err(detail) => {
                        if let Some(b) = state.bridge.take() {
                            b.aclose().await;
                        }
                        Some((Err(detail), state))
                    }
                }
            }
            Ok(None) => {
                if let Some(b) = state.bridge.take() {
                    b.aclose().await;
                }
                None
            }
            Err(e) => {
                let detail = Python::with_gil(|py| {
                    let s = e.to_string();
                    e.print_and_set_sys_last_vars(py);
                    s
                });
                if let Some(b) = state.bridge.take() {
                    b.aclose().await;
                }
                Some((Err(detail), state))
            }
        }
    });
    Box::pin(stream)
}

/// Drop guard for the producer-side source-Stream. If the SDK drops the
/// source mid-iteration (consumer disconnected), this fires `aclose` on
/// the Python iterator as a fire-and-forget task on the wrapper's
/// tokio runtime — driving the generator's `finally` block.
///
/// Without this, the Python async generator stays alive in CPython's
/// async-generator GC list until either (a) the asyncio loop is
/// shut down with `loop.shutdown_asyncgens()`, or (b) the interpreter
/// exits. For a long-running daemon (BoosterApp's sidecar) (a) only
/// happens at shutdown, so cleanup would lag arbitrarily.
struct SourceStreamGuard {
    bridge: Option<PyAsyncIterStream>,
}

impl Drop for SourceStreamGuard {
    fn drop(&mut self) {
        if let Some(bridge) = self.bridge.take() {
            // Fire-and-forget aclose on the wrapper's tokio runtime.
            // The bridge is moved in; the spawned task owns it until
            // aclose completes, then drops it. If the runtime is
            // shutting down, the task may not run — but at that point
            // the asyncio loop is also closing, so the cleanup happens
            // through `shutdown_asyncgens` regardless.
            crate::cluster_tokio_runtime().spawn(async move {
                bridge.aclose().await;
            });
        }
    }
}

// ─── StreamSubscription + FrameIterator ──────────────────────────────────────

/// What `runtime.open_stream` returns on a successful Accept. Carries
/// the producer's [`PyAcceptInfo`]; iterate over frames via
/// `subscription.frames()`.
///
/// The frames iterator can be fetched **at most once**. A second call
/// raises `RuntimeError` — the underlying Rust `Stream` is single-use.
#[pyclass(name = "StreamSubscription")]
pub struct PyStreamSubscription {
    info: PyAcceptInfo,
    frames: Mutex<Option<RustFrameStream>>,
}

type RustFrameStream =
    Pin<Box<dyn Stream<Item = Result<RustConsumerFrame<RustJpegFrame>, RustStreamError>> + Send>>;

#[pymethods]
impl PyStreamSubscription {
    /// Accept-time metadata committed by the producer.
    #[getter]
    fn info(&self) -> PyAcceptInfo {
        self.info.clone()
    }

    /// Drain the frame iterator (sync, blocking). Each `__next__()` blocks
    /// until the next frame arrives over the substream. Stream-end
    /// signals raise typed exceptions:
    ///
    /// - `auki_network.cluster.StreamEndOfStream(reason)` — clean end.
    ///   `.args[0]` is an `EndReason`.
    /// - `auki_network.cluster.StreamConnectionLost` — substream closed
    ///   without an explicit `EndOfStream` (peer disconnect).
    /// - `auki_network.cluster.StreamProtocolError(detail)` — peer
    ///   wrote malformed bytes mid-stream.
    ///
    /// After the typed exception, subsequent `__next__()` calls raise
    /// `StopIteration` (the iterator is exhausted).
    fn frames(&self) -> PyResult<PyFrameIterator> {
        let mut guard = self.frames.lock().expect("StreamSubscription mutex poisoned");
        let frames = guard.take().ok_or_else(|| {
            PyRuntimeError::new_err("StreamSubscription.frames() can only be called once")
        })?;
        Ok(PyFrameIterator {
            frames: Mutex::new(Some(frames)),
        })
    }

    fn __repr__(&self) -> String {
        format!("StreamSubscription(info={})", self.info.__repr__())
    }
}

impl PyStreamSubscription {
    pub(crate) fn from_rust(rust_sub: RustStreamSubscription<RustJpegFrame>) -> Self {
        Self {
            info: PyAcceptInfo {
                inner: rust_sub.info,
            },
            frames: Mutex::new(Some(rust_sub.frames)),
        }
    }
}

/// Sync iterator over a [`PyStreamSubscription`]'s frames. Each
/// `__next__()` blocks the caller's thread on the wrapper's tokio
/// runtime until the next frame arrives.
#[pyclass(name = "FrameIterator")]
pub struct PyFrameIterator {
    frames: Mutex<Option<RustFrameStream>>,
}

#[pymethods]
impl PyFrameIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Block until the next frame arrives. See
    /// [`PyStreamSubscription::frames`] docstring for end-of-stream
    /// signalling.
    fn __next__(&self, py: Python<'_>) -> PyResult<PyConsumerFrame> {
        // Pull the stream out of the mutex for the duration of the
        // poll. Releasing the GIL while we block lets other Python
        // threads run (the wrapper's asyncio loop thread, e.g. for a
        // simultaneous `stream_provider` invocation on the same
        // process).
        let stream_taken = {
            let mut guard = self.frames.lock().expect("FrameIterator mutex poisoned");
            guard.take()
        };
        let mut stream = match stream_taken {
            Some(s) => s,
            None => {
                // Already exhausted (saw a terminator on a previous call).
                return Err(PyStopIteration::new_err(()));
            }
        };

        let item = py.allow_threads(|| {
            let rt = crate::cluster_tokio_runtime();
            rt.block_on(async { stream.next().await })
        });

        match item {
            Some(Ok(frame)) => {
                // Put the stream back; iterator is still live.
                let mut guard = self.frames.lock().expect("FrameIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyConsumerFrame::from_rust(frame))
            }
            Some(Err(stream_err)) => {
                // Terminator. Don't put the stream back — exhausted.
                Err(stream_error_to_pyerr(py, stream_err))
            }
            None => Err(PyStopIteration::new_err(())),
        }
    }
}

// ─── Exception types ─────────────────────────────────────────────────────────
//
// Three typed exceptions surface on the consumer side when a stream
// ends. Plus the open-side `StreamDeclined` and `StreamUnreachable` for
// `runtime.open_stream` failures. Modelled as Python exception classes
// via `create_exception!` — Python consumers catch them by name.

create_exception!(
    auki_network,
    StreamEndOfStream,
    pyo3::exceptions::PyException,
    "Iterator terminator: the producer ended the stream cleanly. \
     `args[0]` is an `EndReason`."
);

create_exception!(
    auki_network,
    StreamConnectionLost,
    pyo3::exceptions::PyException,
    "Iterator terminator: the substream closed without an explicit \
     `EndOfStream` (peer disconnect, transport error). Per grimsby D5b \
     — implicit via libp2p disconnect."
);

create_exception!(
    auki_network,
    StreamProtocolError,
    pyo3::exceptions::PyException,
    "Iterator terminator: the producer wrote malformed bytes or a \
     wire-incompatible payload."
);

create_exception!(
    auki_network,
    StreamDeclined,
    pyo3::exceptions::PyException,
    "`runtime.open_stream` failure: the producer accepted the substream \
     open but declined the request. `args[0]` is a `DeclineReason`."
);

create_exception!(
    auki_network,
    StreamUnreachable,
    pyo3::exceptions::PyException,
    "`runtime.open_stream` failure: libp2p couldn't open the substream \
     (peer not reachable, peer doesn't speak `/auki/stream/1.0.0`, or \
     the open timed out). `args[0]` is a description string."
);

fn stream_error_to_pyerr(py: Python<'_>, err: RustStreamError) -> PyErr {
    match err {
        RustStreamError::EndOfStream { reason } => {
            let py_reason = Py::new(py, PyEndReason { inner: reason })
                .expect("create EndReason for terminator");
            StreamEndOfStream::new_err((py_reason,))
        }
        RustStreamError::ConnectionLost => StreamConnectionLost::new_err(()),
        RustStreamError::Protocol(e) => StreamProtocolError::new_err(format!("{e}")),
    }
}

pub(crate) fn open_stream_error_to_pyerr(py: Python<'_>, err: RustOpenStreamError) -> PyErr {
    match err {
        RustOpenStreamError::Declined { reason } => {
            let py_reason = Py::new(py, PyDeclineReason { inner: reason })
                .expect("create DeclineReason for declined open");
            StreamDeclined::new_err((py_reason,))
        }
        RustOpenStreamError::LibP2p(e) => StreamUnreachable::new_err(format!("libp2p: {e}")),
        RustOpenStreamError::Protocol(e) => StreamProtocolError::new_err(format!("{e}")),
        RustOpenStreamError::Timeout(d) => {
            StreamUnreachable::new_err(format!("open timed out after {d:?}"))
        }
    }
}

// ─── Module registration ─────────────────────────────────────────────────────

pub(crate) fn register(py: Python<'_>, cluster: &Bound<'_, PyModule>) -> PyResult<()> {
    cluster.add_class::<PyStreamRequest>()?;
    cluster.add_class::<PyAcceptInfo>()?;
    cluster.add_class::<PyJpegFrame>()?;
    cluster.add_class::<PyDeclineReason>()?;
    cluster.add_class::<PyEndReason>()?;
    cluster.add_class::<PyProducerFrame>()?;
    cluster.add_class::<PyConsumerFrame>()?;
    cluster.add_class::<PyStreamDecision>()?;
    cluster.add_class::<PyStreamSubscription>()?;
    cluster.add_class::<PyFrameIterator>()?;

    cluster.add("StreamEndOfStream", py.get_type_bound::<StreamEndOfStream>())?;
    cluster.add(
        "StreamConnectionLost",
        py.get_type_bound::<StreamConnectionLost>(),
    )?;
    cluster.add(
        "StreamProtocolError",
        py.get_type_bound::<StreamProtocolError>(),
    )?;
    cluster.add("StreamDeclined", py.get_type_bound::<StreamDeclined>())?;
    cluster.add("StreamUnreachable", py.get_type_bound::<StreamUnreachable>())?;

    Ok(())
}

// `PyValueError` re-export so lib.rs can construct it without needing
// to import the same exception type name twice. Used by lib.rs's
// `open_stream` arg validation.
#[allow(dead_code)]
pub(crate) fn invalid_arg<S: Into<String>>(msg: S) -> PyErr {
    PyValueError::new_err(msg.into())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use auki_network_rs::stream_runtime::OPEN_STREAM_TIMEOUT;

    #[test]
    fn stream_request_round_trips() {
        Python::with_gil(|_py| {
            let r = PyStreamRequest::new("K1-AABB/head_left_cam".into());
            assert_eq!(r.sensor_id(), "K1-AABB/head_left_cam");
            assert_eq!(
                r.__repr__(),
                r#"StreamRequest(sensor_id="K1-AABB/head_left_cam")"#,
            );
        });
    }

    #[test]
    fn accept_info_round_trips_and_compares() {
        Python::with_gil(|_py| {
            let a = PyAcceptInfo::new("h".into(), "c".into(), "ch".into());
            let b = PyAcceptInfo::new("h".into(), "c".into(), "ch".into());
            assert_eq!(a.sensor_hash(), "h");
            assert_eq!(a.clock_id(), "c");
            assert_eq!(a.clock_hash(), "ch");
            assert!(a.__eq__(&b));
            let c = PyAcceptInfo::new("other".into(), "c".into(), "ch".into());
            assert!(!a.__eq__(&c));
        });
    }

    #[test]
    fn jpeg_frame_round_trips_through_pybytes() {
        Python::with_gil(|py| {
            let payload = PyBytes::new_bound(py, &[0xff, 0xd8, 0x01, 0x02, 0x03]);
            let f = PyJpegFrame::new(payload);
            assert_eq!(f.__len__(), 5);
            // Round-trip the bytes back out.
            let out = f.bytes(py);
            assert_eq!(out.as_bytes(), &[0xff, 0xd8, 0x01, 0x02, 0x03]);
        });
    }

    #[test]
    fn decline_reason_factories_carry_kind_and_detail() {
        Python::with_gil(|_py| {
            let nf = PyDeclineReason::sensor_not_found();
            assert_eq!(nf.kind(), "sensor_not_found");
            assert_eq!(nf.detail(), None);

            let una = PyDeclineReason::sensor_unavailable();
            assert_eq!(una.kind(), "sensor_unavailable");

            let psd = PyDeclineReason::producer_shutting_down();
            assert_eq!(psd.kind(), "producer_shutting_down");

            let other = PyDeclineReason::other("custom".into());
            assert_eq!(other.kind(), "other");
            assert_eq!(other.detail(), Some("custom"));

            // Equality tracks the inner Rust enum.
            assert!(nf.__eq__(&PyDeclineReason::sensor_not_found()));
            assert!(!nf.__eq__(&una));
        });
    }

    #[test]
    fn end_reason_factories_carry_kind_and_detail() {
        Python::with_gil(|_py| {
            assert_eq!(PyEndReason::source_ended().kind(), "source_ended");
            assert_eq!(
                PyEndReason::producer_shutting_down().kind(),
                "producer_shutting_down",
            );
            assert_eq!(PyEndReason::session_ended().kind(), "session_ended");
            let perr = PyEndReason::producer_error("encoder died".into());
            assert_eq!(perr.kind(), "producer_error");
            assert_eq!(perr.detail(), Some("encoder died"));
        });
    }

    #[test]
    fn producer_frame_extracts_to_rust_jpeg() {
        Python::with_gil(|py| {
            let payload = PyJpegFrame::new(PyBytes::new_bound(py, &[1, 2, 3]));
            let pf = PyProducerFrame::new(123_456_789, payload);
            let rust = pf.to_rust_jpeg();
            assert_eq!(rust.timestamp_ns, 123_456_789);
            assert_eq!(rust.payload.bytes, vec![1, 2, 3]);
        });
    }

    #[test]
    fn consumer_frame_constructs_from_rust() {
        Python::with_gil(|_py| {
            let rust_frame = RustConsumerFrame {
                timestamp_ns: 9_999,
                seq: 17,
                payload: RustJpegFrame {
                    bytes: vec![0xff, 0xd8, 0xee],
                },
            };
            let pf = PyConsumerFrame::from_rust(rust_frame);
            assert_eq!(pf.timestamp_ns(), 9_999);
            assert_eq!(pf.seq(), 17);
            assert_eq!(pf.payload().__len__(), 3);
        });
    }

    #[test]
    fn stream_decision_factories_tag_correctly() {
        Python::with_gil(|py| {
            // Construct a Python object to stand in for the source iterator
            // (a None object is fine — we only inspect .kind, never drain).
            let placeholder = py.None();
            let info = PyAcceptInfo::new("h".into(), "c".into(), "ch".into());
            let acc = PyStreamDecision::accept(info, placeholder);
            assert_eq!(acc.kind(), "accept");

            let dec = PyStreamDecision::decline(PyDeclineReason::sensor_not_found());
            assert_eq!(dec.kind(), "decline");

            // After taking, the decision reports `consumed`.
            let _taken = acc.take();
            assert_eq!(acc.kind(), "consumed");
        });
    }

    /// `build_stream_provider` over a Python callable that returns a
    /// `Decline(SensorNotFound)` should produce a Rust provider that
    /// hands back the same typed decline. Exercises the Python →
    /// Rust ↔ Rust round-trip through `Arc<dyn Fn>`.
    #[test]
    fn build_stream_provider_decline_round_trip() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            // Build a Python callable: `lambda req: cluster.StreamDecision.decline(...)`.
            // We expose the wrapper module so the lambda can construct
            // wrapped types.
            let module = PyModule::new_bound(py, "test_provider_module").unwrap();
            crate::populate_module(&module).unwrap();
            let cluster = module.getattr("cluster").unwrap();

            // Stash cluster in __main__ so the lambda can resolve it.
            py.run_bound(
                r#"
import sys
def _make(cluster):
    def provider(req):
        return cluster.StreamDecision.decline(cluster.DeclineReason.sensor_not_found())
    return provider
"#,
                None,
                None,
            )
            .unwrap();
            let make = py.eval_bound("_make", None, None).unwrap();
            let provider = make.call1((&cluster,)).unwrap();

            let rust_provider = build_stream_provider(provider.unbind());
            let request = RustStreamRequest {
                sensor_id: "any".into(),
            };
            match rust_provider(request) {
                RustStreamDispatch::Decline {
                    reason: RustDeclineReason::SensorNotFound,
                } => {}
                _ => panic!("expected Decline(SensorNotFound)"),
            }
        });
    }

    /// A provider that raises is caught by the wrapper and surfaced as
    /// `Decline(Other { detail })`. The surface promise is no panic, no
    /// hang — the requester sees a typed decline instead.
    #[test]
    fn build_stream_provider_raising_collapses_to_decline_other() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "test_provider_raise").unwrap();
            crate::populate_module(&module).unwrap();

            py.run_bound(
                r#"
def _bad(req):
    raise RuntimeError("provider broke")
"#,
                None,
                None,
            )
            .unwrap();
            let bad = py.eval_bound("_bad", None, None).unwrap();
            let rust_provider = build_stream_provider(bad.unbind());
            let request = RustStreamRequest {
                sensor_id: "any".into(),
            };
            match rust_provider(request) {
                RustStreamDispatch::Decline {
                    reason: RustDeclineReason::Other { detail },
                } => assert!(
                    detail.contains("provider broke"),
                    "decline detail should carry the Python error: {detail}",
                ),
                _ => panic!("expected Decline(Other) with the Python error in detail"),
            }
        });
    }

    /// `OPEN_STREAM_TIMEOUT` is re-exported from `auki_network_rs` and
    /// shouldn't have changed unexpectedly. Sanity check the Python side
    /// stays in sync if a future SDK release tightens it.
    #[test]
    fn open_stream_timeout_matches_sdk() {
        assert_eq!(
            OPEN_STREAM_TIMEOUT,
            std::time::Duration::from_secs(30),
            "if SDK changes OPEN_STREAM_TIMEOUT, update wrapper docs",
        );
    }
}
