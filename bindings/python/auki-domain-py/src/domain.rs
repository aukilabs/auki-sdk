use std::{path::PathBuf, sync::Arc, time::Duration};

use auki_domain_rs::{
    Domain as RustDomain, DomainAuthority as RustDomainAuthority,
    DomainBuilder as RustDomainBuilder, DomainPeers as RustDomainPeers,
    DomainRoutes as RustDomainRoutes, DomainStatus as RustDomainStatus, KnownPeer as RustKnownPeer,
    KnownPeerEvent as RustKnownPeerEvent, KnownPeerSubscription as RustKnownPeerSubscription,
    MessageChannelReceiver as RustMessageChannelReceiver,
    MessageChannelSender as RustMessageChannelSender, MessageEvent as RustMessageEvent,
    ResourcesRequest, ResourcesRequestV3,
};
use parking_lot::Mutex as SyncMutex;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    pyclass::{PyTraverseError, PyVisit},
    types::{PyAny, PyBytes, PyModule},
};
use tokio::sync::{Mutex, RwLock, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    providers::ProviderSlots,
    runtime_error, session_bridge, streams,
    values::{
        PyDdsVerificationKeys, PyDomainConfig, PyMapLogResource, PyMessageChannelResource,
        PyParticipantInfo, PyReadFrom, PyRegistryListEntry, PyResourceEntry, PySignedP2pCredential,
        PyStreamRequest, json_to_python, parse_peer_id, registry_kind,
    },
};

type CleanupResult = Result<(), String>;
const PYTHON_LEAVE_TIMEOUT: Duration = Duration::from_secs(35);

enum DomainJoinFailure {
    InitialAuthorityRequired,
    Runtime(String),
}

impl DomainJoinFailure {
    fn into_py_err(self) -> PyErr {
        match self {
            Self::InitialAuthorityRequired => PyValueError::new_err(
                "initial DdsVerificationKeys and SignedP2pCredential are required",
            ),
            Self::Runtime(error) => runtime_error(error),
        }
    }
}

/// A successful native join which Python has not claimed yet.
///
/// The native join runs independently from the Python awaitable. Keeping the
/// joined Domain behind this guard makes every cancellation race equivalent:
/// whether the receiver was already closed or the success was queued just
/// before cancellation, dropping the unclaimed value performs the same
/// ordered Domain shutdown instead of merely dropping the libp2p runtime.
struct UnclaimedJoinedDomain {
    joined: Option<(RustDomain, ProviderSlots)>,
}

impl UnclaimedJoinedDomain {
    fn new(domain: RustDomain, providers: ProviderSlots) -> Self {
        Self {
            joined: Some((domain, providers)),
        }
    }

    fn claim(mut self) -> (RustDomain, ProviderSlots) {
        self.joined
            .take()
            .expect("unclaimed joined Domain can only be claimed once")
    }
}

impl Drop for UnclaimedJoinedDomain {
    fn drop(&mut self) {
        let Some((domain, providers)) = self.joined.take() else {
            return;
        };
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            providers.fence();
            let result = domain.leave().await;
            providers.finish_cleanup().await;
            if let Err(error) = result {
                tracing::warn!(%error, "unclaimed Python Domain cleanup failed");
            }
        });
    }
}

struct DomainOwner {
    lifecycle: CancellationToken,
    state: Arc<RwLock<Option<RustDomain>>>,
    providers: ProviderSlots,
    cleanup: SyncMutex<Option<watch::Sender<Option<CleanupResult>>>>,
}

impl DomainOwner {
    fn new(domain: RustDomain, providers: ProviderSlots) -> Arc<Self> {
        Arc::new(Self {
            lifecycle: CancellationToken::new(),
            state: Arc::new(RwLock::new(Some(domain))),
            providers,
            cleanup: SyncMutex::new(None),
        })
    }

    fn begin_cleanup(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.lifecycle.cancel();
        // Stop native services from starting new Python work before beginning
        // ordered Domain shutdown. Active sources remain registered so their
        // async-generator finalizers can be drained after stream pumps stop.
        self.providers.fence();
        let mut cleanup = self.cleanup.lock();
        if let Some(sender) = cleanup.as_ref() {
            return sender.subscribe();
        }

        let (sender, receiver) = watch::channel(None);
        *cleanup = Some(sender.clone());
        let state = Arc::clone(&self.state);
        let providers = self.providers.clone();
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            // This owner task is intentionally not time-limited while waiting
            // for an in-flight binding operation to release its read lease.
            // Python callers have their own bounded wait below; timing one of
            // them out must never poison or abandon eventual native cleanup.
            let result = match state.write().await.take() {
                Some(domain) => domain.leave().await.map_err(|error| error.to_string()),
                None => Ok(()),
            };
            providers.finish_cleanup().await;
            sender.send_replace(Some(result));
        });
        receiver
    }
}

