use std::{env, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use auki_portable_echo::{EchoEndpoint, EchoEventReceiver, EchoServeEvent, PROTOCOL_ID};
use auki_sdk::{
    AukiDiscoveryCandidate, AukiPeer, AukiPeerBootstrap, Credentials, DdsTrackerMode,
    DomainSelection, Multiaddr, PeerId,
};
use tokio::time::Instant;
use uuid::Uuid;

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(120);
const DISCOVERY_RETRY: Duration = Duration::from_millis(500);

#[tokio::main]
async fn main() -> Result<()> {
    let domain_id = required_env("AUKI_DOMAIN_ID")?
        .parse::<Uuid>()
        .context("AUKI_DOMAIN_ID must be a UUID")?;
    let state_dir = PathBuf::from(required_env("AUKI_STATE_DIR")?);
    let bootstrap = AukiPeerBootstrap::dev(credentials_from_env()?)
        .await?
        .with_dds_tracker(discovery_mode_from_env()?);

    let remote_peer = remote_peer_from_env()?;
    let peer = bootstrap
        .start_persistent_peer(
            DomainSelection::new(domain_id),
            state_dir.join("peer.identity"),
        )
        .await?;
    let echo = match EchoEndpoint::mount(peer.protocols()) {
        Ok(echo) => echo,
        Err(error) => {
            let peer_shutdown: Result<()> = peer.shutdown().await.map_err(Into::into);
            return complete_with_cleanup(Err(error.into()), vec![("Auki peer", peer_shutdown)]);
        }
    };
    let echo_events = echo.events();

    let operation = run_started_peer(&peer, &echo, &echo_events, remote_peer.as_ref()).await;
    let echo_shutdown: Result<()> = echo.close().await.map_err(Into::into);
    let peer_shutdown: Result<()> = peer.shutdown().await.map_err(Into::into);
    complete_with_cleanup(
        operation,
        vec![
            ("portable echo endpoint", echo_shutdown),
            ("Auki peer", peer_shutdown),
        ],
    )?;
    println!("STOPPED");
    Ok(())
}

async fn run_started_peer(
    peer: &AukiPeer,
    echo: &EchoEndpoint,
    echo_events: &EchoEventReceiver,
    remote_peer: Option<&RemotePeer>,
) -> Result<()> {
    println!("READY");
    println!("PEER_ID={}", peer.peer_id());

    if let Some(remote) = remote_peer {
        let message = env::var("AUKI_ECHO_MESSAGE")
            .unwrap_or_else(|_| "hello from the shared Rust protocol".to_owned());
        let receipt = match remote {
            RemotePeer::Discovered(peer_id) => {
                send_discovered(peer, echo, *peer_id, message.into_bytes()).await?
            }
            RemotePeer::Manual { peer_id, route } => {
                println!("MANUAL_EXACT_TARGET peer={peer_id}");
                echo.send_exact(*peer_id, route.clone(), message.into_bytes())
                    .await?
            }
        };
        println!(
            "ECHO_OK remote_peer={} relayed={} bytes={}",
            receipt.remote_peer_id,
            receipt.relayed,
            receipt.payload.len()
        );
    }
    if remote_peer.is_none() || env::var_os("AUKI_KEEP_RUNNING").is_some() {
        println!("WAITING_FOR_PEER");
        serve_until_shutdown(echo_events).await?;
    }

    Ok(())
}

fn complete_with_cleanup(
    operation: Result<()>,
    cleanup: Vec<(&'static str, Result<()>)>,
) -> Result<()> {
    let failures = cleanup
        .into_iter()
        .filter_map(|(component, result)| {
            result.err().map(|error| format!("{component}: {error:#}"))
        })
        .collect::<Vec<_>>();
    match (operation, failures.is_empty()) {
        (Ok(()), true) => Ok(()),
        (Ok(()), false) => bail!("ordered shutdown failed: {}", failures.join("; ")),
        (Err(error), true) => Err(error),
        (Err(error), false) => {
            Err(error.context(format!("cleanup also failed: {}", failures.join("; "))))
        }
    }
}

async fn serve_until_shutdown(events: &EchoEventReceiver) -> Result<()> {
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            result = &mut shutdown => return result.context("wait for Ctrl-C"),
            event = events.recv() => match event {
                Some(EchoServeEvent::Served(receipt)) => println!(
                    "ECHO_SERVED remote_peer={} bytes={}",
                    receipt.remote_peer_id,
                    receipt.payload.len()
                ),
                Some(EchoServeEvent::Failed { remote_peer_id, error }) => eprintln!(
                    "ECHO_SERVER_FAILED remote_peer={remote_peer_id} error={error}"
                ),
                Some(EchoServeEvent::Lagged { dropped }) => {
                    eprintln!("ECHO_EVENTS_LAGGED dropped={dropped}")
                }
                None => bail!("portable echo endpoint stopped while waiting for a peer"),
            },
        }
    }
}

