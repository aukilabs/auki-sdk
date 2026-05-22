use auki_network::{
    PeerIdentity, browser_probe,
    join_protocol::{JOIN_PROTOCOL, JoinResponse, read_join_request, write_join_response},
};
use futures::StreamExt as _;
use libp2p::{Multiaddr, StreamProtocol, swarm::SwarmEvent};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
struct Member {
    multiaddrs: Vec<String>,
    join_ts_ns: i64,
}

fn membership_json(
    domain_name: &str,
    manager_peer_id: &str,
    manager_addr: &str,
    members: &BTreeMap<String, Member>,
) -> String {
    let mut peers = vec![serde_json::json!({
        "peer_id": manager_peer_id,
        "multiaddrs": [manager_addr],
        "join_ts_ns": 0,
        "successor_token": [],
    })];
    peers.extend(members.iter().map(|(peer_id, member)| {
        serde_json::json!({
            "peer_id": peer_id,
            "multiaddrs": member.multiaddrs,
            "join_ts_ns": member.join_ts_ns,
            "successor_token": [],
        })
    }));
    serde_json::json!({
        "cluster_name": domain_name,
        "peers": peers,
    })
    .to_string()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seed = [77u8; 32];
    let identity = PeerIdentity::from_seed(&seed);
    let manager_peer_id = identity.peer_id().to_string();
    let listen_addr: Multiaddr = "/ip4/0.0.0.0/udp/0/webrtc-direct".parse()?;
    let mut manager_swarm = browser_probe::build_browser_probe_swarm(&identity)?;
    let mut join_listener = manager_swarm
        .behaviour()
        .stream
        .new_control()
        .accept(StreamProtocol::new(JOIN_PROTOCOL))?;
    let mut browser_session_listener = manager_swarm
        .behaviour()
        .stream
        .new_control()
        .accept(StreamProtocol::new("/auki/browser-session/0.0.1"))?;
    let members = Arc::new(Mutex::new(BTreeMap::<String, Member>::new()));
    let manager_addr = Arc::new(Mutex::new(None::<String>));
    let next_join_ts_ns = Arc::new(Mutex::new(1_i64));

    eprintln!("manager_peer_id={manager_peer_id}");
    manager_swarm.listen_on(listen_addr)?;

    loop {
        tokio::select! {
            Some((peer, mut stream)) = join_listener.next() => {
                let members = members.clone();
                let manager_addr = manager_addr.clone();
                let next_join_ts_ns = next_join_ts_ns.clone();
                let manager_peer_id = manager_peer_id.clone();
                tokio::spawn(async move {
                    let Ok(req) = read_join_request(&mut stream).await else {
                        eprintln!("join request read failed from {peer}");
                        return;
                    };
                    let mut next_join_ts_ns = next_join_ts_ns.lock().await;
                    let join_ts_ns = *next_join_ts_ns;
                    *next_join_ts_ns = next_join_ts_ns.saturating_add(1);
                    drop(next_join_ts_ns);

                    let mut members_guard = members.lock().await;
                    members_guard.insert(
                        peer.to_string(),
                        Member {
                            multiaddrs: req.multiaddrs.iter().map(ToString::to_string).collect(),
                            join_ts_ns,
                        },
                    );
                    drop(members_guard);
                    tokio::time::sleep(Duration::from_millis(100)).await;

                    let members_guard = members.lock().await;
                    let manager_addr = manager_addr.lock().await.clone().unwrap_or_default();
                    let membership_json = membership_json(
                        "browser-full-peer",
                        &manager_peer_id,
                        &manager_addr,
                        &members_guard,
                    );
                    if let Err(err) = write_join_response(
                        &mut stream,
                        &JoinResponse::Accept {
                            membership_json,
                            successor_token: Vec::new(),
                        },
                    ).await {
                        eprintln!("join response write failed to {peer}: {err}");
                    }
                });
            }
            Some((peer, _stream)) = browser_session_listener.next() => {
                eprintln!("unexpected browser-session from {peer}");
                std::process::exit(42);
            }
            Some(event) = manager_swarm.next() => {
                if let SwarmEvent::NewListenAddr { address, .. } = event {
                    let address = format!("{address}/p2p/{manager_peer_id}");
                    *manager_addr.lock().await = Some(address.clone());
                    println!("manager_addr={address}");
                }
            }
        }
    }
}