async fn await_cleanup(mut receiver: watch::Receiver<Option<CleanupResult>>) -> PyResult<()> {
    loop {
        if let Some(result) = receiver.borrow_and_update().clone() {
            return result.map_err(runtime_error);
        }
        receiver.changed().await.map_err(runtime_error)?;
    }
}

async fn await_cleanup_bounded(receiver: watch::Receiver<Option<CleanupResult>>) -> PyResult<()> {
    tokio::time::timeout(PYTHON_LEAVE_TIMEOUT, await_cleanup(receiver))
        .await
        .map_err(|_| {
            PyRuntimeError::new_err(format!(
                "timed out after {}s waiting for Domain cleanup; native cleanup is still running",
                PYTHON_LEAVE_TIMEOUT.as_secs()
            ))
        })?
}

fn stopped_error() -> PyErr {
    PyRuntimeError::new_err("Domain is stopped")
}

macro_rules! domain_read {
    ($owner:expr, |$domain:ident| $operation:expr) => {{
        let owner = $owner;
        async move {
            let state = tokio::select! {
                biased;
                _ = owner.lifecycle.cancelled() => return Err(stopped_error()),
                state = owner.state.read() => state,
            };
            let $domain = state.as_ref().ok_or_else(stopped_error)?;
            tokio::select! {
                biased;
                _ = owner.lifecycle.cancelled() => Err(stopped_error()),
                result = $operation => result,
            }
        }
    }};
}

macro_rules! domain_write {
    ($owner:expr, |$domain:ident| $operation:expr) => {{
        let owner = $owner;
        async move {
            let mut state = tokio::select! {
                biased;
                _ = owner.lifecycle.cancelled() => return Err(stopped_error()),
                state = owner.state.write() => state,
            };
            let $domain = state.as_mut().ok_or_else(stopped_error)?;
            if owner.lifecycle.is_cancelled() {
                return Err(stopped_error());
            }
            $operation
        }
    }};
}

macro_rules! registry_fetch {
    ($self:ident, $py:ident, $peer_id:ident, $id:ident, $hash:ident, $method:ident) => {{
        let owner = Arc::clone(&$self.owner);
        let peer = parse_peer_id($peer_id)?;
        pyo3_async_runtimes::tokio::future_into_py(
            $py,
            domain_read!(owner, |domain| async move {
                let entry = domain
                    .$method(peer, $id, $hash)
                    .await
                    .map_err(runtime_error)?;
                let encoded = serde_json::to_string(&entry).map_err(runtime_error)?;
                Python::with_gil(|py| json_to_python(py, &encoded))
            }),
        )
    }};
}

#[pyclass(name = "DomainBuilder")]
pub(crate) struct PyDomainBuilder {
    peer: Option<Arc<auki_session_rs::Peer>>,
    session: Option<Arc<auki_session_rs::Session>>,
    config: Option<auki_domain_rs::DomainConfig>,
    authority: Option<(
        auki_domain_rs::DdsVerificationKeys,
        auki_domain_rs::SignedP2pCredential,
    )>,
    participant_info: Option<auki_domain_rs::AuthenticatedParticipantInfo>,
    participant_provider: bool,
    providers: ProviderSlots,
    resource_provider: bool,
    map_provider: bool,
    stream_provider: bool,
    registry_app_root: Option<PathBuf>,
    message_channels: Vec<(auki_domain_rs::MessageChannelResource, usize)>,
    consumed: bool,
}

#[pymethods]
impl PyDomainBuilder {
    #[new]
    fn new(
        peer: &Bound<'_, PyAny>,
        session: &Bound<'_, PyAny>,
        config: &PyDomainConfig,
    ) -> PyResult<Self> {
        Ok(Self {
            peer: Some(session_bridge::peer(peer)?),
            session: Some(session_bridge::session(session)?),
            config: Some(config.inner.clone()),
            authority: None,
            participant_info: None,
            participant_provider: false,
            providers: ProviderSlots::default(),
            resource_provider: false,
            map_provider: false,
            stream_provider: false,
            registry_app_root: None,
            message_channels: Vec::new(),
            consumed: false,
        })
    }

    fn authority(
        &mut self,
        keys: &PyDdsVerificationKeys,
        credential: &PySignedP2pCredential,
    ) -> PyResult<()> {
        self.ensure_mutable()?;
        self.authority = Some((keys.inner.clone(), credential.inner.clone()));
        Ok(())
    }

    fn participant_info(&mut self, info: &PyParticipantInfo) -> PyResult<()> {
        self.ensure_mutable()?;
        self.providers.clear_participant();
        self.participant_info = Some(info.inner.clone());
        self.participant_provider = false;
        Ok(())
    }

    fn participant_info_provider(&mut self, py: Python<'_>, callback: Py<PyAny>) -> PyResult<()> {
        self.ensure_mutable()?;
        self.providers.set_participant(py, callback)?;
        self.participant_info = None;
        self.participant_provider = true;
        Ok(())
    }

    fn resource_catalog_provider(&mut self, callback: Py<PyAny>) -> PyResult<()> {
        self.ensure_mutable()?;
        self.providers.set_resource(callback);
        self.resource_provider = true;
        Ok(())
    }

