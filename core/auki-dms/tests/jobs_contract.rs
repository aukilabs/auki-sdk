#![cfg(all(feature = "jobs", not(target_arch = "wasm32")))]

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use auki_auth::{
    AuthClient, AuthEnvironment, AuthSession, Credentials, DomainAccessProvider,
    Error as AuthError, ZitadelSessionCredentials, ZitadelSessionStore,
};
use auki_dms::jobs::{
    AukiDmsJobs, JobEdge, JobListQuery, JobMode, JobSpec, JobStatus, JobTaskSpec, JobsError,
    JobsLimits,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use httpmock::{
    Method::{GET, POST},
    Mock, MockServer,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum Principal {
    User,
    App,
    ImportedZitadel,
}

struct NoopStore;

#[async_trait]
impl ZitadelSessionStore for NoopStore {
    async fn save(&self, _: &ZitadelSessionCredentials) -> Result<(), AuthError> {
        panic!("an unexpired imported-session fixture must not refresh")
    }
}

struct FailingOnceStore {
    failures: AtomicUsize,
    attempts: Mutex<Vec<(String, String)>>,
}

impl FailingOnceStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            failures: AtomicUsize::new(1),
            attempts: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl ZitadelSessionStore for FailingOnceStore {
    async fn save(&self, credentials: &ZitadelSessionCredentials) -> Result<(), AuthError> {
        self.attempts.lock().unwrap().push((
            credentials.access_token().expose_secret().into(),
            credentials.refresh_token().expose_secret().into(),
        ));
        if self
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok()
        {
            Err(AuthError::Persistence)
        } else {
            Ok(())
        }
    }
}

struct Fixture {
    server: MockServer,
    session: AuthSession,
    domain: Uuid,
    job: Uuid,
}

impl Fixture {
    async fn new(principal: Principal) -> Self {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/user/login");
                then.header("content-type", "application/json")
                    .json_body(json!({"access_token":"user-api", "refresh_token":"user-refresh"}));
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/service/domains-access-token")
                    .header(
                        "authorization",
                        match principal {
                            Principal::User => "Bearer user-api",
                            Principal::App => "Basic a2V5OnNlY3JldA==",
                            Principal::ImportedZitadel => "Bearer opaque.zitadel+/=",
                        },
                    );
                then.header("content-type", "application/json")
                    .json_body(json!({"access_token":"dds-service"}));
            })
            .await;
        let environment = AuthEnvironment::new(server.base_url(), server.base_url())
            .unwrap()
            .with_client_id("jobs-contract")
            .unwrap();
        let auth = AuthClient::new(environment).unwrap();
        let session = match principal {
            Principal::User => auth
                .authenticate(Credentials::user_password("test@example.com", "password"))
                .await
                .unwrap(),
            Principal::App => auth
                .authenticate(Credentials::app("key", "secret"))
                .await
                .unwrap(),
            Principal::ImportedZitadel => auth
                .import_zitadel_session(
                    ZitadelSessionCredentials::new(
                        "opaque.zitadel+/=",
                        "refresh",
                        "public-client",
                        server.base_url().parse().unwrap(),
                        None,
                    )
                    .unwrap(),
                    Arc::new(NoopStore),
                )
                .unwrap(),
        };
        Self {
            server,
            session,
            domain: Uuid::new_v4(),
            job: Uuid::new_v4(),
        }
    }

    fn dms_base(&self) -> String {
        format!("{}/tenant/v1", self.server.base_url())
    }

    fn token(&self, domain: Uuid, audiences: &[&str], seconds: i64, marker: &str) -> String {
        format!(
            "e30.{}.{}",
            URL_SAFE_NO_PAD.encode(
                json!({
                    "iss": "dds",
                    "domain_id": domain,
                    "aud": audiences,
                    "exp": chrono::Utc::now().timestamp() + seconds,
                })
                .to_string()
            ),
            marker
        )
    }

    fn grant(&self, domain: Uuid, token: &str) -> Value {
        json!({
            "id": domain,
            "domain_server": {"url": self.server.base_url()},
            "access_token": token,
        })
    }

    async fn auth(&self, value: Value) -> Mock<'_> {
        self.server
            .mock_async(|when, then| {
                when.method(POST)
                    .path(format!("/api/v1/domains/{}/auth", self.domain))
                    .header("authorization", "Bearer dds-service")
                    .header("posemesh-client-id", "jobs-contract")
                    .header_exists("posemesh-sdk-version");
                then.header("content-type", "application/json")
                    .json_body(value);
            })
            .await
    }

    async fn valid_auth(&self, marker: &str) -> (String, Mock<'_>) {
        let token = self.token(self.domain, &[&self.server.base_url(), "dds"], 3600, marker);
        let auth = self.auth(self.grant(self.domain, &token)).await;
        (token, auth)
    }

    fn client(&self) -> auki_dms::jobs::DomainJobsClient {
        AukiDmsJobs::new(self.session.clone(), &self.dms_base())
            .unwrap()
            .in_domain(self.domain)
    }

    fn path(&self, suffix: &str) -> String {
        format!("/tenant/v1/jobs{suffix}")
    }

    fn job_json(&self, id: Uuid, domain: Uuid, status: &str) -> Value {
        json!({
            "id": id,
            "label": "scan pipeline",
            "domain_id": domain,
            "status": status,
            "priority": 7,
            "created_at": "2026-09-15T01:02:03Z",
            "updated_at": "2026-09-15T01:03:04Z",
            "organization_id": Uuid::new_v4(),
            "meta": {"trace": "opaque"},
            "credit_lock_id": null,
            "credit_lock_amount": "12345678901234567890.123456789",
            "credit_locked_at": null,
            "credit_released_at": null,
        })
    }

    fn summary_json() -> Value {
        json!({"queued":0,"leased":0,"running":1,"completed":2,"failed":0,"canceled":0})
    }
}

