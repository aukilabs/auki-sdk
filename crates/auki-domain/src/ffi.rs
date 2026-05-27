use auki_identity::Wallet;
use auki_network::{
    PeerIdentity, Swarm,
    registries_protocol::{RegistryEntryEnvelope, RegistryKind, RegistryRequest},
    resources_protocol::ResourcesResponse,
    sensors_protocol::SensorsResponse,
    stream_protocol::{CameraFrame, DeclineReason, StreamManifest, StreamRequest},
    stream_runtime::{
        StreamDispatch, StreamError, StreamProvider, StreamSubscription as TypedStreamSubscription,
    },
    swarm::{Behaviour, SwarmConfig, build_swarm},
};
use auki_proto::detection::DetectionFrame;
use futures::{StreamExt as _, channel::mpsc as futures_mpsc};
use libp2p_identity::PeerId;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, mpsc as std_mpsc};

use crate::core;

uniffi::setup_scaffolding!();

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum ClusterTargetMode {
    Create,
    Join,
    JoinOrCreate,
    MostRecentOrCreate,
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum BindingDomainError {
    #[error("seed must be exactly 32 bytes, found {len}")]
    InvalidSeedLength { len: u64 },
    #[error("network error: {message}")]
    Network { message: String },
    #[error("domain error: {message}")]
    Domain { message: String },
    #[error("invalid peer id: {message}")]
    InvalidPeerId { message: String },
    #[error("invalid multiaddr: {message}")]
    InvalidMultiaddr { message: String },
    #[error("JSON is not valid: {message}")]
    InvalidJson { message: String },
    #[error("timeout waiting for response")]
    Timeout,
    #[error("closed")]
    Closed,
    #[error("unsupported on this target: {message}")]
    Unsupported { message: String },
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DomainStreamEntry {
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub payload_kind: String,
    pub payload: Vec<u8>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct DomainRuntimeEvent {
    pub kind: String,
    pub peer_id: Option<String>,
    pub payload_json: String,
    pub responder_id: Option<u64>,
}

#[uniffi::export(with_foreign)]
pub trait BindingSensorCatalogProvider: Send + Sync {
    fn snapshot_json(&self) -> Result<String, BindingDomainError>;
}

#[uniffi::export(with_foreign)]
pub trait BindingResourceCatalogProvider: Send + Sync {
    fn snapshot_json(&self) -> Result<String, BindingDomainError>;
}

#[uniffi::export(with_foreign)]
pub trait BindingRegistryEntryProvider: Send + Sync {
    fn entry_json(&self, path: String) -> Result<Option<String>, BindingDomainError>;
}

#[derive(uniffi::Object)]
pub struct DomainClusterManager {
    inner: core::ClusterManager,
    stream_state: Arc<BindingDomainStreamState>,
}

#[derive(uniffi::Object)]
pub struct AukiSignaledDomainPeer {
    inner: Mutex<core::SignaledDomainPeer>,
}

#[derive(uniffi::Object)]
pub struct DomainStreamSubscription {
    manifest_json: String,
    entries: Mutex<std_mpsc::Receiver<DomainStreamRead>>,
    closed: Mutex<bool>,
}

enum DomainStreamRead {
    Entry(DomainStreamEntry),
    End,
    Error(String),
}

struct BindingDomainStreamState {
    next_id: Mutex<u64>,
    open_tx: std_mpsc::Sender<DomainRuntimeEvent>,
    open_rx: Mutex<std_mpsc::Receiver<DomainRuntimeEvent>>,
    pending: Mutex<HashMap<u64, PendingDomainStreamDecision>>,
    active: Mutex<HashMap<u64, ActiveDomainBindingStream>>,
}

type PendingDomainStreamDecision = Arc<(Mutex<Option<BindingDomainStreamDecision>>, Condvar)>;

enum BindingDomainStreamDecision {
    AcceptCamera {
        manifest: StreamManifest,
        source: auki_network::stream_runtime::SourceStream<CameraFrame>,
    },
    AcceptDetection {
        manifest: StreamManifest,
        source: auki_network::stream_runtime::SourceStream<DetectionFrame>,
    },
    Decline {
        reason: DeclineReason,
    },
}

enum ActiveDomainBindingStream {
    Camera(
        futures_mpsc::UnboundedSender<
            Result<auki_network::stream_runtime::StreamItem<CameraFrame>, String>,
        >,
    ),
    Detection(
        futures_mpsc::UnboundedSender<
            Result<auki_network::stream_runtime::StreamItem<DetectionFrame>, String>,
        >,
    ),
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn bootstrap_domain_cluster_manager(
    target_mode: ClusterTargetMode,
    target_name: String,
    wallet_seed: Vec<u8>,
    listen_addrs: Vec<String>,
    advertise_multiaddrs: Vec<String>,
    discovery_url: String,
    daemon_info: core::DaemonInfo,
    agent_version: String,
) -> Result<Arc<DomainClusterManager>, BindingDomainError> {
    let wallet = Wallet::from_seed(&seed32(wallet_seed)?);
    let identity = PeerIdentity::from_wallet(&wallet);
    let swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: parse_multiaddrs(listen_addrs)?,
            agent_version,
            enable_relay_server: false,
        },
    )
    .map_err(|err| BindingDomainError::Network {
        message: err.to_string(),
    })?;

    bootstrap_domain_cluster_manager_with_swarm(
        target_mode,
        target_name,
        identity,
        parse_multiaddrs(advertise_multiaddrs)?,
        discovery_url,
        swarm,
        daemon_info,
    )
    .await
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn bootstrap_domain_cluster_manager_auto_advertise(
    target_mode: ClusterTargetMode,
    target_name: String,
    wallet_seed: Vec<u8>,
    listen_addrs: Vec<String>,
    advertise_multiaddrs_override: Vec<String>,
    advertise_resolution_ms: u64,
    discovery_url: String,
    daemon_info: core::DaemonInfo,
    agent_version: String,
) -> Result<Arc<DomainClusterManager>, BindingDomainError> {
    use std::time::Duration;

    let wallet = Wallet::from_seed(&seed32(wallet_seed)?);
    let identity = PeerIdentity::from_wallet(&wallet);
    let parsed_listen = parse_multiaddrs(listen_addrs)?;
    let mut swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: parsed_listen,
            agent_version,
            enable_relay_server: false,
        },
    )
    .map_err(|err| BindingDomainError::Network {
        message: err.to_string(),
    })?;

    let override_addrs = parse_multiaddrs(advertise_multiaddrs_override)?;
    let override_addrs_opt = if override_addrs.is_empty() {
        None
    } else {
        Some(override_addrs.as_slice())
    };
    let local_multiaddrs = auki_network::swarm::resolve_advertise_multiaddrs(
        &mut swarm,
        override_addrs_opt,
        Duration::from_millis(advertise_resolution_ms),
    )
    .await;

    bootstrap_domain_cluster_manager_with_swarm(
        target_mode,
        target_name,
        identity,
        local_multiaddrs,
        discovery_url,
        swarm,
        daemon_info,
    )
    .await
}

async fn bootstrap_domain_cluster_manager_with_swarm(
    target_mode: ClusterTargetMode,
    target_name: String,
    identity: PeerIdentity,
    local_multiaddrs: Vec<multiaddr::Multiaddr>,
    discovery_url: String,
    swarm: Swarm<Behaviour>,
    daemon_info: core::DaemonInfo,
) -> Result<Arc<DomainClusterManager>, BindingDomainError> {
    let stream_state = Arc::new(BindingDomainStreamState::new());
    let stream_provider = binding_domain_stream_provider(stream_state.clone());

    let manager = core::ClusterManager::bootstrap(
        cluster_target(target_mode, target_name),
        identity,
        local_multiaddrs,
        discovery_url,
        swarm,
        stream_provider,
        daemon_info,
    )
    .await
    .map_err(|err| BindingDomainError::Domain {
        message: err.to_string(),
    })?;

    Ok(Arc::new(DomainClusterManager {
        inner: manager,
        stream_state,
    }))
}

#[uniffi::export]
impl AukiSignaledDomainPeer {
    #[uniffi::constructor]
    pub fn new(
        local_peer_id: String,
        discovery_url: String,
        cluster_name: String,
    ) -> Result<Arc<Self>, BindingDomainError> {
        let inner = core::SignaledDomainPeer::new(local_peer_id, discovery_url, cluster_name)
            .map_err(signaled_domain_error)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(inner),
        }))
    }

    pub fn local_peer_id(&self) -> String {
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .local_peer_id()
            .to_string()
    }

    pub fn cluster_name(&self) -> String {
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .cluster_name()
            .to_string()
    }

    pub fn multiaddrs(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .multiaddrs()
            .expect("SignaledDomainPeer constructor validated multiaddr inputs")
    }

    pub fn set_static_sensor_catalog_json(
        &self,
        catalog_json: String,
    ) -> Result<(), BindingDomainError> {
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .set_static_sensor_catalog_json(catalog_json)
            .map_err(signaled_domain_error)
    }

    pub fn set_static_resource_catalog_json(
        &self,
        catalog_json: String,
    ) -> Result<(), BindingDomainError> {
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .set_static_resource_catalog_json(catalog_json)
            .map_err(signaled_domain_error)
    }

    pub fn set_static_registry_entries_json(
        &self,
        entries_json: String,
    ) -> Result<(), BindingDomainError> {
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .set_static_registry_entries_json(entries_json)
            .map_err(signaled_domain_error)
    }

    pub fn drain_stream_open_requests(&self, _max_events: u32) -> Vec<DomainRuntimeEvent> {
        Vec::new()
    }

    pub fn accept_stream_open(
        &self,
        responder_id: u64,
        manifest_json: String,
    ) -> Result<(), BindingDomainError> {
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .accept_stream_open(responder_id, manifest_json)
            .map_err(signaled_domain_error)
    }

    pub fn decline_stream_open(
        &self,
        responder_id: u64,
        reason: String,
    ) -> Result<(), BindingDomainError> {
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .decline_stream_open(responder_id, reason)
            .map_err(signaled_domain_error)
    }

    pub fn push_stream_entry(
        &self,
        stream_id: u64,
        entry: DomainStreamEntry,
    ) -> Result<(), BindingDomainError> {
        let entry_json = serde_json::json!({
            "timestamp_ns": entry.timestamp_ns,
            "seq": entry.sequence,
            "payload_kind": entry.payload_kind,
            "payload": entry.payload,
        })
        .to_string();
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .push_stream_entry(stream_id, entry_json)
            .map_err(signaled_domain_error)
    }

    pub fn finish_stream(&self, stream_id: u64) -> Result<(), BindingDomainError> {
        self.inner
            .lock()
            .expect("signaled domain peer mutex poisoned")
            .finish_stream(stream_id);
        Ok(())
    }
}

