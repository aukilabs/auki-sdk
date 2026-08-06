//! Python wrappers for grimsby's `Stream<T>` Rust API surface — wire
//! types, [`StreamDecision`], the [`PyStreamProvider`] adapter, and
//! [`StreamSubscription`] / [`StreamEntryIterator`].
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
//!   accept-time metadata via `.manifest`; `.entries()` returns a sync
//!   iterator that blocks on each `__next__()` until the next entry
//!   arrives. Stream-end signals surface as Python exceptions raised
//!   from `__next__()`.
//!
//! Sync everywhere on the *callable surface* (Pattern A, per the
//! status log 2026-05-05) — the asyncio plumbing is internal to the
//! SDK's tokio worker. Caller processes (BoosterApp's `BaseHTTPServer`
//! sidecar; future Sentinel-as-consumer) stay sync-shaped.

use auki_datatypes::{
    audio::Data as RustAudioFrame,
    detection::DetectionFrame as RustDetectionFrame,
    joint_encoders::Data as RustJointEncodersFrame,
    map::MapUpdate as RustMapUpdate,
    point_cloud::Data as RustPointCloudFrame,
    pose::{
        Quat as RustPoseQuat, SpatialTransform as RustPoseSpatialTransform, Vec3 as RustPoseVec3,
    },
    scalar::Data as RustScalarFrame,
};
use auki_logs_py_rs::{
    RawBytes as RustRawLogBytes, RetainedStreamSource as RustRetainedStreamSource,
    STREAM_SOURCE_CAPSULE_NAME,
};
use auki_logs_rs::{Error as RustLogError, Log as RustLog};
use auki_network_rs::stream_protocol::{
    CameraFrame as RustCameraFrame, DeclineReason as RustDeclineReason,
    DynamicIntrinsics as RustDynamicIntrinsics, EndReason as RustEndReason,
    ReadFrom as RustReadFrom, StreamManifest as RustStreamManifest,
    StreamRequest as RustStreamRequest, decline_reason, end_reason,
};
use auki_network_rs::stream_runtime::{
    OpenStreamError as RustOpenStreamError, SourceStream, StreamDispatch as RustStreamDispatch,
    StreamEntry as RustStreamEntry, StreamError as RustStreamError, StreamItem as RustStreamItem,
    StreamProvider, StreamSubscription as RustStreamSubscription,
};
use futures::{Stream, StreamExt};
use prost::Message;
use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyStopIteration, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyCapsule, PyModule};
use std::collections::VecDeque;
use std::ffi::CString;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::stream_bridge::PyAsyncIterStream;

// ─── StreamRequest ───────────────────────────────────────────────────────────

/// Inbound request the SDK delivers to the Python `stream_provider`.
/// Carries `resource_id` (the log's stable identity) and `source_peer_id`
/// (the peer that originally wrote the log).
#[pyclass(name = "StreamRequest", frozen)]
#[derive(Clone, Debug)]
pub struct PyStreamRequest {
    pub(crate) inner: RustStreamRequest,
}

#[pymethods]
impl PyStreamRequest {
    #[new]
    #[pyo3(signature = (*, resource_id, source_peer_id = String::new()))]
    fn new(resource_id: String, source_peer_id: String) -> Self {
        Self {
            inner: RustStreamRequest {
                resource_id,
                source_peer_id,
                ..Default::default()
            },
        }
    }

    #[getter]
    fn resource_id(&self) -> &str {
        &self.inner.resource_id
    }

    #[getter]
    fn source_peer_id(&self) -> &str {
        &self.inner.source_peer_id
    }

    fn __repr__(&self) -> String {
        format!(
            "StreamRequest(resource_id={:?}, source_peer_id={:?})",
            self.inner.resource_id, self.inner.source_peer_id
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── StreamManifest ──────────────────────────────────────────────────────────────

/// Accept-time stream manifest the producer commits to for the
/// lifetime of the subscription.
#[pyclass(name = "StreamManifest", frozen)]
#[derive(Clone, Debug)]
pub struct PyStreamManifest {
    pub(crate) inner: RustStreamManifest,
}

#[pymethods]
impl PyStreamManifest {
    #[new]
    #[pyo3(signature = (*, sensor_id, sensor_hash, clock_id, clock_hash, frame_id=None, frame_hash=None))]
    fn new(
        sensor_id: String,
        sensor_hash: String,
        clock_id: String,
        clock_hash: String,
        frame_id: Option<String>,
        frame_hash: Option<String>,
    ) -> Self {
        Self {
            inner: RustStreamManifest {
                sensor_id,
                sensor_hash,
                clock_id,
                clock_hash,
                frame_id: frame_id.unwrap_or_default(),
                frame_hash: frame_hash.unwrap_or_default(),
                ..Default::default()
            },
        }
    }

    #[staticmethod]
    #[pyo3(signature = (*, resource_id, clock_id, clock_hash, from_frame_id, from_frame_hash, to_frame_id, to_frame_hash, writer_mode=None, expected_rate_hz=None))]
    fn pose_stream(
        resource_id: String,
        clock_id: String,
        clock_hash: String,
        from_frame_id: String,
        from_frame_hash: String,
        to_frame_id: String,
        to_frame_hash: String,
        writer_mode: Option<String>,
        expected_rate_hz: Option<u32>,
    ) -> Self {
        Self {
            inner: RustStreamManifest {
                resource_id,
                payload: "spatial_transform".into(),
                clock_id,
                clock_hash,
                from_frame_id,
                from_frame_hash,
                to_frame_id,
                to_frame_hash,
                writer_mode: writer_mode.unwrap_or_else(|| "movable".into()),
                expected_rate_hz: expected_rate_hz.unwrap_or_default(),
                ..Default::default()
            },
        }
    }

    #[staticmethod]
    #[pyo3(signature = (*, resource_id, map_peer_id, map_id, map_hash, clock_peer_id, clock_id, clock_hash))]
    fn map_stream(
        resource_id: String,
        map_peer_id: String,
        map_id: String,
        map_hash: String,
        clock_peer_id: String,
        clock_id: String,
        clock_hash: String,
    ) -> Self {
        Self {
            inner: RustStreamManifest {
                resource_id,
                payload: "map_update".into(),
                map_peer_id,
                map_id,
                map_hash,
                clock_peer_id,
                clock_id,
                clock_hash,
                ..Default::default()
            },
        }
    }

    #[getter]
    fn sensor_id(&self) -> &str {
        &self.inner.sensor_id
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

    #[getter]
    fn frame_id(&self) -> &str {
        &self.inner.frame_id
    }

    #[getter]
    fn frame_hash(&self) -> &str {
        &self.inner.frame_hash
    }

    #[getter]
    fn resource_id(&self) -> &str {
        &self.inner.resource_id
    }

    #[getter]
    fn payload(&self) -> &str {
        &self.inner.payload
    }

    #[getter]
    fn from_frame_id(&self) -> &str {
        &self.inner.from_frame_id
    }

    #[getter]
    fn from_frame_hash(&self) -> &str {
        &self.inner.from_frame_hash
    }

    #[getter]
    fn to_frame_id(&self) -> &str {
        &self.inner.to_frame_id
    }

    #[getter]
    fn to_frame_hash(&self) -> &str {
        &self.inner.to_frame_hash
    }

    #[getter]
    fn writer_mode(&self) -> &str {
        &self.inner.writer_mode
    }

    #[getter]
    fn expected_rate_hz(&self) -> u32 {
        self.inner.expected_rate_hz
    }

    #[getter]
    fn map_peer_id(&self) -> &str {
        &self.inner.map_peer_id
    }

    #[getter]
    fn map_id(&self) -> &str {
        &self.inner.map_id
    }

    #[getter]
    fn map_hash(&self) -> &str {
        &self.inner.map_hash
    }

    #[getter]
    fn clock_peer_id(&self) -> &str {
        &self.inner.clock_peer_id
    }

    fn __repr__(&self) -> String {
        if self.inner.resource_id.is_empty() {
            format!(
                "StreamManifest(sensor_id={:?}, sensor_hash={:?}, clock_id={:?}, clock_hash={:?}, frame_id={:?}, frame_hash={:?})",
                self.inner.sensor_id,
                self.inner.sensor_hash,
                self.inner.clock_id,
                self.inner.clock_hash,
                self.inner.frame_id,
                self.inner.frame_hash,
            )
        } else {
            format!(
                "StreamManifest(resource_id={:?}, payload={:?}, clock_id={:?}, from_frame_id={:?}, to_frame_id={:?})",
                self.inner.resource_id,
                self.inner.payload,
                self.inner.clock_id,
                self.inner.from_frame_id,
                self.inner.to_frame_id,
            )
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── CameraFrame ──────────────────────────────────────────────────

/// Optional per-frame pinhole intrinsics carried by
/// [`PyCameraFrame`]. Static intrinsics still live in the
/// Sensor Registry; this field is for cameras whose intrinsics can vary
/// per frame.
#[pyclass(name = "DynamicIntrinsics", frozen)]
#[derive(Clone, Debug, PartialEq)]
pub struct PyDynamicIntrinsics {
    pub(crate) inner: RustDynamicIntrinsics,
}

#[pymethods]
impl PyDynamicIntrinsics {
    #[new]
    #[pyo3(signature = (*, fx, fy, cx, cy, distortion_coefficients=None))]
    fn new(fx: f64, fy: f64, cx: f64, cy: f64, distortion_coefficients: Option<Vec<f64>>) -> Self {
        Self {
            inner: RustDynamicIntrinsics {
                fx,
                fy,
                cx,
                cy,
                distortion_coefficients: distortion_coefficients.unwrap_or_default(),
            },
        }
    }

    #[getter]
    fn fx(&self) -> f64 {
        self.inner.fx
    }

    #[getter]
    fn fy(&self) -> f64 {
        self.inner.fy
    }

    #[getter]
    fn cx(&self) -> f64 {
        self.inner.cx
    }

    #[getter]
    fn cy(&self) -> f64 {
        self.inner.cy
    }

    #[getter]
    fn distortion_coefficients(&self) -> Vec<f64> {
        self.inner.distortion_coefficients.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "DynamicIntrinsics(fx={}, fy={}, cx={}, cy={}, distortion_coefficients=<{} values>)",
            self.inner.fx,
            self.inner.fy,
            self.inner.cx,
            self.inner.cy,
            self.inner.distortion_coefficients.len(),
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Camera stream payload `T`. Python exposes this as `CameraFrame`; internally
/// it is the same protobuf record as the on-disk
/// `auki.camera.CameraFrame`, so the bytes inside
/// `StreamEntry.payload` can match the camera Sensor Log entry exactly.
#[pyclass(name = "CameraFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyCameraFrame {
    pub(crate) inner: RustCameraFrame,
}

#[pymethods]
impl PyCameraFrame {
    #[new]
    #[pyo3(signature = (frame, /, dynamic_intrinsics=None))]
    fn new(frame: Bound<'_, PyBytes>, dynamic_intrinsics: Option<PyDynamicIntrinsics>) -> Self {
        Self {
            inner: RustCameraFrame {
                dynamic_intrinsics: dynamic_intrinsics.map(|i| i.inner),
                frame: frame.as_bytes().to_vec(),
            },
        }
    }

    /// Encoded camera frame bytes. Returns a fresh `bytes` copy each call.
    #[getter]
    fn frame<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.frame)
    }

    #[getter]
    fn dynamic_intrinsics(&self) -> Option<PyDynamicIntrinsics> {
        self.inner
            .dynamic_intrinsics
            .clone()
            .map(|inner| PyDynamicIntrinsics { inner })
    }

    fn __len__(&self) -> usize {
        self.inner.frame.len()
    }

    fn __repr__(&self) -> String {
        format!("CameraFrame(<{} frame bytes>)", self.inner.frame.len())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── PointCloudFrame ─────────────────────────────────────────────────────────

/// Dagaz Batch 1 payload `T` — raw CDR-encoded `PointCloud2` ROS message
/// bytes (per [Dagaz](https://www.notion.so/3585c8e96592805b8d83c89f849d3577) D2).
/// Consumer (Park, future Sentinel) parses CDR on its side; the SDK doesn't
/// decode or interpret these bytes. Same shape as [`PyCameraFrame`] —
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
                data: bytes.as_bytes().to_vec(),
            },
        }
    }

    /// Raw CDR-encoded `PointCloud2` bytes. Returns a fresh `bytes` copy
    /// each call.
    #[getter]
    fn bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.data)
    }

    fn __len__(&self) -> usize {
        self.inner.data.len()
    }

    fn __repr__(&self) -> String {
        format!("PointCloudFrame(<{} bytes>)", self.inner.data.len())
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
/// Differs from [`PyCameraFrame`] / [`PyPointCloudFrame`] in payload
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
        format!(
            "JointEncodersFrame(<{} joints>)",
            self.inner.angles_rad.len()
        )
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
/// at handshake. Same opaque-`bytes`-property shape as [`PyCameraFrame`] /
/// [`PyPointCloudFrame`], but the Python getter is named `.data` to
/// match the proto field name (the proto says `bytes data`, not
/// `bytes bytes`).
#[pyclass(name = "AudioFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyAudioFrame {
    pub(crate) inner: RustAudioFrame,
}

// ─── DetectionFrame ─────────────────────────────────────────────────────────

/// Detector-agnostic output from a Detection Log. `data` is decoded according
/// to the open `type` vocabulary; `sensor_hash` pins the input Sensor Registry
/// entry against which the detector ran.
#[pyclass(name = "DetectionFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyDetectionFrame {
    pub(crate) inner: RustDetectionFrame,
}

#[pymethods]
impl PyDetectionFrame {
    #[new]
    #[pyo3(signature = (*, data, sensor_hash, type_))]
    fn new(data: Bound<'_, PyBytes>, sensor_hash: String, type_: String) -> Self {
        Self {
            inner: RustDetectionFrame {
                data: data.as_bytes().to_vec(),
                sensor_hash,
                r#type: type_,
            },
        }
    }

    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.data)
    }