fn graph_spec() -> JobSpec {
    let mut filters = BTreeMap::new();
    filters.insert("hardware".into(), "arm64".into());
    JobSpec {
        label: "scan pipeline".into(),
        priority: 7,
        meta: json!({"request": {"keep": [1, true, null]}}),
        tasks: vec![
            JobTaskSpec {
                label: "capture".into(),
                stage: "local".into(),
                capability: "vendor.example/capture@v2".into(),
                mode: JobMode::Dedicated,
                capability_filters: filters,
                priority: -2,
                inputs_cids: vec!["opaque://input/one".into()],
                outputs_prefix: Some("opaque://outputs/".into()),
                meta: json!({"custom": "preserved"}),
                max_attempts: 5,
            },
            JobTaskSpec::new("global", "vendor.example/reconstruct@v9"),
        ],
        edges: vec![JobEdge {
            from: "local".into(),
            to: "global".into(),
        }],
    }
}

#[tokio::test]
async fn user_app_and_imported_sessions_submit_the_exact_domain_graph() {
    for principal in [Principal::User, Principal::App, Principal::ImportedZitadel] {
        let f = Fixture::new(principal).await;
        let (token, auth) = f.valid_auth("write-grant").await;
        let submit = f
            .server
            .mock_async(|when, then| {
                when.method(POST)
                    .path(f.path(""))
                    .header("authorization", format!("Bearer {token}"))
                    .header("posemesh-client-id", "jobs-contract")
                    .header_missing("Idempotency-Key")
                    .json_body(json!({
                        "label":"scan pipeline", "domain_id":f.domain, "priority":7,
                        "meta":{"request":{"keep":[1,true,null]}},
                        "tasks":[
                            {"label":"capture","stage":"local","capability":"vendor.example/capture@v2",
                             "mode":"dedicated","capability_filters":{"hardware":"arm64"},"priority":-2,
                             "inputs_cids":["opaque://input/one"],"outputs_prefix":"opaque://outputs/",
                             "meta":{"custom":"preserved"},"max_attempts":5},
                            {"label":"global","stage":"global","capability":"vendor.example/reconstruct@v9",
                             "mode":"public","capability_filters":{},"priority":0,"inputs_cids":[],
                             "outputs_prefix":null,"meta":{},"max_attempts":3}
                        ],
                        "edges":[{"from":"local","to":"global"}]
                    }));
                then.header("content-type", "application/json")
                    .json_body(json!({"job_id": f.job}));
            })
            .await;

        assert_eq!(f.client().submit(&graph_spec()).await.unwrap(), f.job);
        auth.assert_calls_async(1).await;
        submit.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn estimate_keeps_decimal_strings_and_dedicated_mode() {
    let f = Fixture::new(Principal::User).await;
    let (token, _) = f.valid_auth("estimate-grant").await;
    let estimate = f
        .server
        .mock_async(|when, then| {
            when.method(POST)
                .path(f.path("/estimate"))
                .header("authorization", format!("Bearer {token}"))
                .body_includes(format!(r#""domain_id":"{}""#, f.domain))
                .body_includes(r#""mode":"dedicated""#);
            then.header("content-type", "application/json").json_body(json!({
                "total":"12345678901234567890.123456789",
                "tasks":[
                    {"label":"capture","stage":"local","capability":"vendor.example/capture@v2",
                     "mode":"dedicated","billing_units":"3.50","estimated_credit_cost":"12345678901234567890.123456789"},
                    {"label":"global","stage":"global","capability":"vendor.example/reconstruct@v9",
                     "mode":"public","billing_units":"1","estimated_credit_cost":"0.25"}
                ]
            }));
        })
        .await;
    let result = f.client().estimate(&graph_spec()).await.unwrap();
    assert_eq!(result.total, "12345678901234567890.123456789");
    assert_eq!(result.tasks[0].billing_units, "3.50");
    assert_eq!(result.tasks[0].mode, JobMode::Dedicated);
    estimate.assert_calls_async(1).await;
}

#[tokio::test]
async fn estimate_rejects_a_task_label_that_does_not_match_the_request() {
    let f = Fixture::new(Principal::User).await;
    f.valid_auth("mismatched-estimate-grant").await;
    let estimate = f
        .server
        .mock_async(|when, then| {
            when.method(POST).path(f.path("/estimate"));
            then.header("content-type", "application/json").json_body(json!({
                "total":"1.25",
                "tasks":[
                    {"label":"wrong-label","stage":"local","capability":"vendor.example/capture@v2",
                     "mode":"dedicated","billing_units":"1","estimated_credit_cost":"1"},
                    {"label":"global","stage":"global","capability":"vendor.example/reconstruct@v9",
                     "mode":"public","billing_units":"1","estimated_credit_cost":"0.25"}
                ]
            }));
        })
        .await;
    assert!(matches!(
        f.client().estimate(&graph_spec()).await,
        Err(JobsError::InvalidResponse(_))
    ));
    estimate.assert_calls_async(1).await;
}

#[tokio::test]
async fn list_preserves_opaque_cursor_and_get_preserves_progress_results_and_receipts() {
    let f = Fixture::new(Principal::App).await;
    let (token, _) = f.valid_auth("read-grant").await;
    let cursor = "opaque+/cursor==&do-not-parse";
    let listed_job = f.job_json(f.job, f.domain, "running");
    let list = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path(f.path(""))
                .query_param("limit", "17")
                .query_param("cursor", cursor)
                .query_param("status", "running")
                .query_param("capabilities", "cap/a")
                .query_param("capabilities", "cap b")
                .query_param("match_all_capabilities", "true")
                .header("authorization", format!("Bearer {token}"));
            then.header("content-type", "application/json")
                .json_body(json!({
                    "items":[{"job":listed_job,"tasks_summary":Fixture::summary_json()}],
                    "next_cursor":"next+/opaque==&still-opaque"
                }));
        })
        .await;
    let page = f
        .client()
        .list(&JobListQuery {
            limit: 17,
            cursor: Some(cursor.into()),
            status: Some(JobStatus::Running),
            capabilities: vec!["cap/a".into(), "cap b".into()],
            match_all_capabilities: true,
        })
        .await
        .unwrap();
    assert_eq!(page.items[0].job.id, f.job);
    assert_eq!(page.items[0].tasks_summary.completed, 2);
    assert_eq!(
        page.next_cursor.as_deref(),
        Some("next+/opaque==&still-opaque")
    );
    assert_eq!(
        page.items[0].job.credit_lock_amount.as_deref(),
        Some("12345678901234567890.123456789")
    );
    list.assert_calls_async(1).await;

    let task = Uuid::new_v4();
    let receipt = Uuid::new_v4();
    let node = Uuid::new_v4();
    let get = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(f.path(&format!("/{}", f.job)));
            then.header("content-type", "application/json").json_body(json!({
                "job":f.job_json(f.job,f.domain,"running"),
                "tasks_summary":Fixture::summary_json(),
                "tasks":[{
                    "id":task,"job_id":f.job,"label":"capture","stage":"local",
                    "capability":"vendor.example/capture@v2","capability_filters":{"hardware":"arm64"},
                    "status":"running","deps_remaining":0,"priority":-2,"inputs_cids":["opaque://input/one"],
                    "outputs_prefix":"opaque://outputs/","organization_id":null,"attempts":1,"max_attempts":5,
                    "lease_expires_at":"2026-09-15T02:00:00Z","reserved_by":node,
                    "meta":{"progress":{"ratio":0.625},"events":[{"kind":"worker","payload":{"x":1}}]},
                    "cancel_requested_at":null,"last_heartbeat_at":"2026-09-15T01:03:00Z",
                    "created_at":"2026-09-15T01:02:03Z","updated_at":"2026-09-15T01:03:04Z",
                    "mode":"dedicated","billing_units":"3.50","estimated_credit_cost":"9.75",
                    "debited_amount":null,"debited_at":null
                }],
                "receipts":[{"id":receipt,"job_id":f.job,"task_id":task,"node_id":node,
                    "outputs":["opaque://result/a"],"meta":{"result":{"quality":"high"}},
                    "created_at":"2026-09-15T01:04:00Z"}]
            }));
        })
        .await;
    let details = f.client().get(f.job).await.unwrap();
    assert_eq!(details.tasks[0].meta["progress"]["ratio"], 0.625);
    assert_eq!(details.tasks[0].meta["events"][0]["kind"], "worker");
    assert_eq!(details.receipts[0].outputs, ["opaque://result/a"]);
    assert_eq!(details.receipts[0].meta["result"]["quality"], "high");
    get.assert_calls_async(1).await;
}

#[tokio::test]
async fn cancel_uses_the_job_route_and_validates_the_acknowledgement() {
    let f = Fixture::new(Principal::User).await;
    let (token, _) = f.valid_auth("cancel-grant").await;
    let cancel = f
        .server
        .mock_async(|when, then| {
            when.method(POST)
                .path(f.path(&format!("/{}/cancel", f.job)))
                .header("authorization", format!("Bearer {token}"));
            then.header("content-type", "application/json")
                .json_body(json!({
                    "id":f.job,"status":"canceled","updated_at":"2026-09-15T01:05:00Z"
                }));
        })
        .await;
    let result = f.client().cancel(f.job).await.unwrap();
    assert_eq!(result.id, f.job);
    assert_eq!(result.status, JobStatus::Canceled);
    cancel.assert_calls_async(1).await;
}

#[tokio::test]
async fn request_and_response_limits_fail_with_typed_errors() {
    let f = Fixture::new(Principal::User).await;
    let (_, auth) = f.valid_auth("limits-grant").await;
    let client = AukiDmsJobs::with_limits(
        f.session.clone(),
        &f.dms_base(),
        JobsLimits {
            max_request_bytes: 128,
            max_response_bytes: 64,
            ..JobsLimits::default()
        },
    )
    .unwrap()
    .in_domain(f.domain);
    let mut spec = graph_spec();
    spec.meta = json!({"large": "x".repeat(256)});
    assert!(matches!(
        client.submit(&spec).await,
        Err(JobsError::TooLarge { maximum: 128 })
    ));
    auth.assert_calls_async(0).await;

    f.valid_auth("response-limit-grant").await;
    let response = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(f.path(""));
            then.header("content-type", "application/json")
                .body("x".repeat(65));
        })
        .await;
    assert!(matches!(
        client.list(&JobListQuery::default()).await,
        Err(JobsError::TooLarge { maximum: 64 })
    ));
    response.assert_calls_async(1).await;
}

