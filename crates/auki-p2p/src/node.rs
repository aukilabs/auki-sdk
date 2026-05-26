//! Minimal libp2p node wrapper for the RFC-first runtime.

use crate::{AukiP2pConfig, ConfigError, DialPolicyError, LocalPeerIdentity};
use futures::StreamExt as _;
use libp2p::{
    Multiaddr, PeerId, Swarm, SwarmBuilder, identify, noise, ping,
    swarm::{DialError, NetworkBehaviour, SwarmEvent, dial_opts::DialOpts},
    tcp, yamux,
};
use std::{fmt, time::Duration};

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
    /// Multiaddrs to bind during construction.
    pub listen_addresses: Vec<Multiaddr>,
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
    swarm: Swarm<Behaviour>,
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

    /// Create a cloneable raw-stream control handle for protocol runtimes.
    pub fn stream_control(&self) -> libp2p_stream::Control {
        self.swarm.behaviour().stream.new_control()
    }

    /// Bind one additional listen address.
    pub fn listen_on(&mut self, address: Multiaddr) -> Result<(), AukiP2pNodeError> {
        self.swarm
            .listen_on(address.clone())
            .map_err(|source| AukiP2pNodeError::Listen {
                address,
                source: source.to_string(),
            })?;
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
                    return Some(AukiP2pEvent::Listening { address });
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    return Some(AukiP2pEvent::ConnectionEstablished { peer_id });
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
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

fn default_agent_version() -> String {
    format!("auki-p2p/{}", env!("CARGO_PKG_VERSION"))
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
