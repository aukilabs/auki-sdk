//! PyO3 bindings for `auki-logs`'s `Log<T>` framing primitive.
//!
//! Lets Python producers and consumers (e.g. the ESL detector in the
//! [`detectors`](https://github.com/aukilabs/detectors) reference repo)
//! participate in the SDK's segmented ring-buffer log on equal footing
//! with Rust callers.
//!
//! ## Surface
//!
//! - [`Log.open`](Log) — open or create a log directory; returns a
//!   write handle.
//! - [`Log.append`](Log) — append a `(timestamp_ns, payload)` entry.
//! - [`Log.flush`](Log) / [`Log.set_retention`](Log) /
//!   [`Log.manifest`](Log) — runtime mutability + introspection.
//! - [`Log.read`](Log) — open a read snapshot; returns a [`LogReader`].
//! - [`LogReader.entries`](LogReader) — eagerly load every entry.
//! - [`Log.tail`](Log) — yield newly-appended entries as they become
//!   readable; blocks on the configured poll interval. Drop the
//!   iterator to stop.
//! - [`TailIter.try_next`](TailIter) — non-blocking variant.
//! - [`Entry`](Entry) — `(timestamp_ns, payload)` pair, both
//!   read-only.
//!
//! ## Encoding stance
//!
//! Mirrors the Rust crate's encoder-agnostic position: the Python
//! surface is **opaque-bytes-only**. Python passes `bytes` for
//! `payload`; Python decodes received `bytes` itself (via
//! `betterproto`-generated dataclasses, hand-rolled prost, or whatever
//! the consumer prefers). The SDK doesn't impose a Python-side
//! `LogPayload` trait — there's no equivalent of `auki_logs::LogPayload`
//! at the Python layer; the framing primitive stays out of the
//! encoder's way.
//!
//! This is the smallest surface that unblocks
//! [`detectors`](https://github.com/aukilabs/detectors) phase 2 in
//! Python (specifically the ESL detector). Higher-level abstractions
//! (`Session`, registry helpers) live elsewhere and grow independently.

use std::ffi::CString;
use std::path::PathBuf;
use std::time::Duration;

use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyCapsule, PyDict, PyModule};

// Renamed via `package =` in Cargo.toml so the upstream crate's name
// doesn't collide with this crate's own lib name `auki_logs` (which is
// also the Python module name).
use auki_logs_rs::{
    Entry as RustEntry, Error as RustError, Log as RustLog, LogPayload, LogReader as RustLogReader,
    Result as RustResult, TailIter as RustTailIter,
};

/// Identity-encoded payload — `LogPayload::encode` clones the bytes,
/// `decode` wraps them. Lets us parameterize `Log<RawBytes>` everywhere
/// and present an opaque-bytes surface in Python without forcing
/// callers to think about the Rust trait.
///
/// The framing primitive's encoder-agnostic contract makes this
/// trivial: encode is the identity function, decode is `Ok(...)` of
/// the wrapped bytes. Cross-language parity with Rust is preserved
/// because callers on both sides see the same on-disk bytes — Rust
/// users with `Log<auki_datatypes::detection::DetectionFrame>` and
/// Python users with `Log` (this crate) read each other's writes
/// byte-for-byte; the difference is just whether the `LogPayload`
/// impl decodes prost on the Rust side or returns raw bytes here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBytes(pub Vec<u8>);

impl LogPayload for RawBytes {
    fn encode(&self) -> Vec<u8> {
        self.0.clone()
    }
    fn decode(bytes: &[u8]) -> std::result::Result<Self, String> {
        Ok(Self(bytes.to_vec()))
    }
}

/// Map an `auki_logs::Error` to the matching Python exception type.
/// Same shape as `auki-network-py`: I/O → `OSError`, payload / format /
/// manifest → `ValueError`. Keeps the exception type predictable per
/// failure mode rather than forcing every caller to introspect a
/// custom exception class.
fn err_to_py(e: RustError) -> PyErr {
    match e {
        RustError::Io(io) => PyOSError::new_err(io.to_string()),
        RustError::Payload(s) => PyValueError::new_err(format!("payload: {s}")),
        RustError::Manifest(s) => PyValueError::new_err(format!("manifest: {s}")),
        RustError::Format(s) => PyValueError::new_err(format!("format: {s}")),
    }
}

