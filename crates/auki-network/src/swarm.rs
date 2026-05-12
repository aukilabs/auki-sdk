//! libp2p `Swarm` builder. Turns a [`PeerIdentity`] and a
//! [`SwarmConfig`] into a configured `libp2p::Swarm<Behaviour>`.
//!
//! Transport: TCP + QUIC + Circuit Relay v2 client (always wired into
//! the transport stack). Both authenticated with Noise (using the peer's
//! ed25519 keypair) and multiplexed with Yamux. Behaviour: `identify` +
//! `ping` always; `allow_list` (cluster trust boundary, populated by
//! [`crate::cluster_runtime`] from `ClusterDoc.peers`) always;
//! `relay::client::Behaviour` always; `relay::Behaviour` (the server
//! side) optional (off by default for consumer daemons; on for the
//! dedicated `aukilabs/relay` infrastructure node).
//!
//! ## Cluster trust boundary at the libp2p layer
//!
//! `allow_list` is `libp2p-allow-block-list::Behaviour<AllowedPeers>`.
//! The list starts empty at swarm-build time and is populated by
//! [`crate::cluster_runtime`] from `ClusterDoc.peers` on spawn and on
//! every Discovery-SSE update. Inbound and outbound connections from
//! non-listed peer-ids are denied at the libp2p `NetworkBehaviour`
//! layer — `handle_pending_inbound_connection` /
//! `handle_pending_outbound_connection` refuse before the noise
//! handshake runs. From an outsider's side we look like we don't
//! exist: no `identify` exchange, no protocol handler ever fires.
//!
//! This is "peers only visible within their cluster" — single
//! enforcement point covering every libp2p protocol on the swarm
//! (`/auki/cluster/0.0.1`, `/auki/heartbeat/0.0.1`,
//! `/auki/registry/0.0.1`, `/auki/stream/0.1.0`, anything future).
//! mDNS deliberately removed — peers don't auto-discover across
//! clusters because they shouldn't be able to.
//!
//! ## Reid milestone-2 architectural commitments encoded here
//!
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
//!   The target peer-id must be in the allow-list (i.e. in the local
//!   `ClusterDoc.peers`) or the outbound dial is refused at the
//!   handshake layer.
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
//!     enable_relay_server: false,
//! }).expect("build swarm");
//! ```

