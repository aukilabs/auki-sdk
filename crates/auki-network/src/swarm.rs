//! libp2p `Swarm` builder. Turns a [`PeerIdentity`] and a
//! [`SwarmConfig`] into a configured `libp2p::Swarm<Behaviour>`.
//!
//! Transport: TCP + QUIC + Circuit Relay v2 client (always wired into
//! the transport stack). Both authenticated with Noise (using the peer's
//! ed25519 keypair) and multiplexed with Yamux. Behaviour: `identify` +
//! `ping` always; `mdns` optional (on by default — Reid M1b dual-channel
//! decision); `relay::client::Behaviour` always; `relay::Behaviour` (the
//! server side) optional (off by default for consumer daemons; on for
//! the dedicated `aukilabs/relay` infrastructure node).
//!
//! ## Reid milestone-2 architectural commitments encoded here
//!
//! - **Dual-channel mDNS** (parking-lot 1a, 2026-05-02). This swarm
//!   advertises on `_p2p._udp.local.` via libp2p mDNS when
//!   `enable_mdns = true`. Daemons keep their existing
//!   `_auki._tcp.local.` advertisement separately (control-API
//!   discovery, unchanged).
//! - **Both-gates relay-server** (parking-lot 2c, 2026-05-02). The
//!   boolean [`SwarmConfig::enable_relay_server`] gates the libp2p
//!   relay server behaviour. Consumers gate `Capability::*`
//!   advertisement on a `ReachabilityRecord` independently — both must
//!   line up for the peer to actually serve the capability.
//! - **Manual peer-id paste for Park-from-home** (parking-lot 3c,
//!   2026-05-02). [`dial_peer`] takes a `PeerId` plus a list of
//!   multiaddrs (which may be circuit-relay-mediated like
//!   `/p2p/<relay>/p2p-circuit/p2p/<target>`); operator obtains the
//!   peer-id and relay multiaddr out-of-band and pastes them into Park.
//!   No Discovery Service dependency for the Reid M2 demo.
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
//!     enable_mdns: true,
//!     enable_relay_server: false,
//! }).expect("build swarm");
//! ```

use crate::{PeerIdentity, cluster_protocol};
use libp2p::{
    Multiaddr, PeerId, Swarm, SwarmBuilder, identify, mdns, noise, ping, relay,
    swarm::{
        DialError, NetworkBehaviour, behaviour::toggle::Toggle, dial_opts::DialOpts,
    },
    tcp, yamux,
};
use std::time::Duration;

/// libp2p protocol id used for the SDK's identify exchanges. Stable; do
/// not change without coordinating with consumers (the agent_version
/// string is the per-deployment knob; the protocol id is the wire format).
pub const IDENTIFY_PROTOCOL: &str = "/auki/identify/0.0.1";

/// Idle connection timeout. libp2p closes a connection that's been idle
/// for this long; ping resets the idle timer, so this only fires if the
/// remote stops responding.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Composed behaviour for the M1b swarm. Generated `BehaviourEvent` enum
/// has one variant per field.
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    /// `_p2p._udp.local.` advertisement + LAN peer discovery. Toggleable
    /// via [`SwarmConfig::enable_mdns`] — on by default for daemons; off
    /// in tests to avoid LAN noise.
    pub mdns: Toggle<mdns::tokio::Behaviour>,
    /// Always present: lets this node act as a relay-*client* (use
    /// another peer's relay-server to traverse NAT). Wiring is automatic
    /// — dial a circuit-relay multiaddr and the relay-client behaviour
    /// handles the routing.
    pub relay_client: relay::client::Behaviour,
    /// Optional: lets this node act as a relay-*server* (forward
    /// circuit-relay traffic on behalf of other peers). Toggleable via
    /// [`SwarmConfig::enable_relay_server`] — off by default for
    /// consumer daemons (BoosterApp, Sentinel); on for the dedicated
    /// `aukilabs/relay` infrastructure node.
    pub relay: Toggle<relay::Behaviour>,
    /// `/auki/cluster/0.0.1` participant exchange. Always present — the
    /// protocol sits idle for swarms that don't participate in a cluster
    /// (the dedicated `aukilabs/relay` infrastructure node), so a knob
    /// would just be ceremony. The behaviour does not auto-respond:
    /// receivers handle `Request` events themselves and call
    /// [`cluster_protocol::Behaviour::send_response`]. See
    /// [`crate::cluster_protocol`].
    pub cluster: cluster_protocol::Behaviour,
    /// libp2p raw-substream multiplexer used by grimsby's
    /// `/auki/stream/0.1.0` typed-byte-stream protocol. Always present —
    /// the behaviour sits idle for swarms that don't open or accept any
    /// streams; a knob would just be ceremony. Bind to a specific
    /// protocol id (typically [`crate::stream_protocol::STREAM_PROTOCOL`])
    /// on the receiving side via `libp2p_stream::Control::accept`, or open
    /// outbound via `libp2p_stream::Control::open_stream`. See
    /// [`crate::stream_protocol`].
    pub stream: libp2p_stream::Behaviour,
}

