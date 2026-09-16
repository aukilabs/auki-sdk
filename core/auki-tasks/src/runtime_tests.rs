use super::*;
use auki_auth::machine::token_manager::{TokenProvider, TokenProviderResult};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use httpmock::prelude::*;
use serde_json::json;
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

struct Auth;
#[async_trait]
impl TokenProvider for Auth {
    async fn bearer(&self) -> TokenProviderResult<String> {
        Ok("fixture".into())
    }
    async fn on_unauthorized(&self) {}
}
struct Handler;
#[async_trait]
impl TaskHandler for Handler {
    async fn run(&self, _: TaskContext) -> Result<TaskResult> {
        Ok(TaskResult::default())
    }
}
struct Peer {
    entered: Notify,
    closing: Notify,
    release: Semaphore,
    fail_shutdown: AtomicBool,
}
#[async_trait]
impl TaskPeerSession for Peer {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn fence(&self) {}
    async fn update(&self, _: TaskPeerGrant) -> Result<()> {
        Ok(())
    }
    async fn refresh_requested(&self) {
        std::future::pending::<()>().await;
    }
    async fn wait_stopped(&self) {
        std::future::pending::<()>().await;
    }
    async fn shutdown(&self) -> Result<()> {
        self.closing.notify_one();
        self.release.acquire().await.unwrap().forget();
        if self.fail_shutdown.load(Ordering::Acquire) {
            return Err(TaskError::PeerCleanup);
        }
        Ok(())
    }
}
struct Factory {
    peer: Arc<Peer>,
    dds: url::Url,
    cancel_start: bool,
    cancel_before_handler: Option<CancellationToken>,
}
#[async_trait]
impl TaskPeerFactory for Factory {
    fn peer_id(&self) -> auki_p2p::PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
    }
    fn dds_url(&self) -> &url::Url {
        &self.dds
    }
    fn dms_url(&self) -> &str {
        self.dds.as_str()
    }
    async fn start(
        &self,
        _: TaskPeerGrant,
        cancel: &CancellationToken,
    ) -> Result<Arc<dyn TaskPeerSession>> {
        self.peer.entered.notify_one();
        if let Some(cancel) = &self.cancel_before_handler {
            cancel.cancel();
        }
        if self.cancel_start {
            cancel.cancelled().await;
            self.peer.shutdown().await?;
            return Err(TaskError::Cancelled);
        }
        Ok(self.peer.clone())
    }
}

fn fixture(server: &MockServer, cancel_start: bool) -> (AukiDmsTasks, Arc<Peer>, Uuid) {
    fixture_with_grant(server, cancel_start, "valid")
}

fn fixture_with_grant(
    server: &MockServer,
    cancel_start: bool,
    peer_grant: &str,
) -> (AukiDmsTasks, Arc<Peer>, Uuid) {
    let task = Uuid::new_v4();
    let domain = Uuid::new_v4();
    let expires = Utc::now() + chrono::Duration::seconds(60);
    let claims = json!({"iss": "dds", "aud": [server.base_url()], "domain_id": domain, "exp": expires.timestamp()});
    let token = format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    let mut grant = json!({"task": {"id": task, "capability": "/example/v1"}, "domain_id": domain,
        "domain_server_url": server.base_url(), "access_token": token, "access_token_expires_at": expires,
        "lease_expires_at": expires, "p2p_access_token": "mock-transport-only", "p2p_access_token_expires_at": expires});
    match peer_grant {
        "absent" => {
            grant.as_object_mut().unwrap().remove("p2p_access_token");
            grant
                .as_object_mut()
                .unwrap()
                .remove("p2p_access_token_expires_at");
        }
        "partial" => {
            grant
                .as_object_mut()
                .unwrap()
                .remove("p2p_access_token_expires_at");
        }
        "expired" => {
            grant["p2p_access_token_expires_at"] = json!(Utc::now() - chrono::Duration::seconds(1));
        }
        _ => {}
    }
    server.mock(|when, then| {
        when.method(GET).path("/tasks");
        then.json_body(grant.clone());
    });
    server.mock(|when, then| {
        when.method(POST).path(format!("/tasks/{task}/heartbeat"));
        then.json_body(grant);
    });
    let client = DmsClient::new(
        server.base_url().parse().unwrap(),
        Duration::from_secs(2),
        Arc::new(Auth),
    )
    .unwrap();
    let mut runtime = AukiDmsTasks::from_client(
        client,
        "fixture".into(),
        vec!["/example/v1".into()],
        TasksConfig::default(),
    )
    .unwrap();
    let peer = Arc::new(Peer {
        entered: Notify::new(),
        closing: Notify::new(),
        release: Semaphore::new(0),
        fail_shutdown: AtomicBool::new(false),
    });
    // Inject only the lifecycle adapter here. SDK tests separately verify real
    // signed peer credentials and transport; this fixture has no network peer.
    Arc::get_mut(&mut runtime.0).unwrap().peer = Some(Arc::new(Factory {
        peer: peer.clone(),
        dds: server.base_url().parse().unwrap(),
        cancel_start,
        cancel_before_handler: None,
    }));
    (runtime, peer, task)
}

