//! Optional relay circuit addresses before Discovery registration.
//!
//! When the local peer has no circuit-relay listen address yet, we wait
//! briefly for AutoNAT v2 (or another behaviour) to confirm a dialable
//! direct surface. If that never succeeds within the window, we query
//! Discovery for infrastructure `relay` nodes, dial one, open a circuit
//! reservation, and merge the resulting `/p2p/.../p2p-circuit/...`
//! listen addrs into the set passed to Discovery / the join handshake.
//!
//! ## Timing overrides (integration tests / tuning)
//!
//! When `auki-network` is linked as a dependency, `cfg(test)` defaults do
//! not apply. Override wall-clock budgets with (seconds, clamped 1–120):
//! - `AUKI_RELAY_PREPARE_AUTONAT_SECS` (default 12 release, 2 in unit tests)
//! - `AUKI_RELAY_PREPARE_RELAY_IDENT_SECS` (default 12 release, 10 in unit tests)
//! - `AUKI_RELAY_PREPARE_RELAY_PHASE_SECS` (default 18 release, 12 in unit tests)

use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt as _;
use libp2p::{Multiaddr, PeerId, Swarm, identify, relay, swarm::SwarmEvent};
use multiaddr::Protocol;

use crate::discovery_client::DiscoveryClient;
use crate::swarm::{Behaviour, BehaviourEvent, dial_peer, is_routable_multiaddr};

fn relay_prepare_secs_from_env(key: &str, default_secs: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(default_secs)
        .clamp(1, 120)
}

/// How long we poll for an AutoNAT success (`result == Ok(())`) or an
/// [`SwarmEvent::ExternalAddrConfirmed`] on a routable address before
/// deciding direct reachability was not established in time.
fn autonat_wait_duration() -> Duration {
    let default_secs = if cfg!(test) { 2 } else { 12 };
    Duration::from_secs(relay_prepare_secs_from_env(
        "AUKI_RELAY_PREPARE_AUTONAT_SECS",
        default_secs,
    ))
}

/// Budget for waiting on identify with a chosen relay before
/// `listen_on` circuit.
fn relay_ident_wait_duration() -> Duration {
    let default_secs = if cfg!(test) { 10 } else { 12 };
    Duration::from_secs(relay_prepare_secs_from_env(
        "AUKI_RELAY_PREPARE_RELAY_IDENT_SECS",
        default_secs,
    ))
}

/// Budget for reservation + `NewListenAddr` after `listen_on` circuit.
fn relay_reservation_phase_duration() -> Duration {
    let default_secs = if cfg!(test) { 12 } else { 18 };
    Duration::from_secs(relay_prepare_secs_from_env(
        "AUKI_RELAY_PREPARE_RELAY_PHASE_SECS",
        default_secs,
    ))
}

fn addrs_contain_circuit(addrs: &[Multiaddr]) -> bool {
    addrs
        .iter()
        .any(|a| a.iter().any(|p| matches!(p, Protocol::P2pCircuit)))
}

