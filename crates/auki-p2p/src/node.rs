//! Minimal libp2p node wrapper for the RFC-first runtime.

use crate::{
    AukiConnectionPath, AukiP2pConfig, ConfigError, ConfiguredPeer, DialPolicyError,
    LocalPeerIdentity,
};
use auki_protocol::v1::{
    base64url,
    status::{LocalPeerStatus, StatusError},
};
use futures::{StreamExt as _, future};
use libp2p::core::{Transport as _, muxing::StreamMuxerBox, transport::Boxed};
use libp2p::{
    Multiaddr, PeerId, Swarm,
    core::{ConnectedPoint, upgrade},
    identify, noise, ping, quic, relay,
    swarm::{
        Config as SwarmConfig, ConnectionId, DialError, NetworkBehaviour, SwarmEvent,
        behaviour::toggle::Toggle, dial_opts::DialOpts,
    },
    tcp, yamux,
};
#[cfg(feature = "browser-webrtc-direct")]
use libp2p_webrtc as webrtc;
use multiaddr::Protocol;
#[cfg(feature = "browser-webrtc-direct")]
use rand::thread_rng;
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, VecDeque},
    fmt,
    time::Duration,
};

/// Identify protocol id used by the new RFC-first runtime.
pub const IDENTIFY_PROTOCOL_ID: &str = "/auki/p2p/identify/0.0.1";

const IDLE_CONNECTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Libp2p behaviour set for the clean runtime skeleton.
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    /// Identify is transport metadata only. Protocol authority remains in
    /// the lifecycle handshake.
    pub identify: identify::Behaviour,
    /// Connection liveness at the libp2p layer.
    pub ping: ping::Behaviour,
    /// Optional Circuit Relay v2 server role. Relay is connectivity only;
    /// protocol authority remains in lifecycle and policy validation.
    pub relay: Toggle<relay::Behaviour>,
    /// Raw substream multiplexer used by lifecycle, offer catalog, Get, and Subscribe.
    pub stream: libp2p_stream::Behaviour,
}

/// Node-level runtime options.
#[derive(Debug, Clone, PartialEq)]
pub struct AukiP2pNodeConfig {
    /// RFC-shaped policy and limit configuration.
    pub p2p: AukiP2pConfig,
    /// Multiaddrs to bind locally during construction. These are not
    /// automatically advertised to remote peers.
    pub listen_addresses: Vec<Multiaddr>,
    /// Operator-supplied dialable addresses to advertise to intended peers.
    pub advertised_addresses: Vec<Multiaddr>,
    /// Relay-mediated connectivity addresses. These are operational hints, not authority.
    pub relay_addresses: Vec<Multiaddr>,
    /// Browser-to-node WebRTC Direct transport support.
    pub browser_webrtc_direct: BrowserWebRtcDirectConfig,
    /// Local Circuit Relay v2 server role.
    pub relay_server: RelayServerConfig,
    /// Identify agent version advertised to remote libp2p peers.
    pub agent_version: String,
}

/// Browser-to-native WebRTC Direct transport configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserWebRtcDirectConfig {
    /// Enable WebRTC Direct transport support for browser-to-node dialing.
    pub enabled: bool,
}

/// Local Circuit Relay v2 server configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayServerConfig {
    /// Enable the local node as a relay server.
    pub enabled: bool,
}

/// Connectivity-only bootstrap record for browser peers.
///
/// This record is an address snapshot only. It does not convey domain
/// authority, lifecycle state, offers, or policy grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AukiBrowserBootstrapRecord {
    /// Native peer id browsers are expected to dial or use as a bootstrap peer.
    pub peer_id: PeerId,
    /// Identify agent version of the native peer.
    pub agent_version: String,
    /// Browser-compatible direct addresses for this peer.
    pub direct_addresses: Vec<Multiaddr>,
    /// Direct WebRTC addresses for browser-to-node dialing.
    pub webrtc_direct_addresses: Vec<Multiaddr>,
    /// Relay-mediated addresses for this peer, when configured by the operator.
    pub relay_addresses: Vec<Multiaddr>,
    /// Browser-compatible relay server addresses for this peer.
    pub relay_server_addresses: Vec<Multiaddr>,
    /// Unique union of direct, relay-mediated, and relay-server bootstrap addresses.
    pub bootstrap_addresses: Vec<Multiaddr>,
}

/// Public events surfaced by the node skeleton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AukiP2pEvent {
    /// A local listener was bound.
    Listening {
        /// Bound listen address.
        address: Multiaddr,
    },
    /// A libp2p connection was established.
    ConnectionEstablished {
        /// Transport-authenticated remote peer id.
        peer_id: PeerId,
    },
    /// A connection exceeded the local per-peer cap and was scheduled for close.
    DuplicateConnectionClosed {
        /// Transport-authenticated remote peer id.
        peer_id: PeerId,
    },
    /// A libp2p connection closed.
    ConnectionClosed {
        /// Transport-authenticated remote peer id.
        peer_id: PeerId,
    },
    /// An outbound dial failed.
    OutgoingConnectionError {
        /// Remote peer id, when libp2p associated the failure with one.
        peer_id: Option<PeerId>,
        /// Human-readable libp2p dial error.
        error: String,
    },
    /// An inbound connection failed before it was established.
    IncomingConnectionError {
        /// Human-readable libp2p inbound error.
        error: String,
    },
}

/// Errors produced by node construction and commands.
#[derive(Debug)]
pub enum AukiP2pNodeError {
    /// P2P config is invalid.
    Config(ConfigError),
    /// Status projection failed.
    Status(StatusError),
    /// Transport stack failed to assemble.
    Transport {
        /// Transport stage that failed.
        stage: &'static str,
        /// Human-readable source error.
        source: String,
    },
    /// Listening failed for one configured or requested address.
    Listen {
        /// Address that failed.
        address: Multiaddr,
        /// Human-readable source error.
        source: String,
    },
    /// Dial policy rejected the address before libp2p saw it.
    DialPolicy(DialPolicyError),
    /// Libp2p rejected the dial request.
    Dial {
        /// Expected remote peer id.
        peer_id: PeerId,
        /// Libp2p dial error.
        source: Box<DialError>,
    },
    /// A WebRTC Direct listen or dial address was used while the node config disabled it.
    BrowserWebRtcDirectDisabled {
        /// Address that required WebRTC Direct support.
        address: Multiaddr,
    },
    /// The node config enabled WebRTC Direct, but the crate feature is not compiled in.
    BrowserWebRtcDirectFeatureDisabled,
}

/// Small, directly pollable libp2p node.
pub struct AukiP2pNode {
    identity: LocalPeerIdentity,
    config: AukiP2pNodeConfig,
    observed_listen_addresses: Vec<Multiaddr>,
    pending_events: VecDeque<AukiP2pEvent>,
    connections: ConnectionTracker,
    swarm: Swarm<Behaviour>,
}

