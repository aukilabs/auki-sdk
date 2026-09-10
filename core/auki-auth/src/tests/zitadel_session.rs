use super::zitadel::{credentials, discovery, token};
use super::*;
use crate::{AuthFailureKind, AuthSession, ZitadelSessionCredentials, ZitadelSessionStore};
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Semaphore;

#[derive(Clone, Debug, PartialEq)]
struct Saved {
    access: String,
    refresh: String,
    client: String,
    issuer: reqwest::Url,
    expiry: Option<chrono::DateTime<Utc>>,
}

impl Saved {
    fn capture(credentials: &ZitadelSessionCredentials) -> Self {
        Self {
            access: credentials.access_token().expose_secret().into(),
            refresh: credentials.refresh_token().expose_secret().into(),
            client: credentials.client_id().into(),
            issuer: credentials.issuer().clone(),
            expiry: credentials.access_token_expires_at(),
        }
    }

    fn import(self) -> ZitadelSessionCredentials {
        ZitadelSessionCredentials::new(
            self.access,
            self.refresh,
            self.client,
            self.issuer,
            self.expiry,
        )
        .unwrap()
    }
}

struct Store {
    attempts: StdMutex<Vec<Saved>>,
    durable: StdMutex<Option<Saved>>,
    entered: Semaphore,
    release: Option<Semaphore>,
    failures: AtomicUsize,
}

impl Store {
    fn new(block: bool, failures: usize) -> Arc<Self> {
        Arc::new(Self {
            attempts: StdMutex::new(Vec::new()),
            durable: StdMutex::new(None),
            entered: Semaphore::new(0),
            release: block.then(|| Semaphore::new(0)),
            failures: AtomicUsize::new(failures),
        })
    }

    async fn wait_for_save(&self) {
        tokio::time::timeout(Duration::from_secs(3), self.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }

    fn allow_save(&self) {
        self.release.as_ref().unwrap().add_permits(1);
    }
}

#[async_trait::async_trait]
impl ZitadelSessionStore for Store {
    async fn save(&self, credentials: &ZitadelSessionCredentials) -> crate::Result<()> {
        let saved = Saved::capture(credentials);
        self.attempts.lock().unwrap().push(saved.clone());
        self.entered.add_permits(1);
        if let Some(release) = &self.release {
            release.acquire().await.unwrap().forget();
        }
        if self
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(Error::InvalidConfiguration("secret-host-storage-error"));
        }
        *self.durable.lock().unwrap() = Some(saved);
        Ok(())
    }
}

fn client(server: &MockServer, limits: AuthLimits) -> AuthClient {
    AuthClient::with_limits(
        AuthEnvironment::new(&server.base_url, &server.base_url).unwrap(),
        limits,
    )
    .unwrap()
}

fn import(server: &MockServer, store: Arc<Store>, expired: bool) -> AuthSession {
    let mut creds = credentials(&server.base_url);
    if expired {
        creds.access_token_expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
    }
    client(server, AuthLimits::default())
        .import_zitadel_session(creds, store)
        .unwrap()
}

async fn wait_requests(server: &MockServer, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while server.requests.lock().await.len() < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

const DOMAIN: Uuid = Uuid::from_u128(0xdddd);

fn test_identity() -> Identity {
    Identity::from_ed25519_seed(&[42; 32])
}

async fn authorize(session: &AuthSession) -> crate::Result<PreparedPeer> {
    session
        .authorize_peer(DOMAIN.into(), &test_identity().proof())
        .await
}

fn admitted() -> Vec<MockResponse> {
    vec![
        challenge_response("proof", [1; 32]),
        signed_peer_response(
            &test_identity(),
            DOMAIN,
            "user",
            Utc::now().timestamp() as u64,
        ),
        keys_response(),
    ]
}

fn assert_counts(requests: &[RecordedRequest], refresh: usize, challenges: usize) {
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.target == "/oauth/v2/token")
            .count(),
        refresh
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.target.ends_with("/p2p/zitadel/challenge"))
            .count(),
        challenges
    );
    assert!(
        requests
            .iter()
            .all(|r| !r.target.starts_with("/service/domains-access-token")
                && r.target != "/user/refresh"
                && !r.target.starts_with("/api/v1/accessible-domains"))
    );
}