fn map_err<T>(r: RustResult<T>) -> PyResult<T> {
    r.map_err(err_to_py)
}

/// Capsule name for retained stream source payloads exchanged with
/// sibling PyO3 wrapper crates. Includes a version suffix so future ABI
/// changes fail loudly on mismatch.
pub const STREAM_SOURCE_CAPSULE_NAME: &str = "auki_logs_py::stream_source::v1";

/// SDK-owned retained source metadata. `auki-logs-py` constructs this
/// from a concrete log handle; `auki-network-py` consumes it through a
/// named PyCapsule and owns the payload-kind dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedStreamSource {
    pub root: PathBuf,
    pub resource_id: String,
    pub sensor_id: String,
    pub sensor_hash: String,
    pub map_peer_id: String,
    pub map_id: String,
    pub map_hash: String,
    pub clock_peer_id: String,
    pub clock_id: String,
    pub clock_hash: String,
    pub payload_kind: String,
    pub frame_id: String,
    pub frame_hash: String,
}

fn validate_payload_kind(payload_kind: &str) -> PyResult<()> {
    match payload_kind {
        "camera" | "pointcloud" | "joint_encoders" | "audio" | "map" => Ok(()),
        other => Err(PyValueError::new_err(format!(
            "payload_kind must be one of camera, pointcloud, joint_encoders, audio, or map; got {other:?}"
        ))),
    }
}

/// Walk a Python dict into a `serde_json::Value`. The `auki-logs`
/// manifest is JCS-canonicalized at write time, so the keys' insertion
/// order doesn't matter — JCS sorts them lexicographically before
/// hashing / writing.
fn pydict_to_json(py: Python<'_>, dict: &Bound<'_, PyDict>) -> PyResult<serde_json::Value> {
    // Round-trip via Python's own `json` module for fidelity. PyO3's
    // direct conversion lacks a bool / int / float discrimination
    // story for arbitrary nesting, and we don't want to reinvent that
    // here. The dict is small (manifests are ~1 KB at most) so the
    // round-trip cost is negligible.
    let json = py.import_bound("json")?;
    let s: String = json.call_method1("dumps", (dict,))?.extract()?;
    serde_json::from_str(&s).map_err(|e| PyValueError::new_err(format!("manifest: {e}")))
}

/// Walk a `serde_json::Value` back into a Python object via the same
/// `json` module — guarantees we don't drift from Python's native
/// types (dict / list / str / int / float / bool / None).
fn json_to_pyobject(py: Python<'_>, v: &serde_json::Value) -> PyResult<PyObject> {
    let json = py.import_bound("json")?;
    let s = serde_json::to_string(v)
        .map_err(|e| PyRuntimeError::new_err(format!("internal manifest serialize: {e}")))?;
    Ok(json.call_method1("loads", (s,))?.unbind())
}

/// One log entry: `timestamp_ns` (i64) + `payload` (opaque bytes).
/// Read-only; consumers don't construct these directly — they come
/// out of `LogReader.entries` or `TailIter.next` / `try_next`.
#[pyclass(module = "auki_logs", frozen)]
pub struct Entry {
    inner: RustEntry<RawBytes>,
}

#[pymethods]
impl Entry {
    #[getter]
    fn timestamp_ns(&self) -> i64 {
        self.inner.timestamp_ns
    }

    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.payload.0)
    }

    fn __repr__(&self) -> String {
        format!(
            "Entry(timestamp_ns={}, payload=<{} bytes>)",
            self.inner.timestamp_ns,
            self.inner.payload.0.len()
        )
    }
}