/// Local per-peer connection retention state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConnectionTracker {
    established: HashMap<PeerId, Vec<TrackedConnection>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackedConnection {
    connection_id: ConnectionId,
    preference: ConnectionPreference,
    path: AukiConnectionPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionPreference {
    Preferred,
    Fallback,
}

/// Decision for a newly established connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionRetention {
    /// Keep the connection.
    Retained,
    /// Close a connection because the per-peer cap has already been reached.
    CloseDuplicate {
        /// Connection scheduled for close.
        connection_id: ConnectionId,
    },
}

impl Default for AukiP2pNodeConfig {
    fn default() -> Self {
        Self::dial_only_development()
    }
}

impl AukiP2pNodeConfig {
    /// Development config that does not bind any listener.
    pub fn dial_only_development() -> Self {
        Self {
            p2p: AukiP2pConfig::development(),
            listen_addresses: Vec::new(),
            advertised_addresses: Vec::new(),
            relay_addresses: Vec::new(),
            browser_webrtc_direct: BrowserWebRtcDirectConfig::disabled(),
            relay_server: RelayServerConfig::disabled(),
            agent_version: default_agent_version(),
        }
    }

    /// Development config that binds an OS-selected loopback TCP port.
    pub fn loopback_tcp_development() -> Self {
        Self {
            p2p: AukiP2pConfig::development(),
            listen_addresses: vec![
                "/ip4/127.0.0.1/tcp/0"
                    .parse()
                    .expect("static loopback listen multiaddr is valid"),
            ],
            advertised_addresses: Vec::new(),
            relay_addresses: Vec::new(),
            browser_webrtc_direct: BrowserWebRtcDirectConfig::disabled(),
            relay_server: RelayServerConfig::disabled(),
            agent_version: default_agent_version(),
        }
    }

    /// Development config that binds an OS-selected loopback WebRTC Direct UDP port.
    pub fn loopback_webrtc_direct_development() -> Self {
        Self {
            p2p: AukiP2pConfig::development(),
            listen_addresses: vec![loopback_webrtc_direct_listen_addr()],
            advertised_addresses: Vec::new(),
            relay_addresses: Vec::new(),
            browser_webrtc_direct: BrowserWebRtcDirectConfig::enabled(),
            relay_server: RelayServerConfig::disabled(),
            agent_version: default_agent_version(),
        }
    }

    /// Development config that binds an OS-selected loopback WebSocket relay port.
    pub fn loopback_relay_server_development() -> Self {
        Self {
            p2p: AukiP2pConfig::development(),
            listen_addresses: vec![loopback_websocket_relay_listen_addr()],
            advertised_addresses: Vec::new(),
            relay_addresses: Vec::new(),
            browser_webrtc_direct: BrowserWebRtcDirectConfig::disabled(),
            relay_server: RelayServerConfig::enabled(),
            agent_version: default_agent_version(),
        }
    }

    /// Development config that exposes a native peer to browsers over WebRTC
    /// Direct and also runs a loopback WebSocket relay server.
    pub fn loopback_browser_reachable_development() -> Self {
        Self {
            p2p: AukiP2pConfig::development(),
            listen_addresses: vec![
                loopback_webrtc_direct_listen_addr(),
                loopback_websocket_relay_listen_addr(),
            ],
            advertised_addresses: Vec::new(),
            relay_addresses: Vec::new(),
            browser_webrtc_direct: BrowserWebRtcDirectConfig::enabled(),
            relay_server: RelayServerConfig::enabled(),
            agent_version: default_agent_version(),
        }
    }
}

impl BrowserWebRtcDirectConfig {
    /// Disable browser-to-node WebRTC Direct transport support.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Enable browser-to-node WebRTC Direct transport support.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }
}

impl RelayServerConfig {
    /// Disable local Circuit Relay v2 server support.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Enable local Circuit Relay v2 server support.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }
}

impl AukiBrowserBootstrapRecord {
    /// Convert the bootstrap record to a JSON value suitable for a local demo
    /// endpoint or browser package boundary.
    pub fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert(
            "peer_id".to_owned(),
            Value::String(self.peer_id.to_string()),
        );
        object.insert(
            "agent_version".to_owned(),
            Value::String(self.agent_version.clone()),
        );
        object.insert(
            "direct_addresses".to_owned(),
            multiaddr_array(&self.direct_addresses),
        );
        object.insert(
            "webrtc_direct_addresses".to_owned(),
            multiaddr_array(&self.webrtc_direct_addresses),
        );
        object.insert(
            "relay_addresses".to_owned(),
            multiaddr_array(&self.relay_addresses),
        );
        object.insert(
            "relay_server_addresses".to_owned(),
            multiaddr_array(&self.relay_server_addresses),
        );
        object.insert(
            "bootstrap_addresses".to_owned(),
            multiaddr_array(&self.bootstrap_addresses),
        );
        Value::Object(object)
    }
}

impl AukiP2pNode {
    /// Build a node and bind every configured listen address.
    pub fn new(
        identity: LocalPeerIdentity,
        config: AukiP2pNodeConfig,
    ) -> Result<Self, AukiP2pNodeError> {
        config.p2p.validate().map_err(AukiP2pNodeError::Config)?;
        validate_browser_webrtc_direct_config(&config)?;

        let local_peer_id = identity.peer_id();
        let transport = build_transport(identity.keypair(), &config)?;
        let behaviour = Behaviour {
            identify: identify::Behaviour::new(
                identify::Config::new(IDENTIFY_PROTOCOL_ID.into(), identity.public_key())
                    .with_agent_version(config.agent_version.clone()),
            ),
            ping: ping::Behaviour::default(),
            relay: Toggle::from(
                config
                    .relay_server
                    .enabled
                    .then(|| relay::Behaviour::new(local_peer_id, relay::Config::default())),
            ),
            stream: libp2p_stream::Behaviour::new(),
        };
        let mut swarm = Swarm::new(
            transport,
            behaviour,
            local_peer_id,
            SwarmConfig::with_tokio_executor()
                .with_idle_connection_timeout(IDLE_CONNECTION_TIMEOUT),
        );

        debug_assert_eq!(*swarm.local_peer_id(), local_peer_id);

        for address in &config.listen_addresses {
            swarm
                .listen_on(address.clone())
                .map_err(|source| AukiP2pNodeError::Listen {
                    address: address.clone(),
                    source: source.to_string(),
                })?;
        }

        Ok(Self {
            identity,
            config,
            observed_listen_addresses: Vec::new(),
            pending_events: VecDeque::new(),
            connections: ConnectionTracker::default(),
            swarm,
        })
    }

    /// Return the local libp2p peer id.
    pub fn peer_id(&self) -> PeerId {
        self.identity.peer_id()
    }

    /// Borrow the local identity and current peer binding.
    pub fn identity(&self) -> &LocalPeerIdentity {
        &self.identity
    }

    /// Borrow the node config.
    pub fn config(&self) -> &AukiP2pNodeConfig {
        &self.config
    }