    fn map_catalog_provider(&mut self, callback: Py<PyAny>) -> PyResult<()> {
        self.ensure_mutable()?;
        self.providers.set_maps(callback);
        self.map_provider = true;
        Ok(())
    }

    fn stream_provider(&mut self, py: Python<'_>, callback: Py<PyAny>) -> PyResult<()> {
        self.ensure_mutable()?;
        self.providers.set_stream(py, callback)?;
        self.stream_provider = true;
        Ok(())
    }

    fn registry_app_root(&mut self, path: PathBuf) -> PyResult<()> {
        self.ensure_mutable()?;
        self.registry_app_root = Some(path);
        Ok(())
    }

    #[pyo3(signature = (resource, capacity=64))]
    fn message_channel(
        &mut self,
        resource: &PyMessageChannelResource,
        capacity: usize,
    ) -> PyResult<()> {
        self.ensure_mutable()?;
        if capacity == 0 {
            return Err(PyValueError::new_err(
                "message channel capacity must be greater than zero",
            ));
        }
        self.message_channels
            .push((resource.inner.clone(), capacity));
        Ok(())
    }

    fn join<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.ensure_mutable()?;
        self.consumed = true;
        let peer = self.peer.take().expect("checked unconsumed builder");
        let session = self.session.take().expect("checked unconsumed builder");
        let config = self.config.take().expect("checked unconsumed builder");
        let authority = self.authority.take();
        let participant_info = self.participant_info.take();
        let participant_provider = self.participant_provider;
        let providers = std::mem::take(&mut self.providers);
        let resource_provider = self.resource_provider;
        let map_provider = self.map_provider;
        let stream_provider = self.stream_provider;
        let registry_app_root = self.registry_app_root.take();
        let message_channels = std::mem::take(&mut self.message_channels);

        let (sender, receiver) = oneshot::channel();
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let result = async {
                let (keys, credential) =
                    authority.ok_or(DomainJoinFailure::InitialAuthorityRequired)?;
                let mut builder = RustDomainBuilder::new(peer.as_ref(), session.as_ref(), config)
                    .authority(keys, credential);
                if let Some(info) = participant_info {
                    builder = builder.participant_info_provider(Arc::new(move || info.clone()));
                } else if participant_provider {
                    builder = builder.participant_info_provider(providers.participant_provider());
                }
                if resource_provider {
                    builder = builder.resource_catalog_provider(providers.resource_provider());
                }
                if map_provider {
                    builder = builder.map_catalog_provider(providers.map_provider());
                }
                if stream_provider {
                    builder = builder.stream_provider(providers.stream_provider());
                }
                if let Some(root) = registry_app_root {
                    builder = builder.registry_app_root(root);
                }
                for (resource, capacity) in message_channels {
                    builder = builder
                        .message_channel(resource, capacity)
                        .map_err(|error| DomainJoinFailure::Runtime(error.to_string()))?;
                }
                let domain = builder
                    .join()
                    .await
                    .map_err(|error| DomainJoinFailure::Runtime(error.to_string()))?;
                Ok(UnclaimedJoinedDomain::new(domain, providers.clone()))
            }
            .await;

            if result.is_err() {
                providers.clear();
            }
            // On a closed receiver, `send` returns the guarded success and its
            // Drop path owns cleanup. On an open receiver, the same guard
            // remains in the channel until Python claims or cancels it.
            let _ = sender.send(result);
        });

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let joined = receiver
                .await
                .map_err(|_| runtime_error("native Domain join task ended without a result"))?
                .map_err(DomainJoinFailure::into_py_err)?;
            let (domain, providers) = joined.claim();
            Python::with_gil(|py| Py::new(py, PyDomain::from_rust(domain, providers)))
        })
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        self.providers.visit(&visit)
    }

    fn __clear__(&mut self) {
        self.providers.clear();
    }
}

impl PyDomainBuilder {
    fn ensure_mutable(&self) -> PyResult<()> {
        if self.consumed {
            Err(PyRuntimeError::new_err(
                "DomainBuilder was consumed by join()",
            ))
        } else {
            Ok(())
        }
    }
}

#[pyclass(name = "Domain")]
pub(crate) struct PyDomain {
    owner: Arc<DomainOwner>,
    domain_id: String,
    peer_id: String,
    listen_addresses: Vec<String>,
    authority: RustDomainAuthority,
    routes: RustDomainRoutes,
    peers: RustDomainPeers,
    status: watch::Receiver<RustDomainStatus>,
    providers: ProviderSlots,
}

impl PyDomain {
    fn from_rust(domain: RustDomain, providers: ProviderSlots) -> Self {
        let domain_id = domain.domain_id().to_string();
        let peer_id = domain.peer_id().to_string();
        let listen_addresses = domain
            .listen_addresses()
            .iter()
            .map(ToString::to_string)
            .collect();
        let authority = domain.authority();
        let routes = domain.routes();
        let peers = domain.known_peers();
        let status = domain.subscribe_status();
        Self {
            owner: DomainOwner::new(domain, providers.clone()),
            domain_id,
            peer_id,
            listen_addresses,
            authority,
            routes,
            peers,
            status,
            providers,
        }
    }
}

