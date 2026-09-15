use super::*;
use async_trait::async_trait;
use auki_auth::SecretString;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use httpmock::prelude::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

struct Peer {
    closing: Notify,
    release: Semaphore,
    refresh: Notify,
    updated: Notify,
    shutdowns: AtomicUsize,
    fail_shutdown: bool,
}
#[async_trait]
impl TaskPeerSession for Peer {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn fence(&self) {}
    async fn update(&self, _: crate::TaskPeerGrant) -> Result<()> {
        self.updated.notify_one();
        Ok(())
    }
    async fn refresh_requested(&self) {
        self.refresh.notified().await;
    }
    async fn wait_stopped(&self) {
        std::future::pending::<()>().await;
    }
    async fn shutdown(&self) -> Result<()> {
        self.shutdowns.fetch_add(1, Ordering::AcqRel);
        self.closing.notify_one();
        self.release.acquire().await.unwrap().forget();
        if self.fail_shutdown {
            return Err(TaskError::PeerCleanup);
        }
        Ok(())
    }
}
struct Factory {
    peer: Arc<Peer>,
    entered: Notify,
    starts: AtomicUsize,
    dds: url::Url,
    block_start: bool,
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
        _: crate::TaskPeerGrant,
        cancel: &CancellationToken,
    ) -> Result<Arc<dyn TaskPeerSession>> {
        self.starts.fetch_add(1, Ordering::AcqRel);
        self.entered.notify_one();
        if self.block_start {
            cancel.cancelled().await;
            self.peer.shutdown().await?;
            return Err(TaskError::Cancelled);
        }
        Ok(self.peer.clone())
    }
}

fn fixture(
    server: &MockServer,
    block_start: bool,
    fail_shutdown: bool,
    assigned: bool,
) -> (Arc<RobotPeer>, Arc<Factory>, AukiRobotCredential) {
    let node = Uuid::new_v4();
    let domain = Uuid::new_v4();
    let now = Utc::now();
    let expires = now + chrono::Duration::seconds(120);
    let claims = json!({"iss": "dds", "aud": [format!("{}/robots", server.base_url())],
        "node_type": "robot", "node_mode": "dedicated", "sub": node, "node_id": node,
        "organization_id": Uuid::new_v4(), "assigned_domain_id": assigned.then_some(domain),
        "iat": now.timestamp(), "exp": expires.timestamp()});
    let token = format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    server.mock(|when, then| {
        when.method(POST).path("/internal/v1/robots/register");
        then.json_body(
            json!({"robot_id": node, "access_token": token, "access_expires_at": expires}),
        );
    });
    server.mock(|when, then| {
        when.method(POST).path("/internal/v1/auth/robot/p2p-token");
        // Only the fake transport below accepts this token; signed SDK transport
        // is tested independently with real local peers in the Python suite.
        then.json_body(
            json!({"p2p_access_token": "fake-transport-only", "p2p_access_expires_at": expires}),
        );
    });
    let config = crate::RobotConfig::new(
        &server.base_url(),
        &server.base_url(),
        SecretString::new("fixture"),
        "1.0.0",
        "fixture",
        &format!("{}/robots", server.base_url()),
        vec!["/example/v1".into()],
    )
    .unwrap();
    let robot = AukiRobotCredential::new(config).unwrap();
    let factory = Arc::new(Factory {
        peer: Arc::new(Peer {
            closing: Notify::new(),
            release: Semaphore::new(0),
            refresh: Notify::new(),
            updated: Notify::new(),
            shutdowns: AtomicUsize::new(0),
            fail_shutdown,
        }),
        entered: Notify::new(),
        starts: AtomicUsize::new(0),
        dds: server.base_url().parse().unwrap(),
        block_start,
    });
    let runtime = Arc::new(RobotPeer::new(
        robot.clone(),
        factory.clone(),
        robot.cancellation().child_token(),
    ));
    (runtime, factory, robot)
}

#[tokio::test]
async fn robot_peer_renews_without_a_lease_and_awaits_repeatable_close() {
    let server = MockServer::start();
    let (runtime, factory, robot) = fixture(&server, false, false, true);
    let cancel = CancellationToken::new();
    runtime.start(&cancel).await.unwrap();
    runtime.start(&cancel).await.unwrap();
    assert_eq!(factory.starts.load(Ordering::Acquire), 1);
    factory.peer.refresh.notify_one();
    tokio::time::timeout(Duration::from_secs(2), factory.peer.updated.notified())
        .await
        .unwrap();
    let clone = runtime.clone();
    let close = tokio::spawn(async move { clone.close().await });
    factory.peer.closing.notified().await;
    assert!(!close.is_finished());
    close.abort();
    let _ = close.await;
    factory.peer.release.add_permits(1);
    runtime.close().await.unwrap();
    runtime.close().await.unwrap();
    assert_eq!(factory.peer.shutdowns.load(Ordering::Acquire), 1);
    assert!(runtime.session().is_none());
    robot.close().await;
}

#[tokio::test]
async fn cancelled_robot_start_waiter_does_not_abandon_partial_peer_cleanup() {
    let server = MockServer::start();
    let (runtime, factory, robot) = fixture(&server, true, false, true);
    let cancel = CancellationToken::new();
    let clone = runtime.clone();
    let caller = cancel.clone();
    let start = tokio::spawn(async move { clone.start(&caller).await });
    factory.entered.notified().await;
    cancel.cancel();
    assert!(matches!(start.await.unwrap(), Err(TaskError::Cancelled)));
    let clone = runtime.clone();
    let close = tokio::spawn(async move { clone.close().await });
    factory.peer.closing.notified().await;
    assert!(!close.is_finished());
    factory.peer.release.add_permits(1);
    close.await.unwrap().unwrap();
    assert_eq!(factory.peer.shutdowns.load(Ordering::Acquire), 1);
    robot.close().await;
}

#[tokio::test]
async fn robot_peer_cleanup_failure_is_retained_for_every_close() {
    let server = MockServer::start();
    let (runtime, factory, robot) = fixture(&server, false, true, true);
    runtime.start(&CancellationToken::new()).await.unwrap();
    factory.peer.release.add_permits(1);
    for _ in 0..2 {
        assert!(matches!(runtime.close().await, Err(TaskError::PeerCleanup)));
    }
    assert_eq!(factory.peer.shutdowns.load(Ordering::Acquire), 1);
    robot.close().await;
}

#[tokio::test]
async fn unassigned_robot_never_starts_a_peer() {
    let server = MockServer::start();
    let (runtime, factory, robot) = fixture(&server, false, false, false);
    runtime.start(&CancellationToken::new()).await.unwrap();
    assert!(runtime.session().is_none());
    assert_eq!(factory.starts.load(Ordering::Acquire), 0);
    runtime.close().await.unwrap();
    robot.close().await;
}
