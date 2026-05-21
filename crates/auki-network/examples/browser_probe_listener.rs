use auki_network::{PeerIdentity, browser_probe};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seed = [41u8; 32];
    let identity = PeerIdentity::from_seed(&seed);
    let listen_addr = "/ip4/0.0.0.0/udp/0/webrtc-direct".parse()?;

    eprintln!("peer_id={}", identity.peer_id());
    browser_probe::listen_and_serve(identity, listen_addr).await?;
    Ok(())
}