#[tokio::test]
async fn zitadel_discovery_is_explicitly_unsupported_without_io_or_refresh() {
    let server = MockServer::start(vec![]).await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), true);
    assert!(matches!(
        session.accessible_domains().await,
        Err(Error::InvalidConfiguration(_))
    ));
    assert!(server.requests.lock().await.is_empty());
    assert!(store.attempts.lock().unwrap().is_empty());
    session.close().await;
    assert!(matches!(
        session.accessible_domains().await,
        Err(Error::SessionClosed)
    ));
    server.finish().await;
}

#[tokio::test]
async fn import_is_local_and_failed_startup_retains_recoverable_session() {
    let server = MockServer::start_with(|base| {
        let mut responses = vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            MockResponse::status(503),
        ];
        responses.extend(admitted());
        responses
    })
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), true);
    assert!(server.requests.lock().await.is_empty());
    assert!(store.attempts.lock().unwrap().is_empty());
    assert!(matches!(
        authorize(&session).await,
        Err(Error::HttpStatus { status: 503, .. })
    ));
    assert_eq!(authorize(&session).await.unwrap().domain.id, DOMAIN);
    let requests = server.finish().await;
    assert_counts(&requests, 1, 2);
    for i in [2, 3, 4] {
        assert_eq!(
            requests[i].headers["authorization"],
            "Bearer replacement-opaque"
        );
    }
    let saved = store.durable.lock().unwrap().clone().unwrap();
    assert_eq!(saved.refresh, "replacement-refresh");
    assert!(saved.expiry.is_some());
    session.close().await;
    assert!(matches!(
        authorize(&session).await,
        Err(Error::SessionClosed)
    ));
}

#[tokio::test]
async fn unknown_expiry_recovers_once_and_proactive_refresh_consumes_same_budget() {
    for proactive in [false, true] {
        let server = MockServer::start_with(|base| {
            let mut responses = Vec::new();
            if !proactive {
                responses.push(MockResponse::status(401));
            }
            responses.extend([
                MockResponse::json(discovery(base)),
                MockResponse::json(token()),
                MockResponse::status(401),
            ]);
            responses
        })
        .await;
        let session = import(&server, Store::new(false, 0), proactive);
        assert_eq!(
            authorize(&session).await.unwrap_err().kind(),
            AuthFailureKind::AuthenticationRequired
        );
        assert!(matches!(
            authorize(&session).await,
            Err(Error::AuthenticationRequired)
        ));
        assert_counts(&server.finish().await, 1, if proactive { 1 } else { 2 });
        session.close().await;
    }
}

#[tokio::test]
async fn concurrent_clones_share_one_refresh_and_cancellation_during_http_keeps_rotation() {
    let server = MockServer::start_with(|base| {
        let mut responses = vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()).delayed(Duration::from_millis(100)),
        ];
        responses.extend(admitted());
        responses.extend(admitted());
        responses
    })
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), true);
    let first = tokio::spawn({
        let session = session.clone();
        async move { authorize(&session).await }
    });
    wait_requests(&server, 2).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let (a, b) = tokio::join!(authorize(&session), authorize(&session));
    assert_eq!(a.unwrap().domain.id, DOMAIN);
    assert_eq!(b.unwrap().domain.id, DOMAIN);
    assert_eq!(store.attempts.lock().unwrap().len(), 1);
    assert_counts(&server.finish().await, 1, 2);
    session.close().await;
}

