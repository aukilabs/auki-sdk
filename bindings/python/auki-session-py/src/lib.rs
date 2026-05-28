//! PyO3 bindings for [`auki-session`](../../../../crates/auki-session).
//!
//! Exposes the declarative `Session` API to Python:
//! - `Session(peer_id, app_id)` — create a session
//! - `with_storage_root(path)` — builder method (returns self)
//! - Read accessors: `peer_id`, `app_id`, `session_id`, `storage_root`
//! - Registry registration: `register_sensor`, `register_clock`,
//!   `register_frame`, `register_detector` — each returns a `RegistryRef`
//! - Log registration: `register_sensor_log`, `register_pose_log`,
//!   `register_time_transform_log`, `register_detection_log` — each
//!   returns the corresponding handle
//! - `catalog()` — returns a list of `dict`s (canonical JSON shape)
//! - Async stubs: `join_domain`, `leave_domain`, `materialize_remote_log`,
//!   `resolve_static_transform` — raise `NotImplementedError` with a clear
//!   message (the Rust side returns `NotImplemented` on these paths)
//!
//! ## Type sharing
//!
//! `RegistryRef` and `LogRef` are defined here as pyclasses that mirror
//! the same-named pyclasses in `auki-registry-py`. Both bindings construct
//! them from the same underlying Rust `auki_registry::{RegistryRef,LogRef}`
//! types. Python code that uses both packages can pass the objects
//! interchangeably through JSON round-trip (dict form) if needed.
//!
//! `PoseSource`, `PoseWriterMode`, and `TimeTransformSource` are accepted
//! as Python dicts / strings, not as pyclasses — same convention as
//! `auki-manifests-py`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pyo3::exceptions::{PyNotImplementedError, PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyModule};
use serde_json::Value as JsonValue;

use auki_manifests_rs as manifests;
use auki_registry_rs as registry;
use auki_session_rs as session;

// ─── JSON helpers ───────────────────────────────────────────────────────────

/// Serialize a Rust value that implements serde::Serialize to a Python object
/// via `json.loads(serde_json::to_string(...))`.
fn rust_to_pyobject<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<PyObject> {
    let s = serde_json::to_string(value)
        .map_err(|e| PyRuntimeError::new_err(format!("internal serialize: {e}")))?;
    let json = py.import_bound("json")?;
    Ok(json.call_method1("loads", (s,))?.unbind())
}

