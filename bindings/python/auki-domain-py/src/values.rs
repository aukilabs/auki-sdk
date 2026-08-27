use std::str::FromStr;

use auki_domain_rs::{
    AuthenticatedParticipantInfo, DdsVerificationKeys, DomainConfig, Identity, MapLogResource,
    MessageChannelResource, Multiaddr, PeerId, ReadFrom, RegistryListEntry, ResourceEntry,
    SignedP2pCredential, StreamRequest,
};
use auki_network::resources_protocol::VariantContent;
use auki_registry::RegistryRef;
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::{PyAny, PyBytes, PyModule},
};
use uuid::Uuid;

#[pyclass(name = "Identity", frozen)]
#[derive(Clone)]
pub(crate) struct PyIdentity {
    pub(crate) inner: Identity,
}

#[pymethods]
impl PyIdentity {
    #[staticmethod]
    fn from_ed25519_seed(seed: Vec<u8>) -> PyResult<Self> {
        let seed: [u8; 32] = seed.try_into().map_err(|seed: Vec<u8>| {
            PyValueError::new_err(format!(
                "Ed25519 seed must contain exactly 32 bytes, got {}",
                seed.len()
            ))
        })?;
        Ok(Self {
            inner: Identity::from_ed25519_seed(&seed),
        })
    }

    #[staticmethod]
    fn from_protobuf_encoding(encoded: Vec<u8>) -> PyResult<Self> {
        Ok(Self {
            inner: Identity::from_protobuf_encoding(&encoded).map_err(crate::runtime_error)?,
        })
    }

    #[staticmethod]
    fn generate() -> Self {
        Self {
            inner: Identity::generate(),
        }
    }

    #[getter]
    fn peer_id(&self) -> String {
        self.inner.peer_id().to_string()
    }

    fn to_protobuf_encoding<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let encoded = self
            .inner
            .to_protobuf_encoding()
            .map_err(crate::runtime_error)?;
        Ok(PyBytes::new_bound(py, &encoded))
    }

    fn __repr__(&self) -> String {
        format!("Identity(peer_id={:?})", self.inner.peer_id().to_string())
    }
}

#[pyclass(name = "DdsVerificationKeys", frozen)]
#[derive(Clone)]
pub(crate) struct PyDdsVerificationKeys {
    pub(crate) inner: DdsVerificationKeys,
}

#[pymethods]
impl PyDdsVerificationKeys {
    #[new]
    #[pyo3(signature = (generation, current_es256_pem, previous_es256_pem=None))]
    fn new(
        generation: u64,
        current_es256_pem: Vec<u8>,
        previous_es256_pem: Option<Vec<u8>>,
    ) -> Self {
        Self {
            inner: DdsVerificationKeys::new(generation, current_es256_pem, previous_es256_pem),
        }
    }

    #[getter]
    fn generation(&self) -> u64 {
        self.inner.generation()
    }

    fn __repr__(&self) -> String {
        format!(
            "DdsVerificationKeys(generation={}, key_material='[redacted]')",
            self.inner.generation()
        )
    }
}

#[pyclass(name = "SignedP2pCredential", frozen)]
#[derive(Clone)]
pub(crate) struct PySignedP2pCredential {
    pub(crate) inner: SignedP2pCredential,
}

#[pymethods]
impl PySignedP2pCredential {
    #[new]
    fn new(compact_token: String) -> PyResult<Self> {
        Ok(Self {
            inner: SignedP2pCredential::new(compact_token).map_err(crate::runtime_error)?,
        })
    }

    fn __repr__(&self) -> &'static str {
        "SignedP2pCredential('[redacted]')"
    }
}

#[pyclass(name = "DomainConfig")]
#[derive(Clone)]
pub(crate) struct PyDomainConfig {
    pub(crate) inner: DomainConfig,
}

#[pymethods]
impl PyDomainConfig {
    #[new]
    fn new(domain_id: &str, identity: &PyIdentity) -> PyResult<Self> {
        let domain_id = Uuid::parse_str(domain_id)
            .map_err(|error| PyValueError::new_err(format!("invalid Domain UUID: {error}")))?;
        Ok(Self {
            inner: DomainConfig::new(domain_id, identity.inner.clone()),
        })
    }

    fn with_listen_addresses(&mut self, addresses: Vec<String>) -> PyResult<()> {
        let addresses = parse_multiaddrs(addresses)?;
        self.inner = self
            .inner
            .clone()
            .with_listen_addresses(addresses)
            .map_err(crate::runtime_error)?;
        Ok(())
    }

    fn with_peer_routes(
        &mut self,
        expected_peer_id: &str,
        candidates: Vec<String>,
    ) -> PyResult<()> {
        let expected_peer = parse_peer_id(expected_peer_id)?;
        self.inner = self
            .inner
            .clone()
            .with_peer_routes(expected_peer, parse_multiaddrs(candidates)?)
            .map_err(crate::runtime_error)?;
        Ok(())
    }