#[tokio::test]
async fn persistence_blocks_downstream_and_cancelled_waiter_does_not_repeat_save() {
    let server = MockServer::start_with(|base| {
        let mut responses = vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
        ];
        responses.extend(admitted());
        responses
    })
    .await;
    let store = Store::new(true, 0);
    let session = import(&server, store.clone(), true);
    let cancellation = CancellationToken::new();
    let first = tokio::spawn({
        let session = session.clone();
        let cancellation = cancellation.clone();
        async move {
            session
                .authorize_peer_with_cancellation(
                    DOMAIN.into(),
                    &test_identity().proof(),
                    &cancellation,
                )
                .await
        }
    });
    store.wait_for_save().await;
    cancellation.cancel();
    assert!(matches!(first.await.unwrap(), Err(Error::Cancelled { .. })));
    assert_eq!(server.requests.lock().await.len(), 2);
    let next = tokio::spawn({
        let session = session.clone();
        async move { authorize(&session).await }
    });
    tokio::task::yield_now().await;
    assert!(!next.is_finished());
    assert_eq!(store.attempts.lock().unwrap().len(), 1);
    store.allow_save();
    assert_eq!(next.await.unwrap().unwrap().domain.id, DOMAIN);
    assert_counts(&server.finish().await, 1, 1);
    session.close().await;
}

#[tokio::test]
async fn rejected_persistence_retries_same_generation_and_restart_imports_latest() {
    let server = MockServer::start_with(|base| {
        let mut responses = vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
        ];
        responses.extend(admitted());
        responses.extend(admitted());
        responses
    })
    .await;
    let store = Store::new(false, 1);
    let session = import(&server, store.clone(), true);
    let error = authorize(&session).await.unwrap_err();
    assert!(matches!(error, Error::Persistence));
    assert!(!format!("{error:?} {error}").contains("secret-host"));
    assert_eq!(server.requests.lock().await.len(), 2);
    authorize(&session).await.unwrap();
    let saved = store.durable.lock().unwrap().clone().unwrap();
    assert_eq!(
        store.attempts.lock().unwrap().clone(),
        vec![saved.clone(), saved.clone()]
    );
    session.close().await;
    let restored = client(&server, AuthLimits::default())
        .import_zitadel_session(saved.import(), store)
        .unwrap();
    authorize(&restored).await.unwrap();
    let requests = server.finish().await;
    assert_counts(&requests, 1, 2);
    assert_eq!(
        requests[5].headers["authorization"],
        "Bearer replacement-opaque"
    );
    restored.close().await;
}

#[tokio::test]
async fn bounded_wait_does_not_cancel_pending_host_write_or_allow_second_rotation() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
        ]
    })
    .await;
    let store = Store::new(true, 0);
    let limits = AuthLimits {
        connect_timeout: Duration::from_millis(30),
        request_timeout: Duration::from_millis(60),
        ..AuthLimits::default()
    };
    let mut creds = credentials(&server.base_url);
    creds.access_token_expires_at = Some(Utc::now());
    let session = client(&server, limits)
        .import_zitadel_session(creds, store.clone())
        .unwrap();
    assert!(matches!(
        authorize(&session).await,
        Err(Error::SessionOperationPending)
    ));
    assert!(matches!(
        authorize(&session).await,
        Err(Error::SessionOperationPending)
    ));
    assert_eq!(store.attempts.lock().unwrap().len(), 1);
    let close = tokio::spawn({
        let session = session.clone();
        async move { session.close().await }
    });
    tokio::task::yield_now().await;
    assert!(!close.is_finished());
    store.allow_save();
    close.await.unwrap();
    assert_counts(&server.finish().await, 1, 0);
}

#[tokio::test]
async fn logout_drains_running_save_even_when_first_close_waiter_is_cancelled() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
        ]
    })
    .await;
    let store = Store::new(true, 0);
    let session = import(&server, store.clone(), true);
    let operation = tokio::spawn({
        let session = session.clone();
        async move { authorize(&session).await }
    });
    store.wait_for_save().await;
    let close = tokio::spawn({
        let session = session.clone();
        async move { session.close().await }
    });
    assert!(matches!(
        operation.await.unwrap(),
        Err(Error::SessionClosed)
    ));
    close.abort();
    let _ = close.await;
    assert!(matches!(
        authorize(&session).await,
        Err(Error::SessionClosed)
    ));
    let close = tokio::spawn({
        let session = session.clone();
        async move { session.close().await }
    });
    tokio::task::yield_now().await;
    assert!(!close.is_finished());
    store.allow_save();
    close.await.unwrap();
    *store.durable.lock().unwrap() = None; // Host clear is safe only after close.
    session.close().await;
    assert!(store.durable.lock().unwrap().is_none());
    assert_counts(&server.finish().await, 1, 0);
}