/// Returns `true` when we should skip relay acquisition — AutoNAT
/// reported success for at least one candidate, or the swarm confirmed
/// an externally-routable listen surface.
async fn poll_direct_reachability_confirmed(swarm: &mut Swarm<Behaviour>) -> bool {
    tokio::time::timeout(autonat_wait_duration(), async {
        while let Some(ev) = swarm.next().await {
            match ev {
                SwarmEvent::Behaviour(BehaviourEvent::AutonatClient(e)) if e.result.is_ok() => {
                    return true;
                }
                SwarmEvent::ExternalAddrConfirmed { address } => {
                    if is_routable_multiaddr(&address) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}

fn ends_with_peer(addr: &Multiaddr, peer: &PeerId) -> bool {
    matches!(addr.iter().last(), Some(Protocol::P2p(p)) if p == *peer)
}

async fn try_reserve_on_relay(
    swarm: &mut Swarm<Behaviour>,
    relay_peer: PeerId,
    relay_raw_addrs: &[String],
) -> Vec<Multiaddr> {
    let mut dial_addrs: Vec<Multiaddr> = Vec::new();
    for s in relay_raw_addrs {
        match Multiaddr::from_str(s) {
            Ok(mut a) => {
                if !ends_with_peer(&a, &relay_peer) {
                    a = a.with(Protocol::P2p(relay_peer));
                }
                dial_addrs.push(a);
            }
            Err(e) => {
                eprintln!("auki-network: skip invalid relay multiaddr {s:?}: {e}");
            }
        }
    }
    if dial_addrs.is_empty() {
        return Vec::new();
    }

    if dial_peer(swarm, relay_peer, dial_addrs.clone()).is_err() {
        eprintln!("auki-network: dial_peer relay {relay_peer} failed");
        return Vec::new();
    }

    let ident_deadline = tokio::time::Instant::now() + relay_ident_wait_duration();
    let mut identified = false;
    while tokio::time::Instant::now() < ident_deadline {
        match tokio::time::timeout(Duration::from_millis(200), swarm.next()).await {
            Ok(Some(SwarmEvent::Behaviour(BehaviourEvent::Identify(
                identify::Event::Received { peer_id, .. },
            )))) if peer_id == relay_peer => {
                identified = true;
                break;
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(_) => {}
        }
    }
    if !identified {
        eprintln!("auki-network: identify with relay {relay_peer} timed out");
        return Vec::new();
    }

    let Some(parent) = dial_addrs
        .iter()
        .find(|a| ends_with_peer(a, &relay_peer))
        .cloned()
    else {
        return Vec::new();
    };
    let circuit_listen = parent.with(Protocol::P2pCircuit);
    if let Err(e) = swarm.listen_on(circuit_listen) {
        eprintln!("auki-network: listen_on relay circuit failed: {e}");
        return Vec::new();
    }

    let mut circuits: Vec<Multiaddr> = Vec::new();
    let collect_deadline = tokio::time::Instant::now() + relay_reservation_phase_duration();
    while tokio::time::Instant::now() < collect_deadline {
        let left = collect_deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left.min(Duration::from_secs(1)), swarm.next()).await {
            Ok(Some(SwarmEvent::NewListenAddr { address, .. })) => {
                if address.iter().any(|p| matches!(p, Protocol::P2pCircuit))
                    && !circuits.contains(&address)
                {
                    circuits.push(address);
                }
            }
            Ok(Some(SwarmEvent::Behaviour(BehaviourEvent::RelayClient(
                relay::client::Event::ReservationReqAccepted { .. },
            )))) => {}
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => break,
        }
    }
    circuits
}

/// If `addrs` already includes a circuit-relay listen surface, returns
/// them unchanged. Otherwise waits for AutoNAT / external confirmation;
/// on failure, queries Discovery for `relay` nodes and merges any
/// reserved circuit listen addrs. Never fails the overall flow — logs
/// and returns the best-effort address set.
pub async fn maybe_enrich_local_multiaddrs_for_discovery_registration(
    swarm: &mut Swarm<Behaviour>,
    discovery: &DiscoveryClient,
    local_peer_id: &PeerId,
    addrs: Vec<Multiaddr>,
) -> Vec<Multiaddr> {
    if addrs_contain_circuit(&addrs) {
        return addrs;
    }

    if poll_direct_reachability_confirmed(swarm).await {
        return addrs;
    }

    let relays = match discovery.list_nodes(Some("relay")).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "auki-network: Discovery list_nodes(relay) failed ({e}) — \
                 continuing with direct addrs only"
            );
            return addrs;
        }
    };
    if relays.is_empty() {
        eprintln!(
            "auki-network: no relay nodes advertised in Discovery — \
             continuing with direct addrs only"
        );
        return addrs;
    }

    let mut out: Vec<Multiaddr> = addrs;
    for node in relays {
        let relay_peer = match PeerId::from_str(&node.peer_id) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "auki-network: skip relay node with bad peer_id {:?}: {e}",
                    node.peer_id
                );
                continue;
            }
        };
        if relay_peer == *local_peer_id {
            continue;
        }

        let circuits = try_reserve_on_relay(swarm, relay_peer, &node.multiaddrs).await;
        for c in circuits {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        if addrs_contain_circuit(&out) {
            break;
        }
    }

    if !addrs_contain_circuit(&out) {
        eprintln!(
            "auki-network: relay reservation did not yield circuit listen addrs — \
             proceeding with direct addresses only"
        );
    }
    out
}