    #[getter]
    fn sensor_hash(&self) -> &str {
        &self.inner.sensor_hash
    }

    #[getter]
    fn r#type(&self) -> &str {
        &self.inner.r#type
    }

    fn __len__(&self) -> usize {
        self.inner.data.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "DetectionFrame(type={:?}, sensor_hash={:?}, <{} data bytes>)",
            self.inner.r#type,
            self.inner.sensor_hash,
            self.inner.data.len(),
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
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

// ─── ScalarFrame ────────────────────────────────────────────────────────────

/// One non-spatial scalar sample. The measured quantity and unit are pinned
/// by the Scalar Sensor Registry entry; the live/log payload only carries the
/// numeric value.
#[pyclass(name = "ScalarFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyScalarFrame {
    pub(crate) inner: RustScalarFrame,
}

#[pymethods]
impl PyScalarFrame {
    #[new]
    #[pyo3(signature = (value, /))]
    fn new(value: f64) -> Self {
        Self {
            inner: RustScalarFrame { value },
        }
    }

    #[getter]
    fn value(&self) -> f64 {
        self.inner.value
    }

    fn __repr__(&self) -> String {
        format!("ScalarFrame(value={})", self.inner.value)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── SpatialTransformFrame ──────────────────────────────────────────────────

/// Live pose stream payload `T` — one flat
/// `auki.pose.SpatialTransform` sample for the frame pair committed in
/// the stream manifest. Python uses seven values:
/// `[tx, ty, tz, qx, qy, qz, qw]`.
#[pyclass(name = "SpatialTransformFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PySpatialTransformFrame {
    pub(crate) inner: RustPoseSpatialTransform,
}

#[pymethods]
impl PySpatialTransformFrame {
    #[new]
    #[pyo3(signature = (values, /))]
    fn new(values: Vec<f64>) -> PyResult<Self> {
        if values.len() != 7 {
            return Err(PyValueError::new_err(format!(
                "SpatialTransformFrame expects 7 values [tx, ty, tz, qx, qy, qz, qw]; got {}",
                values.len()
            )));
        }
        Ok(Self {
            inner: RustPoseSpatialTransform {
                translation: Some(RustPoseVec3 {
                    x: values[0],
                    y: values[1],
                    z: values[2],
                }),
                orientation: Some(RustPoseQuat {
                    x: values[3],
                    y: values[4],
                    z: values[5],
                    w: values[6],
                }),
            },
        })
    }

    #[getter]
    fn values(&self) -> Vec<f64> {
        let t = self.inner.translation.as_ref();
        let q = self.inner.orientation.as_ref();
        vec![
            t.map(|v| v.x).unwrap_or(0.0),
            t.map(|v| v.y).unwrap_or(0.0),
            t.map(|v| v.z).unwrap_or(0.0),
            q.map(|v| v.x).unwrap_or(0.0),
            q.map(|v| v.y).unwrap_or(0.0),
            q.map(|v| v.z).unwrap_or(0.0),
            q.map(|v| v.w).unwrap_or(1.0),
        ]
    }

    fn __len__(&self) -> usize {
        7
    }

    fn __repr__(&self) -> String {
        format!("SpatialTransformFrame({:?})", self.values())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

// ─── MapUpdateFrame ─────────────────────────────────────────────────────────

/// Encoded SDK `MapUpdate` stream payload. Construction validates the
/// protobuf once; `.data` returns the canonical encoded bytes for replay into
/// an SDK map accumulator.
#[pyclass(name = "MapUpdateFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyMapUpdateFrame {
    pub(crate) inner: RustMapUpdate,
}

#[pymethods]
impl PyMapUpdateFrame {
    #[new]
    #[pyo3(signature = (data, /))]
    fn new(data: Bound<'_, PyBytes>) -> PyResult<Self> {
        let inner = RustMapUpdate::decode(data.as_bytes())
            .map_err(|e| PyValueError::new_err(format!("invalid MapUpdate bytes: {e}")))?;
        Ok(Self { inner })
    }

    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.encode_to_vec())
    }

    fn __len__(&self) -> usize {
        self.inner.encoded_len()
    }