#[tokio::test]
async fn malformed_or_cross_domain_responses_are_rejected() {
    let f = Fixture::new(Principal::User).await;
    f.valid_auth("invalid-response-grant").await;
    let foreign = Uuid::new_v4();
    let wrong = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(f.path(&format!("/{}", f.job)));
            then.header("content-type", "application/json")
                .json_body(json!({
                    "job":f.job_json(foreign,Uuid::new_v4(),"running"),
                    "tasks_summary":Fixture::summary_json(),"tasks":[],"receipts":[]
                }));
        })
        .await;
    assert!(matches!(
        f.client().get(f.job).await,
        Err(JobsError::InvalidResponse(_))
    ));
    wrong.assert_calls_async(1).await;
}

#[tokio::test]
async fn domain_server_only_wrong_domain_wrong_id_and_expired_grants_never_reach_dms() {
    for kind in 0..5 {
        let f = Fixture::new(Principal::User).await;
        let dms = f
            .server
            .mock_async(|when, then| {
                when.method(GET).path(f.path(""));
                then.status(200);
            })
            .await;
        let value = match kind {
            0 => f.grant(
                f.domain,
                &f.token(f.domain, &[&f.server.base_url()], 3600, "no-dds-aud"),
            ),
            1 => f.grant(
                f.domain,
                &f.token(
                    Uuid::new_v4(),
                    &[&f.server.base_url(), "dds"],
                    3600,
                    "wrong-domain",
                ),
            ),
            2 => f.grant(
                Uuid::new_v4(),
                &f.token(f.domain, &[&f.server.base_url(), "dds"], 3600, "wrong-id"),
            ),
            3 => f.grant(
                f.domain,
                &f.token(f.domain, &[&f.server.base_url(), "dds"], -10, "expired"),
            ),
            _ => f.grant(
                f.domain,
                &f.token(
                    f.domain,
                    &["https://wrong.example", "dds"],
                    3600,
                    "wrong-aud",
                ),
            ),
        };
        let auth = f.auth(value).await;
        assert!(matches!(
            f.client().list(&JobListQuery::default()).await,
            Err(JobsError::Auth(_) | JobsError::InvalidResponse(_))
        ));
        auth.assert_calls_async(1).await;
        dms.assert_calls_async(0).await;
    }
}