#[tokio::test]
async fn invalid_grant_and_ambiguous_rotation_are_terminal_without_replay() {
    for ambiguous in [false, true] {
        let server = MockServer::start_with(|base| {
            vec![
                MockResponse::json(discovery(base)),
                if ambiguous {
                    MockResponse::json(json!({"access_token":"missing-rotation"}))
                } else {
                    MockResponse {
                        status: 400,
                        ..MockResponse::json(json!({"error":"invalid_grant"}))
                    }
                },
            ]
        })
        .await;
        let session = import(&server, Store::new(false, 0), true);
        assert_eq!(
            authorize(&session).await.unwrap_err().kind(),
            AuthFailureKind::AuthenticationRequired
        );
        assert!(matches!(
            authorize(&session).await,
            Err(Error::AuthenticationRequired)
        ));
        assert_counts(&server.finish().await, 1, 0);
        session.close().await;
    }
}

#[tokio::test]
async fn dds_401_refreshes_and_restarts_entire_bearer_bound_proof_once() {
    let domain = Uuid::new_v4();
    let identity = Identity::generate();
    let server = MockServer::start_with(|base| {
        vec![
            challenge_response("first", [1; 32]),
            MockResponse::status(401),
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            challenge_response("second", [2; 32]),
            signed_peer_response(&identity, domain, "user", Utc::now().timestamp() as u64),
            keys_response(),
        ]
    })
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), false);
    let prepared = session
        .authorize_peer(domain.into(), &identity.proof())
        .await
        .unwrap();
    assert_eq!(prepared.peer_id, identity.peer_id());
    let requests = server.finish().await;
    assert_counts(&requests, 1, 2);
    assert_eq!(
        requests
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        [
            &format!("/api/v1/domains/{domain}/p2p/zitadel/challenge"),
            &format!("/api/v1/domains/{domain}/p2p/zitadel/verify"),
            "/.well-known/openid-configuration",
            "/oauth/v2/token",
            &format!("/api/v1/domains/{domain}/p2p/zitadel/challenge"),
            &format!("/api/v1/domains/{domain}/p2p/zitadel/verify"),
            "/service/p2p-verification-keys",
        ]
    );
    for i in [0, 1] {
        assert_eq!(
            requests[i].headers["authorization"],
            "Bearer opaque.access+/="
        );
    }
    for i in [4, 5] {
        assert_eq!(
            requests[i].headers["authorization"],
            "Bearer replacement-opaque"
        );
    }
    assert_eq!(requests[0].body, requests[4].body);
    let first: Value = serde_json::from_slice(&requests[1].body).unwrap();
    let second: Value = serde_json::from_slice(&requests[5].body).unwrap();
    assert_eq!(first["challenge_id"], "first");
    assert_eq!(second["challenge_id"], "second");
    assert_ne!(first["signature"], second["signature"]);
    assert_eq!(store.attempts.lock().unwrap().len(), 1);
    session.close().await;
}

