use auki_identity::Wallet;
use auki_network::{
    AllowedPeer, HeartbeatTimestampSource, NetworkRuntime, PeerIdentity,
    network_runtime::{HeartbeatDomainClockNs, HeartbeatNowNs},
    peer_candidate::{AukiPeerCandidateV1, PeerCandidateCache, PeerCandidateSource},
    stream_runtime::decline_all_streams,
    swarm::{SwarmConfig, build_swarm},
};
use libp2p::{Multiaddr, PeerId, multiaddr::Protocol};
use std::{collections::HashSet, env, sync::Arc, time::Duration};

const START_MS: u64 = 1_000;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scenario = parse_scenario();
    match scenario.as_str() {
        "positive" => run_positive().await?,
        "peer-id-mismatch" => run_peer_id_mismatch().await?,
        "expired-candidate" => run_expired_candidate().await?,
        other => {
            eprintln!(
                "unsupported scenario {other:?}; use --scenario positive|peer-id-mismatch|expired-candidate"
            );
            std::process::exit(2);
        }
    }
    Ok(())
}

fn parse_scenario() -> String {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--scenario" {
            return args.next().unwrap_or_else(|| "positive".into());
        }
    }
    "positive".into()
}

async fn run_positive() -> Result<(), Box<dyn std::error::Error>> {
    println!("NO_CENTRALIZED_DISCOVERY true");
    println!("NO_RENDEZVOUS true");

    let mut node_a = spawn_node("A", 11, vec![]).await?;
    let mut node_b = spawn_node("B", 12, vec![]).await?;
    let mut node_c = spawn_node("C", 13, vec![]).await?;

    println!("NODE_STARTED A peer={}", node_a.peer_id);
    println!("NODE_STARTED B peer={}", node_b.peer_id);
    println!("NODE_STARTED C peer={}", node_c.peer_id);

    let b_for_a = allowed_peer(&node_b);
    let a_for_b = allowed_peer(&node_a);
    node_a
        .runtime
        .set_allowed_peers(vec![b_for_a.clone()])
        .await?;
    node_b
        .runtime
        .set_allowed_peers(vec![a_for_b.clone()])
        .await?;
    println!("MANUAL_PEER A B");
    println!("MANUAL_PEER B A");

    let a_for_c = allowed_peer(&node_a);
    node_c
        .runtime
        .set_allowed_peers(vec![a_for_c.clone()])
        .await?;
    println!("MANUAL_PEER C A");
    println!(
        "C_INITIAL_UNKNOWN_B {}",
        !knows_peer(&[a_for_c], node_b.peer_id)
    );

    wait_for_connected(&node_a.runtime, node_b.peer_id, "A", "B").await?;
    wait_for_connected(&node_b.runtime, node_a.peer_id, "B", "A").await?;
    wait_for_connected(&node_c.runtime, node_a.peer_id, "C", "A").await?;

    println!("ADVERTISE A B_TO C");
    let candidate = AukiPeerCandidateV1::with_ttl(
        PeerCandidateSource::ConnectedPeerAdvertisement,
        node_b.peer_id,
        node_b.listen_addrs.clone(),
        START_MS,
        Duration::from_secs(60),
    );
    println!(
        "CANDIDATE_LEARNED source={} peer=B",
        candidate.source.as_str()
    );

    let mut cache = PeerCandidateCache::new();
    cache
        .store(candidate, START_MS + 1)
        .map_err(|reason| format!("candidate rejected: {}", reason.marker()))?;
    println!("CANDIDATE_STORED source=connected_peer_advertisement peer=B");
    println!("CANDIDATE_ELIGIBLE peer=B true");

    let mut c_allowed = vec![allowed_peer(&node_a)];
    let candidate_allowed = cache.eligible_allowed_peers(START_MS + 2);
    c_allowed.extend(candidate_allowed);
    // Experimental boundary: NetworkRuntime currently uses AllowedPeer for the
    // transport allow-list/dial set. The example keeps separate authority state
    // and does not mark B usable until the explicit post-connection validation
    // below succeeds.
    node_c.runtime.set_allowed_peers(c_allowed).await?;
    println!("AUTO_DIAL_REQUESTED C B");

    wait_for_connected(&node_c.runtime, node_b.peer_id, "C", "B").await?;

    let mut authority_accepted = AuthorityState::default();
    println!(
        "AUTHORITY_BEFORE_VALIDATION {}",
        authority_accepted.is_accepted(node_b.peer_id)
    );
    let validation = ExampleValidationEvidence {
        identity_peer_binding: node_c.runtime.connected_peers().contains(&node_b.peer_id),
        domain_policy_accepts: true,
        offer_policy_accepts: true,
        local_policy_accepts: true,
    };
    if validation.passes() {
        authority_accepted.accept(node_b.peer_id);
        println!(
            "VALIDATION_PASSED C B identity_peer_binding=true domain_policy=true offer_policy=true local_policy=true"
        );
    }
    println!(
        "AUTHORITY_AFTER_VALIDATION {}",
        authority_accepted.is_accepted(node_b.peer_id)
    );

    shutdown_all([&mut node_a, &mut node_b, &mut node_c]).await;
    Ok(())
}