#[tokio::test]
async fn permission_denials_have_status_and_do_not_runaway_retry() {
    for status in [401, 403] {
        let f = Fixture::new(Principal::App).await;
        let (_, auth) = f.valid_auth("denied-grant").await;
        let denied = f
            .server
            .mock_async(|when, then| {
                when.method(GET).path(f.path(""));
                then.status(status).body("provider secret details");
            })
            .await;
        let error = f.client().list(&JobListQuery::default()).await.unwrap_err();
        assert_eq!(error.http_status(), Some(status));
        assert!(!format!("{error:?}").contains("provider secret details"));
        assert_eq!(
            denied.calls_async().await,
            if status == 401 { 2 } else { 1 }
        );
        assert_eq!(auth.calls_async().await, if status == 401 { 2 } else { 1 });
    }
}

#[tokio::test]
async fn concurrent_401_rejections_share_one_replacement_grant_and_both_succeed() {
    let f = Fixture::new(Principal::User).await;
    let (old_token, old_auth) = f.valid_auth("old-generation").await;
    let old_access = f
        .session
        .domain_access(f.domain, None, &CancellationToken::new())
        .await
        .unwrap();
    old_auth.assert_calls_async(1).await;
    old_auth.delete_async().await;

    let new_token = f.token(
        f.domain,
        &[&f.server.base_url(), "dds"],
        7200,
        "new-generation",
    );
    let renewed = f.auth(f.grant(f.domain, &new_token)).await;
    let rejected = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path(f.path(""))
                .header("authorization", format!("Bearer {old_token}"));
            then.delay(Duration::from_millis(50)).status(401);
        })
        .await;
    let accepted = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path(f.path(""))
                .header("authorization", format!("Bearer {new_token}"));
            then.header("content-type", "application/json")
                .json_body(json!({"items":[],"next_cursor":null}));
        })
        .await;
    let first = f.client();
    let second = f.client();
    let first_query = JobListQuery::default();
    let second_query = JobListQuery::default();
    let (a, b) = tokio::join!(first.list(&first_query), second.list(&second_query));
    assert!(a.is_ok() && b.is_ok());
    assert_eq!(old_access.bearer().expose_secret(), old_token);
    rejected.assert_calls_async(2).await;
    renewed.assert_calls_async(1).await;
    accepted.assert_calls_async(2).await;
}

