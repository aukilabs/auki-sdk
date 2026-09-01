//! Python client and provider-backed endpoint for typed Stream v2.

use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use auki_datatypes::{
    audio, camera::CameraFrame, detection::DetectionFrame, joint_encoders, map::MapUpdate,
    point_cloud, pose, scalar,
};
use auki_protocols::stream::{
    SourceStream, StreamClient, StreamDispatch, StreamEndpoint, StreamEndpointError, StreamEntry,
    StreamError, StreamItem, StreamPayload, StreamProvider, StreamSubscription,
    SubscriptionEntries,
    v2::{DeclineReason, EndReason, ID, ReadFrom, StreamManifest, StreamRequest, end_reason},
};
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use prost::Message;
use pyo3::{
    exceptions::{
        PyAttributeError, PyRuntimeError, PyStopAsyncIteration, PyTypeError, PyValueError,
    },
    prelude::*,
    pyclass::{PyTraverseError, PyVisit},
    types::{PyAny, PyBytes, PyDict, PyModule},
};
use pyo3_async_runtimes::TaskLocals;
use serde::{Deserialize, Serialize};
use tokio::{sync::watch, task::JoinHandle};

use crate::{
    PyAukiPeer,
    cleanup::{CleanupResult, DetachedCleanup, wait_cleanup},
};

use super::support::{
    CancelablePythonAwaitable, CompletionHook, PythonCallback, PythonTaskState,
    enter_tokio_runtime, parse_peer_id, parse_python, parse_target, report_provider_error,
    requester_to_python, require_callable, runtime_error, to_python,
};

const PAYLOAD_KIND_NAMES: &str =
    "camera, point_cloud, joint_encoders, audio, scalar, pose, detection, or map";
const PYTHON_SOURCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamPayloadKind {
    Camera,
    PointCloud,
    JointEncoders,
    Audio,
    Scalar,
    Pose,
    Detection,
    Map,
}