fn credentials_from_env() -> Result<Credentials> {
    let user = optional_pair("AUKI_EMAIL", "AUKI_PASSWORD")?;
    let app = optional_pair("AUKI_APP_ACCESS_KEY", "AUKI_APP_SECRET")?;
    match (user, app) {
        (Some((email, password)), None) => Ok(Credentials::user_password(email, password)),
        (None, Some((access_key, secret))) => Ok(Credentials::app(access_key, secret)),
        (None, None) => {
            bail!("set AUKI_EMAIL/AUKI_PASSWORD or AUKI_APP_ACCESS_KEY/AUKI_APP_SECRET")
        }
        (Some(_), Some(_)) => bail!("configure either User or App credentials, not both"),
    }
}

enum RemotePeer {
    Discovered(PeerId),
    Manual { peer_id: PeerId, route: Multiaddr },
}

fn remote_peer_from_env() -> Result<Option<RemotePeer>> {
    match (
        env::var("AUKI_REMOTE_PEER_ID"),
        env::var("AUKI_REMOTE_ROUTE"),
    ) {
        (Ok(peer_id), Ok(route)) => {
            let peer_id = peer_id
                .parse::<PeerId>()
                .context("invalid remote Peer ID")?;
            let route = route
                .parse::<Multiaddr>()
                .context("invalid remote relay route")?;
            Ok(Some(RemotePeer::Manual { peer_id, route }))
        }
        (Ok(peer_id), Err(env::VarError::NotPresent)) => Ok(Some(RemotePeer::Discovered(
            peer_id
                .parse::<PeerId>()
                .context("invalid discovered remote Peer ID")?,
        ))),
        (Err(env::VarError::NotPresent), Err(env::VarError::NotPresent)) => Ok(None),
        (Err(env::VarError::NotPresent), Ok(_)) => {
            bail!("AUKI_REMOTE_ROUTE requires AUKI_REMOTE_PEER_ID")
        }
        (Err(error), _) => Err(error).context("read AUKI_REMOTE_PEER_ID"),
        (_, Err(error)) => Err(error).context("read AUKI_REMOTE_ROUTE"),
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

async fn send_discovered(
    peer: &AukiPeer,
    echo: &EchoEndpoint,
    expected_peer: PeerId,
    payload: Vec<u8>,
) -> Result<auki_portable_echo::EchoSendReceipt> {
    let candidate = wait_for_candidate(peer, expected_peer).await?;
    let routes = preferred_native_routes(candidate.routes());
    if routes.is_empty() {
        bail!("Echo peer {expected_peer} advertised no native-compatible route");
    }
    println!(
        "DISCOVERY_SELECTED peer={} routes={} expires={}",
        candidate.peer_id(),
        routes.len(),
        candidate.expires_at()
    );

    let mut failures = Vec::new();
    for route in routes {
        match echo
            .send_exact(expected_peer, route.clone(), payload.clone())
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
        .filter(|route| !route.to_string().contains("/wss"))
        .cloned()
        .collect::<Vec<_>>();
    routes.sort_by_key(|route| !route.to_string().contains("/p2p-circuit/"));
    routes
}

fn optional_pair(first: &'static str, second: &'static str) -> Result<Option<(String, String)>> {
    match (env::var(first), env::var(second)) {
        (Ok(first_value), Ok(second_value)) => Ok(Some((first_value, second_value))),
        (Err(env::VarError::NotPresent), Err(env::VarError::NotPresent)) => Ok(None),
        (Err(error), _) => Err(error).with_context(|| format!("read {first}")),
        (_, Err(error)) => Err(error).with_context(|| format!("read {second}")),
    }
}

fn required_env(name: &'static str) -> Result<String> {
    env::var(name).with_context(|| format!("{name} is required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use auki_portable_echo::{MAX_CONCURRENCY, protocol_spec};

    #[test]
    fn native_endpoint_mounts_the_exact_portable_contract() {
        let spec = protocol_spec().unwrap();
        assert_eq!(spec.protocol_id(), PROTOCOL_ID);
        assert_eq!(spec.max_concurrency(), MAX_CONCURRENCY);
    }

    #[test]
    fn cleanup_is_reported_without_discarding_the_primary_failure() {
        let error = complete_with_cleanup(
            Err(anyhow!("outbound echo failed")),
            vec![("Auki peer", Err(anyhow!("booking deletion failed")))],
        )
        .unwrap_err();
        let rendered = format!("{error:#}");
        assert!(rendered.contains("outbound echo failed"));
        assert!(rendered.contains("Auki peer: booking deletion failed"));

        let cleanup_only = complete_with_cleanup(
            Ok(()),
            vec![("Auki peer", Err(anyhow!("booking deletion failed")))],
        )
        .unwrap_err();
        assert!(cleanup_only.to_string().contains("ordered shutdown failed"));
    }
}
