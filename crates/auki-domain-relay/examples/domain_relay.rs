use auki_domain_relay::{DomainRelay, DomainRelayConfig, DomainRelayEvent};
use auki_network::PeerIdentity;
use libp2p::Multiaddr;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen_addresses = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let raw = if args.is_empty() {
            vec![
                "/ip4/0.0.0.0/tcp/0".to_string(),
                "/ip4/0.0.0.0/tcp/0/ws".to_string(),
            ]
        } else {
            args
        };
        raw.into_iter()
            .map(|addr| addr.parse::<Multiaddr>())
            .collect::<Result<Vec<_>, _>>()?
    };
    let identity = PeerIdentity::from_seed(&[78u8; 32]);
    let mut relay = DomainRelay::new(
        &identity,
        DomainRelayConfig {
            listen_addresses,
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