#[pymethods]
impl PyDomain {
    #[staticmethod]
    fn builder(
        peer: &Bound<'_, PyAny>,
        session: &Bound<'_, PyAny>,
        config: &PyDomainConfig,
    ) -> PyResult<PyDomainBuilder> {
        PyDomainBuilder::new(peer, session, config)
    }

    #[getter]
    fn domain_id(&self) -> &str {
        &self.domain_id
    }

    #[getter]
    fn peer_id(&self) -> &str {
        &self.peer_id
    }

    #[getter]
    fn listen_addresses(&self) -> Vec<String> {
        self.listen_addresses.clone()
    }

    fn status(&self) -> PyDomainStatus {
        PyDomainStatus::from(*self.status.borrow())
    }

    fn subscribe_status(&self) -> PyDomainStatusSubscription {
        PyDomainStatusSubscription {
            current: self.status.clone(),
            receiver: Arc::new(Mutex::new(self.status.clone())),
            cancellation: CancellationToken::new(),
        }
    }

    fn authority(&self) -> PyDomainAuthority {
        PyDomainAuthority {
            inner: self.authority.clone(),
        }
    }

    fn routes(&self) -> PyDomainRoutes {
        PyDomainRoutes {
            inner: self.routes.clone(),
        }
    }

    fn known_peers(&self) -> PyKnownPeers {
        PyKnownPeers {
            inner: self.peers.clone(),
        }
    }

    fn catalog(&self) -> PyResult<Vec<PyResourceEntry>> {
        if self.owner.lifecycle.is_cancelled() {
            return Err(stopped_error());
        }
        let state = self
            .owner
            .state
            .try_read()
            .map_err(|_| PyRuntimeError::new_err("another Domain operation is in progress"))?;
        let domain = state.as_ref().ok_or_else(stopped_error)?;
        Ok(domain
            .catalog()
            .map_err(runtime_error)?
            .into_iter()
            .map(PyResourceEntry::from)
            .collect())
    }

    fn set_resource_catalog_provider(&self, callback: Py<PyAny>) -> PyResult<()> {
        if self.owner.lifecycle.is_cancelled() {
            return Err(stopped_error());
        }
        let state = self
            .owner
            .state
            .try_read()
            .map_err(|_| PyRuntimeError::new_err("another Domain operation is in progress"))?;
        let domain = state.as_ref().ok_or_else(stopped_error)?;
        self.providers.set_resource(callback);
        domain
            .set_resource_catalog_provider(self.providers.resource_provider())
            .map_err(runtime_error)
    }

    fn set_map_catalog_provider(&self, callback: Py<PyAny>) -> PyResult<()> {
        if self.owner.lifecycle.is_cancelled() {
            return Err(stopped_error());
        }
        let state = self
            .owner
            .state
            .try_read()
            .map_err(|_| PyRuntimeError::new_err("another Domain operation is in progress"))?;
        let domain = state.as_ref().ok_or_else(stopped_error)?;
        self.providers.set_maps(callback);
        domain
            .set_map_catalog_provider(self.providers.map_provider())
            .map_err(runtime_error)
    }

    fn set_registry_app_root(&self, path: PathBuf) -> PyResult<()> {
        if self.owner.lifecycle.is_cancelled() {
            return Err(stopped_error());
        }
        let state = self
            .owner
            .state
            .try_read()
            .map_err(|_| PyRuntimeError::new_err("another Domain operation is in progress"))?;
        state
            .as_ref()
            .ok_or_else(stopped_error)?
            .set_registry_app_root(path)
            .map_err(runtime_error)
    }

