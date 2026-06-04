use auki_network::{
    PeerIdentity,
    swarm::{Behaviour, BehaviourEvent, SwarmConfig, build_swarm},
};
use futures::StreamExt;
use libp2p::{
    Multiaddr, PeerId, identify,
    multiaddr::Protocol,
    relay,
    swarm::{Swarm, SwarmEvent},
};
use std::{env, error::Error, fs, path::PathBuf, time::Duration};

const TARGET_FILE: &str = "examples/relay-smoke/target-addr.txt";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let timeout = env::var("AUKI_RELAY_SMOKE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30));

    let relay_env = env::var("AUKI_RELAY_ADDR").ok();
    let id_target = PeerIdentity::from_seed(&[72u8; 32]);
    let mut target = build_swarm(
        &id_target,
        SwarmConfig {
            listen_addresses: vec![],
            agent_version: "auki-relay-native-target-smoke/0".into(),
            enable_relay_server: false,
        },
    )?;

    let (mut relay_swarm, relay_addr_with_peer) = match relay_env {
        Some(addr) if !addr.trim().is_empty() => (None, addr.parse()?),
        _ => {
            let id_relay = PeerIdentity::from_seed(&[71u8; 32]);
            let mut relay = build_swarm(
                &id_relay,
                SwarmConfig {
                    listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse()?],
                    agent_version: "auki-relay-smoke/0".into(),
                    enable_relay_server: true,
                },
            )?;
            let relay_addr = wait_for_listen_addr(&mut relay, timeout).await?;
            relay.add_external_address(relay_addr.clone());
            let relay_addr_with_peer = relay_addr.with(Protocol::P2p(*relay.local_peer_id()));
            (Some(relay), relay_addr_with_peer)
        }
    };

    let relay_peer_id = last_peer_id(&relay_addr_with_peer)
        .ok_or("relay address must end with /p2p/<relay-peer-id>")?;

    target.dial(relay_addr_with_peer.clone())?;
    wait_for_identify_with_relay(&mut target, &mut relay_swarm, relay_peer_id, timeout).await?;

    let circuit_listen_addr = relay_addr_with_peer.clone().with(Protocol::P2pCircuit);
    target.listen_on(circuit_listen_addr.clone())?;
    wait_for_reservation(&mut target, &mut relay_swarm, timeout).await?;

    let target_peer_id = *target.local_peer_id();
    let browser_target_addr = format!("{circuit_listen_addr}/webrtc/p2p/{target_peer_id}");
    write_target_addr(&browser_target_addr)?;

    println!("relay_peer_id={relay_peer_id}");
    println!("target_peer_id={target_peer_id}");
    println!("AUKI_RELAY_TARGET_ADDR={browser_target_addr}");
    eprintln!("waiting up to {}s for a browser peer", timeout.as_secs());

    wait_for_browser_connection(&mut target, &mut relay_swarm, relay_peer_id, timeout).await?;
    Ok(())
}

async fn wait_for_listen_addr(
    swarm: &mut Swarm<Behaviour>,
    timeout: Duration,
) -> Result<Multiaddr, Box<dyn Error>> {
    tokio::time::timeout(timeout, async {
        loop {
            if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                return address;
            }
        }
    })
    .await
    .map_err(|_| "relay listen address did not appear within timeout".into())
}

async fn wait_for_identify_with_relay(
    target: &mut Swarm<Behaviour>,
    relay_swarm: &mut Option<Swarm<Behaviour>>,
    relay_peer_id: PeerId,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(timeout, async {
        let mut target_saw_relay = false;
        let mut relay_saw_target = relay_swarm.is_none();

        while !(target_saw_relay && relay_saw_target) {
            tokio::select! {
                event = target.next() => {
                    if let Some(SwarmEvent::Behaviour(BehaviourEvent::Identify(
                        identify::Event::Received { peer_id, .. }
                    ))) = event {
                        if peer_id == relay_peer_id {
                            target_saw_relay = true;
                        }
                    }
                }
                event = next_relay_event(relay_swarm) => {
                    if let Some(SwarmEvent::Behaviour(BehaviourEvent::Identify(
                        identify::Event::Received { .. }
                    ))) = event {
                        relay_saw_target = true;
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| "identify exchange with relay did not complete within timeout".into())
}

async fn wait_for_reservation(
    target: &mut Swarm<Behaviour>,
    relay_swarm: &mut Option<Swarm<Behaviour>>,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(timeout, async {
        loop {
            tokio::select! {
                event = target.next() => {
                    if let Some(SwarmEvent::Behaviour(BehaviourEvent::RelayClient(
                        relay::client::Event::ReservationReqAccepted { .. }
                    ))) = event {
                        return;
                    }
                }
                _ = next_relay_event(relay_swarm) => {}
            }
        }
    })
    .await
    .map_err(|_| "relay reservation did not complete within timeout".into())
}

async fn wait_for_browser_connection(
    target: &mut Swarm<Behaviour>,
    relay_swarm: &mut Option<Swarm<Behaviour>>,
    relay_peer_id: PeerId,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(timeout, async {
        loop {
            tokio::select! {
                event = target.next() => {
                    if let Some(SwarmEvent::ConnectionEstablished { peer_id, .. }) = event {
                        if peer_id != relay_peer_id {
                            println!("browser_peer_id={peer_id}");
                            return;
                        }
                    }
                }
                _ = next_relay_event(relay_swarm) => {}
            }
        }
    })
    .await
    .map_err(|_| {
        "native target did not observe an inbound browser connection within timeout".into()
    })
}

async fn next_relay_event(
    relay_swarm: &mut Option<Swarm<Behaviour>>,
) -> Option<SwarmEvent<BehaviourEvent>> {
    match relay_swarm {
        Some(swarm) => swarm.next().await,
        None => std::future::pending().await,
    }
}

fn last_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    addr.iter()
        .filter_map(|protocol| match protocol {
            Protocol::P2p(peer_id) => Some(peer_id),
            _ => None,
        })
        .last()
}

fn write_target_addr(addr: &str) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(TARGET_FILE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{addr}\n"))?;
    Ok(())
}