impl StreamPayloadKind {
    fn parse(value: &str) -> PyResult<Self> {
        match value {
            "camera" => Ok(Self::Camera),
            "point_cloud" => Ok(Self::PointCloud),
            "joint_encoders" => Ok(Self::JointEncoders),
            "audio" => Ok(Self::Audio),
            "scalar" => Ok(Self::Scalar),
            "pose" => Ok(Self::Pose),
            "detection" => Ok(Self::Detection),
            "map" => Ok(Self::Map),
            _ => Err(PyValueError::new_err(format!(
                "Stream payload_kind must be one of {PAYLOAD_KIND_NAMES}; got {value:?}"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::PointCloud => "point_cloud",
            Self::JointEncoders => "joint_encoders",
            Self::Audio => "audio",
            Self::Scalar => "scalar",
            Self::Pose => "pose",
            Self::Detection => "detection",
            Self::Map => "map",
        }
    }

    fn manifest_payload(self) -> &'static str {
        match self {
            Self::Camera => "camera_frame",
            Self::PointCloud => "point_cloud_frame",
            Self::JointEncoders => "joint_encoders_frame",
            Self::Audio => "audio_frame",
            Self::Scalar => "scalar",
            Self::Pose => "spatial_transform",
            Self::Detection => "detection",
            Self::Map => "map_update",
        }
    }

    fn validate_manifest(self, manifest: &StreamManifest) -> PyResult<()> {
        let expected = self.manifest_payload();
        if manifest.payload == expected {
            Ok(())
        } else {
            Err(PyValueError::new_err(format!(
                "Stream payload_kind {:?} requires manifest payload {expected:?}, got {:?}",
                self.as_str(),
                manifest.payload
            )))
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StreamReadFromRecord {
    #[default]
    Latest,
    FromStart,
    FromTimestamp {
        timestamp_ns: i64,
    },
}

impl From<ReadFrom> for StreamReadFromRecord {
    fn from(value: ReadFrom) -> Self {
        match value {
            ReadFrom::Latest => Self::Latest,
            ReadFrom::FromStart => Self::FromStart,
            ReadFrom::FromTimestamp(timestamp_ns) => Self::FromTimestamp { timestamp_ns },
        }
    }
}

impl From<StreamReadFromRecord> for ReadFrom {
    fn from(value: StreamReadFromRecord) -> Self {
        match value {
            StreamReadFromRecord::Latest => Self::Latest,
            StreamReadFromRecord::FromStart => Self::FromStart,
            StreamReadFromRecord::FromTimestamp { timestamp_ns } => {
                Self::FromTimestamp(timestamp_ns)
            }
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamRequestRecord {
    source_peer_id: String,
    resource_id: String,
    #[serde(default)]
    from: StreamReadFromRecord,
}

impl From<&StreamRequest> for StreamRequestRecord {
    fn from(request: &StreamRequest) -> Self {
        Self {
            source_peer_id: request.source_peer_id.clone(),
            resource_id: request.resource_id.clone(),
            from: request.from.into(),
        }
    }
}

impl From<StreamRequestRecord> for StreamRequest {
    fn from(request: StreamRequestRecord) -> Self {
        Self {
            source_peer_id: request.source_peer_id,
            resource_id: request.resource_id,
            from: request.from.into(),
        }
    }
}

fn request_from_python(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<StreamRequest> {
    parse_python::<StreamRequestRecord>(py, value, "Stream request").map(StreamRequest::from)
}

fn request_to_python(py: Python<'_>, request: &StreamRequest) -> PyResult<PyObject> {
    to_python(py, &StreamRequestRecord::from(request))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct StreamManifestRecord {
    sensor_id: String,
    sensor_hash: String,
    clock_peer_id: String,
    clock_id: String,
    clock_hash: String,
    frame_id: String,
    frame_hash: String,
    resource_id: String,
    payload: String,
    from_frame_id: String,
    from_frame_hash: String,
    to_frame_id: String,
    to_frame_hash: String,
    writer_mode: String,
    expected_rate_hz: u32,
    map_peer_id: String,
    map_id: String,
    map_hash: String,
}

impl From<StreamManifestRecord> for StreamManifest {
    fn from(manifest: StreamManifestRecord) -> Self {
        Self {
            sensor_id: manifest.sensor_id,
            sensor_hash: manifest.sensor_hash,
            clock_peer_id: manifest.clock_peer_id,
            clock_id: manifest.clock_id,
            clock_hash: manifest.clock_hash,
            frame_id: manifest.frame_id,
            frame_hash: manifest.frame_hash,
            resource_id: manifest.resource_id,
            payload: manifest.payload,
            from_frame_id: manifest.from_frame_id,
            from_frame_hash: manifest.from_frame_hash,
            to_frame_id: manifest.to_frame_id,
            to_frame_hash: manifest.to_frame_hash,
            writer_mode: manifest.writer_mode,
            expected_rate_hz: manifest.expected_rate_hz,
            map_peer_id: manifest.map_peer_id,
            map_id: manifest.map_id,
            map_hash: manifest.map_hash,
        }
    }
}

impl From<&StreamManifest> for StreamManifestRecord {
    fn from(manifest: &StreamManifest) -> Self {
        Self {
            sensor_id: manifest.sensor_id.clone(),
            sensor_hash: manifest.sensor_hash.clone(),
            clock_peer_id: manifest.clock_peer_id.clone(),
            clock_id: manifest.clock_id.clone(),
            clock_hash: manifest.clock_hash.clone(),
            frame_id: manifest.frame_id.clone(),
            frame_hash: manifest.frame_hash.clone(),
            resource_id: manifest.resource_id.clone(),
            payload: manifest.payload.clone(),
            from_frame_id: manifest.from_frame_id.clone(),
            from_frame_hash: manifest.from_frame_hash.clone(),
            to_frame_id: manifest.to_frame_id.clone(),
            to_frame_hash: manifest.to_frame_hash.clone(),
            writer_mode: manifest.writer_mode.clone(),
            expected_rate_hz: manifest.expected_rate_hz,
            map_peer_id: manifest.map_peer_id.clone(),
            map_id: manifest.map_id.clone(),
            map_hash: manifest.map_hash.clone(),
        }
    }
}

fn manifest_from_python(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<StreamManifest> {
    parse_python::<StreamManifestRecord>(py, value, "Stream manifest").map(StreamManifest::from)
}

fn manifest_to_python(py: Python<'_>, manifest: &StreamManifest) -> PyResult<PyObject> {
    to_python(py, &StreamManifestRecord::from(manifest))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StreamDeclineReasonRecord {
    SensorNotFound,
    SensorUnavailable,
    ProducerShuttingDown,
    Other { detail: String },
}

fn decline_reason_from_python(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<DeclineReason> {
    let reason: StreamDeclineReasonRecord = parse_python(py, value, "Stream decline reason")?;
    Ok(match reason {
        StreamDeclineReasonRecord::SensorNotFound => DeclineReason::sensor_not_found(),
        StreamDeclineReasonRecord::SensorUnavailable => DeclineReason::sensor_unavailable(),
        StreamDeclineReasonRecord::ProducerShuttingDown => DeclineReason::producer_shutting_down(),
        StreamDeclineReasonRecord::Other { detail } => DeclineReason::other(detail),
    })
}

/// Outbound Stream v2 client backed by the portable Rust protocol.
#[pyclass(name = "AukiStreamClient", frozen)]
#[derive(Clone)]
pub(crate) struct PyAukiStreamClient {
    inner: StreamClient,
}

impl PyAukiStreamClient {
    fn from_inner(inner: StreamClient) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyAukiStreamClient {
    #[new]
    fn new(peer: &PyAukiPeer) -> Self {
        Self::from_inner(StreamClient::new(peer.protocols()))
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    /// Open one typed subscription through the owning peer's configured routes.
    fn subscribe<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        payload_kind: String,
        request: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let remote_peer_id = parse_peer_id(&remote_peer_id)?;
        let payload_kind = StreamPayloadKind::parse(&payload_kind)?;
        let request = request_from_python(py, request)?;
        subscribe_to_python(
            py,
            self.inner.clone(),
            SubscriptionTarget::Configured(remote_peer_id),
            payload_kind,
            request,
        )
    }

    /// Open one typed subscription through an exact authenticated route.
    fn subscribe_exact<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        route: String,
        payload_kind: String,
        request: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (remote_peer_id, route) = parse_target(&remote_peer_id, &route)?;
        let payload_kind = StreamPayloadKind::parse(&payload_kind)?;
        let request = request_from_python(py, request)?;
        subscribe_to_python(
            py,
            self.inner.clone(),
            SubscriptionTarget::Exact(remote_peer_id, route),
            payload_kind,
            request,
        )
    }
}

enum SubscriptionTarget {
    Configured(auki_sdk_rs::PeerId),
    Exact(auki_sdk_rs::PeerId, auki_sdk_rs::Multiaddr),
}

async fn subscribe_typed<T>(
    client: &StreamClient,
    target: SubscriptionTarget,
    request: StreamRequest,
) -> Result<StreamSubscription<T>, StreamEndpointError>
where
    T: StreamPayload,
{
    match target {
        SubscriptionTarget::Configured(remote_peer_id) => {
            client.subscribe::<T>(remote_peer_id, request).await
        }
        SubscriptionTarget::Exact(remote_peer_id, route) => {
            client
                .subscribe_exact::<T>(remote_peer_id, route, request)
                .await
        }
    }
}

async fn subscribe_kind(
    client: &StreamClient,
    target: SubscriptionTarget,
    payload_kind: StreamPayloadKind,
    request: StreamRequest,
) -> Result<AnySubscription, StreamEndpointError> {
    match payload_kind {
        StreamPayloadKind::Camera => subscribe_typed::<CameraFrame>(client, target, request)
            .await
            .map(|subscription| split_subscription(subscription, AnyEntries::Camera)),
        StreamPayloadKind::PointCloud => {
            subscribe_typed::<point_cloud::Data>(client, target, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::PointCloud))
        }
        StreamPayloadKind::JointEncoders => {
            subscribe_typed::<joint_encoders::Data>(client, target, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::JointEncoders))
        }
        StreamPayloadKind::Audio => subscribe_typed::<audio::Data>(client, target, request)
            .await
            .map(|subscription| split_subscription(subscription, AnyEntries::Audio)),
        StreamPayloadKind::Scalar => subscribe_typed::<scalar::Data>(client, target, request)
            .await
            .map(|subscription| split_subscription(subscription, AnyEntries::Scalar)),
        StreamPayloadKind::Pose => {
            subscribe_typed::<pose::SpatialTransform>(client, target, request)
                .await
                .map(|subscription| split_subscription(subscription, AnyEntries::Pose))
        }
        StreamPayloadKind::Detection => subscribe_typed::<DetectionFrame>(client, target, request)
            .await
            .map(|subscription| split_subscription(subscription, AnyEntries::Detection)),
        StreamPayloadKind::Map => subscribe_typed::<MapUpdate>(client, target, request)
            .await
            .map(|subscription| split_subscription(subscription, AnyEntries::Map)),
    }
}

fn subscribe_to_python<'py>(
    py: Python<'py>,
    client: StreamClient,
    target: SubscriptionTarget,
    payload_kind: StreamPayloadKind,
    request: StreamRequest,
) -> PyResult<Bound<'py, PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let subscription = subscribe_kind(&client, target, payload_kind, request)
            .await
            .map_err(|error| runtime_error("subscribe to Stream", error))?;
        payload_kind.validate_manifest(&subscription.manifest)?;
        Python::with_gil(|py| {
            Py::new(
                py,
                PyAukiStreamSubscription::new(
                    payload_kind,
                    subscription.manifest,
                    subscription.entries,
                ),
            )
        })
    })
}

struct PythonLoop {
    locals: Arc<TaskLocals>,
    event_loop: PythonCallback,
    context: PythonCallback,
}

impl PythonLoop {
    fn capture(py: Python<'_>) -> PyResult<Arc<Self>> {
        let locals = pyo3_async_runtimes::tokio::get_current_locals(py).map_err(|error| {
            PyRuntimeError::new_err(format!(
                "AukiStreamEndpoint.mount() must run inside an asyncio event loop: {error}"
            ))
        })?;
        Ok(Arc::new(Self {
            event_loop: Arc::new(locals.event_loop(py).unbind()),
            context: Arc::new(locals.context(py).unbind()),
            locals: Arc::new(locals),
        }))
    }

    fn traverse(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(self.event_loop.as_ref())?;
        visit.call(self.context.as_ref())?;
        Ok(())
    }
}

#[derive(Clone)]
struct PythonStreamProvider {
    callback: PythonCallback,
    python_loop: Arc<PythonLoop>,
    sources: PythonSourceRegistry,
}

impl StreamProvider for PythonStreamProvider {
    fn dispatch(
        &self,
        remote_peer: &auki_sdk_rs::AuthenticatedPeer,
        request: StreamRequest,
    ) -> StreamDispatch {
        match python_stream_dispatch(
            &self.callback,
            &self.python_loop,
            &self.sources,
            remote_peer,
            &request,
        ) {
            Ok(dispatch) => dispatch,
            Err(error) => {
                Python::with_gil(|py| {
                    report_provider_error(py, self.callback.bind(py), error);
                });
                StreamDispatch::Decline {
                    reason: DeclineReason::other("Python Stream provider failed"),
                }
            }
        }
    }
}

fn python_stream_dispatch(
    callback: &PythonCallback,
    python_loop: &Arc<PythonLoop>,
    sources: &PythonSourceRegistry,
    remote_peer: &auki_sdk_rs::AuthenticatedPeer,
    request: &StreamRequest,
) -> PyResult<StreamDispatch> {
    Python::with_gil(|py| {
        let requester = requester_to_python(py, remote_peer)?;
        let request = request_to_python(py, request)?;
        let context = python_loop.locals.context(py).call_method0("copy")?;
        let decision = context.call_method1("run", (callback.bind(py), requester, request))?;
        let decision = decision
            .downcast::<PyDict>()
            .map_err(|_| PyTypeError::new_err("Stream provider must return a decision dict"))?;
        let kind: String = required_item(decision, "kind")?.extract()?;
        match kind.as_str() {
            "decline" => {
                let reason = required_item(decision, "reason")?;
                Ok(StreamDispatch::Decline {
                    reason: decline_reason_from_python(py, &reason)?,
                })
            }
            "accept" => {
                let payload_kind: String = required_item(decision, "payload_kind")?.extract()?;
                let payload_kind = StreamPayloadKind::parse(&payload_kind)?;
                let manifest = manifest_from_python(py, &required_item(decision, "manifest")?)?;
                payload_kind.validate_manifest(&manifest)?;
                let source = Arc::new(required_item(decision, "source")?.unbind());
                accepted_dispatch(payload_kind, manifest, source, python_loop, sources)
            }
            _ => Err(PyValueError::new_err(format!(
                "Stream provider decision kind must be \"accept\" or \"decline\", got {kind:?}"
            ))),
        }
    })
}

fn required_item<'py>(
    value: &Bound<'py, PyDict>,
    name: &'static str,
) -> PyResult<Bound<'py, PyAny>> {
    value
        .get_item(name)?
        .ok_or_else(|| PyValueError::new_err(format!("Stream provider decision requires {name:?}")))
}

fn accepted_dispatch(
    payload_kind: StreamPayloadKind,
    manifest: StreamManifest,
    source: PythonCallback,
    python_loop: &Arc<PythonLoop>,
    sources: &PythonSourceRegistry,
) -> PyResult<StreamDispatch> {
    let locals = Arc::clone(&python_loop.locals);
    let source_error =
        |error: String| PyValueError::new_err(format!("invalid Stream provider source: {error}"));
    Ok(match payload_kind {
        StreamPayloadKind::Camera => StreamDispatch::AcceptCamera {
            manifest,
            source: python_source(source, locals, sources).map_err(source_error)?,
        },
        StreamPayloadKind::PointCloud => StreamDispatch::AcceptPointCloud {
            manifest,
            source: python_source(source, locals, sources).map_err(source_error)?,
        },
        StreamPayloadKind::JointEncoders => StreamDispatch::AcceptJointEncoders {
            manifest,
            source: python_source(source, locals, sources).map_err(source_error)?,
        },
        StreamPayloadKind::Audio => StreamDispatch::AcceptAudio {
            manifest,
            source: python_source(source, locals, sources).map_err(source_error)?,
        },
        StreamPayloadKind::Scalar => StreamDispatch::AcceptScalar {
            manifest,
            source: python_source(source, locals, sources).map_err(source_error)?,
        },
        StreamPayloadKind::Pose => StreamDispatch::AcceptPose {
            manifest,
            source: python_source(source, locals, sources).map_err(source_error)?,
        },
        StreamPayloadKind::Detection => StreamDispatch::AcceptDetection {
            manifest,
            source: python_source(source, locals, sources).map_err(source_error)?,
        },
        StreamPayloadKind::Map => StreamDispatch::AcceptMap {
            manifest,
            source: python_source(source, locals, sources).map_err(source_error)?,
        },
    })
}

struct ActivePythonSource {
    iterator: RwLock<Option<PythonCallback>>,
    pending: RwLock<Option<Arc<PythonTaskState>>>,
    pending_changed: watch::Sender<u64>,
}

impl ActivePythonSource {
    fn iterator(&self) -> Option<PythonCallback> {
        self.iterator.read().clone()
    }

    fn python_references(&self) -> Vec<PythonCallback> {
        let mut references = self.iterator.read().iter().cloned().collect::<Vec<_>>();
        if let Some(pending) = self.pending.read().as_ref() {
            references.extend(pending.python_references());
        }
        references
    }

    fn set_pending(&self, pending: Arc<PythonTaskState>) {
        self.pending.write().replace(pending);
        self.pending_changed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }

    fn clear_pending(&self) {
        self.pending.write().take();
        self.pending_changed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }

    fn cancel_pending(&self) {
        let pending = self.pending.read().clone();
        if let Some(pending) = pending {
            pending.cancel();
        }
    }

    async fn wait_pending(&self) {
        let mut changed = self.pending_changed.subscribe();
        loop {
            if self.pending.read().is_none() {
                return;
            }
            if changed.changed().await.is_err() {
                return;
            }
        }
    }

    fn clear(&self) {
        let iterator = self.iterator.write().take();
        let pending = self.pending.write().take();
        if let Some(pending) = pending {
            pending.cancel();
        }
        self.pending_changed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        drop(iterator);
    }
}

#[derive(Clone, Default)]
struct PythonSourceRegistry {
    sources: Arc<RwLock<Vec<Weak<ActivePythonSource>>>>,
    cleanup: Arc<Mutex<Vec<JoinHandle<()>>>>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl PythonSourceRegistry {
    fn register(&self, iterator: PythonCallback) -> Arc<ActivePythonSource> {
        let (pending_changed, _) = watch::channel(0);
        let active = Arc::new(ActivePythonSource {
            iterator: RwLock::new(Some(iterator)),
            pending: RwLock::new(None),
            pending_changed,
        });
        let mut sources = self.sources.write();
        sources.retain(|source| source.strong_count() > 0);
        sources.push(Arc::downgrade(&active));
        active
    }

    fn visit(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        let sources = self
            .sources
            .read()
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for source in sources {
            for reference in source.python_references() {
                visit.call(reference.as_ref())?;
            }
        }
        Ok(())
    }

    fn clear(&self) {
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
        let errors = Arc::clone(&self.errors);
        let handle = pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            if let Err(error) = iterator.close().await {
                errors.lock().push(error);
            }
        });
        let mut cleanup = self.cleanup.lock();
        cleanup.retain(|task| !task.is_finished());
        cleanup.push(handle);
    }

    fn record_error(&self, error: String) {
        self.errors.lock().push(error);
    }

    async fn shutdown(&self) -> Result<(), String> {
        loop {
            let tasks = std::mem::take(&mut *self.cleanup.lock());
            if tasks.is_empty() {
                break;
            }
            for task in tasks {
                if let Err(error) = task.await {
                    self.record_error(format!("Python Stream source cleanup task failed: {error}"));
                }
            }
        }
        self.clear();
        let errors = std::mem::take(&mut *self.errors.lock());
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

struct PythonAsyncIterator {
    active: Arc<ActivePythonSource>,
    locals: Arc<TaskLocals>,
}

impl PythonAsyncIterator {
    fn from_source(
        source: PythonCallback,
        locals: Arc<TaskLocals>,
        registry: &PythonSourceRegistry,
    ) -> Result<Self, String> {
        let iterator = Python::with_gil(|py| {
            let context = locals.context(py).call_method0("copy")?;
            let aiter = source.bind(py).getattr("__aiter__")?;
            let iterator = context.call_method1("run", (aiter,))?;
            if !iterator.hasattr("__anext__")? {
                return Err(PyTypeError::new_err(
                    "Stream source __aiter__() result must define __anext__()",
                ));
            }
            Ok(Arc::new(iterator.unbind()))
        })
        .map_err(|error| format!("source must be an AsyncIterable: {error}"))?;
        Ok(Self {
            active: registry.register(iterator),
            locals,
        })
    }

    async fn next(&self) -> PyResult<Option<Py<PyAny>>> {
        let Some(iterator) = self.active.iterator() else {
            return Ok(None);
        };
        let active = Arc::clone(&self.active);
        let completion_active = Arc::downgrade(&active);
        let completion_hook: CompletionHook = Arc::new(move || {
            if let Some(active) = completion_active.upgrade() {
                active.clear_pending();
            }
        });
        let pending = Python::with_gil(|py| {
            let context = self.locals.context(py).call_method0("copy")?;
            let anext = iterator.bind(py).getattr("__anext__")?;
            let awaitable = context.call_method1("run", (anext,))?;
            CancelablePythonAwaitable::schedule_with_completion(
                py,
                self.locals.as_ref(),
                awaitable,
                Some(completion_hook),
            )
        })?;
        let task = pending.state();
        active.set_pending(Arc::clone(&task));
        if task.is_completed() {
            active.clear_pending();
        }
        let result = pending.await;
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

    async fn close(&self) -> Result<(), String> {
        self.active.cancel_pending();
        if tokio::time::timeout(PYTHON_SOURCE_CLOSE_TIMEOUT, self.active.wait_pending())
            .await
            .is_err()
        {
            self.active.clear();
            return Err("timed out waiting for Python Stream source cancellation".into());
        }
        let Some(iterator) = self.active.iterator() else {
            return Ok(());
        };
        let scheduled = Python::with_gil(|py| -> PyResult<_> {
            let close = match iterator.bind(py).getattr("aclose") {
                Ok(close) => close,
                Err(error) if error.is_instance_of::<PyAttributeError>(py) => return Ok(None),
                Err(error) => return Err(error),
            };
            let context = self.locals.context(py).call_method0("copy")?;
            let awaitable = context.call_method1("run", (close,))?;
            CancelablePythonAwaitable::schedule(py, self.locals.as_ref(), awaitable).map(Some)
        });
        let scheduled = match scheduled {
            Ok(scheduled) => scheduled,
            Err(error) => {
                self.active.clear();
                return Err(format!("close Python Stream source: {error}"));
            }
        };
        let Some(pending) = scheduled else {
            self.active.clear();
            return Ok(());
        };
        let result = match tokio::time::timeout(PYTHON_SOURCE_CLOSE_TIMEOUT, pending).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(format!("close Python Stream source: {error}")),
            Err(_) => Err("timed out closing Python Stream source".into()),
        };
        self.active.clear();
        result
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
    source: PythonCallback,
    locals: Arc<TaskLocals>,
    registry: &PythonSourceRegistry,
) -> Result<SourceStream<T>, String>
where
    T: Message + Default + Send + 'static,
{
    let iterator = PythonAsyncIterator::from_source(source, locals, registry)?;
    let state = SourceGuard {
        iterator: Some(iterator),
        cleanup: registry.clone(),
    };
    let stream = futures::stream::unfold(state, |mut state| async move {
        let iterator = state.iterator.as_ref()?;
        match iterator.next().await {
            Ok(Some(value)) => {
                let item = Python::with_gil(|py| source_item_from_python::<T>(py, value.bind(py)));
                match item {
                    Ok(item) => Some((Ok(item), state)),
                    Err(error) => {
                        Python::with_gil(|py| {
                            if let Some(iterator) = state
                                .iterator
                                .as_ref()
                                .and_then(|iterator| iterator.active.iterator())
                            {
                                report_provider_error(
                                    py,
                                    iterator.bind(py),
                                    PyValueError::new_err(format!(
                                        "invalid Python Stream source item: {error}"
                                    )),
                                );
                            }
                        });
                        if let Some(iterator) = state.iterator.take()
                            && let Err(close_error) = iterator.close().await
                        {
                            state.cleanup.record_error(close_error);
                        }
                        Some((Err("Python Stream source failed".into()), state))
                    }
                }
            }
            Ok(None) => {
                if let Some(iterator) = state.iterator.take()
                    && let Err(close_error) = iterator.close().await
                {
                    state.cleanup.record_error(close_error);
                }
                None
            }
            Err(error) => {
                Python::with_gil(|py| {
                    if let Some(iterator) = state
                        .iterator
                        .as_ref()
                        .and_then(|iterator| iterator.active.iterator())
                    {
                        report_provider_error(py, iterator.bind(py), error);
                    }
                });
                if let Some(iterator) = state.iterator.take()
                    && let Err(close_error) = iterator.close().await
                {
                    state.cleanup.record_error(close_error);
                }
                Some((Err("Python Stream source failed".into()), state))
            }
        }
    });
    Ok(Box::pin(stream.fuse()))
}

fn source_item_from_python<T>(
    _py: Python<'_>,
    value: &Bound<'_, PyAny>,
) -> Result<StreamItem<T>, String>
where
    T: Message + Default,
{
    let item = value
        .downcast::<PyDict>()
        .map_err(|_| "Stream source must yield a dict".to_owned())?;
    let timestamp_ns = item
        .get_item("timestamp_ns")
        .map_err(|error| format!("read Stream source timestamp_ns: {error}"))?
        .ok_or_else(|| "Stream source item requires \"timestamp_ns\"".to_owned())?
        .extract::<i64>()
        .map_err(|error| format!("read Stream source timestamp_ns: {error}"))?;
    let payload = item
        .get_item("payload")
        .map_err(|error| format!("read Stream source payload: {error}"))?
        .ok_or_else(|| "Stream source item requires \"payload\"".to_owned())?;
    let payload = payload
        .downcast::<PyBytes>()
        .map_err(|_| "Stream source payload must be bytes containing one protobuf value")?;
    let payload = T::decode(payload.as_bytes())
        .map_err(|error| format!("decode typed Stream source payload: {error}"))?;
    Ok(StreamItem {
        timestamp_ns,
        payload,
    })
}

struct StreamEndpointOwner {
    endpoint: Mutex<Option<StreamEndpoint>>,
    callback: Mutex<Option<PythonCallback>>,
    python_loop: Mutex<Option<Arc<PythonLoop>>>,
    sources: PythonSourceRegistry,
    cleanup: DetachedCleanup,
}

impl StreamEndpointOwner {
    fn new(
        endpoint: StreamEndpoint,
        callback: PythonCallback,
        python_loop: Arc<PythonLoop>,
        sources: PythonSourceRegistry,
    ) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            callback: Mutex::new(Some(callback)),
            python_loop: Mutex::new(Some(python_loop)),
            sources,
            cleanup: DetachedCleanup::default(),
        }
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            self.callback.lock().take();
            self.python_loop.lock().take();
            let close = self.endpoint.lock().take().map(StreamEndpoint::close);
            let sources = self.sources.clone();
            async move {
                let result = match close {
                    Some(close) => close.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                };
                let sources_result = sources.shutdown().await;
                result.and(sources_result)
            }
        })
    }

    fn traverse(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Some(callback) = self.callback.lock().as_ref() {
            visit.call(callback.as_ref())?;
        }
        if let Some(python_loop) = self.python_loop.lock().as_ref() {
            python_loop.traverse(visit)?;
        }
        self.sources.visit(visit)
    }
}

impl Drop for StreamEndpointOwner {
    fn drop(&mut self) {
        self.callback.get_mut().take();
        self.python_loop.get_mut().take();
        let Some(endpoint) = self.endpoint.get_mut().take() else {
            return;
        };
        let sources = self.sources.clone();
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let _ = endpoint.close().await;
            let _ = sources.shutdown().await;
        });
    }
}

/// Mounted inbound Stream v2 service backed by a synchronous Python provider.
#[pyclass(name = "AukiStreamEndpoint")]
pub(crate) struct PyAukiStreamEndpoint {
    owner: StreamEndpointOwner,
    client: StreamClient,
}

#[pymethods]
impl PyAukiStreamEndpoint {
    /// Mount Stream v2 from inside a running asyncio event loop.
    ///
    /// The provider is called synchronously as `provider(requester, request)`.
    /// It returns either a decline dict or an accept dict containing
    /// `payload_kind`, `manifest`, and a Python `AsyncIterable` source.
    #[staticmethod]
    fn mount(py: Python<'_>, peer: &PyAukiPeer, provider: Py<PyAny>) -> PyResult<Self> {
        let callback = require_callable(py, provider, "Stream provider")?;
        let python_loop = PythonLoop::capture(py)?;
        let sources = PythonSourceRegistry::default();
        let endpoint = enter_tokio_runtime(|| {
            StreamEndpoint::mount(
                peer.protocols(),
                PythonStreamProvider {
                    callback: callback.clone(),
                    python_loop: Arc::clone(&python_loop),
                    sources: sources.clone(),
                },
            )
        })
        .map_err(|error| runtime_error("mount Stream endpoint", error))?;
        let client = endpoint.client();
        Ok(Self {
            owner: StreamEndpointOwner::new(endpoint, callback, python_loop, sources),
            client,
        })
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    #[getter]
    fn client(&self) -> PyAukiStreamClient {
        PyAukiStreamClient::from_inner(self.client.clone())
    }

    /// Stop admission, cancel admitted handlers, and close Python sources.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_close();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close Stream endpoint", error))
        })
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        self.owner.traverse(&visit)
    }

    fn __clear__(&mut self) {
        self.owner.begin_close();
    }
}

