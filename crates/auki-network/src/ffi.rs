#[cfg(all(feature = "app_instance", feature = "swarm"))]
use crate::app_instance;
use crate::core;
#[cfg(all(feature = "discovery_client", feature = "swarm"))]
use crate::discovery_client::{self, CreateClusterOutcome, DiscoveryClient};
#[cfg(feature = "message_node")]
use crate::message_node::{MessageNode, MessageNodeConfig};
#[cfg(feature = "swarm")]
use crate::{
    diagnostic_protocol::DiagnosticMessage,
    info_protocol::{InfoRequest, InfoResponse},
    join_protocol::{JoinRequest, JoinResponse},
    network_runtime::{self, HeartbeatTimestampSource},
    registries_protocol::{RegistryRequest, RegistryResponse},
    resources_protocol::{ResourcesRequest, ResourcesResponse},
    sensors_protocol::{SensorsRequest, SensorsResponse},
    stream_protocol::{CameraFrame, DeclineReason, StreamManifest, StreamRequest},
    stream_runtime::{
        OpenStreamError, SourceStream, StreamDispatch, StreamError, StreamItem,
        StreamSubscription as TypedStreamSubscription,
    },
    swarm::{self, SwarmConfig},
};
#[cfg(all(feature = "discovery_client", feature = "swarm"))]
use crate::{signaled_address, signaled_peer::SignaledPeerCore};
use auki_identity::Wallet;
#[cfg(feature = "swarm")]
use auki_proto::detection::DetectionFrame;
#[cfg(feature = "swarm")]
use futures::{StreamExt as _, channel::mpsc as futures_mpsc};
#[cfg(any(feature = "message_node", feature = "swarm"))]
use libp2p_identity::PeerId as Libp2pPeerId;
#[cfg(any(feature = "message_node", feature = "swarm"))]
use multiaddr::Multiaddr;
#[cfg(feature = "swarm")]
use prost::Message;
use std::sync::Arc;
#[cfg(all(feature = "discovery_client", feature = "swarm"))]
use std::time::Duration;
#[cfg(feature = "swarm")]
use std::{
    collections::HashMap,
    sync::{Mutex, mpsc as std_mpsc},
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(feature = "swarm")]
use tokio::{
    runtime::Builder,
    sync::{mpsc, oneshot},
};

