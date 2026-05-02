//! libp2p `Swarm` builder. Turns a [`PeerIdentity`] and a
//! [`SwarmConfig`] into a configured `libp2p::Swarm<Behaviour>`.
//!
//! Transport: TCP + QUIC, both authenticated with Noise (using the peer's
//! ed25519 keypair) and multiplexed with Yamux. Behaviour: a minimal
//! [`identify`] + [`ping`] composition — enough for two peers to dial
//! each other, exchange peer info, and keep the connection alive.
//!
//! Circuit Relay v2 (client + server) and mDNS land in M1b.
//!
//! ## Async runtime
//!
//! Tokio. The other SDK consumers (BoosterApp, Sentinel, the Relay app)
//! all use tokio; libp2p exposes a tokio integration via its `tokio`
//! feature, which we enable.
//!
//! ## Example
//!
//! ```no_run
//! # use auki_identity::Wallet;
//! # use auki_network::{PeerIdentity, swarm::{build_swarm, SwarmConfig}};
//! let wallet = Wallet::from_seed(&[7u8; 32]);
//! let identity = PeerIdentity::from_wallet(&wallet);
//! let swarm = build_swarm(&identity, SwarmConfig {
//!     listen_addresses: vec![
//!         "/ip4/0.0.0.0/tcp/0".parse().unwrap(),
//!         "/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap(),
//!     ],
//!     agent_version: "auki-sdk/0.0.0".into(),
//! }).expect("build swarm");
//! ```

use crate::PeerIdentity;
use libp2p::{
    Multiaddr, Swarm, SwarmBuilder, identify, noise, ping, swarm::NetworkBehaviour, tcp, yamux,
};
use std::time::Duration;

/// libp2p protocol id used for the SDK's identify exchanges. Stable; do
/// not change without coordinating with consumers (the agent_version
/// string is the per-deployment knob; the protocol id is the wire format).
pub const IDENTIFY_PROTOCOL: &str = "/auki/identify/1.0.0";

/// Idle connection timeout. libp2p closes a connection that's been idle
/// for this long; ping resets the idle timer, so this only fires if the
/// remote stops responding.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Composed behaviour for the M1a swarm. Generated `BehaviourEvent` enum
/// has one variant per field.
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
}

/// Per-swarm configuration. Listen addresses and the identify
/// `agent_version` string are the only knobs at M1a.
#[derive(Debug, Clone)]
pub struct SwarmConfig {
    /// Multiaddrs the swarm will listen on. Typical values:
    /// `"/ip4/0.0.0.0/tcp/0"` (TCP, OS-chosen port),
    /// `"/ip4/0.0.0.0/udp/0/quic-v1"` (QUIC, OS-chosen port). An empty
    /// list builds a swarm that only dials outbound.
    pub listen_addresses: Vec<Multiaddr>,
    /// Reported as `agent_version` in identify responses. Convention:
    /// `"<consumer-name>/<version>"` (e.g. `"boosterapp/0.1"`).
    pub agent_version: String,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            listen_addresses: vec![],
            agent_version: format!("auki-sdk/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Errors from [`build_swarm`].
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// Transport stack failed to assemble (TCP/Noise/Yamux or QUIC).
    #[error("transport setup failed: {0}")]
    Transport(String),
    /// `listen_on` rejected one of the configured addresses.
    #[error("listen failed for {addr}")]
    Listen {
        addr: Multiaddr,
        #[source]
        source: libp2p::TransportError<std::io::Error>,
    },
}

/// Assemble a libp2p swarm. Constructed swarm starts listening on every
/// address in `config.listen_addresses` before returning.
///
/// The swarm's local peer id matches `identity.peer_id()` exactly — the
/// caller can rely on this for advertising.
pub fn build_swarm(
    identity: &PeerIdentity,
    config: SwarmConfig,
) -> Result<Swarm<Behaviour>, BuildError> {
    let agent_version = config.agent_version;

    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| BuildError::Transport(format!("tcp: {e}")))?
        .with_quic()
        .with_behaviour(|key| Behaviour {
            identify: identify::Behaviour::new(
                identify::Config::new(IDENTIFY_PROTOCOL.into(), key.public())
                    .with_agent_version(agent_version),
            ),
            ping: ping::Behaviour::default(),
        })
        .map_err(|e| BuildError::Transport(format!("behaviour: {e}")))?
        .with_swarm_config(|c| c.with_idle_connection_timeout(IDLE_TIMEOUT))
        .build();