struct AnySubscription {
    manifest: StreamManifest,
    entries: AnyEntries,
}

fn split_subscription<T>(
    subscription: StreamSubscription<T>,
    wrap: fn(SubscriptionEntries<T>) -> AnyEntries,
) -> AnySubscription {
    AnySubscription {
        manifest: subscription.manifest,
        entries: wrap(subscription.entries),
    }
}

enum AnyEntries {
    Camera(SubscriptionEntries<CameraFrame>),
    PointCloud(SubscriptionEntries<point_cloud::Data>),
    JointEncoders(SubscriptionEntries<joint_encoders::Data>),
    Audio(SubscriptionEntries<audio::Data>),
    Scalar(SubscriptionEntries<scalar::Data>),
    Pose(SubscriptionEntries<pose::SpatialTransform>),
    Detection(SubscriptionEntries<DetectionFrame>),
    Map(SubscriptionEntries<MapUpdate>),
}

impl AnyEntries {
    async fn next_encoded(&mut self) -> Option<Result<EncodedStreamEntry, StreamError>> {
        match self {
            Self::Camera(entries) => next_typed(entries).await,
            Self::PointCloud(entries) => next_typed(entries).await,
            Self::JointEncoders(entries) => next_typed(entries).await,
            Self::Audio(entries) => next_typed(entries).await,
            Self::Scalar(entries) => next_typed(entries).await,
            Self::Pose(entries) => next_typed(entries).await,
            Self::Detection(entries) => next_typed(entries).await,
            Self::Map(entries) => next_typed(entries).await,
        }
    }
}

