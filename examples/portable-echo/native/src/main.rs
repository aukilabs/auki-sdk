use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use auki_auth::{AuthClient, AuthEnvironment, Credentials, DomainSelection};
use auki_portable_echo_adapter::{EchoEndpoint, EchoEventReceiver, EchoServeEvent, PROTOCOL_ID};
use auki_sdk::{AukiPeer, AukiPeerConfig, Identity, Multiaddr, PeerId};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    let domain_id = required_env("AUKI_DOMAIN_ID")?
        .parse::<Uuid>()
        .context("AUKI_DOMAIN_ID must be a UUID")?;
    let state_dir = PathBuf::from(required_env("AUKI_STATE_DIR")?);
    fs::create_dir_all(&state_dir)
        .with_context(|| format!("create state directory {}", state_dir.display()))?;

    let identity = Identity::load_or_create(state_dir.join("peer.identity"))?;
    let auth = AuthClient::new(AuthEnvironment::dev())?;
    let session = auth.authenticate(credentials_from_env()?).await?;
    let selected = session
        .accessible_domains()
        .await?
        .into_iter()
        .find(|choice| choice.domain.id == domain_id)
        .context("the authenticated principal cannot access AUKI_DOMAIN_ID")?;
    let prepared = session
        .authorize_peer(DomainSelection::new(selected.domain.id), &identity.proof())
        .await?;

    let remote_peer = remote_peer_from_env()?;
    let peer = AukiPeer::start(identity, prepared, AukiPeerConfig::dev()).await?;
    let context = peer.protocol_context();
    let echo = EchoEndpoint::mount(peer.protocols())?;
    let echo_events = echo.events();

    println!("READY");
    println!("PEER_ID={}", peer.peer_id());
    let mut public_peer_card = None;
    for published in context.routes().snapshot()?.relay_routes {
        println!("RELAY_ROUTE={}", published.route);
        if let Some(wss_route) = published.wss_route {
            println!("RELAY_WSS_ROUTE={wss_route}");
            public_peer_card.get_or_insert_with(|| {
                serde_json::json!({
                    "version": 1,
                    "domainId": peer.domain_id().to_string(),
                    "peerId": peer.peer_id().to_string(),
                    "protocols": [PROTOCOL_ID],
                    "routes": {
                        "wss": wss_route.to_string(),
                        "tcp": published.route.to_string(),
                    },
                })
            });
        }
    }
    if let Some(card) = public_peer_card {
        println!("PEER_CARD={card}");
    }

    if let Some(remote) = remote_peer.as_ref() {
        let message = env::var("AUKI_ECHO_MESSAGE")
            .unwrap_or_else(|_| "hello from the shared Rust protocol".to_owned());
        let receipt = echo
            .send_exact(remote.peer_id, remote.route.clone(), message.into_bytes())
            .await?;
        println!(
            "ECHO_OK remote_peer={} relayed={} bytes={}",
            receipt.remote_peer_id,
            receipt.relayed,
            receipt.payload.len()
        );
    }
    if remote_peer.is_none() || env::var_os("AUKI_KEEP_RUNNING").is_some() {
        println!("WAITING_FOR_PEER");
        serve_until_shutdown(&echo_events).await?;
    }

    let echo_shutdown = echo.close().await;
    let peer_shutdown = peer.shutdown().await;
    echo_shutdown?;
    peer_shutdown?;
    println!("STOPPED");
    Ok(())
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

struct RemotePeer {
    peer_id: PeerId,
    route: Multiaddr,
}

fn remote_peer_from_env() -> Result<Option<RemotePeer>> {
    match optional_pair("AUKI_REMOTE_PEER_ID", "AUKI_REMOTE_ROUTE")? {
        Some((peer_id, route)) => {
            let peer_id = peer_id
                .parse::<PeerId>()
                .context("invalid remote Peer ID")?;
            let route = route
                .parse::<Multiaddr>()
                .context("invalid remote relay route")?;
            Ok(Some(RemotePeer { peer_id, route }))
        }
        None => Ok(None),
    }
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
    use auki_portable_echo_adapter::{MAX_CONCURRENCY, protocol_spec};

    #[test]
    fn native_adapter_mounts_the_exact_portable_contract() {
        let spec = protocol_spec().unwrap();
        assert_eq!(spec.protocol_id(), PROTOCOL_ID);
        assert_eq!(spec.max_concurrency(), MAX_CONCURRENCY);
    }
}