#[cfg(all(test, feature = "swarm", feature = "discovery_client"))]
mod tests {
    use super::*;
    use crate::PeerIdentity;
    use crate::Swarm;
    use crate::discovery_client::DiscoveryClient;
    use crate::swarm::{Behaviour, SwarmConfig, build_swarm};
    use libp2p::swarm::SwarmEvent;
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_tcp_config(agent_version: &str) -> SwarmConfig {
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: agent_version.into(),
            enable_relay_server: false,
            enable_autonat_server: false,
        }
    }

    async fn wait_for_listen_addr(swarm: &mut Swarm<Behaviour>) -> Multiaddr {
        use futures::StreamExt as _;
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

    #[tokio::test]
    async fn skips_enrichment_when_addrs_already_contain_circuit() {
        let id = PeerIdentity::from_seed(&[0xA1u8; 32]);
        let mut swarm = build_swarm(
            &id,
            SwarmConfig {
                listen_addresses: vec![],
                ..test_tcp_config("relay-prepare/0")
            },
        )
        .expect("build swarm");

        let direct = "/ip4/10.0.0.1/tcp/4001".parse::<Multiaddr>().unwrap();
        let pid = PeerIdentity::from_seed(&[0x5Eu8; 32]).peer_id();
        let circuit = format!("/ip4/127.0.0.1/tcp/9/p2p/{pid}/p2p-circuit")
            .parse::<Multiaddr>()
            .expect("circuit multiaddr parses");
        assert!(super::addrs_contain_circuit(&[circuit.clone()]));

        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nodes"))
            .and(query_param("type", "relay"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock)
            .await;

        let discovery = DiscoveryClient::new(mock.uri());
        let got = maybe_enrich_local_multiaddrs_for_discovery_registration(
            &mut swarm,
            &discovery,
            &id.peer_id(),
            vec![direct.clone(), circuit.clone()],
        )
        .await;

        assert_eq!(got, vec![direct, circuit]);
    }

    #[tokio::test]
    async fn list_nodes_failure_returns_original_addrs() {
        let id = PeerIdentity::from_seed(&[0xA2u8; 32]);
        let mut swarm = build_swarm(
            &id,
            SwarmConfig {
                listen_addresses: vec![],
                ..test_tcp_config("relay-prepare/1")
            },
        )
        .expect("build swarm");

        let addrs = vec!["/ip4/192.168.1.10/tcp/4001".parse().unwrap()];

        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nodes"))
            .and(query_param("type", "relay"))
            .respond_with(ResponseTemplate::new(503).set_body_string("unavailable"))
            .mount(&mock)
            .await;

        let discovery = DiscoveryClient::new(mock.uri());
        let got = maybe_enrich_local_multiaddrs_for_discovery_registration(
            &mut swarm,
            &discovery,
            &id.peer_id(),
            addrs.clone(),
        )
        .await;

        assert_eq!(got, addrs);
    }

    #[tokio::test]
    async fn empty_relay_directory_returns_original_addrs() {
        let id = PeerIdentity::from_seed(&[0xA3u8; 32]);
        let mut swarm = build_swarm(
            &id,
            SwarmConfig {
                listen_addresses: vec![],
                ..test_tcp_config("relay-prepare/2")
            },
        )
        .expect("build swarm");

        let addrs = vec!["/ip4/192.168.1.11/tcp/4002".parse().unwrap()];

        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nodes"))
            .and(query_param("type", "relay"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "nodes": [] })))
            .mount(&mock)
            .await;

        let discovery = DiscoveryClient::new(mock.uri());
        let got = maybe_enrich_local_multiaddrs_for_discovery_registration(
            &mut swarm,
            &discovery,
            &id.peer_id(),
            addrs.clone(),
        )
        .await;

        assert_eq!(got, addrs);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn discovery_relay_entry_enables_circuit_listen_addr() {
        let id_relay = PeerIdentity::from_seed(&[0xB0u8; 32]);
        let id_client = PeerIdentity::from_seed(&[0xB1u8; 32]);

        let mut relay_swarm = build_swarm(
            &id_relay,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                agent_version: "relay/0".into(),
                enable_relay_server: true,
                enable_autonat_server: true,
            },
        )
        .expect("build relay");

        let relay_addr = wait_for_listen_addr(&mut relay_swarm).await;
        relay_swarm.add_external_address(relay_addr.clone());
        let relay_addr_with_pid = relay_addr.with(Protocol::P2p(*relay_swarm.local_peer_id()));
        let relay_pid = *relay_swarm.local_peer_id();

        let mock = MockServer::start().await;
        let body = json!({
            "nodes": [{
                "peer_id": relay_pid.to_string(),
                "node_type": "relay",
                "multiaddrs": [relay_addr_with_pid.to_string()],
                "created_ns": 1,
                "last_heartbeat_ns": 1
            }]
        });
        Mock::given(method("GET"))
            .and(path("/nodes"))
            .and(query_param("type", "relay"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock)
            .await;

        let mut client = build_swarm(
            &id_client,
            SwarmConfig {
                listen_addresses: vec![],
                agent_version: "client/0".into(),
                enable_relay_server: false,
                enable_autonat_server: false,
            },
        )
        .expect("build client");

        let base = vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()];

        let discovery = DiscoveryClient::new(mock.uri());

        let relay_driver = tokio::spawn(async move {
            use futures::StreamExt as _;
            while relay_swarm.next().await.is_some() {}
        });

        let enriched = tokio::time::timeout(Duration::from_secs(90), async {
            maybe_enrich_local_multiaddrs_for_discovery_registration(
                &mut client,
                &discovery,
                &id_client.peer_id(),
                base.clone(),
            )
            .await
        })
        .await
        .expect("enrichment timed out");

        relay_driver.abort();

        assert!(
            enriched
                .iter()
                .any(|a| a.iter().any(|p| matches!(p, Protocol::P2pCircuit))),
            "expected at least one circuit multiaddr in {:?}",
            enriched
        );
    }
}