async fn next_typed<T>(
    entries: &mut SubscriptionEntries<T>,
) -> Option<Result<EncodedStreamEntry, StreamError>>
where
    T: Message,
{
    entries
        .next()
        .await
        .map(|entry| entry.map(EncodedStreamEntry::from_typed))
}

struct EncodedStreamEntry {
    timestamp_ns: i64,
    sequence: u64,
    payload: Vec<u8>,
}

impl EncodedStreamEntry {
    fn from_typed<T>(entry: StreamEntry<T>) -> Self
    where
        T: Message,
    {
        Self {
            timestamp_ns: entry.timestamp_ns,
            sequence: entry.seq,
            payload: entry.payload.encode_to_vec(),
        }
    }
}

struct StreamSubscriptionSlot {
    entries: Option<AnyEntries>,
    closed: bool,
    next_pending: bool,
}

struct StreamSubscriptionState {
    slot: Mutex<StreamSubscriptionSlot>,
    cancel: watch::Sender<bool>,
    completed: watch::Sender<bool>,
    cleanup: DetachedCleanup,
}

impl StreamSubscriptionState {
    fn new(entries: AnyEntries) -> Self {
        let (cancel, _) = watch::channel(false);
        let (completed, _) = watch::channel(false);
        Self {
            slot: Mutex::new(StreamSubscriptionSlot {
                entries: Some(entries),
                closed: false,
                next_pending: false,
            }),
            cancel,
            completed,
            cleanup: DetachedCleanup::default(),
        }
    }