    #[getter]
    fn domain_id(&self) -> String {
        self.inner.domain_id().to_string()
    }

    #[getter]
    fn peer_id(&self) -> String {
        self.inner.peer_id().to_string()
    }
}

#[pyclass(name = "ResourceEntry", frozen)]
#[derive(Clone)]
pub(crate) struct PyResourceEntry {
    pub(crate) inner: ResourceEntry,
}

impl From<ResourceEntry> for PyResourceEntry {
    fn from(inner: ResourceEntry) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyResourceEntry {
    #[staticmethod]
    fn from_json(encoded: &str) -> PyResult<Self> {
        Ok(Self {
            inner: serde_json::from_str(encoded).map_err(|error| {
                PyValueError::new_err(format!("invalid ResourceEntry JSON: {error}"))
            })?,
        })
    }

    #[staticmethod]
    fn from_dict(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let encoded = python_json(py, value)?;
        Ok(Self {
            inner: serde_json::from_str(&encoded).map_err(|error| {
                PyValueError::new_err(format!("invalid ResourceEntry dict: {error}"))
            })?,
        })
    }

    #[getter]
    fn source_peer_id(&self) -> &str {
        &self.inner.source_peer_id
    }

    #[getter]
    fn writer_peer_id(&self) -> &str {
        &self.inner.writer_peer_id
    }

    #[getter]
    fn resource_id(&self) -> &str {
        &self.inner.resource_id
    }

    #[getter]
    fn state(&self) -> &str {
        &self.inner.state
    }

    #[getter]
    fn head(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.optional_field(py, "head")
    }

    #[getter]
    fn extent(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.optional_field(py, "extent")
    }

    #[getter]
    fn available(&self, py: Python<'_>) -> PyResult<PyObject> {
        self.required_field(py, "available")
    }

    #[getter]
    fn sensor(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.optional_field(py, "sensor")
    }

    #[getter]
    fn pose(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.optional_field(py, "pose")
    }

    #[getter]
    fn manifest(&self, py: Python<'_>) -> PyResult<PyObject> {
        self.required_field(py, "manifest")
    }

    #[getter]
    fn variant(&self) -> &'static str {
        match self.inner.variant_content {
            VariantContent::SensorLog { .. } => "sensor_log",
            VariantContent::PoseLog { .. } => "pose_log",
            VariantContent::TimeTransformLog { .. } => "time_transform_log",
            VariantContent::DetectionLog { .. } => "detection_log",
        }
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner).map_err(crate::runtime_error)
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_python(py, &self.to_json()?)
    }

    fn __repr__(&self) -> String {
        format!(
            "ResourceEntry(source_peer_id={:?}, writer_peer_id={:?}, resource_id={:?}, variant={:?})",
            self.source_peer_id(),
            self.writer_peer_id(),
            self.resource_id(),
            self.variant()
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl PyResourceEntry {
    fn value(&self) -> PyResult<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(crate::runtime_error)
    }

    fn required_field(&self, py: Python<'_>, name: &str) -> PyResult<PyObject> {
        let value =
            self.value()?.get(name).cloned().ok_or_else(|| {
                PyValueError::new_err(format!("ResourceEntry has no {name} field"))
            })?;
        json_to_python(
            py,
            &serde_json::to_string(&value).map_err(crate::runtime_error)?,
        )
    }

    fn optional_field(&self, py: Python<'_>, name: &str) -> PyResult<Option<PyObject>> {
        let value = self.value()?.get(name).cloned().unwrap_or_default();
        if value.is_null() {
            Ok(None)
        } else {
            json_to_python(
                py,
                &serde_json::to_string(&value).map_err(crate::runtime_error)?,
            )
            .map(Some)
        }
    }
}

#[pyclass(name = "MapLogResource", frozen)]
#[derive(Clone)]
pub(crate) struct PyMapLogResource {
    pub(crate) inner: MapLogResource,
}

impl From<MapLogResource> for PyMapLogResource {
    fn from(inner: MapLogResource) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyMapLogResource {
    #[staticmethod]
    fn from_json(encoded: &str) -> PyResult<Self> {
        let inner: MapLogResource = serde_json::from_str(encoded)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        inner
            .validate()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(Self { inner })
    }

    #[staticmethod]
    fn from_dict(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::from_json(&python_json(py, value)?)
    }

    #[getter]
    fn source_peer_id(&self) -> &str {
        &self.inner.source_peer_id
    }

    #[getter]
    fn writer_peer_id(&self) -> &str {
        &self.inner.writer_peer_id
    }

    #[getter]
    fn resource_id(&self) -> &str {
        &self.inner.resource_id
    }

    #[getter]
    fn map(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_python(
            py,
            &serde_json::to_string(&self.inner.map).map_err(crate::runtime_error)?,
        )
    }

    #[getter]
    fn clock(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_python(
            py,
            &serde_json::to_string(&self.inner.clock).map_err(crate::runtime_error)?,
        )
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner).map_err(crate::runtime_error)
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_python(py, &self.to_json()?)
    }
}

#[pyclass(name = "MessageChannelResource", frozen)]
#[derive(Clone)]
pub(crate) struct PyMessageChannelResource {
    pub(crate) inner: MessageChannelResource,
}

impl From<MessageChannelResource> for PyMessageChannelResource {
    fn from(inner: MessageChannelResource) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyMessageChannelResource {
    #[new]
    fn new(
        owner_peer_id: &str,
        resource_id: String,
        clock_peer_id: String,
        clock_id: String,
        clock_hash: String,
    ) -> PyResult<Self> {
        let inner = MessageChannelResource {
            owner_peer_id: parse_peer_id(owner_peer_id)?,
            resource_id,
            clock: RegistryRef {
                peer_id: clock_peer_id,
                id: clock_id,
                hash: clock_hash,
            },
        };
        inner
            .validate()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(Self { inner })
    }

    #[getter]
    fn owner_peer_id(&self) -> String {
        self.inner.owner_peer_id.to_string()
    }

    #[getter]
    fn resource_id(&self) -> &str {
        &self.inner.resource_id
    }

    #[getter]
    fn clock_peer_id(&self) -> &str {
        &self.inner.clock.peer_id
    }

    #[getter]
    fn clock_id(&self) -> &str {
        &self.inner.clock.id
    }

    #[getter]
    fn clock_hash(&self) -> &str {
        &self.inner.clock.hash
    }
}

#[pyclass(name = "ReadFrom", frozen)]
#[derive(Clone, Copy)]
pub(crate) struct PyReadFrom {
    pub(crate) inner: ReadFrom,
}

#[pymethods]
impl PyReadFrom {
    #[staticmethod]
    fn latest() -> Self {
        Self {
            inner: ReadFrom::Latest,
        }
    }

    #[staticmethod]
    fn from_start() -> Self {
        Self {
            inner: ReadFrom::FromStart,
        }
    }

    #[staticmethod]
    fn from_timestamp(timestamp_ns: i64) -> Self {
        Self {
            inner: ReadFrom::FromTimestamp(timestamp_ns),
        }
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner {
            ReadFrom::Latest => "latest",
            ReadFrom::FromStart => "from_start",
            ReadFrom::FromTimestamp(_) => "from_timestamp",
        }
    }

    #[getter]
    fn timestamp_ns(&self) -> Option<i64> {
        match self.inner {
            ReadFrom::FromTimestamp(value) => Some(value),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        match self.timestamp_ns() {
            Some(value) => format!("ReadFrom.from_timestamp({value})"),
            None => format!("ReadFrom.{}()", self.kind()),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

#[pyclass(name = "StreamRequest", frozen)]
#[derive(Clone)]
pub(crate) struct PyStreamRequest {
    pub(crate) inner: StreamRequest,
}

#[pymethods]
impl PyStreamRequest {
    #[new]
    #[pyo3(signature = (resource_id, source_peer_id=String::new(), from_=None))]
    fn new(
        resource_id: String,
        source_peer_id: String,
        from_: Option<PyRef<'_, PyReadFrom>>,
    ) -> Self {
        Self {
            inner: StreamRequest {
                resource_id,
                source_peer_id,
                from: from_.map_or(ReadFrom::Latest, |value| value.inner),
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

    #[getter]
    fn from_(&self) -> PyReadFrom {
        PyReadFrom {
            inner: self.inner.from,
        }
    }
}

#[pyclass(name = "RegistryListEntry", frozen)]
#[derive(Clone)]
pub(crate) struct PyRegistryListEntry {
    pub(crate) inner: RegistryListEntry,
}

#[pyclass(name = "ParticipantInfo", frozen)]
#[derive(Clone)]
pub(crate) struct PyParticipantInfo {
    pub(crate) inner: AuthenticatedParticipantInfo,
}

impl From<AuthenticatedParticipantInfo> for PyParticipantInfo {
    fn from(inner: AuthenticatedParticipantInfo) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyParticipantInfo {
    #[new]
    #[pyo3(signature = (app, app_version, name, session_id, session_clock_id, session_clock_hash, session_now_ns, peer_id, app_instance=String::new()))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        app: String,
        app_version: String,
        name: String,
        session_id: String,
        session_clock_id: String,
        session_clock_hash: String,
        session_now_ns: u64,
        peer_id: &str,
        app_instance: String,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: AuthenticatedParticipantInfo {
                app,
                app_version,
                name,
                session_id,
                session_clock_id,
                session_clock_hash,
                session_now_ns,
                peer_id: parse_peer_id(peer_id)?,
                app_instance,
            },
        })
    }

    #[getter]
    fn app(&self) -> &str {
        &self.inner.app
    }
    #[getter]
    fn app_version(&self) -> &str {
        &self.inner.app_version
    }
    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }
    #[getter]
    fn session_id(&self) -> &str {
        &self.inner.session_id
    }
    #[getter]
    fn session_clock_id(&self) -> &str {
        &self.inner.session_clock_id
    }
    #[getter]
    fn session_clock_hash(&self) -> &str {
        &self.inner.session_clock_hash
    }
    #[getter]
    fn session_now_ns(&self) -> u64 {
        self.inner.session_now_ns
    }
    #[getter]
    fn peer_id(&self) -> String {
        self.inner.peer_id.to_string()
    }
    #[getter]
    fn app_instance(&self) -> &str {
        &self.inner.app_instance
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner).map_err(crate::runtime_error)
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_python(py, &self.to_json()?)
    }
}

impl From<RegistryListEntry> for PyRegistryListEntry {
    fn from(inner: RegistryListEntry) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyRegistryListEntry {
    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    #[getter]
    fn hash(&self) -> &str {
        &self.inner.hash
    }
}

pub(crate) fn parse_peer_id(value: &str) -> PyResult<PeerId> {
    PeerId::from_str(value)
        .map_err(|error| PyValueError::new_err(format!("invalid Peer ID {value:?}: {error}")))
}

pub(crate) fn parse_multiaddrs(values: Vec<String>) -> PyResult<Vec<Multiaddr>> {
    values
        .into_iter()
        .map(|value| {
            Multiaddr::from_str(&value).map_err(|error| {
                PyValueError::new_err(format!("invalid multiaddr {value:?}: {error}"))
            })
        })
        .collect()
}

pub(crate) fn python_json(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<String> {
    py.import_bound("json")?
        .call_method1("dumps", (value,))?
        .extract()
}

pub(crate) fn json_to_python(py: Python<'_>, encoded: &str) -> PyResult<PyObject> {
    Ok(py
        .import_bound("json")?
        .call_method1("loads", (encoded,))?
        .unbind())
}

pub(crate) fn registry_kind(value: &str) -> PyResult<auki_domain_rs::RegistryKind> {
    use auki_domain_rs::RegistryKind;
    match value {
        "sensor" => Ok(RegistryKind::Sensor),
        "clock" => Ok(RegistryKind::Clock),
        "frame" => Ok(RegistryKind::Frame),
        "detector" => Ok(RegistryKind::Detector),
        "map" => Ok(RegistryKind::Map),
        "device_model" => Ok(RegistryKind::DeviceModel),
        _ => Err(PyValueError::new_err(format!(
            "unknown registry kind {value:?}"
        ))),
    }
}

pub(crate) fn resource_entries(value: &Bound<'_, PyAny>) -> PyResult<Vec<ResourceEntry>> {
    value
        .iter()?
        .map(|item| {
            let item = item?;
            if let Ok(entry) = item.extract::<PyRef<'_, PyResourceEntry>>() {
                return Ok(entry.inner.clone());
            }
            let py = item.py();
            PyResourceEntry::from_json(&python_json(py, &item)?)
                .map(|entry| entry.inner)
                .map_err(|error| {
                    PyTypeError::new_err(format!(
                        "resource provider rows must be ResourceEntry or compatible dicts: {error}"
                    ))
                })
        })
        .collect()
}

pub(crate) fn map_resources(value: &Bound<'_, PyAny>) -> PyResult<Vec<MapLogResource>> {
    value
        .iter()?
        .map(|item| {
            let item = item?;
            if let Ok(entry) = item.extract::<PyRef<'_, PyMapLogResource>>() {
                return Ok(entry.inner.clone());
            }
            let py = item.py();
            PyMapLogResource::from_json(&python_json(py, &item)?)
                .map(|entry| entry.inner)
                .map_err(|error| {
                    PyTypeError::new_err(format!(
                        "map provider rows must be MapLogResource or compatible dicts: {error}"
                    ))
                })
        })
        .collect()
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyIdentity>()?;
    module.add_class::<PyDdsVerificationKeys>()?;
    module.add_class::<PySignedP2pCredential>()?;
    module.add_class::<PyDomainConfig>()?;
    module.add_class::<PyResourceEntry>()?;
    module.add_class::<PyMapLogResource>()?;
    module.add_class::<PyMessageChannelResource>()?;
    module.add_class::<PyReadFrom>()?;
    module.add_class::<PyStreamRequest>()?;
    module.add_class::<PyRegistryListEntry>()?;
    module.add_class::<PyParticipantInfo>()?;
    Ok(())
}