    for addr in config.listen_addresses {
        swarm
            .listen_on(addr.clone())
            .map_err(|source| BuildError::Listen { addr, source })?;
    }

    Ok(swarm)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PeerIdentity;
    use futures::StreamExt;
    use libp2p::swarm::SwarmEvent;

    /// Wait for a swarm's first `NewListenAddr` event and return the
    /// address. Times out after 5 seconds.
    async fn wait_for_listen_addr(swarm: &mut Swarm<Behaviour>) -> Multiaddr {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                    return address;
                }
            }
        })
        .await
        .expect("listen addr did not appear within timeout")
    }

    /// Drive both swarms until each has received an Identify::Received
    /// event from the other, or 10 s elapse.
    async fn run_until_mutual_identify(a: &mut Swarm<Behaviour>, b: &mut Swarm<Behaviour>) {
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let mut a_received = false;
            let mut b_received = false;
            while !(a_received && b_received) {
                tokio::select! {
                    Some(event) = a.next() => {
                        if let SwarmEvent::Behaviour(BehaviourEvent::Identify(
                            identify::Event::Received { .. }
                        )) = event {
                            a_received = true;
                        }
                    }
                    Some(event) = b.next() => {
                        if let SwarmEvent::Behaviour(BehaviourEvent::Identify(
                            identify::Event::Received { .. }
                        )) = event {
                            b_received = true;
                        }
                    }
                }
            }
        })
        .await;
        assert!(result.is_ok(), "identify exchange did not complete in time");
    }

    #[tokio::test]
    async fn local_peer_id_matches_identity() {
        let identity = PeerIdentity::from_seed(&[1u8; 32]);
        let swarm = build_swarm(
            &identity,
            SwarmConfig {
                listen_addresses: vec![],
                agent_version: "test/0".into(),
            },
        )
        .unwrap();
        assert_eq!(*swarm.local_peer_id(), identity.peer_id());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_peers_identify_each_other_over_tcp() {
        let id_a = PeerIdentity::from_seed(&[2u8; 32]);
        let id_b = PeerIdentity::from_seed(&[3u8; 32]);

        let mut a = build_swarm(
            &id_a,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                agent_version: "test-a/0".into(),
            },
        )
        .unwrap();
        let mut b = build_swarm(
            &id_b,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                agent_version: "test-b/0".into(),
            },
        )
        .unwrap();

        let addr_a = wait_for_listen_addr(&mut a).await;
        b.dial(addr_a).expect("dial");
        run_until_mutual_identify(&mut a, &mut b).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_peers_identify_each_other_over_quic() {
        let id_a = PeerIdentity::from_seed(&[4u8; 32]);
        let id_b = PeerIdentity::from_seed(&[5u8; 32]);

        let mut a = build_swarm(
            &id_a,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
                agent_version: "test-a/0".into(),
            },
        )
        .unwrap();
        let mut b = build_swarm(
            &id_b,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
                agent_version: "test-b/0".into(),
            },
        )
        .unwrap();

        let addr_a = wait_for_listen_addr(&mut a).await;
        b.dial(addr_a).expect("dial");
        run_until_mutual_identify(&mut a, &mut b).await;
    }

    #[tokio::test]
    async fn build_listens_on_all_provided_addresses() {
        let identity = PeerIdentity::from_seed(&[6u8; 32]);
        let mut swarm = build_swarm(
            &identity,
            SwarmConfig {
                listen_addresses: vec![
                    "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
                    "/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap(),
                ],
                agent_version: "test/0".into(),
            },
        )
        .unwrap();

        // Two listen addresses configured; expect at least two
        // NewListenAddr events before timing out.
        let mut seen = 0;
        tokio::time::timeout(Duration::from_secs(5), async {
            while seen < 2 {
                if let Some(SwarmEvent::NewListenAddr { .. }) = swarm.next().await {
                    seen += 1;
                }
            }
        })
        .await
        .expect("did not see both NewListenAddr events");
        assert_eq!(seen, 2);
    }
}
