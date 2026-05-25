use auki_domain_relay::{DomainRelay, DomainRelayConfig, DomainRelayEvent};
use auki_network::PeerIdentity;
use libp2p::Multiaddr;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen_addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/ip4/0.0.0.0/tcp/0/ws".to_string())
        .parse::<Multiaddr>()?;
    let identity = PeerIdentity::from_seed(&[78u8; 32]);
    let mut relay = DomainRelay::new(
        &identity,
        DomainRelayConfig {
            listen_addresses: vec![listen_addr],
            agent_version: "auki-domain-relay/example".to_string(),
        },
    )
    .await?;

    eprintln!("relay_peer_id={}", relay.peer_id());
    while let Some(event) = relay.next_event().await {
        let DomainRelayEvent::Listening { relay_multiaddr } = event;
        println!("relay_multiaddr={relay_multiaddr}");
    }

    Ok(())
}
