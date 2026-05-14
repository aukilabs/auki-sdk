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
    AudioFrame as RustAudioFrame,
    AcceptInfo as RustAcceptInfo, DeclineReason as RustDeclineReason,
    EndReason as RustEndReason, JointEncodersFrame as RustJointEncodersFrame,
    JpegFrame as RustJpegFrame, PointCloudFrame as RustPointCloudFrame,
    StreamRequest as RustStreamRequest, decline_reason, end_reason,
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
use pyo3::types::{PyBytes, PyCapsule, PyModule};
use std::ffi::CString;
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

// ─── PointCloudFrame ─────────────────────────────────────────────────────────

/// Dagaz Batch 1 payload `T` — raw CDR-encoded `PointCloud2` ROS message
/// bytes (per [Dagaz](https://www.notion.so/3585c8e96592805b8d83c89f849d3577) D2).
/// Consumer (Park, future Sentinel) parses CDR on its side; the SDK doesn't
/// decode or interpret these bytes. Same shape as [`PyJpegFrame`] —
/// opaque-bytes-with-a-`bytes`-property — but the wire envelope uses a
/// base64 adapter so a 22 MB/s raw stream lands at ~30 MB/s on the wire
/// instead of ~80 MB/s (grimsby's JSON-of-binary tax dodge, per Dagaz D2).
#[pyclass(name = "PointCloudFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyPointCloudFrame {
    pub(crate) inner: RustPointCloudFrame,
}

#[pymethods]
impl PyPointCloudFrame {
    #[new]
    #[pyo3(signature = (bytes, /))]
    fn new(bytes: Bound<'_, PyBytes>) -> Self {
        Self {
            inner: RustPointCloudFrame {
                bytes: bytes.as_bytes().to_vec(),
            },
        }
    }

    /// Raw CDR-encoded `PointCloud2` bytes. Returns a fresh `bytes` copy
    /// each call.
    #[getter]
    fn bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.bytes)
    }

    fn __len__(&self) -> usize {
        self.inner.bytes.len()
    }

    fn __repr__(&self) -> String {
        format!("PointCloudFrame(<{} bytes>)", self.inner.bytes.len())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── JointEncodersFrame ──────────────────────────────────────────────────────

/// sawslin Phase B payload `T` — one joint-angle sample, `repeated float
/// angles_rad` indexed in the producer's emit order. Length is pinned by
/// the registry entry's `JointEncoders { joint_count }` body and the
/// consumer enforces it on its side. Wire bytes are byte-identical to
/// the on-disk `auki.joint_encoders.JointEncodersLogEntry` payload by
/// design (locked in `auki-datatypes` by the
/// `joint_encoders_disk_wire_byte_identical` test).
///
/// Differs from [`PyJpegFrame`] / [`PyPointCloudFrame`] in payload
/// shape: a `list[float]` of joint angles, not opaque `bytes`. The
/// underlying prost type is the same `repeated float angles_rad` Vec
/// you'd encode by hand if there were no Python binding.
#[pyclass(name = "JointEncodersFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyJointEncodersFrame {
    pub(crate) inner: RustJointEncodersFrame,
}

#[pymethods]
impl PyJointEncodersFrame {
    #[new]
    #[pyo3(signature = (angles_rad, /))]
    fn new(angles_rad: Vec<f32>) -> Self {
        Self {
            inner: RustJointEncodersFrame { angles_rad },
        }
    }

    /// Joint angle readings in radians, in the producer's emit order.
    /// Returns a fresh list each call.
    #[getter]
    fn angles_rad(&self) -> Vec<f32> {
        self.inner.angles_rad.clone()
    }

    fn __len__(&self) -> usize {
        self.inner.angles_rad.len()
    }

    fn __repr__(&self) -> String {
        format!("JointEncodersFrame(<{} joints>)", self.inner.angles_rad.len())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── AudioFrame ──────────────────────────────────────────────────────────────

/// Dialogue Batch 1 payload `T` — interleaved PCM audio bytes. Per-chunk
/// payload mirroring `auki.audio.AudioLogEntry` (same `bytes data = 1`
/// proto field, byte-identical wire/disk by design — locked in
/// `auki-datatypes` by `audio_disk_wire_byte_identical`). Opaque-bytes:
/// `sample_format` / `channels` / `sample_rate_hz` / `channel_layout`
/// resolution comes from `(sensor_id, sensor_hash) → SensorBody::Audio`
/// at handshake. Same opaque-`bytes`-property shape as [`PyJpegFrame`] /
/// [`PyPointCloudFrame`], but the Python getter is named `.data` to
/// match the proto field name (the proto says `bytes data`, not
/// `bytes bytes`).
#[pyclass(name = "AudioFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyAudioFrame {
    pub(crate) inner: RustAudioFrame,
}

#[pymethods]
impl PyAudioFrame {
    #[new]
    #[pyo3(signature = (data, /))]
    fn new(data: Bound<'_, PyBytes>) -> Self {
        Self {
            inner: RustAudioFrame {
                data: data.as_bytes().to_vec(),
            },
        }
    }

    /// Raw interleaved PCM bytes. Returns a fresh `bytes` copy each
    /// call. Layout (sample format, channel count, sample rate) comes
    /// from the registry entry pinned by `sensor_hash` at handshake;
    /// the wire payload is opaque.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.data)
    }

    fn __len__(&self) -> usize {
        self.inner.data.len()
    }

