use std::{
    collections::BTreeMap,
    env,
    io::{self, Write},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use auki_camera_mesh::{
    CAMERA_RATE_HZ, CameraEvent, CameraProtocols, CameraRole, PeerCard, deterministic_jpeg,
    synthetic_jpegs,
};
use auki_sdk::{
    AukiDiscovery, AukiPeerBootstrap, Credentials, DdsTrackerMode, DomainSelection, PeerId,
};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum Command {
    Discover {
        id: String,
        #[serde(default)]
        protocol: Option<String>,
    },
    Approve {
        id: String,
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    View {
        id: String,
        target: PeerCard,
        #[serde(default = "default_frame_count")]
        frames: usize,
    },
    ExerciseLive {
        id: String,
        target: PeerCard,
        #[serde(rename = "requestId", default)]
        request_id: Option<String>,
    },
    Pause {
        id: String,
        target: PeerCard,
    },
    Resume {
        id: String,
        target: PeerCard,
    },
    Snapshot {
        id: String,
        target: PeerCard,
        #[serde(rename = "requestId", default)]
        request_id: Option<String>,
    },
    Shutdown {
        id: String,
    },
}

fn default_frame_count() -> usize {
    3
}

#[tokio::main]
async fn main() -> Result<()> {
    let role = env::var("AUKI_CAMERA_ROLE")
        .unwrap_or_else(|_| "publisher".into())
        .parse::<CameraRole>()?;
    let auto_approve_same_domain = env_flag("AUKI_CAMERA_AUTO_APPROVE")?;
    let domain_id = required_env("AUKI_DOMAIN_ID")?
        .parse::<Uuid>()
        .context("AUKI_DOMAIN_ID must be a UUID")?;
    let identity_file = PathBuf::from(required_env("AUKI_IDENTITY_FILE")?);
    let bootstrap = AukiPeerBootstrap::dev(credentials_from_env()?)
        .await?
        .with_dds_tracker(discovery_mode_from_env(role)?);
    let peer = bootstrap
        .start_persistent_peer(DomainSelection::new(domain_id), identity_file)
        .await?;
    let display_name =
        env::var("AUKI_NODE_NAME").unwrap_or_else(|_| format!("native-camera-{}", role.as_str()));
    let (protocols, events) = match CameraProtocols::mount(&peer, role, display_name).await {
        Ok(value) => value,
        Err(error) => {
            let _ = peer.shutdown().await;
            return Err(error);
        }
    };
    protocols.set_auto_approve_same_domain(auto_approve_same_domain);
    emit(&serde_json::json!({
        "event": "ready",
        "runtime": "native",
        "role": role,
        "card": protocols.card(),
    }))?;

    let discovery = peer.discovery_handle()?;
    let operation = tokio::select! {
        result = command_loop(&protocols, &discovery, events) => result,
        signal = tokio::signal::ctrl_c() => signal.context("wait for Ctrl-C"),
    };
    let protocol_shutdown = protocols.close().await;
    let peer_shutdown = peer.shutdown().await.map_err(anyhow::Error::from);
    finish(
        operation,
        [
            ("Camera Mesh endpoints", protocol_shutdown),
            ("Auki peer", peer_shutdown),
        ],
    )?;
    emit(&serde_json::json!({"event":"stopped","runtime":"native"}))?;
    Ok(())
}

async fn command_loop(
    protocols: &CameraProtocols,
    discovery: &AukiDiscovery,
    mut events: tokio::sync::mpsc::Receiver<CameraEvent>,
) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let frames = camera_frames(protocols.role())?;
    let mut frame_index = 0;
    let mut frame_tick = tokio::time::interval(std::time::Duration::from_millis(
        1_000 / u64::from(CAMERA_RATE_HZ),
    ));
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = frame_tick.tick(), if !frames.is_empty() => {
                protocols.replace_frame(frames[frame_index].clone())?;
                frame_index = (frame_index + 1) % frames.len();
            }
            event = events.recv() => {
                if let Some(event) = event {
                    emit(&serde_json::to_value(event)?)?;
                }
            }
            line = lines.next_line() => {
                let Some(line) = line.context("read JSONL command")? else {
                    return Ok(());
                };
                let command = match serde_json::from_str::<Command>(&line) {
                    Ok(command) => command,
                    Err(error) => {
                        emit(&serde_json::json!({"event":"command_error","error":format!("invalid command: {error}")}))?;
                        continue;
                    }
                };
                if handle_command(protocols, discovery, command).await? {
                    return Ok(());
                }
            }
        }
    }
}