/// Convert a Python object to `serde_json::Value` via `json.dumps(...)`.
fn pyobject_to_json(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<JsonValue> {
    let json = py.import_bound("json")?;
    let s: String = json.call_method1("dumps", (value,))?.extract()?;
    serde_json::from_str(&s).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

/// Parse a Rust type from a Python object via JSON round-trip.
fn parse_py<T: serde::de::DeserializeOwned>(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<T> {
    let json = pyobject_to_json(py, value, name)?;
    serde_json::from_value(json).map_err(|e| PyValueError::new_err(format!("{name}: {e}")))
}

/// Parse a `registry::RegistryRef` from a Python object that is either a
/// `RegistryRef` pyclass instance or a plain dict with `peer_id`, `id`, `hash`.
fn parse_registry_ref(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<registry::RegistryRef> {
    if let Ok(r) = value.extract::<RegistryRef>() {
        return Ok(registry::RegistryRef {
            peer_id: r.peer_id,
            id: r.id,
            hash: r.hash,
        });
    }
    parse_py(py, value, name)
}

/// Parse a `registry::LogRef` from a Python object that is either a
/// `LogRef` pyclass instance or a plain dict with `source_peer_id`, `resource_id`.
fn parse_log_ref(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<registry::LogRef> {
    if let Ok(r) = value.extract::<LogRef>() {
        return Ok(registry::LogRef {
            source_peer_id: r.source_peer_id,
            resource_id: r.resource_id,
        });
    }
    parse_py(py, value, name)
}

/// Convert a `session::SessionError` to a Python exception.
fn map_session_error(err: session::SessionError) -> PyErr {
    match err {
        session::SessionError::Io(e) => PyOSError::new_err(e.to_string()),
        session::SessionError::InvalidId(e) => PyValueError::new_err(e.to_string()),
        session::SessionError::Registry(e) => PyValueError::new_err(e.to_string()),
        session::SessionError::DuplicateLog {
            source_peer_id,
            resource_id,
        } => PyValueError::new_err(format!(
            "duplicate log {source_peer_id}/{resource_id}"
        )),
        session::SessionError::Materialization(
            session::MaterializationError::NotImplemented,
        ) => PyNotImplementedError::new_err(
            "not implemented: full materialization deferred to Phase 5",
        ),
        session::SessionError::Materialization(e) => {
            PyRuntimeError::new_err(format!("materialization: {e}"))
        }
        session::SessionError::DomainBootstrap(e) => {
            PyNotImplementedError::new_err(format!(
                "join_domain not yet supported from Python (requires libp2p swarm): {e}"
            ))
        }
        session::SessionError::DomainShutdown(e) => {
            PyNotImplementedError::new_err(format!(
                "leave_domain not yet supported from Python (requires libp2p swarm): {e}"
            ))
        }
    }
}

// ─── RegistryRef / LogRef pyclasses ─────────────────────────────────────────

/// Python representation of a `(peer_id, id, hash)` registry reference.
/// Mirrors the same-named class in `auki-registry-py`.
#[pyclass]
#[derive(Clone)]
pub struct RegistryRef {
    #[pyo3(get, set)]
    pub peer_id: String,
    #[pyo3(get, set)]
    pub id: String,
    #[pyo3(get, set)]
    pub hash: String,
}

#[pymethods]
impl RegistryRef {
    #[new]
    fn new(peer_id: String, id: String, hash: String) -> Self {
        Self { peer_id, id, hash }
    }

    fn __repr__(&self) -> String {
        format!(
            "RegistryRef(peer_id={:?}, id={:?}, hash={:?})",
            self.peer_id, self.id, self.hash
        )
    }
}

/// Python representation of a `(source_peer_id, resource_id)` log reference.
/// Mirrors the same-named class in `auki-registry-py`.
#[pyclass]
#[derive(Clone)]
pub struct LogRef {
    #[pyo3(get, set)]
    pub source_peer_id: String,
    #[pyo3(get, set)]
    pub resource_id: String,
}

#[pymethods]
impl LogRef {
    #[new]
    fn new(source_peer_id: String, resource_id: String) -> Self {
        Self {
            source_peer_id,
            resource_id,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "LogRef(source_peer_id={:?}, resource_id={:?})",
            self.source_peer_id, self.resource_id
        )
    }
}

// ─── HeadSpec pyclass ────────────────────────────────────────────────────────

/// Head-window spec. Controls rolling vs fixed semantics for a log.
///
/// Use `HeadSpec.rolling(retention_ns)` or `HeadSpec.fixed()`.
#[pyclass]
#[derive(Clone)]
pub struct HeadSpec {
    inner: session::HeadSpec,
}

#[pymethods]
impl HeadSpec {
    /// Rolling retention window: segments older than `retention_ns` nanoseconds
    /// are eligible for eviction.
    #[staticmethod]
    fn rolling(retention_ns: i64) -> Self {
        Self {
            inner: session::HeadSpec::Rolling { retention_ns },
        }
    }

    /// Fixed log: no automatic eviction.
    #[staticmethod]
    fn fixed() -> Self {
        Self {
            inner: session::HeadSpec::Fixed,
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            session::HeadSpec::Rolling { retention_ns } => {
                format!("HeadSpec.rolling({retention_ns})")
            }
            session::HeadSpec::Fixed => "HeadSpec.fixed()".to_string(),
        }
    }
}

// ─── FrameDef pyclass ────────────────────────────────────────────────────────

/// Coordinate frame preset. Used with `Session.register_frame`.
///
/// Use one of the four named classmethods:
/// - `FrameDef.ros_body()` — REP-103 body frame
/// - `FrameDef.ros_optical()` — REP-103 camera optical frame
/// - `FrameDef.opengl()` — OpenGL / Three.js right-handed frame
/// - `FrameDef.unity()` — Unity left-handed frame
#[pyclass]
#[derive(Clone)]
pub struct FrameDef {
    inner: FrameDefInner,
}

#[derive(Clone)]
enum FrameDefInner {
    RosBody,
    RosOptical,
    OpenGl,
    Unity,
}

#[pymethods]
impl FrameDef {
    /// REP-103 body frame: right-handed, X forward, Y left, Z up, meters.
    #[staticmethod]
    fn ros_body() -> Self {
        Self {
            inner: FrameDefInner::RosBody,
        }
    }
    /// REP-103 camera optical frame: right-handed, X right, Y down, Z forward, meters.
    #[staticmethod]
    fn ros_optical() -> Self {
        Self {
            inner: FrameDefInner::RosOptical,
        }
    }
    /// OpenGL / Three.js: right-handed, X right, Y up, Z backward, meters.
    #[staticmethod]
    fn opengl() -> Self {
        Self {
            inner: FrameDefInner::OpenGl,
        }
    }
    /// Unity: left-handed, X right, Y up, Z forward, meters.
    #[staticmethod]
    fn unity() -> Self {
        Self {
            inner: FrameDefInner::Unity,
        }
    }

    fn __repr__(&self) -> &str {
        match &self.inner {
            FrameDefInner::RosBody => "FrameDef.ros_body()",
            FrameDefInner::RosOptical => "FrameDef.ros_optical()",
            FrameDefInner::OpenGl => "FrameDef.opengl()",
            FrameDefInner::Unity => "FrameDef.unity()",
        }
    }
}

impl FrameDef {
    fn into_rust_frame_def(self) -> session::FrameDef {
        match self.inner {
            FrameDefInner::RosBody => session::FrameDef::ros_body(),
            FrameDefInner::RosOptical => session::FrameDef::ros_optical(),
            FrameDefInner::OpenGl => session::FrameDef::opengl(),
            FrameDefInner::Unity => session::FrameDef::unity(),
        }
    }
}

// ─── Log spec pyclasses ──────────────────────────────────────────────────────

/// Spec for registering a sensor log.
#[pyclass]
#[derive(Clone)]
pub struct SensorLogSpec {
    inner: session::SensorLogSpec,
}

#[pymethods]
impl SensorLogSpec {
    /// Construct a SensorLogSpec.
    ///
    /// Parameters
    /// ----------
    /// sensor : RegistryRef
    ///     Registry reference for the sensor.
    /// clock : RegistryRef
    ///     Registry reference for the clock.
    /// head : HeadSpec
    ///     Rolling or fixed head spec.
    /// segment_duration_ns : int
    ///     Segment roll-over interval in nanoseconds.
    /// retention_ns : int
    ///     Segment eviction age in nanoseconds (0 = keep forever).
    /// frame : RegistryRef or None
    ///     Optional registry reference for the spatial frame.
    #[new]
    #[pyo3(signature = (sensor, clock, head, segment_duration_ns, retention_ns, frame=None))]
    fn new(
        py: Python<'_>,
        sensor: &Bound<'_, PyAny>,
        clock: &Bound<'_, PyAny>,
        head: &HeadSpec,
        segment_duration_ns: u64,
        retention_ns: u64,
        frame: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let sensor_ref = parse_registry_ref(py, sensor, "sensor")?;
        let clock_ref = parse_registry_ref(py, clock, "clock")?;
        let frame_ref = match frame {
            Some(f) => Some(parse_registry_ref(py, f, "frame")?),
            None => None,
        };
        Ok(Self {
            inner: session::SensorLogSpec {
                sensor: sensor_ref,
                clock: clock_ref,
                frame: frame_ref,
                head: head.inner.clone(),
                segment_duration: Duration::from_nanos(segment_duration_ns),
                retention: Duration::from_nanos(retention_ns),
            },
        })
    }
}

/// Spec for registering a pose log.
#[pyclass]
#[derive(Clone)]
pub struct PoseLogSpec {
    inner: session::PoseLogSpec,
}

#[pymethods]
impl PoseLogSpec {
    /// Construct a PoseLogSpec.
    ///
    /// Parameters
    /// ----------
    /// from_frame : RegistryRef
    /// to_frame : RegistryRef
    /// clock : RegistryRef
    /// source : dict
    ///     PoseSource tagged-enum dict, e.g. ``{"kind": "manual"}`` or
    ///     ``{"kind": "ros2_tf", "publishers": ["my_node"]}``.
    /// writer_mode : str
    ///     ``"rigid"`` or ``"movable"``.
    /// expected_rate_hz : int
    /// head : HeadSpec
    /// segment_duration_ns : int
    /// retention_ns : int
    #[new]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        from_frame: &Bound<'_, PyAny>,
        to_frame: &Bound<'_, PyAny>,
        clock: &Bound<'_, PyAny>,
        source: &Bound<'_, PyAny>,
        writer_mode: &str,
        expected_rate_hz: u32,
        head: &HeadSpec,
        segment_duration_ns: u64,
        retention_ns: u64,
    ) -> PyResult<Self> {
        let from_ref = parse_registry_ref(py, from_frame, "from_frame")?;
        let to_ref = parse_registry_ref(py, to_frame, "to_frame")?;
        let clock_ref = parse_registry_ref(py, clock, "clock")?;
        let pose_source: manifests::PoseSource = parse_py(py, source, "source")?;
        let mode: manifests::PoseWriterMode =
            serde_json::from_value(serde_json::Value::String(writer_mode.to_string()))
                .map_err(|e| PyValueError::new_err(format!("writer_mode: {e}")))?;
        Ok(Self {
            inner: session::PoseLogSpec {
                from_frame: from_ref,
                to_frame: to_ref,
                clock: clock_ref,
                source: pose_source,
                writer_mode: mode,
                expected_rate_hz,
                head: head.inner.clone(),
                segment_duration: Duration::from_nanos(segment_duration_ns),
                retention: Duration::from_nanos(retention_ns),
            },
        })
    }
}

/// Spec for registering a time-transform log.
#[pyclass]
#[derive(Clone)]
pub struct TimeTransformLogSpec {
    inner: session::TimeTransformLogSpec,
}

#[pymethods]
impl TimeTransformLogSpec {
    /// Construct a TimeTransformLogSpec.
    ///
    /// Parameters
    /// ----------
    /// from_clock : RegistryRef
    /// to_clock : RegistryRef
    /// source : dict
    ///     TimeTransformSource tagged-enum dict, e.g. ``{"kind": "local_clock_read"}``.
    /// head : HeadSpec
    /// segment_duration_ns : int
    /// retention_ns : int
    #[new]
    fn new(
        py: Python<'_>,
        from_clock: &Bound<'_, PyAny>,
        to_clock: &Bound<'_, PyAny>,
        source: &Bound<'_, PyAny>,
        head: &HeadSpec,
        segment_duration_ns: u64,
        retention_ns: u64,
    ) -> PyResult<Self> {
        let from_ref = parse_registry_ref(py, from_clock, "from_clock")?;
        let to_ref = parse_registry_ref(py, to_clock, "to_clock")?;
        let tt_source: manifests::TimeTransformSource = parse_py(py, source, "source")?;
        Ok(Self {
            inner: session::TimeTransformLogSpec {
                from_clock: from_ref,
                to_clock: to_ref,
                source: tt_source,
                head: head.inner.clone(),
                segment_duration: Duration::from_nanos(segment_duration_ns),
                retention: Duration::from_nanos(retention_ns),
            },
        })
    }
}

/// Spec for registering a detection log.
#[pyclass]
#[derive(Clone)]
pub struct DetectionLogSpec {
    inner: session::DetectionLogSpec,
}

#[pymethods]
impl DetectionLogSpec {
    /// Construct a DetectionLogSpec.
    ///
    /// Parameters
    /// ----------
    /// detector : RegistryRef
    /// input_log : LogRef
    ///     Log reference to the input sensor log being tailed.
    /// input_sensor : RegistryRef
    ///     Registry reference copied from the input log's manifest.
    /// clock : RegistryRef
    /// head : HeadSpec
    /// segment_duration_ns : int
    /// retention_ns : int
    #[new]
    fn new(
        py: Python<'_>,
        detector: &Bound<'_, PyAny>,
        input_log: &Bound<'_, PyAny>,
        input_sensor: &Bound<'_, PyAny>,
        clock: &Bound<'_, PyAny>,
        head: &HeadSpec,
        segment_duration_ns: u64,
        retention_ns: u64,
    ) -> PyResult<Self> {
        let detector_ref = parse_registry_ref(py, detector, "detector")?;
        let log_ref = parse_log_ref(py, input_log, "input_log")?;
        let input_sensor_ref = parse_registry_ref(py, input_sensor, "input_sensor")?;
        let clock_ref = parse_registry_ref(py, clock, "clock")?;
        Ok(Self {
            inner: session::DetectionLogSpec {
                detector: detector_ref,
                input_log: log_ref,
                input_sensor: input_sensor_ref,
                clock: clock_ref,
                head: head.inner.clone(),
                segment_duration: Duration::from_nanos(segment_duration_ns),
                retention: Duration::from_nanos(retention_ns),
            },
        })
    }
}

// ─── Log handle pyclasses ────────────────────────────────────────────────────

/// Handle returned by `Session.register_sensor_log`.
#[pyclass]
#[derive(Debug)]
pub struct SensorLogHandle {
    #[pyo3(get)]
    pub resource_id: String,
    log_ref_inner: registry::LogRef,
}

#[pymethods]
impl SensorLogHandle {
    /// The `(source_peer_id, resource_id)` log reference.
    #[getter]
    fn log_ref(&self) -> LogRef {
        LogRef {
            source_peer_id: self.log_ref_inner.source_peer_id.clone(),
            resource_id: self.log_ref_inner.resource_id.clone(),
        }
    }
}

/// Handle returned by `Session.register_pose_log`.
#[pyclass]
#[derive(Debug)]
pub struct PoseLogHandle {
    #[pyo3(get)]
    pub resource_id: String,
    log_ref_inner: registry::LogRef,
}

#[pymethods]
impl PoseLogHandle {
    /// The `(source_peer_id, resource_id)` log reference.
    #[getter]
    fn log_ref(&self) -> LogRef {
        LogRef {
            source_peer_id: self.log_ref_inner.source_peer_id.clone(),
            resource_id: self.log_ref_inner.resource_id.clone(),
        }
    }
}

/// Handle returned by `Session.register_time_transform_log`.
#[pyclass]
#[derive(Debug)]
pub struct TimeTransformLogHandle {
    #[pyo3(get)]
    pub resource_id: String,
    log_ref_inner: registry::LogRef,
}

#[pymethods]
impl TimeTransformLogHandle {
    /// The `(source_peer_id, resource_id)` log reference.
    #[getter]
    fn log_ref(&self) -> LogRef {
        LogRef {
            source_peer_id: self.log_ref_inner.source_peer_id.clone(),
            resource_id: self.log_ref_inner.resource_id.clone(),
        }
    }
}

/// Handle returned by `Session.register_detection_log`.
#[pyclass]
#[derive(Debug)]
pub struct DetectionLogHandle {
    #[pyo3(get)]
    pub resource_id: String,
    log_ref_inner: registry::LogRef,
}

#[pymethods]
impl DetectionLogHandle {
    /// The `(source_peer_id, resource_id)` log reference.
    #[getter]
    fn log_ref(&self) -> LogRef {
        LogRef {
            source_peer_id: self.log_ref_inner.source_peer_id.clone(),
            resource_id: self.log_ref_inner.resource_id.clone(),
        }
    }
}

/// Handle returned by a successful `Session.materialize_remote_log`.
/// Stub — this path is currently unreachable (the Rust side returns
/// `MaterializationError::NotImplemented`).
#[pyclass]
#[derive(Debug)]
pub struct MaterializedLogHandle {
    log_ref_inner: registry::LogRef,
}

#[pymethods]
impl MaterializedLogHandle {
    /// The `(source_peer_id, resource_id)` log reference.
    #[getter]
    fn log_ref(&self) -> LogRef {
        LogRef {
            source_peer_id: self.log_ref_inner.source_peer_id.clone(),
            resource_id: self.log_ref_inner.resource_id.clone(),
        }
    }
}

// ─── Session pyclass ─────────────────────────────────────────────────────────

/// Per-process declarative API for the Auki SDK.
///
/// Usage
/// -----
/// ```python
/// import tempfile, pathlib
/// from auki_session import Session, FrameDef, HeadSpec, SensorLogSpec
///
/// tmp = pathlib.Path(tempfile.mkdtemp())
/// s = Session("galbot", "galbot-ctrl").with_storage_root(str(tmp))
///
/// frame_ref = s.register_frame("head_left_optical", FrameDef.ros_optical())
/// sensor_ref = s.register_sensor(
///     "head_left_rgb",
///     {"kind": "camera", "type": "rgb", "width": 1920, "height": 1200,
///      "frame_rate_hz": 30, "pixel_format": "rgb8", "color_space": "srgb",
///      "intrinsics_model": "pinhole", "distortion_model": "brown_conrady",
///      "frame": frame_ref},
/// )
/// clock_ref = s.register_clock(
///     "sdk_clock",
///     {"type": "monotonic_clock", "unit": "ns", "monotonic": True,
///      "scope": "device-local"},
/// )
/// spec = SensorLogSpec(
///     sensor=sensor_ref, clock=clock_ref, frame=frame_ref,
///     head=HeadSpec.rolling(5_000_000_000),
///     segment_duration_ns=1_000_000_000, retention_ns=5_000_000_000,
/// )
/// handle = s.register_sensor_log(spec)
/// rows = s.catalog()   # list of dicts, one per registered log
/// ```
#[pyclass]
pub struct Session {
    inner: Arc<parking_lot::Mutex<session::Session>>,
}

#[pymethods]
impl Session {
    /// Construct a new session.
    ///
    /// Parameters
    /// ----------
    /// peer_id : str
    ///     Unique identifier for this peer (e.g. device serial number).
    /// app_id : str
    ///     Application identifier (e.g. ``"galbot-ctrl"``).
    #[new]
    fn new(peer_id: String, app_id: String) -> Self {
        Self {
            inner: Arc::new(parking_lot::Mutex::new(session::Session::new(
                peer_id, app_id,
            ))),
        }
    }

    /// Set the storage root for registry and log files.
    ///
    /// Mutates the session in-place via the Rust `Session::set_storage_root`
    /// mutator, preserving the session_id ULID.
    /// Call this before registering sensors/logs to avoid losing registrations.
    ///
    /// Returns self for chaining: ``s = Session("p","a").with_storage_root("/tmp")``
    fn with_storage_root(slf: Py<Session>, path: &str, py: Python<'_>) -> Py<Session> {
        {
            let guard = slf.borrow(py);
            guard.inner.lock().set_storage_root(PathBuf::from(path));
        }
        slf
    }

    /// The peer identifier (e.g. device serial number).
    #[getter]
    fn peer_id(&self) -> String {
        self.inner.lock().peer_id()
    }

    /// The application identifier.
    #[getter]
    fn app_id(&self) -> String {
        self.inner.lock().app_id()
    }

    /// The ULID session identifier (unique per Session instance).
    #[getter]
    fn session_id(&self) -> String {
        self.inner.lock().session_id()
    }

    /// The storage root path (defaults to `.`).
    #[getter]
    fn storage_root(&self) -> String {
        self.inner.lock().storage_root().to_string_lossy().into_owned()
    }

    // ─── Registry registration ────────────────────────────────────────

    /// Register a sensor, writing the entry to disk.
    ///
    /// Parameters
    /// ----------
    /// sensor_id : str
    ///     Sensor identifier (e.g. ``"head_left_rgb"``).
    /// body : dict or SensorRegistryEntry-shaped dict
    ///     Sensor body dict as produced by `auki_registry.camera_sensor_entry`
    ///     or equivalent. Must have ``"kind"`` and ``"type"`` fields.
    ///
    /// Returns
    /// -------
    /// RegistryRef
    fn register_sensor(
        &self,
        py: Python<'_>,
        sensor_id: &str,
        body: &Bound<'_, PyAny>,
    ) -> PyResult<RegistryRef> {
        let sensor_body: registry::SensorBody = parse_py(py, body, "body")?;
        let r = self
            .inner
            .lock()
            .register_sensor(sensor_id, sensor_body)
            .map_err(map_session_error)?;
        Ok(RegistryRef {
            peer_id: r.peer_id,
            id: r.id,
            hash: r.hash,
        })
    }

    /// Register a clock, writing the entry to disk.
    ///
    /// Parameters
    /// ----------
    /// clock_id : str
    ///     Clock identifier (e.g. ``"session/sdk_clock"``).
    /// body : dict
    ///     Clock body dict as produced by `auki_registry.monotonic_clock_entry`
    ///     or equivalent.
    ///
    /// Returns
    /// -------
    /// RegistryRef
    fn register_clock(
        &self,
        py: Python<'_>,
        clock_id: &str,
        body: &Bound<'_, PyAny>,
    ) -> PyResult<RegistryRef> {
        let clock_body: registry::ClockBody = parse_py(py, body, "body")?;
        let r = self
            .inner
            .lock()
            .register_clock(clock_id, clock_body)
            .map_err(map_session_error)?;
        Ok(RegistryRef {
            peer_id: r.peer_id,
            id: r.id,
            hash: r.hash,
        })
    }

    /// Register a coordinate frame using a preset.
    ///
    /// Parameters
    /// ----------
    /// frame_id : str
    /// frame_def : FrameDef
    ///
    /// Returns
    /// -------
    /// RegistryRef
    fn register_frame(&self, frame_id: &str, frame_def: &FrameDef) -> PyResult<RegistryRef> {
        let r = self
            .inner
            .lock()
            .register_frame(frame_id, frame_def.clone().into_rust_frame_def())
            .map_err(map_session_error)?;
        Ok(RegistryRef {
            peer_id: r.peer_id,
            id: r.id,
            hash: r.hash,
        })
    }

    /// Register a detector, writing the entry to disk.
    ///
    /// Parameters
    /// ----------
    /// detector_id : str
    /// body : dict
    ///     Detector body dict.
    /// output_types : list[str]
    ///     Detection type strings this detector emits (e.g. ``["aruco"]``).
    ///
    /// Returns
    /// -------
    /// RegistryRef
    fn register_detector(
        &self,
        py: Python<'_>,
        detector_id: &str,
        body: &Bound<'_, PyAny>,
        output_types: Vec<String>,
    ) -> PyResult<RegistryRef> {
        let detector_body: registry::DetectorBody = parse_py(py, body, "body")?;
        let r = self
            .inner
            .lock()
            .register_detector(detector_id, detector_body, output_types)
            .map_err(map_session_error)?;
        Ok(RegistryRef {
            peer_id: r.peer_id,
            id: r.id,
            hash: r.hash,
        })
    }

    // ─── Log registration ─────────────────────────────────────────────

    /// Register a sensor log.
    ///
    /// Returns
    /// -------
    /// SensorLogHandle
    fn register_sensor_log(&self, spec: &SensorLogSpec) -> PyResult<SensorLogHandle> {
        let handle = self
            .inner
            .lock()
            .register_sensor_log(spec.inner.clone())
            .map_err(map_session_error)?;
        Ok(SensorLogHandle {
            resource_id: handle.resource_id().to_string(),
            log_ref_inner: handle.log_ref().clone(),
        })
    }

    /// Register a pose log.
    ///
    /// Returns
    /// -------
    /// PoseLogHandle
    fn register_pose_log(&self, spec: &PoseLogSpec) -> PyResult<PoseLogHandle> {
        let handle = self
            .inner
            .lock()
            .register_pose_log(spec.inner.clone())
            .map_err(map_session_error)?;
        Ok(PoseLogHandle {
            resource_id: handle.resource_id().to_string(),
            log_ref_inner: handle.log_ref().clone(),
        })
    }

    /// Register a time-transform log.
    ///
    /// Returns
    /// -------
    /// TimeTransformLogHandle
    fn register_time_transform_log(
        &self,
        spec: &TimeTransformLogSpec,
    ) -> PyResult<TimeTransformLogHandle> {
        let handle = self
            .inner
            .lock()
            .register_time_transform_log(spec.inner.clone())
            .map_err(map_session_error)?;
        Ok(TimeTransformLogHandle {
            resource_id: handle.resource_id().to_string(),
            log_ref_inner: handle.log_ref().clone(),
        })
    }

    /// Register a detection log.
    ///
    /// Returns
    /// -------
    /// DetectionLogHandle
    fn register_detection_log(&self, spec: &DetectionLogSpec) -> PyResult<DetectionLogHandle> {
        let handle = self
            .inner
            .lock()
            .register_detection_log(spec.inner.clone())
            .map_err(map_session_error)?;
        Ok(DetectionLogHandle {
            resource_id: handle.resource_id().to_string(),
            log_ref_inner: handle.log_ref().clone(),
        })
    }

    // ─── Catalog ──────────────────────────────────────────────────────

    /// Return the catalog as a list of dicts in the canonical `ResourceEntry` JSON shape.
    ///
    /// Returns
    /// -------
    /// list[dict]
    fn catalog(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        let entries = self.inner.lock().catalog();
        entries
            .iter()
            .map(|e| rust_to_pyobject(py, e))
            .collect()
    }

    // ─── Domain stubs ─────────────────────────────────────────────────

    /// Join or create a cluster domain.
    ///
    /// **Not yet supported from Python.** The Rust implementation requires
    /// a pre-built libp2p swarm and stream provider, which cannot be
    /// constructed from Python yet. Raises `NotImplementedError`.
    fn join_domain(&self, _config: &Bound<'_, PyAny>) -> PyResult<()> {
        Err(PyNotImplementedError::new_err(
            "join_domain is not yet supported from Python: \
             requires a pre-built libp2p swarm (DomainConfig). \
             Use the Rust API or wait for a future Python binding.",
        ))
    }

    /// Leave the current cluster domain.
    ///
    /// **Not yet supported from Python.** Raises `NotImplementedError`.
    fn leave_domain(&self) -> PyResult<()> {
        Err(PyNotImplementedError::new_err(
            "leave_domain is not yet supported from Python: \
             requires an active libp2p ClusterManager. \
             Use the Rust API or wait for a future Python binding.",
        ))
    }

    // ─── Materialization stubs ────────────────────────────────────────

    /// Open a remote peer's log locally (deferred to Phase 5).
    ///
    /// Always raises `NotImplementedError` with a message indicating that
    /// full implementation is deferred to Phase 5.
    ///
    /// Parameters
    /// ----------
    /// log_ref : LogRef or dict
    ///     ``{"source_peer_id": ..., "resource_id": ...}``
    /// retention_ns : int
    ///     Desired retention window in nanoseconds.
    /// segment_duration_ns : int
    ///     Desired segment duration in nanoseconds.
    /// Open a remote peer's log locally (deferred to Phase 5).
    ///
    /// Always raises `NotImplementedError`.
    #[pyo3(signature = (log_ref, *, retention_ns, segment_duration_ns))]
    #[allow(unused_variables)]
    fn materialize_remote_log(
        &self,
        log_ref: &Bound<'_, PyAny>,
        retention_ns: u64,
        segment_duration_ns: u64,
    ) -> PyResult<MaterializedLogHandle> {
        // The Rust side returns MaterializationError::NotImplemented unconditionally.
        // We short-circuit here rather than driving the async runtime, so that
        // tests don't need a live tokio context and the Python error is raised cleanly.
        Err(PyNotImplementedError::new_err(
            "not implemented: full materialization deferred to Phase 5 \
             (open remote stream, ingest samples)",
        ))
    }

    /// Resolve a static (single-sample) pose log to its spatial transform.
    ///
    /// Always raises `NotImplementedError` with a message indicating that
    /// full implementation is deferred to Phase 5.
    ///
    /// Parameters
    /// ----------
    /// log_ref : LogRef or dict
    ///     ``{"source_peer_id": ..., "resource_id": ...}``
    ///
    /// Returns
    /// -------
    /// dict
    ///     `SpatialTransform` dict on success (currently unreachable).
    /// Resolve a static (single-sample) pose log to its spatial transform.
    ///
    /// Always raises `NotImplementedError`.
    #[allow(unused_variables)]
    fn resolve_static_transform(
        &self,
        log_ref: &Bound<'_, PyAny>,
    ) -> PyResult<PyObject> {
        // The Rust side returns MaterializationError::NotImplemented unconditionally.
        // We short-circuit here rather than driving the async runtime, so that
        // tests don't need a live tokio context and the Python error is raised cleanly.
        Err(PyNotImplementedError::new_err(
            "not implemented: resolve_static_transform deferred to Phase 5 \
             (requires remote stream path)",
        ))
    }
}

// ─── Module entry point ──────────────────────────────────────────────────────

#[pymodule]
fn auki_session(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RegistryRef>()?;
    m.add_class::<LogRef>()?;
    m.add_class::<HeadSpec>()?;
    m.add_class::<FrameDef>()?;
    m.add_class::<SensorLogSpec>()?;
    m.add_class::<PoseLogSpec>()?;
    m.add_class::<TimeTransformLogSpec>()?;
    m.add_class::<DetectionLogSpec>()?;
    m.add_class::<SensorLogHandle>()?;
    m.add_class::<PoseLogHandle>()?;
    m.add_class::<TimeTransformLogHandle>()?;
    m.add_class::<DetectionLogHandle>()?;
    m.add_class::<MaterializedLogHandle>()?;
    m.add_class::<Session>()?;
    Ok(())
}

// ─── Rust tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::{PyDict, PyList, PyModule};

    // ─── helpers ──────────────────────────────────────────────────────

    fn make_frame_dict<'py>(py: Python<'py>) -> Bound<'py, PyDict> {
        let frame_hash = {
            let entry = registry::FrameRegistryEntry::ros_optical("galbot", "head_optical");
            entry.hash()
        };
        let d = PyDict::new_bound(py);
        d.set_item("peer_id", "galbot").unwrap();
        d.set_item("id", "head_optical").unwrap();
        d.set_item("hash", &frame_hash).unwrap();
        d
    }

    fn make_camera_body_dict<'py>(py: Python<'py>, frame_dict: &Bound<'py, PyDict>) -> Bound<'py, PyDict> {
        let d = PyDict::new_bound(py);
        d.set_item("kind", "camera").unwrap();
        d.set_item("type", "rgb").unwrap();
        d.set_item("width", 1920u32).unwrap();
        d.set_item("height", 1200u32).unwrap();
        d.set_item("frame_rate_hz", 30u32).unwrap();
        d.set_item("pixel_format", "rgb8").unwrap();
        d.set_item("color_space", "srgb").unwrap();
        d.set_item("intrinsics_model", "pinhole").unwrap();
        d.set_item("distortion_model", "brown_conrady").unwrap();
        d.set_item("frame", frame_dict).unwrap();
        d
    }

    fn make_monotonic_clock_dict<'py>(py: Python<'py>) -> Bound<'py, PyDict> {
        let d = PyDict::new_bound(py);
        d.set_item("type", "monotonic_clock").unwrap();
        d.set_item("unit", "ns").unwrap();
        d.set_item("monotonic", true).unwrap();
        d.set_item("scope", "device-local").unwrap();
        d
    }

    // ─── session construction ──────────────────────────────────────────

    #[test]
    fn module_exposes_session_class() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_session").unwrap();
            auki_session(py, &module).unwrap();
            assert!(module.getattr("Session").is_ok());
            assert!(module.getattr("HeadSpec").is_ok());
            assert!(module.getattr("FrameDef").is_ok());
            assert!(module.getattr("SensorLogSpec").is_ok());
            assert!(module.getattr("RegistryRef").is_ok());
            assert!(module.getattr("LogRef").is_ok());
        });
    }

    #[test]
    fn session_new_carries_peer_and_app_id() {
        Python::with_gil(|_py| {
            let s = Session::new("galbot".to_string(), "galbot-ctrl".to_string());
            assert_eq!(s.peer_id(), "galbot");
            assert_eq!(s.app_id(), "galbot-ctrl");
            assert_eq!(s.session_id().len(), 26); // ULID
        });
    }

    #[test]
    fn session_with_storage_root() {
        Python::with_gil(|py| {
            let tmp = tempfile::tempdir().unwrap();
            let s = Py::new(py, Session::new("p".to_string(), "a".to_string())).unwrap();
            let s2 = Session::with_storage_root(s, tmp.path().to_str().unwrap(), py);
            let s2_ref = s2.borrow(py);
            assert_eq!(
                s2_ref.storage_root(),
                tmp.path().to_string_lossy().as_ref()
            );
        });
    }

    #[test]
    fn register_frame_returns_registry_ref() {
        Python::with_gil(|py| {
            let tmp = tempfile::tempdir().unwrap();
            let rust_session = session::Session::new("galbot", "ctrl")
                .with_storage_root(tmp.path().to_path_buf());
            let s = Session {
                inner: Arc::new(parking_lot::Mutex::new(rust_session)),
            };
            let def = FrameDef::ros_optical();
            let r = s.register_frame("head_optical", &def).unwrap();
            assert_eq!(r.peer_id, "galbot");
            assert_eq!(r.id, "head_optical");
            assert!(!r.hash.is_empty());
            let _ = py; // keep py in scope to satisfy test conventions
        });
    }

    #[test]
    fn register_sensor_log_end_to_end() {
        Python::with_gil(|py| {
            let tmp = tempfile::tempdir().unwrap();
            let rust_session = session::Session::new("galbot", "ctrl")
                .with_storage_root(tmp.path().to_path_buf());
            let s = Session {
                inner: Arc::new(parking_lot::Mutex::new(rust_session)),
            };

            // register frame
            let frame_def = FrameDef::ros_optical();
            let frame_ref = s.register_frame("head_optical", &frame_def).unwrap();

            // register sensor via dict
            let frame_dict = make_frame_dict(py);
            // Use the actual frame_ref hash
            frame_dict.set_item("hash", &frame_ref.hash).unwrap();
            let body_dict = make_camera_body_dict(py, &frame_dict);
            let sensor_ref = s
                .register_sensor(py, "head_left_rgb", body_dict.as_any())
                .unwrap();

            // register clock
            let clock_dict = make_monotonic_clock_dict(py);
            let clock_ref = s
                .register_clock(py, "sdk_clock", clock_dict.as_any())
                .unwrap();

            // build spec
            let head = HeadSpec::rolling(5_000_000_000);
            let frame_ref_py = Py::new(py, frame_ref.clone()).unwrap();
            let sensor_ref_py = Py::new(py, sensor_ref.clone()).unwrap();
            let clock_ref_py = Py::new(py, clock_ref.clone()).unwrap();
            let spec = SensorLogSpec::new(
                py,
                sensor_ref_py.bind(py),
                clock_ref_py.bind(py),
                &head,
                1_000_000_000,
                5_000_000_000,
                Some(frame_ref_py.bind(py)),
            )
            .unwrap();

            let handle = s.register_sensor_log(&spec).unwrap();
            assert_eq!(handle.resource_id, "head_left_rgb");
            assert_eq!(handle.log_ref_inner.source_peer_id, "galbot");
            assert_eq!(handle.log_ref_inner.resource_id, "head_left_rgb");

            // catalog
            let catalog = s.catalog(py).unwrap();
            assert_eq!(catalog.len(), 1);
            let row = catalog[0].bind(py);
            let resource_id: String =
                row.get_item("resource_id").unwrap().extract().unwrap();
            assert_eq!(resource_id, "head_left_rgb");
        });
    }

    #[test]
    fn materialize_remote_log_raises_not_implemented() {
        Python::with_gil(|py| {
            let tmp = tempfile::tempdir().unwrap();
            let rust_session = session::Session::new("galbot", "ctrl")
                .with_storage_root(tmp.path().to_path_buf());
            let s = Session {
                inner: Arc::new(parking_lot::Mutex::new(rust_session)),
            };
            let log_ref_dict = PyDict::new_bound(py);
            log_ref_dict
                .set_item("source_peer_id", "galbot")
                .unwrap();
            log_ref_dict
                .set_item("resource_id", "head_left_rgb")
                .unwrap();
            // materialize_remote_log no longer takes py; just log_ref, retention_ns, segment_duration_ns
            let result = s.materialize_remote_log(log_ref_dict.as_any(), 300_000_000_000, 10_000_000_000);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.is_instance_of::<PyNotImplementedError>(py),
                "expected NotImplementedError, got: {:?}", err
            );
        });
    }

    #[test]
    fn resolve_static_transform_raises_not_implemented() {
        Python::with_gil(|py| {
            let tmp = tempfile::tempdir().unwrap();
            let rust_session = session::Session::new("galbot", "ctrl")
                .with_storage_root(tmp.path().to_path_buf());
            let s = Session {
                inner: Arc::new(parking_lot::Mutex::new(rust_session)),
            };
            let log_ref_dict = PyDict::new_bound(py);
            log_ref_dict.set_item("source_peer_id", "park").unwrap();
            log_ref_dict
                .set_item("resource_id", "world->base_link")
                .unwrap();
            // resolve_static_transform no longer takes py
            let result = s.resolve_static_transform(log_ref_dict.as_any());
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.is_instance_of::<PyNotImplementedError>(py),
                "expected NotImplementedError, got: {:?}", err
            );
        });
    }
}
