use std::{env, fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use auki_auth::{AuthClient, AuthEnvironment, Credentials, DomainSelection};
use auki_portable_echo_protocol::{
    EchoRequest, ID as ECHO_PROTOCOL_ID, MAX_FRAME_BYTES, run_client, run_server,
};
use auki_sdk::{
    AukiPeer, AukiPeerConfig, AukiProtocolError, AukiProtocolSpec, Identity, Multiaddr, PeerId,
};
use futures::AsyncWriteExt;
use tokio::time::timeout;
use uuid::Uuid;

const MAX_CONCURRENCY: usize = 32;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

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

    let config = AukiPeerConfig::dev();
    let (config, remote_peer) = remote_peer_from_env(config)?;
    let peer = AukiPeer::start(identity, prepared, config).await?;
    let context = peer.protocol_context();
    let protocols = peer.protocols();

    let _echo_registration =
        protocols.register(echo_protocol_spec()?, |mut stream| async move {
            let remote_peer = stream.remote_peer().peer_id;
            match timeout(OPERATION_TIMEOUT, run_server(&mut stream)).await {
                Ok(Ok(request)) => println!(
                    "ECHO_SERVED remote_peer={remote_peer} bytes={}",
                    request.as_bytes().len()
                ),
                Ok(Err(error)) => {
                    eprintln!("ECHO_SERVER_FAILED remote_peer={remote_peer} error={error}")
                }
                Err(_) => eprintln!("ECHO_SERVER_TIMEOUT remote_peer={remote_peer}"),
            }
            let _ = stream.close().await;
        })?;

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
                    "protocols": [ECHO_PROTOCOL_ID],
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

    if let Some(remote_peer) = remote_peer {
        run_echo_client(&protocols, remote_peer).await?;
    }
    if remote_peer.is_none() || env::var_os("AUKI_KEEP_RUNNING").is_some() {
        println!("WAITING_FOR_PEER");
        tokio::signal::ctrl_c().await?;
    }

    peer.shutdown().await?;
    println!("STOPPED");
    Ok(())
}

fn echo_protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(
        ECHO_PROTOCOL_ID,
        MAX_CONCURRENCY,
        u32::try_from(MAX_FRAME_BYTES).expect("the portable frame bound fits in u32"),
    )
}

async fn run_echo_client(
    protocols: &auki_sdk::AukiPeerProtocols,
    remote_peer: PeerId,
) -> Result<()> {
    let message = env::var("AUKI_ECHO_MESSAGE")
        .unwrap_or_else(|_| "hello from the shared Rust protocol".to_owned());
    let request = EchoRequest::new(message.into_bytes())?;
    let mut stream = protocols
        .open(remote_peer, ECHO_PROTOCOL_ID)
        .await
        .context("open authenticated echo stream")?;
    let relayed = stream.is_relayed();

    let exchange = timeout(OPERATION_TIMEOUT, run_client(&mut stream, request)).await;
    let close = stream.close().await;
    let response = exchange.context("echo exchange timed out")??;
    close.context("close authenticated echo stream")?;

    println!(
        "ECHO_OK remote_peer={remote_peer} relayed={relayed} bytes={}",
        response.as_bytes().len()
    );
    Ok(())
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

fn remote_peer_from_env(config: AukiPeerConfig) -> Result<(AukiPeerConfig, Option<PeerId>)> {
    match optional_pair("AUKI_REMOTE_PEER_ID", "AUKI_REMOTE_ROUTE")? {
        Some((peer_id, route)) => {
            let peer_id = peer_id
                .parse::<PeerId>()
                .context("invalid remote Peer ID")?;
            let route = route
                .parse::<Multiaddr>()
                .context("invalid remote relay route")?;
            let config = config.with_peer_routes(peer_id, [route])?;
            Ok((config, Some(peer_id)))
        }
        None => Ok((config, None)),
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

    #[test]
    fn native_adapter_mounts_the_exact_portable_contract() {
        let spec = echo_protocol_spec().unwrap();
        assert_eq!(spec.protocol_id(), ECHO_PROTOCOL_ID);
        assert_eq!(spec.max_concurrency(), MAX_CONCURRENCY);
        assert_eq!(spec.max_frame_bytes(), MAX_FRAME_BYTES as u32);
    }
}