    fn begin_next(self: &Arc<Self>) -> PyResult<Option<PendingStreamEntries>> {
        let mut slot = self.slot.lock();
        if slot.closed {
            return Ok(None);
        }
        if slot.next_pending {
            return Err(PyRuntimeError::new_err(
                "Stream subscription already has a pending next()",
            ));
        }
        let entries = slot
            .entries
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("Stream subscription is unavailable"))?;
        slot.next_pending = true;
        Ok(Some(PendingStreamEntries {
            state: Arc::clone(self),
            entries: Some(entries),
        }))
    }

    fn finish_next(&self, entries: AnyEntries, ended: bool) {
        let mut entries = Some(entries);
        let complete = {
            let mut slot = self.slot.lock();
            slot.next_pending = false;
            if slot.closed || ended {
                slot.closed = true;
                true
            } else {
                slot.entries = entries.take();
                false
            }
        };
        drop(entries);
        if complete {
            self.cancel.send_replace(true);
            self.completed.send_replace(true);
        }
    }

    fn begin_cancel(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            self.cancel.send_replace(true);
            let entries = {
                let mut slot = self.slot.lock();
                slot.closed = true;
                if slot.next_pending {
                    None
                } else {
                    slot.entries.take()
                }
            };
            if let Some(entries) = entries {
                drop(entries);
                self.completed.send_replace(true);
            }
            let completion = self.completed.subscribe();
            async move { wait_subscription_completion(completion).await }
        })
    }
}