    fn fetch_participant_info<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let peer = parse_peer_id(peer_id)?;
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                let info = domain
                    .fetch_participant_info(peer)
                    .await
                    .map_err(runtime_error)?;
                Python::with_gil(|py| Py::new(py, PyParticipantInfo::from(info)))
            }),
        )
    }

    #[pyo3(signature = (peer_id, variants=None))]
    fn fetch_resources_catalog<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        variants: Option<Vec<String>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let peer = parse_peer_id(peer_id)?;
        let request = resource_request(variants)?;
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                let resources = domain
                    .fetch_resources_catalog_with(peer, request)
                    .await
                    .map_err(runtime_error)?
                    .resources
                    .into_iter()
                    .map(PyResourceEntry::from)
                    .collect::<Vec<_>>();
                Ok(resources)
            }),
        )
    }

    fn fetch_message_channels<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let peer = parse_peer_id(peer_id)?;
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                let response = domain
                    .fetch_resources_catalog_v3_with(
                        peer,
                        ResourcesRequestV3 {
                            variants: vec![auki_domain_rs::ResourceVariantV3::MessageChannel],
                        },
                    )
                    .await
                    .map_err(runtime_error)?;
                Ok(response
                    .resources
                    .into_iter()
                    .filter_map(|resource| match resource {
                        auki_domain_rs::ResourceEntryV3::MessageChannel(resource) => {
                            Some(PyMessageChannelResource::from(resource))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>())
            }),
        )
    }

    fn fetch_map_catalog<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let peer = parse_peer_id(peer_id)?;
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                Ok(domain
                    .fetch_map_catalog(peer)
                    .await
                    .map_err(runtime_error)?
                    .resources
                    .into_iter()
                    .map(PyMapLogResource::from)
                    .collect::<Vec<_>>())
            }),
        )
    }

    fn list_registry_entries<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        kind: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let peer = parse_peer_id(peer_id)?;
        let kind = registry_kind(kind)?;
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                Ok(domain
                    .list_registry_entries(peer, kind)
                    .await
                    .map_err(runtime_error)?
                    .into_iter()
                    .map(PyRegistryListEntry::from)
                    .collect::<Vec<_>>())
            }),
        )
    }

    fn fetch_sensor_entry<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        id: String,
        hash: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        registry_fetch!(self, py, peer_id, id, hash, fetch_sensor_entry)
    }
    fn fetch_clock_entry<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        id: String,
        hash: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        registry_fetch!(self, py, peer_id, id, hash, fetch_clock_entry)
    }
    fn fetch_frame_entry<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        id: String,
        hash: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        registry_fetch!(self, py, peer_id, id, hash, fetch_frame_entry)
    }
    fn fetch_detector_entry<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        id: String,
        hash: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        registry_fetch!(self, py, peer_id, id, hash, fetch_detector_entry)
    }
    fn fetch_map_entry<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        id: String,
        hash: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        registry_fetch!(self, py, peer_id, id, hash, fetch_map_entry)
    }
    fn fetch_device_model_entry<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        id: String,
        hash: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        registry_fetch!(self, py, peer_id, id, hash, fetch_device_model_entry)
    }

    fn fetch_blob<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        sha256: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let peer = parse_peer_id(peer_id)?;
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                let bytes = domain
                    .fetch_blob(peer, sha256)
                    .await
                    .map_err(runtime_error)?;
                Ok(Python::with_gil(|py| {
                    PyBytes::new_bound(py, &bytes).unbind()
                }))
            }),
        )
    }

    fn take_message_channel_receiver<'py>(
        &self,
        py: Python<'py>,
        resource_id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_write!(owner, |domain| {
                match domain.take_message_channel_receiver(&resource_id) {
                    Some(receiver) => Python::with_gil(|py| {
                        Py::new(
                            py,
                            PyMessageChannelReceiver {
                                resource: PyMessageChannelResource::from(
                                    receiver.resource().clone(),
                                ),
                                inner: Arc::new(Mutex::new(receiver)),
                                cancellation: CancellationToken::new(),
                            },
                        )
                        .map(|value| value.into_any())
                    }),
                    None => Ok(Python::with_gil(|py| py.None())),
                }
            }),
        )
    }

    fn open_message_channel<'py>(
        &self,
        py: Python<'py>,
        resource: &PyMessageChannelResource,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let resource = resource.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                let sender = domain
                    .open_message_channel(resource.owner_peer_id, &resource)
                    .await
                    .map_err(runtime_error)?;
                Python::with_gil(|py| Py::new(py, PyMessageChannelSender { inner: sender }))
            }),
        )
    }

    fn send_message<'py>(
        &self,
        py: Python<'py>,
        resource: &PyMessageChannelResource,
        message_type: String,
        timestamp_ns: i64,
        payload: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let resource = resource.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                domain
                    .send_message(&resource, message_type, timestamp_ns, payload)
                    .await
                    .map_err(runtime_error)
            }),
        )
    }

    #[pyo3(signature = (peer_id, request, payload_kind=None))]
    fn open_stream<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        request: &PyStreamRequest,
        payload_kind: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let peer = parse_peer_id(peer_id)?;
        let request = request.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                let kind = match payload_kind {
                    Some(kind) => kind,
                    None => {
                        let rows = domain
                            .fetch_resources_catalog(peer)
                            .await
                            .map_err(runtime_error)?
                            .resources;
                        streams::infer_payload_kind(
                            &rows,
                            &request.source_peer_id,
                            &request.resource_id,
                        )?
                        .to_string()
                    }
                };
                let subscription = streams::open(domain, peer, request, &kind).await?;
                Python::with_gil(|py| Py::new(py, subscription))
            }),
        )
    }

    #[pyo3(signature = (peer_id, resource, from_=None))]
    fn open_map_stream<'py>(
        &self,
        py: Python<'py>,
        peer_id: &str,
        resource: &PyMapLogResource,
        from_: Option<PyRef<'_, PyReadFrom>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let owner = Arc::clone(&self.owner);
        let peer = parse_peer_id(peer_id)?;
        let resource = resource.clone();
        let from = from_.map_or(auki_domain_rs::ReadFrom::FromStart, |value| value.inner);
        pyo3_async_runtimes::tokio::future_into_py(
            py,
            domain_read!(owner, |domain| async move {
                let subscription = streams::open_map(domain, peer, &resource, from).await?;
                Python::with_gil(|py| Py::new(py, subscription))
            }),
        )
    }

    fn leave<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_cleanup();
        pyo3_async_runtimes::tokio::future_into_py(py, await_cleanup_bounded(cleanup))
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        self.providers.visit(&visit)
    }

    fn __clear__(&mut self) {
        self.owner.begin_cleanup();
    }
}

