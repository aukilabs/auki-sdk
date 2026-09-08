use std::{
    env,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use auki_portable_echo::{EchoEndpoint, PROTOCOL_ID};
use auki_sdk::{
    AukiDiscoveryCandidate, AukiPeer, AukiPeerBootstrap, Credentials, DdsTrackerMode,
    DomainSelection, Multiaddr, PeerId,
};

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(120);
const DISCOVERY_RETRY: Duration = Duration::from_millis(500);

#[tokio::main]
async fn main() -> Result<()> {
    let bootstrap = AukiPeerBootstrap::dev(Credentials::user_password(
        env::var("AUKI_EMAIL")?,
        env::var("AUKI_PASSWORD")?,
    ))
    .await?
    .with_dds_tracker(discovery_mode_from_env()?);
    let peer = bootstrap
        .start_persistent_peer(
            DomainSelection::new(env::var("AUKI_DOMAIN_ID")?.parse()?),
            env::var("AUKI_IDENTITY_FILE").unwrap_or_else(|_| "./state/peer.identity".to_owned()),
        )
        .await?;

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
        println!("route: {}", relay.routes.tcp());

        match target_from_args()? {
            Some(EchoTarget::Discovered(remote_peer)) => {
                let receipt = send_discovered(peer, &echo, remote_peer, "hello from Auki").await?;
                println!("echo: {}", String::from_utf8_lossy(&receipt.payload));
            }
            Some(EchoTarget::Manual { peer_id, route }) => {
                println!("using manual exact target fallback");
                let receipt = echo
                    .send_exact(peer_id, route, "hello from Auki")
                    .await?;
                println!("echo: {}", String::from_utf8_lossy(&receipt.payload));
            }
            None => {
                match peer.discover_protocol(PROTOCOL_ID).await {
                    Ok(candidates) => print_candidates(&candidates),
                    Err(error) => eprintln!("refresh Echo discovery failed: {error}"),
                }
                println!(
                    "serving; use --discover <PEER_ID> from another terminal or press Ctrl-C to stop"
                );
                let mut statuses = peer.subscribe_status();
                tokio::select! {
                    result = tokio::signal::ctrl_c() => result?,
                    terminal = statuses.wait_for(|status| status.is_terminal()) => {
                        let status = *terminal.context("wait for terminal Auki peer status")?;
                        bail!("Auki peer stopped unexpectedly: {status:?}");
                    }
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

enum EchoTarget {
    Discovered(PeerId),
    Manual { peer_id: PeerId, route: Multiaddr },
}

fn target_from_args() -> Result<Option<EchoTarget>> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [] => Ok(None),
        [flag, peer_id] if flag == "--discover" => Ok(Some(EchoTarget::Discovered(
            peer_id.parse().context("invalid discovered Peer ID")?,
        ))),
        [peer_id, route] => Ok(Some(EchoTarget::Manual {
            peer_id: peer_id.parse().context("invalid manual Peer ID")?,
            route: route.parse().context("invalid manual exact route")?,
        })),
        _ => bail!("usage: auki-portable-echo-native [--discover PEER_ID | PEER_ID EXACT_ROUTE]"),
    }
}

fn discovery_mode_from_env() -> Result<DdsTrackerMode> {
    match env::var("AUKI_DISCOVERY_MODE") {
        Ok(mode) if mode == "discover_only" => Ok(DdsTrackerMode::DiscoverOnly),
        Ok(mode) if mode == "discover_and_advertise" => Ok(DdsTrackerMode::DiscoverAndAdvertise),
        Ok(other) => bail!(
            "AUKI_DISCOVERY_MODE must be discover_only or discover_and_advertise, got {other:?}"
        ),
        Err(env::VarError::NotPresent) => Ok(DdsTrackerMode::DiscoverAndAdvertise),
        Err(error) => Err(error).context("read AUKI_DISCOVERY_MODE"),
    }
}

fn print_candidates(candidates: &[AukiDiscoveryCandidate]) {
    println!("discovered Echo peers (untrusted until exact dial):");
    if candidates.is_empty() {
        println!("  none");
    }
    for candidate in candidates {
        println!(
            "  {} expires={} routes={}",
            candidate.peer_id(),
            candidate.expires_at(),
            candidate.routes().len()
        );
    }
}

async fn send_discovered(
    peer: &AukiPeer,
    echo: &EchoEndpoint,
    expected_peer: PeerId,
    payload: impl AsRef<[u8]>,
) -> Result<auki_portable_echo::EchoSendReceipt> {
    let candidate = wait_for_candidate(peer, expected_peer).await?;
    print_candidates(std::slice::from_ref(&candidate));
    let routes = preferred_native_routes(candidate.routes());
    if routes.is_empty() {
        bail!("Echo peer {expected_peer} advertised no native-compatible route");
    }

    let mut failures = Vec::new();
    for route in routes {
        match echo
            .send_exact(expected_peer, route.clone(), payload.as_ref())
            .await
        {
            Ok(receipt) => return Ok(receipt),
            Err(error) => failures.push(format!("{route}: {error}")),
        }
    }
    bail!(
        "every discovered route for Echo peer {expected_peer} failed: {}",
        failures.join("; ")
    )
}

async fn wait_for_candidate(
    peer: &AukiPeer,
    expected_peer: PeerId,
) -> Result<AukiDiscoveryCandidate> {
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let mut last_error = None;
    loop {
        match peer.discover_protocol(PROTOCOL_ID).await {
            Ok(candidates) => {
                if let Some(candidate) = candidates
                    .into_iter()
                    .find(|candidate| candidate.peer_id() == expected_peer)
                {
                    return Ok(candidate);
                }
            }
            Err(error) => last_error = Some(error.to_string()),
        }
        if Instant::now() >= deadline {
            let detail = last_error
                .map(|error| format!("; last DDS error: {error}"))
                .unwrap_or_default();
            bail!("Echo peer {expected_peer} was not discovered before timeout{detail}");
        }
        tokio::time::sleep(DISCOVERY_RETRY).await;
    }
}

fn preferred_native_routes(routes: &[Multiaddr]) -> Vec<Multiaddr> {
    let mut routes = routes
        .iter()
        .filter(|route| !has_wss_protocol(route))
        .cloned()
        .collect::<Vec<_>>();
    routes.sort_by_key(|route| !route.to_string().contains("/p2p-circuit/"));
    routes
}

fn has_wss_protocol(route: &Multiaddr) -> bool {
    route
        .to_string()
        .split('/')
        .any(|component| component == "wss")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_discovery_prefers_circuit_routes_and_ignores_wss() {
        let direct = "/dns4/wss-node.example.com/tcp/4001/p2p/12D3KooWJ5Xw8jCxxbVZXcaUpf7h8fWgpcnH9tGgNfZQ1nSJXUL3"
            .parse::<Multiaddr>()
            .unwrap();
        let relay = "/dns4/relay.example.com/tcp/443/p2p/12D3KooWKe31227N64kxokD3Z913sP4i7B1a9ProcvyJt95QTrqM/p2p-circuit/p2p/12D3KooWJ5Xw8jCxxbVZXcaUpf7h8fWgpcnH9tGgNfZQ1nSJXUL3"
            .parse::<Multiaddr>()
            .unwrap();
        let wss = "/dns4/relay.example.com/tcp/4443/wss/p2p/12D3KooWKe31227N64kxokD3Z913sP4i7B1a9ProcvyJt95QTrqM/p2p-circuit/p2p/12D3KooWJ5Xw8jCxxbVZXcaUpf7h8fWgpcnH9tGgNfZQ1nSJXUL3"
            .parse::<Multiaddr>()
            .unwrap();

        assert_eq!(
            preferred_native_routes(&[direct.clone(), wss, relay.clone()]),
            vec![relay, direct]
        );
    }
}