struct PendingStreamEntries {
    state: Arc<StreamSubscriptionState>,
    entries: Option<AnyEntries>,
}

impl PendingStreamEntries {
    fn entries(&mut self) -> &mut AnyEntries {
        self.entries
            .as_mut()
            .expect("a pending Stream read owns its native entries")
    }

    fn finish(mut self, ended: bool) {
        let entries = self
            .entries
            .take()
            .expect("a pending Stream read finishes only once");
        self.state.finish_next(entries, ended);
    }
}

impl Drop for PendingStreamEntries {
    fn drop(&mut self) {
        if let Some(entries) = self.entries.take() {
            self.state.finish_next(entries, false);
        }
    }
}

async fn subscription_next(
    mut pending: PendingStreamEntries,
) -> Option<Result<EncodedStreamEntry, StreamError>> {
    let mut cancellation = pending.state.cancel.subscribe();
    let cancelled = *cancellation.borrow();
    let item = if cancelled {
        None
    } else {
        tokio::select! {
            biased;
            _ = cancellation.changed() => None,
            item = pending.entries().next_encoded() => item,
        }
    };
    let ended = !matches!(item, Some(Ok(_)));
    pending.finish(ended);
    item
}

async fn wait_subscription_completion(mut completion: watch::Receiver<bool>) -> Result<(), String> {
    loop {
        if *completion.borrow_and_update() {
            return Ok(());
        }
        if completion.changed().await.is_err() {
            return Err("Stream subscription cleanup ended without a result".into());
        }
    }
}