#[uniffi::export]
impl DomainClusterManager {
    pub fn cluster_name(&self) -> String {
        self.inner.cluster_name().to_string()
    }

    pub fn local_peer_id(&self) -> String {
        self.inner.local_peer_id().to_string()
    }

    pub fn local_multiaddrs(&self) -> Vec<String> {
        self.inner
            .local_multiaddrs()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    pub fn is_manager(&self) -> bool {
        self.inner.is_manager()
    }

    pub fn manager_peer_id(&self) -> String {
        self.inner.manager_peer_id().to_string()
    }

    pub fn peer_count(&self) -> u64 {
        self.inner.peer_count() as u64
    }

    pub fn membership_json(&self) -> Result<String, BindingDomainError> {
        serde_json::to_string(&self.inner.membership()).map_err(json_error)
    }

    pub fn participant_info_json(&self) -> Result<String, BindingDomainError> {
        serde_json::to_string(&self.inner.participant_info()).map_err(json_error)
    }

    pub fn domain_time_now(&self) -> Result<i64, BindingDomainError> {
        self.inner
            .domain_time_now()
            .map_err(|err| BindingDomainError::Domain {
                message: err.to_string(),
            })
    }

    pub fn broadcast_diagnostic_message_json(
        &self,
        message_json: String,
    ) -> Result<(), BindingDomainError> {
        let message = parse_json::<core::DiagnosticMessage>(&message_json)?;
        self.inner
            .broadcast_diagnostic_message(message)
            .map_err(domain_network_error)
    }

    pub fn drain_diagnostic_messages_json(&self, max_events: u32) -> Vec<String> {
        self.inner
            .drain_diagnostic_messages()
            .into_iter()
            .take(max_events as usize)
            .filter_map(|message| serde_json::to_string(&diagnostic_message_json(&message)).ok())
            .collect()
    }

    pub fn drain_membership_events_json(&self, max_events: u32) -> Vec<String> {
        if max_events == 0 {
            return Vec::new();
        }
        vec![
            serde_json::json!({
                "kind": "membership_snapshot",
                "membership": self.inner.membership(),
            })
            .to_string(),
        ]
    }

    pub fn clock_sync_estimate_json(&self, peer_id: String) -> Result<String, BindingDomainError> {
        parse_peer_id(&peer_id)?;
        let estimate = self
            .inner
            .clock_sync_estimates()
            .into_iter()
            .find(|estimate| {
                estimate.from_clock_id().contains(&peer_id)
                    || estimate.to_clock_id().contains(&peer_id)
            })
            .map(|estimate| clock_transform_estimate_json(&estimate));
        Ok(serde_json::json!({ "estimate": estimate }).to_string())
    }

    pub fn clock_sync_estimates_json(&self) -> Result<String, BindingDomainError> {
        let estimates = self
            .inner
            .clock_sync_estimates()
            .iter()
            .map(clock_transform_estimate_json)
            .collect::<Vec<_>>();
        json_string(&serde_json::json!({ "estimates": estimates }))
    }

    pub fn domain_clock_estimate_json(&self) -> Result<String, BindingDomainError> {
        let estimate = self.inner.domain_clock_estimate().map_err(domain_error)?;
        json_string(&domain_clock_estimate_json(&estimate))
    }

    pub fn set_sensor_catalog_provider(
        &self,
        provider: Arc<dyn BindingSensorCatalogProvider>,
    ) -> Result<(), BindingDomainError> {
        self.inner
            .set_sensor_catalog_provider(Arc::new(ForeignSensorCatalogProvider { provider }));
        Ok(())
    }

    pub fn set_resource_catalog_provider(
        &self,
        provider: Arc<dyn BindingResourceCatalogProvider>,
    ) -> Result<(), BindingDomainError> {
        self.inner
            .set_resource_catalog_provider(Arc::new(ForeignResourceCatalogProvider { provider }));
        Ok(())
    }

    pub fn set_registry_entry_provider(
        &self,
        provider: Arc<dyn BindingRegistryEntryProvider>,
    ) -> Result<(), BindingDomainError> {
        self.inner
            .set_registry_entry_provider(Arc::new(ForeignRegistryEntryProvider { provider }));
        Ok(())
    }

    pub fn set_static_sensor_catalog_json(
        &self,
        catalog_json: String,
    ) -> Result<(), BindingDomainError> {
        let sensors = parse_sensor_catalog_json(&catalog_json)?;
        self.inner
            .set_sensor_catalog_provider(Arc::new(StaticSensorCatalogProvider { sensors }));
        Ok(())
    }

    pub fn set_static_resource_catalog_json(
        &self,
        catalog_json: String,
    ) -> Result<(), BindingDomainError> {
        let resources = parse_resource_catalog_json(&catalog_json)?;
        self.inner
            .set_resource_catalog_provider(Arc::new(StaticResourceCatalogProvider { resources }));
        Ok(())
    }

    pub fn set_static_registry_entries_json(
        &self,
        entries_json: String,
    ) -> Result<(), BindingDomainError> {
        let entries = parse_registry_entries_json(&entries_json)?;
        self.inner
            .set_registry_entry_provider(Arc::new(StaticRegistryEntryProvider { entries }));
        Ok(())
    }

    pub fn drain_stream_open_requests(&self, max_events: u32) -> Vec<DomainRuntimeEvent> {
        self.stream_state.drain_open_requests(max_events)
    }

    pub fn accept_stream_open(
        &self,
        responder_id: u64,
        manifest_json: String,
    ) -> Result<u64, BindingDomainError> {
        self.stream_state.accept_open(responder_id, &manifest_json)
    }

    pub fn decline_stream_open(
        &self,
        responder_id: u64,
        reason: String,
    ) -> Result<(), BindingDomainError> {
        self.stream_state.decline_open(responder_id, &reason)
    }

    pub fn push_stream_entry(
        &self,
        stream_id: u64,
        entry: DomainStreamEntry,
    ) -> Result<(), BindingDomainError> {
        self.stream_state.push_entry(stream_id, entry)
    }

    pub fn finish_stream(&self, stream_id: u64) -> Result<(), BindingDomainError> {
        self.stream_state.finish_stream(stream_id)
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl DomainClusterManager {
    pub async fn fetch_participant_info_json(
        &self,
        peer_id: String,
    ) -> Result<String, BindingDomainError> {
        let peer_id = parse_peer_id(&peer_id)?;
        let info = self
            .inner
            .fetch_participant_info(peer_id)
            .await
            .map_err(|err| BindingDomainError::Network {
                message: err.to_string(),
            })?;
        serde_json::to_string(&info).map_err(json_error)
    }

    pub async fn admit_peer(
        &self,
        peer_id: String,
        multiaddrs: Vec<String>,
    ) -> Result<String, BindingDomainError> {
        let peer_id = parse_peer_id(&peer_id)?;
        let member = self
            .inner
            .admit_peer(peer_id, parse_multiaddrs(multiaddrs)?)
            .await
            .map_err(domain_error)?;
        serde_json::to_string(&member).map_err(json_error)
    }

    pub async fn fetch_sensor_catalog_json(
        &self,
        peer_id: String,
        _timeout_ms: u64,
    ) -> Result<String, BindingDomainError> {
        let peer_id = parse_peer_id(&peer_id)?;
        let response = self
            .inner
            .fetch_sensors_catalog(peer_id)
            .await
            .map_err(domain_network_error)?;
        json_string(&response)
    }

    pub async fn fetch_resource_catalog_json(
        &self,
        peer_id: String,
        _timeout_ms: u64,
    ) -> Result<String, BindingDomainError> {
        let peer_id = parse_peer_id(&peer_id)?;
        let response = self
            .inner
            .fetch_resources_catalog(peer_id)
            .await
            .map_err(domain_network_error)?;
        json_string(&response)
    }

    pub async fn fetch_registry_entry_json(
        &self,
        peer_id: String,
        path: String,
        _timeout_ms: u64,
    ) -> Result<String, BindingDomainError> {
        let peer_id = parse_peer_id(&peer_id)?;
        let request = parse_json::<RegistryRequest>(&path)?;
        match request.kind {
            RegistryKind::Sensor => {
                let entry = self
                    .inner
                    .fetch_sensor_entry(peer_id, request.id, request.hash)
                    .await
                    .map_err(domain_network_error)?;
                json_string(&entry)
            }
            RegistryKind::Clock => {
                let entry = self
                    .inner
                    .fetch_clock_entry(peer_id, request.id, request.hash)
                    .await
                    .map_err(domain_network_error)?;
                json_string(&entry)
            }
            RegistryKind::Frame => {
                let entry = self
                    .inner
                    .fetch_frame_entry(peer_id, request.id, request.hash)
                    .await
                    .map_err(domain_network_error)?;
                json_string(&entry)
            }
            RegistryKind::Detector => {
                let entry = self
                    .inner
                    .fetch_detector_entry(peer_id, request.id, request.hash)
                    .await
                    .map_err(domain_network_error)?;
                json_string(&entry)
            }
        }
    }

    pub async fn open_stream_bytes(
        &self,
        peer_id: String,
        request_json: String,
        payload_kind: String,
        _timeout_ms: u64,
    ) -> Result<Arc<DomainStreamSubscription>, BindingDomainError> {
        let peer_id = parse_peer_id(&peer_id)?;
        let request = parse_stream_request_json(&request_json)?;
        let payload_kind = normalize_payload_kind(&payload_kind)?;
        match payload_kind.as_str() {
            "camera" => {
                let subscription = self
                    .inner
                    .open_stream::<CameraFrame>(peer_id, request)
                    .await
                    .map_err(domain_network_error)?;
                DomainStreamSubscription::from_typed(subscription, payload_kind)
            }
            "detection" => {
                let subscription = self
                    .inner
                    .open_stream::<DetectionFrame>(peer_id, request)
                    .await
                    .map_err(domain_network_error)?;
                DomainStreamSubscription::from_typed(subscription, payload_kind)
            }
            other => Err(BindingDomainError::Unsupported {
                message: format!("unsupported stream payload kind: {other}"),
            }),
        }
    }

    pub async fn shutdown(&self) -> Result<(), BindingDomainError> {
        self.inner
            .shutdown()
            .await
            .map_err(|err| BindingDomainError::Domain {
                message: err.to_string(),
            })
    }
}

#[uniffi::export]
impl DomainStreamSubscription {
    pub fn manifest_json(&self) -> String {
        self.manifest_json.clone()
    }

    pub fn next_entry(
        &self,
        timeout_ms: u64,
    ) -> Result<Option<DomainStreamEntry>, BindingDomainError> {
        if *self
            .closed
            .lock()
            .expect("domain stream subscription closed mutex poisoned")
        {
            return Ok(None);
        }

        let timeout = std::time::Duration::from_millis(timeout_ms);
        let next = self
            .entries
            .lock()
            .expect("domain stream subscription entries mutex poisoned")
            .recv_timeout(timeout)
            .map_err(|err| match err {
                std_mpsc::RecvTimeoutError::Timeout => BindingDomainError::Timeout,
                std_mpsc::RecvTimeoutError::Disconnected => BindingDomainError::Closed,
            })?;

        match next {
            DomainStreamRead::Entry(entry) => Ok(Some(entry)),
            DomainStreamRead::End => {
                *self
                    .closed
                    .lock()
                    .expect("domain stream subscription closed mutex poisoned") = true;
                Ok(None)
            }
            DomainStreamRead::Error(message) => Err(BindingDomainError::Network { message }),
        }
    }

    pub fn close(&self) -> Result<(), BindingDomainError> {
        *self
            .closed
            .lock()
            .expect("domain stream subscription closed mutex poisoned") = true;
        Ok(())
    }
}

impl DomainStreamSubscription {
    fn from_typed<T>(
        mut subscription: TypedStreamSubscription<T>,
        payload_kind: String,
    ) -> Result<Arc<Self>, BindingDomainError>
    where
        T: prost::Message + Send + 'static,
    {
        let manifest_json = stream_manifest_json(&subscription.manifest);
        let (tx, rx) = std_mpsc::channel();
        tokio::spawn(async move {
            while let Some(next) = subscription.entries.next().await {
                match next {
                    Ok(entry) => {
                        let Ok(timestamp_ns) = u64::try_from(entry.timestamp_ns) else {
                            let _ = tx.send(DomainStreamRead::Error(format!(
                                "stream timestamp is negative: {}",
                                entry.timestamp_ns
                            )));
                            return;
                        };
                        let _ = tx.send(DomainStreamRead::Entry(DomainStreamEntry {
                            sequence: entry.seq,
                            timestamp_ns,
                            payload_kind: payload_kind.clone(),
                            payload: prost::Message::encode_to_vec(&entry.payload),
                        }));
                    }
                    Err(StreamError::EndOfStream { .. }) => {
                        let _ = tx.send(DomainStreamRead::End);
                        return;
                    }
                    Err(err) => {
                        let _ = tx.send(DomainStreamRead::Error(err.to_string()));
                        return;
                    }
                }
            }
            let _ = tx.send(DomainStreamRead::End);
        });

        Ok(Arc::new(Self {
            manifest_json,
            entries: Mutex::new(rx),
            closed: Mutex::new(false),
        }))
    }
}

impl BindingDomainStreamState {
    fn new() -> Self {
        let (open_tx, open_rx) = std_mpsc::channel();
        Self {
            next_id: Mutex::new(1),
            open_tx,
            open_rx: Mutex::new(open_rx),
            pending: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
        }
    }

    fn next_id(&self) -> u64 {
        let mut next = self.next_id.lock().expect("stream id mutex poisoned");
        let id = *next;
        *next = next.saturating_add(1).max(1);
        id
    }

    fn drain_open_requests(&self, max_events: u32) -> Vec<DomainRuntimeEvent> {
        let rx = self
            .open_rx
            .lock()
            .expect("domain stream open receiver mutex poisoned");
        let mut events = Vec::new();
        for _ in 0..max_events {
            match rx.try_recv() {
                Ok(event) => events.push(event),
                Err(std_mpsc::TryRecvError::Empty | std_mpsc::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }
        events
    }

    fn accept_open(
        &self,
        responder_id: u64,
        manifest_json: &str,
    ) -> Result<u64, BindingDomainError> {
        let manifest = parse_stream_manifest_json(manifest_json)?;
        let payload_kind = infer_payload_kind(&manifest.sensor_id)?;
        let responder = self
            .pending
            .lock()
            .expect("domain stream pending mutex poisoned")
            .remove(&responder_id)
            .ok_or(BindingDomainError::Closed)?;
        let stream_id = self.next_id();

        let decision = match payload_kind.as_str() {
            "camera" => {
                let (tx, rx) = futures_mpsc::unbounded();
                self.active
                    .lock()
                    .expect("active domain stream mutex poisoned")
                    .insert(stream_id, ActiveDomainBindingStream::Camera(tx));
                BindingDomainStreamDecision::AcceptCamera {
                    manifest,
                    source: Box::pin(rx),
                }
            }
            "detection" => {
                let (tx, rx) = futures_mpsc::unbounded();
                self.active
                    .lock()
                    .expect("active domain stream mutex poisoned")
                    .insert(stream_id, ActiveDomainBindingStream::Detection(tx));
                BindingDomainStreamDecision::AcceptDetection {
                    manifest,
                    source: Box::pin(rx),
                }
            }
            other => {
                return Err(BindingDomainError::Unsupported {
                    message: format!("unsupported stream payload kind: {other}"),
                });
            }
        };

        let (decision_slot, decision_ready) = &*responder;
        *decision_slot
            .lock()
            .expect("domain stream decision mutex poisoned") = Some(decision);
        decision_ready.notify_one();
        Ok(stream_id)
    }

    fn decline_open(&self, responder_id: u64, reason: &str) -> Result<(), BindingDomainError> {
        let responder = self
            .pending
            .lock()
            .expect("domain stream pending mutex poisoned")
            .remove(&responder_id)
            .ok_or(BindingDomainError::Closed)?;
        let (decision_slot, decision_ready) = &*responder;
        *decision_slot
            .lock()
            .expect("domain stream decision mutex poisoned") =
            Some(BindingDomainStreamDecision::Decline {
                reason: binding_decline_reason(reason),
            });
        decision_ready.notify_one();
        Ok(())
    }

    fn push_entry(
        &self,
        stream_id: u64,
        entry: DomainStreamEntry,
    ) -> Result<(), BindingDomainError> {
        let timestamp_ns =
            i64::try_from(entry.timestamp_ns).map_err(|_| BindingDomainError::Domain {
                message: "stream timestamp does not fit i64".to_string(),
            })?;
        let mut active = self
            .active
            .lock()
            .expect("active domain stream mutex poisoned");
        let stream = active
            .get_mut(&stream_id)
            .ok_or(BindingDomainError::Closed)?;
        match stream {
            ActiveDomainBindingStream::Camera(tx) => {
                let payload =
                    <CameraFrame as prost::Message>::decode(&*entry.payload).map_err(|err| {
                        BindingDomainError::InvalidJson {
                            message: format!("camera payload decode: {err}"),
                        }
                    })?;
                tx.unbounded_send(Ok(auki_network::stream_runtime::StreamItem {
                    timestamp_ns,
                    payload,
                }))
                .map_err(|_| BindingDomainError::Closed)
            }
            ActiveDomainBindingStream::Detection(tx) => {
                let payload =
                    <DetectionFrame as prost::Message>::decode(&*entry.payload).map_err(|err| {
                        BindingDomainError::InvalidJson {
                            message: format!("detection payload decode: {err}"),
                        }
                    })?;
                tx.unbounded_send(Ok(auki_network::stream_runtime::StreamItem {
                    timestamp_ns,
                    payload,
                }))
                .map_err(|_| BindingDomainError::Closed)
            }
        }
    }

    fn finish_stream(&self, stream_id: u64) -> Result<(), BindingDomainError> {
        self.active
            .lock()
            .expect("active domain stream mutex poisoned")
            .remove(&stream_id)
            .ok_or(BindingDomainError::Closed)?;
        Ok(())
    }
}

fn binding_domain_stream_provider(state: Arc<BindingDomainStreamState>) -> StreamProvider {
    Arc::new(move |peer, request| {
        let responder_id = state.next_id();
        let pending_decision = Arc::new((Mutex::new(None), Condvar::new()));
        state
            .pending
            .lock()
            .expect("domain stream pending mutex poisoned")
            .insert(responder_id, pending_decision.clone());
        let _ = state.open_tx.send(DomainRuntimeEvent {
            kind: "stream_open_request".to_string(),
            peer_id: Some(peer.to_string()),
            payload_json: stream_request_json(&request),
            responder_id: Some(responder_id),
        });

        let (decision_slot, decision_ready) = &*pending_decision;
        let decision = {
            let decision = decision_slot
                .lock()
                .expect("domain stream decision mutex poisoned");
            let (mut decision, _) = decision_ready
                .wait_timeout_while(decision, std::time::Duration::from_secs(30), |decision| {
                    decision.is_none()
                })
                .expect("domain stream decision condvar poisoned");
            decision.take()
        };

        match decision {
            Some(BindingDomainStreamDecision::AcceptCamera { manifest, source }) => {
                StreamDispatch::AcceptCamera { manifest, source }
            }
            Some(BindingDomainStreamDecision::AcceptDetection { manifest, source }) => {
                StreamDispatch::AcceptDetection { manifest, source }
            }
            Some(BindingDomainStreamDecision::Decline { reason }) => {
                StreamDispatch::Decline { reason }
            }
            None => {
                state
                    .pending
                    .lock()
                    .expect("domain stream pending mutex poisoned")
                    .remove(&responder_id);
                StreamDispatch::Decline {
                    reason: DeclineReason::producer_shutting_down(),
                }
            }
        }
    })
}

struct ForeignSensorCatalogProvider {
    provider: Arc<dyn BindingSensorCatalogProvider>,
}

impl core::SensorCatalogProvider for ForeignSensorCatalogProvider {
    fn snapshot(&self) -> Vec<core::SensorEntry> {
        match self
            .provider
            .snapshot_json()
            .and_then(|json| parse_sensor_catalog_json(&json))
        {
            Ok(sensors) => sensors,
            Err(err) => {
                eprintln!("auki-domain binding sensor provider failed: {err}");
                Vec::new()
            }
        }
    }
}

struct ForeignResourceCatalogProvider {
    provider: Arc<dyn BindingResourceCatalogProvider>,
}

impl core::ResourceCatalogProvider for ForeignResourceCatalogProvider {
    fn snapshot(&self) -> Vec<core::ResourceEntry> {
        match self
            .provider
            .snapshot_json()
            .and_then(|json| parse_resource_catalog_json(&json))
        {
            Ok(resources) => resources,
            Err(err) => {
                eprintln!("auki-domain binding resource provider failed: {err}");
                Vec::new()
            }
        }
    }
}

struct ForeignRegistryEntryProvider {
    provider: Arc<dyn BindingRegistryEntryProvider>,
}

impl core::RegistryEntryProvider for ForeignRegistryEntryProvider {
    fn entry(&self, request: &RegistryRequest) -> Option<RegistryEntryEnvelope> {
        let request_json = serde_json::to_string(request).ok()?;
        let canonical_json = match self.provider.entry_json(request_json) {
            Ok(Some(json)) => json,
            Ok(None) => return None,
            Err(err) => {
                eprintln!("auki-domain binding registry provider failed: {err}");
                return None;
            }
        };
        let actual_hash = auki_hash::hash_jcs_bytes(canonical_json.as_bytes());
        if actual_hash != request.hash {
            eprintln!(
                "auki-domain binding registry provider hash mismatch for {:?} {:?}: expected {}, got {}",
                request.kind, request.id, request.hash, actual_hash
            );
            return None;
        }
        Some(RegistryEntryEnvelope {
            kind: request.kind,
            id: request.id.clone(),
            hash: request.hash.clone(),
            canonical_json,
        })
    }
}

struct StaticSensorCatalogProvider {
    sensors: Vec<core::SensorEntry>,
}

impl core::SensorCatalogProvider for StaticSensorCatalogProvider {
    fn snapshot(&self) -> Vec<core::SensorEntry> {
        self.sensors.clone()
    }
}

struct StaticResourceCatalogProvider {
    resources: Vec<core::ResourceEntry>,
}

impl core::ResourceCatalogProvider for StaticResourceCatalogProvider {
    fn snapshot(&self) -> Vec<core::ResourceEntry> {
        self.resources.clone()
    }
}

struct StaticRegistryEntryProvider {
    entries: HashMap<String, RegistryEntryEnvelope>,
}

impl core::RegistryEntryProvider for StaticRegistryEntryProvider {
    fn entry(&self, request: &RegistryRequest) -> Option<RegistryEntryEnvelope> {
        self.entries
            .get(&registry_entry_key(
                request.kind,
                &request.id,
                &request.hash,
            ))
            .cloned()
    }
}

#[uniffi::export]
pub fn cluster_membership_new_json(cluster_name: String) -> String {
    core::cluster_membership_new_json(&cluster_name)
}

#[uniffi::export]
pub fn cluster_membership_filename_json(
    membership_json: String,
) -> Result<String, BindingDomainError> {
    core::cluster_membership_filename_json(&membership_json).map_err(Into::into)
}

#[uniffi::export]
pub fn cluster_membership_peer_count_json(
    membership_json: String,
) -> Result<u64, BindingDomainError> {
    core::cluster_membership_peer_count_json(&membership_json).map_err(Into::into)
}

#[uniffi::export]
pub fn cluster_membership_admit_member_json(
    membership_json: String,
    member_json: String,
) -> Result<String, BindingDomainError> {
    core::cluster_membership_admit_member_json(&membership_json, &member_json).map_err(Into::into)
}

#[uniffi::export]
pub fn elect_successor_json(
    membership_json: String,
    local_peer_id: String,
    connected_peer_ids: Vec<String>,
) -> Result<Option<String>, BindingDomainError> {
    core::elect_successor_json(&membership_json, &local_peer_id, connected_peer_ids)
        .map_err(Into::into)
}

fn seed32(seed: Vec<u8>) -> Result<[u8; 32], BindingDomainError> {
    let len = seed.len();
    seed.try_into()
        .map_err(|_| BindingDomainError::InvalidSeedLength { len: len as u64 })
}

fn cluster_target(mode: ClusterTargetMode, name: String) -> core::ClusterTarget {
    match mode {
        ClusterTargetMode::Create => core::ClusterTarget::create(name),
        ClusterTargetMode::Join => core::ClusterTarget::join(name),
        ClusterTargetMode::JoinOrCreate => core::ClusterTarget::join_or_create(name),
        ClusterTargetMode::MostRecentOrCreate => core::ClusterTarget::most_recent_or_create(name),
    }
}

fn parse_peer_id(value: &str) -> Result<PeerId, BindingDomainError> {
    value
        .parse()
        .map_err(|_| BindingDomainError::InvalidPeerId {
            message: value.to_string(),
        })
}

fn parse_multiaddrs(values: Vec<String>) -> Result<Vec<multiaddr::Multiaddr>, BindingDomainError> {
    values
        .into_iter()
        .map(|value| {
            value
                .parse()
                .map_err(|_| BindingDomainError::InvalidMultiaddr { message: value })
        })
        .collect()
}

fn parse_sensor_catalog_json(json: &str) -> Result<Vec<core::SensorEntry>, BindingDomainError> {
    if let Ok(response) = serde_json::from_str::<SensorsResponse>(json) {
        return Ok(response.sensors);
    }
    parse_json::<Vec<core::SensorEntry>>(json)
}

fn parse_resource_catalog_json(json: &str) -> Result<Vec<core::ResourceEntry>, BindingDomainError> {
    if let Ok(response) = serde_json::from_str::<ResourcesResponse>(json) {
        return Ok(response.resources);
    }
    parse_json::<Vec<core::ResourceEntry>>(json)
}

#[derive(serde::Deserialize)]
struct BindingRegistryEntries {
    entries: Vec<RegistryEntryEnvelope>,
}

fn parse_registry_entries_json(
    json: &str,
) -> Result<HashMap<String, RegistryEntryEnvelope>, BindingDomainError> {
    let entries = if let Ok(envelope) = serde_json::from_str::<BindingRegistryEntries>(json) {
        envelope.entries
    } else {
        parse_json::<Vec<RegistryEntryEnvelope>>(json)?
    };

    let mut map = HashMap::new();
    for entry in entries {
        let actual_hash = auki_hash::hash_jcs_bytes(entry.canonical_json.as_bytes());
        if actual_hash != entry.hash {
            return Err(BindingDomainError::InvalidJson {
                message: format!(
                    "registry entry hash mismatch for {:?} {:?}: expected {}, got {}",
                    entry.kind, entry.id, entry.hash, actual_hash
                ),
            });
        }
        map.insert(
            registry_entry_key(entry.kind, &entry.id, &entry.hash),
            entry,
        );
    }
    Ok(map)
}

fn registry_entry_key(kind: RegistryKind, id: &str, hash: &str) -> String {
    format!("{}:{id}:{hash}", kind.as_str())
}

fn normalize_payload_kind(kind: &str) -> Result<String, BindingDomainError> {
    match kind {
        "camera" | "detection" => Ok(kind.to_string()),
        other => Err(BindingDomainError::Unsupported {
            message: format!("unsupported stream payload kind: {other}"),
        }),
    }
}

fn parse_stream_request_json(json: &str) -> Result<StreamRequest, BindingDomainError> {
    let value = serde_json::from_str::<serde_json::Value>(json).map_err(json_error)?;
    Ok(StreamRequest {
        sensor_id: required_json_string(&value, "sensor_id")?,
    })
}

fn stream_request_json(request: &StreamRequest) -> String {
    #[derive(serde::Serialize)]
    struct Ordered<'a> {
        sensor_id: &'a str,
    }
    serde_json::to_string(&Ordered {
        sensor_id: &request.sensor_id,
    })
    .expect("stream request JSON serialization is infallible")
}

fn parse_stream_manifest_json(json: &str) -> Result<StreamManifest, BindingDomainError> {
    let value = serde_json::from_str::<serde_json::Value>(json).map_err(json_error)?;
    Ok(StreamManifest {
        sensor_id: required_json_string(&value, "sensor_id")?,
        sensor_hash: required_json_string(&value, "sensor_hash")?,
        clock_id: required_json_string(&value, "clock_id")?,
        clock_hash: required_json_string(&value, "clock_hash")?,
        frame_id: required_json_string(&value, "frame_id")?,
        frame_hash: required_json_string(&value, "frame_hash")?,
    })
}

fn stream_manifest_json(manifest: &StreamManifest) -> String {
    #[derive(serde::Serialize)]
    struct Ordered<'a> {
        sensor_id: &'a str,
        sensor_hash: &'a str,
        clock_id: &'a str,
        clock_hash: &'a str,
        frame_id: &'a str,
        frame_hash: &'a str,
    }
    serde_json::to_string(&Ordered {
        sensor_id: &manifest.sensor_id,
        sensor_hash: &manifest.sensor_hash,
        clock_id: &manifest.clock_id,
        clock_hash: &manifest.clock_hash,
        frame_id: &manifest.frame_id,
        frame_hash: &manifest.frame_hash,
    })
    .expect("stream manifest JSON serialization is infallible")
}

fn infer_payload_kind(sensor_id: &str) -> Result<String, BindingDomainError> {
    if sensor_id.contains("detect") || sensor_id.contains("detector") {
        Ok("detection".to_string())
    } else {
        Ok("camera".to_string())
    }
}

fn binding_decline_reason(reason: &str) -> DeclineReason {
    match reason {
        "sensor_not_found" => DeclineReason::sensor_not_found(),
        "sensor_unavailable" => DeclineReason::sensor_unavailable(),
        "producer_shutting_down" => DeclineReason::producer_shutting_down(),
        other => DeclineReason::other(other),
    }
}

fn required_json_string(
    value: &serde_json::Value,
    field: &'static str,
) -> Result<String, BindingDomainError> {
    value
        .get(field)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| BindingDomainError::InvalidJson {
            message: format!("missing string field `{field}`"),
        })
}

fn parse_json<T: for<'de> serde::Deserialize<'de>>(json: &str) -> Result<T, BindingDomainError> {
    serde_json::from_str(json).map_err(json_error)
}

fn json_string<T: serde::Serialize>(value: &T) -> Result<String, BindingDomainError> {
    serde_json::to_string(value).map_err(json_error)
}

fn json_error(err: serde_json::Error) -> BindingDomainError {
    BindingDomainError::InvalidJson {
        message: err.to_string(),
    }
}

fn domain_error(err: impl std::fmt::Display) -> BindingDomainError {
    BindingDomainError::Domain {
        message: err.to_string(),
    }
}

fn domain_network_error(err: impl std::fmt::Display) -> BindingDomainError {
    BindingDomainError::Network {
        message: err.to_string(),
    }
}

fn signaled_domain_error(err: core::SignaledDomainPeerError) -> BindingDomainError {
    match err {
        core::SignaledDomainPeerError::MissingClusterName => BindingDomainError::Domain {
            message: err.to_string(),
        },
        core::SignaledDomainPeerError::Network(message) => BindingDomainError::Network { message },
        core::SignaledDomainPeerError::InvalidJson(message) => {
            BindingDomainError::InvalidJson { message }
        }
    }
}

impl From<core::DomainDataError> for BindingDomainError {
    fn from(err: core::DomainDataError) -> Self {
        match err {
            core::DomainDataError::InvalidJson(message) => Self::InvalidJson { message },
            core::DomainDataError::InvalidPeerId(message) => Self::InvalidPeerId { message },
            core::DomainDataError::InvalidMultiaddr(message) => Self::InvalidMultiaddr { message },
        }
    }
}

fn diagnostic_message_json(message: &core::InboundDiagnosticMessage) -> serde_json::Value {
    serde_json::json!({
        "peer_id": message.peer_id.to_string(),
        "message": message.message,
    })
}

fn clock_transform_estimate_json(estimate: &core::ClockTransformEstimate) -> serde_json::Value {
    serde_json::json!({
        "from_clock_id": estimate.from_clock_id(),
        "from_clock_hash": estimate.from_clock_hash(),
        "to_clock_id": estimate.to_clock_id(),
        "to_clock_hash": estimate.to_clock_hash(),
        "offset_ns": estimate.offset_ns,
        "uncertainty_ns": estimate.uncertainty_ns,
        "observed_at_clock_ns": estimate.observed_at_clock_ns,
        "sample_count": estimate.sample_count,
    })
}

fn domain_clock_estimate_json(estimate: &core::DomainClockEstimate) -> serde_json::Value {
    serde_json::json!({
        "cluster_name": estimate.cluster_name,
        "local_clock_id": estimate.local_clock_id,
        "local_clock_hash": estimate.local_clock_hash,
        "domain_clock_id": estimate.domain_clock_id,
        "domain_clock_hash": estimate.domain_clock_hash,
        "backing_peer_id": estimate.backing_peer_id,
        "backing_clock_id": estimate.backing_clock_id,
        "backing_clock_hash": estimate.backing_clock_hash,
        "peer_to_backing_offset_ns": estimate.peer_to_backing_offset_ns,
        "backing_to_domain_offset_ns": estimate.backing_to_domain_offset_ns,
        "total_offset_ns": estimate.total_offset_ns,
        "uncertainty_ns": estimate.uncertainty_ns,
        "observed_at_clock_ns": estimate.observed_at_clock_ns,
    })
}