/// Per-swarm configuration.
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
    /// Enable libp2p mDNS (`_p2p._udp.local.`) for LAN peer discovery.
    /// On for daemons by default; off in tests to avoid cross-test
    /// interference. Daemons' existing `_auki._tcp.local.` advertisement
    /// is unaffected (control-API discovery is separate).
    pub enable_mdns: bool,
    /// Enable the libp2p relay-*server* behaviour so this node forwards
    /// circuit-relay traffic on behalf of other peers. Off for consumer
    /// daemons (BoosterApp, Sentinel) by default. The dedicated
    /// `aukilabs/relay` app sets this `true`. The `Capability::*`
    /// strings advertised in a `ReachabilityRecord` are independent —
    /// both must line up for the peer to actually serve the capability.
    pub enable_relay_server: bool,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            listen_addresses: vec![],
            agent_version: format!("auki-sdk/{}", env!("CARGO_PKG_VERSION")),
            enable_mdns: true,
            enable_relay_server: false,
        }
    }
}

/// Errors from [`build_swarm`].
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// Transport stack failed to assemble (TCP/Noise/Yamux, QUIC, or
    /// the relay-client transport upgrade).
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
    let enable_relay_server = config.enable_relay_server;
    let local_pid = identity.peer_id();

    // Construct mDNS outside the builder closure so we can surface a
    // proper BuildError; the closure can only return Behaviour or
    // Result<Behaviour, Box<dyn Error>>.
    let mdns_b = if config.enable_mdns {
        Some(
            mdns::tokio::Behaviour::new(mdns::Config::default(), local_pid)
                .map_err(|e| BuildError::Transport(format!("mdns: {e}")))?,
        )
    } else {
        None
    };

    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| BuildError::Transport(format!("tcp: {e}")))?
        .with_quic()
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| BuildError::Transport(format!("relay_client: {e}")))?
        .with_behaviour(|key, relay_client| Behaviour {
            identify: identify::Behaviour::new(
                identify::Config::new(IDENTIFY_PROTOCOL.into(), key.public())
                    .with_agent_version(agent_version),
            ),
            ping: ping::Behaviour::default(),
            mdns: Toggle::from(mdns_b),
            relay_client,
            relay: Toggle::from(enable_relay_server.then(|| {
                relay::Behaviour::new(local_pid, relay::Config::default())
            })),
            cluster: cluster_protocol::behaviour(),
            stream: libp2p_stream::Behaviour::new(),
        })
        .expect("behaviour construction is infallible")
        .with_swarm_config(|c| c.with_idle_connection_timeout(IDLE_TIMEOUT))
        .build();

    for addr in config.listen_addresses {
        swarm
            .listen_on(addr.clone())
            .map_err(|source| BuildError::Listen { addr, source })?;
    }

    Ok(swarm)
}