/// One accepted typed Stream v2 subscription.
#[pyclass(name = "AukiStreamSubscription")]
pub(crate) struct PyAukiStreamSubscription {
    payload_kind: StreamPayloadKind,
    manifest: StreamManifest,
    state: Arc<StreamSubscriptionState>,
}

impl PyAukiStreamSubscription {
    fn new(payload_kind: StreamPayloadKind, manifest: StreamManifest, entries: AnyEntries) -> Self {
        Self {
            payload_kind,
            manifest,
            state: Arc::new(StreamSubscriptionState::new(entries)),
        }
    }
}

#[pymethods]
impl PyAukiStreamSubscription {
    #[getter]
    fn payload_kind(&self) -> &'static str {
        self.payload_kind.as_str()
    }

    #[getter]
    fn manifest(&self, py: Python<'_>) -> PyResult<PyObject> {
        manifest_to_python(py, &self.manifest)
    }

    /// Pull one entry dict, one explicit terminal dict, or `None` after cancel.
    /// Only one `next()` may be pending at a time.
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let Some(pending) = self.state.begin_next()? else {
            return pyo3_async_runtimes::tokio::future_into_py(py, async move {
                Ok(Python::with_gil(|py| py.None()))
            });
        };
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let item = subscription_next(pending).await;
            Python::with_gil(|py| match item {
                Some(Ok(entry)) => entry_to_python(py, entry),
                Some(Err(StreamError::EndOfStream { reason })) => end_to_python(py, reason),
                Some(Err(error)) => Err(runtime_error("read Stream entry", error)),
                None => Ok(py.None()),
            })
        })
    }

    /// Idempotently cancel this subscription and await local route release.
    fn cancel<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.state.begin_cancel();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("cancel Stream subscription", error))
        })
    }
}

impl Drop for PyAukiStreamSubscription {
    fn drop(&mut self) {
        self.state.begin_cancel();
    }
}

fn entry_to_python(py: Python<'_>, entry: EncodedStreamEntry) -> PyResult<PyObject> {
    let value = PyDict::new_bound(py);
    value.set_item("kind", "entry")?;
    let entry_value = PyDict::new_bound(py);
    entry_value.set_item("timestamp_ns", entry.timestamp_ns)?;
    entry_value.set_item("sequence", entry.sequence)?;
    entry_value.set_item("payload", PyBytes::new_bound(py, &entry.payload))?;
    value.set_item("entry", entry_value)?;
    Ok(value.unbind().into_any())
}

fn end_to_python(py: Python<'_>, reason: EndReason) -> PyResult<PyObject> {
    let value = PyDict::new_bound(py);
    value.set_item("kind", "end")?;
    let reason_value = PyDict::new_bound(py);
    match reason.kind {
        Some(end_reason::Kind::SourceEnded(_)) => {
            reason_value.set_item("kind", "source_ended")?;
        }
        Some(end_reason::Kind::ProducerShuttingDown(_)) => {
            reason_value.set_item("kind", "producer_shutting_down")?;
        }
        Some(end_reason::Kind::SessionEnded(_)) => {
            reason_value.set_item("kind", "session_ended")?;
        }
        Some(end_reason::Kind::ProducerError(error)) => {
            reason_value.set_item("kind", "producer_error")?;
            reason_value.set_item("detail", error.detail)?;
        }
        None => return Err(PyRuntimeError::new_err("Stream end reason has no kind")),
    }
    value.set_item("reason", reason_value)?;
    Ok(value.unbind().into_any())
}