impl Drop for PyDomain {
    fn drop(&mut self) {
        self.owner.begin_cleanup();
    }
}

fn resource_request(variants: Option<Vec<String>>) -> PyResult<ResourcesRequest> {
    use auki_network::resources_protocol::Variant;
    let variants = variants
        .unwrap_or_default()
        .into_iter()
        .map(|variant| match variant.as_str() {
            "sensor_log" => Ok(Variant::SensorLog),
            "pose_log" => Ok(Variant::PoseLog),
            "time_transform_log" => Ok(Variant::TimeTransformLog),
            "detection_log" => Ok(Variant::DetectionLog),
            _ => Err(PyValueError::new_err(format!(
                "unknown Resource Catalog variant {variant:?}"
            ))),
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(ResourcesRequest { variants })
}

#[pyclass(name = "DomainAuthority", frozen)]
#[derive(Clone)]
pub(crate) struct PyDomainAuthority {
    inner: RustDomainAuthority,
}

#[pymethods]
impl PyDomainAuthority {
    #[getter]
    fn peer_id(&self) -> String {
        self.inner.peer_id().to_string()
    }
    #[getter]
    fn domain_id(&self) -> String {
        self.inner.domain_id().to_string()
    }
    fn install_verification_keys<'py>(
        &self,
        py: Python<'py>,
        keys: &PyDdsVerificationKeys,
    ) -> PyResult<Bound<'py, PyAny>> {
        let authority = self.inner.clone();
        let keys = keys.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            authority
                .install_verification_keys(keys)
                .await
                .map_err(runtime_error)
        })
    }
    fn install_credential<'py>(
        &self,
        py: Python<'py>,
        credential: &PySignedP2pCredential,
    ) -> PyResult<Bound<'py, PyAny>> {
        let authority = self.inner.clone();
        let credential = credential.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            authority
                .install_credential(credential)
                .await
                .map_err(runtime_error)
        })
    }
    fn peer_public_key_protobuf<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new_bound(
            py,
            &self
                .inner
                .peer_public_key_protobuf()
                .map_err(runtime_error)?,
        ))
    }
    fn sign_peer_challenge<'py>(
        &self,
        py: Python<'py>,
        challenge: &[u8],
    ) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new_bound(
            py,
            &self
                .inner
                .sign_peer_challenge(challenge)
                .map_err(runtime_error)?,
        ))
    }
}

#[pyclass(name = "DomainStatus", frozen)]
#[derive(Clone)]
pub(crate) struct PyDomainStatus {
    state: String,
    failure: Option<String>,
}

impl From<RustDomainStatus> for PyDomainStatus {
    fn from(status: RustDomainStatus) -> Self {
        match status {
            RustDomainStatus::Ready => Self {
                state: "ready".into(),
                failure: None,
            },
            RustDomainStatus::CredentialUnavailable => Self {
                state: "credential_unavailable".into(),
                failure: None,
            },
            RustDomainStatus::Failed(failure) => Self {
                state: "failed".into(),
                failure: Some(format!("{failure:?}")),
            },
            RustDomainStatus::Stopped => Self {
                state: "stopped".into(),
                failure: None,
            },
        }
    }
}

#[pymethods]
impl PyDomainStatus {
    #[getter]
    fn state(&self) -> &str {
        &self.state
    }
    #[getter]
    fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
    fn __repr__(&self) -> String {
        format!(
            "DomainStatus(state={:?}, failure={:?})",
            self.state, self.failure
        )
    }
}

#[pyclass(name = "DomainStatusSubscription")]
pub(crate) struct PyDomainStatusSubscription {
    current: watch::Receiver<RustDomainStatus>,
    receiver: Arc<Mutex<watch::Receiver<RustDomainStatus>>>,
    cancellation: CancellationToken,
}

#[pymethods]
impl PyDomainStatusSubscription {
    fn current(&self) -> PyDomainStatus {
        PyDomainStatus::from(*self.current.borrow())
    }
    fn changed<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let receiver = Arc::clone(&self.receiver);
        let cancellation = self.cancellation.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut receiver = receiver.lock().await;
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(stopped_error()),
                result = receiver.changed() => result.map_err(runtime_error)?,
            }
            Ok(PyDomainStatus::from(*receiver.borrow_and_update()))
        })
    }
}