#[tokio::test]
async fn peer_cleanup_keeps_the_execution_slot_and_precedes_dms_completion() {
    let server = MockServer::start();
    let (runtime, peer, task) = fixture(&server, false);
    let complete = server.mock(|when, then| {
        when.method(POST).path(format!("/tasks/{task}/complete"));
        then.status(200);
    });
    let worker = runtime.clone();
    let running = tokio::spawn(async move {
        worker
            .run_once("/example/v1", &Handler, &CancellationToken::new())
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), peer.closing.notified())
        .await
        .unwrap();
    assert!(!running.is_finished());
    complete.assert_calls(0);
    assert!(matches!(
        runtime
            .claim("/example/v1", &CancellationToken::new())
            .await,
        Err(TaskError::Busy)
    ));
    peer.release.add_permits(1);
    assert_eq!(running.await.unwrap().unwrap(), TaskOutcome::Completed);
    complete.assert_calls(1);
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_during_peer_startup_awaits_cleanup_before_runtime_close() {
    let server = MockServer::start();
    let (runtime, peer, task) = fixture(&server, true);
    let complete = server.mock(|when, then| {
        when.method(POST).path(format!("/tasks/{task}/complete"));
        then.status(200);
    });
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    let worker = runtime.clone();
    let running =
        tokio::spawn(async move { worker.run_once("/example/v1", &Handler, &cancel).await });
    tokio::time::timeout(Duration::from_secs(3), peer.entered.notified())
        .await
        .unwrap();
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(3), peer.closing.notified())
        .await
        .unwrap();
    let closing = tokio::spawn(async move {
        runtime.close().await.unwrap();
    });
    tokio::task::yield_now().await;
    assert!(!closing.is_finished());
    peer.release.add_permits(1);
    assert!(matches!(running.await.unwrap(), Err(TaskError::Cancelled)));
    tokio::time::timeout(Duration::from_secs(3), closing)
        .await
        .unwrap()
        .unwrap();
    complete.assert_calls(0);
}

#[tokio::test]
async fn cancellation_after_peer_start_does_not_start_a_handler_just_to_clean_it_up() {
    let server = MockServer::start();
    let (mut runtime, peer, _) = fixture(&server, false);
    let cancellation = CancellationToken::new();
    Arc::get_mut(&mut runtime.0).unwrap().peer = Some(Arc::new(Factory {
        peer: peer.clone(),
        dds: server.base_url().parse().unwrap(),
        cancel_start: false,
        cancel_before_handler: Some(cancellation.clone()),
    }));
    peer.release.add_permits(1);
    struct MustNotRun;
    #[async_trait]
    impl TaskHandler for MustNotRun {
        async fn run(&self, _: TaskContext) -> Result<TaskResult> {
            panic!("cancelled robot must not begin application work");
        }
    }
    assert!(matches!(
        runtime
            .run_once("/example/v1", &MustNotRun, &cancellation)
            .await,
        Err(TaskError::Cancelled)
    ));
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn peer_cleanup_failure_is_reported_by_run_and_repeatable_close() {
    let server = MockServer::start();
    let (runtime, peer, task) = fixture(&server, false);
    let complete = server.mock(|when, then| {
        when.method(POST).path(format!("/tasks/{task}/complete"));
        then.status(200);
    });
    peer.fail_shutdown.store(true, Ordering::Release);
    peer.release.add_permits(1);
    assert!(matches!(
        runtime
            .run_once("/example/v1", &Handler, &CancellationToken::new())
            .await,
        Err(TaskError::PeerCleanup)
    ));
    for _ in 0..2 {
        assert!(matches!(runtime.close().await, Err(TaskError::PeerCleanup)));
    }
    complete.assert_calls(0);
}

#[tokio::test]
async fn optional_peer_execution_allows_absence_but_rejects_partial_or_expired_authority() {
    for (grant, optional, succeeds) in [
        ("absent", true, true),
        ("absent", false, false),
        ("partial", true, false),
        ("expired", true, false),
    ] {
        let server = MockServer::start();
        let (runtime, peer, task) = fixture_with_grant(&server, false, grant);
        let complete = server.mock(|when, then| {
            when.method(POST).path(format!("/tasks/{task}/complete"));
            then.status(200);
        });
        let cancellation = CancellationToken::new();
        let result = match runtime.claim_any(&cancellation).await {
            Ok(Some(lease)) if optional => {
                lease
                    .execute_with_optional_peer(&Handler, &cancellation)
                    .await
            }
            Ok(Some(lease)) => lease.execute(&Handler, &cancellation).await,
            Err(error) => Err(error),
            Ok(None) => panic!("fixture must supply a lease"),
        };
        assert_eq!(result.is_ok(), succeeds, "{grant}, optional={optional}");
        if !succeeds {
            assert!(matches!(result, Err(TaskError::Authority(_))));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(10), peer.entered.notified())
                .await
                .is_err()
        );
        complete.assert_calls(usize::from(succeeds));
        runtime.close().await.unwrap();
    }
}
