//! Real-dev native smoke host. Credentials arrive only on stdin, never argv/logs.
use anyhow::{Result, anyhow, ensure};
use auki_protocols::info::{InfoClient, InfoEndpoint, v1::AuthenticatedParticipantInfo};
use auki_sdk::{
    AukiPeer, AukiPeerBootstrap, AukiPeerBootstrapError, AukiPeerConfig, AukiRelayConfig,
    AuthClient, AuthEnvironment, AuthError, AuthFailureKind, AuthSession, DdsTrackerMode,
    DomainSelection, ZitadelSessionCredentials, ZitadelSessionStore,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

// Secret-bearing types intentionally do not implement Debug.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    access_token: String,
    refresh_token: String,
    client_id: String,
    issuer: String,
    access_token_expires_at: Option<DateTime<Utc>>,
}

impl Payload {
    fn credentials(self, exercise_refresh: bool) -> Result<ZitadelSessionCredentials> {
        ensure!(
            matches!(
                self.issuer.as_str(),
                "https://auth.dev.aukiverse.com" | "https://auth.dev.aukiverse.com/"
            ),
            "unexpected issuer"
        );
        // Explicit test-only import hint. The server's token lifetimes are untouched.
        let expiry = if exercise_refresh {
            Some(Utc::now() - chrono::Duration::seconds(1))
        } else {
            self.access_token_expires_at
        };
        Ok(ZitadelSessionCredentials::new(
            self.access_token,
            self.refresh_token,
            self.client_id,
            self.issuer.parse().map_err(|_| anyhow!("invalid issuer"))?,
            expiry,
        )?)
    }

