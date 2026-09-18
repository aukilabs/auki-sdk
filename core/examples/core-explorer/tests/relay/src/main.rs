//! Loopback test relay. Source admission is synthetic; endpoint P2P auth is real.
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use libp2p::{
    StreamProtocol, SwarmBuilder, noise, relay, swarm::NetworkBehaviour, swarm::SwarmEvent, tcp,
    yamux,
};
use std::time::Duration;

#[derive(NetworkBehaviour)]
struct Behaviour {
    relay: relay::Behaviour,
    streams: libp2p_stream::Behaviour,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let identity = libp2p::identity::Keypair::generate_ed25519();
    let peer = identity.public().to_peer_id();
    let streams = libp2p_stream::Behaviour::new();
    let mut incoming = streams
        .new_control()
        .accept(StreamProtocol::new("/auki-p2p/relay-auth/1"))?;
    let mut swarm = SwarmBuilder::with_existing_identity(identity)
        .with_tokio()
        .with_tcp(
            tcp::Config::default().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(move |_| Behaviour {
            streams,
            relay: relay::Behaviour::new(
                peer,
                relay::Config {
                    reservation_duration: Duration::from_secs(120),
                    max_circuit_duration: Duration::from_secs(120),
                    max_circuit_bytes: 10_485_760,
                    reservation_rate_limiters: Vec::new(),
                    circuit_src_rate_limiters: Vec::new(),
                    ..Default::default()
                },
            ),
        })?
        .build();
    swarm.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?;
    let admission = tokio::spawn(async move {
        while let Some((_, mut stream)) = incoming.next().await {
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                let mut length = [0; 4];
                stream.read_exact(&mut length).await?;
                let size = u32::from_be_bytes(length) as usize;
                anyhow::ensure!((1..=65536).contains(&size), "invalid frame size");
                let mut bytes = vec![0; size];
                stream.read_exact(&mut bytes).await?;
                let request: serde_json::Value = serde_json::from_slice(&bytes)?;
                anyhow::ensure!(request["version"] == 1, "unsupported admission version");
                let response = serde_json::to_vec(&serde_json::json!({"accepted": true,
                    "accepted_until": (chrono::Utc::now() + chrono::Duration::seconds(20)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)}))?;
                stream.write_all(&(response.len() as u32).to_be_bytes()).await?;
                stream.write_all(&response).await?;
                stream.flush().await?;
                Ok::<_, anyhow::Error>(())
            }).await;
        }
    });
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            event = swarm.select_next_some() => if let SwarmEvent::NewListenAddr { address, .. } = event {
                swarm.add_external_address(address.clone());
                println!("{}", serde_json::json!({"peer": peer.to_string(), "address": address.to_string()}));
            }
        }
    }
    admission.abort();
    let _ = admission.await;
    Ok(())
}