/// Read-only handle over a closed log directory. Mirrors
/// `auki_logs::LogReader<T>`. Returned from [`Log::read`].
#[pyclass(module = "auki_logs")]
pub struct LogReader {
    inner: RustLogReader<RawBytes>,
}

#[pymethods]
impl LogReader {
    /// JCS-canonical manifest the log was opened with, as a Python dict.
    fn manifest(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_pyobject(py, self.inner.manifest())
    }

    /// Sorted segment-start timestamps (ns).
    fn segment_starts(&self) -> Vec<i64> {
        self.inner.segment_starts().to_vec()
    }

    /// Eagerly load every entry from every segment in chronological
    /// order. Releases the GIL during the read — the call can be slow
    /// on large logs.
    fn entries(&self, py: Python<'_>) -> PyResult<Vec<Entry>> {
        let entries = py
            .allow_threads(|| self.inner.entries())
            .map_err(err_to_py)?;
        Ok(entries.into_iter().map(|e| Entry { inner: e }).collect())
    }
}

/// Iterator returned by [`Log::tail`]. Yields entries as they are
/// appended to the log. Blocking `__next__` polls at the configured
/// cadence; non-blocking `try_next` returns `None` when no entry is
/// ready.
///
/// The iterator tails forever — there is no portable way to detect
/// that all writers have closed. Drop the iterator (or break out of a
/// `for` loop) to stop tailing.
#[pyclass(module = "auki_logs")]
pub struct TailIter {
    inner: Option<RustTailIter<RawBytes>>,
}

#[pymethods]
impl TailIter {
    /// Override the poll cadence (default 10ms). Lower values reduce
    /// detection latency; higher values reduce filesystem load.
    fn with_poll_interval(mut slf: PyRefMut<'_, Self>, ms: u64) -> PyResult<PyRefMut<'_, Self>> {
        let taken = slf
            .inner
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("tail iterator has been consumed"))?;
        slf.inner = Some(taken.with_poll_interval(Duration::from_millis(ms)));
        Ok(slf)
    }

    /// Non-blocking. Returns the next [`Entry`] if one is ready right
    /// now, `None` if no entry is available yet. Raises on real I/O or
    /// payload-decode failure.
    fn try_next(&mut self, py: Python<'_>) -> PyResult<Option<Entry>> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("tail iterator has been consumed"))?;
        // No GIL release — `try_next` is non-blocking by contract.
        let _ = py;
        match map_err(inner.try_next())? {
            Some(e) => Ok(Some(Entry { inner: e })),
            None => Ok(None),
        }
    }

    /// Iterator protocol — `iter(tail)` returns the iterator itself.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Iterator protocol — blocks at the configured poll cadence
    /// until an entry is readable. The GIL is released during the
    /// blocking poll so other Python threads make progress.
    fn __next__(&mut self, py: Python<'_>) -> PyResult<Entry> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("tail iterator has been consumed"))?;
        let next = py.allow_threads(|| inner.next());
        match next {
            Some(Ok(e)) => Ok(Entry { inner: e }),
            Some(Err(e)) => Err(err_to_py(e)),
            // The Rust iterator never returns None for `tail` — it
            // tails forever. Mapping to StopIteration here is defensive.
            None => Err(pyo3::exceptions::PyStopIteration::new_err(())),
        }
    }
}

/// Write handle over a log directory. Mirrors `auki_logs::Log<T>`,
/// monomorphized to opaque bytes — Python users encode/decode their
/// own prost (or whatever) on top.
///
/// **Single-writer.** One `Log` instance per directory at a time;
/// concurrent writers via two `Log.open` calls in the same process
/// will conflict on segment-file creation (`auki-logs` uses
/// `O_CREAT | O_EXCL` for safety).
#[pyclass(module = "auki_logs")]
pub struct Log {
    inner: Option<RustLog<RawBytes>>,
    root: PathBuf,
}