    /// Configured local bind addresses.
    pub fn configured_listen_addresses(&self) -> &[Multiaddr] {
        &self.config.listen_addresses
    }

    /// Listen addresses observed from libp2p after binding.
    pub fn observed_listen_addresses(&self) -> &[Multiaddr] {
        &self.observed_listen_addresses
    }

    /// Observed listen addresses with the local `/p2p/<peer-id>` suffix.
    pub fn observed_dialable_listen_addresses(&self) -> Vec<Multiaddr> {
        self.observed_listen_addresses
            .iter()
            .cloned()
            .map(|address| address_with_peer_id(address, self.peer_id()))
            .collect()
    }

    /// Observed relay-server addresses with the local `/p2p/<peer-id>` suffix.
    pub fn observed_relay_server_addresses(&self) -> Vec<Multiaddr> {
        if !self.config.relay_server.enabled {
            return Vec::new();
        }
        self.observed_dialable_listen_addresses()
    }

    /// Observed WebSocket relay-server addresses usable by browser peers.
    pub fn observed_browser_relay_server_addresses(&self) -> Vec<Multiaddr> {
        self.observed_relay_server_addresses()
            .into_iter()
            .filter(is_websocket_address)
            .collect()
    }

    /// Operator-supplied advertised addresses.
    pub fn advertised_addresses(&self) -> &[Multiaddr] {
        &self.config.advertised_addresses
    }

    /// Relay-mediated connectivity addresses.
    pub fn relay_addresses(&self) -> &[Multiaddr] {
        &self.config.relay_addresses
    }

    /// Build a connectivity-only browser bootstrap record.
    pub fn browser_bootstrap_record(&self) -> AukiBrowserBootstrapRecord {
        let peer_id = self.peer_id();
        let direct_addresses = self.browser_direct_addresses();
        let webrtc_direct_addresses = direct_addresses
            .iter()
            .filter(|address| is_webrtc_direct_address(address))
            .cloned()
            .collect();
        let relay_addresses = self
            .config
            .relay_addresses
            .iter()
            .cloned()
            .map(|address| address_with_peer_id(address, peer_id))
            .collect::<Vec<_>>();
        let relay_server_addresses = self.observed_browser_relay_server_addresses();
        let mut bootstrap_addresses = Vec::new();
        for address in direct_addresses
            .iter()
            .chain(relay_addresses.iter())
            .chain(relay_server_addresses.iter())
        {
            push_unique(&mut bootstrap_addresses, address.clone());
        }

        AukiBrowserBootstrapRecord {
            peer_id,
            agent_version: self.config.agent_version.clone(),
            direct_addresses,
            webrtc_direct_addresses,
            relay_addresses,
            relay_server_addresses,
            bootstrap_addresses,
        }
    }

    pub(crate) fn configured_peers(&self) -> &[ConfiguredPeer] {
        &self.config.p2p.configured_peers
    }

    pub(crate) fn configured_peer(&self, peer_id: PeerId) -> Option<&ConfiguredPeer> {
        self.config
            .p2p
            .configured_peers
            .iter()
            .find(|peer| peer.peer_id == peer_id)
    }

    pub(crate) fn push_pending_event(&mut self, event: AukiP2pEvent) {
        self.pending_events.push_back(event);
    }

    pub(crate) fn pop_pending_event(&mut self) -> Option<AukiP2pEvent> {
        self.pending_events.pop_front()
    }

    pub(crate) fn upsert_configured_peer(
        &mut self,
        peer: ConfiguredPeer,
    ) -> Result<(), AukiP2pNodeError> {
        peer.validate_dial_addresses(self.config.p2p.dial_policy)
            .map_err(AukiP2pNodeError::DialPolicy)?;
        if let Some(existing) = self
            .config
            .p2p
            .configured_peers
            .iter_mut()
            .find(|existing| existing.peer_id == peer.peer_id)
        {
            *existing = peer;
        } else {
            self.config.p2p.configured_peers.push(peer);
        }
        Ok(())
    }

    /// Number of retained active connections for one peer.
    pub fn active_connection_count(&self, peer_id: PeerId) -> usize {
        self.connections.active_count(peer_id)
    }

    /// Runtime-observed paths for retained active connections to one peer.
    pub fn active_connection_paths(&self, peer_id: PeerId) -> Vec<AukiConnectionPath> {
        self.connections.active_paths(peer_id)
    }

    /// Add one operator-supplied advertised address.
    pub fn add_advertised_address(&mut self, address: Multiaddr) {
        push_unique(&mut self.config.advertised_addresses, address);
    }

    /// Add one relay-mediated connectivity address.
    pub fn add_relay_address(&mut self, address: Multiaddr) {
        push_unique(&mut self.config.relay_addresses, address);
    }

    /// Project local peer identity and address state into RFC status.
    pub fn local_peer_status(&self) -> Result<LocalPeerStatus, AukiP2pNodeError> {
        let mut object = Map::new();
        object.insert(
            "peer_id".to_owned(),
            Value::String(self.peer_id().to_string()),
        );
        object.insert(
            "wallet_public_key".to_owned(),
            Value::String(base64url::encode(&self.identity.wallet_public_key().0)),
        );
        if let Ok(issued_at) = self.identity.peer_binding().issued_at() {
            object.insert(
                "peer_binding_issued_at".to_owned(),
                Value::String(issued_at.to_owned()),
            );
        }
        object.insert(
            "authorization_mode".to_owned(),
            Value::String(self.config.p2p.peer_admission.mode().as_str().to_owned()),
        );
        object.insert(
            "agent_version".to_owned(),
            Value::String(self.config.agent_version.clone()),
        );
        object.insert(
            "relay_server_enabled".to_owned(),
            Value::Bool(self.config.relay_server.enabled),
        );
        object.insert(
            "relay_involved".to_owned(),
            Value::Bool(self.connections.relay_involved()),
        );
        object.insert(
            "active_transport_paths".to_owned(),
            Value::Array(self.active_connection_path_status_values(
                !self.config.p2p.status_privacy.redact_addresses,
            )),
        );

        if !self.config.p2p.status_privacy.redact_addresses {
            let browser_bootstrap = self.browser_bootstrap_record();
            object.insert(
                "listen_addresses".to_owned(),
                multiaddr_array(self.status_listen_addresses()),
            );
            object.insert(
                "advertised_addresses".to_owned(),
                multiaddr_array(&self.config.advertised_addresses),
            );
            object.insert(
                "relay_server_addresses".to_owned(),
                multiaddr_array(&self.observed_relay_server_addresses()),
            );
            object.insert(
                "relay_addresses".to_owned(),
                multiaddr_array(&browser_bootstrap.relay_addresses),
            );
            object.insert(
                "browser_bootstrap_addresses".to_owned(),
                multiaddr_array(&browser_bootstrap.bootstrap_addresses),
            );
            object.insert(
                "browser_direct_addresses".to_owned(),
                multiaddr_array(&browser_bootstrap.direct_addresses),
            );
            object.insert(
                "browser_webrtc_direct_addresses".to_owned(),
                multiaddr_array(&browser_bootstrap.webrtc_direct_addresses),
            );
            object.insert(
                "browser_relay_server_addresses".to_owned(),
                multiaddr_array(&browser_bootstrap.relay_server_addresses),
            );
            object.insert("browser_bootstrap".to_owned(), browser_bootstrap.to_value());
        }

        LocalPeerStatus::from_value(Value::Object(object)).map_err(AukiP2pNodeError::Status)
    }