/// Dial a peer through one or more multiaddrs. Multiaddrs may be direct
/// (e.g. `/ip4/.../tcp/...`, `/ip4/.../udp/.../quic-v1`) or
/// circuit-relay-mediated (`/p2p/<relay>/p2p-circuit/p2p/<target>`).
///
/// Park-from-home (Reid parking-lot 3c, 2026-05-02): the operator
/// pastes the daemon's peer-id and a relay multiaddr into Park's UI.
/// Park calls this helper with both. The swarm's `relay_client`
/// behaviour handles the routing; consumer doesn't need to know whether
/// the connection is direct or relayed.
pub fn dial_peer(
    swarm: &mut Swarm<Behaviour>,
    peer: PeerId,
    addresses: Vec<Multiaddr>,
) -> Result<(), DialError> {
    let opts = DialOpts::peer_id(peer).addresses(addresses).build();
    swarm.dial(opts)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PeerIdentity;
    use futures::StreamExt;
    use libp2p::swarm::SwarmEvent;

    /// Build config used by most tests — mDNS off (no LAN noise),
    /// relay-server off, listening on a single OS-chosen TCP port.
    fn test_tcp_config(agent_version: &str) -> SwarmConfig {
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: agent_version.into(),
            enable_mdns: false,
            enable_relay_server: false,
        }
    }

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
                ..test_tcp_config("test/0")
            },
        )
        .unwrap();
        assert_eq!(*swarm.local_peer_id(), identity.peer_id());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_peers_identify_each_other_over_tcp() {
        let id_a = PeerIdentity::from_seed(&[2u8; 32]);
        let id_b = PeerIdentity::from_seed(&[3u8; 32]);

        let mut a = build_swarm(&id_a, test_tcp_config("test-a/0")).unwrap();
        let mut b = build_swarm(&id_b, test_tcp_config("test-b/0")).unwrap();

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
                ..test_tcp_config("test-a/0")
            },
        )
        .unwrap();
        let mut b = build_swarm(
            &id_b,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
                ..test_tcp_config("test-b/0")
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
                ..test_tcp_config("test/0")
            },
        )
        .unwrap();

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

    #[tokio::test]
    async fn build_with_mdns_enabled_succeeds() {
        // Construction-only test. Real mDNS discovery requires a network
        // interface that carries multicast (loopback typically does not);
        // verifying actual discovery on a developer machine and CI is
        // brittle. Daemon-level integration verifies cross-LAN behaviour.
        let identity = PeerIdentity::from_seed(&[7u8; 32]);
        let _swarm = build_swarm(
            &identity,
            SwarmConfig {
                listen_addresses: vec![],
                agent_version: "test/0".into(),
                enable_mdns: true,
                enable_relay_server: false,
            },
        )
        .expect("build with mdns enabled");
    }

    #[tokio::test]
    async fn build_with_relay_server_enabled_succeeds() {
        // Construction-only test. The substantive relay test below
        // verifies the server actually accepts a reservation.
        let identity = PeerIdentity::from_seed(&[8u8; 32]);
        let _swarm = build_swarm(
            &identity,
            SwarmConfig {
                listen_addresses: vec![],
                agent_version: "relay/0".into(),
                enable_mdns: false,
                enable_relay_server: true,
            },
        )
        .expect("build with relay server enabled");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn relay_server_accepts_reservation() {
        let id_relay = PeerIdentity::from_seed(&[10u8; 32]);
        let id_client = PeerIdentity::from_seed(&[11u8; 32]);

        let mut relay_swarm = build_swarm(
            &id_relay,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                agent_version: "relay/0".into(),
                enable_mdns: false,
                enable_relay_server: true,
            },
        )
        .unwrap();

        let mut client = build_swarm(
            &id_client,
            SwarmConfig {
                listen_addresses: vec![],
                agent_version: "client/0".into(),
                enable_mdns: false,
                enable_relay_server: false,
            },
        )
        .unwrap();

        let relay_addr = wait_for_listen_addr(&mut relay_swarm).await;
        // Loopback test: tell the swarm its listen address is also an
        // external address so the relay-server includes it in
        // reservation responses. On real networks this happens via
        // identify (the client tells the relay what address it dialed)
        // or AutoNAT.
        relay_swarm.add_external_address(relay_addr.clone());
        let relay_addr_with_pid =
            relay_addr.with(libp2p::multiaddr::Protocol::P2p(*relay_swarm.local_peer_id()));

        // Establish a regular connection to the relay first; the relay
        // only accepts reservations from peers that have identified
        // themselves (so it knows they support being relayed).
        client
            .dial(relay_addr_with_pid.clone())
            .expect("dial relay");

        // Wait until both sides complete an identify exchange.
        let identified = tokio::time::timeout(Duration::from_secs(15), async {
            let mut relay_saw_client = false;
            let mut client_saw_relay = false;
            while !(relay_saw_client && client_saw_relay) {
                tokio::select! {
                    Some(event) = relay_swarm.next() => {
                        if let SwarmEvent::Behaviour(BehaviourEvent::Identify(
                            identify::Event::Received { .. }
                        )) = event { relay_saw_client = true; }
                    }
                    Some(event) = client.next() => {
                        if let SwarmEvent::Behaviour(BehaviourEvent::Identify(
                            identify::Event::Received { .. }
                        )) = event { client_saw_relay = true; }
                    }
                }
            }
        })
        .await;
        assert!(
            identified.is_ok(),
            "identify between relay and client did not complete"
        );

        // Now listening on the circuit address triggers a reservation
        // request via the existing connection.
        let circuit_listen_addr =
            relay_addr_with_pid.with(libp2p::multiaddr::Protocol::P2pCircuit);
        client
            .listen_on(circuit_listen_addr)
            .expect("listen on circuit");

        let result = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                tokio::select! {
                    _ = relay_swarm.next() => {}
                    event = client.next() => {
                        if let Some(SwarmEvent::Behaviour(BehaviourEvent::RelayClient(
                            relay::client::Event::ReservationReqAccepted { .. }
                        ))) = event {
                            return;
                        }
                    }
                }
            }
        })
        .await;
        assert!(result.is_ok(), "relay reservation did not complete in time");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dial_peer_helper_dials_direct_address() {
        let id_a = PeerIdentity::from_seed(&[12u8; 32]);
        let id_b = PeerIdentity::from_seed(&[13u8; 32]);

        let mut a = build_swarm(&id_a, test_tcp_config("test-a/0")).unwrap();
        let mut b = build_swarm(&id_b, test_tcp_config("test-b/0")).unwrap();

        let addr_a = wait_for_listen_addr(&mut a).await;
        dial_peer(&mut b, id_a.peer_id(), vec![addr_a]).expect("dial via helper");
        run_until_mutual_identify(&mut a, &mut b).await;
    }
}