pub(super) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyAukiStreamClient>()?;
    module.add_class::<PyAukiStreamEndpoint>()?;
    module.add_class::<PyAukiStreamSubscription>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use auki_sdk_rs::Identity;

    use super::super::support::requester;
    use super::*;

    struct PythonLoopHarness {
        context: Arc<PythonLoop>,
        event_loop: Py<PyAny>,
        thread: Py<PyAny>,
    }

    impl PythonLoopHarness {
        fn start(context_var: &Py<PyAny>) -> PyResult<Self> {
            Python::with_gil(|py| {
                let asyncio = py.import_bound("asyncio")?;
                let event_loop = asyncio.call_method0("new_event_loop")?;
                context_var.bind(py).call_method1("set", ("captured",))?;
                let locals = TaskLocals::new(event_loop.clone()).copy_context(py)?;
                context_var.bind(py).call_method1("set", ("uncaptured",))?;
                let threading = py.import_bound("threading")?;
                let kwargs = PyDict::new_bound(py);
                kwargs.set_item("target", event_loop.getattr("run_forever")?)?;
                kwargs.set_item("daemon", true)?;
                let thread = threading.call_method("Thread", (), Some(&kwargs))?;
                thread.call_method0("start")?;
                Ok(Self {
                    context: Arc::new(PythonLoop {
                        event_loop: Arc::new(locals.event_loop(py).unbind()),
                        context: Arc::new(locals.context(py).unbind()),
                        locals: Arc::new(locals),
                    }),
                    event_loop: event_loop.unbind(),
                    thread: thread.unbind(),
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
    fn payload_kinds_lock_the_eight_manifest_families() {
        for (name, manifest_payload) in [
            ("camera", "camera_frame"),
            ("point_cloud", "point_cloud_frame"),
            ("joint_encoders", "joint_encoders_frame"),
            ("audio", "audio_frame"),
            ("scalar", "scalar"),
            ("pose", "spatial_transform"),
            ("detection", "detection"),
            ("map", "map_update"),
        ] {
            let kind = StreamPayloadKind::parse(name).unwrap();
            assert_eq!(kind.manifest_payload(), manifest_payload);
            assert!(
                kind.validate_manifest(&StreamManifest {
                    payload: manifest_payload.into(),
                    ..Default::default()
                })
                .is_ok()
            );
        }
        assert!(StreamPayloadKind::parse("pointcloud").is_err());
    }

    #[test]
    fn request_and_manifest_records_use_python_snake_case() {
        Python::with_gil(|py| {
            let request = StreamRequest {
                source_peer_id: "source".into(),
                resource_id: "camera".into(),
                from: ReadFrom::FromTimestamp(42),
            };
            let value = request_to_python(py, &request).unwrap();
            assert_eq!(request_from_python(py, value.bind(py)).unwrap(), request);
            assert_eq!(
                value
                    .bind(py)
                    .get_item("from")
                    .unwrap()
                    .get_item("timestamp_ns")
                    .unwrap()
                    .extract::<i64>()
                    .unwrap(),
                42
            );

            let manifest = StreamManifest {
                resource_id: "camera".into(),
                payload: "camera_frame".into(),
                ..Default::default()
            };
            let value = manifest_to_python(py, &manifest).unwrap();
            assert_eq!(manifest_from_python(py, value.bind(py)).unwrap(), manifest);
        });
    }

    #[test]
    fn pending_python_anext_is_cancelled_and_closed_before_registry_shutdown() {
        pyo3::prepare_freethreaded_python();
        let stream_context = Python::with_gil(|py| -> PyResult<_> {
            let context_var = py
                .import_bound("contextvars")?
                .getattr("ContextVar")?
                .call1(("auki_stream_test",))?;
            Ok(context_var.unbind())
        })
        .expect("seed provider context");
        let python_loop =
            PythonLoopHarness::start(&stream_context).expect("start Python event loop");
        let (callback, finished, trace) = Python::with_gil(|py| -> PyResult<_> {
            let payload = CameraFrame {
                frame: b"camera-frame".to_vec(),
                ..Default::default()
            }
            .encode_to_vec();
            let finished = py.import_bound("threading")?.call_method0("Event")?;
            let module = PyModule::from_code_bound(
                py,
                r#"
import asyncio
trace = []

async def source(payload, finished):
    trace.append("source-enter")
    assert stream_context.get() == "captured"
    try:
        yield {"timestamp_ns": 42, "payload": payload}
        await asyncio.Event().wait()
    finally:
        trace.append("finally-enter")
        assert stream_context.get() == "captured"
        await asyncio.sleep(0.05)
        finished.set()
        trace.append("finally-exit")

class Source:
    def __init__(self, payload, finished):
        self.iterator = source(payload, finished)

    def __aiter__(self):
        trace.append("aiter")
        assert stream_context.get() == "captured"
        return self

    def __anext__(self):
        trace.append("anext")
        assert stream_context.get() == "captured"
        return self.iterator.__anext__()

    def aclose(self):
        trace.append("aclose")
        assert stream_context.get() == "captured"
        return self.iterator.aclose()

def provider(requester, request):
    trace.append("provider")
    assert stream_context.get() == "captured"
    assert requester["peer_type"] == "native_app"
    assert request["resource_id"] == "camera"
    return {
        "kind": "accept",
        "payload_kind": "camera",
        "manifest": {"resource_id": "camera", "payload": "camera_frame"},
        "source": Source(payload, finished),
    }
"#,
                "stream_provider_test.py",
                "stream_provider_test",
            )?;
            module.setattr("stream_context", stream_context.bind(py))?;
            module.setattr("payload", PyBytes::new_bound(py, &payload))?;
            module.setattr("finished", finished.clone())?;
            Ok((
                Arc::new(module.getattr("provider")?.unbind()),
                finished.unbind(),
                module.getattr("trace")?.unbind(),
            ))
        })
        .expect("build Python provider");
        let sources = PythonSourceRegistry::default();
        let provider = PythonStreamProvider {
            callback,
            python_loop: Arc::clone(&python_loop.context),
            sources: sources.clone(),
        };
        let request = StreamRequest {
            source_peer_id: "source".into(),
            resource_id: "camera".into(),
            from: ReadFrom::Latest,
        };
        let dispatch = provider.dispatch(&requester(Identity::generate().peer_id()), request);
        let StreamDispatch::AcceptCamera { mut source, .. } = dispatch else {
            panic!("provider did not accept a camera source");
        };

        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let first = source.next().await.unwrap().unwrap();
            assert_eq!(first.timestamp_ns, 42);
            assert_eq!(first.payload.frame, b"camera-frame");

            let mut pending = Box::pin(source.next());
            assert!(futures::poll!(pending.as_mut()).is_pending());
            drop(pending);
            drop(source);
            sources.shutdown().await.unwrap();

            let closed = Python::with_gil(|py| {
                finished
                    .bind(py)
                    .call_method0("is_set")
                    .unwrap()
                    .extract::<bool>()
                    .unwrap()
            });
            let trace = Python::with_gil(|py| trace.bind(py).extract::<Vec<String>>().unwrap());
            assert!(
                closed,
                "shutdown returned before async-generator finally: {trace:?}"
            );
        });
        python_loop.stop();
    }

    #[test]
    fn subscription_cancel_interrupts_one_pending_next_and_releases_entries() {
        let entries = AnyEntries::Camera(Box::pin(futures::stream::pending()));
        let state = Arc::new(StreamSubscriptionState::new(entries));
        let pending = state.begin_next().unwrap().unwrap();
        let cleanup = state.begin_cancel();
        pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
            assert!(subscription_next(pending).await.is_none());
            assert!(wait_cleanup(cleanup).await.is_ok());
        });
    }

    #[test]
    fn module_registers_stream_roles() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_sdk").unwrap();
            register(&module).unwrap();
            for name in [
                "AukiStreamClient",
                "AukiStreamEndpoint",
                "AukiStreamSubscription",
            ] {
                assert!(module.getattr(name).is_ok(), "missing {name}");
            }
        });
    }
}