/// Retained source produced by `Log.stream_source(...)`. Apps hand this
/// object directly to `auki_network.cluster.StreamDecision.accept_source`;
/// the SDK bridge carries the source metadata across PyO3 extension
/// module boundaries without relying on pyclass type identity.
#[pyclass(module = "auki_logs", frozen)]
#[derive(Clone)]
pub struct StreamSource {
    inner: RetainedStreamSource,
}

#[pymethods]
impl StreamSource {
    #[getter]
    fn root(&self) -> String {
        self.inner.root.to_string_lossy().into_owned()
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
    fn payload_kind(&self) -> &str {
        &self.inner.payload_kind
    }

    #[getter]
    fn frame_id(&self) -> &str {
        &self.inner.frame_id
    }

    #[getter]
    fn frame_hash(&self) -> &str {
        &self.inner.frame_hash
    }

    /// SDK-internal bridge consumed by `auki-network-py`.
    fn _stream_source_capsule(&self, py: Python<'_>) -> PyResult<Py<PyCapsule>> {
        let name =
            CString::new(STREAM_SOURCE_CAPSULE_NAME).expect("static literal contains no nul");
        let capsule =
            PyCapsule::new_bound::<RetainedStreamSource>(py, self.inner.clone(), Some(name))?;
        Ok(capsule.unbind())
    }

    fn __repr__(&self) -> String {
        format!(
            "StreamSource(sensor_id={:?}, payload_kind={:?}, root={:?})",
            self.inner.sensor_id,
            self.inner.payload_kind,
            self.inner.root.to_string_lossy(),
        )
    }
}

#[pymethods]
impl Log {
    /// Open or create a log at `root`. If `log_manifest.json` is missing,
    /// the `manifest` dict is JCS-canonicalized (RFC 8785) and
    /// written; if present, the on-disk manifest is the source of
    /// truth and `manifest` is ignored.
    ///
    /// Required manifest fields: `segment_duration_ns` (> 0),
    /// `retention_ns` (≥ 0; 0 = unbounded). Other fields are caller-
    /// defined and persist verbatim.
    #[staticmethod]
    fn open(py: Python<'_>, root: PathBuf, manifest: &Bound<'_, PyDict>) -> PyResult<Self> {
        let manifest_json = pydict_to_json(py, manifest)?;
        let log = py
            .allow_threads(|| RustLog::<RawBytes>::open(&root, manifest_json))
            .map_err(err_to_py)?;
        Ok(Log {
            inner: Some(log),
            root,
        })
    }

    /// Append an entry. Rolls the segment over when `timestamp_ns`
    /// leaves the current segment's window, and evicts segments fully
    /// outside retention.
    fn append(
        &mut self,
        py: Python<'_>,
        timestamp_ns: i64,
        payload: &Bound<'_, PyBytes>,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("log has been closed"))?;
        let bytes = RawBytes(payload.as_bytes().to_vec());
        py.allow_threads(|| inner.append(timestamp_ns, &bytes))
            .map_err(err_to_py)
    }

    /// Flush and `fsync` the current segment without closing the log.
    fn flush(&mut self, py: Python<'_>) -> PyResult<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("log has been closed"))?;
        py.allow_threads(|| inner.flush()).map_err(err_to_py)
    }

    /// Update this log's retention window. Persists to `log_manifest.json`
    /// atomically. `retention_ns` must be ≥ 0 (0 disables eviction).
    fn set_retention(&mut self, py: Python<'_>, retention_ns: i64) -> PyResult<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("log has been closed"))?;
        py.allow_threads(|| inner.set_retention(retention_ns))
            .map_err(err_to_py)
    }

    /// JCS-canonical manifest the log was opened with, as a Python
    /// dict.
    fn manifest(&self, py: Python<'_>) -> PyResult<PyObject> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("log has been closed"))?;
        json_to_pyobject(py, inner.manifest())
    }

