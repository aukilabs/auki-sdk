use auki_network::PeerIdentity;
use futures::StreamExt as _;
use libp2p::{Multiaddr, noise, relay, swarm::SwarmEvent, yamux};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let identity = PeerIdentity::from_seed(&[78u8; 32]);
    let peer_id = identity.peer_id();
    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0/ws".parse()?;
    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_tokio()
        .with_websocket(noise::Config::new, yamux::Config::default)
        .await?
        .with_behaviour(|_| RelayBehaviour {
            relay: relay::Behaviour::new(peer_id, relay::Config::default()),
        })?
        .build();

    eprintln!("relay_peer_id={peer_id}");
    swarm.listen_on(listen_addr)?;

    while let Some(event) = swarm.next().await {
        if let SwarmEvent::NewListenAddr { address, .. } = event {
            swarm.add_external_address(address.clone());
            println!("relay_addr={address}/p2p/{peer_id}");
        }
    }

    Ok(())
}

#[derive(libp2p::swarm::NetworkBehaviour)]
struct RelayBehaviour {
    relay: relay::Behaviour,
}