#[tokio::test]
async fn denied_domain_does_not_poison_other_peers_sharing_session() {
    for (status, deny_during_verify) in [
        (403, false),
        (403, true),
        (404, false),
        (404, true),
        (503, false),
        (503, true),
    ] {
        let denied = Uuid::new_v4();
        let allowed = Uuid::new_v4();
        let identity = Identity::generate();
        let mut responses = vec![];
        if deny_during_verify {
            responses.push(challenge_response("denied", [4; 32]));
        }
        responses.push(MockResponse::status(status));
        responses.extend([
            challenge_response("allowed", [3; 32]),
            signed_peer_response(&identity, allowed, "user", Utc::now().timestamp() as u64),
            keys_response(),
        ]);
        let server = MockServer::start(responses).await;
        let session = import(&server, Store::new(false, 0), false);
        let error = session
            .authorize_peer(denied.into(), &Identity::generate().proof())
            .await
            .unwrap_err();
        assert_eq!(
            error.kind(),
            if status == 503 {
                AuthFailureKind::Transient
            } else {
                AuthFailureKind::AuthorizationDenied
            }
        );
        let prepared = session
            .authorize_peer(allowed.into(), &identity.proof())
            .await
            .unwrap();
        assert_eq!(prepared.domain.id, allowed);
        assert_counts(&server.finish().await, 0, 2);
        session.close().await;
    }
}

#[tokio::test]
async fn proactive_refresh_budget_is_not_reset_by_dds_recovery() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            MockResponse::status(401),
        ]
    })
    .await;
    let session = import(&server, Store::new(false, 0), true);
    assert!(matches!(
        authorize(&session).await,
        Err(Error::AuthenticationRequired)
    ));
    assert!(matches!(
        authorize(&session).await,
        Err(Error::AuthenticationRequired)
    ));
    assert_counts(&server.finish().await, 1, 1);
    session.close().await;
}

#[tokio::test]
async fn pending_save_is_retried_before_reusing_a_previous_bearer_snapshot() {
    let server = MockServer::start_with(|base| {
        let mut responses = admitted();
        responses.extend([
            MockResponse::status(401),
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
        ]);
        responses.extend(admitted());
        responses
    })
    .await;
    let store = Store::new(true, 1);
    let session = import(&server, store.clone(), false);
    authorize(&session).await.unwrap();
    let failing = tokio::spawn({
        let session = session.clone();
        async move { authorize(&session).await }
    });
    store.wait_for_save().await;
    store.allow_save();
    assert!(matches!(failing.await.unwrap(), Err(Error::Persistence)));
    let retry = tokio::spawn({
        let session = session.clone();
        async move { authorize(&session).await }
    });
    store.wait_for_save().await;
    assert_eq!(
        server.requests.lock().await.len(),
        6,
        "old snapshot cannot bypass pending persistence"
    );
    store.allow_save();
    retry.await.unwrap().unwrap();
    let attempts = store.attempts.lock().unwrap().clone();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], attempts[1]);
    let requests = server.finish().await;
    assert_counts(&requests, 1, 3);
    assert_eq!(
        requests[6].headers["authorization"],
        "Bearer replacement-opaque"
    );
    session.close().await;
}

#[tokio::test]
async fn lost_refresh_response_is_not_replayed_after_wait_timeout() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()).delayed(Duration::from_millis(160)),
        ]
    })
    .await;
    let store = Store::new(false, 0);
    let limits = AuthLimits {
        connect_timeout: Duration::from_millis(30),
        request_timeout: Duration::from_millis(60),
        ..AuthLimits::default()
    };
    let mut creds = credentials(&server.base_url);
    creds.access_token_expires_at = Some(Utc::now());
    let session = client(&server, limits)
        .import_zitadel_session(creds, store.clone())
        .unwrap();
    let first = authorize(&session).await.unwrap_err();
    assert!(matches!(
        first,
        Error::SessionOperationPending | Error::RefreshOutcomeUnknown
    ));
    let requests = server.finish().await;
    assert_eq!(
        authorize(&session).await.unwrap_err().kind(),
        AuthFailureKind::AuthenticationRequired
    );
    assert!(matches!(
        authorize(&session).await,
        Err(Error::AuthenticationRequired)
    ));
    assert_counts(&requests, 1, 0);
    assert!(store.attempts.lock().unwrap().is_empty());
    session.close().await;
}