    fn replacement(value: &ZitadelSessionCredentials) -> Self {
        Self {
            access_token: value.access_token().expose_secret().into(),
            refresh_token: value.refresh_token().expose_secret().into(),
            client_id: value.client_id().into(),
            issuer: value.issuer().to_string(),
            access_token_expires_at: value.access_token_expires_at(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    domain_id: Uuid,
    credentials: Payload,
    credentials_path: PathBuf,
    seed_hash: String,
    exercise_refresh: bool,
}

struct Store {
    path: PathBuf,
    seed_hash: String,
    saves: AtomicU64,
    can_resume: AtomicBool,
}

#[async_trait::async_trait]
impl ZitadelSessionStore for Store {
    async fn save(&self, credentials: &ZitadelSessionCredentials) -> Result<(), AuthError> {
        let save = || -> Result<()> {
            let pending = self
                .path
                .with_extension(format!("{}.pending", Uuid::new_v4()));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&pending)?;
            let bytes = serde_json::to_vec(&json!({
                "seedHash": self.seed_hash,
                "credentials": Payload::replacement(credentials),
            }))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(pending, &self.path)?;
            File::open(
                self.path
                    .parent()
                    .ok_or_else(|| anyhow!("state parent missing"))?,
            )?
            .sync_all()?;
            Ok(())
        };
        if save().is_err() {
            self.can_resume.store(false, Ordering::SeqCst);
            return Err(AuthError::Persistence);
        }
        let saves = self.saves.fetch_add(1, Ordering::SeqCst) + 1;
        println!("{}", json!({"event":"credentials-saved", "saves":saves}));
        Ok(())
    }
}

fn auth_error(error: AuthError, store: &Store) -> anyhow::Error {
    if !matches!(
        error.kind(),
        AuthFailureKind::AuthorizationDenied | AuthFailureKind::Configuration
    ) {
        store.can_resume.store(false, Ordering::SeqCst);
    }
    // AuthError intentionally contains no remote body or credential material.
    anyhow!("{:?}: {}", error.kind(), error)
}

fn bootstrap_error(error: AukiPeerBootstrapError, store: &Store) -> anyhow::Error {
    match error {
        AukiPeerBootstrapError::AuthorizePeer(error) => auth_error(error, store),
        // Display the outer diagnostic only, never arbitrary response bodies or error chains.
        other => anyhow!("{}", other),
    }
}

async fn smoke(
    session: &AuthSession,
    store: &Store,
    domain: Uuid,
    peers: &mut Vec<AukiPeer>,
    endpoints: &mut Vec<InfoEndpoint>,
) -> Result<()> {
    let relay = AukiRelayConfig {
        requested_duration: Duration::from_secs(300),
        ..AukiRelayConfig::default()
    };
    let config = AukiPeerConfig::dev().with_relay(relay)?;
    let boot = AukiPeerBootstrap::from_session(session.clone(), config)
        .with_dds_tracker(DdsTrackerMode::DiscoverAndAdvertise);
    for index in 0..2 {
        println!("{}", json!({"event":"peer-starting", "index":index}));
        // Sequential starts keep every completed owner available for awaited cleanup on failure.
        let peer = boot
            .start_ephemeral_peer(DomainSelection::new(domain))
            .await
            .map_err(|error| bootstrap_error(error, store))?;
        let peer_id = peer.peer_id();
        peers.push(peer);
        endpoints.push(
            InfoEndpoint::mount(
                peers[index].protocols(),
                move |_requester: &auki_sdk::AuthenticatedPeer| {
                    Some(AuthenticatedParticipantInfo {
                        app: "zitadel-dev-native".into(),
                        app_version: "0.1.0".into(),
                        name: format!("native-{index}"),
                        session_id: "native-smoke".into(),
                        session_clock_id: "native-smoke".into(),
                        session_clock_hash: "native-smoke".into(),
                        session_now_ns: 0,
                        peer_id,
                        app_instance: "native-smoke".into(),
                    })
                },
            )
            .map_err(|_| anyhow!("Info endpoint mount failed"))?,
        );
        println!(
            "{}",
            json!({"event":"peer-ready", "index":index, "peerId":peer_id.to_string()})
        );
    }
    let started = Instant::now();
    let mut successes = [0_u64; 2];
    while started.elapsed() < Duration::from_secs(120) {
        ensure!(
            peers.iter().all(|peer| !peer.status().is_terminal()),
            "peer became terminal"
        );
        for index in 0..2 {
            let target = peers[1 - index].peer_id();
            let candidates = peers[index]
                .discover_protocol(auki_protocols::info::v1::ID)
                .await
                .map_err(|_| anyhow!("DDS protocol discovery failed"))?;
            // Never probe unrelated dev participants. Force a native TCP relay circuit.
            let route = candidates
                .iter()
                .find(|candidate| candidate.peer_id() == target)
                .and_then(|candidate| {
                    candidate.routes().iter().find(|route| {
                        let route = route.to_string();
                        route.contains("/p2p-circuit/") && !route.contains("/wss/")
                    })
                })
                .cloned();
            if let Some(route) = route {
                let result = tokio::time::timeout(
                    Duration::from_secs(15),
                    InfoClient::new(peers[index].protocols()).fetch_exact(target, route),
                )
                .await;
                if let Ok(Ok(info)) = result {
                    ensure!(
                        info.peer_id == target && info.app == "zitadel-dev-native",
                        "unexpected Info response"
                    );
                    successes[index] += 1;
                }
            }
        }
        println!("{}", json!({"event":"relay-probe", "successes":successes}));
        if successes.iter().all(|count| *count >= 3) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    anyhow::bail!("bidirectional native relay probes did not converge within 120 seconds")
}

async fn run() -> Result<()> {
    let mut input = String::new();
    std::io::stdin()
        .take(64 * 1024)
        .read_to_string(&mut input)
        .map_err(|_| anyhow!("could not read private input"))?;
    let config: Config =
        serde_json::from_str(&input).map_err(|_| anyhow!("invalid private input"))?;
    drop(input);
    let store = Arc::new(Store {
        path: config.credentials_path,
        seed_hash: config.seed_hash,
        saves: AtomicU64::new(0),
        can_resume: AtomicBool::new(true),
    });
    let client = AuthClient::new(AuthEnvironment::dev())?;
    let session = client.import_zitadel_session(
        config.credentials.credentials(config.exercise_refresh)?,
        store.clone(),
    )?;
    println!(
        "{}",
        json!({"event":"session-imported", "exerciseRefresh":config.exercise_refresh})
    );
    let mut peers = Vec::new();
    let mut endpoints = Vec::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = tokio::select! {
        result = smoke(&session, &store, config.domain_id, &mut peers, &mut endpoints) => result,
        _ = tokio::signal::ctrl_c() => {
            store.can_resume.store(false, Ordering::SeqCst);
            Err(anyhow!("test interrupted; in-flight startup bookings may expire by TTL"))
        },
        _ = terminate.recv() => {
            store.can_resume.store(false, Ordering::SeqCst);
            Err(anyhow!("test terminated; in-flight startup bookings may expire by TTL"))
        },
    };
    let mut cleanup_ok = true;
    for endpoint in endpoints {
        cleanup_ok &= endpoint.close().await.is_ok();
    }
    for peer in peers {
        cleanup_ok &= peer.shutdown().await.is_ok();
    }
    session.close().await;
    let saves = store.saves.load(Ordering::SeqCst);
    println!(
        "{}",
        json!({"event":"session-closed", "canResume":store.can_resume.load(Ordering::SeqCst),
        "cleanupOk":cleanup_ok, "credentialSaves":saves})
    );
    ensure!(
        cleanup_ok,
        "peer cleanup failed; inspect this run's bookings/advertisements"
    );
    result?;
    ensure!(
        !config.exercise_refresh || saves > 0,
        "explicit refresh was not durably saved"
    );
    Ok(())
}

#[tokio::main]
async fn main() {
    match run().await {
        Ok(()) => println!("{}", json!({"event":"passed"})),
        Err(error) => {
            eprintln!("{}", json!({"event":"failed", "reason":error.to_string()}));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    #[tokio::test]
    async fn replacement_is_private_complete_and_reimportable() {
        let dir =
            std::env::temp_dir().join(format!("zitadel-native-store-test-{}", Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let store = Store {
            path: dir.join("session.json"),
            seed_hash: "synthetic".into(),
            saves: AtomicU64::new(0),
            can_resume: AtomicBool::new(true),
        };
        for generation in 0..2 {
            let credentials = ZitadelSessionCredentials::new(
                format!("synthetic-access-{generation}"),
                format!("synthetic-refresh-{generation}"),
                "synthetic-client",
                "https://auth.dev.aukiverse.com".parse().unwrap(),
                Some(Utc::now() + chrono::Duration::hours(1)),
            )
            .unwrap();
            store.save(&credentials).await.unwrap();
            assert_eq!(
                fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
            assert_eq!(saved["seedHash"], "synthetic");
            let payload: Payload = serde_json::from_value(saved["credentials"].clone()).unwrap();
            let imported = payload.credentials(false).unwrap();
            assert_eq!(
                imported.refresh_token().expose_secret(),
                credentials.refresh_token().expose_secret()
            );
            assert_eq!(
                imported.access_token_expires_at(),
                credentials.access_token_expires_at()
            );
        }
        assert_eq!(store.saves.load(Ordering::SeqCst), 2);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_file(&store.path).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn unsafe_refresh_outcomes_disallow_seed_replay() {
        let store = Store {
            path: PathBuf::new(),
            seed_hash: String::new(),
            saves: AtomicU64::new(0),
            can_resume: AtomicBool::new(true),
        };
        let _ = auth_error(AuthError::AuthorizationDenied, &store);
        assert!(store.can_resume.load(Ordering::SeqCst));
        let _ = auth_error(AuthError::SessionOperationPending, &store);
        assert!(!store.can_resume.load(Ordering::SeqCst));
    }
}