    fn __repr__(&self) -> String {
        format!("AudioFrame(<{} bytes>)", self.inner.data.len())
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
            inner: RustDeclineReason::sensor_not_found(),
        }
    }

    #[staticmethod]
    fn sensor_unavailable() -> Self {
        Self {
            inner: RustDeclineReason::sensor_unavailable(),
        }
    }

    #[staticmethod]
    fn producer_shutting_down() -> Self {
        Self {
            inner: RustDeclineReason::producer_shutting_down(),
        }
    }

    #[staticmethod]
    #[pyo3(signature = (*, detail))]
    fn other(detail: String) -> Self {
        Self {
            inner: RustDeclineReason::other(detail),
        }
    }

    /// snake-case discriminator: `"sensor_not_found"`, `"sensor_unavailable"`,
    /// `"producer_shutting_down"`, or `"other"`. Stable across SDK versions
    /// (the prost-generated oneof tag, snake-cased per the `.proto`).
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner.kind {
            Some(decline_reason::Kind::SensorNotFound(_)) => "sensor_not_found",
            Some(decline_reason::Kind::SensorUnavailable(_)) => "sensor_unavailable",
            Some(decline_reason::Kind::ProducerShuttingDown(_)) => "producer_shutting_down",
            Some(decline_reason::Kind::Other(_)) => "other",
            None => "unspecified",
        }
    }

    /// Free-form detail string. Populated only for the `other` variant;
    /// `None` for the named variants.
    #[getter]
    fn detail(&self) -> Option<&str> {
        match &self.inner.kind {
            Some(decline_reason::Kind::Other(other)) => Some(other.detail.as_str()),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner.kind {
            Some(decline_reason::Kind::Other(other)) => {
                format!("DeclineReason.other(detail={:?})", other.detail)
            }
            Some(decline_reason::Kind::SensorNotFound(_)) => {
                "DeclineReason.sensor_not_found()".to_string()
            }
            Some(decline_reason::Kind::SensorUnavailable(_)) => {
                "DeclineReason.sensor_unavailable()".to_string()
            }
            Some(decline_reason::Kind::ProducerShuttingDown(_)) => {
                "DeclineReason.producer_shutting_down()".to_string()
            }
            None => "DeclineReason.<unspecified>()".to_string(),
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
            inner: RustEndReason::source_ended(),
        }
    }

    #[staticmethod]
    fn producer_shutting_down() -> Self {
        Self {
            inner: RustEndReason::producer_shutting_down(),
        }
    }

    #[staticmethod]
    fn session_ended() -> Self {
        Self {
            inner: RustEndReason::session_ended(),
        }
    }

    #[staticmethod]
    #[pyo3(signature = (*, detail))]
    fn producer_error(detail: String) -> Self {
        Self {
            inner: RustEndReason::producer_error(detail),
        }
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner.kind {
            Some(end_reason::Kind::SourceEnded(_)) => "source_ended",
            Some(end_reason::Kind::ProducerShuttingDown(_)) => "producer_shutting_down",
            Some(end_reason::Kind::SessionEnded(_)) => "session_ended",
            Some(end_reason::Kind::ProducerError(_)) => "producer_error",
            None => "unspecified",
        }
    }

    #[getter]
    fn detail(&self) -> Option<&str> {
        match &self.inner.kind {
            Some(end_reason::Kind::ProducerError(err)) => Some(err.detail.as_str()),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner.kind {
            Some(end_reason::Kind::ProducerError(err)) => {
                format!("EndReason.producer_error(detail={:?})", err.detail)
            }
            Some(end_reason::Kind::SourceEnded(_)) => "EndReason.source_ended()".to_string(),
            Some(end_reason::Kind::ProducerShuttingDown(_)) => {
                "EndReason.producer_shutting_down()".to_string()
            }
            Some(end_reason::Kind::SessionEnded(_)) => "EndReason.session_ended()".to_string(),
            None => "EndReason.<unspecified>()".to_string(),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── Frame payload (Jpeg vs PointCloud vs JointEncoders) ─────────────────────

/// Tagged union over the payload `T`s the SDK currently supports.
/// The producer's [`PyProducerFrame`] and the consumer's [`PyConsumerFrame`]
/// both carry one of these; the wire-side substream is mono-`T` per the
/// matching [`RustStreamDispatch`] variant.
#[derive(Clone, Debug)]
pub(crate) enum FramePayload {
    Jpeg(PyJpegFrame),
    PointCloud(PyPointCloudFrame),
    JointEncoders(PyJointEncodersFrame),
    Audio(PyAudioFrame),
}

impl FramePayload {
    fn from_py(payload: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(jpeg) = payload.extract::<PyJpegFrame>() {
            return Ok(Self::Jpeg(jpeg));
        }
        if let Ok(pc) = payload.extract::<PyPointCloudFrame>() {
            return Ok(Self::PointCloud(pc));
        }
        if let Ok(je) = payload.extract::<PyJointEncodersFrame>() {
            return Ok(Self::JointEncoders(je));
        }
        if let Ok(a) = payload.extract::<PyAudioFrame>() {
            return Ok(Self::Audio(a));
        }
        Err(PyValueError::new_err(format!(
            "frame payload must be a JpegFrame, PointCloudFrame, JointEncodersFrame, or AudioFrame; got {}",
            payload
                .repr()
                .map(|r| r.to_string())
                .unwrap_or_else(|_| "<unrepr>".into()),
        )))
    }

    fn into_py(self, py: Python<'_>) -> PyObject {
        match self {
            Self::Jpeg(f) => Py::new(py, f).expect("alloc JpegFrame").into_py(py),
            Self::PointCloud(f) => Py::new(py, f).expect("alloc PointCloudFrame").into_py(py),
            Self::JointEncoders(f) => Py::new(py, f)
                .expect("alloc JointEncodersFrame")
                .into_py(py),
            Self::Audio(f) => Py::new(py, f).expect("alloc AudioFrame").into_py(py),
        }
    }

    fn repr(&self) -> String {
        match self {
            Self::Jpeg(f) => f.__repr__(),
            Self::PointCloud(f) => f.__repr__(),
            Self::JointEncoders(f) => f.__repr__(),
            Self::Audio(f) => f.__repr__(),
        }
    }
}

// ─── ProducerFrame ───────────────────────────────────────────────────────────

/// What the producer's source-iterator yields. `seq` is stamped by the
/// SDK at send time; producers only set `timestamp_ns` + `payload`.
///
/// `payload` accepts either a [`PyJpegFrame`] or a [`PyPointCloudFrame`]
/// (Dagaz Batch 2). The SDK type-checks the payload against the matching
/// [`PyStreamDecision`] accept variant when draining the source iterator
/// — yielding a `JpegFrame` from an `accept_pointcloud(...)` source ends
/// the substream with `EndReason::ProducerError`.
#[pyclass(name = "ProducerFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyProducerFrame {
    pub(crate) timestamp_ns: i64,
    pub(crate) payload: FramePayload,
}

#[pymethods]
impl PyProducerFrame {
    #[new]
    #[pyo3(signature = (*, timestamp_ns, payload))]
    fn new(timestamp_ns: i64, payload: Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            timestamp_ns,
            payload: FramePayload::from_py(&payload)?,
        })
    }

    #[getter]
    fn timestamp_ns(&self) -> i64 {
        self.timestamp_ns
    }

    /// The wrapped frame payload — either a `JpegFrame` or a
    /// `PointCloudFrame`. Returns a fresh Python object each call.
    #[getter]
    fn payload(&self, py: Python<'_>) -> PyObject {
        self.payload.clone().into_py(py)
    }

    fn __repr__(&self) -> String {
        format!(
            "ProducerFrame(timestamp_ns={}, payload={})",
            self.timestamp_ns,
            self.payload.repr(),
        )
    }
}

impl PyProducerFrame {
    /// Convert to a `RustProducerFrame<RustJpegFrame>`. Errors with a
    /// human-readable detail if the payload is `PointCloud`. Used by
    /// the producer-side source-stream pump for an `AcceptJpeg`
    /// dispatch.
    pub(crate) fn to_rust_jpeg(&self) -> Result<RustProducerFrame<RustJpegFrame>, String> {
        match &self.payload {
            FramePayload::Jpeg(f) => Ok(RustProducerFrame {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptJpeg source yielded a ProducerFrame with {} payload; \
                 the substream is mono-T — yield JpegFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }

    /// Convert to a `RustProducerFrame<RustPointCloudFrame>`. Errors
    /// with a human-readable detail if the payload is the wrong variant.
    pub(crate) fn to_rust_pointcloud(
        &self,
    ) -> Result<RustProducerFrame<RustPointCloudFrame>, String> {
        match &self.payload {
            FramePayload::PointCloud(f) => Ok(RustProducerFrame {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptPointCloud source yielded a ProducerFrame with {} payload; \
                 the substream is mono-T — yield PointCloudFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }

    /// Convert to a `RustProducerFrame<RustJointEncodersFrame>`. Errors
    /// with a human-readable detail if the payload is the wrong variant.
    pub(crate) fn to_rust_joint_encoders(
        &self,
    ) -> Result<RustProducerFrame<RustJointEncodersFrame>, String> {
        match &self.payload {
            FramePayload::JointEncoders(f) => Ok(RustProducerFrame {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptJointEncoders source yielded a ProducerFrame with {} payload; \
                 the substream is mono-T — yield JointEncodersFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }

    /// Convert to a `RustProducerFrame<RustAudioFrame>`. Errors with a
    /// human-readable detail if the payload is the wrong variant. Used
    /// by the producer-side source-stream pump for an `AcceptAudio`
    /// dispatch (Dialogue Batch 1).
    pub(crate) fn to_rust_audio(&self) -> Result<RustProducerFrame<RustAudioFrame>, String> {
        match &self.payload {
            FramePayload::Audio(f) => Ok(RustProducerFrame {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptAudio source yielded a ProducerFrame with {} payload; \
                 the substream is mono-T — yield AudioFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }
}

impl FramePayload {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Jpeg(_) => "JpegFrame",
            Self::PointCloud(_) => "PointCloudFrame",
            Self::JointEncoders(_) => "JointEncodersFrame",
            Self::Audio(_) => "AudioFrame",
        }
    }
}

// ─── ConsumerFrame ───────────────────────────────────────────────────────────

/// What the consumer reads off `StreamSubscription.frames()`. Same as
/// [`PyProducerFrame`] but with the SDK-stamped `seq` exposed.
///
/// `payload` is whichever `T` the producer accepted with — `JpegFrame`
/// for an `accept(...)` substream or `PointCloudFrame` for an
/// `accept_pointcloud(...)` substream. Each substream is mono-`T`, so a
/// given `StreamSubscription` only ever surfaces one payload variant.
#[pyclass(name = "ConsumerFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyConsumerFrame {
    timestamp_ns: i64,
    seq: u64,
    payload: FramePayload,
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

    /// The wrapped frame payload — either a `JpegFrame` or a
    /// `PointCloudFrame`. Returns a fresh Python object each call.
    #[getter]
    fn payload(&self, py: Python<'_>) -> PyObject {
        self.payload.clone().into_py(py)
    }

    fn __repr__(&self) -> String {
        format!(
            "ConsumerFrame(timestamp_ns={}, seq={}, payload={})",
            self.timestamp_ns,
            self.seq,
            self.payload.repr(),
        )
    }
}

impl PyConsumerFrame {
    fn from_rust_jpeg(frame: RustConsumerFrame<RustJpegFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: FramePayload::Jpeg(PyJpegFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_pointcloud(frame: RustConsumerFrame<RustPointCloudFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: FramePayload::PointCloud(PyPointCloudFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_joint_encoders(frame: RustConsumerFrame<RustJointEncodersFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: FramePayload::JointEncoders(PyJointEncodersFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_audio(frame: RustConsumerFrame<RustAudioFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: FramePayload::Audio(PyAudioFrame {
                inner: frame.payload,
            }),
        }
    }
}

// ─── StreamDecision ──────────────────────────────────────────────────────────

/// Provider's accept/decline decision. Construct via the static
/// factories `accept(info, source)` (JPEG substream) /
/// `accept_pointcloud(info, source)` (PointCloud substream — Dagaz Batch
/// 2) / `decline(reason)` — there is no public constructor.
///
/// `source` is **a Python async iterator yielding [`PyProducerFrame`]
/// values**. Typically an `async def` generator; any object with
/// `__aiter__` / `__anext__` works. The SDK drains it on the wrapper's
/// asyncio loop; `finally` blocks fire when the SDK drops the iterator
/// (consumer disconnect → `aclose` driven through).
///
/// **Each substream is mono-`T`.** The `accept` factory commits to a
/// `JpegFrame` substream — yielding a `PointCloudFrame` ends the stream
/// with `EndReason::ProducerError`. Use `accept_pointcloud` for a
/// `PointCloudFrame` substream.
#[pyclass(name = "StreamDecision", frozen)]
pub struct PyStreamDecision {
    pub(crate) inner: Mutex<Option<DecisionInner>>,
}

pub(crate) enum DecisionInner {
    AcceptJpeg {
        info: PyAcceptInfo,
        source: Py<PyAny>,
    },
    AcceptPointCloud {
        info: PyAcceptInfo,
        source: Py<PyAny>,
    },
    AcceptJointEncoders {
        info: PyAcceptInfo,
        source: Py<PyAny>,
    },
    AcceptAudio {
        info: PyAcceptInfo,
        source: Py<PyAny>,
    },
    Decline {
        reason: PyDeclineReason,
    },
}

#[pymethods]
impl PyStreamDecision {
    /// Accept the request with a JPEG source. The async iterator must
    /// yield `ProducerFrame(payload=JpegFrame(...))` values; yielding a
    /// `PointCloudFrame` ends the stream with `EndReason::ProducerError`.
    #[staticmethod]
    #[pyo3(signature = (*, info, source))]
    fn accept(info: PyAcceptInfo, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptJpeg { info, source })),
        }
    }

    /// Accept the request with a PointCloud source (Dagaz Batch 2). The
    /// async iterator must yield `ProducerFrame(payload=PointCloudFrame(...))`
    /// values carrying CDR-encoded `PointCloud2` ROS message bytes; the
    /// consumer (Park, future Sentinel) parses CDR on its side.
    #[staticmethod]
    #[pyo3(signature = (*, info, source))]
    fn accept_pointcloud(info: PyAcceptInfo, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptPointCloud { info, source })),
        }
    }

    /// Accept the request with a JointEncoders source (sawslin Phase B).
    /// The async iterator must yield
    /// `ProducerFrame(payload=JointEncodersFrame(angles_rad))` values;
    /// each `angles_rad` length must match the registry entry's
    /// `JointEncoders { joint_count }` (consumer-enforced; the SDK
    /// doesn't validate length on the wire).
    #[staticmethod]
    #[pyo3(signature = (*, info, source))]
    fn accept_joint_encoders(info: PyAcceptInfo, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptJointEncoders { info, source })),
        }
    }

    /// Accept the request with an Audio source (Dialogue Batch 1). The
    /// async iterator must yield
    /// `ProducerFrame(payload=AudioFrame(data))` values carrying
    /// interleaved PCM bytes; sample format / channels / sample rate /
    /// channel layout are resolved out-of-band via
    /// `(sensor_id, sensor_hash) → SensorBody::Audio`, so the wire
    /// payload itself is opaque-bytes.
    #[staticmethod]
    #[pyo3(signature = (*, info, source))]
    fn accept_audio(info: PyAcceptInfo, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptAudio { info, source })),
        }
    }

    #[staticmethod]
    #[pyo3(signature = (reason, /))]
    fn decline(reason: PyDeclineReason) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::Decline { reason })),
        }
    }

    /// Discriminator: `"accept"` (JPEG), `"accept_pointcloud"`,
    /// `"accept_joint_encoders"`, `"accept_audio"`, `"decline"`, or
    /// `"consumed"` (post-`take`). Read-only inspection; the actual
    /// fields aren't exposed because the source iterator is consumed
    /// by the SDK exactly once.
    #[getter]
    fn kind(&self) -> &'static str {
        let guard = self.inner.lock().expect("PyStreamDecision mutex poisoned");
        match guard.as_ref() {
            Some(DecisionInner::AcceptJpeg { .. }) => "accept",
            Some(DecisionInner::AcceptPointCloud { .. }) => "accept_pointcloud",
            Some(DecisionInner::AcceptJointEncoders { .. }) => "accept_joint_encoders",
            Some(DecisionInner::AcceptAudio { .. }) => "accept_audio",
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
/// Maps the Python [`PyStreamDecision`]'s [`DecisionInner`] variants
/// (`AcceptJpeg`, `AcceptPointCloud`, `Decline`) onto the matching
/// Rust [`RustStreamDispatch`] variant. Each substream is mono-`T`;
/// the `T` is decided here by which factory the Python provider used
/// (`accept` → `AcceptJpeg`, `accept_pointcloud` → `AcceptPointCloud`).
///
/// Behaviour on Python exception / non-`StreamDecision` return:
/// the wrapper logs the offence via `tracing::warn!` and synthesizes a
/// `Decline { reason: Other { detail: <error string> } }` so the
/// requester sees a typed failure rather than a hung substream.
///
/// Visibility: `pub` (was `pub(crate)`) so other in-workspace PyO3
/// wrapper crates can reuse it. Notably [`auki-domain-py`](../../auki-domain-py)
/// wires its Python `stream_provider` kwarg through this function in
/// its `init_domain` / `init_or_join_domain` entry points; without
/// reaching this adapter it would have to re-implement ~500 lines of
/// `PyStreamDecision` / `PyAcceptInfo` / `PyDeclineReason` pyclass
/// plumbing. Promoted 2026-05-13.
pub fn build_stream_provider(callable: Py<PyAny>) -> StreamProvider {
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
        // carrying the error string.
        match decision_or_err {
            Err(detail) => RustStreamDispatch::Decline {
                reason: RustDeclineReason::other(detail),
            },
            Ok(DecisionInner::Decline { reason }) => RustStreamDispatch::Decline {
                reason: reason.inner,
            },
            Ok(DecisionInner::AcceptJpeg { info, source }) => {
                let source_stream =
                    python_iter_into_source_stream::<RustJpegFrame>(source, |pf| {
                        pf.to_rust_jpeg()
                    });
                RustStreamDispatch::AcceptJpeg {
                    info: info.inner,
                    source: source_stream,
                }
            }
            Ok(DecisionInner::AcceptPointCloud { info, source }) => {
                let source_stream =
                    python_iter_into_source_stream::<RustPointCloudFrame>(source, |pf| {
                        pf.to_rust_pointcloud()
                    });
                RustStreamDispatch::AcceptPointCloud {
                    info: info.inner,
                    source: source_stream,
                }
            }
            Ok(DecisionInner::AcceptJointEncoders { info, source }) => {
                let source_stream =
                    python_iter_into_source_stream::<RustJointEncodersFrame>(source, |pf| {
                        pf.to_rust_joint_encoders()
                    });
                RustStreamDispatch::AcceptJointEncoders {
                    info: info.inner,
                    source: source_stream,
                }
            }
            Ok(DecisionInner::AcceptAudio { info, source }) => {
                let source_stream =
                    python_iter_into_source_stream::<RustAudioFrame>(source, |pf| {
                        pf.to_rust_audio()
                    });
                RustStreamDispatch::AcceptAudio {
                    info: info.inner,
                    source: source_stream,
                }
            }
        }
    })
}