impl Drop for PyDomainStatusSubscription {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[pyclass(name = "PeerRoutes", frozen)]
pub(crate) struct PyPeerRoutes {
    expected_peer_id: String,
    candidates: Vec<String>,
}

#[pymethods]
impl PyPeerRoutes {
    #[getter]
    fn expected_peer_id(&self) -> &str {
        &self.expected_peer_id
    }
    #[getter]
    fn candidates(&self) -> Vec<String> {
        self.candidates.clone()
    }
}

#[pyclass(name = "DomainRouteSnapshot", frozen)]
pub(crate) struct PyDomainRouteSnapshot {
    revision: u64,
    peers: Vec<PyPeerRoutes>,
    total_candidates: usize,
}

#[pymethods]
impl PyDomainRouteSnapshot {
    #[getter]
    fn revision(&self) -> u64 {
        self.revision
    }
    #[getter]
    fn peers(&self) -> Vec<PyPeerRoutes> {
        self.peers
            .iter()
            .map(|p| PyPeerRoutes {
                expected_peer_id: p.expected_peer_id.clone(),
                candidates: p.candidates.clone(),
            })
            .collect()
    }
    #[getter]
    fn total_candidates(&self) -> usize {
        self.total_candidates
    }
}

#[pyclass(name = "DomainRoutes", frozen)]
#[derive(Clone)]
pub(crate) struct PyDomainRoutes {
    inner: RustDomainRoutes,
}

#[pymethods]
impl PyDomainRoutes {
    fn replace(&self, peer_id: &str, candidates: Vec<String>) -> PyResult<PyDomainRouteSnapshot> {
        let peer = parse_peer_id(peer_id)?;
        let candidates = crate::values::parse_multiaddrs(candidates)?;
        self.inner
            .replace(peer, candidates)
            .map(route_snapshot)
            .map_err(runtime_error)
    }
    fn remove(&self, peer_id: &str) -> PyResult<PyDomainRouteSnapshot> {
        self.inner
            .remove(parse_peer_id(peer_id)?)
            .map(route_snapshot)
            .map_err(runtime_error)
    }
    fn snapshot(&self) -> PyResult<PyDomainRouteSnapshot> {
        self.inner
            .snapshot()
            .map(route_snapshot)
            .map_err(runtime_error)
    }
}

fn route_snapshot(snapshot: auki_domain_rs::DomainRouteSnapshot) -> PyDomainRouteSnapshot {
    PyDomainRouteSnapshot {
        revision: snapshot.revision,
        total_candidates: snapshot.total_candidates,
        peers: snapshot
            .peers
            .into_iter()
            .map(|peer| PyPeerRoutes {
                expected_peer_id: peer.expected_peer.to_string(),
                candidates: peer
                    .candidates
                    .into_iter()
                    .map(|route| route.to_string())
                    .collect(),
            })
            .collect(),
    }
}

#[pyclass(name = "KnownPeer", frozen)]
#[derive(Clone)]
pub(crate) struct PyKnownPeer {
    peer_id: String,
    authenticated_until: String,
    application_name: Option<String>,
    application_version: Option<String>,
    participant_info: Option<PyParticipantInfo>,
}

impl From<RustKnownPeer> for PyKnownPeer {
    fn from(peer: RustKnownPeer) -> Self {
        Self {
            peer_id: peer.peer_id().to_string(),
            authenticated_until: peer.authenticated_until().to_rfc3339(),
            application_name: peer.application().map(|a| a.name.clone()),
            application_version: peer.application().map(|a| a.version.clone()),
            participant_info: peer
                .participant_info()
                .cloned()
                .map(PyParticipantInfo::from),
        }
    }
}

#[pymethods]
impl PyKnownPeer {
    #[getter]
    fn peer_id(&self) -> &str {
        &self.peer_id
    }
    #[getter]
    fn authenticated_until(&self) -> &str {
        &self.authenticated_until
    }
    #[getter]
    fn application_name(&self) -> Option<&str> {
        self.application_name.as_deref()
    }
    #[getter]
    fn application_version(&self) -> Option<&str> {
        self.application_version.as_deref()
    }
    #[getter]
    fn participant_info(&self) -> Option<PyParticipantInfo> {
        self.participant_info.clone()
    }
}

#[pyclass(name = "KnownPeerEvent", frozen)]
pub(crate) struct PyKnownPeerEvent {
    kind: String,
    peer: Option<PyKnownPeer>,
    peer_id: String,
    reason: Option<String>,
}

#[pymethods]
impl PyKnownPeerEvent {
    #[getter]
    fn kind(&self) -> &str {
        &self.kind
    }
    #[getter]
    fn peer(&self) -> Option<PyKnownPeer> {
        self.peer.clone()
    }
    #[getter]
    fn peer_id(&self) -> &str {
        &self.peer_id
    }
    #[getter]
    fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

fn peer_event(event: RustKnownPeerEvent) -> PyKnownPeerEvent {
    match event {
        RustKnownPeerEvent::Appeared(peer) => {
            let peer = PyKnownPeer::from(peer);
            PyKnownPeerEvent {
                kind: "appeared".into(),
                peer_id: peer.peer_id.clone(),
                peer: Some(peer),
                reason: None,
            }
        }
        RustKnownPeerEvent::Updated(peer) => {
            let peer = PyKnownPeer::from(peer);
            PyKnownPeerEvent {
                kind: "updated".into(),
                peer_id: peer.peer_id.clone(),
                peer: Some(peer),
                reason: None,
            }
        }
        RustKnownPeerEvent::Disappeared { peer_id, reason } => PyKnownPeerEvent {
            kind: "disappeared".into(),
            peer: None,
            peer_id: peer_id.to_string(),
            reason: Some(format!("{reason:?}")),
        },
    }
}

#[pyclass(name = "KnownPeerSubscription")]
pub(crate) struct PyKnownPeerSubscription {
    inner: Arc<Mutex<RustKnownPeerSubscription>>,
    cancellation: CancellationToken,
}

#[pymethods]
impl PyKnownPeerSubscription {
    fn recv<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let cancellation = self.cancellation.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let event = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(stopped_error()),
                event = async { inner.lock().await.recv().await } => event.map_err(runtime_error)?,
            };
            Ok(peer_event(event))
        })
    }
}

