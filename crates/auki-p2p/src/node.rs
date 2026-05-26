//! Minimal libp2p node wrapper for the RFC-first runtime.

use crate::{AukiP2pConfig, ConfigError, ConfiguredPeer, DialPolicyError, LocalPeerIdentity};
use auki_protocol::v1::{
    base64url,
    status::{LocalPeerStatus, StatusError},
};
use futures::StreamExt as _;
use libp2p::{
    Multiaddr, PeerId, Swarm, SwarmBuilder,
    core::ConnectedPoint,
    identify, noise, ping,
    swarm::{ConnectionId, DialError, NetworkBehaviour, SwarmEvent, dial_opts::DialOpts},
    tcp, yamux,
};
use serde_json::{Map, Value};
use std::{collections::HashMap, fmt, time::Duration};

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
    /// Identify agent version advertised to remote libp2p peers.
    pub agent_version: String,
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
        source: DialError,
    },
}

/// Small, directly pollable libp2p node.
pub struct AukiP2pNode {
    identity: LocalPeerIdentity,
    config: AukiP2pNodeConfig,
    observed_listen_addresses: Vec<Multiaddr>,
    connections: ConnectionTracker,
    swarm: Swarm<Behaviour>,
}

/// Local per-peer connection retention state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConnectionTracker {
    established: HashMap<PeerId, Vec<TrackedConnection>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrackedConnection {
    connection_id: ConnectionId,
    preference: ConnectionPreference,
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
            agent_version: default_agent_version(),
        }
    }
}

impl AukiP2pNode {
    /// Build a node and bind every configured listen address.
    pub fn new(
        identity: LocalPeerIdentity,
        config: AukiP2pNodeConfig,
    ) -> Result<Self, AukiP2pNodeError> {
        config.p2p.validate().map_err(AukiP2pNodeError::Config)?;

        let local_peer_id = identity.peer_id();
        let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(|err| AukiP2pNodeError::Transport {
                stage: "tcp",
                source: err.to_string(),
            })?
            .with_quic()
            .with_behaviour(|key| Behaviour {
                identify: identify::Behaviour::new(
                    identify::Config::new(IDENTIFY_PROTOCOL_ID.into(), key.public())
                        .with_agent_version(config.agent_version.clone()),
                ),
                ping: ping::Behaviour::default(),
                stream: libp2p_stream::Behaviour::new(),
            })
            .map_err(|err| AukiP2pNodeError::Transport {
                stage: "behaviour",
                source: err.to_string(),
            })?
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(IDLE_CONNECTION_TIMEOUT))
            .build();

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

    /// Operator-supplied advertised addresses.
    pub fn advertised_addresses(&self) -> &[Multiaddr] {
        &self.config.advertised_addresses
    }

    /// Relay-mediated connectivity addresses.
    pub fn relay_addresses(&self) -> &[Multiaddr] {
        &self.config.relay_addresses
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

        if !self.config.p2p.status_privacy.redact_addresses {
            object.insert(
                "listen_addresses".to_owned(),
                multiaddr_array(self.status_listen_addresses()),
            );
            object.insert(
                "advertised_addresses".to_owned(),
                multiaddr_array(&self.config.advertised_addresses),
            );
        }

        LocalPeerStatus::from_value(Value::Object(object)).map_err(AukiP2pNodeError::Status)
    }

    /// Create a cloneable raw-stream control handle for protocol runtimes.
    pub fn stream_control(&self) -> libp2p_stream::Control {
        self.swarm.behaviour().stream.new_control()
    }

    /// Bind one additional listen address.
    pub fn listen_on(&mut self, address: Multiaddr) -> Result<(), AukiP2pNodeError> {
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
            self.config
                .p2p
                .dial_policy
                .check(address)
                .map_err(AukiP2pNodeError::DialPolicy)?;
        }

        let opts = DialOpts::peer_id(peer_id).addresses(addresses).build();
        self.swarm
            .dial(opts)
            .map_err(|source| AukiP2pNodeError::Dial { peer_id, source })
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
                    let preference =
                        ConnectionPreference::for_endpoint(local_peer_id, peer_id, &endpoint);
                    let retention = self.connections.established(
                        peer_id,
                        connection_id,
                        preference,
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
        };

        if connections.len() < cap {
            connections.push(connection);
            return ConnectionRetention::Retained;
        }

        if preference == ConnectionPreference::Preferred {
            if let Some((index, replaced)) = connections
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
    fn status_listen_addresses(&self) -> &[Multiaddr] {
        if self.observed_listen_addresses.is_empty() {
            &self.config.listen_addresses
        } else {
            &self.observed_listen_addresses
        }
    }
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

    #[test]
    fn connection_tracker_rejects_connections_over_peer_cap() {
        let peer_id = identity(20).peer_id();
        let first = ConnectionId::new_unchecked(1);
        let second = ConnectionId::new_unchecked(2);
        let mut tracker = ConnectionTracker::default();

        assert_eq!(
            tracker.established(peer_id, first, ConnectionPreference::Preferred, 1),
            ConnectionRetention::Retained
        );
        assert_eq!(tracker.active_count(peer_id), 1);
        assert_eq!(
            tracker.established(peer_id, second, ConnectionPreference::Fallback, 1),
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
            tracker.established(peer_id, second, ConnectionPreference::Preferred, 1),
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
                1
            ),
            ConnectionRetention::Retained
        );
        assert_eq!(
            lower_peer_tracker.established(
                higher_peer_id,
                preferred,
                ConnectionPreference::Preferred,
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
                1
            ),
            ConnectionRetention::Retained
        );
        assert_eq!(
            higher_peer_tracker.established(
                lower_peer_id,
                preferred,
                ConnectionPreference::Preferred,
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
    }

    #[test]
    fn node_config_keeps_address_roles_separate() {
        let advertised: Multiaddr = "/ip4/203.0.113.10/tcp/4001".parse().unwrap();
        let relay: Multiaddr = "/ip4/198.51.100.10/tcp/4001/p2p-circuit".parse().unwrap();
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config.advertised_addresses.push(advertised.clone());
        config.relay_addresses.push(relay.clone());
        let node = AukiP2pNode::new(identity(25), config).unwrap();

        assert!(node.configured_listen_addresses().is_empty());
        assert!(node.observed_listen_addresses().is_empty());
        assert_eq!(node.advertised_addresses(), &[advertised]);
        assert_eq!(node.relay_addresses(), &[relay]);

        let status = node.local_peer_status().expect("local status");
        assert!(status.listen_addresses.is_empty());
        assert_eq!(
            status.advertised_addresses,
            vec!["/ip4/203.0.113.10/tcp/4001"]
        );
    }

    #[tokio::test]
    async fn loopback_listener_is_not_auto_advertised() {
        let mut listener =
            AukiP2pNode::new(identity(26), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();

        assert!(listener.advertised_addresses().is_empty());
        let address = wait_for_listen_addr(&mut listener).await;

        assert_eq!(listener.configured_listen_addresses().len(), 1);
        assert_eq!(listener.observed_listen_addresses(), &[address.clone()]);
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