use crate::{PeerIdentity, cluster_protocol, heartbeat_protocol};
use libp2p::{
    Multiaddr, PeerId, Swarm, SwarmBuilder, identify, noise, ping, relay,
    swarm::{
        DialError, NetworkBehaviour, behaviour::toggle::Toggle, dial_opts::DialOpts,
    },
    tcp, yamux,
};
use libp2p_allow_block_list as allow_block_list;
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
    /// Cluster trust boundary. Inbound and outbound connections from
    /// peers NOT in this allow-list are denied at the libp2p
    /// `NetworkBehaviour` layer — the noise handshake never completes,
    /// no protocol handler fires, no `identify` exchange leaks our
    /// peer-id. Populated by [`crate::cluster_runtime`] from
    /// `ClusterDoc.peers` on spawn and on every Discovery-SSE update;
    /// empty at swarm-build time. See the module-level docs for the
    /// "peers only visible within their cluster" rationale.
    pub allow_list: allow_block_list::Behaviour<allow_block_list::AllowedPeers>,
    /// Always present: lets this node act as a relay-*client* (use
    /// another peer's relay-server to traverse NAT). Wiring is automatic
    /// — dial a circuit-relay multiaddr and the relay-client behaviour
    /// handles the routing. The dial target must still be in the
    /// [`allow_list`](Self::allow_list) (i.e. in the local cluster doc)
    /// or the outbound dial is refused before the relay-client handshake.
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
    /// `/auki/heartbeat/0.0.1` Manager↔member liveness exchange.
    /// Always present — the protocol sits idle until the Manager-side
    /// state machine in [`auki-domain`](../../../auki-domain) starts
    /// ticking. Same shape as [`cluster_protocol`]: the behaviour
    /// does not auto-respond; receivers handle `Request` events and
    /// call `send_response`. See [`crate::heartbeat_protocol`].
    pub heartbeat: heartbeat_protocol::Behaviour,
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
            // Empty at build time. `cluster_runtime` populates from
            // `ClusterDoc.peers` on spawn and on every Discovery-SSE
            // update via `swarm.behaviour_mut().allow_list.allow_peer(p)`
            // / `disallow_peer(p)`. With an empty list, the swarm refuses
            // every inbound and outbound libp2p handshake — a
            // freshly-built swarm with no doc is invisible by design.
            allow_list: allow_block_list::Behaviour::<allow_block_list::AllowedPeers>::default(),
            relay_client,
            relay: Toggle::from(enable_relay_server.then(|| {
                relay::Behaviour::new(local_pid, relay::Config::default())
            })),
            cluster: cluster_protocol::behaviour(),
            heartbeat: heartbeat_protocol::behaviour(),
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

    /// Build config used by most tests — relay-server off, listening on
    /// a single OS-chosen TCP port. Tests that exercise peer-to-peer
    /// interaction must additionally call [`allow_pair`] to populate
    /// each side's allow-list with the other's peer-id; an empty
    /// allow-list refuses every connection (the production "invisible
    /// outside the cluster" default).
    fn test_tcp_config(agent_version: &str) -> SwarmConfig {
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: agent_version.into(),
            enable_relay_server: false,
        }
    }

    /// Mutually allow-list two test swarms so they can complete a
    /// libp2p handshake. Mirrors what `cluster_runtime` would do in
    /// production on every `ClusterDoc.peers` update — production
    /// callers don't poke `allow_list` directly.
    fn allow_pair(a: &mut Swarm<Behaviour>, b: &mut Swarm<Behaviour>) {
        let a_pid = *a.local_peer_id();
        let b_pid = *b.local_peer_id();
        a.behaviour_mut().allow_list.allow_peer(b_pid);
        b.behaviour_mut().allow_list.allow_peer(a_pid);
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
        allow_pair(&mut a, &mut b);

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
        allow_pair(&mut a, &mut b);

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
    async fn freshly_built_swarm_has_empty_allow_list() {
        // The "peers only visible within their cluster" default — a
        // swarm built without any allow_list calls accepts no
        // libp2p handshakes. `cluster_runtime` populates the list
        // from `ClusterDoc.peers` on spawn; until then, the swarm is
        // dark by design.
        let identity = PeerIdentity::from_seed(&[7u8; 32]);
        let swarm = build_swarm(
            &identity,
            SwarmConfig {
                listen_addresses: vec![],
                agent_version: "test/0".into(),
                enable_relay_server: false,
            },
        )
        .expect("build");
        // No public read accessor; smoke-test that we built the swarm
        // and that `local_peer_id` returned the expected pid. The
        // allow_list emptiness is exercised end-to-end by
        // `outsider_dial_is_refused` below.
        assert_eq!(*swarm.local_peer_id(), identity.peer_id());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn outsider_dial_is_refused_when_allow_list_does_not_include_peer() {
        // The headline trust-boundary guarantee. A built two-swarm
        // pair where neither side has the other in its allow-list:
        // the dial completes the transport handshake but the libp2p
        // behaviour layer refuses the connection before any
        // protocol-level event (identify, our cluster protocols) can
        // fire. Asserted by NOT observing an Identify::Received in a
        // bounded window.
        let id_a = PeerIdentity::from_seed(&[20u8; 32]);
        let id_b = PeerIdentity::from_seed(&[21u8; 32]);

        let mut a = build_swarm(&id_a, test_tcp_config("test-a/0")).unwrap();
        let mut b = build_swarm(&id_b, test_tcp_config("test-b/0")).unwrap();
        // Deliberately NO `allow_pair` call — both allow-lists empty.

        let addr_a = wait_for_listen_addr(&mut a).await;
        let _ = b.dial(addr_a);

        let identified = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                tokio::select! {
                    Some(event) = a.next() => {
                        if matches!(
                            event,
                            SwarmEvent::Behaviour(BehaviourEvent::Identify(
                                identify::Event::Received { .. }
                            ))
                        ) {
                            return true;
                        }
                    }
                    Some(event) = b.next() => {
                        if matches!(
                            event,
                            SwarmEvent::Behaviour(BehaviourEvent::Identify(
                                identify::Event::Received { .. }
                            ))
                        ) {
                            return true;
                        }
                    }
                }
            }
        })
        .await;
        assert!(
            identified.is_err(),
            "identify exchange completed despite allow-list refusing the connection",
        );
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
                enable_relay_server: true,
            },
        )
        .unwrap();

        let mut client = build_swarm(
            &id_client,
            SwarmConfig {
                listen_addresses: vec![],
                agent_version: "client/0".into(),
                enable_relay_server: false,
            },
        )
        .unwrap();
        // Mutually allow-list — production `cluster_runtime` would do
        // this from `ClusterDoc.peers` on spawn; in this test we wire
        // it manually since there's no runtime.
        allow_pair(&mut relay_swarm, &mut client);

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
        allow_pair(&mut a, &mut b);

        let addr_a = wait_for_listen_addr(&mut a).await;
        dial_peer(&mut b, id_a.peer_id(), vec![addr_a]).expect("dial via helper");
        run_until_mutual_identify(&mut a, &mut b).await;
    }
}
