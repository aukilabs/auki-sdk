//! Synthetic local acceptance host. No login UI and no shared service defaults.
use anyhow::{Result, ensure};
use auki_protocols::info::{InfoClient, InfoEndpoint, v1::AuthenticatedParticipantInfo};
use auki_sdk::{
    AukiPeerBootstrap, AukiPeerConfig, AuthClient, AuthEnvironment, AuthError, AuthFailureKind,
    AuthSession, Credentials, DdsTrackerMode, DomainSelection, Identity, ZitadelSessionCredentials,
    ZitadelSessionStore,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

const FIXTURE: &str = "http://127.0.0.1:18123";
const API: &str = "http://127.0.0.1:18120";
const DDS: &str = "http://127.0.0.1:18121";
const DMS: &str = "http://127.0.0.1:18122/v1/";
const INFO: &str = auki_protocols::info::v1::ID;

// Deliberately no Debug implementation for a secret-bearing host payload.
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
    fn credentials(self) -> Result<ZitadelSessionCredentials> {
        Ok(ZitadelSessionCredentials::new(
            self.access_token,
            self.refresh_token,
            self.client_id,
            self.issuer.parse()?,
            self.access_token_expires_at,
        )?)
    }
    fn replacement(c: &ZitadelSessionCredentials) -> Self {
        Self {
            access_token: c.access_token().expose_secret().into(),
            refresh_token: c.refresh_token().expose_secret().into(),
            client_id: c.client_id().into(),
            issuer: c.issuer().to_string(),
            access_token_expires_at: c.access_token_expires_at(),
        }
    }
}
struct Store {
    path: PathBuf,
    fail: AtomicBool,
    saves: AtomicU64,
}
#[async_trait::async_trait]
impl ZitadelSessionStore for Store {
    async fn save(&self, credentials: &ZitadelSessionCredentials) -> Result<(), AuthError> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(AuthError::Persistence);
        }
        let save = || -> Result<()> {
            let pending = self.path.with_extension("pending");
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&pending)?;
            file.write_all(&serde_json::to_vec(&Payload::replacement(credentials))?)?;
            file.sync_all()?;
            fs::rename(pending, &self.path)?;
            File::open(self.path.parent().unwrap())?.sync_all()?;
            Ok(())
        };
        save().map_err(|_| AuthError::Persistence)
    }
}
async fn get(path: &str) -> Result<Value> {
    Ok(reqwest::get(format!("{FIXTURE}{path}"))
        .await?
        .error_for_status()?
        .json()
        .await?)
}
async fn post(path: &str, value: Value) -> Result<()> {
    reqwest::Client::new()
        .post(format!("{FIXTURE}{path}"))
        .json(&value)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
fn client() -> Result<AuthClient> {
    Ok(AuthClient::new(AuthEnvironment::new(API, DDS)?)?)
}
async fn import(name: &str, path: PathBuf) -> Result<(AuthSession, Arc<Store>)> {
    let payload: Payload = serde_json::from_value(get(&format!("/__credentials/{name}")).await?)?;
    let store = Arc::new(Store {
        path,
        fail: AtomicBool::new(false),
        saves: AtomicU64::new(0),
    });
    Ok((
        client()?.import_zitadel_session(payload.credentials()?, store.clone())?,
        store,
    ))
}
async fn authorize(session: &AuthSession, domain: Uuid) -> Result<(), AuthError> {
    session
        .authorize_peer(domain.into(), &Identity::generate().proof())
        .await
        .map(|_| ())
}
async fn expect_failure(session: &AuthSession, domain: Uuid, kind: AuthFailureKind) -> Result<()> {
    let result = authorize(session, domain).await;
    ensure!(result.is_err(), "expected auth failure");
    ensure!(
        result.unwrap_err().kind() == kind,
        "wrong auth failure classification"
    );
    Ok(())
}
async fn refreshes(name: &str) -> Result<u64> {
    Ok(get("/__stats").await?["grants"][name]["refreshes"]
        .as_u64()
        .unwrap())
}

async fn recovery_cases(dir: &std::path::Path, seed: &HashMap<String, String>) -> Result<()> {
    let domain: Uuid = seed["domainId"].parse()?;
    let other_domain: Uuid = seed["otherDomainId"].parse()?;
    let foreign_domain: Uuid = seed["foreignDomainId"].parse()?;
    let (session, store) = import("native-save", dir.join("native-save.json")).await?;
    store.fail.store(true, Ordering::SeqCst);
    expect_failure(&session, domain, AuthFailureKind::Persistence).await?;
    store.fail.store(false, Ordering::SeqCst);
    authorize(&session, domain).await?;
    ensure!(
        refreshes("native-save").await? == 1 && store.saves.load(Ordering::SeqCst) == 2,
        "persistence replayed refresh"
    );
    session.close().await;
    let saved: Payload = serde_json::from_slice(&fs::read(&store.path)?)?;
    let restarted = client()?.import_zitadel_session(saved.credentials()?, store.clone())?;
    expect_failure(
        &restarted,
        other_domain,
        AuthFailureKind::AuthorizationDenied,
    )
    .await?;
    authorize(&restarted, domain).await?;
    ensure!(
        refreshes("native-save").await? == 1,
        "restart replayed refresh"
    );
    restarted.close().await;
    fs::remove_file(&store.path)?;

    let (session, store) = import("native-startup", dir.join("native-startup.json")).await?;
    post(
        "/__configure",
        json!({"grant":"native-startup", "policy":"outage"}),
    )
    .await?;
    expect_failure(&session, domain, AuthFailureKind::Transient).await?;
    post(
        "/__configure",
        json!({"grant":"native-startup", "policy":null}),
    )
    .await?;
    authorize(&session, domain).await?;
    ensure!(
        refreshes("native-startup").await? == 1,
        "startup retry lost refresh owner"
    );
    session.close().await;
    fs::remove_file(&store.path)?;

    let (session, store) = import("native-cancel", dir.join("native-cancel.json")).await?;
    post(
        "/__configure",
        json!({"grant":"native-cancel", "refreshDelayMs":1000}),
    )
    .await?;
    let observer_session = session.clone();
    let observer = tokio::spawn(async move { authorize(&observer_session, domain).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while refreshes("native-cancel").await? == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    // The fixture has consumed the rotating credential, but has not replied.
    ensure!(
        !observer.is_finished(),
        "observer completed before cancellation barrier"
    );
    observer.abort();
    ensure!(
        observer.await.unwrap_err().is_cancelled(),
        "observer was not cancelled"
    );
    authorize(&session, domain).await?;
    ensure!(
        refreshes("native-cancel").await? == 1,
        "cancellation replayed consumed refresh"
    );
    session.close().await;
    fs::remove_file(&store.path)?;

    let (session, _) = import("native-invalid", dir.join("native-invalid.json")).await?;
    post(
        "/__configure",
        json!({"grant":"native-invalid", "error":"invalid_grant"}),
    )
    .await?;
    expect_failure(&session, domain, AuthFailureKind::AuthenticationRequired).await?;
    expect_failure(&session, domain, AuthFailureKind::AuthenticationRequired).await?;
    ensure!(
        refreshes("native-invalid").await? == 1,
        "terminal auth retried"
    );
    session.close().await;

    for (name, count) in [("denied-native", 0), ("org-native", 2)] {
        let (session, store) = import(name, dir.join(format!("{name}.json"))).await?;
        if count == 0 {
            expect_failure(&session, domain, AuthFailureKind::AuthorizationDenied).await?;
        } else {
            let boot = AukiPeerBootstrap::from_session(
                session.clone(),
                AukiPeerConfig::new(DMS)?.without_relay(),
            );
            // Two different Domains share this org user's one renewable session.
            let (first, second) = tokio::try_join!(
                boot.start_ephemeral_peer(domain.into()),
                boot.start_ephemeral_peer(other_domain.into()),
            )?;
            ensure!(
                first.peer_id() != second.peer_id(),
                "Domain peers reused an identity"
            );
            ensure!(
                refreshes(name).await? == 1,
                "multi-Domain start duplicated rotation"
            );
            expect_failure(
                &session,
                foreign_domain,
                AuthFailureKind::AuthorizationDenied,
            )
            .await?;
            first.shutdown().await?;
            second.shutdown().await?;
        }
        session.close().await;
        fs::remove_file(&store.path)?;
    }
    // Actual legacy API password/app issuance and DDS possession proof; no
    // pre-minted local bearer or fake authorizer substitutes for those paths.
    for credentials in [
        Credentials::user_password(&seed["legacyEmail"], seed["legacyPassword"].as_str()),
        Credentials::app(&seed["appKey"], seed["appSecret"].as_str()),
    ] {
        let session = client()?.authenticate(credentials).await?;
        let boot = AukiPeerBootstrap::from_session(
            session.clone(),
            AukiPeerConfig::new(DMS)?.without_relay(),
        );
        let peer = boot
            .start_ephemeral_peer(DomainSelection::new(seed["domainId"].parse::<Uuid>()?))
            .await?;
        peer.shutdown().await?;
        session.close().await;
    }
    post(
        "/__event",
        json!({"runtime":"native", "event":"recovery-passed", "cases":7}),
    )
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let result = run().await;
    if result.is_err() {
        let _ = post("/__event", json!({"runtime":"native", "event":"failed"})).await;
    }
    result
}
async fn run() -> Result<()> {
    let dir = PathBuf::from(std::env::var("Z10_RUN_DIR")?);
    let seed: HashMap<String, String> = serde_json::from_slice(&fs::read(dir.join("seed.json"))?)?;
    recovery_cases(&dir, &seed).await?;
    let (session, store) = import("native-live", dir.join("native-live.json")).await?;
    let boot = AukiPeerBootstrap::from_session(session.clone(), AukiPeerConfig::new(DMS)?)
        .with_dds_tracker(DdsTrackerMode::DiscoverAndAdvertise);
    let selection = DomainSelection::new(seed["domainId"].parse::<Uuid>()?);
    let (first, second) = tokio::try_join!(
        boot.start_ephemeral_peer(selection),
        boot.start_ephemeral_peer(selection)
    )?;
    let peers = [first, second];
    let ids = peers
        .iter()
        .map(|p| p.peer_id().to_string())
        .collect::<Vec<_>>();
    let mut endpoints = Vec::new();
    for peer in &peers {
        let peer_id = peer.peer_id();
        endpoints.push(InfoEndpoint::mount(
            peer.protocols(),
            move |requester: &auki_sdk::AuthenticatedPeer| {
                Some(AuthenticatedParticipantInfo {
                    app: "zitadel-acceptance".into(),
                    app_version: "0.1.0".into(),
                    name: "native".into(),
                    session_id: requester.subject.clone(),
                    session_clock_id: "z10".into(),
                    session_clock_hash: "z10".into(),
                    session_now_ns: 0,
                    peer_id,
                    app_instance: "native".into(),
                })
            },
        )?);
    }
    ensure!(
        refreshes("native-live").await? == 1,
        "concurrent starts rotated twice"
    );
    post(
        "/__event",
        json!({"runtime":"native", "event":"ready", "peerIds":ids}),
    )
    .await?;
    let started = std::time::Instant::now();
    let mut successes = 0_u64;
    loop {
        if get("/__config").await?["stop"] == true {
            break;
        }
        ensure!(
            started.elapsed() < Duration::from_secs(95 * 60),
            "native acceptance deadline"
        );
        ensure!(
            peers.iter().all(|p| !p.status().is_terminal()),
            "terminal native runtime"
        );
        let mut browsers = 0;
        for peer in &peers {
            let info = InfoClient::new(peer.protocols());
            for candidate in peer.discover_protocol(INFO).await? {
                let Some(route) = candidate.routes().iter().find(|r| {
                    r.to_string().contains("/p2p-circuit/") && !r.to_string().contains("/wss/")
                }) else {
                    continue;
                };
                match info.fetch_exact(candidate.peer_id(), route.clone()).await {
                    Ok(value) => {
                        ensure!(
                            value.session_id == seed["subject"],
                            "remote did not authenticate exact opaque subject"
                        );
                        successes += 1;
                        if value.name == "browser" {
                            browsers += 1;
                        }
                    }
                    Err(_) => { /* brief reachability transitions are bounded by the controller's last-success check */
                    }
                }
            }
        }
        ensure!(
            peers
                .iter()
                .map(|p| p.peer_id().to_string())
                .collect::<Vec<_>>()
                == ids,
            "Peer ID changed"
        );
        post(
            "/__event",
            json!({"runtime":"native", "event":"probe", "peerIds":ids, "successes":successes,
            "browserProbes":browsers, "saves":store.saves.load(Ordering::SeqCst)}),
        )
        .await?;
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    for endpoint in endpoints {
        endpoint.close().await?;
    }
    for peer in peers {
        peer.shutdown().await?;
    }
    session.close().await;
    fs::remove_file(&store.path)?;
    post(
        "/__event",
        json!({"runtime":"native", "event":"stopped", "successes":successes}),
    )
    .await?;
    Ok(())
}