    /// Create a cloneable raw-stream control handle for protocol runtimes.
    pub fn stream_control(&self) -> libp2p_stream::Control {
        self.swarm.behaviour().stream.new_control()
    }

    /// Bind one additional listen address.
    pub fn listen_on(&mut self, address: Multiaddr) -> Result<(), AukiP2pNodeError> {
        self.validate_browser_webrtc_direct_address(&address)?;
        self.swarm
            .listen_on(address.clone())
            .map_err(|source| AukiP2pNodeError::Listen {
                address: address.clone(),
                source: source.to_string(),
            })?;
        push_unique(&mut self.config.listen_addresses, address);
        Ok(())
    }

    /// Dial a peer through explicit addresses after applying the dial policy.
    pub fn dial_peer(
        &mut self,
        peer_id: PeerId,
        addresses: Vec<Multiaddr>,
    ) -> Result<(), AukiP2pNodeError> {
        for address in &addresses {
            self.validate_browser_webrtc_direct_address(address)?;
            self.config
                .p2p
                .dial_policy
                .check(address)
                .map_err(AukiP2pNodeError::DialPolicy)?;
        }

        let opts = DialOpts::peer_id(peer_id).addresses(addresses).build();
        self.swarm
            .dial(opts)
            .map_err(|source| AukiP2pNodeError::Dial {
                peer_id,
                source: Box::new(source),
            })
    }

    /// Wait until the next public node event is available.
    pub async fn next_event(&mut self) -> Option<AukiP2pEvent> {
        while let Some(event) = self.swarm.next().await {
            match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    push_unique(&mut self.observed_listen_addresses, address.clone());
                    return Some(AukiP2pEvent::Listening { address });
                }
                SwarmEvent::ConnectionEstablished {
                    peer_id,
                    connection_id,
                    endpoint,
                    ..
                } => {
                    let local_peer_id = self.peer_id();
                    let path = AukiConnectionPath::from_endpoint(&endpoint);
                    let preference =
                        ConnectionPreference::for_endpoint(local_peer_id, peer_id, &endpoint);
                    let retention = self.connections.established(
                        peer_id,
                        connection_id,
                        preference,
                        path,
                        self.config.p2p.limits.active_connections_per_peer_id,
                    );
                    match retention {
                        ConnectionRetention::Retained => {
                            return Some(AukiP2pEvent::ConnectionEstablished { peer_id });
                        }
                        ConnectionRetention::CloseDuplicate {
                            connection_id: close_connection_id,
                        } => {
                            self.swarm.close_connection(close_connection_id);
                            if close_connection_id == connection_id {
                                return Some(AukiP2pEvent::DuplicateConnectionClosed { peer_id });
                            }
                            return Some(AukiP2pEvent::ConnectionEstablished { peer_id });
                        }
                    }
                }
                SwarmEvent::ConnectionClosed {
                    peer_id,
                    connection_id,
                    ..
                } => {
                    self.connections.closed(peer_id, connection_id);
                    return Some(AukiP2pEvent::ConnectionClosed { peer_id });
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    return Some(AukiP2pEvent::OutgoingConnectionError {
                        peer_id,
                        error: error.to_string(),
                    });
                }
                SwarmEvent::IncomingConnectionError { error, .. } => {
                    return Some(AukiP2pEvent::IncomingConnectionError {
                        error: error.to_string(),
                    });
                }
                _ => {}
            }
        }

        None
    }
}

impl fmt::Display for AukiP2pNodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(f, "invalid p2p config: {error}"),
            Self::Status(error) => write!(f, "local peer status projection failed: {error}"),
            Self::Transport { stage, source } => {
                write!(f, "libp2p transport setup failed at {stage}: {source}")
            }
            Self::Listen { address, source } => {
                write!(f, "listen failed on {address}: {source}")
            }
            Self::DialPolicy(error) => write!(f, "{error}"),
            Self::Dial { peer_id, source } => write!(f, "dial {peer_id}: {source}"),
            Self::BrowserWebRtcDirectDisabled { address } => {
                write!(
                    f,
                    "WebRTC Direct is disabled but address {address} requires it"
                )
            }
            Self::BrowserWebRtcDirectFeatureDisabled => write!(
                f,
                "WebRTC Direct is enabled but auki-p2p was built without the browser-webrtc-direct feature"
            ),
        }
    }
}

impl std::error::Error for AukiP2pNodeError {}

impl ConnectionTracker {
    /// Track a newly established connection under the per-peer cap.
    fn established(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        preference: ConnectionPreference,
        path: AukiConnectionPath,
        cap: usize,
    ) -> ConnectionRetention {
        if cap == 0 {
            return ConnectionRetention::CloseDuplicate { connection_id };
        }

        let connections = self.established.entry(peer_id).or_default();
        if connections
            .iter()
            .any(|connection| connection.connection_id == connection_id)
        {
            return ConnectionRetention::Retained;
        }

        let connection = TrackedConnection {
            connection_id,
            preference,
            path,
        };

        if connections.len() < cap {
            connections.push(connection);
            return ConnectionRetention::Retained;
        }

        if preference == ConnectionPreference::Preferred
            && let Some((index, replaced)) = connections
                .iter()
                .enumerate()
                .find(|(_, connection)| connection.preference == ConnectionPreference::Fallback)
        {
            let replaced_connection_id = replaced.connection_id;
            connections[index] = connection;
            return ConnectionRetention::CloseDuplicate {
                connection_id: replaced_connection_id,
            };
        }

        ConnectionRetention::CloseDuplicate { connection_id }
    }
}

impl ConnectionPreference {
    fn for_endpoint(
        local_peer_id: PeerId,
        remote_peer_id: PeerId,
        endpoint: &ConnectedPoint,
    ) -> Self {
        let local_peer_should_dial = local_peer_id < remote_peer_id;
        if endpoint.is_dialer() == local_peer_should_dial {
            Self::Preferred
        } else {
            Self::Fallback
        }
    }
}

impl ConnectionTracker {
    /// Remove a closed connection from tracking.
    fn closed(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        if let Some(connections) = self.established.get_mut(&peer_id) {
            connections.retain(|connection| connection.connection_id != connection_id);
            if connections.is_empty() {
                self.established.remove(&peer_id);
            }
        }
    }

    /// Number of retained active connections for `peer_id`.
    fn active_count(&self, peer_id: PeerId) -> usize {
        self.established
            .get(&peer_id)
            .map_or(0, |connections| connections.len())
    }

