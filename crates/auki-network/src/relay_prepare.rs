//! Optional relay circuit addresses before Discovery registration.
//!
//! When the local peer has no circuit-relay listen address yet, we wait
//! briefly for AutoNAT v2 (or another behaviour) to confirm a dialable
//! direct surface. If that never succeeds within the window, we query
//! Discovery for infrastructure `relay` nodes, dial one, open a circuit
//! reservation, and merge the resulting `/p2p/.../p2p-circuit/...`
//! listen addrs into the set passed to Discovery / the join handshake.

use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt as _;
use libp2p::{
    Multiaddr, PeerId, identify, relay,
    swarm::SwarmEvent,
    Swarm,
};
use multiaddr::Protocol;

use crate::discovery_client::DiscoveryClient;
use crate::swarm::{Behaviour, BehaviourEvent, dial_peer, is_routable_multiaddr};

/// How long we poll for an AutoNAT success (`result == Ok(())`) or an
/// [`SwarmEvent::ExternalAddrConfirmed`] on a routable address before
/// deciding direct reachability was not established in time.
const AUTONAT_WAIT: Duration = Duration::from_secs(12);

/// Budget for identify + reservation + `NewListenAddr` after choosing a
/// relay from Discovery.
const RELAY_RESERVATION_PHASE: Duration = Duration::from_secs(18);

fn addrs_contain_circuit(addrs: &[Multiaddr]) -> bool {
    addrs
        .iter()
        .any(|a| a.iter().any(|p| matches!(p, Protocol::P2pCircuit)))
}

/// Returns `true` when we should skip relay acquisition — AutoNAT
/// reported success for at least one candidate, or the swarm confirmed
/// an externally-routable listen surface.
async fn poll_direct_reachability_confirmed(swarm: &mut Swarm<Behaviour>) -> bool {
    tokio::time::timeout(AUTONAT_WAIT, async {
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

    let ident_deadline = tokio::time::Instant::now() + Duration::from_secs(12);
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
    let collect_deadline = tokio::time::Instant::now() + RELAY_RESERVATION_PHASE;
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