    fn __repr__(&self) -> String {
        format!("MapUpdateFrame(<{} bytes>)", self.inner.encoded_len())
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

// ─── Stream payload (Camera vs PointCloud vs JointEncoders) ────────────────────

/// Tagged union over the payload `T`s the SDK currently supports.
/// The producer's [`PyStreamItem`] and the consumer's [`PyStreamEntry`]
/// both carry one of these; the wire-side substream is mono-`T` per the
/// matching [`RustStreamDispatch`] variant.
#[derive(Clone, Debug)]
pub(crate) enum StreamPayload {
    Camera(PyCameraFrame),
    PointCloud(PyPointCloudFrame),
    JointEncoders(PyJointEncodersFrame),
    Audio(PyAudioFrame),
    Scalar(PyScalarFrame),
    Detection(PyDetectionFrame),
    Pose(PySpatialTransformFrame),
    Map(PyMapUpdateFrame),
}

impl StreamPayload {
    fn from_py(payload: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(camera) = payload.extract::<PyCameraFrame>() {
            return Ok(Self::Camera(camera));
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
        if let Ok(scalar) = payload.extract::<PyScalarFrame>() {
            return Ok(Self::Scalar(scalar));
        }
        if let Ok(detection) = payload.extract::<PyDetectionFrame>() {
            return Ok(Self::Detection(detection));
        }
        if let Ok(pose) = payload.extract::<PySpatialTransformFrame>() {
            return Ok(Self::Pose(pose));
        }
        if let Ok(map) = payload.extract::<PyMapUpdateFrame>() {
            return Ok(Self::Map(map));
        }
        Err(PyValueError::new_err(format!(
            "stream payload must be a CameraFrame, PointCloudFrame, JointEncodersFrame, AudioFrame, ScalarFrame, DetectionFrame, SpatialTransformFrame, or MapUpdateFrame; got {}",
            payload
                .repr()
                .map(|r| r.to_string())
                .unwrap_or_else(|_| "<unrepr>".into()),
        )))
    }

    fn into_py(self, py: Python<'_>) -> PyObject {
        match self {
            Self::Camera(f) => Py::new(py, f).expect("alloc CameraFrame").into_py(py),
            Self::PointCloud(f) => Py::new(py, f).expect("alloc PointCloudFrame").into_py(py),
            Self::JointEncoders(f) => Py::new(py, f)
                .expect("alloc JointEncodersFrame")
                .into_py(py),
            Self::Audio(f) => Py::new(py, f).expect("alloc AudioFrame").into_py(py),
            Self::Scalar(f) => Py::new(py, f).expect("alloc ScalarFrame").into_py(py),
            Self::Detection(f) => Py::new(py, f).expect("alloc DetectionFrame").into_py(py),
            Self::Pose(f) => Py::new(py, f)
                .expect("alloc SpatialTransformFrame")
                .into_py(py),
            Self::Map(f) => Py::new(py, f).expect("alloc MapUpdateFrame").into_py(py),
        }
    }

    fn repr(&self) -> String {
        match self {
            Self::Camera(f) => f.__repr__(),
            Self::PointCloud(f) => f.__repr__(),
            Self::JointEncoders(f) => f.__repr__(),
            Self::Audio(f) => f.__repr__(),
            Self::Scalar(f) => f.__repr__(),
            Self::Detection(f) => f.__repr__(),
            Self::Pose(f) => f.__repr__(),
            Self::Map(f) => f.__repr__(),
        }
    }
}

// ─── StreamItem ───────────────────────────────────────────────────────────

/// What the producer's source-iterator yields. `seq` is stamped by the
/// SDK at send time; producers only set `timestamp_ns` + `payload`.
///
/// `payload` accepts either a [`PyCameraFrame`] or a [`PyPointCloudFrame`]
/// (Dagaz Batch 2). The SDK type-checks the payload against the matching
/// [`PyStreamDecision`] accept variant when draining the source iterator
/// — yielding a `CameraFrame` from an `accept_pointcloud(...)` source ends
/// the substream with `EndReason::ProducerError`.
#[pyclass(name = "StreamItem", frozen)]
#[derive(Clone, Debug)]
pub struct PyStreamItem {
    pub(crate) timestamp_ns: i64,
    pub(crate) payload: StreamPayload,
}

#[pymethods]
impl PyStreamItem {
    #[new]
    #[pyo3(signature = (*, timestamp_ns, payload))]
    fn new(timestamp_ns: i64, payload: Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            timestamp_ns,
            payload: StreamPayload::from_py(&payload)?,
        })
    }

    #[getter]
    fn timestamp_ns(&self) -> i64 {
        self.timestamp_ns
    }

    /// The wrapped stream payload — either a `CameraFrame` or a
    /// `PointCloudFrame`. Returns a fresh Python object each call.
    #[getter]
    fn payload(&self, py: Python<'_>) -> PyObject {
        self.payload.clone().into_py(py)
    }

    fn __repr__(&self) -> String {
        format!(
            "StreamItem(timestamp_ns={}, payload={})",
            self.timestamp_ns,
            self.payload.repr(),
        )
    }
}

impl PyStreamItem {
    /// Convert to a `RustStreamItem<RustCameraFrame>`. Errors with a
    /// human-readable detail if the payload is `PointCloud`. Used by
    /// the producer-side source-stream pump for an `AcceptCamera`
    /// dispatch.
    pub(crate) fn to_rust_camera(&self) -> Result<RustStreamItem<RustCameraFrame>, String> {
        match &self.payload {
            StreamPayload::Camera(f) => Ok(RustStreamItem {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptCamera source yielded a StreamItem with {} payload; \
                 the substream is mono-T — yield CameraFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }

    /// Convert to a `RustStreamItem<RustPointCloudFrame>`. Errors
    /// with a human-readable detail if the payload is the wrong variant.
    pub(crate) fn to_rust_pointcloud(&self) -> Result<RustStreamItem<RustPointCloudFrame>, String> {
        match &self.payload {
            StreamPayload::PointCloud(f) => Ok(RustStreamItem {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptPointCloud source yielded a StreamItem with {} payload; \
                 the substream is mono-T — yield PointCloudFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }

    /// Convert to a `RustStreamItem<RustJointEncodersFrame>`. Errors
    /// with a human-readable detail if the payload is the wrong variant.
    pub(crate) fn to_rust_joint_encoders(
        &self,
    ) -> Result<RustStreamItem<RustJointEncodersFrame>, String> {
        match &self.payload {
            StreamPayload::JointEncoders(f) => Ok(RustStreamItem {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptJointEncoders source yielded a StreamItem with {} payload; \
                 the substream is mono-T — yield JointEncodersFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }

    /// Convert to a `RustStreamItem<RustAudioFrame>`. Errors with a
    /// human-readable detail if the payload is the wrong variant. Used
    /// by the producer-side source-stream pump for an `AcceptAudio`
    /// dispatch (Dialogue Batch 1).
    pub(crate) fn to_rust_audio(&self) -> Result<RustStreamItem<RustAudioFrame>, String> {
        match &self.payload {
            StreamPayload::Audio(f) => Ok(RustStreamItem {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptAudio source yielded a StreamItem with {} payload; \
                 the substream is mono-T — yield AudioFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }

    pub(crate) fn to_rust_scalar(&self) -> Result<RustStreamItem<RustScalarFrame>, String> {
        match &self.payload {
            StreamPayload::Scalar(f) => Ok(RustStreamItem {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptScalar source yielded a StreamItem with {} payload; \
                 the substream is mono-T — yield ScalarFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }

    /// Convert to a `RustStreamItem<RustPoseSpatialTransform>`. Errors
    /// with a human-readable detail if the payload is the wrong variant.
    /// Used by the producer-side source-stream pump for an `AcceptPose`
    /// dispatch.
    pub(crate) fn to_rust_pose(&self) -> Result<RustStreamItem<RustPoseSpatialTransform>, String> {
        match &self.payload {
            StreamPayload::Pose(f) => Ok(RustStreamItem {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptPose source yielded a StreamItem with {} payload; \
                 the substream is mono-T — yield SpatialTransformFrame or use the matching factory",
                other.kind_name(),
            )),
        }
    }

    pub(crate) fn to_rust_map(&self) -> Result<RustStreamItem<RustMapUpdate>, String> {
        match &self.payload {
            StreamPayload::Map(f) => Ok(RustStreamItem {
                timestamp_ns: self.timestamp_ns,
                payload: f.inner.clone(),
            }),
            other => Err(format!(
                "AcceptMap source yielded a StreamItem with {} payload; the substream is mono-T",
                other.kind_name(),
            )),
        }
    }
}

impl StreamPayload {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Camera(_) => "CameraFrame",
            Self::PointCloud(_) => "PointCloudFrame",
            Self::JointEncoders(_) => "JointEncodersFrame",
            Self::Audio(_) => "AudioFrame",
            Self::Scalar(_) => "ScalarFrame",
            Self::Detection(_) => "DetectionFrame",
            Self::Pose(_) => "SpatialTransformFrame",
            Self::Map(_) => "MapUpdateFrame",
        }
    }
}

// ─── StreamEntry ───────────────────────────────────────────────────────────

/// What the consumer reads off `StreamSubscription.entries()`. Same as
/// [`PyStreamItem`] but with the SDK-stamped `seq` exposed.
///
/// `payload` is whichever `T` the producer accepted with — `CameraFrame`
/// for an `accept_camera(...)` substream or `PointCloudFrame` for an
/// `accept_pointcloud(...)` substream. Each substream is mono-`T`, so a
/// given `StreamSubscription` only ever surfaces one payload variant.
#[pyclass(name = "StreamEntry", frozen)]
#[derive(Clone, Debug)]
pub struct PyStreamEntry {
    timestamp_ns: i64,
    seq: u64,
    payload: StreamPayload,
}

#[pymethods]
impl PyStreamEntry {
    #[getter]
    fn timestamp_ns(&self) -> i64 {
        self.timestamp_ns
    }

    #[getter]
    fn seq(&self) -> u64 {
        self.seq
    }

    /// The wrapped stream payload — either a `CameraFrame` or a
    /// `PointCloudFrame`. Returns a fresh Python object each call.
    #[getter]
    fn payload(&self, py: Python<'_>) -> PyObject {
        self.payload.clone().into_py(py)
    }

    fn __repr__(&self) -> String {
        format!(
            "StreamEntry(timestamp_ns={}, seq={}, payload={})",
            self.timestamp_ns,
            self.seq,
            self.payload.repr(),
        )
    }
}

impl PyStreamEntry {
    fn from_rust_camera(frame: RustStreamEntry<RustCameraFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: StreamPayload::Camera(PyCameraFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_pointcloud(frame: RustStreamEntry<RustPointCloudFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: StreamPayload::PointCloud(PyPointCloudFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_joint_encoders(frame: RustStreamEntry<RustJointEncodersFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: StreamPayload::JointEncoders(PyJointEncodersFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_audio(frame: RustStreamEntry<RustAudioFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: StreamPayload::Audio(PyAudioFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_scalar(frame: RustStreamEntry<RustScalarFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: StreamPayload::Scalar(PyScalarFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_detection(frame: RustStreamEntry<RustDetectionFrame>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: StreamPayload::Detection(PyDetectionFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_pose(frame: RustStreamEntry<RustPoseSpatialTransform>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: StreamPayload::Pose(PySpatialTransformFrame {
                inner: frame.payload,
            }),
        }
    }

    fn from_rust_map(frame: RustStreamEntry<RustMapUpdate>) -> Self {
        Self {
            timestamp_ns: frame.timestamp_ns,
            seq: frame.seq,
            payload: StreamPayload::Map(PyMapUpdateFrame {
                inner: frame.payload,
            }),
        }
    }
}

// ─── StreamDecision ──────────────────────────────────────────────────────────

/// Provider's accept/decline decision. Construct via the static
/// factories `accept_camera(manifest, source)`,
/// `accept_pointcloud(manifest, source)`,
/// `accept_joint_encoders(manifest, source)`, `accept_audio(manifest,
/// source)`, or `decline(reason)` — there is no public constructor.
///
/// `source` is **a Python async iterator yielding [`PyStreamItem`]
/// values**. Typically an `async def` generator; any object with
/// `__aiter__` / `__anext__` works. The SDK drains it on the wrapper's
/// asyncio loop; `finally` blocks fire when the SDK drops the iterator
/// (consumer disconnect → `aclose` driven through).
///
/// **Each substream is mono-`T`.** The `accept_camera` factory commits
/// to a `CameraFrame` substream — yielding a
/// `PointCloudFrame` ends the stream with `EndReason::ProducerError`.
/// Use `accept_pointcloud` for a `PointCloudFrame` substream.
#[pyclass(name = "StreamDecision", frozen)]
pub struct PyStreamDecision {
    pub(crate) inner: Mutex<Option<DecisionInner>>,
}

pub(crate) enum DecisionInner {
    AcceptCamera {
        manifest: PyStreamManifest,
        source: Py<PyAny>,
    },
    AcceptCameraRetained {
        manifest: PyStreamManifest,
        source: RustRetainedStreamSource,
    },
    AcceptPointCloud {
        manifest: PyStreamManifest,
        source: Py<PyAny>,
    },
    AcceptPointCloudRetained {
        manifest: PyStreamManifest,
        source: RustRetainedStreamSource,
    },
    AcceptJointEncoders {
        manifest: PyStreamManifest,
        source: Py<PyAny>,
    },
    AcceptJointEncodersRetained {
        manifest: PyStreamManifest,
        source: RustRetainedStreamSource,
    },
    AcceptAudio {
        manifest: PyStreamManifest,
        source: Py<PyAny>,
    },
    AcceptAudioRetained {
        manifest: PyStreamManifest,
        source: RustRetainedStreamSource,
    },
    AcceptScalar {
        manifest: PyStreamManifest,
        source: Py<PyAny>,
    },
    AcceptScalarRetained {
        manifest: PyStreamManifest,
        source: RustRetainedStreamSource,
    },
    AcceptPose {
        manifest: PyStreamManifest,
        source: Py<PyAny>,
    },
    AcceptMap {
        manifest: PyStreamManifest,
        source: Py<PyAny>,
    },
    AcceptMapRetained {
        manifest: PyStreamManifest,
        source: RustRetainedStreamSource,
    },
    Decline {
        reason: PyDeclineReason,
    },
}

#[pymethods]
impl PyStreamDecision {
    /// Accept the request with a camera source. The async iterator must
    /// yield `StreamItem(payload=CameraFrame(...))` values; yielding a
    /// `PointCloudFrame` ends the stream with `EndReason::ProducerError`.
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_camera(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptCamera { manifest, source })),
        }
    }

    /// Accept the request with a PointCloud source (Dagaz Batch 2). The
    /// async iterator must yield `StreamItem(payload=PointCloudFrame(...))`
    /// values carrying CDR-encoded `PointCloud2` ROS message bytes; the
    /// consumer (Park, future Sentinel) parses CDR on its side.
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_pointcloud(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptPointCloud { manifest, source })),
        }
    }

    /// Accept the request with a JointEncoders source (sawslin Phase B).
    /// The async iterator must yield
    /// `StreamItem(payload=JointEncodersFrame(angles_rad))` values;
    /// each `angles_rad` length must match the registry entry's
    /// `JointEncoders { joint_count }` (consumer-enforced; the SDK
    /// doesn't validate length on the wire).
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_joint_encoders(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptJointEncoders {
                manifest,
                source,
            })),
        }
    }

    /// Accept the request with an Audio source (Dialogue Batch 1). The
    /// async iterator must yield
    /// `StreamItem(payload=AudioFrame(data))` values carrying
    /// interleaved PCM bytes; sample format / channels / sample rate /
    /// channel layout are resolved out-of-band via
    /// `(sensor_id, sensor_hash) → SensorBody::Audio`, so the wire
    /// payload itself is opaque-bytes.
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_audio(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptAudio { manifest, source })),
        }
    }

    /// Accept the request with a non-spatial scalar source. The registry
    /// entry supplies the quantity and unit; each item carries a ScalarFrame.
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_scalar(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptScalar { manifest, source })),
        }
    }

    /// Accept the request with a pose source. The async iterator must
    /// yield `StreamItem(payload=SpatialTransformFrame(values))`
    /// values; frame-pair identity lives in the `StreamManifest`
    /// resource fields.
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_pose(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptPose { manifest, source })),
        }
    }

    /// Accept an SDK Map Log source yielding `MapUpdateFrame` values.
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_map(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::AcceptMap { manifest, source })),
        }
    }

    /// Accept the request with an SDK-owned retained log source created
    /// by `auki_logs.Log.stream_source(...)`. This is the recommended
    /// producer API for apps: the SDK builds the stream manifest,
    /// tails retained log bytes, decodes them into the correct payload
    /// type, and maps to the runtime's typed dispatch internally.
    #[staticmethod]
    #[pyo3(signature = (source, /))]
    fn accept_source(source: Bound<'_, PyAny>) -> PyResult<Self> {
        let retained = retained_stream_source_from_python(&source)?;
        let manifest = PyStreamManifest {
            inner: manifest_from_retained_source(&retained),
        };
        let inner = match retained.payload_kind.as_str() {
            "camera" => DecisionInner::AcceptCameraRetained {
                manifest,
                source: retained,
            },
            "pointcloud" => DecisionInner::AcceptPointCloudRetained {
                manifest,
                source: retained,
            },
            "joint_encoders" => DecisionInner::AcceptJointEncodersRetained {
                manifest,
                source: retained,
            },
            "audio" => DecisionInner::AcceptAudioRetained {
                manifest,
                source: retained,
            },
            "scalar" => DecisionInner::AcceptScalarRetained {
                manifest,
                source: retained,
            },
            "map" => DecisionInner::AcceptMapRetained {
                manifest,
                source: retained,
            },
            other => {
                return Err(PyValueError::new_err(format!(
                    "payload_kind must be one of camera, pointcloud, joint_encoders, audio, scalar, or map; got {other:?}"
                )));
            }
        };
        Ok(Self {
            inner: Mutex::new(Some(inner)),
        })
    }

    #[staticmethod]
    #[pyo3(signature = (reason, /))]
    fn decline(reason: PyDeclineReason) -> Self {
        Self {
            inner: Mutex::new(Some(DecisionInner::Decline { reason })),
        }
    }

    /// Discriminator: `"accept_camera"`, `"accept_pointcloud"`,
    /// `"accept_joint_encoders"`, `"accept_audio"`, `"accept_pose"`,
    /// `"decline"`, or `"consumed"` (post-`take`). Read-only
    /// inspection; the actual fields aren't exposed because the source
    /// iterator is consumed by the SDK exactly once.
    #[getter]
    fn kind(&self) -> &'static str {
        let guard = self.inner.lock().expect("PyStreamDecision mutex poisoned");
        match guard.as_ref() {
            Some(DecisionInner::AcceptCamera { .. })
            | Some(DecisionInner::AcceptCameraRetained { .. }) => "accept_camera",
            Some(DecisionInner::AcceptPointCloud { .. })
            | Some(DecisionInner::AcceptPointCloudRetained { .. }) => "accept_pointcloud",
            Some(DecisionInner::AcceptJointEncoders { .. })
            | Some(DecisionInner::AcceptJointEncodersRetained { .. }) => "accept_joint_encoders",
            Some(DecisionInner::AcceptAudio { .. })
            | Some(DecisionInner::AcceptAudioRetained { .. }) => "accept_audio",
            Some(DecisionInner::AcceptScalar { .. })
            | Some(DecisionInner::AcceptScalarRetained { .. }) => "accept_scalar",
            Some(DecisionInner::AcceptPose { .. }) => "accept_pose",
            Some(DecisionInner::AcceptMap { .. })
            | Some(DecisionInner::AcceptMapRetained { .. }) => "accept_map",
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
/// `Callable[[str, StreamRequest], StreamDecision]`. Used by
/// `cluster.spawn` when the consumer passes `stream_provider=...`.
///
/// The first argument is the requester's libp2p PeerId rendered as a
/// Python `str` (matching every other peer-id surface in the Python
/// API). Producers can use it for per-requester policy — Park's
/// Dialogue audio is the load-bearing example: with N robots in one
/// cluster the operator's mic must only stream to the one robot
/// they are currently inspecting.
///
/// Maps the Python [`PyStreamDecision`]'s [`DecisionInner`] variants
/// (`AcceptCamera`, `AcceptPointCloud`, `Decline`) onto the matching
/// Rust [`RustStreamDispatch`] variant. Each substream is mono-`T`;
/// the `T` is decided here by which factory the Python provider used
/// (`accept_camera` → `AcceptCamera`,
/// `accept_pointcloud` → `AcceptPointCloud`).
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
/// `PyStreamDecision` / `PyStreamManifest` / `PyDeclineReason` pyclass
/// plumbing. Promoted 2026-05-13.
pub fn build_stream_provider(callable: Py<PyAny>) -> StreamProvider {
    Arc::new(
        move |peer: libp2p_identity::PeerId, request: RustStreamRequest| {
            let read_from = request.from;
            let py_request = PyStreamRequest { inner: request };
            let peer_str = peer.to_string();

            // Step 1 (under GIL): call the Python provider, extract a
            // PyStreamDecision (or normalize errors to a Decline).
            let decision_or_err: Result<DecisionInner, String> = Python::with_gil(|py| {
                let result = match callable.call1(py, (peer_str.clone(), py_request.clone())) {
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
                Ok(DecisionInner::AcceptCamera { manifest, source }) => {
                    let source_stream =
                        python_iter_into_source_stream::<RustCameraFrame>(source, |pf| {
                            pf.to_rust_camera()
                        });
                    RustStreamDispatch::AcceptCamera {
                        manifest: manifest.inner,
                        source: source_stream,
                    }
                }
                Ok(DecisionInner::AcceptCameraRetained { manifest, source }) => {
                    match retained_log_into_source_stream(
                        source,
                        decode_retained_camera,
                        &read_from,
                    ) {
                        Ok(source) => RustStreamDispatch::AcceptCamera {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(e) => RustStreamDispatch::Decline {
                            reason: RustDeclineReason::other(e.to_string()),
                        },
                    }
                }
                Ok(DecisionInner::AcceptPointCloud { manifest, source }) => {
                    let source_stream =
                        python_iter_into_source_stream::<RustPointCloudFrame>(source, |pf| {
                            pf.to_rust_pointcloud()
                        });
                    RustStreamDispatch::AcceptPointCloud {
                        manifest: manifest.inner,
                        source: source_stream,
                    }
                }
                Ok(DecisionInner::AcceptPointCloudRetained { manifest, source }) => {
                    match retained_log_into_source_stream(
                        source,
                        decode_retained_pointcloud,
                        &read_from,
                    ) {
                        Ok(source) => RustStreamDispatch::AcceptPointCloud {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(e) => RustStreamDispatch::Decline {
                            reason: RustDeclineReason::other(e.to_string()),
                        },
                    }
                }
                Ok(DecisionInner::AcceptJointEncoders { manifest, source }) => {
                    let source_stream =
                        python_iter_into_source_stream::<RustJointEncodersFrame>(source, |pf| {
                            pf.to_rust_joint_encoders()
                        });
                    RustStreamDispatch::AcceptJointEncoders {
                        manifest: manifest.inner,
                        source: source_stream,
                    }
                }
                Ok(DecisionInner::AcceptJointEncodersRetained { manifest, source }) => {
                    match retained_log_into_source_stream(
                        source,
                        decode_retained_joint_encoders,
                        &read_from,
                    ) {
                        Ok(source) => RustStreamDispatch::AcceptJointEncoders {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(e) => RustStreamDispatch::Decline {
                            reason: RustDeclineReason::other(e.to_string()),
                        },
                    }
                }
                Ok(DecisionInner::AcceptAudio { manifest, source }) => {
                    let source_stream =
                        python_iter_into_source_stream::<RustAudioFrame>(source, |pf| {
                            pf.to_rust_audio()
                        });
                    RustStreamDispatch::AcceptAudio {
                        manifest: manifest.inner,
                        source: source_stream,
                    }
                }
                Ok(DecisionInner::AcceptAudioRetained { manifest, source }) => {
                    match retained_log_into_source_stream(source, decode_retained_audio, &read_from)
                    {
                        Ok(source) => RustStreamDispatch::AcceptAudio {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(e) => RustStreamDispatch::Decline {
                            reason: RustDeclineReason::other(e.to_string()),
                        },
                    }
                }
                Ok(DecisionInner::AcceptScalar { manifest, source }) => {
                    let source_stream =
                        python_iter_into_source_stream::<RustScalarFrame>(source, |pf| {
                            pf.to_rust_scalar()
                        });
                    RustStreamDispatch::AcceptScalar {
                        manifest: manifest.inner,
                        source: source_stream,
                    }
                }
                Ok(DecisionInner::AcceptScalarRetained { manifest, source }) => {
                    match retained_log_into_source_stream(
                        source,
                        decode_retained_scalar,
                        &read_from,
                    ) {
                        Ok(source) => RustStreamDispatch::AcceptScalar {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(e) => RustStreamDispatch::Decline {
                            reason: RustDeclineReason::other(e.to_string()),
                        },
                    }
                }
                Ok(DecisionInner::AcceptPose { manifest, source }) => {
                    let source_stream =
                        python_iter_into_source_stream::<RustPoseSpatialTransform>(source, |pf| {
                            pf.to_rust_pose()
                        });
                    RustStreamDispatch::AcceptPose {
                        manifest: manifest.inner,
                        source: source_stream,
                    }
                }
                Ok(DecisionInner::AcceptMap { manifest, source }) => {
                    let source_stream =
                        python_iter_into_source_stream::<RustMapUpdate>(source, |pf| {
                            pf.to_rust_map()
                        });
                    RustStreamDispatch::AcceptMap {
                        manifest: manifest.inner,
                        source: source_stream,
                    }
                }
                Ok(DecisionInner::AcceptMapRetained { manifest, source }) => {
                    match retained_log_into_source_stream(source, decode_retained_map, &read_from) {
                        Ok(source) => RustStreamDispatch::AcceptMap {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(e) => RustStreamDispatch::Decline {
                            reason: RustDeclineReason::other(e.to_string()),
                        },
                    }
                }
            }
        },
    )
}

fn retained_stream_source_from_python(
    source: &Bound<'_, PyAny>,
) -> PyResult<RustRetainedStreamSource> {
    let capsule_obj = source.call_method0("_stream_source_capsule").map_err(|e| {
        PyValueError::new_err(format!(
            "accept_source expects auki_logs.Log.stream_source(...) object: {e}"
        ))
    })?;
    let capsule = capsule_obj.downcast::<PyCapsule>().map_err(|e| {
        PyRuntimeError::new_err(format!("stream source bridge returned non-PyCapsule: {e}"))
    })?;
    let expected_name =
        CString::new(STREAM_SOURCE_CAPSULE_NAME).expect("static literal contains no nul");
    match capsule.name()? {
        Some(name) if name == expected_name.as_c_str() => {}
        Some(other) => {
            return Err(PyRuntimeError::new_err(format!(
                "stream-source capsule has unexpected name {other:?} (want {STREAM_SOURCE_CAPSULE_NAME:?})"
            )));
        }
        None => {
            return Err(PyRuntimeError::new_err(
                "stream-source capsule has no name; rejecting",
            ));
        }
    }
    // SAFETY: the capsule name pins the payload ABI to
    // `RustRetainedStreamSource`, and `auki-network-py` depends on the
    // same `auki-logs-py` rlib that created it.
    let source_ref: &RustRetainedStreamSource =
        unsafe { capsule.reference::<RustRetainedStreamSource>() };
    Ok(source_ref.clone())
}

fn manifest_from_retained_source(source: &RustRetainedStreamSource) -> RustStreamManifest {
    if source.payload_kind == "map" {
        return RustStreamManifest {
            resource_id: source.resource_id.clone(),
            payload: "map_update".into(),
            map_peer_id: source.map_peer_id.clone(),
            map_id: source.map_id.clone(),
            map_hash: source.map_hash.clone(),
            clock_peer_id: source.clock_peer_id.clone(),
            clock_id: source.clock_id.clone(),
            clock_hash: source.clock_hash.clone(),
            ..Default::default()
        };
    }
    RustStreamManifest {
        sensor_id: source.sensor_id.clone(),
        sensor_hash: source.sensor_hash.clone(),
        clock_id: source.clock_id.clone(),
        clock_hash: source.clock_hash.clone(),
        frame_id: source.frame_id.clone(),
        frame_hash: source.frame_hash.clone(),
        ..Default::default()
    }
}

fn log_error_to_string(e: RustLogError) -> String {
    e.to_string()
}

fn retained_log_into_source_stream<T>(
    source: RustRetainedStreamSource,
    decode: fn(Vec<u8>) -> Result<T, String>,
    read_from: &RustReadFrom,
) -> PyResult<SourceStream<T>>
where
    T: Send + 'static,
{
    let tail = RustLog::<RustRawLogBytes>::tail(&source.root)
        .map_err(|e| PyRuntimeError::new_err(format!("stream source tail: {e}")))?;
    let historical = match read_from {
        RustReadFrom::Latest => Vec::new(),
        RustReadFrom::FromStart => RustLog::<RustRawLogBytes>::read(&source.root)
            .and_then(|reader| reader.entries())
            .map_err(|e| PyRuntimeError::new_err(format!("stream source read: {e}")))?,
        RustReadFrom::FromTimestamp(start_ns) => RustLog::<RustRawLogBytes>::read(&source.root)
            .and_then(|reader| reader.entries())
            .map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| entry.timestamp_ns >= *start_ns)
                    .collect()
            })
            .map_err(|e| PyRuntimeError::new_err(format!("stream source read: {e}")))?,
    };
    let state = RetainedSourceState {
        historical: historical.into(),
        tail: Some(tail),
    };

    let stream = futures::stream::unfold(state, move |mut state| async move {
        if let Some(entry) = state.historical.pop_front() {
            let frame = decode(entry.payload.0).map(|payload| RustStreamItem {
                timestamp_ns: entry.timestamp_ns,
                payload,
            });
            return Some((frame, state));
        }

        let Some(tail) = state.tail.take() else {
            return None;
        };
        let joined = crate::cluster_tokio_runtime()
            .spawn_blocking(move || {
                let mut tail = tail;
                let next = tail.next();
                (tail, next)
            })
            .await;

        let (tail, next) = match joined {
            Ok(pair) => pair,
            Err(e) => {
                return Some((Err(format!("stream source tail task failed: {e}")), state));
            }
        };
        state.tail = Some(tail);

        match next {
            Some(Ok(entry)) => {
                let frame = decode(entry.payload.0).map(|payload| RustStreamItem {
                    timestamp_ns: entry.timestamp_ns,
                    payload,
                });
                Some((frame, state))
            }
            Some(Err(e)) => Some((Err(log_error_to_string(e)), state)),
            None => None,
        }
    });
    Ok(Box::pin(stream))
}

struct RetainedSourceState {
    historical: VecDeque<auki_logs_rs::Entry<RustRawLogBytes>>,
    tail: Option<auki_logs_rs::TailIter<RustRawLogBytes>>,
}

fn decode_retained_camera(bytes: Vec<u8>) -> Result<RustCameraFrame, String> {
    RustCameraFrame::decode(&*bytes).map_err(|e| e.to_string())
}

fn decode_retained_map(bytes: Vec<u8>) -> Result<RustMapUpdate, String> {
    RustMapUpdate::decode(&*bytes).map_err(|e| e.to_string())
}

fn decode_retained_pointcloud(bytes: Vec<u8>) -> Result<RustPointCloudFrame, String> {
    RustPointCloudFrame::decode(&*bytes).map_err(|e| e.to_string())
}

fn decode_retained_joint_encoders(bytes: Vec<u8>) -> Result<RustJointEncodersFrame, String> {
    RustJointEncodersFrame::decode(&*bytes).map_err(|e| e.to_string())
}

fn decode_retained_audio(bytes: Vec<u8>) -> Result<RustAudioFrame, String> {
    RustAudioFrame::decode(&*bytes).map_err(|e| e.to_string())
}

fn decode_retained_scalar(bytes: Vec<u8>) -> Result<RustScalarFrame, String> {
    RustScalarFrame::decode(&*bytes).map_err(|e| e.to_string())
}

/// Convert a Python async iterator (yielding `PyStreamItem`) into a
/// Rust [`SourceStream<T>`] the SDK can drain. The `convert` callback
/// extracts the per-substream typed payload from each yielded
/// [`PyStreamItem`] — `CameraFrame` for an `AcceptCamera` dispatch,
/// `PointCloudFrame` for `AcceptPointCloud`. Yielding a frame with the
/// wrong payload variant produces `Some(Err("..."))`, which the SDK
/// converts into [`auki_network::stream_protocol::EndReason::ProducerError`]
/// on the wire and ends the stream.
///
/// Type contract: each yielded Python value must extract as
/// [`PyStreamItem`]. Anything else maps to `Some(Err("..."))` with
/// the same end-of-stream effect.
///
/// Lifetime / cleanup: the bridge is held inside [`SourceStreamGuard`].
/// On natural end (`StopAsyncIteration` or first error) we explicitly
/// fire `aclose` on the iterator before yielding `None`. On unnatural
/// drop (consumer disconnect mid-stream → SDK drops the `SourceStream`),
/// `Drop` on [`SourceStreamGuard`] schedules a fire-and-forget `aclose`
/// task on the wrapper's tokio runtime so the generator's `finally`
/// block fires promptly rather than waiting for asyncio's gc hooks.
/// The returned stream is fused so polls after natural termination keep
/// returning `None` without re-entering the Python bridge.
fn python_iter_into_source_stream<T>(
    aiter: Py<PyAny>,
    convert: fn(&PyStreamItem) -> Result<RustStreamItem<T>, String>,
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
                // StreamItem<T> using the substream-typed `convert`.
                let result = Python::with_gil(|py| -> Result<RustStreamItem<T>, String> {
                    let bound = value.bind(py);
                    match bound.extract::<PyRef<PyStreamItem>>() {
                        Ok(pf) => convert(&pf),
                        Err(_) => Err(format!(
                            "stream_provider source must yield StreamItem; got {}",
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
    Box::pin(stream.fuse())
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

// ─── StreamSubscription + StreamEntryIterator ──────────────────────────────────────

type RustCameraFrameStream =
    Pin<Box<dyn Stream<Item = Result<RustStreamEntry<RustCameraFrame>, RustStreamError>> + Send>>;
type RustPointCloudFrameStream = Pin<
    Box<dyn Stream<Item = Result<RustStreamEntry<RustPointCloudFrame>, RustStreamError>> + Send>,
>;
type RustJointEncodersFrameStream = Pin<
    Box<dyn Stream<Item = Result<RustStreamEntry<RustJointEncodersFrame>, RustStreamError>> + Send>,
>;
type RustAudioFrameStream =
    Pin<Box<dyn Stream<Item = Result<RustStreamEntry<RustAudioFrame>, RustStreamError>> + Send>>;
type RustScalarFrameStream =
    Pin<Box<dyn Stream<Item = Result<RustStreamEntry<RustScalarFrame>, RustStreamError>> + Send>>;
type RustDetectionFrameStream = Pin<
    Box<dyn Stream<Item = Result<RustStreamEntry<RustDetectionFrame>, RustStreamError>> + Send>,
>;
type RustPoseFrameStream = Pin<
    Box<
        dyn Stream<Item = Result<RustStreamEntry<RustPoseSpatialTransform>, RustStreamError>>
            + Send,
    >,
>;
type RustMapFrameStream =
    Pin<Box<dyn Stream<Item = Result<RustStreamEntry<RustMapUpdate>, RustStreamError>> + Send>>;

/// Tagged union over the underlying typed entry stream the SDK returns
/// for an open subscription. Each substream is mono-`T`, so the
/// variant is fixed at `open_stream` time and never changes for the
/// lifetime of the subscription.
enum EntryStreamKind {
    Camera(RustCameraFrameStream),
    PointCloud(RustPointCloudFrameStream),
    JointEncoders(RustJointEncodersFrameStream),
    Audio(RustAudioFrameStream),
    Scalar(RustScalarFrameStream),
    Detection(RustDetectionFrameStream),
    Pose(RustPoseFrameStream),
    Map(RustMapFrameStream),
}

/// What `runtime.open_stream` / `runtime.open_pointcloud_stream`
/// returns on a successful Accept. Carries the producer's
/// [`PyStreamManifest`]; iterate over entries via `subscription.entries()`.
///
/// The entries iterator can be fetched **at most once**. A second call
/// raises `RuntimeError` — the underlying Rust `Stream` is single-use.
#[pyclass(name = "StreamSubscription")]
pub struct PyStreamSubscription {
    manifest: PyStreamManifest,
    entries: Mutex<Option<EntryStreamKind>>,
}

#[pymethods]
impl PyStreamSubscription {
    /// Accept-time stream manifest committed by the producer.
    #[getter]
    fn manifest(&self) -> PyStreamManifest {
        self.manifest.clone()
    }

    /// Drain the entry iterator (sync, blocking). Each `__next__()` blocks
    /// until the next entry arrives over the substream. Stream-end
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
    fn entries(&self) -> PyResult<PyStreamEntryIterator> {
        let mut guard = self
            .entries
            .lock()
            .expect("StreamSubscription mutex poisoned");
        let frames = guard.take().ok_or_else(|| {
            PyRuntimeError::new_err("StreamSubscription.entries() can only be called once")
        })?;
        Ok(PyStreamEntryIterator {
            entries: Mutex::new(Some(frames)),
        })
    }

    fn __repr__(&self) -> String {
        format!("StreamSubscription(manifest={})", self.manifest.__repr__())
    }
}

impl PyStreamSubscription {
    pub fn from_rust_camera(rust_sub: RustStreamSubscription<RustCameraFrame>) -> Self {
        Self {
            manifest: PyStreamManifest {
                inner: rust_sub.manifest,
            },
            entries: Mutex::new(Some(EntryStreamKind::Camera(rust_sub.entries))),
        }
    }

    pub fn from_rust_pointcloud(rust_sub: RustStreamSubscription<RustPointCloudFrame>) -> Self {
        Self {
            manifest: PyStreamManifest {
                inner: rust_sub.manifest,
            },
            entries: Mutex::new(Some(EntryStreamKind::PointCloud(rust_sub.entries))),
        }
    }

    pub fn from_rust_joint_encoders(
        rust_sub: RustStreamSubscription<RustJointEncodersFrame>,
    ) -> Self {
        Self {
            manifest: PyStreamManifest {
                inner: rust_sub.manifest,
            },
            entries: Mutex::new(Some(EntryStreamKind::JointEncoders(rust_sub.entries))),
        }
    }

    pub fn from_rust_audio(rust_sub: RustStreamSubscription<RustAudioFrame>) -> Self {
        Self {
            manifest: PyStreamManifest {
                inner: rust_sub.manifest,
            },
            entries: Mutex::new(Some(EntryStreamKind::Audio(rust_sub.entries))),
        }
    }

    pub fn from_rust_scalar(rust_sub: RustStreamSubscription<RustScalarFrame>) -> Self {
        Self {
            manifest: PyStreamManifest {
                inner: rust_sub.manifest,
            },
            entries: Mutex::new(Some(EntryStreamKind::Scalar(rust_sub.entries))),
        }
    }

    pub fn from_rust_detection(rust_sub: RustStreamSubscription<RustDetectionFrame>) -> Self {
        Self {
            manifest: PyStreamManifest {
                inner: rust_sub.manifest,
            },
            entries: Mutex::new(Some(EntryStreamKind::Detection(rust_sub.entries))),
        }
    }

    pub fn from_rust_pose(rust_sub: RustStreamSubscription<RustPoseSpatialTransform>) -> Self {
        Self {
            manifest: PyStreamManifest {
                inner: rust_sub.manifest,
            },
            entries: Mutex::new(Some(EntryStreamKind::Pose(rust_sub.entries))),
        }
    }

    pub fn from_rust_map(rust_sub: RustStreamSubscription<RustMapUpdate>) -> Self {
        Self {
            manifest: PyStreamManifest {
                inner: rust_sub.manifest,
            },
            entries: Mutex::new(Some(EntryStreamKind::Map(rust_sub.entries))),
        }
    }
}

/// Sync iterator over a [`PyStreamSubscription`]'s entries. Each
/// `__next__()` blocks the caller's thread on the wrapper's tokio
/// runtime until the next entry arrives. The substream's payload `T`
/// is fixed at open time; `__next__` dispatches on the stored
/// [`EntryStreamKind`] variant.
#[pyclass(name = "StreamEntryIterator")]
pub struct PyStreamEntryIterator {
    entries: Mutex<Option<EntryStreamKind>>,
}

/// Internal: result of polling one item out of either typed stream.
/// Used to keep the per-T monomorphization inside the `block_on`
/// closure and convert to a [`PyStreamEntry`] with the GIL held
/// afterwards.
enum EntryNext {
    Camera(Result<RustStreamEntry<RustCameraFrame>, RustStreamError>),
    PointCloud(Result<RustStreamEntry<RustPointCloudFrame>, RustStreamError>),
    JointEncoders(Result<RustStreamEntry<RustJointEncodersFrame>, RustStreamError>),
    Audio(Result<RustStreamEntry<RustAudioFrame>, RustStreamError>),
    Scalar(Result<RustStreamEntry<RustScalarFrame>, RustStreamError>),
    Detection(Result<RustStreamEntry<RustDetectionFrame>, RustStreamError>),
    Pose(Result<RustStreamEntry<RustPoseSpatialTransform>, RustStreamError>),
    Map(Result<RustStreamEntry<RustMapUpdate>, RustStreamError>),
    Done,
}

#[pymethods]
impl PyStreamEntryIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Block until the next entry arrives. See
    /// [`PyStreamSubscription::entries`] docstring for end-of-stream
    /// signalling.
    fn __next__(&self, py: Python<'_>) -> PyResult<PyStreamEntry> {
        // Pull the stream out of the mutex for the duration of the
        // poll. Releasing the GIL while we block lets other Python
        // threads run (the wrapper's asyncio loop thread, e.g. for a
        // simultaneous `stream_provider` invocation on the same
        // process).
        let stream_taken = {
            let mut guard = self
                .entries
                .lock()
                .expect("StreamEntryIterator mutex poisoned");
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
                    EntryStreamKind::Camera(s) => match s.next().await {
                        Some(item) => EntryNext::Camera(item),
                        None => EntryNext::Done,
                    },
                    EntryStreamKind::PointCloud(s) => match s.next().await {
                        Some(item) => EntryNext::PointCloud(item),
                        None => EntryNext::Done,
                    },
                    EntryStreamKind::JointEncoders(s) => match s.next().await {
                        Some(item) => EntryNext::JointEncoders(item),
                        None => EntryNext::Done,
                    },
                    EntryStreamKind::Audio(s) => match s.next().await {
                        Some(item) => EntryNext::Audio(item),
                        None => EntryNext::Done,
                    },
                    EntryStreamKind::Scalar(s) => match s.next().await {
                        Some(item) => EntryNext::Scalar(item),
                        None => EntryNext::Done,
                    },
                    EntryStreamKind::Detection(s) => match s.next().await {
                        Some(item) => EntryNext::Detection(item),
                        None => EntryNext::Done,
                    },
                    EntryStreamKind::Pose(s) => match s.next().await {
                        Some(item) => EntryNext::Pose(item),
                        None => EntryNext::Done,
                    },
                    EntryStreamKind::Map(s) => match s.next().await {
                        Some(item) => EntryNext::Map(item),
                        None => EntryNext::Done,
                    },
                }
            })
        });

        match item {
            EntryNext::Camera(Ok(frame)) => {
                let mut guard = self
                    .entries
                    .lock()
                    .expect("StreamEntryIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyStreamEntry::from_rust_camera(frame))
            }
            EntryNext::PointCloud(Ok(frame)) => {
                let mut guard = self
                    .entries
                    .lock()
                    .expect("StreamEntryIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyStreamEntry::from_rust_pointcloud(frame))
            }
            EntryNext::JointEncoders(Ok(frame)) => {
                let mut guard = self
                    .entries
                    .lock()
                    .expect("StreamEntryIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyStreamEntry::from_rust_joint_encoders(frame))
            }
            EntryNext::Audio(Ok(frame)) => {
                let mut guard = self
                    .entries
                    .lock()
                    .expect("StreamEntryIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyStreamEntry::from_rust_audio(frame))
            }
            EntryNext::Scalar(Ok(frame)) => {
                let mut guard = self
                    .entries
                    .lock()
                    .expect("StreamEntryIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyStreamEntry::from_rust_scalar(frame))
            }
            EntryNext::Detection(Ok(frame)) => {
                let mut guard = self
                    .entries
                    .lock()
                    .expect("StreamEntryIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyStreamEntry::from_rust_detection(frame))
            }
            EntryNext::Pose(Ok(frame)) => {
                let mut guard = self
                    .entries
                    .lock()
                    .expect("StreamEntryIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyStreamEntry::from_rust_pose(frame))
            }
            EntryNext::Map(Ok(frame)) => {
                let mut guard = self
                    .entries
                    .lock()
                    .expect("StreamEntryIterator mutex poisoned");
                *guard = Some(stream);
                Ok(PyStreamEntry::from_rust_map(frame))
            }
            EntryNext::Camera(Err(stream_err))
            | EntryNext::PointCloud(Err(stream_err))
            | EntryNext::JointEncoders(Err(stream_err))
            | EntryNext::Audio(Err(stream_err))
            | EntryNext::Scalar(Err(stream_err))
            | EntryNext::Detection(Err(stream_err))
            | EntryNext::Pose(Err(stream_err))
            | EntryNext::Map(Err(stream_err)) => {
                // Terminator. Don't put the stream back — exhausted.
                Err(stream_error_to_pyerr(py, stream_err))
            }
            EntryNext::Done => Err(PyStopIteration::new_err(())),
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
    cluster.add_class::<PyStreamManifest>()?;
    cluster.add_class::<PyDynamicIntrinsics>()?;
    cluster.add_class::<PyCameraFrame>()?;
    cluster.add_class::<PyPointCloudFrame>()?;
    cluster.add_class::<PyJointEncodersFrame>()?;
    cluster.add_class::<PyAudioFrame>()?;
    cluster.add_class::<PyScalarFrame>()?;
    cluster.add_class::<PyDetectionFrame>()?;
    cluster.add_class::<PySpatialTransformFrame>()?;
    cluster.add_class::<PyMapUpdateFrame>()?;
    cluster.add_class::<PyDeclineReason>()?;
    cluster.add_class::<PyEndReason>()?;
    cluster.add_class::<PyStreamItem>()?;
    cluster.add_class::<PyStreamEntry>()?;
    cluster.add_class::<PyStreamDecision>()?;
    cluster.add_class::<PyStreamSubscription>()?;
    cluster.add_class::<PyStreamEntryIterator>()?;

    cluster.add_function(wrap_pyfunction!(_build_stream_provider, cluster)?)?;

    cluster.add(
        "StreamEndOfStream",
        py.get_type_bound::<StreamEndOfStream>(),
    )?;
    cluster.add(
        "StreamConnectionLost",
        py.get_type_bound::<StreamConnectionLost>(),
    )?;
    cluster.add(
        "StreamProtocolError",
        py.get_type_bound::<StreamProtocolError>(),
    )?;
    cluster.add("StreamDeclined", py.get_type_bound::<StreamDeclined>())?;
    cluster.add(
        "StreamUnreachable",
        py.get_type_bound::<StreamUnreachable>(),
    )?;

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
    use auki_datatypes::audio::Data as AudioData;
    use auki_datatypes::camera::CameraFrame;
    use auki_datatypes::joint_encoders::Data as JointEncodersData;
    use auki_datatypes::point_cloud::Data as PointCloudData;
    use auki_datatypes::pose::SpatialTransform as PoseSpatialTransform;
    use auki_logs_rs::Log as RawLog;
    use auki_network_rs::PeerIdentity;
    use auki_network_rs::stream_runtime::OPEN_STREAM_TIMEOUT;
    use prost::Message;
    use serde_json::json;

    fn test_peer_id() -> libp2p_identity::PeerId {
        PeerIdentity::from_seed(&[23u8; 32]).peer_id()
    }

    fn raw_log_manifest() -> serde_json::Value {
        json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 10_000_000_000i64,
            "kind": "test",
        })
    }

    fn retained_source_for(root: &std::path::Path, payload_kind: &str) -> RustRetainedStreamSource {
        RustRetainedStreamSource {
            root: root.to_path_buf(),
            resource_id: String::new(),
            sensor_id: format!("robot/{payload_kind}"),
            sensor_hash: "sensor-hash".into(),
            map_peer_id: String::new(),
            map_id: String::new(),
            map_hash: String::new(),
            clock_peer_id: String::new(),
            clock_id: "robot/clock".into(),
            clock_hash: "clock-hash".into(),
            payload_kind: payload_kind.into(),
            frame_id: "robot/base".into(),
            frame_hash: "frame-hash".into(),
        }
    }

    fn append_raw_payload(root: &std::path::Path, timestamp_ns: i64, payload: Vec<u8>) {
        let mut log = RawLog::<RustRawLogBytes>::open(root, raw_log_manifest()).unwrap();
        log.append(timestamp_ns, &RustRawLogBytes(payload)).unwrap();
        log.flush().unwrap();
    }

    #[test]
    fn exhausted_python_source_remains_terminated() {
        pyo3::prepare_freethreaded_python();
        let _ = crate::stream_bridge::asyncio_locals();
        for attempt in 0..3 {
            let module_name = format!("test_empty_source_{attempt}");
            let aiter = Python::with_gil(|py| {
                let module = PyModule::from_code_bound(
                    py,
                    "async def _gen():\n    if False:\n        yield None\n",
                    "test_empty_source.py",
                    &module_name,
                )?;
                Ok::<Py<PyAny>, PyErr>(module.getattr("_gen")?.call0()?.unbind())
            })
            .unwrap();
            let mut source = python_iter_into_source_stream::<RustCameraFrame>(aiter, |_| {
                unreachable!("empty source must not invoke conversion")
            });

            crate::cluster_tokio_runtime().block_on(async {
                assert!(source.next().await.is_none());
                assert!(source.next().await.is_none());
            });
        }
    }

    #[test]
    fn dropping_python_source_closes_suspended_generator() {
        pyo3::prepare_freethreaded_python();
        let _ = crate::stream_bridge::asyncio_locals();
        let (aiter, module) = Python::with_gil(|py| {
            let item = PyStreamItem::new(123, camera_frame_as_any(py, b"jpeg"))?;
            let item = Py::new(py, item)?;
            let module = PyModule::from_code_bound(
                py,
                "closed = False\n\
                 async def _gen(item):\n\
                 \x20   global closed\n\
                 \x20   try:\n\
                 \x20       yield item\n\
                 \x20   finally:\n\
                 \x20       closed = True\n",
                "test_dropped_source.py",
                "test_dropped_source",
            )?;
            let aiter = module.getattr("_gen")?.call1((item,))?.unbind();
            Ok::<_, PyErr>((aiter, module.unbind()))
        })
        .unwrap();
        let mut source =
            python_iter_into_source_stream::<RustCameraFrame>(aiter, PyStreamItem::to_rust_camera);

        crate::cluster_tokio_runtime().block_on(async {
            let first = source
                .next()
                .await
                .expect("generator should yield one item")
                .expect("camera item should convert");
            assert_eq!(first.payload.frame, b"jpeg");
            drop(source);

            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                loop {
                    let closed = Python::with_gil(|py| {
                        module
                            .bind(py)
                            .getattr("closed")
                            .and_then(|value| value.extract::<bool>())
                    })
                    .expect("read generator cleanup flag");
                    if closed {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("dropping source should aclose its Python generator");
        });
    }

    #[test]
    fn stream_request_round_trips() {
        Python::with_gil(|_py| {
            let r = PyStreamRequest::new("K1-AABB/head_left_cam".into(), "galbot".into());
            assert_eq!(r.resource_id(), "K1-AABB/head_left_cam");
            assert_eq!(r.source_peer_id(), "galbot");
            assert_eq!(
                r.__repr__(),
                r#"StreamRequest(resource_id="K1-AABB/head_left_cam", source_peer_id="galbot")"#,
            );
        });
    }

    #[test]
    fn stream_manifest_round_trips_and_compares() {
        Python::with_gil(|_py| {
            let a = PyStreamManifest::new(
                "sensor".into(),
                "h".into(),
                "c".into(),
                "ch".into(),
                Some("frame".into()),
                Some("fh".into()),
            );
            let b = PyStreamManifest::new(
                "sensor".into(),
                "h".into(),
                "c".into(),
                "ch".into(),
                Some("frame".into()),
                Some("fh".into()),
            );
            assert_eq!(a.sensor_id(), "sensor");
            assert_eq!(a.sensor_hash(), "h");
            assert_eq!(a.clock_id(), "c");
            assert_eq!(a.clock_hash(), "ch");
            assert_eq!(a.frame_id(), "frame");
            assert_eq!(a.frame_hash(), "fh");
            assert!(a.__eq__(&b));
            let c = PyStreamManifest::new(
                "sensor".into(),
                "other".into(),
                "c".into(),
                "ch".into(),
                Some("frame".into()),
                Some("fh".into()),
            );
            assert!(!a.__eq__(&c));
        });
    }

    #[test]
    fn camera_frame_round_trips_through_pybytes() {
        Python::with_gil(|py| {
            let payload = PyBytes::new_bound(py, &[0xff, 0xd8, 0x01, 0x02, 0x03]);
            let intrinsics =
                PyDynamicIntrinsics::new(400.0, 401.0, 272.5, 244.5, Some(vec![0.1, -0.2]));
            let f = PyCameraFrame::new(payload, Some(intrinsics.clone()));
            assert_eq!(f.__len__(), 5);
            // Round-trip the bytes back out.
            let out = f.frame(py);
            assert_eq!(out.as_bytes(), &[0xff, 0xd8, 0x01, 0x02, 0x03]);
            assert_eq!(f.dynamic_intrinsics(), Some(intrinsics));
        });
    }

    #[test]
    fn retained_camera_source_decodes_historical_log_payload() {
        let dir = tempfile::tempdir().unwrap();
        let entry = CameraFrame {
            dynamic_intrinsics: None,
            frame: b"jpeg".to_vec(),
        };
        append_raw_payload(dir.path(), 100, entry.encode_to_vec());

        let mut stream = retained_log_into_source_stream(
            retained_source_for(dir.path(), "camera"),
            decode_retained_camera,
            &RustReadFrom::FromStart,
        )
        .unwrap();
        let got = crate::cluster_tokio_runtime()
            .block_on(async { stream.next().await })
            .unwrap()
            .unwrap();

        assert_eq!(got.timestamp_ns, 100);
        assert_eq!(got.payload.frame, b"jpeg");
    }

    #[test]
    fn retained_latest_source_skips_existing_log_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = RawLog::<RustRawLogBytes>::open(dir.path(), raw_log_manifest()).unwrap();
        let old_entry = CameraFrame {
            dynamic_intrinsics: None,
            frame: b"old-jpeg".to_vec(),
        };
        log.append(100, &RustRawLogBytes(old_entry.encode_to_vec()))
            .unwrap();
        log.flush().unwrap();

        let mut stream = retained_log_into_source_stream(
            retained_source_for(dir.path(), "camera"),
            decode_retained_camera,
            &RustReadFrom::Latest,
        )
        .unwrap();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let new_entry = CameraFrame {
                dynamic_intrinsics: None,
                frame: b"new-jpeg".to_vec(),
            };
            log.append(200, &RustRawLogBytes(new_entry.encode_to_vec()))
                .unwrap();
            log.flush().unwrap();
        });

        let got = futures::executor::block_on(async { stream.next().await })
            .unwrap()
            .unwrap();
        writer.join().unwrap();

        assert_eq!(got.timestamp_ns, 200);
        assert_eq!(got.payload.frame, b"new-jpeg");
    }

    #[test]
    fn retained_pointcloud_source_maps_log_data_to_stream_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let entry = PointCloudData {
            data: b"cdr".to_vec(),
        };
        append_raw_payload(dir.path(), 200, entry.encode_to_vec());

        let mut stream = retained_log_into_source_stream(
            retained_source_for(dir.path(), "pointcloud"),
            decode_retained_pointcloud,
            &RustReadFrom::FromStart,
        )
        .unwrap();
        let got = crate::cluster_tokio_runtime()
            .block_on(async { stream.next().await })
            .unwrap()
            .unwrap();

        assert_eq!(got.timestamp_ns, 200);
        assert_eq!(got.payload.data, b"cdr");
    }

    #[test]
    fn retained_joint_encoder_source_maps_log_angles_to_stream_angles() {
        let dir = tempfile::tempdir().unwrap();
        let entry = JointEncodersData {
            angles_rad: vec![0.1, 0.2, 0.3],
        };
        append_raw_payload(dir.path(), 300, entry.encode_to_vec());

        let mut stream = retained_log_into_source_stream(
            retained_source_for(dir.path(), "joint_encoders"),
            decode_retained_joint_encoders,
            &RustReadFrom::FromStart,
        )
        .unwrap();
        let got = crate::cluster_tokio_runtime()
            .block_on(async { stream.next().await })
            .unwrap()
            .unwrap();

        assert_eq!(got.timestamp_ns, 300);
        assert_eq!(got.payload.angles_rad, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn retained_audio_source_maps_log_data_to_stream_data() {
        let dir = tempfile::tempdir().unwrap();
        let entry = AudioData {
            data: b"pcm".to_vec(),
        };
        append_raw_payload(dir.path(), 400, entry.encode_to_vec());

        let mut stream = retained_log_into_source_stream(
            retained_source_for(dir.path(), "audio"),
            decode_retained_audio,
            &RustReadFrom::FromStart,
        )
        .unwrap();
        let got = crate::cluster_tokio_runtime()
            .block_on(async { stream.next().await })
            .unwrap()
            .unwrap();

        assert_eq!(got.timestamp_ns, 400);
        assert_eq!(got.payload.data, b"pcm");
    }

    #[test]
    fn retained_map_source_replays_map_update_with_pinned_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let update = RustMapUpdate::default();
        append_raw_payload(dir.path(), 500, update.encode_to_vec());
        let mut source = retained_source_for(dir.path(), "map");
        source.resource_id = "voxel/world".into();
        source.map_peer_id = "peer-a".into();
        source.map_id = "voxel/world".into();
        source.map_hash = "map-hash".into();
        source.clock_peer_id = "peer-a".into();

        let manifest = manifest_from_retained_source(&source);
        assert_eq!(manifest.payload, "map_update");
        assert_eq!(manifest.resource_id, "voxel/world");
        assert_eq!(manifest.map_hash, "map-hash");

        let mut stream =
            retained_log_into_source_stream(source, decode_retained_map, &RustReadFrom::FromStart)
                .unwrap();
        let got = crate::cluster_tokio_runtime()
            .block_on(async { stream.next().await })
            .unwrap()
            .unwrap();
        assert_eq!(got.timestamp_ns, 500);
        assert_eq!(got.payload, update);
    }

    #[test]
    fn retained_source_tail_does_not_require_ambient_tokio_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = RawLog::<RustRawLogBytes>::open(dir.path(), raw_log_manifest()).unwrap();
        log.flush().unwrap();
        drop(log);
        let mut stream = retained_log_into_source_stream(
            retained_source_for(dir.path(), "camera"),
            decode_retained_camera,
            &RustReadFrom::Latest,
        )
        .unwrap();
        let writer_dir = dir.path().to_path_buf();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let entry = CameraFrame {
                dynamic_intrinsics: None,
                frame: b"tail-jpeg".to_vec(),
            };
            append_raw_payload(&writer_dir, 500, entry.encode_to_vec());
        });

        let got = futures::executor::block_on(async { stream.next().await })
            .unwrap()
            .unwrap();
        writer.join().unwrap();

        assert_eq!(got.timestamp_ns, 500);
        assert_eq!(got.payload.frame, b"tail-jpeg");
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

    /// Helper: wrap a [`PyCameraFrame`] as a `Bound<'_, PyAny>` for the
    /// new typed-payload `PyStreamItem::new` constructor (which
    /// accepts either `CameraFrame` or `PointCloudFrame`).
    fn camera_frame_as_any<'py>(py: Python<'py>, bytes: &[u8]) -> Bound<'py, PyAny> {
        let frame = PyCameraFrame::new(PyBytes::new_bound(py, bytes), None);
        Py::new(py, frame)
            .expect("alloc PyCameraFrame")
            .bind(py)
            .clone()
            .into_any()
    }

    /// Helper: wrap a [`PyPointCloudFrame`] as a `Bound<'_, PyAny>` for
    /// the typed-payload `PyStreamItem::new` constructor.
    fn pointcloud_frame_as_any<'py>(py: Python<'py>, bytes: &[u8]) -> Bound<'py, PyAny> {
        let frame = PyPointCloudFrame::new(PyBytes::new_bound(py, bytes));
        Py::new(py, frame)
            .expect("alloc PyPointCloudFrame")
            .bind(py)
            .clone()
            .into_any()
    }

    /// Helper: wrap a [`PyAudioFrame`] as a `Bound<'_, PyAny>` for the
    /// typed-payload `PyStreamItem::new` constructor (Dialogue
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
    /// `PyCameraFrame` / `PyPointCloudFrame`. Same opaque-bytes shape on
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
    fn detection_frame_exposes_typed_envelope_fields() {
        Python::with_gil(|py| {
            let payload = PyBytes::new_bound(py, br#"{"schema_version":1,"codes":[]}"#);
            let frame = PyDetectionFrame::new(payload, "sensor-hash".into(), "qr".into());
            assert_eq!(
                frame.data(py).as_bytes(),
                br#"{"schema_version":1,"codes":[]}"#
            );
            assert_eq!(frame.sensor_hash(), "sensor-hash");
            assert_eq!(frame.r#type(), "qr");

            let object = Py::new(py, frame).unwrap();
            assert_eq!(
                object
                    .bind(py)
                    .getattr("type")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "qr"
            );
        });
    }

    #[test]
    fn spatial_transform_frame_round_trips_flat_values() {
        Python::with_gil(|_py| {
            let f = PySpatialTransformFrame::new(vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
            assert_eq!(f.values(), vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0]);
            assert_eq!(f.__len__(), 7);
            assert!(f.__repr__().starts_with("SpatialTransformFrame("));
            assert!(f.__repr__().contains("1.0"));
        });
    }

    #[test]
    fn stream_item_extracts_to_rust_camera() {
        Python::with_gil(|py| {
            let payload_any = camera_frame_as_any(py, &[1, 2, 3]);
            let pf = PyStreamItem::new(123_456_789, payload_any).unwrap();
            let rust = pf.to_rust_camera().expect("payload is Camera");
            assert_eq!(rust.timestamp_ns, 123_456_789);
            assert_eq!(rust.payload.frame, vec![1, 2, 3]);
        });
    }

    #[test]
    fn stream_item_extracts_to_rust_pointcloud() {
        Python::with_gil(|py| {
            let payload_any = pointcloud_frame_as_any(py, &[0xaa, 0xbb]);
            let pf = PyStreamItem::new(42, payload_any).unwrap();
            let rust = pf.to_rust_pointcloud().expect("payload is PointCloud");
            assert_eq!(rust.timestamp_ns, 42);
            assert_eq!(rust.payload.data, vec![0xaa, 0xbb]);
        });
    }

    #[test]
    fn stream_item_extracts_to_rust_audio() {
        Python::with_gil(|py| {
            let payload_any = audio_frame_as_any(py, &[0x01, 0x02, 0x03]);
            let pf = PyStreamItem::new(987_654, payload_any).unwrap();
            let rust = pf.to_rust_audio().expect("payload is Audio");
            assert_eq!(rust.timestamp_ns, 987_654);
            assert_eq!(rust.payload.data, vec![0x01, 0x02, 0x03]);
        });
    }

    #[test]
    fn stream_item_extracts_to_rust_pose() {
        Python::with_gil(|py| {
            let frame =
                PySpatialTransformFrame::new(vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
            let payload_any = Py::new(py, frame).unwrap().bind(py).clone().into_any();
            let item = PyStreamItem::new(123, payload_any).unwrap();
            let rust: RustStreamItem<PoseSpatialTransform> =
                item.to_rust_pose().expect("payload is pose");
            assert_eq!(rust.timestamp_ns, 123);
            assert_eq!(rust.payload.translation.unwrap().x, 1.0);
            assert_eq!(rust.payload.orientation.unwrap().w, 1.0);
        });
    }

    /// Mismatched payload variant on `to_rust_*` must surface a
    /// human-readable error — the source-stream pump turns this into an
    /// `EndReason::ProducerError` on the wire rather than ending the
    /// substream silently.
    #[test]
    fn stream_item_to_rust_errors_on_mismatched_payload() {
        Python::with_gil(|py| {
            let pf_camera = PyStreamItem::new(0, camera_frame_as_any(py, &[1])).unwrap();
            let err = pf_camera
                .to_rust_pointcloud()
                .expect_err("camera != pointcloud");
            assert!(err.contains("AcceptPointCloud"), "{err}");
            assert!(err.contains("PointCloudFrame"), "{err}");

            let pf_pc = PyStreamItem::new(0, pointcloud_frame_as_any(py, &[2])).unwrap();
            let err = pf_pc.to_rust_camera().expect_err("pointcloud ≠ camera");
            assert!(err.contains("AcceptCamera"), "{err}");
            assert!(err.contains("CameraFrame"), "{err}");

            // Audio ≠ Camera: same mismatch shape for the Dialogue arm.
            let pf_audio = PyStreamItem::new(0, audio_frame_as_any(py, &[3])).unwrap();
            let err = pf_audio.to_rust_camera().expect_err("audio ≠ camera");
            assert!(err.contains("AcceptCamera"), "{err}");
            assert!(err.contains("AudioFrame"), "{err}");
        });
    }

    /// A non-CameraFrame / non-PointCloudFrame object passed as `payload`
    /// must surface a `ValueError` at construction time — the Python
    /// surface is closed over the two SDK-supported `T`s.
    #[test]
    fn stream_item_rejects_unknown_payload_type() {
        Python::with_gil(|py| {
            // py.None() is a stand-in for "anything that isn't a frame
            // PyClass" — same shape as a Python user passing a dict, an
            // int, a custom class, etc.
            let bad = py.None();
            let err = PyStreamItem::new(0, bad.bind(py).clone()).expect_err("None is not a frame");
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(err.to_string().contains("CameraFrame"), "{err}");
            assert!(err.to_string().contains("PointCloudFrame"), "{err}");
            assert!(err.to_string().contains("AudioFrame"), "{err}");
        });
    }

    #[test]
    fn stream_entry_constructs_from_rust_camera() {
        Python::with_gil(|_py| {
            let rust_frame = RustStreamEntry {
                timestamp_ns: 9_999,
                seq: 17,
                payload: RustCameraFrame {
                    dynamic_intrinsics: None,
                    frame: vec![0xff, 0xd8, 0xee],
                },
            };
            let pf = PyStreamEntry::from_rust_camera(rust_frame);
            assert_eq!(pf.timestamp_ns(), 9_999);
            assert_eq!(pf.seq(), 17);
            // Inspect the payload variant directly via the
            // `pub(crate)` field — exposing `__len__` would require
            // routing through the GIL-bound getter.
            match &pf.payload {
                StreamPayload::Camera(j) => assert_eq!(j.inner.frame.len(), 3),
                _ => panic!("expected Camera payload variant"),
            }
        });
    }

    #[test]
    fn stream_entry_constructs_from_rust_pointcloud() {
        Python::with_gil(|_py| {
            let rust_frame = RustStreamEntry {
                timestamp_ns: 1_000,
                seq: 7,
                payload: RustPointCloudFrame {
                    data: vec![0x01, 0x02, 0x03, 0x04],
                },
            };
            let pf = PyStreamEntry::from_rust_pointcloud(rust_frame);
            assert_eq!(pf.timestamp_ns(), 1_000);
            assert_eq!(pf.seq(), 7);
            match &pf.payload {
                StreamPayload::PointCloud(p) => assert_eq!(p.inner.data.len(), 4),
                _ => panic!("expected PointCloud payload variant"),
            }
        });
    }

    #[test]
    fn stream_entry_constructs_from_rust_audio() {
        Python::with_gil(|_py| {
            let rust_frame = RustStreamEntry {
                timestamp_ns: 5_555,
                seq: 99,
                payload: RustAudioFrame {
                    data: vec![0xab, 0xcd, 0xef],
                },
            };
            let pf = PyStreamEntry::from_rust_audio(rust_frame);
            assert_eq!(pf.timestamp_ns(), 5_555);
            assert_eq!(pf.seq(), 99);
            match &pf.payload {
                StreamPayload::Audio(a) => assert_eq!(a.inner.data.len(), 3),
                _ => panic!("expected Audio payload variant"),
            }
        });
    }

    #[test]
    fn stream_decision_factories_tag_correctly() {
        Python::with_gil(|py| {
            // Construct a Python object to stand in for the source iterator
            // (a None object is fine — we only inspect .kind, never drain).
            let manifest = PyStreamManifest::new(
                "sensor".into(),
                "h".into(),
                "c".into(),
                "ch".into(),
                Some("frame".into()),
                Some("fh".into()),
            );
            let acc = PyStreamDecision::accept_camera(manifest.clone(), py.None());
            assert_eq!(acc.kind(), "accept_camera");

            let acc_pc = PyStreamDecision::accept_pointcloud(manifest.clone(), py.None());
            assert_eq!(acc_pc.kind(), "accept_pointcloud");

            let acc_audio = PyStreamDecision::accept_audio(manifest, py.None());
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
    def provider(peer, req):
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
                resource_id: "any".into(),
                ..Default::default()
            };
            match rust_provider(test_peer_id(), request) {
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
def _bad(peer, req):
    raise RuntimeError("provider broke")
"#,
                None,
                None,
            )
            .unwrap();
            let bad = py.eval_bound("_bad", None, None).unwrap();
            let rust_provider = build_stream_provider(bad.unbind());
            let request = RustStreamRequest {
                resource_id: "any".into(),
                ..Default::default()
            };
            match rust_provider(test_peer_id(), request) {
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

    /// `build_stream_provider` mapping the Python
    /// `accept_camera(manifest, source)`
    /// factory onto `RustStreamDispatch::AcceptCamera`. We don't drain the
    /// source-stream here (that requires the wrapper's tokio runtime
    /// + asyncio loop scaffolding from the cross-language tests); we
    /// only assert that the dispatch variant matches the Python call.
    #[test]
    fn build_stream_provider_accept_camera_maps_to_dispatch_acceptcamera() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "test_provider_accept_camera").unwrap();
            crate::populate_module(&module).unwrap();
            let cluster = module.getattr("cluster").unwrap();

            py.run_bound(
                r#"
def _make(cluster):
    async def _src():
        if False:
            yield None  # makes this an async generator
    def provider(peer, req):
        return cluster.StreamDecision.accept_camera(
            manifest=cluster.StreamManifest(
                sensor_id=req.resource_id,
                sensor_hash="h",
                clock_id="c",
                clock_hash="ch",
                frame_id="frame",
                frame_hash="fh",
            ),
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

            match rust_provider(
                test_peer_id(),
                RustStreamRequest {
                    resource_id: "any".into(),
                    ..Default::default()
                },
            ) {
                RustStreamDispatch::AcceptCamera {
                    manifest,
                    source: _,
                } => {
                    assert_eq!(manifest.sensor_id, "any");
                    assert_eq!(manifest.sensor_hash, "h");
                    assert_eq!(manifest.clock_id, "c");
                    assert_eq!(manifest.frame_id, "frame");
                }
                _ => panic!("expected AcceptCamera"),
            }
        });
    }

    /// `build_stream_provider` mapping `accept_pointcloud(manifest, source)`
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
    def provider(peer, req):
        return cluster.StreamDecision.accept_pointcloud(
            manifest=cluster.StreamManifest(
                sensor_id=req.resource_id,
                sensor_hash="pc",
                clock_id="c",
                clock_hash="ch",
                frame_id="pc_frame",
                frame_hash="pc_fh",
            ),
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

            match rust_provider(
                test_peer_id(),
                RustStreamRequest {
                    resource_id: "any".into(),
                    ..Default::default()
                },
            ) {
                RustStreamDispatch::AcceptPointCloud {
                    manifest,
                    source: _,
                } => {
                    assert_eq!(manifest.sensor_hash, "pc");
                    assert_eq!(manifest.frame_hash, "pc_fh");
                }
                _ => panic!("expected AcceptPointCloud"),
            }
        });
    }

    #[test]
    fn scalar_frame_and_stream_item_preserve_value() {
        let frame = PyScalarFrame::new(73.5);
        assert_eq!(frame.value(), 73.5);
        assert_eq!(frame.__repr__(), "ScalarFrame(value=73.5)");

        let item = PyStreamItem {
            timestamp_ns: 42,
            payload: StreamPayload::Scalar(frame),
        };
        let rust = item.to_rust_scalar().unwrap();
        assert_eq!(rust.timestamp_ns, 42);
        assert_eq!(rust.payload.value, 73.5);
    }

    #[test]
    fn stream_entry_constructs_from_rust_scalar() {
        let entry = PyStreamEntry::from_rust_scalar(RustStreamEntry {
            timestamp_ns: 99,
            seq: 7,
            payload: RustScalarFrame { value: 12.25 },
        });
        assert_eq!(entry.timestamp_ns, 99);
        assert_eq!(entry.seq, 7);
        let StreamPayload::Scalar(frame) = entry.payload else {
            panic!("expected ScalarFrame");
        };
        assert_eq!(frame.value(), 12.25);
    }

    #[test]
    fn retained_scalar_decoder_uses_canonical_payload() {
        let bytes = RustScalarFrame { value: 88.0 }.encode_to_vec();
        assert_eq!(decode_retained_scalar(bytes).unwrap().value, 88.0);
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
