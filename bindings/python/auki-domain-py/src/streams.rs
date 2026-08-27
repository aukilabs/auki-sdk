use std::{
    collections::VecDeque,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Weak},
    time::Duration,
};

use auki_datatypes::{
    audio::Data as AudioFrame, detection::DetectionFrame,
    joint_encoders::Data as JointEncodersFrame, map::MapUpdate,
    point_cloud::Data as PointCloudFrame, pose::SpatialTransform, scalar::Data as ScalarFrame,
};
use auki_domain_rs::{
    Domain, PeerId, ReadFrom, SourceStream, StreamDispatch, StreamItem, StreamRequest,
};
use auki_logs_rs::{Log as RetainedLog, LogPayload};
use auki_network::{
    resources_protocol::{ResourceEntry, SensorKind, VariantContent},
    stream_protocol::{CameraFrame, DeclineReason, StreamManifest, decline_reason},
    stream_runtime::{StreamEntry, StreamSubscription},
};
use futures::{Stream, StreamExt};
use prost::Message;
use pyo3::{
    exceptions::{PyFileNotFoundError, PyRuntimeError, PyStopAsyncIteration, PyValueError},
    prelude::*,
    pyclass::{PyTraverseError, PyVisit},
    types::{PyAny, PyBytes, PyModule},
};
use pyo3_async_runtimes::TaskLocals;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    runtime_error,
    values::{PyMapLogResource, PyStreamRequest},
};

type SyncMutex<T> = parking_lot::Mutex<T>;

const PYTHON_SOURCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

type RawEntries = Pin<Box<dyn Stream<Item = Result<RawEntry, String>> + Send>>;

struct RawEntry {
    timestamp_ns: i64,
    seq: u64,
    payload: StreamPayload,
}