#[tokio::test]
async fn imported_refresh_persistence_failure_blocks_submission_and_retries_same_snapshot() {
    let server = MockServer::start_async().await;
    let domain = Uuid::new_v4();
    let job = Uuid::new_v4();
    let base = server.base_url();
    let discovery = server
        .mock_async(|when, then| {
            when.method(GET).path("/.well-known/openid-configuration");
            then.header("content-type", "application/json")
                .json_body(json!({
                    "issuer":base,
                    "token_endpoint":format!("{base}/oauth/v2/token"),
                    "token_endpoint_auth_methods_supported":["none"],
                    "grant_types_supported":["refresh_token"]
                }));
        })
        .await;
    let refresh = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/oauth/v2/token")
                .body_includes("grant_type=refresh_token")
                .body_includes("refresh_token=old-refresh");
            then.header("content-type", "application/json")
                .json_body(json!({
                    "access_token":"replacement-opaque",
                    "refresh_token":"replacement-refresh",
                    "expires_in":3600,
                    "token_type":"Bearer"
                }));
        })
        .await;
    let service = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/service/domains-access-token")
                .header("authorization", "Bearer replacement-opaque");
            then.header("content-type", "application/json")
                .json_body(json!({"access_token":"dds-service"}));
        })
        .await;
    let token = format!(
        "e30.{}.write-grant",
        URL_SAFE_NO_PAD.encode(
            json!({
                "iss":"dds","domain_id":domain,"aud":[base,"dds"],
                "exp":chrono::Utc::now().timestamp()+3600
            })
            .to_string()
        )
    );
    let domain_auth = server
        .mock_async(|when, then| {
            when.method(POST)
                .path(format!("/api/v1/domains/{domain}/auth"))
                .header("authorization", "Bearer dds-service");
            then.header("content-type", "application/json")
                .json_body(json!({
                    "id":domain,"domain_server":{"url":base},"access_token":token
                }));
        })
        .await;
    let submit = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/jobs")
                .header("authorization", format!("Bearer {token}"));
            then.header("content-type", "application/json")
                .json_body(json!({"job_id":job}));
        })
        .await;
    let store = FailingOnceStore::new();
    let session = AuthClient::new(
        AuthEnvironment::new(&base, &base)
            .unwrap()
            .with_client_id("jobs-contract")
            .unwrap(),
    )
    .unwrap()
    .import_zitadel_session(
        ZitadelSessionCredentials::new(
            "old-access",
            "old-refresh",
            "public-client",
            base.parse().unwrap(),
            Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
        )
        .unwrap(),
        store.clone(),
    )
    .unwrap();
    let client = AukiDmsJobs::new(session, &format!("{base}/v1"))
        .unwrap()
        .in_domain(domain);

    let error = client.submit(&graph_spec()).await.unwrap_err();
    assert_eq!(error.code(), "persistence");
    assert!(matches!(error, JobsError::Auth(AuthError::Persistence)));
    submit.assert_calls_async(0).await;
    service.assert_calls_async(0).await;
    domain_auth.assert_calls_async(0).await;
    assert_eq!(store.attempts.lock().unwrap().len(), 1);

    assert_eq!(client.submit(&graph_spec()).await.unwrap(), job);
    {
        let attempts = store.attempts.lock().unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], attempts[1]);
    }
    discovery.assert_calls_async(1).await;
    refresh.assert_calls_async(1).await;
    service.assert_calls_async(1).await;
    domain_auth.assert_calls_async(1).await;
    submit.assert_calls_async(1).await;
}