impl Drop for PyKnownPeerSubscription {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[pyclass(name = "KnownPeers", frozen)]
#[derive(Clone)]
pub(crate) struct PyKnownPeers {
    inner: RustDomainPeers,
}

#[pymethods]
impl PyKnownPeers {
    fn snapshot(&self) -> Vec<PyKnownPeer> {
        self.inner
            .snapshot()
            .peers()
            .iter()
            .cloned()
            .map(PyKnownPeer::from)
            .collect()
    }
    fn peer_count(&self) -> usize {
        self.inner.peer_count()
    }
    fn subscribe(&self) -> PyKnownPeerSubscription {
        PyKnownPeerSubscription {
            inner: Arc::new(Mutex::new(self.inner.subscribe())),
            cancellation: CancellationToken::new(),
        }
    }
}

#[pyclass(name = "MessageEvent", frozen)]
pub(crate) struct PyMessageEvent {
    inner: RustMessageEvent,
}

#[pymethods]
impl PyMessageEvent {
    #[getter]
    fn resource_id(&self) -> &str {
        &self.inner.channel.resource_id
    }
    #[getter]
    fn sender_peer_id(&self) -> String {
        self.inner.sender.to_string()
    }
    #[getter]
    fn message_type(&self) -> &str {
        &self.inner.r#type
    }
    #[getter]
    fn r#type(&self) -> &str {
        &self.inner.r#type
    }
    #[getter]
    fn timestamp_ns(&self) -> i64 {
        self.inner.timestamp_ns
    }
    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.payload)
    }
}

#[pyclass(name = "MessageChannelReceiver")]
pub(crate) struct PyMessageChannelReceiver {
    resource: PyMessageChannelResource,
    inner: Arc<Mutex<RustMessageChannelReceiver>>,
    cancellation: CancellationToken,
}

#[pymethods]
impl PyMessageChannelReceiver {
    #[getter]
    fn resource(&self) -> PyMessageChannelResource {
        self.resource.clone()
    }

    fn recv<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        let cancellation = self.cancellation.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let event = tokio::select! {
                biased;
                _ = cancellation.cancelled() => None,
                event = async { inner.lock().await.recv().await } => event,
            };
            match event {
                Some(event) => Python::with_gil(|py| {
                    Py::new(py, PyMessageEvent { inner: event }).map(|value| value.into_any())
                }),
                None => Ok(Python::with_gil(|py| py.None())),
            }
        })
    }
}

impl Drop for PyMessageChannelReceiver {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[pyclass(name = "MessageChannelSender", frozen)]
#[derive(Clone)]
pub(crate) struct PyMessageChannelSender {
    inner: RustMessageChannelSender,
}

#[pymethods]
impl PyMessageChannelSender {
    fn send<'py>(
        &self,
        py: Python<'py>,
        message_type: String,
        timestamp_ns: i64,
        payload: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let sender = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            sender
                .send(message_type, timestamp_ns, payload)
                .await
                .map_err(runtime_error)
        })
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyDomainBuilder>()?;
    module.add_class::<PyDomain>()?;
    module.add_class::<PyDomainAuthority>()?;
    module.add_class::<PyDomainStatus>()?;
    module.add_class::<PyDomainStatusSubscription>()?;
    module.add_class::<PyDomainRoutes>()?;
    module.add_class::<PyDomainRouteSnapshot>()?;
    module.add_class::<PyPeerRoutes>()?;
    module.add_class::<PyKnownPeers>()?;
    module.add_class::<PyKnownPeer>()?;
    module.add_class::<PyKnownPeerEvent>()?;
    module.add_class::<PyKnownPeerSubscription>()?;
    module.add_class::<PyMessageEvent>()?;
    module.add_class::<PyMessageChannelReceiver>()?;
    module.add_class::<PyMessageChannelSender>()?;
    Ok(())
}