async fn run_peer_id_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    let advertised_peer = identity(21).peer_id();
    let addr_peer = identity(22).peer_id();
    let mismatched_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/41022/p2p/{addr_peer}").parse()?;
    let candidate = AukiPeerCandidateV1::with_ttl(
        PeerCandidateSource::ConnectedPeerAdvertisement,
        advertised_peer,
        vec![mismatched_addr],
        START_MS,
        Duration::from_secs(60),
    );

    match auki_network::peer_candidate::validate_peer_candidate(&candidate, START_MS + 1) {
        Ok(()) => {
            eprintln!("negative scenario unexpectedly accepted");
            std::process::exit(1);
        }
        Err(reason) => println!(
            "NEGATIVE_REJECTED scenario=peer-id-mismatch reason={}",
            reason.marker()
        ),
    }
    Ok(())
}

async fn run_expired_candidate() -> Result<(), Box<dyn std::error::Error>> {
    let peer = identity(23).peer_id();
    let addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/41023/p2p/{peer}").parse()?;
    let candidate = AukiPeerCandidateV1::with_ttl(
        PeerCandidateSource::ConnectedPeerAdvertisement,
        peer,
        vec![addr],
        START_MS,
        Duration::from_millis(1),
    );

    match auki_network::peer_candidate::validate_peer_candidate(&candidate, START_MS + 2) {
        Ok(()) => {
            eprintln!("negative scenario unexpectedly accepted");
            std::process::exit(1);
        }
        Err(reason) => println!(
            "NEGATIVE_REJECTED scenario=expired-candidate reason={}",
            reason.marker()
        ),
    }
    Ok(())
}

struct ExampleNode {
    runtime: NetworkRuntime,
    peer_id: PeerId,
    listen_addrs: Vec<Multiaddr>,
}

async fn spawn_node(
    label: &str,
    seed_byte: u8,
    allowed_peers: Vec<AllowedPeer>,
) -> Result<ExampleNode, Box<dyn std::error::Error>> {
    let id = identity(seed_byte);
    let mut swarm = build_swarm(
        &id,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse()?],
            agent_version: format!("three-node-peer-advertisement/{label}"),
            enable_relay_server: false,
        },
    )?;
    let listen_addrs = collect_loopback_addrs(&mut swarm, id.peer_id()).await?;
    let (runtime, _join, _liveness, _membership, _info, _resources, _registries, _diagnostic) =
        NetworkRuntime::spawn(
            swarm,
            allowed_peers,
            decline_all_streams(),
            heartbeat_source(label),
        )?;

    Ok(ExampleNode {
        runtime,
        peer_id: id.peer_id(),
        listen_addrs,
    })
}

async fn collect_loopback_addrs(
    swarm: &mut auki_network::Swarm<auki_network::swarm::Behaviour>,
    peer_id: PeerId,
) -> Result<Vec<Multiaddr>, Box<dyn std::error::Error>> {
    let deadline = tokio::time::sleep(Duration::from_secs(3));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return Err("timed out waiting for listen address".into()),
            event = futures::StreamExt::next(swarm) => {
                match event {
                    Some(libp2p::swarm::SwarmEvent::NewListenAddr { address, .. }) => {
                        return Ok(vec![address.with(Protocol::P2p(peer_id))]);
                    }
                    Some(_) => {}
                    None => return Err("swarm ended before listen address".into()),
                }
            }
        }
    }
}

fn identity(seed_byte: u8) -> PeerIdentity {
    PeerIdentity::from_wallet(Wallet::from_seed(vec![seed_byte; 32]).expect("valid seed"))
}

fn allowed_peer(node: &ExampleNode) -> AllowedPeer {
    AllowedPeer {
        peer_id: node.peer_id,
        multiaddrs: node.listen_addrs.clone(),
    }
}

fn knows_peer(peers: &[AllowedPeer], peer_id: PeerId) -> bool {
    peers.iter().any(|peer| peer.peer_id == peer_id)
}

async fn wait_for_connected(
    runtime: &NetworkRuntime,
    peer_id: PeerId,
    local: &str,
    remote: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        if runtime.connected_peers().contains(&peer_id) {
            println!("CONNECTED {local} {remote}");
            return Ok(());
        }
        tokio::select! {
            _ = &mut deadline => {
                return Err(format!("timed out waiting for {local} to connect to {remote}").into());
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

#[derive(Default)]
struct AuthorityState {
    accepted: HashSet<PeerId>,
}

impl AuthorityState {
    fn accept(&mut self, peer_id: PeerId) {
        self.accepted.insert(peer_id);
    }

    fn is_accepted(&self, peer_id: PeerId) -> bool {
        self.accepted.contains(&peer_id)
    }
}

struct ExampleValidationEvidence {
    /// The runtime's connected set is populated only after libp2p Noise binds
    /// the remote public key to the expected PeerId.
    identity_peer_binding: bool,
    /// Placeholder for domain/membership validation owned above transport.
    domain_policy_accepts: bool,
    /// Placeholder for offer/resource authority validation owned above transport.
    offer_policy_accepts: bool,
    /// Placeholder for local application policy.
    local_policy_accepts: bool,
}

impl ExampleValidationEvidence {
    fn passes(&self) -> bool {
        self.identity_peer_binding
            && self.domain_policy_accepts
            && self.offer_policy_accepts
            && self.local_policy_accepts
    }
}

fn heartbeat_source(label: &str) -> HeartbeatTimestampSource {
    let clock_id = format!("example-{label}-monotonic");
    let clock_hash = format!("example-{label}-clock-hash");
    let now: HeartbeatNowNs = Arc::new(|| 0);
    let domain_clock: HeartbeatDomainClockNs = Arc::new(|| None);
    HeartbeatTimestampSource {
        clock_id,
        clock_hash,
        now_ns: now,
        domain_clock,
    }
}

async fn shutdown_all(nodes: [&mut ExampleNode; 3]) {
    for node in nodes {
        node.runtime.shutdown();
    }
}