#[pyclass(name = "StreamManifest", frozen)]
#[derive(Clone)]
pub(crate) struct PyStreamManifest {
    inner: StreamManifest,
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
            inner: StreamManifest {
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
    #[allow(clippy::too_many_arguments)]
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
            inner: StreamManifest {
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
    #[allow(clippy::too_many_arguments)]
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
            inner: StreamManifest {
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
    fn clock_peer_id(&self) -> &str {
        &self.inner.clock_peer_id
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
    #[pyo3(name = "from_frame_id")]
    fn source_frame_id(&self) -> &str {
        &self.inner.from_frame_id
    }
    #[getter]
    #[pyo3(name = "from_frame_hash")]
    fn source_frame_hash(&self) -> &str {
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

#[pyclass(name = "StreamManifestBuilder")]
pub(crate) struct PyStreamManifestBuilder;

#[pymethods]
impl PyStreamManifestBuilder {
    #[staticmethod]
    #[pyo3(signature = (app_root, sensor_peer_id, sensor_id, sensor_hash, clock_id, clock_hash))]
    fn from_registry(
        py: Python<'_>,
        app_root: PathBuf,
        sensor_peer_id: String,
        sensor_id: String,
        sensor_hash: String,
        clock_id: String,
        clock_hash: String,
    ) -> PyResult<PyStreamManifest> {
        let inner = py
            .allow_threads(|| {
                auki_domain_rs::StreamManifestBuilder::from_registry(
                    &app_root,
                    sensor_peer_id,
                    sensor_id,
                    sensor_hash,
                    clock_id,
                    clock_hash,
                )
            })
            .map_err(runtime_error)?;
        Ok(PyStreamManifest { inner })
    }
}

// Producer payloads intentionally retain the public shape from the former
// `auki-network-py` bridge.  The authenticated Domain owns transport and
// admission now; these classes are only typed application payloads.

#[pyclass(name = "DynamicIntrinsics", frozen)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PyDynamicIntrinsics {
    inner: auki_network::stream_protocol::DynamicIntrinsics,
}

#[pymethods]
impl PyDynamicIntrinsics {
    #[new]
    #[pyo3(signature = (*, fx, fy, cx, cy, distortion_coefficients=None))]
    fn new(fx: f64, fy: f64, cx: f64, cy: f64, distortion_coefficients: Option<Vec<f64>>) -> Self {
        Self {
            inner: auki_network::stream_protocol::DynamicIntrinsics {
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

#[pyclass(name = "CameraFrame", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PyCameraFrame {
    inner: CameraFrame,
}

#[pymethods]
impl PyCameraFrame {
    #[new]
    #[pyo3(signature = (frame, /, dynamic_intrinsics=None))]
    fn new(frame: Bound<'_, PyBytes>, dynamic_intrinsics: Option<PyDynamicIntrinsics>) -> Self {
        Self {
            inner: CameraFrame {
                dynamic_intrinsics: dynamic_intrinsics.map(|value| value.inner),
                frame: frame.as_bytes().to_vec(),
            },
        }
    }
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

#[pyclass(name = "PointCloudFrame", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PyPointCloudFrame {
    inner: PointCloudFrame,
}

#[pymethods]
impl PyPointCloudFrame {
    #[new]
    #[pyo3(signature = (bytes, /))]
    fn new(bytes: Bound<'_, PyBytes>) -> Self {
        Self {
            inner: PointCloudFrame {
                data: bytes.as_bytes().to_vec(),
            },
        }
    }
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

#[pyclass(name = "JointEncodersFrame", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PyJointEncodersFrame {
    inner: JointEncodersFrame,
}

#[pymethods]
impl PyJointEncodersFrame {
    #[new]
    #[pyo3(signature = (angles_rad, /))]
    fn new(angles_rad: Vec<f32>) -> Self {
        Self {
            inner: JointEncodersFrame { angles_rad },
        }
    }
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

#[pyclass(name = "AudioFrame", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PyAudioFrame {
    inner: AudioFrame,
}

#[pymethods]
impl PyAudioFrame {
    #[new]
    #[pyo3(signature = (data, /))]
    fn new(data: Bound<'_, PyBytes>) -> Self {
        Self {
            inner: AudioFrame {
                data: data.as_bytes().to_vec(),
            },
        }
    }
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

#[pyclass(name = "ScalarFrame", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PyScalarFrame {
    inner: ScalarFrame,
}

#[pymethods]
impl PyScalarFrame {
    #[new]
    #[pyo3(signature = (value, /))]
    fn new(value: f64) -> Self {
        Self {
            inner: ScalarFrame { value },
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

#[pyclass(name = "DetectionFrame", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PyDetectionFrame {
    inner: DetectionFrame,
}

#[pymethods]
impl PyDetectionFrame {
    #[new]
    #[pyo3(signature = (*, data, sensor_hash, type_))]
    fn new(data: Bound<'_, PyBytes>, sensor_hash: String, type_: String) -> Self {
        Self {
            inner: DetectionFrame {
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

#[pyclass(name = "SpatialTransformFrame", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PySpatialTransformFrame {
    inner: SpatialTransform,
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
            inner: SpatialTransform {
                translation: Some(auki_datatypes::pose::Vec3 {
                    x: values[0],
                    y: values[1],
                    z: values[2],
                }),
                orientation: Some(auki_datatypes::pose::Quat {
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
        let translation = self.inner.translation.as_ref();
        let orientation = self.inner.orientation.as_ref();
        vec![
            translation.map(|value| value.x).unwrap_or(0.0),
            translation.map(|value| value.y).unwrap_or(0.0),
            translation.map(|value| value.z).unwrap_or(0.0),
            orientation.map(|value| value.x).unwrap_or(0.0),
            orientation.map(|value| value.y).unwrap_or(0.0),
            orientation.map(|value| value.z).unwrap_or(0.0),
            orientation.map(|value| value.w).unwrap_or(1.0),
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

#[pyclass(name = "MapUpdateFrame", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PyMapUpdateFrame {
    inner: MapUpdate,
}

#[pymethods]
impl PyMapUpdateFrame {
    #[new]
    #[pyo3(signature = (data, /))]
    fn new(data: Bound<'_, PyBytes>) -> PyResult<Self> {
        let inner = <MapUpdate as Message>::decode(data.as_bytes())
            .map_err(|error| PyValueError::new_err(format!("invalid MapUpdate bytes: {error}")))?;
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

#[derive(Clone, Debug)]
enum StreamPayload {
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
    fn from_python(payload: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(value) = payload.extract::<PyCameraFrame>() {
            return Ok(Self::Camera(value));
        }
        if let Ok(value) = payload.extract::<PyPointCloudFrame>() {
            return Ok(Self::PointCloud(value));
        }
        if let Ok(value) = payload.extract::<PyJointEncodersFrame>() {
            return Ok(Self::JointEncoders(value));
        }
        if let Ok(value) = payload.extract::<PyAudioFrame>() {
            return Ok(Self::Audio(value));
        }
        if let Ok(value) = payload.extract::<PyScalarFrame>() {
            return Ok(Self::Scalar(value));
        }
        if let Ok(value) = payload.extract::<PyDetectionFrame>() {
            return Ok(Self::Detection(value));
        }
        if let Ok(value) = payload.extract::<PySpatialTransformFrame>() {
            return Ok(Self::Pose(value));
        }
        if let Ok(value) = payload.extract::<PyMapUpdateFrame>() {
            return Ok(Self::Map(value));
        }
        Err(PyValueError::new_err(format!(
            "stream payload must be a CameraFrame, PointCloudFrame, JointEncodersFrame, AudioFrame, ScalarFrame, DetectionFrame, SpatialTransformFrame, or MapUpdateFrame; got {}",
            payload
                .repr()
                .map(|value| value.to_string())
                .unwrap_or_else(|_| "<unrepr>".into())
        )))
    }

    fn clone_into_python(&self, py: Python<'_>) -> PyObject {
        match self {
            Self::Camera(value) => Py::new(py, value.clone())
                .expect("allocate CameraFrame")
                .into_py(py),
            Self::PointCloud(value) => Py::new(py, value.clone())
                .expect("allocate PointCloudFrame")
                .into_py(py),
            Self::JointEncoders(value) => Py::new(py, value.clone())
                .expect("allocate JointEncodersFrame")
                .into_py(py),
            Self::Audio(value) => Py::new(py, value.clone())
                .expect("allocate AudioFrame")
                .into_py(py),
            Self::Scalar(value) => Py::new(py, value.clone())
                .expect("allocate ScalarFrame")
                .into_py(py),
            Self::Detection(value) => Py::new(py, value.clone())
                .expect("allocate DetectionFrame")
                .into_py(py),
            Self::Pose(value) => Py::new(py, value.clone())
                .expect("allocate SpatialTransformFrame")
                .into_py(py),
            Self::Map(value) => Py::new(py, value.clone())
                .expect("allocate MapUpdateFrame")
                .into_py(py),
        }
    }

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

    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Camera(value) => value.inner.encode_to_vec(),
            Self::PointCloud(value) => value.inner.encode_to_vec(),
            Self::JointEncoders(value) => value.inner.encode_to_vec(),
            Self::Audio(value) => value.inner.encode_to_vec(),
            Self::Scalar(value) => value.inner.encode_to_vec(),
            Self::Detection(value) => value.inner.encode_to_vec(),
            Self::Pose(value) => value.inner.encode_to_vec(),
            Self::Map(value) => value.inner.encode_to_vec(),
        }
    }
}

#[pyclass(name = "StreamItem", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PyStreamItem {
    timestamp_ns: i64,
    payload: StreamPayload,
}

#[pymethods]
impl PyStreamItem {
    #[new]
    #[pyo3(signature = (*, timestamp_ns, payload))]
    fn new(timestamp_ns: i64, payload: Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            timestamp_ns,
            payload: StreamPayload::from_python(&payload)?,
        })
    }
    #[getter]
    fn timestamp_ns(&self) -> i64 {
        self.timestamp_ns
    }
    #[getter]
    fn payload(&self, py: Python<'_>) -> PyObject {
        self.payload.clone_into_python(py)
    }
    fn __repr__(&self) -> String {
        format!(
            "StreamItem(timestamp_ns={}, payload={})",
            self.timestamp_ns,
            self.payload.kind_name(),
        )
    }
}

#[pyclass(name = "DeclineReason", frozen)]
#[derive(Clone, Debug)]
pub(crate) struct PyDeclineReason {
    inner: DeclineReason,
}

#[pymethods]
impl PyDeclineReason {
    #[staticmethod]
    fn sensor_not_found() -> Self {
        Self {
            inner: DeclineReason::sensor_not_found(),
        }
    }
    #[staticmethod]
    fn sensor_unavailable() -> Self {
        Self {
            inner: DeclineReason::sensor_unavailable(),
        }
    }
    #[staticmethod]
    fn producer_shutting_down() -> Self {
        Self {
            inner: DeclineReason::producer_shutting_down(),
        }
    }
    #[staticmethod]
    #[pyo3(signature = (*, detail))]
    fn other(detail: String) -> Self {
        Self {
            inner: DeclineReason::other(detail),
        }
    }
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
    #[getter]
    fn detail(&self) -> Option<&str> {
        match &self.inner.kind {
            Some(decline_reason::Kind::Other(value)) => Some(value.detail.as_str()),
            _ => None,
        }
    }
    fn __repr__(&self) -> String {
        match &self.inner.kind {
            Some(decline_reason::Kind::Other(value)) => {
                format!("DeclineReason.other(detail={:?})", value.detail)
            }
            Some(decline_reason::Kind::SensorNotFound(_)) => {
                "DeclineReason.sensor_not_found()".into()
            }
            Some(decline_reason::Kind::SensorUnavailable(_)) => {
                "DeclineReason.sensor_unavailable()".into()
            }
            Some(decline_reason::Kind::ProducerShuttingDown(_)) => {
                "DeclineReason.producer_shutting_down()".into()
            }
            None => "DeclineReason.<unspecified>()".into(),
        }
    }
    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

#[derive(Clone)]
struct RetainedSource {
    root: PathBuf,
    resource_id: String,
    sensor_id: String,
    sensor_hash: String,
    map_peer_id: String,
    map_id: String,
    map_hash: String,
    clock_peer_id: String,
    clock_id: String,
    clock_hash: String,
    payload_kind: String,
    frame_id: String,
    frame_hash: String,
}

impl RetainedSource {
    fn from_python(source: &Bound<'_, PyAny>) -> PyResult<Self> {
        fn required(source: &Bound<'_, PyAny>, name: &str) -> PyResult<String> {
            let value: String = source
                .getattr(name)
                .and_then(|value| value.extract())
                .map_err(|error| {
                    PyValueError::new_err(format!(
                        "accept_source expects auki_logs.StreamSource.{name}: {error}"
                    ))
                })?;
            if value.is_empty() {
                return Err(PyValueError::new_err(format!(
                    "accept_source requires non-empty StreamSource.{name}"
                )));
            }
            Ok(value)
        }

        fn optional(source: &Bound<'_, PyAny>, name: &str) -> PyResult<String> {
            match source.getattr(name) {
                Ok(value) => value.extract(),
                Err(error)
                    if error.is_instance_of::<pyo3::exceptions::PyAttributeError>(source.py()) =>
                {
                    Ok(String::new())
                }
                Err(error) => Err(error),
            }
        }

        let root = PathBuf::from(required(source, "root")?);
        let payload_kind = required(source, "payload_kind")?;
        let retained = Self {
            root,
            resource_id: optional(source, "resource_id")?,
            sensor_id: optional(source, "sensor_id")?,
            sensor_hash: optional(source, "sensor_hash")?,
            map_peer_id: optional(source, "map_peer_id")?,
            map_id: optional(source, "map_id")?,
            map_hash: optional(source, "map_hash")?,
            clock_peer_id: optional(source, "clock_peer_id")?,
            clock_id: required(source, "clock_id")?,
            clock_hash: required(source, "clock_hash")?,
            payload_kind,
            frame_id: optional(source, "frame_id")?,
            frame_hash: optional(source, "frame_hash")?,
        };
        retained.validate()?;
        Ok(retained)
    }

    fn validate(&self) -> PyResult<()> {
        match self.payload_kind.as_str() {
            "camera" | "pointcloud" | "point_cloud" | "joint_encoders" | "audio" | "scalar" => {
                for (name, value) in [
                    ("sensor_id", &self.sensor_id),
                    ("sensor_hash", &self.sensor_hash),
                ] {
                    if value.is_empty() {
                        return Err(PyValueError::new_err(format!(
                            "accept_source requires non-empty StreamSource.{name}"
                        )));
                    }
                }
            }
            "map" => {
                for (name, value) in [
                    ("resource_id", &self.resource_id),
                    ("map_peer_id", &self.map_peer_id),
                    ("map_id", &self.map_id),
                    ("map_hash", &self.map_hash),
                    ("clock_peer_id", &self.clock_peer_id),
                ] {
                    if value.is_empty() {
                        return Err(PyValueError::new_err(format!(
                            "accept_source requires non-empty StreamSource.{name}"
                        )));
                    }
                }
            }
            other => {
                return Err(PyValueError::new_err(format!(
                    "StreamSource.payload_kind must be camera, pointcloud, joint_encoders, audio, scalar, or map; got {other:?}"
                )));
            }
        }
        Ok(())
    }

    fn manifest(&self) -> StreamManifest {
        if self.payload_kind == "map" {
            StreamManifest {
                resource_id: self.resource_id.clone(),
                payload: "map_update".into(),
                map_peer_id: self.map_peer_id.clone(),
                map_id: self.map_id.clone(),
                map_hash: self.map_hash.clone(),
                clock_peer_id: self.clock_peer_id.clone(),
                clock_id: self.clock_id.clone(),
                clock_hash: self.clock_hash.clone(),
                ..Default::default()
            }
        } else {
            StreamManifest {
                sensor_id: self.sensor_id.clone(),
                sensor_hash: self.sensor_hash.clone(),
                clock_id: self.clock_id.clone(),
                clock_hash: self.clock_hash.clone(),
                frame_id: self.frame_id.clone(),
                frame_hash: self.frame_hash.clone(),
                ..Default::default()
            }
        }
    }

    fn decision_kind(&self) -> &'static str {
        match self.payload_kind.as_str() {
            "camera" => "accept_camera",
            "pointcloud" | "point_cloud" => "accept_pointcloud",
            "joint_encoders" => "accept_joint_encoders",
            "audio" => "accept_audio",
            "scalar" => "accept_scalar",
            "map" => "accept_map",
            _ => "accept_source",
        }
    }
}

enum DecisionInner {
    Camera {
        manifest: PyStreamManifest,
        source: Arc<Py<PyAny>>,
    },
    PointCloud {
        manifest: PyStreamManifest,
        source: Arc<Py<PyAny>>,
    },
    JointEncoders {
        manifest: PyStreamManifest,
        source: Arc<Py<PyAny>>,
    },
    Audio {
        manifest: PyStreamManifest,
        source: Arc<Py<PyAny>>,
    },
    Scalar {
        manifest: PyStreamManifest,
        source: Arc<Py<PyAny>>,
    },
    Detection {
        manifest: PyStreamManifest,
        source: Arc<Py<PyAny>>,
    },
    Pose {
        manifest: PyStreamManifest,
        source: Arc<Py<PyAny>>,
    },
    Map {
        manifest: PyStreamManifest,
        source: Arc<Py<PyAny>>,
    },
    Retained {
        manifest: PyStreamManifest,
        source: Box<RetainedSource>,
    },
    Decline {
        reason: PyDeclineReason,
    },
}

impl DecisionInner {
    fn source(&self) -> Option<Arc<Py<PyAny>>> {
        match self {
            Self::Camera { source, .. }
            | Self::PointCloud { source, .. }
            | Self::JointEncoders { source, .. }
            | Self::Audio { source, .. }
            | Self::Scalar { source, .. }
            | Self::Detection { source, .. }
            | Self::Pose { source, .. }
            | Self::Map { source, .. } => Some(Arc::clone(source)),
            Self::Retained { .. } | Self::Decline { .. } => None,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Camera { .. } => "accept_camera",
            Self::PointCloud { .. } => "accept_pointcloud",
            Self::JointEncoders { .. } => "accept_joint_encoders",
            Self::Audio { .. } => "accept_audio",
            Self::Scalar { .. } => "accept_scalar",
            Self::Detection { .. } => "accept_detection",
            Self::Pose { .. } => "accept_pose",
            Self::Map { .. } => "accept_map",
            Self::Retained { source, .. } => source.decision_kind(),
            Self::Decline { .. } => "decline",
        }
    }
}

#[pyclass(name = "StreamDecision", frozen)]
pub(crate) struct PyStreamDecision {
    inner: SyncMutex<Option<DecisionInner>>,
}

#[pymethods]
impl PyStreamDecision {
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_camera(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self::accepted(DecisionInner::Camera {
            manifest,
            source: Arc::new(source),
        })
    }
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_pointcloud(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self::accepted(DecisionInner::PointCloud {
            manifest,
            source: Arc::new(source),
        })
    }
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_joint_encoders(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self::accepted(DecisionInner::JointEncoders {
            manifest,
            source: Arc::new(source),
        })
    }
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_audio(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self::accepted(DecisionInner::Audio {
            manifest,
            source: Arc::new(source),
        })
    }
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_scalar(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self::accepted(DecisionInner::Scalar {
            manifest,
            source: Arc::new(source),
        })
    }
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_detection(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self::accepted(DecisionInner::Detection {
            manifest,
            source: Arc::new(source),
        })
    }
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_pose(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self::accepted(DecisionInner::Pose {
            manifest,
            source: Arc::new(source),
        })
    }
    #[staticmethod]
    #[pyo3(signature = (*, manifest, source))]
    fn accept_map(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
        Self::accepted(DecisionInner::Map {
            manifest,
            source: Arc::new(source),
        })
    }
    #[staticmethod]
    #[pyo3(signature = (source, /))]
    fn accept_source(source: &Bound<'_, PyAny>) -> PyResult<Self> {
        let source = RetainedSource::from_python(source)?;
        let manifest = PyStreamManifest {
            inner: source.manifest(),
        };
        Ok(Self::accepted(DecisionInner::Retained {
            manifest,
            source: Box::new(source),
        }))
    }
    #[staticmethod]
    #[pyo3(signature = (reason, /))]
    fn decline(reason: PyDeclineReason) -> Self {
        Self::accepted(DecisionInner::Decline { reason })
    }
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner
            .lock()
            .as_ref()
            .map_or("consumed", DecisionInner::kind)
    }
    fn __repr__(&self) -> String {
        format!("StreamDecision.{}()", self.kind())
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        let source = self.inner.lock().as_ref().and_then(DecisionInner::source);
        if let Some(source) = source {
            visit.call(source.as_ref())?;
        }
        Ok(())
    }

    fn __clear__(&self) {
        let decision = self.inner.lock().take();
        drop(decision);
    }
}

impl PyStreamDecision {
    fn accepted(inner: DecisionInner) -> Self {
        Self {
            inner: SyncMutex::new(Some(inner)),
        }
    }
    fn take(&self) -> Option<DecisionInner> {
        self.inner.lock().take()
    }
}

struct ActivePythonSource {
    iterator: parking_lot::RwLock<Option<Arc<Py<PyAny>>>>,
    pending: parking_lot::RwLock<Option<Arc<Py<PyAny>>>>,
}

impl ActivePythonSource {
    fn python(&self) -> Option<Arc<Py<PyAny>>> {
        self.iterator.read().clone()
    }

    fn set_pending(&self, pending: Arc<Py<PyAny>>) {
        let previous = self.pending.write().replace(pending);
        drop(previous);
    }

    fn clear_pending(&self) {
        let pending = self.pending.write().take();
        drop(pending);
    }

    fn clear(&self) {
        let iterator = self.iterator.write().take();
        let pending = self.pending.write().take();
        if let Some(pending) = pending {
            Python::with_gil(|py| {
                let _ = pending.bind(py).call_method0("cancel");
            });
        }
        drop(iterator);
    }
}

#[derive(Clone, Default)]
pub(crate) struct PythonSourceRegistry {
    sources: Arc<parking_lot::RwLock<Vec<Weak<ActivePythonSource>>>>,
    cleanup: TaskTracker,
}

impl PythonSourceRegistry {
    fn register(&self, iterator: Arc<Py<PyAny>>) -> Arc<ActivePythonSource> {
        let active = Arc::new(ActivePythonSource {
            iterator: parking_lot::RwLock::new(Some(iterator)),
            pending: parking_lot::RwLock::new(None),
        });
        let mut sources = self.sources.write();
        sources.retain(|source| source.strong_count() > 0);
        sources.push(Arc::downgrade(&active));
        active
    }

    pub(crate) fn visit(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        let sources = self
            .sources
            .read()
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for source in sources {
            if let Some(iterator) = source.python() {
                visit.call(iterator.as_ref())?;
            }
        }
        Ok(())
    }

    pub(crate) fn clear(&self) {
        let sources = self
            .sources
            .read()
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for source in sources {
            source.clear();
        }
        self.sources.write().clear();
    }

    fn schedule_close(&self, iterator: PythonAsyncIterator) {
        self.cleanup.spawn_on(
            async move {
                iterator.close().await;
            },
            pyo3_async_runtimes::tokio::get_runtime().handle(),
        );
    }

    /// Close every source dropped by the native stream pump and wait until
    /// those bounded Python cleanup tasks have finished.
    pub(crate) async fn shutdown(&self) {
        self.cleanup.close();
        self.cleanup.wait().await;
        self.clear();
    }
}

struct PythonAsyncIterator {
    active: Arc<ActivePythonSource>,
    locals: Arc<TaskLocals>,
}

#[pyclass]
struct PythonFutureCompletion {
    sender: SyncMutex<Option<tokio::sync::oneshot::Sender<PyResult<Py<PyAny>>>>>,
}

#[pymethods]
impl PythonFutureCompletion {
    fn __call__(&self, future: &Bound<'_, PyAny>) {
        let result = future.call_method0("result").map(Bound::unbind);
        if let Some(sender) = self.sender.lock().take() {
            let _ = sender.send(result);
        }
    }
}

struct PendingPythonFuture {
    future: Arc<Py<PyAny>>,
    active: Option<Arc<ActivePythonSource>>,
    complete: bool,
}

impl PendingPythonFuture {
    fn finish(mut self) {
        self.complete = true;
        if let Some(active) = self.active.take() {
            active.clear_pending();
        }
    }
}

impl Drop for PendingPythonFuture {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        Python::with_gil(|py| {
            let _ = self.future.bind(py).call_method0("cancel");
        });
        if let Some(active) = self.active.take() {
            active.clear_pending();
        }
    }
}

fn schedule_python(
    py: Python<'_>,
    locals: &TaskLocals,
    awaitable: Bound<'_, PyAny>,
    active: Option<Arc<ActivePythonSource>>,
) -> PyResult<(
    PendingPythonFuture,
    tokio::sync::oneshot::Receiver<PyResult<Py<PyAny>>>,
)> {
    static COROUTINE_WRAPPER: pyo3::sync::GILOnceCell<Py<PyAny>> = pyo3::sync::GILOnceCell::new();
    let wrapper = COROUTINE_WRAPPER.get_or_try_init(py, || {
        let module = PyModule::from_code_bound(
            py,
            "async def _auki_await(value):\n    return await value\n",
            "_auki_domain_stream_bridge.py",
            "_auki_domain_stream_bridge",
        )?;
        Ok::<_, PyErr>(module.getattr("_auki_await")?.unbind())
    })?;
    let coroutine = wrapper.bind(py).call1((awaitable,))?;
    let asyncio = py.import_bound("asyncio")?;
    let run_coroutine_threadsafe = asyncio.getattr("run_coroutine_threadsafe")?;
    // Clone the captured context for each concurrent subscription. A single
    // `Context` cannot be entered from two threads at once.
    let context = locals.context(py).call_method0("copy")?;
    let future = Arc::new(
        context
            .call_method1(
                "run",
                (run_coroutine_threadsafe, coroutine, locals.event_loop(py)),
            )?
            .unbind(),
    );
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let completion = Py::new(
        py,
        PythonFutureCompletion {
            sender: SyncMutex::new(Some(sender)),
        },
    )?;
    if let Err(error) = future
        .bind(py)
        .call_method1("add_done_callback", (completion,))
    {
        let _ = future.bind(py).call_method0("cancel");
        return Err(error);
    }
    if let Some(active) = active.as_ref() {
        active.set_pending(Arc::clone(&future));
    }
    Ok((
        PendingPythonFuture {
            future,
            active,
            complete: false,
        },
        receiver,
    ))
}

impl PythonAsyncIterator {
    fn from_source(
        source: Arc<Py<PyAny>>,
        locals: Arc<TaskLocals>,
        registry: &PythonSourceRegistry,
    ) -> Result<Self, String> {
        let iterator = Python::with_gil(|py| {
            source
                .bind(py)
                .call_method0("__aiter__")
                .map(|iterator| Arc::new(iterator.unbind()))
        })
        .map_err(|error| format!("stream source is not an async iterator: {error}"))?;
        Ok(Self {
            active: registry.register(iterator),
            locals,
        })
    }

    async fn next(&self) -> PyResult<Option<Py<PyAny>>> {
        let Some(iterator) = self.active.python() else {
            return Ok(None);
        };
        let (pending, receiver) = Python::with_gil(|py| {
            let coroutine = iterator.bind(py).call_method0("__anext__")?;
            schedule_python(
                py,
                self.locals.as_ref(),
                coroutine,
                Some(Arc::clone(&self.active)),
            )
        })?;
        let result = receiver.await.map_err(|_| {
            PyRuntimeError::new_err("Python stream task ended without reporting a result")
        })?;
        pending.finish();
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error)
                if Python::with_gil(|py| error.is_instance_of::<PyStopAsyncIteration>(py)) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    async fn close(&self) {
        let Some(iterator) = self.active.python() else {
            return;
        };
        let Some(scheduled) = Python::with_gil(|py| {
            let close = iterator.bind(py).getattr("aclose").ok()?;
            let coroutine = close.call0().ok()?;
            schedule_python(py, self.locals.as_ref(), coroutine, None).ok()
        }) else {
            self.active.clear();
            return;
        };
        let (pending, receiver) = scheduled;
        if let Ok(result) = tokio::time::timeout(PYTHON_SOURCE_CLOSE_TIMEOUT, receiver).await {
            let _ = result;
            pending.finish();
        }
        self.active.clear();
    }
}

struct SourceGuard {
    iterator: Option<PythonAsyncIterator>,
    cleanup: PythonSourceRegistry,
}

impl Drop for SourceGuard {
    fn drop(&mut self) {
        if let Some(iterator) = self.iterator.take() {
            self.cleanup.schedule_close(iterator);
        }
    }
}

fn python_source<T>(
    source: Arc<Py<PyAny>>,
    locals: Arc<TaskLocals>,
    registry: &PythonSourceRegistry,
    convert: fn(&PyStreamItem) -> Result<StreamItem<T>, String>,
) -> Result<SourceStream<T>, String>
where
    T: Send + 'static,
{
    let iterator = PythonAsyncIterator::from_source(source, locals, registry)?;
    let state = SourceGuard {
        iterator: Some(iterator),
        cleanup: registry.clone(),
    };
    let stream = futures::stream::unfold(state, move |mut state| async move {
        let iterator = state.iterator.as_ref()?;
        match iterator.next().await {
            Ok(Some(value)) => {
                let item = Python::with_gil(|py| {
                    value
                        .bind(py)
                        .extract::<PyRef<PyStreamItem>>()
                        .map_err(|error| format!("stream source must yield StreamItem: {error}"))
                        .and_then(|item| convert(&item))
                });
                match item {
                    Ok(item) => Some((Ok(item), state)),
                    Err(error) => {
                        if let Some(iterator) = state.iterator.take() {
                            iterator.close().await;
                        }
                        Some((Err(error), state))
                    }
                }
            }
            Ok(None) => {
                if let Some(iterator) = state.iterator.take() {
                    iterator.close().await;
                }
                None
            }
            Err(error) => {
                let detail = Python::with_gil(|py| {
                    let detail = error.to_string();
                    error.print_and_set_sys_last_vars(py);
                    detail
                });
                if let Some(iterator) = state.iterator.take() {
                    iterator.close().await;
                }
                Some((Err(detail), state))
            }
        }
    });
    Ok(Box::pin(stream.fuse()))
}

macro_rules! typed_item {
    ($name:ident, $variant:ident, $ty:ty) => {
        fn $name(item: &PyStreamItem) -> Result<StreamItem<$ty>, String> {
            match &item.payload {
                StreamPayload::$variant(value) => Ok(StreamItem {
                    timestamp_ns: item.timestamp_ns,
                    payload: value.inner.clone(),
                }),
                other => Err(format!(
                    "typed stream source expected {} but received {}",
                    stringify!($variant),
                    other.kind_name(),
                )),
            }
        }
    };
}

typed_item!(camera_item, Camera, CameraFrame);
typed_item!(point_cloud_item, PointCloud, PointCloudFrame);
typed_item!(joint_encoders_item, JointEncoders, JointEncodersFrame);
typed_item!(audio_item, Audio, AudioFrame);
typed_item!(scalar_item, Scalar, ScalarFrame);
typed_item!(detection_item, Detection, DetectionFrame);
typed_item!(pose_item, Pose, SpatialTransform);
typed_item!(map_item, Map, MapUpdate);

#[derive(Clone)]
struct RawLogBytes(Vec<u8>);

impl LogPayload for RawLogBytes {
    fn encode(&self) -> Vec<u8> {
        self.0.clone()
    }
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        Ok(Self(bytes.to_vec()))
    }
}

struct RetainedState {
    historical: VecDeque<auki_logs_rs::Entry<RawLogBytes>>,
    tail: Option<auki_logs_rs::TailIter<RawLogBytes>>,
}

fn retained_source<T>(
    source: RetainedSource,
    read_from: ReadFrom,
    decode: fn(&[u8]) -> Result<T, String>,
) -> Result<SourceStream<T>, String>
where
    T: Send + 'static,
{
    // Open the tail first so writes after this point are observable. This
    // preserves the established retained-source handoff semantics.
    let tail = RetainedLog::<RawLogBytes>::tail(&source.root)
        .map_err(|error| format!("stream source tail: {error}"))?;
    let historical = match read_from {
        ReadFrom::Latest => Vec::new(),
        ReadFrom::FromStart => RetainedLog::<RawLogBytes>::read(&source.root)
            .and_then(|reader| reader.entries())
            .map_err(|error| format!("stream source read: {error}"))?,
        ReadFrom::FromTimestamp(start_ns) => RetainedLog::<RawLogBytes>::read(&source.root)
            .and_then(|reader| reader.entries())
            .map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| entry.timestamp_ns >= start_ns)
                    .collect()
            })
            .map_err(|error| format!("stream source read: {error}"))?,
    };
    let state = RetainedState {
        historical: historical.into(),
        tail: Some(tail),
    };
    let stream = futures::stream::unfold(state, move |mut state| async move {
        if let Some(entry) = state.historical.pop_front() {
            let item = decode(&entry.payload.0).map(|payload| StreamItem {
                timestamp_ns: entry.timestamp_ns,
                payload,
            });
            return Some((item, state));
        }

        loop {
            let result = state.tail.as_mut()?.try_next();
            match result {
                Ok(Some(entry)) => {
                    let item = decode(&entry.payload.0).map(|payload| StreamItem {
                        timestamp_ns: entry.timestamp_ns,
                        payload,
                    });
                    return Some((item, state));
                }
                Ok(None) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(error) => {
                    state.tail.take();
                    return Some((Err(error.to_string()), state));
                }
            }
        }
    });
    Ok(Box::pin(stream))
}

fn decode_message<T: Message + Default>(bytes: &[u8]) -> Result<T, String> {
    T::decode(bytes).map_err(|error| error.to_string())
}

fn decline(detail: impl Into<String>) -> StreamDispatch {
    StreamDispatch::Decline {
        reason: DeclineReason::other(detail.into()),
    }
}

pub(crate) fn dispatch_python_stream(
    callback: Arc<Py<PyAny>>,
    locals: Arc<TaskLocals>,
    registry: PythonSourceRegistry,
    requester: PeerId,
    request: StreamRequest,
) -> StreamDispatch {
    let read_from = request.from;
    let decision = Python::with_gil(|py| -> Result<DecisionInner, String> {
        let value = callback
            .bind(py)
            .call1((requester.to_string(), PyStreamRequest { inner: request }))
            .map_err(|error| format!("stream provider raised: {error}"))?;
        value
            .extract::<PyRef<PyStreamDecision>>()
            .map_err(|error| format!("stream provider returned non-StreamDecision: {error}"))?
            .take()
            .ok_or_else(|| "stream provider returned an already-consumed StreamDecision".into())
    });

    let decision = match decision {
        Ok(decision) => decision,
        Err(error) => {
            tracing::warn!(%error, "Python stream provider declined after callback failure");
            return decline(error);
        }
    };

    match decision {
        DecisionInner::Camera { manifest, source } => {
            match python_source(source, Arc::clone(&locals), &registry, camera_item) {
                Ok(source) => StreamDispatch::AcceptCamera {
                    manifest: manifest.inner,
                    source,
                },
                Err(error) => decline(error),
            }
        }
        DecisionInner::PointCloud { manifest, source } => {
            match python_source(source, Arc::clone(&locals), &registry, point_cloud_item) {
                Ok(source) => StreamDispatch::AcceptPointCloud {
                    manifest: manifest.inner,
                    source,
                },
                Err(error) => decline(error),
            }
        }
        DecisionInner::JointEncoders { manifest, source } => {
            match python_source(source, Arc::clone(&locals), &registry, joint_encoders_item) {
                Ok(source) => StreamDispatch::AcceptJointEncoders {
                    manifest: manifest.inner,
                    source,
                },
                Err(error) => decline(error),
            }
        }
        DecisionInner::Audio { manifest, source } => {
            match python_source(source, Arc::clone(&locals), &registry, audio_item) {
                Ok(source) => StreamDispatch::AcceptAudio {
                    manifest: manifest.inner,
                    source,
                },
                Err(error) => decline(error),
            }
        }
        DecisionInner::Scalar { manifest, source } => {
            match python_source(source, Arc::clone(&locals), &registry, scalar_item) {
                Ok(source) => StreamDispatch::AcceptScalar {
                    manifest: manifest.inner,
                    source,
                },
                Err(error) => decline(error),
            }
        }
        DecisionInner::Detection { manifest, source } => {
            match python_source(source, Arc::clone(&locals), &registry, detection_item) {
                Ok(source) => StreamDispatch::AcceptDetection {
                    manifest: manifest.inner,
                    source,
                },
                Err(error) => decline(error),
            }
        }
        DecisionInner::Pose { manifest, source } => {
            match python_source(source, Arc::clone(&locals), &registry, pose_item) {
                Ok(source) => StreamDispatch::AcceptPose {
                    manifest: manifest.inner,
                    source,
                },
                Err(error) => decline(error),
            }
        }
        DecisionInner::Map { manifest, source } => {
            match python_source(source, locals, &registry, map_item) {
                Ok(source) => StreamDispatch::AcceptMap {
                    manifest: manifest.inner,
                    source,
                },
                Err(error) => decline(error),
            }
        }
        DecisionInner::Retained { manifest, source } => {
            let payload_kind = source.payload_kind.clone();
            match payload_kind.as_str() {
                "camera" => {
                    match retained_source(*source, read_from, decode_message::<CameraFrame>) {
                        Ok(source) => StreamDispatch::AcceptCamera {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(error) => decline(error),
                    }
                }
                "pointcloud" | "point_cloud" => {
                    match retained_source(*source, read_from, decode_message::<PointCloudFrame>) {
                        Ok(source) => StreamDispatch::AcceptPointCloud {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(error) => decline(error),
                    }
                }
                "joint_encoders" => {
                    match retained_source(*source, read_from, decode_message::<JointEncodersFrame>)
                    {
                        Ok(source) => StreamDispatch::AcceptJointEncoders {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(error) => decline(error),
                    }
                }
                "audio" => {
                    match retained_source(*source, read_from, decode_message::<AudioFrame>) {
                        Ok(source) => StreamDispatch::AcceptAudio {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(error) => decline(error),
                    }
                }
                "scalar" => {
                    match retained_source(*source, read_from, decode_message::<ScalarFrame>) {
                        Ok(source) => StreamDispatch::AcceptScalar {
                            manifest: manifest.inner,
                            source,
                        },
                        Err(error) => decline(error),
                    }
                }
                "map" => match retained_source(*source, read_from, decode_message::<MapUpdate>) {
                    Ok(source) => StreamDispatch::AcceptMap {
                        manifest: manifest.inner,
                        source,
                    },
                    Err(error) => decline(error),
                },
                other => decline(format!("unsupported retained payload kind {other:?}")),
            }
        }
        DecisionInner::Decline { reason } => StreamDispatch::Decline {
            reason: reason.inner,
        },
    }
}

#[pyclass(name = "StreamEntry", frozen)]
pub(crate) struct PyStreamEntry {
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

    #[getter]
    fn payload(&self, py: Python<'_>) -> PyObject {
        self.payload.clone_into_python(py)
    }

    #[getter]
    fn payload_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.payload.encode())
    }
}

#[pyclass(name = "StreamSubscription")]
pub(crate) struct PyStreamSubscription {
    manifest: PyStreamManifest,
    payload_kind: String,
    entries: std::sync::Arc<AsyncMutex<RawEntries>>,
    cancellation: CancellationToken,
}

#[pymethods]
impl PyStreamSubscription {
    #[getter]
    fn manifest(&self) -> PyStreamManifest {
        self.manifest.clone()
    }

    #[getter]
    fn payload_kind(&self) -> &str {
        &self.payload_kind
    }

    fn recv<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let entries = std::sync::Arc::clone(&self.entries);
        let cancellation = self.cancellation.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let item = tokio::select! {
                biased;
                _ = cancellation.cancelled() => None,
                item = async { entries.lock().await.next().await } => item,
            };
            match item {
                Some(Ok(item)) => Python::with_gil(|py| {
                    Py::new(
                        py,
                        PyStreamEntry {
                            timestamp_ns: item.timestamp_ns,
                            seq: item.seq,
                            payload: item.payload,
                        },
                    )
                    .map(Py::into_any)
                }),
                Some(Err(error)) => Err(runtime_error(error)),
                None => Ok(Python::with_gil(|py| py.None())),
            }
        })
    }
}

impl Drop for PyStreamSubscription {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

pub(crate) async fn open(
    domain: &Domain,
    expected_peer: PeerId,
    request: StreamRequest,
    payload_kind: &str,
) -> PyResult<PyStreamSubscription> {
    let subscription = match payload_kind {
        "camera" => erase(
            domain
                .open_stream::<CameraFrame>(expected_peer, request)
                .await
                .map_err(runtime_error)?,
            payload_kind,
            |inner| StreamPayload::Camera(PyCameraFrame { inner }),
        ),
        "point_cloud" => erase(
            domain
                .open_stream::<PointCloudFrame>(expected_peer, request)
                .await
                .map_err(runtime_error)?,
            payload_kind,
            |inner| StreamPayload::PointCloud(PyPointCloudFrame { inner }),
        ),
        "joint_encoders" => erase(
            domain
                .open_stream::<JointEncodersFrame>(expected_peer, request)
                .await
                .map_err(runtime_error)?,
            payload_kind,
            |inner| StreamPayload::JointEncoders(PyJointEncodersFrame { inner }),
        ),
        "audio" => erase(
            domain
                .open_stream::<AudioFrame>(expected_peer, request)
                .await
                .map_err(runtime_error)?,
            payload_kind,
            |inner| StreamPayload::Audio(PyAudioFrame { inner }),
        ),
        "scalar" => erase(
            domain
                .open_stream::<ScalarFrame>(expected_peer, request)
                .await
                .map_err(runtime_error)?,
            payload_kind,
            |inner| StreamPayload::Scalar(PyScalarFrame { inner }),
        ),
        "detection" => erase(
            domain
                .open_stream::<DetectionFrame>(expected_peer, request)
                .await
                .map_err(runtime_error)?,
            payload_kind,
            |inner| StreamPayload::Detection(PyDetectionFrame { inner }),
        ),
        "pose" => erase(
            domain
                .open_stream::<SpatialTransform>(expected_peer, request)
                .await
                .map_err(runtime_error)?,
            payload_kind,
            |inner| StreamPayload::Pose(PySpatialTransformFrame { inner }),
        ),
        "map" => erase(
            domain
                .open_stream::<MapUpdate>(expected_peer, request)
                .await
                .map_err(runtime_error)?,
            payload_kind,
            |inner| StreamPayload::Map(PyMapUpdateFrame { inner }),
        ),
        _ => {
            return Err(PyValueError::new_err(format!(
                "unsupported payload_kind {payload_kind:?}; expected camera, point_cloud, joint_encoders, audio, scalar, detection, pose, or map"
            )));
        }
    };
    Ok(subscription)
}

pub(crate) async fn open_map(
    domain: &Domain,
    expected_peer: PeerId,
    resource: &PyMapLogResource,
    from: auki_domain_rs::ReadFrom,
) -> PyResult<PyStreamSubscription> {
    let subscription = domain
        .open_map_stream(expected_peer, &resource.inner, from)
        .await
        .map_err(runtime_error)?;
    Ok(erase(subscription, "map", |inner| {
        StreamPayload::Map(PyMapUpdateFrame { inner })
    }))
}

fn erase<T>(
    subscription: StreamSubscription<T>,
    payload_kind: &str,
    wrap: fn(T) -> StreamPayload,
) -> PyStreamSubscription
where
    T: Send + 'static,
{
    let manifest = PyStreamManifest {
        inner: subscription.manifest,
    };
    let entries = subscription.entries.map(move |result| {
        result
            .map(
                |StreamEntry {
                     timestamp_ns,
                     seq,
                     payload,
                 }| RawEntry {
                    timestamp_ns,
                    seq,
                    payload: wrap(payload),
                },
            )
            .map_err(|error| error.to_string())
    });
    PyStreamSubscription {
        manifest,
        payload_kind: payload_kind.to_string(),
        entries: std::sync::Arc::new(AsyncMutex::new(Box::pin(entries))),
        cancellation: CancellationToken::new(),
    }
}

pub(crate) fn infer_payload_kind(
    entries: &[ResourceEntry],
    source_peer_id: &str,
    resource_id: &str,
) -> PyResult<&'static str> {
    for entry in entries {
        if entry.resource_id != resource_id
            || (!source_peer_id.is_empty() && entry.source_peer_id != source_peer_id)
        {
            continue;
        }
        return match &entry.variant_content {
            VariantContent::SensorLog { .. } => match entry.sensor.as_ref().map(|s| s.kind) {
                Some(SensorKind::Camera) => Ok("camera"),
                Some(SensorKind::Audio) => Ok("audio"),
                Some(SensorKind::Scalar) => Ok("scalar"),
                Some(SensorKind::JointEncoders) => Ok("joint_encoders"),
                Some(SensorKind::Rangefinder | SensorKind::Rf) => Ok("point_cloud"),
                None => Err(PyValueError::new_err(
                    "sensor_log resource has no sensor metadata",
                )),
            },
            VariantContent::PoseLog { .. } => Ok("pose"),
            VariantContent::DetectionLog { .. } => Ok("detection"),
            VariantContent::TimeTransformLog { .. } => Err(PyValueError::new_err(
                "time-transform resources do not have a retained typed Python payload",
            )),
        };
    }
    Err(PyFileNotFoundError::new_err(format!(
        "resource {source_peer_id:?}/{resource_id:?} is absent from the authenticated catalog"
    )))
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyStreamManifest>()?;
    module.add_class::<PyStreamManifestBuilder>()?;
    module.add_class::<PyDynamicIntrinsics>()?;
    module.add_class::<PyCameraFrame>()?;
    module.add_class::<PyPointCloudFrame>()?;
    module.add_class::<PyJointEncodersFrame>()?;
    module.add_class::<PyAudioFrame>()?;
    module.add_class::<PyScalarFrame>()?;
    module.add_class::<PyDetectionFrame>()?;
    module.add_class::<PySpatialTransformFrame>()?;
    module.add_class::<PyMapUpdateFrame>()?;
    module.add_class::<PyDeclineReason>()?;
    module.add_class::<PyStreamItem>()?;
    module.add_class::<PyStreamDecision>()?;
    module.add_class::<PyStreamEntry>()?;
    module.add_class::<PyStreamSubscription>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    struct PythonLoop {
        locals: Arc<TaskLocals>,
        event_loop: Py<PyAny>,
        thread: Py<PyAny>,
        marker: Py<PyAny>,
    }

    impl PythonLoop {
        fn start() -> PyResult<Self> {
            Python::with_gil(|py| {
                let asyncio = py.import_bound("asyncio")?;
                let event_loop = asyncio.call_method0("new_event_loop")?;
                let marker = py
                    .import_bound("contextvars")?
                    .getattr("ContextVar")?
                    .call1(("auki_stream_test",))?;
                marker.call_method1("set", ("captured",))?;
                let locals = TaskLocals::new(event_loop.clone()).copy_context(py)?;
                marker.call_method1("set", ("outside",))?;
                let threading = py.import_bound("threading")?;
                let kwargs = pyo3::types::PyDict::new_bound(py);
                kwargs.set_item("target", event_loop.getattr("run_forever")?)?;
                kwargs.set_item("daemon", true)?;
                let thread = threading.call_method("Thread", (), Some(&kwargs))?;
                thread.call_method0("start")?;
                Ok(Self {
                    locals: Arc::new(locals),
                    event_loop: event_loop.unbind(),
                    thread: thread.unbind(),
                    marker: marker.unbind(),
                })
            })
        }

        fn stop(self) {
            Python::with_gil(|py| {
                let stop = self.event_loop.bind(py).getattr("stop").unwrap();
                self.event_loop
                    .bind(py)
                    .call_method1("call_soon_threadsafe", (stop,))
                    .unwrap();
                self.thread.bind(py).call_method1("join", (1.0,)).unwrap();
            });
        }
    }

    #[test]
    fn suspended_async_generator_is_cancelled_when_native_source_is_dropped() {
        pyo3::prepare_freethreaded_python();
        let python_loop = PythonLoop::start().expect("start Python event loop");
        let (source, finished) = Python::with_gil(|py| -> PyResult<_> {
            let item = Py::new(
                py,
                PyStreamItem {
                    timestamp_ns: 42,
                    payload: StreamPayload::Camera(PyCameraFrame {
                        inner: CameraFrame {
                            frame: b"frame".to_vec(),
                            ..Default::default()
                        },
                    }),
                },
            )?;
            let finished = py.import_bound("threading")?.call_method0("Event")?;
            let module = PyModule::from_code_bound(
                py,
                "import asyncio\nasync def source(item, finished, marker):\n    assert marker.get() == 'captured'\n    try:\n        yield item\n        await asyncio.Event().wait()\n    finally:\n        finished.set()\n",
                "stream_cancel_test.py",
                "stream_cancel_test",
            )?;
            let source = module
                .getattr("source")?
                .call1((item, finished.clone(), python_loop.marker.bind(py)))?
                .unbind();
            Ok((Arc::new(source), finished.unbind()))
        })
        .expect("build Python async generator");

        let registry = PythonSourceRegistry::default();
        let native = python_source(
            source,
            Arc::clone(&python_loop.locals),
            &registry,
            camera_item,
        )
        .expect("bridge async generator");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let task = tokio::spawn(async move {
                let mut native = native;
                let first = native.next().await.expect("first source item").unwrap();
                assert_eq!(first.timestamp_ns, 42);
                let _ = native.next().await;
            });
            tokio::time::sleep(Duration::from_millis(50)).await;
            task.abort();
            let _ = task.await;
            registry.shutdown().await;

            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let is_set = Python::with_gil(|py| {
                    finished
                        .bind(py)
                        .call_method0("is_set")
                        .unwrap()
                        .extract::<bool>()
                        .unwrap()
                });
                if is_set {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "generator finally block did not run"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        python_loop.stop();
    }

    #[test]
    fn yielded_async_generator_is_closed_when_native_source_drops_between_polls() {
        pyo3::prepare_freethreaded_python();
        let python_loop = PythonLoop::start().expect("start Python event loop");
        let (source, finished) = Python::with_gil(|py| -> PyResult<_> {
            let item = Py::new(
                py,
                PyStreamItem {
                    timestamp_ns: 7,
                    payload: StreamPayload::Camera(PyCameraFrame {
                        inner: CameraFrame {
                            frame: b"one-frame".to_vec(),
                            ..Default::default()
                        },
                    }),
                },
            )?;
            let finished = py.import_bound("threading")?.call_method0("Event")?;
            let module = PyModule::from_code_bound(
                py,
                "async def source(item, finished):\n    try:\n        yield item\n    finally:\n        finished.set()\n",
                "stream_between_polls_close_test.py",
                "stream_between_polls_close_test",
            )?;
            let source = module
                .getattr("source")?
                .call1((item, finished.clone()))?
                .unbind();
            Ok((Arc::new(source), finished.unbind()))
        })
        .expect("build Python async generator");

        let registry = PythonSourceRegistry::default();
        let native = python_source(
            source,
            Arc::clone(&python_loop.locals),
            &registry,
            camera_item,
        )
        .expect("bridge async generator");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let mut native = native;
            let first = native.next().await.expect("first source item").unwrap();
            assert_eq!(first.timestamp_ns, 7);
            drop(native);

            registry.shutdown().await;
            assert!(Python::with_gil(|py| {
                finished
                    .bind(py)
                    .call_method0("is_set")
                    .unwrap()
                    .extract::<bool>()
                    .unwrap()
            }));
        });
        python_loop.stop();
    }
}