uniffi::setup_scaffolding!();

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("seed must be exactly 32 bytes, found {len}")]
    InvalidSeedLength { len: u64 },
    #[error("invalid peer id: {value}")]
    InvalidPeerId { value: String },
    #[error("invalid multiaddr: {value}")]
    InvalidMultiaddr { value: String },
    #[error("message node: {message}")]
    MessageNode { message: String },
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BindingAllowedPeer {
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BindingSwarmConfig {
    pub wallet_seed: Vec<u8>,
    pub listen_multiaddrs: Vec<String>,
    pub agent_version: String,
    pub allowed_peers: Vec<BindingAllowedPeer>,
    pub heartbeat_clock_id: Option<String>,
    pub heartbeat_clock_hash_hex: Option<String>,
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BindingUpdateReport {
    pub accepted: Vec<String>,
    pub rejected_json: String,
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum BindingNetworkError {
    #[error("seed must be exactly 32 bytes, found {len}")]
    InvalidSeedLength { len: u64 },
    #[error("invalid peer id: {message}")]
    InvalidPeerId { message: String },
    #[error("invalid multiaddr: {message}")]
    InvalidMultiaddr { message: String },
    #[error("runtime error: {message}")]
    Runtime { message: String },
    #[error("invalid json: {message}")]
    InvalidJson { message: String },
    #[error("timeout waiting for response")]
    Timeout,
    #[error("closed")]
    Closed,
    #[error("unsupported on this target: {message}")]
    Unsupported { message: String },
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BindingRuntimeEvent {
    pub kind: String,
    pub peer_id: Option<String>,
    pub payload_json: String,
    pub responder_id: Option<u64>,
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BindingProtocolResponse {
    pub peer_id: String,
    pub payload_json: String,
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BindingStreamRequest {
    pub peer_id: String,
    pub request_json: String,
    pub payload_kind: String,
    pub timeout_ms: u64,
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BindingStreamEntry {
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub payload_kind: String,
    pub payload: Vec<u8>,
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BindingSignalRequest {
    pub recipient_peer_id: String,
    pub from_peer_id: String,
    pub connection_id: String,
    pub kind: String,
    pub payload_json: String,
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct BindingSignalPoll {
    pub peer_id: String,
    pub since: u64,
    pub timeout_ms: u64,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    pub value: String,
}

#[cfg(feature = "message_node")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AukiMessageEvent {
    pub peer_id: String,
    pub envelope: Vec<u8>,
}

#[derive(uniffi::Object)]
pub struct PeerIdentity {
    inner: core::PeerIdentity,
}

#[uniffi::export]
impl PeerIdentity {
    #[uniffi::constructor]
    pub fn from_seed(seed: Vec<u8>) -> Result<Arc<Self>, NetworkError> {
        Ok(Arc::new(Self {
            inner: core::PeerIdentity::from_seed(&seed32(seed)?),
        }))
    }

    #[uniffi::constructor]
    pub fn from_wallet_seed(seed: Vec<u8>) -> Result<Arc<Self>, NetworkError> {
        let wallet = Wallet::from_seed(&seed32(seed)?);
        Ok(Arc::new(Self {
            inner: core::PeerIdentity::from_wallet(&wallet),
        }))
    }

    pub fn peer_id(&self) -> String {
        self.inner.peer_id().to_string()
    }

    pub fn public_key_protobuf(&self) -> Vec<u8> {
        self.inner.public_key().encode_protobuf()
    }
}

#[uniffi::export]
pub fn peer_derivation_label() -> String {
    core::PEER_DERIVATION_LABEL.to_string()
}

#[uniffi::export]
pub fn peer_id_from_wallet_seed(seed: Vec<u8>) -> Result<String, NetworkError> {
    let wallet = Wallet::from_seed(&seed32(seed)?);
    Ok(core::PeerIdentity::from_wallet(&wallet)
        .peer_id()
        .to_string())
}

#[uniffi::export]
pub fn networking_capabilities() -> Vec<Capability> {
    [
        core::Capability::MESSAGE_FORWARDING,
        core::Capability::BULK_DATA_CHANNEL,
        core::Capability::TURN,
        core::Capability::SFU,
    ]
    .into_iter()
    .map(|value| Capability {
        value: value.to_string(),
    })
    .collect()
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Object)]
pub struct AukiNetworkRuntime {
    inner: network_runtime::NetworkRuntime,
    runtime: tokio::runtime::Runtime,
    stream_state: Arc<BindingStreamState>,
    next_responder_id: Mutex<u64>,
    join_responders: Mutex<HashMap<u64, oneshot::Sender<JoinResponse>>>,
    info_responders: Mutex<HashMap<u64, oneshot::Sender<InfoResponse>>>,
    resources_responders: Mutex<HashMap<u64, oneshot::Sender<ResourcesResponse>>>,
    sensors_responders: Mutex<HashMap<u64, oneshot::Sender<SensorsResponse>>>,
    registry_responders: Mutex<HashMap<u64, oneshot::Sender<RegistryResponse>>>,
    _join_events: Mutex<mpsc::Receiver<network_runtime::JoinEvent>>,
    _liveness_events: Mutex<mpsc::Receiver<network_runtime::PeerLivenessEvent>>,
    _membership_events: Mutex<mpsc::Receiver<network_runtime::MembershipEvent>>,
    _info_events: Mutex<mpsc::Receiver<network_runtime::InfoRequestEvent>>,
    _resources_events: Mutex<mpsc::Receiver<network_runtime::ResourcesRequestEvent>>,
    _sensors_events: Mutex<mpsc::Receiver<network_runtime::SensorsRequestEvent>>,
    _registry_events: Mutex<mpsc::Receiver<network_runtime::RegistryRequestEvent>>,
    _diagnostic_events: Mutex<mpsc::Receiver<network_runtime::DiagnosticEvent>>,
}

#[cfg(feature = "swarm")]
#[derive(uniffi::Object)]
pub struct AukiStreamSubscription {
    manifest_json: String,
    entries: Mutex<std_mpsc::Receiver<BindingStreamRead>>,
    closed: Mutex<bool>,
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
#[derive(uniffi::Object)]
pub struct AukiDiscoveryClient {
    inner: DiscoveryClient,
    runtime: tokio::runtime::Runtime,
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
#[derive(uniffi::Object)]
pub struct AukiSignaledPeerCore {
    inner: Mutex<SignaledPeerCore>,
}

#[cfg(feature = "swarm")]
enum BindingStreamRead {
    Entry(BindingStreamEntry),
    End,
    Error(String),
}

#[cfg(feature = "swarm")]
struct BindingStreamState {
    next_id: Mutex<u64>,
    open_tx: std_mpsc::Sender<BindingRuntimeEvent>,
    open_rx: Mutex<std_mpsc::Receiver<BindingRuntimeEvent>>,
    pending: Mutex<HashMap<u64, std_mpsc::Sender<BindingStreamDecision>>>,
    active: Mutex<HashMap<u64, ActiveBindingStream>>,
}

#[cfg(feature = "swarm")]
enum BindingStreamDecision {
    AcceptCamera {
        manifest: StreamManifest,
        source: SourceStream<CameraFrame>,
    },
    AcceptDetection {
        manifest: StreamManifest,
        source: SourceStream<DetectionFrame>,
    },
    Decline {
        reason: DeclineReason,
    },
}

#[cfg(feature = "swarm")]
enum ActiveBindingStream {
    Camera(futures_mpsc::UnboundedSender<Result<StreamItem<CameraFrame>, String>>),
    Detection(futures_mpsc::UnboundedSender<Result<StreamItem<DetectionFrame>, String>>),
}

#[cfg(feature = "swarm")]
#[uniffi::export]
impl AukiNetworkRuntime {
    #[uniffi::constructor]
    pub fn spawn(config: BindingSwarmConfig) -> Result<Arc<Self>, BindingNetworkError> {
        let wallet = Wallet::from_seed(&seed32_binding(config.wallet_seed)?);
        let identity = core::PeerIdentity::from_wallet(&wallet);
        let listen_addresses = parse_binding_multiaddrs(config.listen_multiaddrs)?;
        let allowed_peers = parse_binding_allowed_peers(config.allowed_peers)?;
        let stream_state = Arc::new(BindingStreamState::new());
        let heartbeat_timestamps = binding_heartbeat_source(
            identity.peer_id().to_string(),
            config.heartbeat_clock_id,
            config.heartbeat_clock_hash_hex,
        );

        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|err| BindingNetworkError::Runtime {
                message: err.to_string(),
            })?;

        let swarm = {
            let _guard = runtime.enter();
            swarm::build_swarm(
                &identity,
                SwarmConfig {
                    listen_addresses,
                    agent_version: config.agent_version,
                    enable_relay_server: false,
                },
            )
            .map_err(|err| BindingNetworkError::Runtime {
                message: err.to_string(),
            })?
        };

        let (
            inner,
            join_events,
            liveness_events,
            membership_events,
            info_events,
            resources_events,
            sensors_events,
            registry_events,
            diagnostic_events,
        ) = {
            let _guard = runtime.enter();
            network_runtime::NetworkRuntime::spawn(
                swarm,
                allowed_peers,
                binding_stream_provider(stream_state.clone()),
                heartbeat_timestamps,
            )
            .map_err(|err| BindingNetworkError::Runtime {
                message: err.to_string(),
            })?
        };

        Ok(Arc::new(Self {
            runtime,
            inner,
            stream_state,
            next_responder_id: Mutex::new(1),
            join_responders: Mutex::new(HashMap::new()),
            info_responders: Mutex::new(HashMap::new()),
            resources_responders: Mutex::new(HashMap::new()),
            sensors_responders: Mutex::new(HashMap::new()),
            registry_responders: Mutex::new(HashMap::new()),
            _join_events: Mutex::new(join_events),
            _liveness_events: Mutex::new(liveness_events),
            _membership_events: Mutex::new(membership_events),
            _info_events: Mutex::new(info_events),
            _resources_events: Mutex::new(resources_events),
            _sensors_events: Mutex::new(sensors_events),
            _registry_events: Mutex::new(registry_events),
            _diagnostic_events: Mutex::new(diagnostic_events),
        }))
    }

    pub fn local_peer_id(&self) -> String {
        self.inner.local_peer_id().to_string()
    }

    pub fn listen_multiaddrs(&self) -> Vec<String> {
        self.inner
            .listen_addrs()
            .into_iter()
            .map(|addr| addr.to_string())
            .collect()
    }

    pub fn connected_peers(&self) -> Vec<String> {
        self.inner
            .connected_peers()
            .into_iter()
            .map(|peer| peer.to_string())
            .collect()
    }

    pub fn drain_runtime_events(&self, _max_events: u32) -> Vec<BindingRuntimeEvent> {
        Vec::new()
    }

    pub fn drain_membership_events(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        drain_receiver(&self._membership_events, max_events, |event| {
            BindingRuntimeEvent {
                kind: "membership".to_string(),
                peer_id: Some(event.peer.to_string()),
                payload_json: json_string(&event.update),
                responder_id: None,
            }
        })
    }

    pub fn drain_liveness_events(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        drain_receiver(
            &self._liveness_events,
            max_events,
            liveness_event_to_binding,
        )
    }

    pub fn drain_diagnostic_events(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        drain_receiver(&self._diagnostic_events, max_events, |event| {
            BindingRuntimeEvent {
                kind: "diagnostic".to_string(),
                peer_id: Some(event.peer.to_string()),
                payload_json: json_string(&event.message),
                responder_id: None,
            }
        })
    }

    pub fn drain_join_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        drain_receiver(&self._join_events, max_events, |event| {
            let responder_id = self.store_join_responder(event.ack);
            BindingRuntimeEvent {
                kind: "join_request".to_string(),
                peer_id: Some(event.peer.to_string()),
                payload_json: json_string(&event.request),
                responder_id: Some(responder_id),
            }
        })
    }

    pub fn drain_participant_info_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        drain_receiver(&self._info_events, max_events, |event| {
            let responder_id = self.store_info_responder(event.ack);
            BindingRuntimeEvent {
                kind: "participant_info_request".to_string(),
                peer_id: Some(event.peer.to_string()),
                payload_json: json_string(&event.request),
                responder_id: Some(responder_id),
            }
        })
    }

    pub fn drain_sensor_catalog_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        drain_receiver(&self._sensors_events, max_events, |event| {
            let responder_id = self.store_sensors_responder(event.ack);
            BindingRuntimeEvent {
                kind: "sensor_catalog_request".to_string(),
                peer_id: Some(event.peer.to_string()),
                payload_json: json_string(&event.request),
                responder_id: Some(responder_id),
            }
        })
    }

    pub fn drain_resource_catalog_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        drain_receiver(&self._resources_events, max_events, |event| {
            let responder_id = self.store_resources_responder(event.ack);
            BindingRuntimeEvent {
                kind: "resource_catalog_request".to_string(),
                peer_id: Some(event.peer.to_string()),
                payload_json: json_string(&event.request),
                responder_id: Some(responder_id),
            }
        })
    }

    pub fn drain_registry_entry_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        drain_receiver(&self._registry_events, max_events, |event| {
            let responder_id = self.store_registry_responder(event.ack);
            BindingRuntimeEvent {
                kind: "registry_entry_request".to_string(),
                peer_id: Some(event.peer.to_string()),
                payload_json: json_string(&event.request),
                responder_id: Some(responder_id),
            }
        })
    }

    pub fn drain_stream_open_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        self.stream_state.drain_open_requests(max_events)
    }

    pub fn set_allowed_peers(
        &self,
        peers: Vec<BindingAllowedPeer>,
    ) -> Result<BindingUpdateReport, BindingNetworkError> {
        let accepted = peers
            .iter()
            .map(|peer| peer.peer_id.clone())
            .collect::<Vec<_>>();
        let parsed = parse_binding_allowed_peers(peers)?;
        self.runtime
            .block_on(self.inner.set_allowed_peers(parsed))
            .map_err(binding_update_error)?;
        Ok(BindingUpdateReport {
            accepted,
            rejected_json: "[]".to_string(),
        })
    }

    pub fn set_heartbeat_targets(&self, peer_ids: Vec<String>) -> Result<(), BindingNetworkError> {
        let peers = peer_ids
            .into_iter()
            .map(|peer_id| parse_binding_peer_id(&peer_id))
            .collect::<Result<Vec<_>, _>>()?;
        self.runtime
            .block_on(self.inner.set_heartbeat_targets(peers))
            .map_err(binding_update_error)
    }

    pub fn broadcast_diagnostic_message_json(
        &self,
        message_json: String,
    ) -> Result<(), BindingNetworkError> {
        let message = parse_json::<DiagnosticMessage>(&message_json)?;
        let _guard = self.runtime.enter();
        self.inner
            .broadcast_diagnostic_message(message)
            .map_err(binding_runtime_error)
    }

    pub fn send_join_request_json(
        &self,
        peer_id: String,
        request_json: String,
        _timeout_ms: u64,
    ) -> Result<BindingProtocolResponse, BindingNetworkError> {
        let peer = parse_binding_peer_id(&peer_id)?;
        let request = parse_json::<JoinRequest>(&request_json)?;
        let response = self
            .runtime
            .block_on(self.inner.send_join_request(peer, request))
            .map_err(binding_runtime_error)?;
        Ok(BindingProtocolResponse {
            peer_id,
            payload_json: json_result_string(&response)?,
        })
    }

    pub fn request_participant_info_json(
        &self,
        peer_id: String,
        request_json: String,
        _timeout_ms: u64,
    ) -> Result<BindingProtocolResponse, BindingNetworkError> {
        let peer = parse_binding_peer_id(&peer_id)?;
        let _request = parse_json::<InfoRequest>(&request_json)?;
        let response = self
            .runtime
            .block_on(self.inner.request_participant_info(peer))
            .map_err(binding_runtime_error)?;
        Ok(BindingProtocolResponse {
            peer_id,
            payload_json: json_result_string(&response)?,
        })
    }

    pub fn request_sensor_catalog_json(
        &self,
        peer_id: String,
        request_json: String,
        _timeout_ms: u64,
    ) -> Result<BindingProtocolResponse, BindingNetworkError> {
        let peer = parse_binding_peer_id(&peer_id)?;
        let request = parse_json::<SensorsRequest>(&request_json)?;
        let response = self
            .runtime
            .block_on(self.inner.request_sensors_catalog_with(peer, request))
            .map_err(binding_runtime_error)?;
        Ok(BindingProtocolResponse {
            peer_id,
            payload_json: json_result_string(&response)?,
        })
    }

    pub fn request_resource_catalog_json(
        &self,
        peer_id: String,
        request_json: String,
        _timeout_ms: u64,
    ) -> Result<BindingProtocolResponse, BindingNetworkError> {
        let peer = parse_binding_peer_id(&peer_id)?;
        let request = parse_json::<ResourcesRequest>(&request_json)?;
        let response = self
            .runtime
            .block_on(self.inner.request_resources_catalog_with(peer, request))
            .map_err(binding_runtime_error)?;
        Ok(BindingProtocolResponse {
            peer_id,
            payload_json: json_result_string(&response)?,
        })
    }

    pub fn request_registry_entry_json(
        &self,
        peer_id: String,
        request_json: String,
        _timeout_ms: u64,
    ) -> Result<BindingProtocolResponse, BindingNetworkError> {
        let peer = parse_binding_peer_id(&peer_id)?;
        let request = parse_json::<RegistryRequest>(&request_json)?;
        let response = self
            .runtime
            .block_on(self.inner.request_registry_entry(peer, request))
            .map_err(binding_runtime_error)?;
        Ok(BindingProtocolResponse {
            peer_id,
            payload_json: json_result_string(&response)?,
        })
    }

    pub fn open_stream_bytes(
        &self,
        request: BindingStreamRequest,
    ) -> Result<Arc<AukiStreamSubscription>, BindingNetworkError> {
        let peer = parse_binding_peer_id(&request.peer_id)?;
        let stream_request = parse_stream_request_json(&request.request_json)?;
        let payload_kind = normalize_payload_kind(&request.payload_kind)?;
        match payload_kind.as_str() {
            "camera" => {
                let subscription = self
                    .runtime
                    .block_on(self.inner.open_stream::<CameraFrame>(peer, stream_request))
                    .map_err(binding_open_stream_error)?;
                Ok(AukiStreamSubscription::from_typed(
                    subscription,
                    payload_kind,
                    &self.runtime,
                ))
            }
            "detection" => {
                let subscription = self
                    .runtime
                    .block_on(
                        self.inner
                            .open_stream::<DetectionFrame>(peer, stream_request),
                    )
                    .map_err(binding_open_stream_error)?;
                Ok(AukiStreamSubscription::from_typed(
                    subscription,
                    payload_kind,
                    &self.runtime,
                ))
            }
            other => Err(BindingNetworkError::Unsupported {
                message: format!("unsupported stream payload kind: {other}"),
            }),
        }
    }

    pub fn accept_stream_open(
        &self,
        responder_id: u64,
        manifest_json: String,
    ) -> Result<u64, BindingNetworkError> {
        self.stream_state.accept_open(responder_id, &manifest_json)
    }

    pub fn decline_stream_open(
        &self,
        responder_id: u64,
        reason: String,
    ) -> Result<(), BindingNetworkError> {
        self.stream_state.decline_open(responder_id, &reason)
    }

    pub fn push_stream_entry(
        &self,
        stream_id: u64,
        entry: BindingStreamEntry,
    ) -> Result<(), BindingNetworkError> {
        self.stream_state.push_entry(stream_id, entry)
    }

    pub fn finish_stream(&self, stream_id: u64) -> Result<(), BindingNetworkError> {
        self.stream_state.finish_stream(stream_id)
    }

    pub fn respond_join_json(
        &self,
        responder_id: u64,
        response_json: String,
    ) -> Result<(), BindingNetworkError> {
        let response = parse_json::<JoinResponse>(&response_json)?;
        send_response(&self.join_responders, responder_id, response)
    }

    pub fn respond_participant_info_json(
        &self,
        responder_id: u64,
        response_json: String,
    ) -> Result<(), BindingNetworkError> {
        let response = parse_json::<InfoResponse>(&response_json)?;
        send_response(&self.info_responders, responder_id, response)
    }

    pub fn respond_sensor_catalog_json(
        &self,
        responder_id: u64,
        response_json: String,
    ) -> Result<(), BindingNetworkError> {
        let response = parse_json::<SensorsResponse>(&response_json)?;
        send_response(&self.sensors_responders, responder_id, response)
    }

    pub fn respond_resource_catalog_json(
        &self,
        responder_id: u64,
        response_json: String,
    ) -> Result<(), BindingNetworkError> {
        let response = parse_json::<ResourcesResponse>(&response_json)?;
        send_response(&self.resources_responders, responder_id, response)
    }

    pub fn respond_registry_entry_json(
        &self,
        responder_id: u64,
        response_json: String,
    ) -> Result<(), BindingNetworkError> {
        let response = parse_json::<RegistryResponse>(&response_json)?;
        send_response(&self.registry_responders, responder_id, response)
    }

    pub fn shutdown(&self) -> Result<(), BindingNetworkError> {
        self.inner.shutdown();
        Ok(())
    }
}

#[cfg(feature = "swarm")]
#[uniffi::export]
impl AukiStreamSubscription {
    pub fn manifest_json(&self) -> String {
        self.manifest_json.clone()
    }

    pub fn next_entry(
        &self,
        timeout_ms: u64,
    ) -> Result<Option<BindingStreamEntry>, BindingNetworkError> {
        if *self
            .closed
            .lock()
            .expect("stream subscription closed mutex poisoned")
        {
            return Ok(None);
        }

        let timeout = std::time::Duration::from_millis(timeout_ms);
        let next = self
            .entries
            .lock()
            .expect("stream subscription entries mutex poisoned")
            .recv_timeout(timeout)
            .map_err(|err| match err {
                std_mpsc::RecvTimeoutError::Timeout => BindingNetworkError::Timeout,
                std_mpsc::RecvTimeoutError::Disconnected => BindingNetworkError::Closed,
            })?;

        match next {
            BindingStreamRead::Entry(entry) => Ok(Some(entry)),
            BindingStreamRead::End => {
                *self
                    .closed
                    .lock()
                    .expect("stream subscription closed mutex poisoned") = true;
                Ok(None)
            }
            BindingStreamRead::Error(message) => Err(BindingNetworkError::Runtime { message }),
        }
    }

    pub fn close(&self) -> Result<(), BindingNetworkError> {
        *self
            .closed
            .lock()
            .expect("stream subscription closed mutex poisoned") = true;
        Ok(())
    }
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
#[uniffi::export]
pub fn discovery_client(base_url: String) -> Result<Arc<AukiDiscoveryClient>, BindingNetworkError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| BindingNetworkError::Runtime {
            message: err.to_string(),
        })?;
    Ok(Arc::new(AukiDiscoveryClient {
        inner: DiscoveryClient::new(base_url),
        runtime,
    }))
}

#[cfg(all(feature = "app_instance", feature = "swarm"))]
#[uniffi::export]
pub fn derive_app_instance_json(
    wallet_seed: Vec<u8>,
    app_id: String,
) -> Result<String, BindingNetworkError> {
    #[derive(serde::Serialize)]
    struct AppInstanceBinding {
        app_id: String,
        app_instance: String,
        peer_id: String,
        peer_derivation_label: &'static str,
    }

    let wallet = Wallet::from_seed(&seed32_binding(wallet_seed)?);
    let peer = core::PeerIdentity::from_wallet(&wallet);
    let app_instance = app_instance::derive().map_err(binding_runtime_error)?;
    json_result_string(&AppInstanceBinding {
        app_id,
        app_instance,
        peer_id: peer.peer_id().to_string(),
        peer_derivation_label: core::PEER_DERIVATION_LABEL,
    })
}

#[cfg(all(feature = "app_instance", feature = "swarm"))]
#[uniffi::export]
pub fn app_instance_peer_id(app_instance_json: String) -> Result<String, BindingNetworkError> {
    #[derive(serde::Deserialize)]
    struct AppInstanceBinding {
        peer_id: String,
    }

    let binding = parse_json::<AppInstanceBinding>(&app_instance_json)?;
    parse_binding_peer_id(&binding.peer_id)?;
    Ok(binding.peer_id)
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
#[uniffi::export]
impl AukiSignaledPeerCore {
    #[uniffi::constructor]
    pub fn new(
        local_peer_id: String,
        discovery_url: String,
    ) -> Result<Arc<Self>, BindingNetworkError> {
        let inner = SignaledPeerCore::new(local_peer_id, discovery_url).map_err(|err| {
            BindingNetworkError::Runtime {
                message: err.to_string(),
            }
        })?;
        Ok(Arc::new(Self {
            inner: Mutex::new(inner),
        }))
    }

    pub fn local_peer_id(&self) -> String {
        self.inner
            .lock()
            .expect("signaled peer mutex poisoned")
            .local_peer_id()
            .to_string()
    }

    pub fn signaled_multiaddr(&self) -> String {
        let inner = self.inner.lock().expect("signaled peer mutex poisoned");
        signaled_address::format_signaled_address(inner.discovery_url(), inner.local_peer_id())
            .expect("SignaledPeerCore constructor validated address inputs")
    }
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
#[uniffi::export]
impl AukiDiscoveryClient {
    pub fn register_peer_json(
        &self,
        registration_json: String,
        timeout_ms: u64,
    ) -> Result<String, BindingNetworkError> {
        #[derive(serde::Deserialize)]
        struct Registration {
            name: String,
            manager_peer_id: Option<String>,
            manager_multiaddrs: Option<Vec<String>>,
            relay_multiaddrs: Option<Vec<String>>,
            peer_count: Option<u32>,
            mode: Option<String>,
        }

        let registration = parse_json::<Registration>(&registration_json)?;
        let mode = registration.mode.as_deref().unwrap_or("create");
        match mode {
            "create" | "register" => {
                let peer_id = parse_binding_peer_id(required_option(
                    registration.manager_peer_id.as_deref(),
                    "manager_peer_id",
                )?)?;
                let addrs = parse_binding_multiaddrs(required_option(
                    registration.manager_multiaddrs,
                    "manager_multiaddrs",
                )?)?;
                let relays =
                    parse_binding_multiaddrs(registration.relay_multiaddrs.unwrap_or_default())?;
                let outcome = self.block_on_discovery(
                    timeout_ms,
                    self.inner.create_cluster_with_relays(
                        &registration.name,
                        &peer_id,
                        &addrs,
                        &relays,
                    ),
                )?;
                match outcome {
                    CreateClusterOutcome::Created(entry) => {
                        json_result_string(&serde_json::json!({
                            "kind": "created",
                            "entry": discovery_entry_json(&entry),
                        }))
                    }
                    CreateClusterOutcome::AlreadyExists => json_result_string(&serde_json::json!({
                        "kind": "already_exists",
                    })),
                }
            }
            "rotate" | "rotate_manager" => {
                let peer_id = parse_binding_peer_id(required_option(
                    registration.manager_peer_id.as_deref(),
                    "manager_peer_id",
                )?)?;
                let addrs = parse_binding_multiaddrs(required_option(
                    registration.manager_multiaddrs,
                    "manager_multiaddrs",
                )?)?;
                let relays =
                    parse_binding_multiaddrs(registration.relay_multiaddrs.unwrap_or_default())?;
                let entry = self.block_on_discovery(
                    timeout_ms,
                    self.inner.rotate_manager_with_relays(
                        &registration.name,
                        &peer_id,
                        &addrs,
                        &relays,
                    ),
                )?;
                json_result_string(&serde_json::json!({
                    "kind": "updated",
                    "entry": discovery_entry_json(&entry),
                }))
            }
            "liveness" => {
                let peer_count =
                    registration
                        .peer_count
                        .ok_or_else(|| BindingNetworkError::InvalidJson {
                            message: "missing numeric field `peer_count`".to_string(),
                        })?;
                let entry = self.block_on_discovery(
                    timeout_ms,
                    self.inner.liveness_check(&registration.name, peer_count),
                )?;
                json_result_string(&serde_json::json!({
                    "kind": "liveness",
                    "entry": discovery_entry_json(&entry),
                }))
            }
            other => Err(BindingNetworkError::Unsupported {
                message: format!("unsupported discovery registration mode: {other}"),
            }),
        }
    }

    pub fn discover_peers_json(
        &self,
        query_json: String,
        timeout_ms: u64,
    ) -> Result<String, BindingNetworkError> {
        #[derive(serde::Deserialize)]
        struct Query {
            name: Option<String>,
        }

        let query = parse_json::<Query>(&query_json)?;
        let mut entries = self.block_on_discovery(timeout_ms, self.inner.list_clusters())?;
        if let Some(name) = query.name {
            entries.retain(|entry| entry.name == name);
        }
        let clusters = entries.iter().map(discovery_entry_json).collect::<Vec<_>>();
        json_result_string(&serde_json::json!({ "clusters": clusters }))
    }

    pub fn discover_nodes_json(
        &self,
        query_json: String,
        timeout_ms: u64,
    ) -> Result<String, BindingNetworkError> {
        #[derive(serde::Deserialize)]
        struct Query {
            #[serde(rename = "type")]
            node_type: Option<String>,
        }

        let query = parse_json::<Query>(&query_json)?;
        let entries = if let Some(node_type) = query.node_type {
            self.block_on_discovery(timeout_ms, self.inner.list_nodes_by_type(&node_type))?
        } else {
            self.block_on_discovery(timeout_ms, self.inner.list_nodes())?
        };
        let nodes = entries.iter().map(discovery_node_json).collect::<Vec<_>>();
        json_result_string(&serde_json::json!({ "nodes": nodes }))
    }

    pub fn send_signal_json(
        &self,
        request: BindingSignalRequest,
        timeout_ms: u64,
    ) -> Result<String, BindingNetworkError> {
        let payload = parse_json::<serde_json::Value>(&request.payload_json)?;
        let message = self.block_on_discovery(
            timeout_ms,
            self.inner.send_signal(discovery_client::SignalRequest {
                recipient_peer_id: request.recipient_peer_id,
                from_peer_id: request.from_peer_id,
                connection_id: request.connection_id,
                kind: request.kind,
                payload,
            }),
        )?;
        json_result_string(&message)
    }

    pub fn poll_signals_json(
        &self,
        query: BindingSignalPoll,
        timeout_ms: u64,
    ) -> Result<String, BindingNetworkError> {
        let messages = self.block_on_discovery(
            timeout_ms,
            self.inner.poll_signals(discovery_client::SignalPoll {
                peer_id: query.peer_id,
                since: query.since,
                timeout_ms: query.timeout_ms,
            }),
        )?;
        json_result_string(&serde_json::json!({ "messages": messages }))
    }

    pub fn unregister_peer_json(
        &self,
        peer_id: String,
        timeout_ms: u64,
    ) -> Result<(), BindingNetworkError> {
        self.block_on_discovery(timeout_ms, self.inner.deregister(&peer_id))
    }
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
impl AukiDiscoveryClient {
    fn block_on_discovery<T>(
        &self,
        timeout_ms: u64,
        future: impl std::future::Future<Output = Result<T, discovery_client::DiscoveryError>>,
    ) -> Result<T, BindingNetworkError> {
        let duration = Duration::from_millis(timeout_ms.max(1));
        self.runtime
            .block_on(async move { tokio::time::timeout(duration, future).await })
            .map_err(|_| BindingNetworkError::Timeout)?
            .map_err(discovery_error)
    }
}

#[cfg(feature = "swarm")]
impl AukiNetworkRuntime {
    fn store_join_responder(&self, responder: oneshot::Sender<JoinResponse>) -> u64 {
        self.store_responder(&self.join_responders, responder)
    }

    fn store_info_responder(&self, responder: oneshot::Sender<InfoResponse>) -> u64 {
        self.store_responder(&self.info_responders, responder)
    }

    fn store_resources_responder(&self, responder: oneshot::Sender<ResourcesResponse>) -> u64 {
        self.store_responder(&self.resources_responders, responder)
    }

    fn store_sensors_responder(&self, responder: oneshot::Sender<SensorsResponse>) -> u64 {
        self.store_responder(&self.sensors_responders, responder)
    }

    fn store_registry_responder(&self, responder: oneshot::Sender<RegistryResponse>) -> u64 {
        self.store_responder(&self.registry_responders, responder)
    }

    fn store_responder<T>(
        &self,
        responders: &Mutex<HashMap<u64, oneshot::Sender<T>>>,
        responder: oneshot::Sender<T>,
    ) -> u64 {
        let mut next = self
            .next_responder_id
            .lock()
            .expect("responder id mutex poisoned");
        let id = *next;
        *next = next.saturating_add(1).max(1);
        responders
            .lock()
            .expect("responder registry mutex poisoned")
            .insert(id, responder);
        id
    }
}

#[cfg(feature = "swarm")]
impl BindingStreamState {
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

    fn drain_open_requests(&self, max_events: u32) -> Vec<BindingRuntimeEvent> {
        let rx = self
            .open_rx
            .lock()
            .expect("stream open receiver mutex poisoned");
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
    ) -> Result<u64, BindingNetworkError> {
        let manifest = parse_stream_manifest_json(manifest_json)?;
        let payload_kind = infer_payload_kind(&manifest.sensor_id)?;
        let responder = self
            .pending
            .lock()
            .expect("stream pending mutex poisoned")
            .remove(&responder_id)
            .ok_or(BindingNetworkError::Closed)?;
        let stream_id = self.next_id();

        let decision = match payload_kind.as_str() {
            "camera" => {
                let (tx, rx) = futures_mpsc::unbounded();
                self.active
                    .lock()
                    .expect("active stream mutex poisoned")
                    .insert(stream_id, ActiveBindingStream::Camera(tx));
                BindingStreamDecision::AcceptCamera {
                    manifest,
                    source: Box::pin(rx),
                }
            }
            "detection" => {
                let (tx, rx) = futures_mpsc::unbounded();
                self.active
                    .lock()
                    .expect("active stream mutex poisoned")
                    .insert(stream_id, ActiveBindingStream::Detection(tx));
                BindingStreamDecision::AcceptDetection {
                    manifest,
                    source: Box::pin(rx),
                }
            }
            other => {
                return Err(BindingNetworkError::Unsupported {
                    message: format!("unsupported stream payload kind: {other}"),
                });
            }
        };

        if responder.send(decision).is_err() {
            self.active
                .lock()
                .expect("active stream mutex poisoned")
                .remove(&stream_id);
            return Err(BindingNetworkError::Closed);
        }
        Ok(stream_id)
    }

    fn decline_open(&self, responder_id: u64, reason: &str) -> Result<(), BindingNetworkError> {
        let responder = self
            .pending
            .lock()
            .expect("stream pending mutex poisoned")
            .remove(&responder_id)
            .ok_or(BindingNetworkError::Closed)?;
        responder
            .send(BindingStreamDecision::Decline {
                reason: binding_decline_reason(reason),
            })
            .map_err(|_| BindingNetworkError::Closed)
    }

    fn push_entry(
        &self,
        stream_id: u64,
        entry: BindingStreamEntry,
    ) -> Result<(), BindingNetworkError> {
        let timestamp_ns =
            i64::try_from(entry.timestamp_ns).map_err(|_| BindingNetworkError::Runtime {
                message: "stream timestamp does not fit i64".to_string(),
            })?;
        let mut active = self.active.lock().expect("active stream mutex poisoned");
        let stream = active
            .get_mut(&stream_id)
            .ok_or(BindingNetworkError::Closed)?;
        match stream {
            ActiveBindingStream::Camera(tx) => {
                let payload = CameraFrame::decode(&*entry.payload).map_err(|err| {
                    BindingNetworkError::InvalidJson {
                        message: format!("camera payload decode: {err}"),
                    }
                })?;
                tx.unbounded_send(Ok(StreamItem {
                    timestamp_ns,
                    payload,
                }))
                .map_err(|_| BindingNetworkError::Closed)
            }
            ActiveBindingStream::Detection(tx) => {
                let payload = DetectionFrame::decode(&*entry.payload).map_err(|err| {
                    BindingNetworkError::InvalidJson {
                        message: format!("detection payload decode: {err}"),
                    }
                })?;
                tx.unbounded_send(Ok(StreamItem {
                    timestamp_ns,
                    payload,
                }))
                .map_err(|_| BindingNetworkError::Closed)
            }
        }
    }

    fn finish_stream(&self, stream_id: u64) -> Result<(), BindingNetworkError> {
        self.active
            .lock()
            .expect("active stream mutex poisoned")
            .remove(&stream_id)
            .ok_or(BindingNetworkError::Closed)?;
        Ok(())
    }
}

#[cfg(feature = "swarm")]
impl AukiStreamSubscription {
    fn from_typed<T>(
        subscription: TypedStreamSubscription<T>,
        payload_kind: String,
        runtime: &tokio::runtime::Runtime,
    ) -> Arc<Self>
    where
        T: Message + Default + Send + 'static,
    {
        let manifest_json = stream_manifest_json(&subscription.manifest);
        let (tx, rx) = std_mpsc::channel();
        runtime.spawn(binding_stream_reader(subscription, payload_kind, tx));
        Arc::new(Self {
            manifest_json,
            entries: Mutex::new(rx),
            closed: Mutex::new(false),
        })
    }
}

#[cfg(feature = "swarm")]
async fn binding_stream_reader<T>(
    mut subscription: TypedStreamSubscription<T>,
    payload_kind: String,
    tx: std_mpsc::Sender<BindingStreamRead>,
) where
    T: Message + Default + Send + 'static,
{
    while let Some(item) = subscription.entries.next().await {
        match item {
            Ok(entry) => {
                let timestamp_ns = match u64::try_from(entry.timestamp_ns) {
                    Ok(value) => value,
                    Err(_) => {
                        let _ = tx.send(BindingStreamRead::Error(
                            "stream timestamp was negative".to_string(),
                        ));
                        return;
                    }
                };
                let payload = entry.payload.encode_to_vec();
                if tx
                    .send(BindingStreamRead::Entry(BindingStreamEntry {
                        sequence: entry.seq,
                        timestamp_ns,
                        payload_kind: payload_kind.clone(),
                        payload,
                    }))
                    .is_err()
                {
                    return;
                }
            }
            Err(StreamError::EndOfStream { .. }) => {
                let _ = tx.send(BindingStreamRead::End);
                return;
            }
            Err(err) => {
                let _ = tx.send(BindingStreamRead::Error(err.to_string()));
                return;
            }
        }
    }
    let _ = tx.send(BindingStreamRead::End);
}

#[cfg(feature = "swarm")]
fn binding_stream_provider(
    state: Arc<BindingStreamState>,
) -> crate::stream_runtime::StreamProvider {
    Arc::new(move |peer, request| {
        let responder_id = state.next_id();
        let (decision_tx, decision_rx) = std_mpsc::channel();
        state
            .pending
            .lock()
            .expect("stream pending mutex poisoned")
            .insert(responder_id, decision_tx);
        let _ = state.open_tx.send(BindingRuntimeEvent {
            kind: "stream_open_request".to_string(),
            peer_id: Some(peer.to_string()),
            payload_json: stream_request_json(&request),
            responder_id: Some(responder_id),
        });

        match decision_rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(BindingStreamDecision::AcceptCamera { manifest, source }) => {
                StreamDispatch::AcceptCamera { manifest, source }
            }
            Ok(BindingStreamDecision::AcceptDetection { manifest, source }) => {
                StreamDispatch::AcceptDetection { manifest, source }
            }
            Ok(BindingStreamDecision::Decline { reason }) => StreamDispatch::Decline { reason },
            Err(_) => {
                state
                    .pending
                    .lock()
                    .expect("stream pending mutex poisoned")
                    .remove(&responder_id);
                StreamDispatch::Decline {
                    reason: DeclineReason::producer_shutting_down(),
                }
            }
        }
    })
}

#[cfg(feature = "message_node")]
#[derive(uniffi::Object)]
pub struct AukiMessageNode {
    inner: MessageNode,
}

#[cfg(feature = "message_node")]
#[uniffi::export]
impl AukiMessageNode {
    #[uniffi::constructor]
    pub fn from_wallet_seed(
        seed: Vec<u8>,
        listen_addrs: Vec<String>,
        agent_version: String,
    ) -> Result<Arc<Self>, NetworkError> {
        let wallet = Wallet::from_seed(&seed32(seed)?);
        let identity = core::PeerIdentity::from_wallet(&wallet);
        let inner = MessageNode::spawn(
            identity,
            MessageNodeConfig {
                listen_addresses: parse_multiaddrs(listen_addrs)?,
                agent_version,
            },
        )
        .map_err(network_error)?;
        Ok(Arc::new(Self { inner }))
    }

    pub fn peer_id(&self) -> String {
        self.inner.local_peer_id().to_string()
    }

    pub fn listen_addrs(&self) -> Vec<String> {
        self.inner
            .listen_addrs()
            .into_iter()
            .map(|addr| addr.to_string())
            .collect()
    }

    pub fn dial(&self, peer_id: String, addrs: Vec<String>) -> Result<(), NetworkError> {
        self.inner
            .dial(parse_peer_id(&peer_id)?, parse_multiaddrs(addrs)?)
            .map_err(network_error)
    }

    pub fn send_message_envelope_bytes(
        &self,
        peer_id: String,
        envelope: Vec<u8>,
    ) -> Result<Vec<u8>, NetworkError> {
        let ack = self
            .inner
            .send_envelope_bytes(parse_peer_id(&peer_id)?, envelope)
            .map_err(network_error)?;
        Ok(ack.encode_to_vec())
    }

    pub fn next_event(&self) -> Result<Option<AukiMessageEvent>, NetworkError> {
        let Some(event) = self.inner.next_event().map_err(network_error)? else {
            return Ok(None);
        };
        Ok(Some(AukiMessageEvent {
            peer_id: event.peer_id.to_string(),
            envelope: event.envelope.encode_to_vec(),
        }))
    }

    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}

fn seed32(seed: Vec<u8>) -> Result<[u8; 32], NetworkError> {
    let len = seed.len();
    seed.try_into()
        .map_err(|_| NetworkError::InvalidSeedLength { len: len as u64 })
}

#[cfg(feature = "swarm")]
fn seed32_binding(seed: Vec<u8>) -> Result<[u8; 32], BindingNetworkError> {
    let len = seed.len();
    seed.try_into()
        .map_err(|_| BindingNetworkError::InvalidSeedLength { len: len as u64 })
}

#[cfg(any(feature = "message_node", feature = "swarm"))]
fn parse_peer_id(value: &str) -> Result<Libp2pPeerId, NetworkError> {
    value.parse().map_err(|_| NetworkError::InvalidPeerId {
        value: value.to_string(),
    })
}

#[cfg(any(feature = "message_node", feature = "swarm"))]
fn parse_multiaddrs(values: Vec<String>) -> Result<Vec<Multiaddr>, NetworkError> {
    values
        .into_iter()
        .map(|value| {
            value
                .parse()
                .map_err(|_| NetworkError::InvalidMultiaddr { value })
        })
        .collect()
}

#[cfg(feature = "swarm")]
fn parse_binding_peer_id(value: &str) -> Result<Libp2pPeerId, BindingNetworkError> {
    value
        .parse()
        .map_err(|_| BindingNetworkError::InvalidPeerId {
            message: value.to_string(),
        })
}

#[cfg(feature = "swarm")]
fn parse_binding_multiaddrs(values: Vec<String>) -> Result<Vec<Multiaddr>, BindingNetworkError> {
    values
        .into_iter()
        .map(|value| {
            value
                .parse()
                .map_err(|_| BindingNetworkError::InvalidMultiaddr { message: value })
        })
        .collect()
}

#[cfg(feature = "swarm")]
fn parse_binding_allowed_peers(
    peers: Vec<BindingAllowedPeer>,
) -> Result<Vec<network_runtime::AllowedPeer>, BindingNetworkError> {
    peers
        .into_iter()
        .map(|peer| {
            Ok(network_runtime::AllowedPeer {
                peer_id: parse_binding_peer_id(&peer.peer_id)?,
                multiaddrs: parse_binding_multiaddrs(peer.multiaddrs)?,
            })
        })
        .collect()
}

#[cfg(feature = "swarm")]
fn drain_receiver<T>(
    receiver: &Mutex<mpsc::Receiver<T>>,
    max_events: u32,
    mut map: impl FnMut(T) -> BindingRuntimeEvent,
) -> Vec<BindingRuntimeEvent> {
    let mut receiver = receiver
        .lock()
        .expect("binding event receiver mutex poisoned");
    let mut events = Vec::new();
    for _ in 0..max_events {
        match receiver.try_recv() {
            Ok(event) => events.push(map(event)),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                break;
            }
        }
    }
    events
}

#[cfg(feature = "swarm")]
fn liveness_event_to_binding(event: network_runtime::PeerLivenessEvent) -> BindingRuntimeEvent {
    match event {
        network_runtime::PeerLivenessEvent::Connected { peer_id } => BindingRuntimeEvent {
            kind: "connected".to_string(),
            peer_id: Some(peer_id.to_string()),
            payload_json: "{}".to_string(),
            responder_id: None,
        },
        network_runtime::PeerLivenessEvent::Disconnected { peer_id } => BindingRuntimeEvent {
            kind: "disconnected".to_string(),
            peer_id: Some(peer_id.to_string()),
            payload_json: "{}".to_string(),
            responder_id: None,
        },
        network_runtime::PeerLivenessEvent::HeartbeatReceived {
            peer_id,
            observation,
        } => BindingRuntimeEvent {
            kind: "heartbeat_received".to_string(),
            peer_id: Some(peer_id.to_string()),
            payload_json: serde_json::json!({
                "heartbeat": observation.heartbeat,
                "received_at_clock_ns": observation.received_at_clock_ns,
                "local_clock_id": observation.local_clock_id,
                "local_clock_hash": observation.local_clock_hash,
            })
            .to_string(),
            responder_id: None,
        },
        network_runtime::PeerLivenessEvent::HeartbeatNtpSampleObserved {
            peer_id,
            observation,
        } => BindingRuntimeEvent {
            kind: "heartbeat_ntp_sample_observed".to_string(),
            peer_id: Some(peer_id.to_string()),
            payload_json: serde_json::json!({
                "local_clock_id": observation.local_clock_id,
                "local_clock_hash": observation.local_clock_hash,
                "remote_clock_id": observation.remote_clock_id,
                "remote_clock_hash": observation.remote_clock_hash,
                "offset_ns": observation.sample.offset_ns,
                "uncertainty_ns": observation.sample.uncertainty_ns,
                "round_trip_ns": observation.sample.round_trip_ns,
                "remote_processing_ns": observation.sample.remote_processing_ns,
                "observed_at_clock_ns": observation.sample.observed_at_clock_ns,
            })
            .to_string(),
            responder_id: None,
        },
        network_runtime::PeerLivenessEvent::HeartbeatStreamClosed { peer_id } => {
            BindingRuntimeEvent {
                kind: "heartbeat_stream_closed".to_string(),
                peer_id: Some(peer_id.to_string()),
                payload_json: "{}".to_string(),
                responder_id: None,
            }
        }
    }
}

#[cfg(feature = "swarm")]
fn parse_json<T: for<'de> serde::Deserialize<'de>>(json: &str) -> Result<T, BindingNetworkError> {
    serde_json::from_str(json).map_err(|err| BindingNetworkError::InvalidJson {
        message: err.to_string(),
    })
}

#[cfg(feature = "swarm")]
fn parse_stream_request_json(json: &str) -> Result<StreamRequest, BindingNetworkError> {
    let value = serde_json::from_str::<serde_json::Value>(json).map_err(|err| {
        BindingNetworkError::InvalidJson {
            message: err.to_string(),
        }
    })?;
    let sensor_id = required_json_string(&value, "sensor_id")?;
    Ok(StreamRequest { sensor_id })
}

#[cfg(feature = "swarm")]
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

#[cfg(feature = "swarm")]
fn parse_stream_manifest_json(json: &str) -> Result<StreamManifest, BindingNetworkError> {
    let value = serde_json::from_str::<serde_json::Value>(json).map_err(|err| {
        BindingNetworkError::InvalidJson {
            message: err.to_string(),
        }
    })?;
    Ok(StreamManifest {
        sensor_id: required_json_string(&value, "sensor_id")?,
        sensor_hash: required_json_string(&value, "sensor_hash")?,
        clock_id: required_json_string(&value, "clock_id")?,
        clock_hash: required_json_string(&value, "clock_hash")?,
        frame_id: required_json_string(&value, "frame_id")?,
        frame_hash: required_json_string(&value, "frame_hash")?,
    })
}

#[cfg(feature = "swarm")]
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

#[cfg(feature = "swarm")]
fn required_json_string(
    value: &serde_json::Value,
    field: &'static str,
) -> Result<String, BindingNetworkError> {
    value
        .get(field)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| BindingNetworkError::InvalidJson {
            message: format!("missing string field `{field}`"),
        })
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn required_option<T>(value: Option<T>, field: &'static str) -> Result<T, BindingNetworkError> {
    value.ok_or_else(|| BindingNetworkError::InvalidJson {
        message: format!("missing field `{field}`"),
    })
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn discovery_entry_json(entry: &discovery_client::ClusterEntry) -> serde_json::Value {
    serde_json::json!({
        "name": entry.name,
        "manager_peer_id": entry.manager_peer_id.to_string(),
        "manager_multiaddrs": entry
            .manager_multiaddrs
            .iter()
            .map(|addr| addr.to_string())
            .collect::<Vec<_>>(),
        "relay_multiaddrs": entry
            .relay_multiaddrs
            .iter()
            .map(|addr| addr.to_string())
            .collect::<Vec<_>>(),
        "peer_count": entry.peer_count,
        "created_ns": entry.created_ns,
        "last_liveness_check_ns": entry.last_liveness_check_ns,
    })
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn discovery_node_json(entry: &discovery_client::NodeEntry) -> serde_json::Value {
    serde_json::json!({
        "peer_id": entry.peer_id.to_string(),
        "node_type": entry.node_type,
        "multiaddrs": entry
            .multiaddrs
            .iter()
            .map(|addr| addr.to_string())
            .collect::<Vec<_>>(),
        "created_ns": entry.created_ns,
        "last_liveness_check_ns": entry.last_liveness_check_ns,
    })
}

#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn discovery_error(error: discovery_client::DiscoveryError) -> BindingNetworkError {
    match error {
        discovery_client::DiscoveryError::InvalidPeerId(message) => {
            BindingNetworkError::InvalidPeerId { message }
        }
        discovery_client::DiscoveryError::InvalidMultiaddr(message) => {
            BindingNetworkError::InvalidMultiaddr { message }
        }
        other => BindingNetworkError::Runtime {
            message: other.to_string(),
        },
    }
}

#[cfg(feature = "swarm")]
fn normalize_payload_kind(kind: &str) -> Result<String, BindingNetworkError> {
    match kind {
        "camera" | "detection" => Ok(kind.to_string()),
        other => Err(BindingNetworkError::Unsupported {
            message: format!("unsupported stream payload kind: {other}"),
        }),
    }
}

#[cfg(feature = "swarm")]
fn infer_payload_kind(sensor_id: &str) -> Result<String, BindingNetworkError> {
    if sensor_id.contains("detect") || sensor_id.contains("detector") {
        Ok("detection".to_string())
    } else {
        Ok("camera".to_string())
    }
}

#[cfg(feature = "swarm")]
fn binding_decline_reason(reason: &str) -> DeclineReason {
    match reason {
        "sensor_not_found" => DeclineReason::sensor_not_found(),
        "sensor_unavailable" => DeclineReason::sensor_unavailable(),
        "producer_shutting_down" => DeclineReason::producer_shutting_down(),
        other => DeclineReason::other(other),
    }
}

#[cfg(feature = "swarm")]
fn json_result_string<T: serde::Serialize>(value: &T) -> Result<String, BindingNetworkError> {
    serde_json::to_string(value).map_err(|err| BindingNetworkError::InvalidJson {
        message: err.to_string(),
    })
}

#[cfg(feature = "swarm")]
fn json_string<T: serde::Serialize>(value: &T) -> String {
    json_result_string(value).unwrap_or_else(|err| {
        serde_json::json!({
            "code": "serialization_failed",
            "message": err.to_string(),
        })
        .to_string()
    })
}

#[cfg(feature = "swarm")]
fn send_response<T>(
    responders: &Mutex<HashMap<u64, oneshot::Sender<T>>>,
    responder_id: u64,
    response: T,
) -> Result<(), BindingNetworkError> {
    let responder = responders
        .lock()
        .expect("responder registry mutex poisoned")
        .remove(&responder_id)
        .ok_or(BindingNetworkError::Closed)?;
    responder
        .send(response)
        .map_err(|_| BindingNetworkError::Closed)
}

#[cfg(feature = "swarm")]
fn binding_heartbeat_source(
    local_peer_id: String,
    heartbeat_clock_id: Option<String>,
    heartbeat_clock_hash: Option<String>,
) -> HeartbeatTimestampSource {
    let clock_id =
        heartbeat_clock_id.unwrap_or_else(|| format!("{local_peer_id}/binding-heartbeat-clock"));
    let clock_hash = heartbeat_clock_hash.unwrap_or_else(|| "binding-heartbeat-clock".to_string());
    HeartbeatTimestampSource {
        clock_id,
        clock_hash,
        now_ns: Arc::new(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
                .unwrap_or(0)
        }),
        domain_clock: Arc::new(|| None),
    }
}

#[cfg(feature = "swarm")]
fn binding_update_error(error: network_runtime::UpdateError) -> BindingNetworkError {
    BindingNetworkError::Runtime {
        message: error.to_string(),
    }
}

#[cfg(feature = "swarm")]
fn binding_runtime_error(error: impl std::fmt::Display) -> BindingNetworkError {
    BindingNetworkError::Runtime {
        message: error.to_string(),
    }
}

#[cfg(feature = "swarm")]
fn binding_open_stream_error(error: OpenStreamError) -> BindingNetworkError {
    match error {
        OpenStreamError::Timeout(_) => BindingNetworkError::Timeout,
        other => BindingNetworkError::Runtime {
            message: other.to_string(),
        },
    }
}

#[cfg(feature = "message_node")]
fn network_error(error: crate::message_node::MessageNodeError) -> NetworkError {
    NetworkError::MessageNode {
        message: error.to_string(),
    }
}