#[tokio::test]
async fn pre_cancelled_submit_performs_no_authentication_or_dms_request() {
    let f = Fixture::new(Principal::User).await;
    let (_, auth) = f.valid_auth("unused-cancelled-grant").await;
    let submit = f
        .server
        .mock_async(|when, then| {
            when.method(POST).path(f.path(""));
            then.header("content-type", "application/json")
                .json_body(json!({"job_id":f.job}));
        })
        .await;
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        f.client()
            .submit_with_cancellation(&graph_spec(), &cancellation)
            .await,
        Err(JobsError::Cancelled)
    ));
    auth.assert_calls_async(0).await;
    submit.assert_calls_async(0).await;
}

#[tokio::test]
async fn explicit_or_malformed_ambiguous_submit_responses_are_never_replayed() {
    for case in ["408", "503", "malformed-200"] {
        let f = Fixture::new(Principal::User).await;
        f.valid_auth("ambiguous-response-grant").await;
        let submit = f
            .server
            .mock_async(|when, then| {
                when.method(POST).path(f.path(""));
                match case {
                    "408" => then.status(408),
                    "503" => then.status(503),
                    _ => then
                        .status(200)
                        .header("content-type", "application/json")
                        .body("not-json"),
                };
            })
            .await;
        assert!(matches!(
            f.client().submit(&graph_spec()).await,
            Err(JobsError::SubmissionUncertain { source }) if match case {
                "408" => matches!(*source, JobsError::HttpStatus { status: 408 }),
                "503" => matches!(*source, JobsError::HttpStatus { status: 503 }),
                _ => matches!(*source, JobsError::InvalidResponse(_)),
            }
        ));
        submit.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn timeout_and_cancellation_make_submit_uncertain_without_replay() {
    for (cancel, keyed) in [(false, false), (true, false), (false, true), (true, true)] {
        let f = Fixture::new(Principal::User).await;
        f.valid_auth("ambiguous-submit-grant").await;
        let submit = f
            .server
            .mock_async(|when, then| {
                when.method(POST).path(f.path(""));
                then.delay(Duration::from_secs(2))
                    .header("content-type", "application/json")
                    .json_body(json!({"job_id":f.job}));
            })
            .await;
        let client = AukiDmsJobs::with_limits(
            f.session.clone(),
            &f.dms_base(),
            JobsLimits {
                request_timeout: Duration::from_millis(100),
                ..JobsLimits::default()
            },
        )
        .unwrap()
        .in_domain(f.domain);
        let cancellation = CancellationToken::new();
        let work = tokio::spawn({
            let client = client.clone();
            let cancellation = cancellation.clone();
            async move {
                if keyed {
                    client
                        .submit_with_key_and_cancellation(
                            &graph_spec(),
                            "cancel-fixture",
                            &cancellation,
                        )
                        .await
                } else {
                    client
                        .submit_with_cancellation(&graph_spec(), &cancellation)
                        .await
                }
            }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while submit.calls_async().await == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if cancel {
            cancellation.cancel();
        }
        let error = work.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            JobsError::SubmissionUncertain { source }
                if matches!(*source, JobsError::TimedOut | JobsError::Cancelled)
        ));
        submit.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn transport_failure_after_starting_submit_is_uncertain_and_not_retried() {
    let f = Fixture::new(Principal::User).await;
    f.valid_auth("transport-grant").await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = AukiDmsJobs::new(f.session.clone(), &format!("http://{address}/v1"))
        .unwrap()
        .in_domain(f.domain);
    assert!(matches!(
        client.submit(&graph_spec()).await,
        Err(JobsError::SubmissionUncertain { source })
            if matches!(*source, JobsError::Transport)
    ));
}

#[tokio::test]
async fn invalid_specs_and_queries_fail_before_authentication() {
    let f = Fixture::new(Principal::User).await;
    let (_, auth) = f.valid_auth("unused-grant").await;
    let empty = JobSpec {
        label: "empty".into(),
        priority: 0,
        meta: json!({}),
        tasks: vec![],
        edges: vec![],
    };
    assert!(matches!(
        f.client().submit(&empty).await,
        Err(JobsError::InvalidInput(_))
    ));
    assert!(matches!(
        f.client()
            .list(&JobListQuery {
                limit: 0,
                ..JobListQuery::default()
            })
            .await,
        Err(JobsError::InvalidInput(_))
    ));
    auth.assert_calls_async(0).await;
}

#[tokio::test]
async fn closing_the_shared_session_cancels_an_inflight_request_without_retry() {
    let f = Fixture::new(Principal::User).await;
    f.valid_auth("session-close-grant").await;
    let delayed = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(f.path(""));
            then.delay(Duration::from_secs(2))
                .header("content-type", "application/json")
                .json_body(json!({"items":[],"next_cursor":null}));
        })
        .await;
    let client = f.client();
    let work = tokio::spawn(async move { client.list(&JobListQuery::default()).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while delayed.calls_async().await == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    f.session.close().await;
    assert!(matches!(
        work.await.unwrap(),
        Err(JobsError::Auth(AuthError::SessionClosed))
    ));
    delayed.assert_calls_async(1).await;
}

#[tokio::test]
async fn closing_client_or_session_during_submit_is_uncertain_and_never_replayed() {
    for close_session in [false, true] {
        let f = Fixture::new(Principal::User).await;
        f.valid_auth("inflight-close-grant").await;
        let submit = f
            .server
            .mock_async(|when, then| {
                when.method(POST).path(f.path(""));
                then.delay(Duration::from_secs(2))
                    .header("content-type", "application/json")
                    .json_body(json!({"job_id":f.job}));
            })
            .await;
        let client = f.client();
        let work = tokio::spawn({
            let client = client.clone();
            async move { client.submit(&graph_spec()).await }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while submit.calls_async().await == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if close_session {
            f.session.close().await;
        } else {
            tokio::time::timeout(Duration::from_millis(500), client.close())
                .await
                .expect("client close must drain the submission future");
        }
        assert!(matches!(
            work.await.unwrap(),
            Err(JobsError::SubmissionUncertain { source })
                if if close_session {
                    matches!(*source, JobsError::Auth(AuthError::SessionClosed))
                } else {
                    matches!(*source, JobsError::Closed)
                }
        ));
        submit.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn close_cancels_inflight_work_but_keeps_sibling_clients_and_session_alive() {
    let f = Fixture::new(Principal::User).await;
    f.valid_auth("close-grant").await;
    let delayed = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(f.path(""));
            then.delay(Duration::from_secs(2))
                .header("content-type", "application/json")
                .json_body(json!({"items":[],"next_cursor":null}));
        })
        .await;
    let first = f.client();
    let sibling = f.client();
    let work = tokio::spawn({
        let first = first.clone();
        async move { first.list(&JobListQuery::default()).await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while delayed.calls_async().await == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_millis(500), first.close())
        .await
        .expect("close must drain its request");
    assert!(matches!(work.await.unwrap(), Err(JobsError::Closed)));
    assert!(matches!(
        first.list(&JobListQuery::default()).await,
        Err(JobsError::Closed)
    ));

    let cancel = f
        .server
        .mock_async(|when, then| {
            when.method(POST)
                .path(f.path(&format!("/{}/cancel", f.job)));
            then.header("content-type", "application/json")
                .json_body(json!({
                    "id":f.job,"status":"canceled","updated_at":"2026-09-15T01:05:00Z"
                }));
        })
        .await;
    sibling.cancel(f.job).await.unwrap();
    cancel.assert_calls_async(1).await;

    f.session.close().await;
    assert!(matches!(
        sibling.cancel(f.job).await,
        Err(JobsError::Auth(AuthError::SessionClosed))
    ));
}

#[tokio::test]
async fn keyed_submit_preserves_key_and_body_through_one_grant_renewal() {
    for principal in [Principal::User, Principal::App, Principal::ImportedZitadel] {
        let f = Fixture::new(principal).await;
        let (old_token, old_auth) = f.valid_auth("keyed-old").await;
        f.session
            .domain_access(f.domain, None, &CancellationToken::new())
            .await
            .unwrap();
        old_auth.delete_async().await;
        let (new_token, renewed) = f.valid_auth("keyed-new").await;
        let spec = graph_spec();
        let mut body = serde_json::to_value(&spec).unwrap();
        body["domain_id"] = json!(f.domain);
        let key = "k".repeat(128);
        let rejected = f
            .server
            .mock_async(|when, then| {
                when.method(POST)
                    .path(f.path(""))
                    .header("authorization", format!("Bearer {old_token}"))
                    .header("Idempotency-Key", &key)
                    .json_body(body.clone());
                then.status(401);
            })
            .await;
        let accepted = f
            .server
            .mock_async(|when, then| {
                when.method(POST)
                    .path(f.path(""))
                    .header("authorization", format!("Bearer {new_token}"))
                    .header("Idempotency-Key", &key)
                    .json_body(body.clone());
                then.header("content-type", "application/json")
                    .json_body(json!({"job_id": f.job}));
            })
            .await;
        assert_eq!(
            f.client().submit_with_key(&spec, &key).await.unwrap(),
            f.job
        );
        renewed.assert_calls_async(1).await;
        rejected.assert_calls_async(1).await;
        accepted.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn uncertain_keyed_submit_requires_explicit_same_key_recovery() {
    let f = Fixture::new(Principal::User).await;
    f.valid_auth("keyed-recovery").await;
    let client = f.client();
    let spec = graph_spec();
    let mut body = serde_json::to_value(&spec).unwrap();
    body["domain_id"] = json!(f.domain);
    let failed = f
        .server
        .mock_async(|when, then| {
            when.method(POST)
                .path(f.path(""))
                .header("Idempotency-Key", "persisted-operation")
                .json_body(body.clone());
            then.status(503)
                .header("Retry-After", "1")
                .body("private backend detail");
        })
        .await;
    let error = client
        .submit_with_key(&spec, "persisted-operation")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "submission_uncertain");
    assert_eq!(error.http_status(), Some(503));
    assert!(!format!("{error:?}").contains("private backend detail"));
    failed.assert_calls_async(1).await;
    failed.delete_async().await;
    let recovered = f
        .server
        .mock_async(|when, then| {
            when.method(POST)
                .path(f.path(""))
                .header("Idempotency-Key", "persisted-operation")
                .json_body(body.clone());
            then.header("content-type", "application/json")
                .json_body(json!({"job_id": f.job}));
        })
        .await;
    for _ in 0..2 {
        assert_eq!(
            client
                .submit_with_key(&spec, "persisted-operation")
                .await
                .unwrap(),
            f.job
        );
    }
    recovered.assert_calls_async(2).await;
}

#[tokio::test]
async fn only_keyed_conflicts_with_valid_retry_hints_are_in_progress() {
    for (keyed, hint, expected) in [
        (true, Some("1"), Some(1)),
        (true, Some("0"), Some(0)),
        (false, Some("1"), None),
        (true, None, None),
        (true, Some("+1"), None),
        (true, Some("1, 2"), None),
        (true, Some("4294967296"), None),
        (true, Some("Thu, 17 Sep 2026 09:00:00 GMT"), None),
    ] {
        let f = Fixture::new(Principal::User).await;
        f.valid_auth("conflict-grant").await;
        let conflict = f
            .server
            .mock_async(|when, then| {
                when.method(POST).path(f.path(""));
                let then = then.status(409).body("private conflict detail");
                if let Some(hint) = hint {
                    then.header("Retry-After", hint);
                }
            })
            .await;
        let client = f.client();
        let error = if keyed {
            client
                .submit_with_key(&graph_spec(), "busy-operation")
                .await
        } else {
            client.submit(&graph_spec()).await
        }
        .unwrap_err();
        assert_eq!(error.http_status(), Some(409));
        assert_eq!(error.retry_after_seconds(), expected);
        assert_eq!(
            error.code(),
            if expected.is_some() {
                "submission_in_progress"
            } else {
                "http_status"
            }
        );
        assert!(!format!("{error:?}").contains("private conflict detail"));
        conflict.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn invalid_keys_and_pre_cancelled_keyed_requests_never_authenticate() {
    let f = Fixture::new(Principal::User).await;
    let (_, auth) = f.valid_auth("unused-key-grant").await;
    let client = f.client();
    for key in [
        "".into(),
        "x".repeat(129),
        "private key".into(),
        "private\nkey".into(),
        "private\tkey".into(),
        "private\u{7f}key".into(),
        "privaté".into(),
    ] {
        let error = client
            .submit_with_key(&graph_spec(), &key)
            .await
            .unwrap_err();
        assert!(matches!(error, JobsError::InvalidInput(_)));
        assert!(!format!("{error:?}").contains("private"));
    }
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        client
            .submit_with_key_and_cancellation(&graph_spec(), "valid-key", &cancel)
            .await,
        Err(JobsError::Cancelled)
    ));
    client.close().await;
    assert!(matches!(
        client.submit_with_key(&graph_spec(), "valid-key").await,
        Err(JobsError::Closed)
    ));
    auth.assert_calls_async(0).await;
}