    fn active_paths(&self, peer_id: PeerId) -> Vec<AukiConnectionPath> {
        self.established
            .get(&peer_id)
            .map(|connections| {
                connections
                    .iter()
                    .map(|connection| connection.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn all_active_paths(&self) -> Vec<(PeerId, AukiConnectionPath)> {
        self.established
            .iter()
            .flat_map(|(peer_id, connections)| {
                connections
                    .iter()
                    .map(|connection| (*peer_id, connection.path.clone()))
            })
            .collect()
    }

    fn relay_involved(&self) -> bool {
        self.established.values().any(|connections| {
            connections
                .iter()
                .any(|connection| connection.path.relay_involved)
        })
    }
}

fn default_agent_version() -> String {
    format!("auki-p2p/{}", env!("CARGO_PKG_VERSION"))
}

fn push_unique(addresses: &mut Vec<Multiaddr>, address: Multiaddr) {
    if !addresses.contains(&address) {
        addresses.push(address);
    }
}

fn multiaddr_array(addresses: &[Multiaddr]) -> Value {
    Value::Array(
        addresses
            .iter()
            .map(|address| Value::String(address.to_string()))
            .collect(),
    )
}

impl AukiP2pNode {
    fn validate_browser_webrtc_direct_address(
        &self,
        address: &Multiaddr,
    ) -> Result<(), AukiP2pNodeError> {
        if is_webrtc_direct_address(address) && !self.config.browser_webrtc_direct.enabled {
            return Err(AukiP2pNodeError::BrowserWebRtcDirectDisabled {
                address: address.clone(),
            });
        }
        Ok(())
    }

    fn status_listen_addresses(&self) -> &[Multiaddr] {
        if self.observed_listen_addresses.is_empty() {
            &self.config.listen_addresses
        } else {
            &self.observed_listen_addresses
        }
    }

    fn browser_direct_addresses(&self) -> Vec<Multiaddr> {
        let peer_id = self.peer_id();
        let mut addresses = Vec::new();
        for address in self.observed_dialable_listen_addresses().into_iter().chain(
            self.config
                .advertised_addresses
                .iter()
                .cloned()
                .map(|address| address_with_peer_id(address, peer_id)),
        ) {
            if is_browser_direct_address(&address) {
                push_unique(&mut addresses, address);
            }
        }
        addresses
    }

    fn active_connection_path_status_values(&self, include_addresses: bool) -> Vec<Value> {
        self.connections
            .all_active_paths()
            .into_iter()
            .map(|(peer_id, path)| {
                let mut value = path.to_status_value(include_addresses);
                if let Value::Object(object) = &mut value {
                    object.insert("peer_id".to_owned(), Value::String(peer_id.to_string()));
                }
                value
            })
            .collect()
    }
}

/// Return a loopback WebRTC Direct listen address with an OS-selected UDP port.
pub fn loopback_webrtc_direct_listen_addr() -> Multiaddr {
    "/ip4/127.0.0.1/udp/0/webrtc-direct"
        .parse()
        .expect("static loopback WebRTC Direct listen multiaddr is valid")
}

/// Return a loopback WebSocket relay listen address with an OS-selected TCP port.
pub fn loopback_websocket_relay_listen_addr() -> Multiaddr {
    "/ip4/127.0.0.1/tcp/0/ws"
        .parse()
        .expect("static loopback WebSocket relay listen multiaddr is valid")
}

fn validate_browser_webrtc_direct_config(
    config: &AukiP2pNodeConfig,
) -> Result<(), AukiP2pNodeError> {
    for address in &config.listen_addresses {
        if is_webrtc_direct_address(address) && !config.browser_webrtc_direct.enabled {
            return Err(AukiP2pNodeError::BrowserWebRtcDirectDisabled {
                address: address.clone(),
            });
        }
    }

    if config.browser_webrtc_direct.enabled {
        browser_webrtc_direct_feature_enabled()?;
    }

    Ok(())
}

fn is_webrtc_direct_address(address: &Multiaddr) -> bool {
    address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::WebRTCDirect))
}

fn is_websocket_address(address: &Multiaddr) -> bool {
    address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Ws(_) | Protocol::Wss(_)))
}

fn is_browser_direct_address(address: &Multiaddr) -> bool {
    is_webrtc_direct_address(address) || is_websocket_address(address)
}

fn address_with_peer_id(address: Multiaddr, peer_id: PeerId) -> Multiaddr {
    if matches!(address.iter().last(), Some(Protocol::P2p(_))) {
        address
    } else {
        address.with(Protocol::P2p(peer_id))
    }
}

fn build_transport(
    keypair: &libp2p_identity::Keypair,
    config: &AukiP2pNodeConfig,
) -> Result<Boxed<(PeerId, StreamMuxerBox)>, AukiP2pNodeError> {
    let transport = tcp_transport(keypair)?
        .or_transport(quic_transport(keypair))
        .map(|either, _| either_into_inner(either))
        .boxed();
    let transport = transport
        .or_transport(websocket_transport(keypair)?)
        .map(|either, _| either_into_inner(either))
        .boxed();

    #[cfg(feature = "browser-webrtc-direct")]
    let transport = if config.browser_webrtc_direct.enabled {
        transport
            .or_transport(webrtc_direct_transport(keypair))
            .map(|either, _| either_into_inner(either))
            .boxed()
    } else {
        transport
    };

    #[cfg(not(feature = "browser-webrtc-direct"))]
    let _ = config;

    Ok(transport)
}

fn tcp_transport(
    keypair: &libp2p_identity::Keypair,
) -> Result<Boxed<(PeerId, StreamMuxerBox)>, AukiP2pNodeError> {
    Ok(tcp::tokio::Transport::new(tcp::Config::default())
        .upgrade(upgrade::Version::V1Lazy)
        .authenticate(noise_config(keypair, "tcp")?)
        .multiplex(yamux::Config::default())
        .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
        .boxed())
}

fn quic_transport(keypair: &libp2p_identity::Keypair) -> Boxed<(PeerId, StreamMuxerBox)> {
    quic::tokio::Transport::new(quic::Config::new(keypair))
        .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
        .boxed()
}

fn websocket_transport(
    keypair: &libp2p_identity::Keypair,
) -> Result<Boxed<(PeerId, StreamMuxerBox)>, AukiP2pNodeError> {
    Ok(
        libp2p::websocket::Config::new(tcp::tokio::Transport::new(tcp::Config::default()))
            .upgrade(upgrade::Version::V1Lazy)
            .authenticate(noise_config(keypair, "websocket")?)
            .multiplex(yamux::Config::default())
            .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
            .boxed(),
    )
}

fn noise_config(
    keypair: &libp2p_identity::Keypair,
    stage: &'static str,
) -> Result<noise::Config, AukiP2pNodeError> {
    noise::Config::new(keypair).map_err(|err| AukiP2pNodeError::Transport {
        stage,
        source: err.to_string(),
    })
}

fn either_into_inner<T>(either: future::Either<T, T>) -> T {
    match either {
        future::Either::Left(inner) | future::Either::Right(inner) => inner,
    }
}

