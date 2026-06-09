//! Domain Relay capability for browser-reachable Auki Domains.

use auki_network::PeerIdentity;
use futures::StreamExt as _;
use libp2p::{
    Multiaddr, PeerId, identify, noise, ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use thiserror::Error;

const IDENTIFY_PROTOCOL: &str = "/auki/domain-relay/0.0.1";

/// Configuration for a Domain Relay instance.
#[derive(Debug, Clone)]
pub struct DomainRelayConfig {
    /// Addresses the relay should listen on. Browser-compatible
    /// deployments typically include WebSocket addresses.
    pub listen_addresses: Vec<Multiaddr>,
    /// libp2p identify agent version advertised by this relay.
    pub agent_version: String,
}

/// Events emitted by [`DomainRelay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainRelayEvent {
    /// The relay is listening on a Discovery-ready multiaddr suffixed
    /// with `/p2p/<relay-peer-id>`.
    Listening { relay_multiaddr: Multiaddr },
}

/// Errors that can occur while building or binding the relay.
#[derive(Debug, Error)]
pub enum DomainRelayError {
    /// libp2p transport or behaviour construction failed.
    #[error("relay build failed: {0}")]
    Build(String),
    /// One configured listen address could not be bound.
    #[error("relay listen failed on {addr}: {source}")]
    Listen {
        /// Address that failed.
        addr: Multiaddr,
        /// libp2p listen error.
        #[source]
        source: libp2p::TransportError<std::io::Error>,
    },
}

/// Running Domain Relay.
pub struct DomainRelay {
    peer_id: PeerId,
    swarm: libp2p::Swarm<RelayBehaviour>,
}

impl DomainRelay {
    /// Build and start a relay server for `identity`.
    pub async fn new(
        identity: &PeerIdentity,
        config: DomainRelayConfig,
    ) -> Result<Self, DomainRelayError> {
        let peer_id = identity.peer_id();
        let agent_version = config.agent_version;
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(identity.keypair().clone())
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(|err| DomainRelayError::Build(format!("tcp: {err}")))?
            .with_dns()
            .map_err(|err| DomainRelayError::Build(format!("dns: {err}")))?
            .with_websocket(noise::Config::new, yamux::Config::default)
            .await
            .map_err(|err| DomainRelayError::Build(format!("websocket: {err}")))?
            .with_behaviour(|key| RelayBehaviour {
                identify: identify::Behaviour::new(
                    identify::Config::new(IDENTIFY_PROTOCOL.into(), key.public())
                        .with_agent_version(agent_version),
                ),
                ping: ping::Behaviour::default(),
                relay: relay::Behaviour::new(peer_id, relay::Config::default()),
            })
            .map_err(|err| DomainRelayError::Build(err.to_string()))?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(std::time::Duration::from_secs(60))
            })
            .build();

        for addr in config.listen_addresses {
            swarm
                .listen_on(addr.clone())
                .map_err(|source| DomainRelayError::Listen { addr, source })?;
        }

        Ok(Self { peer_id, swarm })
    }

    /// Relay peer id.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Poll the relay until a public lifecycle event is available.
    pub async fn next_event(&mut self) -> Option<DomainRelayEvent> {
        while let Some(event) = self.swarm.next().await {
            if let SwarmEvent::NewListenAddr { address, .. } = event {
                self.swarm.add_external_address(address.clone());
                let relay_multiaddr = address.with(libp2p::multiaddr::Protocol::P2p(self.peer_id));
                return Some(DomainRelayEvent::Listening { relay_multiaddr });
            }
        }
        None
    }
}

#[derive(NetworkBehaviour)]
struct RelayBehaviour {
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    relay: relay::Behaviour,
}
