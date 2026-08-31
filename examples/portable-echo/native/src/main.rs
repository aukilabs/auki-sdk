use std::env;

use anyhow::{Context, Result, bail};
use auki_auth::{AuthClient, AuthEnvironment, Credentials, DomainSelection};
use auki_portable_echo_adapter::EchoEndpoint;
use auki_sdk::{AukiPeer, AukiPeerConfig, Identity};

#[tokio::main]
async fn main() -> Result<()> {
    let identity = Identity::load_or_create(
        env::var("AUKI_IDENTITY_FILE").unwrap_or_else(|_| "./state/peer.identity".to_owned()),
    )?;
    let session = AuthClient::new(AuthEnvironment::dev())?
        .authenticate(Credentials::user_password(
            env::var("AUKI_EMAIL")?,
            env::var("AUKI_PASSWORD")?,
        ))
        .await?;
    let prepared = session
        .authorize_peer(
            DomainSelection::new(env::var("AUKI_DOMAIN_ID")?.parse()?),
            &identity.proof(),
        )
        .await?;
    let peer = AukiPeer::start(identity, prepared, AukiPeerConfig::dev()).await?;

    let operation = run(&peer).await;
    let cleanup = peer.shutdown().await;
    operation?;
    cleanup?;
    Ok(())
}

async fn run(peer: &AukiPeer) -> Result<()> {
    let echo = EchoEndpoint::mount(peer.protocols())?;
    let operation = async {
        let relay = peer
            .protocol_context()
            .routes()
            .snapshot()?
            .relay_routes
            .into_iter()
            .next()
            .context("peer started without its required relay route")?;
        println!("peer: {}", peer.peer_id());
        println!("route: {}", relay.route);

        let mut arguments = env::args().skip(1);
        if let Some(remote_peer) = arguments.next() {
            let remote_route = arguments.next().context("REMOTE_ROUTE is required")?;
            let receipt = echo
                .send_exact(
                    remote_peer.parse()?,
                    remote_route.parse()?,
                    "hello from Auki",
                )
                .await?;
            println!("echo: {}", String::from_utf8_lossy(&receipt.payload));
        } else {
            println!("serving; press Ctrl-C to stop");
            let mut statuses = peer.subscribe_status();
            tokio::select! {
                result = tokio::signal::ctrl_c() => result?,
                terminal = statuses.wait_for(|status| status.is_terminal()) => {
                    let status = *terminal.context("wait for terminal Auki peer status")?;
                    bail!("Auki peer stopped unexpectedly: {status:?}");
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;

    let cleanup = echo.close().await;
    operation?;
    cleanup?;
    Ok(())
}