#[tokio::test]
async fn close_during_refresh_drains_response_without_starting_a_save() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()).delayed(Duration::from_millis(100)),
        ]
    })
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), true);
    let operation = tokio::spawn({
        let session = session.clone();
        async move { authorize(&session).await }
    });
    wait_requests(&server, 2).await;
    session.close().await;
    assert!(matches!(
        operation.await.unwrap(),
        Err(Error::SessionClosed)
    ));
    assert!(store.attempts.lock().unwrap().is_empty());
    assert_counts(&server.finish().await, 1, 0);
}

#[tokio::test]
async fn transient_discovery_recovers_but_invalid_client_is_latched() {
    for configuration in [false, true] {
        let server = MockServer::start_with(|base| {
            if configuration {
                vec![
                    MockResponse::json(discovery(base)),
                    MockResponse {
                        status: 400,
                        ..MockResponse::json(json!({"error":"invalid_client"}))
                    },
                ]
            } else {
                let mut responses = vec![
                    MockResponse::status(503),
                    MockResponse::json(discovery(base)),
                    MockResponse::json(token()),
                ];
                responses.extend(admitted());
                responses
            }
        })
        .await;
        let session = import(&server, Store::new(false, 0), true);
        let error = authorize(&session).await.unwrap_err();
        if configuration {
            assert_eq!(error.kind(), AuthFailureKind::Configuration);
            assert_eq!(
                authorize(&session).await.unwrap_err().kind(),
                AuthFailureKind::Configuration
            );
        } else {
            assert_eq!(error.kind(), AuthFailureKind::Transient);
            authorize(&session).await.unwrap();
        }
        assert_counts(&server.finish().await, 1, if configuration { 0 } else { 1 });
        session.close().await;
    }
}

#[tokio::test]
async fn two_domains_and_peers_share_rotations_without_changing_peer_ids() {
    let domain_a = Uuid::new_v4();
    let domain_b = Uuid::new_v4();
    let a = Identity::generate();
    let b = Identity::generate();
    let now = Utc::now().timestamp() as u64;
    let server = MockServer::start_with(|base| {
        let mut next = token();
        next["refresh_token"] = json!("second-generation-refresh");
        next["access_token"] = json!("second-generation-access");
        vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            challenge_response("initial-a", [1; 32]),
            signed_peer_response(&a, domain_a, "user", now - 2),
            keys_response(),
            challenge_response("initial-b", [2; 32]),
            signed_peer_response(&b, domain_b, "user", now - 2),
            keys_response(),
            MockResponse::status(401),
            MockResponse::json(discovery(base)),
            MockResponse::json(next),
            challenge_response("renew-a", [3; 32]),
            signed_peer_response(&a, domain_a, "user", now),
            keys_response(),
            challenge_response("renew-b", [4; 32]),
            signed_peer_response(&b, domain_b, "user", now),
            keys_response(),
        ]
    })
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), true);
    let a_proof = a.proof();
    let b_proof = b.proof();
    let (prepared_a, prepared_b) = tokio::join!(
        session.authorize_peer(domain_a.into(), &a_proof),
        session.authorize_peer(domain_b.into(), &b_proof)
    );
    let prepared_a = prepared_a.unwrap();
    let prepared_b = prepared_b.unwrap();
    let (renewed_a, renewed_b) =
        tokio::join!(prepared_a.renewal.renew(), prepared_b.renewal.renew());
    assert_eq!(renewed_a.unwrap().peer_id, a.peer_id());
    assert_eq!(renewed_b.unwrap().peer_id, b.peer_id());
    let requests = server.finish().await;
    assert_counts(&requests, 2, 5);
    let rotations: Vec<_> = requests
        .iter()
        .filter(|r| r.target == "/oauth/v2/token")
        .collect();
    assert!(
        std::str::from_utf8(&rotations[1].body)
            .unwrap()
            .contains("refresh_token=replacement-refresh")
    );
    assert_eq!(store.attempts.lock().unwrap().len(), 2);
    assert_eq!(
        store.durable.lock().unwrap().as_ref().unwrap().refresh,
        "second-generation-refresh"
    );
    session.close().await;
}