fn camera_frames(role: CameraRole) -> Result<Vec<Vec<u8>>> {
    if role != CameraRole::Publisher {
        return Ok(Vec::new());
    }
    let mode = match env::var("AUKI_CAMERA_FRAME_MODE") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => "animated".into(),
        Err(error) => return Err(error).context("read AUKI_CAMERA_FRAME_MODE"),
    };
    match mode.as_str() {
        "still" => Ok(vec![deterministic_jpeg()?]),
        "animated" => synthetic_jpegs(),
        value => bail!("AUKI_CAMERA_FRAME_MODE must be animated or still, got {value:?}"),
    }
}

async fn handle_command(
    protocols: &CameraProtocols,
    discovery: &AukiDiscovery,
    command: Command,
) -> Result<bool> {
    match command {
        Command::Discover { id, protocol } => {
            match protocols.discover(discovery, protocol.as_deref()).await {
                Ok(candidates) => emit(&serde_json::json!({
                    "event":"discovery_result","id":id,"ok":true,
                    "protocol":protocol,"candidates":candidates,
                }))?,
                Err(error) => emit(&serde_json::json!({
                    "event":"discovery_result","id":id,"ok":false,
                    "protocol":protocol,"candidates":[],"error":format!("{error:#}"),
                }))?,
            }
        }
        Command::Approve { id, peer_id } => match peer_id.parse::<PeerId>() {
            Ok(peer_id) => {
                protocols.approve(peer_id);
                emit(&serde_json::json!({
                    "event":"approve_result","id":id,"ok":true,"peerId":peer_id,
                }))?;
            }
            Err(error) => emit(&serde_json::json!({
                "event":"approve_result","id":id,"ok":false,"peerId":peer_id,
                "error":format!("invalid Peer ID: {error}"),
            }))?,
        },
        Command::View { id, target, frames } => {
            let target_peer_id = target.peer_id.clone();
            match protocols.view(&target, frames).await {
                Ok(report) => emit(&serde_json::json!({
                    "event":"view_result","id":id,"ok":true,
                    "targetPeerId":target_peer_id,"checks":report.checks,
                    "frames":report.frames,"frameSha256":report.frame_sha256,
                    "frameBytes":report.frame_bytes,
                }))?,
                Err(error) => emit(&serde_json::json!({
                    "event":"view_result","id":id,"ok":false,
                    "targetPeerId":target_peer_id,
                    "checks":failed_view_checks(),
                    "frames":0,
                    "error":format!("{error:#}"),
                }))?,
            }
        }
        Command::ExerciseLive {
            id,
            target,
            request_id,
        } => {
            let target_peer_id = target.peer_id.clone();
            match protocols.exercise_live(&target, request_id).await {
                Ok(report) => emit(&serde_json::json!({
                    "event":"exercise_live_result","id":id,"ok":true,
                    "report":report,
                }))?,
                Err(error) => emit(&serde_json::json!({
                    "event":"exercise_live_result","id":id,"ok":false,
                    "targetPeerId":target_peer_id,"error":format!("{error:#}"),
                }))?,
            }
        }
        Command::Pause { id, target } => {
            emit_control_result(
                &id,
                "camera.pause",
                &target.peer_id,
                protocols.send_pause(&target).await,
            )?;
        }
        Command::Resume { id, target } => {
            emit_control_result(
                &id,
                "camera.resume",
                &target.peer_id,
                protocols.send_resume(&target).await,
            )?;
        }
        Command::Snapshot {
            id,
            target,
            request_id,
        } => {
            let target_peer_id = target.peer_id.clone();
            let request_id = request_id.unwrap_or_else(|| Uuid::new_v4().to_string());
            match protocols
                .request_snapshot(&target, Some(request_id.clone()))
                .await
            {
                Ok(report) => emit(&serde_json::json!({
                    "event":"snapshot_result","id":id,"ok":true,
                    "requestId":report.request_id,"targetPeerId":report.target_peer_id,
                    "sha256":report.sha256,"size":report.size,
                }))?,
                Err(error) => emit(&serde_json::json!({
                    "event":"snapshot_result","id":id,"ok":false,
                    "requestId":request_id,"targetPeerId":target_peer_id,
                    "error":format!("{error:#}"),
                }))?,
            }
        }
        Command::Shutdown { id } => {
            emit(&serde_json::json!({"event":"shutdown_ack","id":id}))?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn emit_control_result(
    id: &str,
    control: &str,
    target_peer_id: &str,
    result: Result<()>,
) -> Result<()> {
    match result {
        Ok(()) => emit(&serde_json::json!({
            "event":"control_result","id":id,"ok":true,
            "control":control,"targetPeerId":target_peer_id,
        })),
        Err(error) => emit(&serde_json::json!({
            "event":"control_result","id":id,"ok":false,
            "control":control,"targetPeerId":target_peer_id,"error":format!("{error:#}"),
        })),
    }
}

fn failed_view_checks() -> BTreeMap<&'static str, bool> {
    ["info", "catalog", "registry", "stream"]
        .into_iter()
        .map(|check| (check, false))
        .collect()
}

fn emit(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    io::stdout().flush().context("flush JSONL event")
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

fn discovery_mode_from_env(role: CameraRole) -> Result<DdsTrackerMode> {
    match env::var("AUKI_DISCOVERY_MODE") {
        Ok(value) if value == "discover_only" => Ok(DdsTrackerMode::DiscoverOnly),
        Ok(value) if value == "discover_and_advertise" => Ok(DdsTrackerMode::DiscoverAndAdvertise),
        Ok(value) => bail!(
            "AUKI_DISCOVERY_MODE must be discover_only or discover_and_advertise, got {value:?}"
        ),
        Err(env::VarError::NotPresent) => Ok(match role {
            CameraRole::Publisher => DdsTrackerMode::DiscoverAndAdvertise,
            CameraRole::Viewer => DdsTrackerMode::DiscoverOnly,
        }),
        Err(error) => Err(error).context("read AUKI_DISCOVERY_MODE"),
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

fn env_flag(name: &'static str) -> Result<bool> {
    match env::var(name) {
        Ok(value) => parse_env_flag(name, &value),
        Err(env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(error).with_context(|| format!("read {name}")),
    }
}

fn parse_env_flag(name: &'static str, value: &str) -> Result<bool> {
    match value {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        value => bail!("{name} must be 1, true, 0, or false; got {value:?}"),
    }
}

fn finish<const N: usize>(operation: Result<()>, cleanup: [(&str, Result<()>); N]) -> Result<()> {
    let cleanup_errors = cleanup
        .into_iter()
        .filter_map(|(name, result)| result.err().map(|error| format!("{name}: {error:#}")))
        .collect::<Vec<_>>();
    match (operation, cleanup_errors.is_empty()) {
        (Ok(()), true) => Ok(()),
        (Ok(()), false) => bail!("ordered shutdown failed: {}", cleanup_errors.join("; ")),
        (Err(error), true) => Err(error),
        (Err(error), false) => Err(error.context(format!(
            "cleanup also failed: {}",
            cleanup_errors.join("; ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "runtime": "native",
            "domainId": "4e990513-b110-467b-84ca-09a42d786f6d",
            "peerId": "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan",
            "protocols": [],
            "routes": {
                "tcp": "/ip4/127.0.0.1/tcp/9000/p2p/12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan",
                "wss": "/dns4/relay.example.com/tcp/443/wss/p2p/12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            }
        })
    }

    #[test]
    fn jsonl_defaults_match_the_cross_runtime_contract() {
        let discover: Command =
            serde_json::from_value(serde_json::json!({"command":"discover","id":"d"})).unwrap();
        assert!(matches!(discover, Command::Discover { protocol: None, .. }));

        let view: Command = serde_json::from_value(serde_json::json!({
            "command":"view", "id":"v", "target":target()
        }))
        .unwrap();
        assert!(matches!(view, Command::View { frames: 3, .. }));

        let exercise: Command = serde_json::from_value(serde_json::json!({
            "command":"exercise_live", "id":"live", "target":target(),
            "requestId":"acceptance-snapshot"
        }))
        .unwrap();
        assert!(matches!(
            exercise,
            Command::ExerciseLive {
                id,
                request_id: Some(request_id),
                ..
            } if id == "live" && request_id == "acceptance-snapshot"
        ));

        assert_eq!(
            failed_view_checks(),
            [
                ("info", false),
                ("catalog", false),
                ("registry", false),
                ("stream", false),
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn environment_flags_are_explicit() {
        assert!(parse_env_flag("FLAG", "1").unwrap());
        assert!(parse_env_flag("FLAG", "true").unwrap());
        assert!(!parse_env_flag("FLAG", "0").unwrap());
        assert!(!parse_env_flag("FLAG", "false").unwrap());
        assert!(parse_env_flag("FLAG", "yes").is_err());
    }
}