#[cfg(feature = "browser-webrtc-direct")]
fn browser_webrtc_direct_feature_enabled() -> Result<(), AukiP2pNodeError> {
    Ok(())
}

#[cfg(not(feature = "browser-webrtc-direct"))]
fn browser_webrtc_direct_feature_enabled() -> Result<(), AukiP2pNodeError> {
    Err(AukiP2pNodeError::BrowserWebRtcDirectFeatureDisabled)
}

#[cfg(feature = "browser-webrtc-direct")]
fn webrtc_direct_transport(keypair: &libp2p_identity::Keypair) -> Boxed<(PeerId, StreamMuxerBox)> {
    let certificate = webrtc::tokio::Certificate::generate(&mut thread_rng())
        .expect("WebRTC certificate generation should succeed");
    webrtc::tokio::Transport::new(keypair.clone(), certificate)
        .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_identity::Wallet;
    use tokio::time::{Duration, timeout};

    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";

    fn identity(seed: u8) -> LocalPeerIdentity {
        let wallet = Wallet::from_seed(vec![seed; 32]).expect("32-byte seed");
        LocalPeerIdentity::from_wallet(wallet, ISSUED_AT, Some("node-test"))
            .expect("local peer identity")
    }

    async fn wait_for_listen_addr(node: &mut AukiP2pNode) -> Multiaddr {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(AukiP2pEvent::Listening { address }) = node.next_event().await {
                    return address;
                }
            }
        })
        .await
        .expect("listen address should be emitted")
    }

    #[cfg(feature = "browser-webrtc-direct")]
    async fn wait_for_listen_addr_count(node: &mut AukiP2pNode, count: usize) -> Vec<Multiaddr> {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(AukiP2pEvent::Listening { .. }) = node.next_event().await {
                    if node.observed_listen_addresses().len() >= count {
                        return node.observed_listen_addresses().to_vec();
                    }
                }
            }
        })
        .await
        .expect("listen addresses should be emitted")
    }

    fn test_connection_path(port: u16) -> AukiConnectionPath {
        AukiConnectionPath::from_endpoint(&ConnectedPoint::Dialer {
            address: format!("/ip4/127.0.0.1/tcp/{port}").parse().unwrap(),
            role_override: libp2p::core::Endpoint::Dialer,
            port_use: libp2p::core::transport::PortUse::New,
        })
    }

    #[test]
    fn loopback_webrtc_direct_config_enables_browser_transport() {
        let config = AukiP2pNodeConfig::loopback_webrtc_direct_development();

        assert!(config.browser_webrtc_direct.enabled);
        assert!(!config.relay_server.enabled);
        assert_eq!(
            config.listen_addresses,
            vec![loopback_webrtc_direct_listen_addr()]
        );
    }

    #[test]
    fn loopback_relay_server_config_enables_relay_role() {
        let config = AukiP2pNodeConfig::loopback_relay_server_development();

        assert!(config.relay_server.enabled);
        assert!(!config.browser_webrtc_direct.enabled);
        assert_eq!(
            config.listen_addresses,
            vec![loopback_websocket_relay_listen_addr()]
        );
    }

    #[test]
    fn loopback_browser_reachable_config_enables_webrtc_and_relay() {
        let config = AukiP2pNodeConfig::loopback_browser_reachable_development();

        assert!(config.browser_webrtc_direct.enabled);
        assert!(config.relay_server.enabled);
        assert_eq!(
            config.listen_addresses,
            vec![
                loopback_webrtc_direct_listen_addr(),
                loopback_websocket_relay_listen_addr()
            ]
        );
    }

    #[test]
    fn webrtc_direct_listen_address_requires_node_enablement() {
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config
            .listen_addresses
            .push(loopback_webrtc_direct_listen_addr());

        let error = match AukiP2pNode::new(identity(28), config) {
            Ok(_) => panic!("WebRTC Direct listen address should be rejected when disabled"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AukiP2pNodeError::BrowserWebRtcDirectDisabled { .. }
        ));
    }

    #[test]
    #[cfg(not(feature = "browser-webrtc-direct"))]
    fn enabling_webrtc_direct_without_crate_feature_fails() {
        let config = AukiP2pNodeConfig::loopback_webrtc_direct_development();

        let error = match AukiP2pNode::new(identity(29), config) {
            Ok(_) => panic!("WebRTC Direct should require the browser-webrtc-direct feature"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AukiP2pNodeError::BrowserWebRtcDirectFeatureDisabled
        ));
    }

    #[tokio::test]
    #[cfg(feature = "browser-webrtc-direct")]
    async fn loopback_webrtc_direct_listener_emits_dialable_address() {
        let mut listener = AukiP2pNode::new(
            identity(29),
            AukiP2pNodeConfig::loopback_webrtc_direct_development(),
        )
        .unwrap();

        let address = wait_for_listen_addr(&mut listener).await;

        assert!(is_webrtc_direct_address(&address));
        assert!(address.to_string().contains("/certhash/"));
        assert_eq!(listener.observed_listen_addresses(), &[address.clone()]);
        assert_eq!(
            listener.observed_dialable_listen_addresses(),
            vec![address.clone().with(Protocol::P2p(listener.peer_id()))]
        );

        let status = listener.local_peer_status().expect("local status");
        assert_eq!(status.listen_addresses, vec![address.to_string()]);
    }

    #[tokio::test]
    #[cfg(feature = "browser-webrtc-direct")]
    async fn loopback_browser_reachable_listener_builds_browser_bootstrap_record() {
        let mut node = AukiP2pNode::new(
            identity(35),
            AukiP2pNodeConfig::loopback_browser_reachable_development(),
        )
        .unwrap();

        let addresses = wait_for_listen_addr_count(&mut node, 2).await;
        let record = node.browser_bootstrap_record();

        assert!(addresses.iter().any(is_webrtc_direct_address));
        assert!(addresses.iter().any(is_websocket_address));
        assert_eq!(record.peer_id, node.peer_id());
        assert_eq!(record.agent_version, default_agent_version());
        assert_eq!(record.direct_addresses.len(), 2);
        assert_eq!(record.webrtc_direct_addresses.len(), 1);
        assert_eq!(record.relay_server_addresses.len(), 1);
        assert_eq!(record.bootstrap_addresses.len(), 2);
        assert!(
            record.webrtc_direct_addresses[0]
                .to_string()
                .contains("/certhash/")
        );
        assert!(is_websocket_address(&record.relay_server_addresses[0]));
        assert!(record.bootstrap_addresses.iter().all(|address| {
            address.iter().last().is_some_and(
                |protocol| matches!(protocol, Protocol::P2p(peer) if peer == node.peer_id()),
            )
        }));

        let status = node.local_peer_status().expect("local status");
        assert_eq!(
            status
                .value()
                .get("browser_bootstrap_addresses")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
    }

    #[tokio::test]
    async fn loopback_relay_server_listener_emits_browser_relay_address() {
        let mut relay = AukiP2pNode::new(
            identity(34),
            AukiP2pNodeConfig::loopback_relay_server_development(),
        )
        .unwrap();

        let address = wait_for_listen_addr(&mut relay).await;
        let dialable = address.clone().with(Protocol::P2p(relay.peer_id()));
        let dialable_string = dialable.to_string();

        assert!(is_websocket_address(&address));
        assert_eq!(
            relay.observed_listen_addresses(),
            std::slice::from_ref(&address)
        );
        assert_eq!(
            relay.observed_relay_server_addresses(),
            vec![dialable.clone()]
        );
        assert_eq!(
            relay.observed_browser_relay_server_addresses(),
            vec![dialable.clone()]
        );
        assert!(
            relay.relay_addresses().is_empty(),
            "local relay server addresses must stay separate from remote relay hints"
        );

        let record = relay.browser_bootstrap_record();
        assert_eq!(record.direct_addresses, vec![dialable.clone()]);
        assert!(record.webrtc_direct_addresses.is_empty());
        assert!(record.relay_addresses.is_empty());
        assert_eq!(record.relay_server_addresses, vec![dialable.clone()]);
        assert_eq!(record.bootstrap_addresses, vec![dialable.clone()]);

        let bootstrap_value = record.to_value();
        let peer_id = relay.peer_id().to_string();
        assert_eq!(
            bootstrap_value.get("peer_id").and_then(Value::as_str),
            Some(peer_id.as_str())
        );
        assert!(bootstrap_value.get("local_domains").is_none());
        assert!(bootstrap_value.get("offers").is_none());

        let status = relay.local_peer_status().expect("local status");
        assert_eq!(
            status
                .value()
                .get("relay_server_enabled")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            status
                .value()
                .get("relay_server_addresses")
                .and_then(Value::as_array)
                .and_then(|addresses| addresses.first())
                .and_then(Value::as_str),
            Some(dialable_string.as_str())
        );
        assert_eq!(
            status
                .value()
                .get("browser_bootstrap_addresses")
                .and_then(Value::as_array)
                .and_then(|addresses| addresses.first())
                .and_then(Value::as_str),
            Some(dialable_string.as_str())
        );
    }

    #[test]
    fn connection_tracker_rejects_connections_over_peer_cap() {
        let peer_id = identity(20).peer_id();
        let first = ConnectionId::new_unchecked(1);
        let second = ConnectionId::new_unchecked(2);
        let mut tracker = ConnectionTracker::default();

        assert_eq!(
            tracker.established(
                peer_id,
                first,
                ConnectionPreference::Preferred,
                test_connection_path(1),
                1
            ),
            ConnectionRetention::Retained
        );
        assert_eq!(tracker.active_count(peer_id), 1);
        assert_eq!(tracker.active_paths(peer_id).len(), 1);
        assert_eq!(
            tracker.established(
                peer_id,
                second,
                ConnectionPreference::Fallback,
                test_connection_path(2),
                1
            ),
            ConnectionRetention::CloseDuplicate {
                connection_id: second
            }
        );
        assert_eq!(tracker.active_count(peer_id), 1);
        tracker.closed(peer_id, second);
        assert_eq!(tracker.active_count(peer_id), 1);

        tracker.closed(peer_id, first);
        assert_eq!(tracker.active_count(peer_id), 0);
        assert_eq!(
            tracker.established(
                peer_id,
                second,
                ConnectionPreference::Preferred,
                test_connection_path(2),
                1
            ),
            ConnectionRetention::Retained
        );
    }

    #[test]
    fn connection_tracker_replaces_fallback_with_preferred_connection() {
        let local_peer_id = identity(30).peer_id();
        let remote_peer_id = identity(31).peer_id();
        let (lower_peer_id, higher_peer_id) = if local_peer_id < remote_peer_id {
            (local_peer_id, remote_peer_id)
        } else {
            (remote_peer_id, local_peer_id)
        };
        let fallback = ConnectionId::new_unchecked(1);
        let preferred = ConnectionId::new_unchecked(2);
        let mut lower_peer_tracker = ConnectionTracker::default();
        let mut higher_peer_tracker = ConnectionTracker::default();

        assert_eq!(
            lower_peer_tracker.established(
                higher_peer_id,
                fallback,
                ConnectionPreference::Fallback,
                test_connection_path(1),
                1
            ),
            ConnectionRetention::Retained
        );
        assert_eq!(
            lower_peer_tracker.established(
                higher_peer_id,
                preferred,
                ConnectionPreference::Preferred,
                test_connection_path(2),
                1
            ),
            ConnectionRetention::CloseDuplicate {
                connection_id: fallback
            }
        );
        assert_eq!(
            lower_peer_tracker.established[&higher_peer_id][0].connection_id,
            preferred
        );

        assert_eq!(
            higher_peer_tracker.established(
                lower_peer_id,
                fallback,
                ConnectionPreference::Fallback,
                test_connection_path(3),
                1
            ),
            ConnectionRetention::Retained
        );
        assert_eq!(
            higher_peer_tracker.established(
                lower_peer_id,
                preferred,
                ConnectionPreference::Preferred,
                test_connection_path(4),
                1
            ),
            ConnectionRetention::CloseDuplicate {
                connection_id: fallback
            }
        );
        assert_eq!(
            higher_peer_tracker.established[&lower_peer_id][0].connection_id,
            preferred
        );
    }

    #[test]
    fn connection_preference_selects_lower_peer_dialer_side() {
        let first_peer_id = identity(32).peer_id();
        let second_peer_id = identity(33).peer_id();
        let (lower_peer_id, higher_peer_id) = if first_peer_id < second_peer_id {
            (first_peer_id, second_peer_id)
        } else {
            (second_peer_id, first_peer_id)
        };
        let dialer_endpoint = ConnectedPoint::Dialer {
            address: "/ip4/127.0.0.1/tcp/1".parse().unwrap(),
            role_override: libp2p::core::Endpoint::Dialer,
            port_use: libp2p::core::transport::PortUse::New,
        };
        let listener_endpoint = ConnectedPoint::Listener {
            local_addr: "/ip4/127.0.0.1/tcp/2".parse().unwrap(),
            send_back_addr: "/ip4/127.0.0.1/tcp/3".parse().unwrap(),
        };

        assert_eq!(
            ConnectionPreference::for_endpoint(lower_peer_id, higher_peer_id, &dialer_endpoint),
            ConnectionPreference::Preferred
        );
        assert_eq!(
            ConnectionPreference::for_endpoint(lower_peer_id, higher_peer_id, &listener_endpoint),
            ConnectionPreference::Fallback
        );
        assert_eq!(
            ConnectionPreference::for_endpoint(higher_peer_id, lower_peer_id, &listener_endpoint),
            ConnectionPreference::Preferred
        );
        assert_eq!(
            ConnectionPreference::for_endpoint(higher_peer_id, lower_peer_id, &dialer_endpoint),
            ConnectionPreference::Fallback
        );
    }

    #[tokio::test]
    async fn two_loopback_nodes_connect_with_deterministic_identities() {
        let mut dialer =
            AukiP2pNode::new(identity(21), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener =
            AukiP2pNode::new(identity(22), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();
        let dialer_peer_id = dialer.peer_id();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;

        dialer
            .dial_peer(listener_peer_id, vec![listener_addr])
            .expect("dial should be accepted");

        timeout(Duration::from_secs(10), async {
            let mut dialer_observed_listener = false;
            let mut listener_observed_dialer = false;

            loop {
                tokio::select! {
                    event = dialer.next_event() => {
                        if let Some(AukiP2pEvent::ConnectionEstablished { peer_id }) = event {
                            dialer_observed_listener |= peer_id == listener_peer_id;
                        }
                    }
                    event = listener.next_event() => {
                        if let Some(AukiP2pEvent::ConnectionEstablished { peer_id }) = event {
                            listener_observed_dialer |= peer_id == dialer_peer_id;
                        }
                    }
                }

                if dialer_observed_listener && listener_observed_dialer {
                    break;
                }
            }
        })
        .await
        .expect("both peers should observe an authenticated connection");

        let dialer_paths = dialer.active_connection_paths(listener_peer_id);
        assert_eq!(dialer_paths.len(), 1);
        assert_eq!(dialer_paths[0].direction.as_str(), "dialer");
        assert_eq!(dialer_paths[0].transport.as_str(), "tcp");
        assert!(!dialer_paths[0].relay_involved);

        let status = dialer.local_peer_status().expect("local status");
        assert_eq!(
            status
                .value()
                .get("relay_involved")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            status
                .value()
                .get("active_transport_paths")
                .and_then(Value::as_array)
                .and_then(|paths| paths.first())
                .and_then(|path| path.get("transport"))
                .and_then(Value::as_str),
            Some("tcp")
        );
    }

    #[test]
    fn node_config_keeps_address_roles_separate() {
        let advertised: Multiaddr = "/ip4/203.0.113.10/tcp/4001/ws".parse().unwrap();
        let relay: Multiaddr = "/ip4/198.51.100.10/tcp/4001/p2p-circuit".parse().unwrap();
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config.advertised_addresses.push(advertised.clone());
        config.relay_addresses.push(relay.clone());
        let node = AukiP2pNode::new(identity(25), config).unwrap();
        let advertised_dialable = advertised.clone().with(Protocol::P2p(node.peer_id()));
        let relay_dialable = relay.clone().with(Protocol::P2p(node.peer_id()));

        assert!(node.configured_listen_addresses().is_empty());
        assert!(node.observed_listen_addresses().is_empty());
        assert_eq!(node.advertised_addresses(), &[advertised]);
        assert_eq!(node.relay_addresses(), &[relay]);

        let record = node.browser_bootstrap_record();
        assert_eq!(record.direct_addresses, vec![advertised_dialable.clone()]);
        assert!(record.webrtc_direct_addresses.is_empty());
        assert_eq!(record.relay_addresses, vec![relay_dialable.clone()]);
        assert!(record.relay_server_addresses.is_empty());
        assert_eq!(
            record.bootstrap_addresses,
            vec![advertised_dialable.clone(), relay_dialable.clone()]
        );

        let status = node.local_peer_status().expect("local status");
        assert!(status.listen_addresses.is_empty());
        assert_eq!(
            status.advertised_addresses,
            vec!["/ip4/203.0.113.10/tcp/4001/ws"]
        );
        assert_eq!(
            status
                .value()
                .get("relay_addresses")
                .and_then(Value::as_array)
                .and_then(|addresses| addresses.first())
                .and_then(Value::as_str),
            Some(relay_dialable.to_string().as_str())
        );
        assert_eq!(
            status
                .value()
                .get("browser_bootstrap_addresses")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(
            status
                .value()
                .get("browser_bootstrap")
                .and_then(|value| value.get("local_domains"))
                .is_none()
        );
        assert!(
            status
                .value()
                .get("browser_bootstrap")
                .and_then(|value| value.get("offers"))
                .is_none()
        );
    }

    #[test]
    fn address_with_peer_id_appends_terminal_peer_after_relay_path() {
        let relay_peer = identity(36).peer_id();
        let local_peer = identity(37).peer_id();
        let relay_path: Multiaddr =
            format!("/ip4/127.0.0.1/tcp/4001/ws/p2p/{relay_peer}/p2p-circuit")
                .parse()
                .unwrap();

        let address = address_with_peer_id(relay_path, local_peer);

        assert!(matches!(
            address.iter().last(),
            Some(Protocol::P2p(peer)) if peer == local_peer
        ));
        assert_eq!(
            address.to_string(),
            format!("/ip4/127.0.0.1/tcp/4001/ws/p2p/{relay_peer}/p2p-circuit/p2p/{local_peer}")
        );
    }

    #[tokio::test]
    async fn loopback_listener_is_not_auto_advertised() {
        let mut listener =
            AukiP2pNode::new(identity(26), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();

        assert!(listener.advertised_addresses().is_empty());
        let address = wait_for_listen_addr(&mut listener).await;

        assert_eq!(listener.configured_listen_addresses().len(), 1);
        assert_eq!(
            listener.observed_listen_addresses(),
            std::slice::from_ref(&address)
        );
        assert_eq!(
            listener.observed_dialable_listen_addresses(),
            vec![address.clone().with(Protocol::P2p(listener.peer_id()))]
        );
        assert!(listener.advertised_addresses().is_empty());

        let status = listener.local_peer_status().expect("local status");
        assert_eq!(status.listen_addresses, vec![address.to_string()]);
        assert!(status.advertised_addresses.is_empty());
    }

    #[tokio::test]
    async fn local_peer_status_redacts_addresses_under_privacy_policy() {
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config.listen_addresses = vec!["/ip4/0.0.0.0/tcp/0".parse().unwrap()];
        config.advertised_addresses = vec!["/ip4/203.0.113.10/tcp/4001".parse().unwrap()];
        config.p2p.status_privacy = crate::StatusPrivacyConfig::production_recommended();
        let node = AukiP2pNode::new(identity(27), config).unwrap();

        let status = node.local_peer_status().expect("local status");

        assert!(status.listen_addresses.is_empty());
        assert!(status.advertised_addresses.is_empty());
        assert!(status.value().get("relay_addresses").is_none());
        assert!(status.value().get("browser_bootstrap_addresses").is_none());
        assert!(status.value().get("browser_bootstrap").is_none());
    }

    #[tokio::test]
    async fn dial_policy_rejects_address_before_libp2p_dial() {
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config.p2p.dial_policy = crate::DialPolicy::production_recommended();
        let mut node = AukiP2pNode::new(identity(23), config).unwrap();
        let remote = identity(24).peer_id();
        let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();

        let error = node.dial_peer(remote, vec![loopback]).unwrap_err();

        assert!(matches!(error, AukiP2pNodeError::DialPolicy(_)));
    }
}