/// Convert a Python async iterator (yielding `PyProducerFrame`) into a
/// Rust [`SourceStream<T>`] the SDK can drain. The `convert` callback
/// extracts the per-substream typed payload from each yielded
/// [`PyProducerFrame`] — `JpegFrame` for an `AcceptJpeg` dispatch,
/// `PointCloudFrame` for `AcceptPointCloud`. Yielding a frame with the
/// wrong payload variant produces `Some(Err("..."))`, which the SDK
/// converts into [`auki_network::stream_protocol::EndReason::ProducerError`]
/// on the wire and ends the stream.
///
/// Type contract: each yielded Python value must extract as
/// [`PyProducerFrame`]. Anything else maps to `Some(Err("..."))` with
/// the same end-of-stream effect.
///
/// Lifetime / cleanup: the bridge is held inside [`SourceStreamGuard`].
/// On natural end (`StopAsyncIteration` or first error) we explicitly
/// fire `aclose` on the iterator before yielding `None`. On unnatural
/// drop (consumer disconnect mid-stream → SDK drops the `SourceStream`),
/// `Drop` on [`SourceStreamGuard`] schedules a fire-and-forget `aclose`
/// task on the wrapper's tokio runtime so the generator's `finally`
/// block fires promptly rather than waiting for asyncio's gc hooks.
fn python_iter_into_source_stream<T>(
    aiter: Py<PyAny>,
    convert: fn(&PyProducerFrame) -> Result<RustProducerFrame<T>, String>,
) -> SourceStream<T>
where
    T: Send + 'static,
{
    let bridge = PyAsyncIterStream::new(aiter);
    let state = SourceStreamGuard {
        bridge: Some(bridge),
    };

    let stream = futures::stream::unfold(state, move |mut state| async move {
        let bridge = state.bridge.as_ref()?;
        match bridge.next().await {
            Ok(Some(value)) => {
                // Type-check the yielded item under GIL; convert to
                // ProducerFrame<T> using the substream-typed `convert`.
                let result = Python::with_gil(|py| -> Result<RustProducerFrame<T>, String> {
                    let bound = value.bind(py);
                    match bound.extract::<PyRef<PyProducerFrame>>() {
                        Ok(pf) => convert(&pf),
                        Err(_) => Err(format!(
                            "stream_provider source must yield ProducerFrame; got {}",
                            bound
                                .repr()
                                .map(|r| r.to_string())
                                .unwrap_or_else(|_| "<unrepr>".into())
                        )),
                    }
                });
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

type RustJpegFrameStream =
    Pin<Box<dyn Stream<Item = Result<RustConsumerFrame<RustJpegFrame>, RustStreamError>> + Send>>;
type RustPointCloudFrameStream = Pin<
    Box<
        dyn Stream<Item = Result<RustConsumerFrame<RustPointCloudFrame>, RustStreamError>> + Send,
    >,
>;
type RustJointEncodersFrameStream = Pin<
    Box<
        dyn Stream<
                Item = Result<RustConsumerFrame<RustJointEncodersFrame>, RustStreamError>,
            > + Send,
    >,
>;
type RustAudioFrameStream = Pin<
    Box<dyn Stream<Item = Result<RustConsumerFrame<RustAudioFrame>, RustStreamError>> + Send>,
>;

/// Tagged union over the underlying typed frame-stream the SDK returns
/// for an open subscription. Each substream is mono-`T`, so the
/// variant is fixed at `open_stream` time and never changes for the
/// lifetime of the subscription.
enum FrameStreamKind {
    Jpeg(RustJpegFrameStream),
    PointCloud(RustPointCloudFrameStream),
    JointEncoders(RustJointEncodersFrameStream),
    Audio(RustAudioFrameStream),
}

/// What `runtime.open_stream` / `runtime.open_pointcloud_stream`
/// returns on a successful Accept. Carries the producer's
/// [`PyAcceptInfo`]; iterate over frames via `subscription.frames()`.
///
/// The frames iterator can be fetched **at most once**. A second call
/// raises `RuntimeError` — the underlying Rust `Stream` is single-use.
#[pyclass(name = "StreamSubscription")]
pub struct PyStreamSubscription {
    info: PyAcceptInfo,
    frames: Mutex<Option<FrameStreamKind>>,
}

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
    pub fn from_rust_jpeg(rust_sub: RustStreamSubscription<RustJpegFrame>) -> Self {
        Self {
            info: PyAcceptInfo {
                inner: rust_sub.info,
            },
            frames: Mutex::new(Some(FrameStreamKind::Jpeg(rust_sub.frames))),
        }
    }

    pub fn from_rust_pointcloud(rust_sub: RustStreamSubscription<RustPointCloudFrame>) -> Self {
        Self {
            info: PyAcceptInfo {
                inner: rust_sub.info,
            },
            frames: Mutex::new(Some(FrameStreamKind::PointCloud(rust_sub.frames))),
        }
    }

    pub fn from_rust_joint_encoders(
        rust_sub: RustStreamSubscription<RustJointEncodersFrame>,
    ) -> Self {
        Self {
            info: PyAcceptInfo {
                inner: rust_sub.info,
            },
            frames: Mutex::new(Some(FrameStreamKind::JointEncoders(rust_sub.frames))),
        }
    }

    pub fn from_rust_audio(rust_sub: RustStreamSubscription<RustAudioFrame>) -> Self {
        Self {
            info: PyAcceptInfo {
                inner: rust_sub.info,
            },
            frames: Mutex::new(Some(FrameStreamKind::Audio(rust_sub.frames))),
        }
    }
}

/// Sync iterator over a [`PyStreamSubscription`]'s frames. Each
/// `__next__()` blocks the caller's thread on the wrapper's tokio
/// runtime until the next frame arrives. The substream's payload `T`
/// is fixed at open time; `__next__` dispatches on the stored
/// [`FrameStreamKind`] variant.
#[pyclass(name = "FrameIterator")]
pub struct PyFrameIterator {
    frames: Mutex<Option<FrameStreamKind>>,
}

/// Internal: result of polling one item out of either typed stream.
/// Used to keep the per-T monomorphization inside the `block_on`
/// closure and convert to a [`PyConsumerFrame`] with the GIL held
/// afterwards.
enum FrameNext {
    Jpeg(Result<RustConsumerFrame<RustJpegFrame>, RustStreamError>),
    PointCloud(Result<RustConsumerFrame<RustPointCloudFrame>, RustStreamError>),
    JointEncoders(Result<RustConsumerFrame<RustJointEncodersFrame>, RustStreamError>),
    Audio(Result<RustConsumerFrame<RustAudioFrame>, RustStreamError>),
    Done,
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
            rt.block_on(async {
                match &mut stream {
                    FrameStreamKind::Jpeg(s) => match s.next().await {
                        Some(item) => FrameNext::Jpeg(item),
                        None => FrameNext::Done,
                    },
                    FrameStreamKind::PointCloud(s) => match s.next().await {
                        Some(item) => FrameNext::PointCloud(item),
                        None => FrameNext::Done,
                    },
                    FrameStreamKind::JointEncoders(s) => match s.next().await {
                        Some(item) => FrameNext::JointEncoders(item),
                        None => FrameNext::Done,
                    },
                    FrameStreamKind::Audio(s) => match s.next().await {
                        Some(item) => FrameNext::Audio(item),
                        None => FrameNext::Done,
                    },
                }
            })
        });

        match item {
            FrameNext::Jpeg(Ok(frame)) => {
                let mut guard = self.frames.lock().expect("FrameIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyConsumerFrame::from_rust_jpeg(frame))
            }
            FrameNext::PointCloud(Ok(frame)) => {
                let mut guard = self.frames.lock().expect("FrameIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyConsumerFrame::from_rust_pointcloud(frame))
            }
            FrameNext::JointEncoders(Ok(frame)) => {
                let mut guard = self.frames.lock().expect("FrameIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyConsumerFrame::from_rust_joint_encoders(frame))
            }
            FrameNext::Audio(Ok(frame)) => {
                let mut guard = self.frames.lock().expect("FrameIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyConsumerFrame::from_rust_audio(frame))
            }
            FrameNext::Jpeg(Err(stream_err))
            | FrameNext::PointCloud(Err(stream_err))
            | FrameNext::JointEncoders(Err(stream_err))
            | FrameNext::Audio(Err(stream_err)) => {
                // Terminator. Don't put the stream back — exhausted.
                Err(stream_error_to_pyerr(py, stream_err))
            }
            FrameNext::Done => Err(PyStopIteration::new_err(())),
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
     (peer not reachable, peer doesn't speak `/auki/stream/0.1.0`, or \
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

pub fn open_stream_error_to_pyerr(py: Python<'_>, err: RustOpenStreamError) -> PyErr {
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

// ─── PyCapsule bridge for cross-.so consumers ────────────────────────────────

/// Capsule name for `StreamProvider` payloads exchanged with sibling
/// PyO3 wrapper crates. Includes a version suffix so future ABI changes
/// can rev the name and fail loudly on mismatch.
pub const STREAM_PROVIDER_CAPSULE_NAME: &str = "auki_network_py::stream_provider::v1";

/// Build a Rust `StreamProvider` from a Python callable and return it
/// wrapped in a `PyCapsule`. Used by `auki-domain-py` to cross the
/// `.so` boundary without dual-PyClass-identity errors — the
/// `PyStreamDecision` extract inside `build_stream_provider` executes
/// in this `.so` where the class is registered.
///
/// The capsule owns a `Box<StreamProvider>`. Consumers clone the `Arc`
/// out via `PyCapsule::reference::<StreamProvider>()`; PyO3's
/// destructor drops the box when the capsule is GC'd.
///
/// Underscore-prefixed in the Python namespace — this is an
/// SDK-internal bridge, not a public API for daemon authors.
#[pyfunction]
fn _build_stream_provider(py: Python<'_>, callable: Py<PyAny>) -> PyResult<Py<PyCapsule>> {
    let provider: StreamProvider = build_stream_provider(callable);
    let name = CString::new(STREAM_PROVIDER_CAPSULE_NAME).expect("static literal contains no nul");
    let capsule = PyCapsule::new_bound::<StreamProvider>(py, provider, Some(name))?;
    Ok(capsule.unbind())
}

// ─── Module registration ─────────────────────────────────────────────────────

pub(crate) fn register(py: Python<'_>, cluster: &Bound<'_, PyModule>) -> PyResult<()> {
    cluster.add_class::<PyStreamRequest>()?;
    cluster.add_class::<PyAcceptInfo>()?;
    cluster.add_class::<PyJpegFrame>()?;
    cluster.add_class::<PyPointCloudFrame>()?;
    cluster.add_class::<PyJointEncodersFrame>()?;
    cluster.add_class::<PyAudioFrame>()?;
    cluster.add_class::<PyDeclineReason>()?;
    cluster.add_class::<PyEndReason>()?;
    cluster.add_class::<PyProducerFrame>()?;
    cluster.add_class::<PyConsumerFrame>()?;
    cluster.add_class::<PyStreamDecision>()?;
    cluster.add_class::<PyStreamSubscription>()?;
    cluster.add_class::<PyFrameIterator>()?;

    cluster.add_function(wrap_pyfunction!(_build_stream_provider, cluster)?)?;

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

    /// Helper: wrap a [`PyJpegFrame`] as a `Bound<'_, PyAny>` for the
    /// new typed-payload `PyProducerFrame::new` constructor (which
    /// accepts either `JpegFrame` or `PointCloudFrame`).
    fn jpeg_frame_as_any<'py>(py: Python<'py>, bytes: &[u8]) -> Bound<'py, PyAny> {
        let frame = PyJpegFrame::new(PyBytes::new_bound(py, bytes));
        Py::new(py, frame)
            .expect("alloc PyJpegFrame")
            .bind(py)
            .clone()
            .into_any()
    }

    /// Helper: wrap a [`PyPointCloudFrame`] as a `Bound<'_, PyAny>` for
    /// the typed-payload `PyProducerFrame::new` constructor.
    fn pointcloud_frame_as_any<'py>(py: Python<'py>, bytes: &[u8]) -> Bound<'py, PyAny> {
        let frame = PyPointCloudFrame::new(PyBytes::new_bound(py, bytes));
        Py::new(py, frame)
            .expect("alloc PyPointCloudFrame")
            .bind(py)
            .clone()
            .into_any()
    }

    /// Helper: wrap a [`PyAudioFrame`] as a `Bound<'_, PyAny>` for the
    /// typed-payload `PyProducerFrame::new` constructor (Dialogue
    /// Batch 1).
    fn audio_frame_as_any<'py>(py: Python<'py>, data: &[u8]) -> Bound<'py, PyAny> {
        let frame = PyAudioFrame::new(PyBytes::new_bound(py, data));
        Py::new(py, frame)
            .expect("alloc PyAudioFrame")
            .bind(py)
            .clone()
            .into_any()
    }

    #[test]
    fn point_cloud_frame_round_trips_through_pybytes() {
        Python::with_gil(|py| {
            let payload = PyBytes::new_bound(py, &[0x10, 0x20, 0x30]);
            let f = PyPointCloudFrame::new(payload);
            assert_eq!(f.__len__(), 3);
            assert_eq!(f.bytes(py).as_bytes(), &[0x10, 0x20, 0x30]);
            assert_eq!(f.__repr__(), "PointCloudFrame(<3 bytes>)");
        });
    }

    /// Dialogue Batch 1 — `PyAudioFrame` is the audio analog of
    /// `PyJpegFrame` / `PyPointCloudFrame`. Same opaque-bytes shape on
    /// the Python surface, but the getter is named `.data` to match
    /// the underlying `bytes data = 1` proto field (not `bytes bytes
    /// = 1`).
    #[test]
    fn audio_frame_round_trips_through_pybytes() {
        Python::with_gil(|py| {
            let payload = PyBytes::new_bound(py, &[0x00, 0x80, 0xff, 0x7f]);
            let f = PyAudioFrame::new(payload);
            assert_eq!(f.__len__(), 4);
            assert_eq!(f.data(py).as_bytes(), &[0x00, 0x80, 0xff, 0x7f]);
            assert_eq!(f.__repr__(), "AudioFrame(<4 bytes>)");
        });
    }

    #[test]
    fn producer_frame_extracts_to_rust_jpeg() {
        Python::with_gil(|py| {
            let payload_any = jpeg_frame_as_any(py, &[1, 2, 3]);
            let pf = PyProducerFrame::new(123_456_789, payload_any).unwrap();
            let rust = pf.to_rust_jpeg().expect("payload is Jpeg");
            assert_eq!(rust.timestamp_ns, 123_456_789);
            assert_eq!(rust.payload.bytes, vec![1, 2, 3]);
        });
    }

    #[test]
    fn producer_frame_extracts_to_rust_pointcloud() {
        Python::with_gil(|py| {
            let payload_any = pointcloud_frame_as_any(py, &[0xaa, 0xbb]);
            let pf = PyProducerFrame::new(42, payload_any).unwrap();
            let rust = pf.to_rust_pointcloud().expect("payload is PointCloud");
            assert_eq!(rust.timestamp_ns, 42);
            assert_eq!(rust.payload.bytes, vec![0xaa, 0xbb]);
        });
    }

    #[test]
    fn producer_frame_extracts_to_rust_audio() {
        Python::with_gil(|py| {
            let payload_any = audio_frame_as_any(py, &[0x01, 0x02, 0x03]);
            let pf = PyProducerFrame::new(987_654, payload_any).unwrap();
            let rust = pf.to_rust_audio().expect("payload is Audio");
            assert_eq!(rust.timestamp_ns, 987_654);
            assert_eq!(rust.payload.data, vec![0x01, 0x02, 0x03]);
        });
    }

    /// Mismatched payload variant on `to_rust_*` must surface a
    /// human-readable error — the source-stream pump turns this into an
    /// `EndReason::ProducerError` on the wire rather than ending the
    /// substream silently.
    #[test]
    fn producer_frame_to_rust_errors_on_mismatched_payload() {
        Python::with_gil(|py| {
            let pf_jpeg = PyProducerFrame::new(0, jpeg_frame_as_any(py, &[1])).unwrap();
            let err = pf_jpeg.to_rust_pointcloud().expect_err("jpeg ≠ pointcloud");
            assert!(err.contains("AcceptPointCloud"), "{err}");
            assert!(err.contains("PointCloudFrame"), "{err}");

            let pf_pc =
                PyProducerFrame::new(0, pointcloud_frame_as_any(py, &[2])).unwrap();
            let err = pf_pc.to_rust_jpeg().expect_err("pointcloud ≠ jpeg");
            assert!(err.contains("AcceptJpeg"), "{err}");
            assert!(err.contains("JpegFrame"), "{err}");

            // Audio ≠ Jpeg: same mismatch shape for the Dialogue arm.
            let pf_audio = PyProducerFrame::new(0, audio_frame_as_any(py, &[3])).unwrap();
            let err = pf_audio.to_rust_jpeg().expect_err("audio ≠ jpeg");
            assert!(err.contains("AcceptJpeg"), "{err}");
            assert!(err.contains("AudioFrame"), "{err}");
        });
    }

    /// A non-JpegFrame / non-PointCloudFrame object passed as `payload`
    /// must surface a `ValueError` at construction time — the Python
    /// surface is closed over the two SDK-supported `T`s.
    #[test]
    fn producer_frame_rejects_unknown_payload_type() {
        Python::with_gil(|py| {
            // py.None() is a stand-in for "anything that isn't a frame
            // PyClass" — same shape as a Python user passing a dict, an
            // int, a custom class, etc.
            let bad = py.None();
            let err =
                PyProducerFrame::new(0, bad.bind(py).clone()).expect_err("None is not a frame");
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(err.to_string().contains("JpegFrame"), "{err}");
            assert!(err.to_string().contains("PointCloudFrame"), "{err}");
            assert!(err.to_string().contains("AudioFrame"), "{err}");
        });
    }

    #[test]
    fn consumer_frame_constructs_from_rust_jpeg() {
        Python::with_gil(|_py| {
            let rust_frame = RustConsumerFrame {
                timestamp_ns: 9_999,
                seq: 17,
                payload: RustJpegFrame {
                    bytes: vec![0xff, 0xd8, 0xee],
                },
            };
            let pf = PyConsumerFrame::from_rust_jpeg(rust_frame);
            assert_eq!(pf.timestamp_ns(), 9_999);
            assert_eq!(pf.seq(), 17);
            // Inspect the payload variant directly via the
            // `pub(crate)` field — exposing `__len__` would require
            // routing through the GIL-bound getter.
            match &pf.payload {
                FramePayload::Jpeg(j) => assert_eq!(j.inner.bytes.len(), 3),
                _ => panic!("expected Jpeg payload variant"),
            }
        });
    }

    #[test]
    fn consumer_frame_constructs_from_rust_pointcloud() {
        Python::with_gil(|_py| {
            let rust_frame = RustConsumerFrame {
                timestamp_ns: 1_000,
                seq: 7,
                payload: RustPointCloudFrame {
                    bytes: vec![0x01, 0x02, 0x03, 0x04],
                },
            };
            let pf = PyConsumerFrame::from_rust_pointcloud(rust_frame);
            assert_eq!(pf.timestamp_ns(), 1_000);
            assert_eq!(pf.seq(), 7);
            match &pf.payload {
                FramePayload::PointCloud(p) => assert_eq!(p.inner.bytes.len(), 4),
                _ => panic!("expected PointCloud payload variant"),
            }
        });
    }

    #[test]
    fn consumer_frame_constructs_from_rust_audio() {
        Python::with_gil(|_py| {
            let rust_frame = RustConsumerFrame {
                timestamp_ns: 5_555,
                seq: 99,
                payload: RustAudioFrame {
                    data: vec![0xab, 0xcd, 0xef],
                },
            };
            let pf = PyConsumerFrame::from_rust_audio(rust_frame);
            assert_eq!(pf.timestamp_ns(), 5_555);
            assert_eq!(pf.seq(), 99);
            match &pf.payload {
                FramePayload::Audio(a) => assert_eq!(a.inner.data.len(), 3),
                _ => panic!("expected Audio payload variant"),
            }
        });
    }

    #[test]
    fn stream_decision_factories_tag_correctly() {
        Python::with_gil(|py| {
            // Construct a Python object to stand in for the source iterator
            // (a None object is fine — we only inspect .kind, never drain).
            let info = PyAcceptInfo::new("h".into(), "c".into(), "ch".into());
            let acc = PyStreamDecision::accept(info.clone(), py.None());
            assert_eq!(acc.kind(), "accept");

            let acc_pc = PyStreamDecision::accept_pointcloud(info.clone(), py.None());
            assert_eq!(acc_pc.kind(), "accept_pointcloud");

            let acc_audio = PyStreamDecision::accept_audio(info, py.None());
            assert_eq!(acc_audio.kind(), "accept_audio");

            let dec = PyStreamDecision::decline(PyDeclineReason::sensor_not_found());
            assert_eq!(dec.kind(), "decline");

            // After taking, the decision reports `consumed`.
            let _taken = acc.take();
            assert_eq!(acc.kind(), "consumed");
            let _taken = acc_pc.take();
            assert_eq!(acc_pc.kind(), "consumed");
            let _taken = acc_audio.take();
            assert_eq!(acc_audio.kind(), "consumed");
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
                RustStreamDispatch::Decline { reason }
                    if matches!(reason.kind, Some(decline_reason::Kind::SensorNotFound(_))) => {}
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
                RustStreamDispatch::Decline { reason } => match reason.kind {
                    Some(decline_reason::Kind::Other(decline_reason::Other { detail })) => assert!(
                        detail.contains("provider broke"),
                        "decline detail should carry the Python error: {detail}",
                    ),
                    other => panic!("expected Other variant; got {other:?}"),
                },
                _ => panic!("expected Decline(Other) with the Python error in detail"),
            }
        });
    }

    /// `build_stream_provider` mapping the Python `accept(info, source)`
    /// factory onto `RustStreamDispatch::AcceptJpeg`. We don't drain the
    /// source-stream here (that requires the wrapper's tokio runtime
    /// + asyncio loop scaffolding from the cross-language tests); we
    /// only assert that the dispatch variant matches the Python call.
    #[test]
    fn build_stream_provider_accept_jpeg_maps_to_dispatch_acceptjpeg() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "test_provider_accept_jpeg").unwrap();
            crate::populate_module(&module).unwrap();
            let cluster = module.getattr("cluster").unwrap();

            py.run_bound(
                r#"
def _make(cluster):
    async def _src():
        if False:
            yield None  # makes this an async generator
    def provider(req):
        return cluster.StreamDecision.accept(
            info=cluster.AcceptInfo(sensor_hash="h", clock_id="c", clock_hash="ch"),
            source=_src(),
        )
    return provider
"#,
                None,
                None,
            )
            .unwrap();
            let make = py.eval_bound("_make", None, None).unwrap();
            let provider = make.call1((&cluster,)).unwrap();
            let rust_provider = build_stream_provider(provider.unbind());

            match rust_provider(RustStreamRequest {
                sensor_id: "any".into(),
            }) {
                RustStreamDispatch::AcceptJpeg { info, source: _ } => {
                    assert_eq!(info.sensor_hash, "h");
                    assert_eq!(info.clock_id, "c");
                }
                _ => panic!("expected AcceptJpeg"),
            }
        });
    }

    /// `build_stream_provider` mapping `accept_pointcloud(info, source)`
    /// onto `RustStreamDispatch::AcceptPointCloud` (Dagaz Batch 2 — the
    /// new dispatch arm).
    #[test]
    fn build_stream_provider_accept_pointcloud_maps_to_dispatch_acceptpointcloud() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "test_provider_accept_pc").unwrap();
            crate::populate_module(&module).unwrap();
            let cluster = module.getattr("cluster").unwrap();

            py.run_bound(
                r#"
def _make(cluster):
    async def _src():
        if False:
            yield None
    def provider(req):
        return cluster.StreamDecision.accept_pointcloud(
            info=cluster.AcceptInfo(sensor_hash="pc", clock_id="c", clock_hash="ch"),
            source=_src(),
        )
    return provider
"#,
                None,
                None,
            )
            .unwrap();
            let make = py.eval_bound("_make", None, None).unwrap();
            let provider = make.call1((&cluster,)).unwrap();
            let rust_provider = build_stream_provider(provider.unbind());

            match rust_provider(RustStreamRequest {
                sensor_id: "any".into(),
            }) {
                RustStreamDispatch::AcceptPointCloud { info, source: _ } => {
                    assert_eq!(info.sensor_hash, "pc");
                }
                _ => panic!("expected AcceptPointCloud"),
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