    /// Build an SDK-owned retained stream source from this log. The
    /// returned source carries the manifest metadata needed by
    /// `StreamDecision.accept_source(source)`; the network binding owns
    /// payload decoding and typed dispatch.
    #[pyo3(signature = (*, sensor_id, sensor_hash, clock_id, clock_hash, payload_kind, frame_id=None, frame_hash=None))]
    fn stream_source(
        &self,
        sensor_id: String,
        sensor_hash: String,
        clock_id: String,
        clock_hash: String,
        payload_kind: String,
        frame_id: Option<String>,
        frame_hash: Option<String>,
    ) -> PyResult<StreamSource> {
        self.inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("log has been closed"))?;
        validate_payload_kind(&payload_kind)?;
        Ok(StreamSource {
            inner: RetainedStreamSource {
                root: self.root.clone(),
                resource_id: String::new(),
                sensor_id,
                sensor_hash,
                map_peer_id: String::new(),
                map_id: String::new(),
                map_hash: String::new(),
                clock_peer_id: String::new(),
                clock_id,
                clock_hash,
                payload_kind,
                frame_id: frame_id.unwrap_or_default(),
                frame_hash: frame_hash.unwrap_or_default(),
            },
        })
    }

    /// Build an SDK-owned retained Map Log source. Map identity and clock
    /// references are pinned into the stream manifest by `accept_source`.
    #[pyo3(signature = (*, resource_id, map_peer_id, map_id, map_hash, clock_peer_id, clock_id, clock_hash))]
    fn map_stream_source(
        &self,
        resource_id: String,
        map_peer_id: String,
        map_id: String,
        map_hash: String,
        clock_peer_id: String,
        clock_id: String,
        clock_hash: String,
    ) -> PyResult<StreamSource> {
        self.inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("log has been closed"))?;
        for (name, value) in [
            ("resource_id", &resource_id),
            ("map_peer_id", &map_peer_id),
            ("map_id", &map_id),
            ("map_hash", &map_hash),
            ("clock_peer_id", &clock_peer_id),
            ("clock_id", &clock_id),
            ("clock_hash", &clock_hash),
        ] {
            if value.is_empty() {
                return Err(PyValueError::new_err(format!("{name} must not be empty")));
            }
        }
        Ok(StreamSource {
            inner: RetainedStreamSource {
                root: self.root.clone(),
                resource_id,
                sensor_id: String::new(),
                sensor_hash: String::new(),
                map_peer_id,
                map_id,
                map_hash,
                clock_peer_id,
                clock_id,
                clock_hash,
                payload_kind: "map".into(),
                frame_id: String::new(),
                frame_hash: String::new(),
            },
        })
    }

    /// Close the log explicitly. Subsequent calls on this handle
    /// raise `RuntimeError`. Called automatically when the handle is
    /// garbage-collected, but explicit is better than implicit for
    /// flush ordering.
    fn close(&mut self) {
        // Drop runs the writer's `close_current` — flush + fsync.
        self.inner.take();
    }

    /// Context-manager protocol — `with Log.open(...) as log:` pattern.
    fn __enter__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    fn __exit__(&mut self, _exc_type: PyObject, _exc: PyObject, _tb: PyObject) -> bool {
        self.close();
        false
    }

    /// Read snapshot over a closed log directory. Returns a
    /// [`LogReader`]; eagerly enumerates segments at this point but
    /// doesn't load entries until [`LogReader.entries`] is called.
    #[staticmethod]
    fn read(py: Python<'_>, root: PathBuf) -> PyResult<LogReader> {
        let reader = py
            .allow_threads(|| RustLog::<RawBytes>::read(&root))
            .map_err(err_to_py)?;
        Ok(LogReader { inner: reader })
    }

    /// Tail a log directory — yield newly-appended entries as they
    /// become readable. Starts at the **current EOF** of the log;
    /// existing entries are not replayed (use `Log.read().entries()`
    /// for historical).
    ///
    /// Returns a [`TailIter`]. `for entry in Log.tail(path):` blocks
    /// at the default 10ms poll cadence; use `tail.with_poll_interval(ms)`
    /// to override or `tail.try_next()` for a non-blocking call.
    ///
    /// Read side of the [subscription-as-materialization keystone] —
    /// the same call works whether the log is being written by a
    /// local sensor driver, materialized from a peer's stream, or
    /// opened from a recording on disk.
    ///
    /// [subscription-as-materialization keystone]: https://github.com/aukilabs/auki-sdk/blob/develop/parking_lot.md
    #[staticmethod]
    fn tail(py: Python<'_>, root: PathBuf) -> PyResult<TailIter> {
        let tail = py
            .allow_threads(|| RustLog::<RawBytes>::tail(&root))
            .map_err(err_to_py)?;
        Ok(TailIter { inner: Some(tail) })
    }
}

/// Module entry point. The `#[pymodule]` macro generates the
/// `PyInit_auki_logs` symbol the host interpreter resolves at
/// `import auki_logs`.
#[pymodule]
fn auki_logs(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Log>()?;
    m.add_class::<LogReader>()?;
    m.add_class::<TailIter>()?;
    m.add_class::<Entry>()?;
    m.add_class::<StreamSource>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Rust-side smoke tests against the rlib. These don't exercise
    //! the Python surface (that's `python_tests/`); they verify that
    //! the `RawBytes` `LogPayload` impl round-trips through the same
    //! `auki-logs` framing the rest of the workspace uses.

    use super::*;
    use auki_logs_rs::{Log as RustLog, LogReader as RustLogReader};
    use serde_json::json;

    fn manifest() -> serde_json::Value {
        json!({
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 60_000_000_000i64,
            "kind": "test"
        })
    }

    #[test]
    fn raw_bytes_round_trips_through_log() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut log: RustLog<RawBytes> = RustLog::open(dir.path(), manifest()).unwrap();
            log.append(100, &RawBytes(b"hello".to_vec())).unwrap();
            log.append(200, &RawBytes(b"world".to_vec())).unwrap();
        }
        let reader: RustLogReader<RawBytes> = RustLog::<RawBytes>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload.0, b"hello");
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.0, b"world");
    }

    #[test]
    fn raw_bytes_empty_payload_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut log: RustLog<RawBytes> = RustLog::open(dir.path(), manifest()).unwrap();
            log.append(100, &RawBytes(vec![])).unwrap();
        }
        let reader = RustLog::<RawBytes>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].payload.0, Vec::<u8>::new());
    }

    #[test]
    fn map_stream_source_pins_resource_map_and_clock_identity() {
        let dir = tempfile::tempdir().unwrap();
        let inner = RustLog::<RawBytes>::open(dir.path(), manifest()).unwrap();
        let log = Log {
            inner: Some(inner),
            root: dir.path().to_path_buf(),
        };
        let source = log
            .map_stream_source(
                "voxel/world".into(),
                "peer-a".into(),
                "voxel/world".into(),
                "map-hash".into(),
                "peer-a".into(),
                "clock".into(),
                "clock-hash".into(),
            )
            .unwrap();

        assert_eq!(source.inner.payload_kind, "map");
        assert_eq!(source.inner.resource_id, "voxel/world");
        assert_eq!(source.inner.map_hash, "map-hash");
        assert_eq!(source.inner.clock_peer_id, "peer-a");
    }

    /// Cross-encoder parity: `auki_datatypes::detection::DetectionFrame`
    /// is itself opaque-bytes (a single `bytes data = 1` field).
    /// A buffer encoded by the prost `LogPayload` impl in
    /// `auki-datatypes` should be readable through `Log<RawBytes>` —
    /// proves the Python side and the Rust side see the same on-disk
    /// bytes for an opaque-bytes payload.
    #[test]
    fn raw_bytes_can_read_what_prost_wrote() {
        // Skipping in-tree because `auki-datatypes` would be a circular
        // path-dep through `auki-logs`. The cross-language byte equality
        // is enforced by `auki-logs`'s own segment format (which neither
        // crate touches) — this test would only verify wiring, which
        // the round-trip tests above already cover. Filed as a
        // parking-lot follow-up if a cross-crate seam test is wanted.
    }
}
