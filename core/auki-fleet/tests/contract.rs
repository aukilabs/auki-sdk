#![cfg(not(target_arch = "wasm32"))]
use auki_auth::{
    AuthClient, AuthEnvironment, AuthSession, Credentials, DomainAccessProvider,
    ZitadelSessionCredentials, ZitadelSessionStore,
};
use auki_dms::jobs::JobMode;
use auki_fleet::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use httpmock::{
    Method::{GET, POST},
    Mock, MockServer,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Fixture {
    server: MockServer,
    session: AuthSession,
    domain: Uuid,
    org: Uuid,
    robot: Uuid,
    node: Uuid,
    job: Uuid,
    task: Uuid,
}

fn token(claims: Value) -> String {
    format!("e30.{}.fixture", URL_SAFE_NO_PAD.encode(claims.to_string()))
}

impl Fixture {
    async fn new(app: bool) -> Self {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/user/login");
                then.header("content-type", "application/json").json_body(
                    json!({"access_token":"fixture-user","refresh_token":"fixture-refresh"}),
                );
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/service/domains-access-token");
                then.header("content-type", "application/json")
                    .json_body(json!({"access_token":"fixture-dds"}));
            })
            .await;
        let auth =
            AuthClient::new(AuthEnvironment::new(server.base_url(), server.base_url()).unwrap())
                .unwrap();
        let session = auth
            .authenticate(if app {
                Credentials::app("fixture-key", "fixture-secret")
            } else {
                Credentials::user_password("fixture@example.test", "fixture-password")
            })
            .await
            .unwrap();
        let fixture = Self {
            server,
            session,
            domain: Uuid::new_v4(),
            org: Uuid::new_v4(),
            robot: Uuid::new_v4(),
            node: Uuid::new_v4(),
            job: Uuid::new_v4(),
            task: Uuid::new_v4(),
        };
        let grant = token(
            json!({"iss":"dds", "type":if app {"app-access"} else {"user-access"},
            "org":fixture.org,"domain_id":fixture.domain,"aud":[fixture.server.base_url(),"dds"],
            "exp":chrono::Utc::now().timestamp()+3600}),
        );
        fixture.server.mock_async(|when, then| {
            when.method(POST).path(format!("/api/v1/domains/{}/auth", fixture.domain));
            then.header("content-type", "application/json").json_body(json!({"id":fixture.domain,"domain_server":{"url":fixture.server.base_url()},"access_token":grant}));
        }).await;
        fixture
    }
    fn client(&self) -> DomainFleetClient {
        AukiFleet::new(
            self.session.clone(),
            &format!("{}/v1/", self.server.base_url()),
        )
        .unwrap()
        .in_domain(self.domain)
    }
    async fn get(&self, path: impl Into<String>, value: Value) -> Mock<'_> {
        let path = path.into();
        self.server
            .mock_async(|when, then| {
                when.method(GET).path(path);
                then.header("content-type", "application/json")
                    .json_body(value);
            })
            .await
    }
    fn robot_json(&self) -> Value {
        json!({"id":self.robot,"organization_id":self.org,"assigned_domain_id":self.domain,
            "name":"inspection robot","capabilities":["com.example.inspect.v1"],"status":"online",
            "last_seen_at":chrono::Utc::now().to_rfc3339(),"active_lease_expires_at":null})
    }
    fn node_json(&self) -> Value {
        json!({"id":self.node,"organization_id":self.org,"name":"converter",
            "capabilities":["com.example.convert.v1"],"mode":"dedicated","status":"online"})
    }
    async fn inventory(&self) {
        self.get(
            format!("/api/v1/domains/{}/robots", self.domain),
            json!({"robots":[self.robot_json()]}),
        )
        .await;
        self.get("/api/v1/nodes", json!({"nodes":[self.node_json()]}))
            .await;
    }
    fn summary(&self, running: u32) -> Value {
        json!({"queued":0,"leased":0,"running":running,"completed":0,"failed":0,"canceled":0})
    }
    fn job_json(&self) -> Value {
        json!({"id":self.job,"label":"inspection","domain_id":self.domain,"organization_id":self.org,
            "status":"canceled","priority":0,"created_at":chrono::Utc::now().to_rfc3339(),
            "updated_at":chrono::Utc::now().to_rfc3339(),"meta":{}})
    }
    fn task_json(&self, worker: Uuid, seconds: i64) -> Value {
        let now = chrono::Utc::now();
        json!({"id":self.task,"job_id":self.job,"label":"inspect","stage":"inspect",
            "capability":"com.example.inspect.v1","capability_filters":{},"status":"running",
            "deps_remaining":0,"priority":0,"inputs_cids":[],"outputs_prefix":null,
            "organization_id":self.org,"attempts":1,"max_attempts":1,"reserved_by":worker,
            "lease_expires_at":(now+chrono::Duration::seconds(seconds)).to_rfc3339(),"meta":{},
            "last_heartbeat_at":now.to_rfc3339(),"created_at":now.to_rfc3339(),"updated_at":now.to_rfc3339(),
            "mode":"dedicated","billing_units":"0"})
    }
    async fn activity(&self, worker: Option<Uuid>, seconds: i64) {
        let items = if worker.is_some() {
            vec![json!({"job":self.job_json(),"tasks_summary":self.summary(1)})]
        } else {
            vec![]
        };
        self.get("/v1/jobs", json!({"items":items,"next_cursor":null}))
            .await;
        if let Some(worker) = worker {
            self.get(
                format!("/v1/jobs/{}", self.job),
                json!({"job":self.job_json(),"tasks_summary":self.summary(1),
                "tasks":[self.task_json(worker, seconds)],"receipts":[]}),
            )
            .await;
        }
    }
    async fn busy(&self, worker: Option<Uuid>) -> Mock<'_> {
        let nodes = worker
            .map(|worker| {
                vec![
                    json!({"node_id":worker,"task_id":self.task,"job_id":self.job,
            "task_status":"running","job_status":"canceled","task_mode":"dedicated"}),
                ]
            })
            .unwrap_or_default();
        self.get("/v1/nodes/busy", json!({"nodes":nodes})).await
    }
}

fn source(snapshot: &FleetSnapshot, source: FleetSource) -> &FleetSourceReport {
    snapshot
        .sources
        .iter()
        .find(|report| report.source == source)
        .unwrap()
}

#[tokio::test]
async fn domain_inventory_and_candidate_pool_keep_different_associations() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.activity(Some(f.robot), 120).await;
    f.busy(Some(f.robot)).await;
    let fleet = f.client();
    let domain = fleet.list(&FleetQuery::default()).await.unwrap();
    assert!(domain.complete);
    for kind in [FleetSource::Nodes, FleetSource::Robots] {
        assert_eq!(source(&domain, kind).state, FleetSourceState::Complete);
        assert_eq!(
            source(&domain, kind).codes,
            ["legacy_inventory_unpaginated"]
        );
    }
    assert_eq!(domain.machines.len(), 1);
    assert_eq!(domain.machines[0].kind, FleetMachineKind::Robot);
    assert_eq!(domain.machines[0].association, FleetAssociation::Assigned);
    assert_eq!(domain.machines[0].work_state, FleetWorkState::Busy);
    // Terminal job status does not erase the still-running task.
    assert_eq!(domain.machines[0].activity.len(), 1);
    let pool = fleet
        .compute_pool(&ComputePoolQuery {
            mode: JobMode::Dedicated,
            capabilities: vec!["com.example.convert.v1".into()],
            match_all_capabilities: true,
        })
        .await
        .unwrap();
    assert_eq!(pool.machines.len(), 1);
    assert_eq!(pool.machines[0].association, FleetAssociation::Candidate);
    assert_eq!(pool.machines[0].work_state, FleetWorkState::Idle);
    assert!(pool.machines[0].last_seen_at.is_none());
    fleet.close().await;
    assert_eq!(
        fleet.list(&FleetQuery::default()).await.unwrap_err().code(),
        "closed"
    );
    // Closing one fleet handle never closes the shared login.
    assert!(
        f.session
            .inventory_nodes(&CancellationToken::new())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn app_can_observe_domain_tasks_but_cannot_infer_global_busy_or_idle() {
    let f = Fixture::new(true).await;
    f.inventory().await;
    f.activity(Some(f.node), 120).await;
    let busy = f.busy(Some(f.node)).await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert!(!snapshot.complete);
    assert_eq!(snapshot.machines.len(), 2);
    assert_eq!(
        source(&snapshot, FleetSource::Busy).state,
        FleetSourceState::Unsupported
    );
    assert!(
        snapshot
            .machines
            .iter()
            .all(|machine| machine.work_state == FleetWorkState::Unknown)
    );
    let node = snapshot
        .machines
        .iter()
        .find(|machine| machine.id == f.node)
        .unwrap();
    assert_eq!(node.association, FleetAssociation::ActiveTask);
    assert_eq!(node.activity.len(), 1);
    busy.assert_calls_async(0).await;
}

#[tokio::test]
async fn missing_inventory_preserves_robots_and_unresolved_task_identity() {
    let f = Fixture::new(false).await;
    f.get(
        format!("/api/v1/domains/{}/robots", f.domain),
        json!({"robots":[f.robot_json()]}),
    )
    .await;
    f.server
        .mock_async(|when, then| {
            when.method(GET).path("/api/v1/nodes");
            then.status(503);
        })
        .await;
    f.activity(Some(f.node), 120).await;
    f.busy(Some(f.node)).await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert_eq!(snapshot.machines.len(), 1);
    assert_eq!(snapshot.unresolved_activity[0].worker_id, f.node);
    assert_eq!(
        source(&snapshot, FleetSource::Nodes).state,
        FleetSourceState::Unavailable
    );
    assert!(!snapshot.complete);
}

#[tokio::test]
async fn denied_busy_feed_does_not_convert_missing_activity_to_idle() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.activity(None, 0).await;
    f.server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/nodes/busy");
            then.status(403);
        })
        .await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert_eq!(snapshot.machines[0].work_state, FleetWorkState::Unknown);
    assert_eq!(
        source(&snapshot, FleetSource::Busy).state,
        FleetSourceState::Denied
    );
    assert_eq!(source(&snapshot, FleetSource::Busy).http_status, Some(403));
}

#[tokio::test]
async fn expired_task_lease_does_not_assert_current_work_state() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.activity(Some(f.robot), -30).await;
    f.busy(Some(f.robot)).await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert_eq!(snapshot.machines[0].work_state, FleetWorkState::Unknown);
    assert_eq!(snapshot.machines[0].activity.len(), 1);
}

#[tokio::test]
async fn conflicting_busy_assignment_preserves_activity_without_claiming_busy() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.activity(Some(f.robot), 120).await;
    f.get(
        "/v1/nodes/busy",
        json!({"nodes":[{"node_id":f.robot,"task_id":Uuid::new_v4(),
        "job_id":f.job,"task_status":"running","job_status":"running","task_mode":"dedicated"}]}),
    )
    .await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert_eq!(snapshot.machines[0].work_state, FleetWorkState::Unknown);
    assert_eq!(snapshot.machines[0].activity[0].task_id, f.task);
}

#[tokio::test]
async fn absent_busy_entry_cannot_make_offline_or_unknown_presence_idle() {
    for status in ["offline", "future-provider-status"] {
        let f = Fixture::new(false).await;
        let mut robot = f.robot_json();
        robot["status"] = json!(status);
        f.get(
            format!("/api/v1/domains/{}/robots", f.domain),
            json!({"robots":[robot]}),
        )
        .await;
        f.get("/api/v1/nodes", json!({"nodes":[]})).await;
        f.activity(None, 0).await;
        f.busy(None).await;
        let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
        assert_eq!(snapshot.machines[0].work_state, FleetWorkState::Unknown);
        assert_eq!(snapshot.machines[0].provider_status, status);
    }
}

#[tokio::test]
async fn broad_busy_feed_does_not_disclose_unrelated_job_references() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.activity(None, 0).await;
    f.busy(Some(f.robot)).await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert_eq!(snapshot.machines[0].work_state, FleetWorkState::Busy);
    assert!(snapshot.machines[0].activity.is_empty());
    assert!(
        !serde_json::to_string(&snapshot)
            .unwrap()
            .contains(&f.job.to_string())
    );
}

#[tokio::test]
async fn busy_absence_does_not_cover_other_organizations_dedicated_robots() {
    let f = Fixture::new(false).await;
    let mut robot = f.robot_json();
    robot["organization_id"] = json!(Uuid::new_v4());
    f.get(
        format!("/api/v1/domains/{}/robots", f.domain),
        json!({"robots":[robot]}),
    )
    .await;
    f.get("/api/v1/nodes", json!({"nodes":[]})).await;
    f.activity(None, 0).await;
    f.busy(None).await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert_eq!(snapshot.machines[0].work_state, FleetWorkState::Unknown);
}

#[tokio::test]
async fn wrong_domain_robot_fails_closed() {
    let f = Fixture::new(false).await;
    let mut robot = f.robot_json();
    robot["assigned_domain_id"] = json!(Uuid::new_v4());
    f.get(
        format!("/api/v1/domains/{}/robots", f.domain),
        json!({"robots":[robot]}),
    )
    .await;
    f.get("/api/v1/nodes", json!({"nodes":[]})).await;
    f.activity(None, 0).await;
    f.busy(None).await;
    assert_eq!(
        f.client()
            .list(&FleetQuery::default())
            .await
            .unwrap_err()
            .code(),
        "invalid_response"
    );
}

#[tokio::test]
async fn duplicate_or_ambiguous_worker_identity_fails_closed() {
    let f = Fixture::new(false).await;
    let mut node = f.node_json();
    node["id"] = json!(f.robot);
    f.get(
        format!("/api/v1/domains/{}/robots", f.domain),
        json!({"robots":[f.robot_json()]}),
    )
    .await;
    f.get("/api/v1/nodes", json!({"nodes":[node]})).await;
    f.activity(None, 0).await;
    f.busy(None).await;
    assert_eq!(
        f.client()
            .list(&FleetQuery::default())
            .await
            .unwrap_err()
            .code(),
        "invalid_response"
    );
}

#[tokio::test]
async fn node_filters_preserve_custom_capabilities_and_exclude_infrastructure() {
    let f = Fixture::new(false).await;
    let mut relay = f.node_json();
    relay["id"] = json!(Uuid::new_v4());
    relay["capabilities"] = json!(["/p2p/circuit-relay/v1"]);
    f.get("/api/v1/nodes", json!({"nodes":[f.node_json(),relay]}))
        .await;
    f.activity(None, 0).await;
    f.busy(None).await;
    let fleet = f.client();
    let mut query = ComputePoolQuery {
        mode: JobMode::Dedicated,
        capabilities: vec![],
        match_all_capabilities: false,
    };
    assert_eq!(fleet.compute_pool(&query).await.unwrap().machines.len(), 1);
    query.capabilities = vec!["com.example.convert.v1".into(), "different".into()];
    assert_eq!(fleet.compute_pool(&query).await.unwrap().machines.len(), 1);
    query.match_all_capabilities = true;
    assert!(
        fleet
            .compute_pool(&query)
            .await
            .unwrap()
            .machines
            .is_empty()
    );
    query.mode = JobMode::Public;
    query.capabilities.clear();
    assert!(
        fleet
            .compute_pool(&query)
            .await
            .unwrap()
            .machines
            .is_empty()
    );
}

async fn job_page<'a>(
    f: &'a Fixture,
    cursor: Option<&str>,
    items: Value,
    next_cursor: Option<&str>,
) -> Mock<'a> {
    f.server
        .mock_async(|when, then| {
            let when = when.method(GET).path("/v1/jobs").query_param("limit", "50");
            if let Some(cursor) = cursor {
                when.query_param("cursor", cursor);
            } else {
                when.query_param_missing("cursor");
            }
            then.header("content-type", "application/json")
                .json_body(json!({"items": items, "next_cursor": next_cursor}));
        })
        .await
}

#[tokio::test]
async fn complete_job_pagination_preserves_activity_on_both_sides_of_boundary() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    let next_job = Uuid::new_v4();
    let next_task = Uuid::new_v4();
    let mut node_job = f.job_json();
    node_job["id"] = json!(next_job);
    let mut node_task = f.task_json(f.node, 120);
    node_task["id"] = json!(next_task);
    node_task["job_id"] = json!(next_job);
    node_task["capability"] = json!("com.example.convert.v1");
    let mut items: Vec<Value> = (0..49)
        .map(|_| {
            let mut job = f.job_json();
            job["id"] = json!(Uuid::new_v4());
            job["status"] = json!("completed");
            json!({"job":job,"tasks_summary":f.summary(0)})
        })
        .collect();
    items.push(json!({"job":f.job_json(),"tasks_summary":f.summary(1)}));
    // Exact opaque cursor matching also checks that the SDK does not synthesize it.
    let cursor = "boundary/+== opaque";
    let first = job_page(&f, None, json!(items), Some(cursor)).await;
    let second = job_page(
        &f,
        Some(cursor),
        json!([{"job":node_job,"tasks_summary":f.summary(1)}]),
        None,
    )
    .await;
    let robot_details = f
        .get(
            format!("/v1/jobs/{}", f.job),
            json!({"job":f.job_json(),"tasks_summary":f.summary(1),
            "tasks":[f.task_json(f.robot,120)],"receipts":[]}),
        )
        .await;
    let node_details = f
        .get(
            format!("/v1/jobs/{next_job}"),
            json!({"job":node_job,"tasks_summary":f.summary(1),
            "tasks":[node_task],"receipts":[]}),
        )
        .await;
    f.get(
        "/v1/nodes/busy",
        json!({"nodes":[
            {"node_id":f.robot,"task_id":f.task,"job_id":f.job,
             "task_status":"running","job_status":"canceled","task_mode":"dedicated"},
            {"node_id":f.node,"task_id":next_task,"job_id":next_job,
             "task_status":"running","job_status":"canceled","task_mode":"dedicated"}
        ]}),
    )
    .await;
    let fleet = f.client();
    let domain = fleet.list(&FleetQuery::default()).await.unwrap();
    let pool = fleet
        .compute_pool(&ComputePoolQuery {
            mode: JobMode::Dedicated,
            capabilities: vec![],
            match_all_capabilities: false,
        })
        .await
        .unwrap();
    for snapshot in [&domain, &pool] {
        assert!(snapshot.complete);
        let jobs = source(snapshot, FleetSource::Jobs);
        assert_eq!(jobs.state, FleetSourceState::Complete);
        assert!(jobs.codes.is_empty());
        let node = snapshot.machines.iter().find(|m| m.id == f.node).unwrap();
        assert_eq!(node.activity.len(), 1);
        assert_eq!(node.activity[0].task_id, next_task);
        assert_eq!(node.work_state, FleetWorkState::Busy);
    }
    let robot = domain.machines.iter().find(|m| m.id == f.robot).unwrap();
    assert_eq!(robot.activity.len(), 1);
    assert_eq!(robot.activity[0].task_id, f.task);
    assert_eq!(robot.work_state, FleetWorkState::Busy);
    assert_eq!(domain.machines.len(), 2);
    assert_eq!(pool.machines.len(), 1);
    first.assert_calls_async(2).await;
    second.assert_calls_async(2).await;
    robot_details.assert_calls_async(2).await;
    node_details.assert_calls_async(2).await;
}

#[tokio::test]
async fn repeated_job_cursor_remains_explicitly_partial() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.busy(None).await;
    f.get(
        "/v1/jobs",
        json!({"items":[],"next_cursor":"opaque-cursor"}),
    )
    .await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert!(!snapshot.complete);
    let jobs = source(&snapshot, FleetSource::Jobs);
    assert_eq!(jobs.codes, vec!["repeated_job_cursor"]);
}

#[tokio::test]
async fn job_page_limit_remains_explicitly_partial() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.busy(None).await;
    let first = job_page(&f, None, json!([]), Some("next-page")).await;
    let unvisited = job_page(&f, Some("next-page"), json!([]), None).await;
    let fleet = AukiFleet::with_limits(
        f.session.clone(),
        &format!("{}/v1/", f.server.base_url()),
        FleetLimits {
            max_job_pages: 1,
            ..Default::default()
        },
    )
    .unwrap()
    .in_domain(f.domain);
    let snapshot = fleet.list(&FleetQuery::default()).await.unwrap();
    assert!(!snapshot.complete);
    let jobs = source(&snapshot, FleetSource::Jobs);
    assert_eq!(jobs.state, FleetSourceState::Partial);
    assert_eq!(jobs.codes, vec!["job_page_limit"]);
    first.assert_calls_async(1).await;
    unvisited.assert_calls_async(0).await;
}

#[tokio::test]
async fn duplicate_jobs_across_pages_remain_explicitly_partial() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.busy(None).await;
    let items = json!([{"job":f.job_json(),"tasks_summary":f.summary(0)}]);
    let first = job_page(&f, None, items.clone(), Some("next-page")).await;
    let second = job_page(&f, Some("next-page"), items, None).await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert!(!snapshot.complete);
    let jobs = source(&snapshot, FleetSource::Jobs);
    assert_eq!(jobs.state, FleetSourceState::Partial);
    assert_eq!(jobs.codes, vec!["changing_job_pages"]);
    first.assert_calls_async(1).await;
    second.assert_calls_async(1).await;
}

#[tokio::test]
async fn later_job_page_failure_preserves_earlier_activity_as_partial() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.busy(Some(f.robot)).await;
    let first = job_page(
        &f,
        None,
        json!([{"job":f.job_json(),"tasks_summary":f.summary(1)}]),
        Some("next-page"),
    )
    .await;
    let failed = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/jobs")
                .query_param("cursor", "next-page");
            then.status(503);
        })
        .await;
    let details = f
        .get(
            format!("/v1/jobs/{}", f.job),
            json!({"job":f.job_json(),"tasks_summary":f.summary(1),
            "tasks":[f.task_json(f.robot,120)],"receipts":[]}),
        )
        .await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert!(!snapshot.complete);
    let jobs = source(&snapshot, FleetSource::Jobs);
    assert_eq!(jobs.state, FleetSourceState::Partial);
    assert_eq!(jobs.http_status, Some(503));
    assert!(!jobs.codes.is_empty());
    let robot = snapshot.machines.iter().find(|m| m.id == f.robot).unwrap();
    assert_eq!(robot.activity.len(), 1);
    assert_eq!(robot.activity[0].task_id, f.task);
    first.assert_calls_async(1).await;
    failed.assert_calls_async(1).await;
    details.assert_calls_async(1).await;
}

#[tokio::test]
async fn cancellation_and_close_drain_inflight_work() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.busy(None).await;
    let delayed = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/jobs");
            then.delay(Duration::from_secs(5))
                .header("content-type", "application/json")
                .json_body(json!({"items":[],"next_cursor":null}));
        })
        .await;
    let fleet = f.client();
    let operation = tokio::spawn({
        let fleet = fleet.clone();
        async move { fleet.list(&FleetQuery::default()).await }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while delayed.calls_async().await == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), fleet.close())
        .await
        .unwrap();
    assert_eq!(operation.await.unwrap().unwrap_err().code(), "closed");
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        f.client()
            .list_with_cancellation(&FleetQuery::default(), &token)
            .await
            .unwrap_err()
            .code(),
        "cancelled"
    );
    f.session.close().await;
    assert_eq!(
        f.client()
            .list(&FleetQuery::default())
            .await
            .unwrap_err()
            .code(),
        "closed"
    );
}

struct NoopStore;
#[async_trait::async_trait]
impl ZitadelSessionStore for NoopStore {
    async fn save(&self, _: &ZitadelSessionCredentials) -> Result<(), auki_auth::Error> {
        panic!("unexpected refresh")
    }
}

#[tokio::test]
async fn imported_viewer_and_scoped_user_never_query_broad_node_inventory() {
    for viewer in [true, false] {
        let server = MockServer::start_async().await;
        let domain = Uuid::new_v4();
        let grant = token(
            json!({"iss":"api","type":if viewer {"app-access"} else {"user-access"},
            "org":Uuid::new_v4(),"sub":"fixture-human","aud":["domain-service"],
            "domains":if viewer {vec![]} else {vec![domain]},
            "iat":chrono::Utc::now().timestamp()-30,"exp":chrono::Utc::now().timestamp()+3600}),
        );
        server
            .mock_async(|when, then| {
                when.method(POST).path("/service/domains-access-token");
                then.header("content-type", "application/json")
                    .json_body(json!({"access_token":grant}));
            })
            .await;
        let nodes = server
            .mock_async(|when, then| {
                when.method(GET).path("/api/v1/nodes");
                then.header("content-type", "application/json")
                    .json_body(json!({"nodes":[]}));
            })
            .await;
        let robots = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path(format!("/api/v1/domains/{domain}/robots"));
                then.header("content-type", "application/json")
                    .json_body(json!({"robots":[]}));
            })
            .await;
        let auth =
            AuthClient::new(AuthEnvironment::new(server.base_url(), server.base_url()).unwrap())
                .unwrap();
        let session = auth
            .import_zitadel_session(
                ZitadelSessionCredentials::new(
                    "fixture-access",
                    "fixture-refresh",
                    "public-client",
                    server.base_url().parse().unwrap(),
                    None,
                )
                .unwrap(),
                Arc::new(NoopStore),
            )
            .unwrap();
        let cancel = CancellationToken::new();
        assert!(session.inventory_nodes(&cancel).await.unwrap().is_none());
        assert_eq!(
            session
                .inventory_robots(domain, &cancel)
                .await
                .unwrap()
                .is_some(),
            !viewer
        );
        if !viewer {
            assert!(matches!(
                session.inventory_robots(Uuid::new_v4(), &cancel).await,
                Err(auki_auth::Error::DomainNotAccessible)
            ));
        }
        nodes.assert_calls_async(0).await;
        robots.assert_calls_async(if viewer { 0 } else { 1 }).await;
    }
}

fn imported_claims() -> Value {
    json!({"iss":"api","type":"user-access","org":Uuid::new_v4(),"sub":"fixture-human",
        "aud":["domain-service"],"domains":[],"iat":chrono::Utc::now().timestamp()-30,
        "exp":chrono::Utc::now().timestamp()+3600})
}

fn imported(
    server: &MockServer,
    store: Arc<dyn ZitadelSessionStore>,
    expired: bool,
) -> AuthSession {
    AuthClient::new(AuthEnvironment::new(server.base_url(), server.base_url()).unwrap())
        .unwrap()
        .import_zitadel_session(
            ZitadelSessionCredentials::new(
                "old-access",
                "old-refresh",
                "public-client",
                server.base_url().parse().unwrap(),
                expired.then(|| chrono::Utc::now() - chrono::Duration::seconds(1)),
            )
            .unwrap(),
            store,
        )
        .unwrap()
}

#[tokio::test]
async fn imported_inventory_rejects_wrong_audience_issuer_expiry_and_organization() {
    for (field, value) in [
        ("aud", json!(["wrong-service"])),
        ("iss", json!("wrong-issuer")),
        ("exp", json!(1)),
        ("org", json!(Uuid::nil())),
    ] {
        let server = MockServer::start_async().await;
        let mut claims = imported_claims();
        claims[field] = value;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/service/domains-access-token");
                then.header("content-type", "application/json")
                    .json_body(json!({"access_token":token(claims)}));
            })
            .await;
        let nodes = server
            .mock_async(|when, then| {
                when.method(GET).path("/api/v1/nodes");
                then.header("content-type", "application/json")
                    .json_body(json!({"nodes":[]}));
            })
            .await;
        let session = imported(&server, Arc::new(NoopStore), false);
        assert!(
            session
                .inventory_nodes(&CancellationToken::new())
                .await
                .is_err(),
            "accepted {field}"
        );
        nodes.assert_calls_async(0).await;
    }
}

struct FailingOnceStore(std::sync::Mutex<Vec<(String, String)>>);
#[async_trait::async_trait]
impl ZitadelSessionStore for FailingOnceStore {
    async fn save(&self, credentials: &ZitadelSessionCredentials) -> Result<(), auki_auth::Error> {
        let mut attempts = self.0.lock().unwrap();
        attempts.push((
            credentials.access_token().expose_secret().into(),
            credentials.refresh_token().expose_secret().into(),
        ));
        if attempts.len() == 1 {
            Err(auki_auth::Error::Persistence)
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn inventory_refresh_awaits_persistence_and_recovers_the_same_snapshot() {
    let server = MockServer::start_async().await;
    server.mock_async(|when,then| {
        when.method(GET).path("/.well-known/openid-configuration");
        then.header("content-type","application/json").json_body(json!({"issuer":server.base_url(),
            "token_endpoint":format!("{}/oauth/v2/token",server.base_url()),
            "token_endpoint_auth_methods_supported":["none"],"grant_types_supported":["refresh_token"]}));
    }).await;
    let refresh = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/oauth/v2/token")
                .body_includes("refresh_token=old-refresh");
            then.header("content-type", "application/json")
                .json_body(json!({"access_token":"replacement-access",
            "refresh_token":"replacement-refresh","expires_in":3600,"token_type":"Bearer"}));
        })
        .await;
    let exchange = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/service/domains-access-token")
                .header("authorization", "Bearer replacement-access");
            then.header("content-type", "application/json")
                .json_body(json!({"access_token":token(imported_claims())}));
        })
        .await;
    let nodes = server
        .mock_async(|when, then| {
            when.method(GET).path("/api/v1/nodes");
            then.header("content-type", "application/json")
                .json_body(json!({"nodes":[]}));
        })
        .await;
    let store = Arc::new(FailingOnceStore(std::sync::Mutex::new(vec![])));
    let session = imported(&server, store.clone(), true);
    let cancel = CancellationToken::new();
    assert!(matches!(
        session.inventory_nodes(&cancel).await,
        Err(auki_auth::Error::Persistence)
    ));
    exchange.assert_calls_async(0).await;
    nodes.assert_calls_async(0).await;
    assert!(
        session
            .inventory_nodes(&cancel)
            .await
            .unwrap()
            .unwrap()
            .is_empty()
    );
    let attempts = store.0.lock().unwrap().clone();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], attempts[1]);
    refresh.assert_calls_async(1).await;
    exchange.assert_calls_async(1).await;
    nodes.assert_calls_async(1).await;
}

#[tokio::test]
async fn renewed_busy_grant_changes_idle_coverage_to_the_successful_organization() {
    let f = Fixture::new(false).await;
    let old = f
        .session
        .domain_access(f.domain, None, &CancellationToken::new())
        .await
        .unwrap();
    f.server.reset_async().await;
    let new_org = Uuid::new_v4();
    let grant = token(json!({"iss":"dds","type":"user-access","org":new_org,
        "domain_id":f.domain,"aud":[f.server.base_url(),"dds"],"exp":chrono::Utc::now().timestamp()+3600}));
    f.server
        .mock_async(|when, then| {
            when.method(POST).path("/service/domains-access-token");
            then.header("content-type", "application/json")
                .json_body(json!({"access_token":"fixture-dds"}));
        })
        .await;
    let renewed = f
        .server
        .mock_async(|when, then| {
            when.method(POST)
                .path(format!("/api/v1/domains/{}/auth", f.domain));
            then.header("content-type", "application/json")
                .json_body(json!({"id":f.domain,
            "domain_server":{"url":f.server.base_url()},"access_token":grant}));
        })
        .await;
    let rejected = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/nodes/busy").header(
                "authorization",
                format!("Bearer {}", old.bearer().expose_secret()),
            );
            then.status(401);
        })
        .await;
    let accepted = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/nodes/busy")
                .header("authorization", format!("Bearer {grant}"));
            then.header("content-type", "application/json")
                .json_body(json!({"nodes":[]}));
        })
        .await;
    f.inventory().await;
    f.activity(None, 0).await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert_eq!(snapshot.machines[0].work_state, FleetWorkState::Unknown);
    assert_eq!(
        source(&snapshot, FleetSource::Busy).state,
        FleetSourceState::Complete
    );
    rejected.assert_calls_async(1).await;
    renewed.assert_calls_async(1).await;
    accepted.assert_calls_async(1).await;
}

#[tokio::test]
async fn oversized_inventory_preserves_robot_results_and_reports_its_limit() {
    let f = Fixture::new(false).await;
    let mut node = f.node_json();
    node["name"] = json!("x".repeat(600_000));
    f.get("/api/v1/nodes", json!({"nodes":[node]})).await;
    f.get(
        format!("/api/v1/domains/{}/robots", f.domain),
        json!({"robots":[f.robot_json()]}),
    )
    .await;
    f.activity(None, 0).await;
    f.busy(None).await;
    let snapshot = f.client().list(&FleetQuery::default()).await.unwrap();
    assert!(!snapshot.complete);
    assert_eq!(snapshot.machines.len(), 1);
    assert_eq!(
        source(&snapshot, FleetSource::Nodes).codes,
        vec!["too_large"]
    );
}

#[tokio::test]
async fn snapshot_deadline_cancels_slow_sources_and_allows_awaited_close() {
    let f = Fixture::new(false).await;
    f.inventory().await;
    f.busy(None).await;
    f.server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/jobs");
            then.delay(Duration::from_secs(5))
                .header("content-type", "application/json")
                .json_body(json!({"items":[],"next_cursor":null}));
        })
        .await;
    let fleet = AukiFleet::with_limits(
        f.session.clone(),
        &format!("{}/v1", f.server.base_url()),
        FleetLimits {
            snapshot_timeout: Duration::from_millis(100),
            ..Default::default()
        },
    )
    .unwrap()
    .in_domain(f.domain);
    assert!(matches!(
        fleet.list(&FleetQuery::default()).await,
        Err(FleetError::TimedOut)
    ));
    tokio::time::timeout(Duration::from_secs(1), fleet.close())
        .await
        .unwrap();
}

impl Fixture {
    async fn inventory_page(
        &self,
        robots: bool,
        cursor: Option<&str>,
        value: Value,
        status: u16,
    ) -> Mock<'_> {
        self.server
            .mock_async(|when, then| {
                let path = if robots {
                    format!("/api/v1/domains/{}/robots", self.domain)
                } else {
                    "/api/v1/nodes".into()
                };
                let mut when = when.method(GET).path(path).query_param("limit", "100");
                if !robots {
                    when = when
                        .query_param("org", "all")
                        .query_param("staking_status", "all");
                }
                if let Some(cursor) = cursor {
                    when.query_param("cursor", cursor);
                } else {
                    when.query_param_missing("cursor");
                }
                then.status(status)
                    .header("content-type", "application/json")
                    .json_body(value);
            })
            .await
    }
}

fn page(records: Value, robots: bool, next: &str) -> Value {
    let mut body = json!({"pagination":{"version":1,"limit":100,"next_cursor":next}});
    body[if robots { "robots" } else { "nodes" }] = records;
    body
}

#[tokio::test]
async fn paged_inventory_joins_activity_on_later_pages_and_preserves_filters() {
    let mut f = Fixture::new(false).await;
    f.node = Uuid::from_u128(2);
    f.robot = Uuid::from_u128(3);
    let mut first_node = f.node_json();
    first_node["id"] = json!(Uuid::from_u128(1));
    let mut second_robot = f.robot_json();
    second_robot["id"] = json!(Uuid::from_u128(4));
    let first = f
        .inventory_page(
            false,
            None,
            page(json!([first_node]), false, "node-next"),
            200,
        )
        .await;
    let second = f
        .inventory_page(
            false,
            Some("node-next"),
            page(json!([f.node_json()]), false, ""),
            200,
        )
        .await;
    f.inventory_page(
        true,
        None,
        page(json!([f.robot_json()]), true, "robot-next"),
        200,
    )
    .await;
    f.inventory_page(
        true,
        Some("robot-next"),
        page(json!([second_robot]), true, ""),
        200,
    )
    .await;
    f.activity(Some(f.node), 60).await;
    f.busy(Some(f.node)).await;
    let client = f.client();
    let snapshot = client.list(&FleetQuery::default()).await.unwrap();
    assert!(snapshot.complete);
    assert_eq!(snapshot.machines.len(), 3);
    assert!(snapshot.unresolved_activity.is_empty());
    assert_eq!(
        snapshot
            .machines
            .iter()
            .find(|m| m.id == f.node)
            .unwrap()
            .work_state,
        FleetWorkState::Busy
    );
    first.assert_calls_async(1).await;
    second.assert_calls_async(1).await;
    client.close().await;
}

#[tokio::test]
async fn later_inventory_denial_retains_prior_records_and_reports_partial() {
    let f = Fixture::new(false).await;
    f.inventory_page(
        false,
        None,
        page(json!([f.node_json()]), false, "next"),
        200,
    )
    .await;
    f.inventory_page(false, Some("next"), json!({}), 403).await;
    f.activity(None, 0).await;
    f.busy(None).await;
    let client = f.client();
    let snapshot = client
        .compute_pool(&ComputePoolQuery {
            mode: JobMode::Dedicated,
            capabilities: Vec::new(),
            match_all_capabilities: false,
        })
        .await
        .unwrap();
    assert!(!snapshot.complete);
    assert_eq!(snapshot.machines.len(), 1);
    let nodes = source(&snapshot, FleetSource::Nodes);
    assert_eq!(nodes.state, FleetSourceState::Partial);
    assert_eq!(nodes.http_status, Some(403));
    assert!(
        nodes
            .codes
            .iter()
            .any(|code| code == "authorization_denied")
    );
    client.close().await;
}

#[tokio::test]
async fn inventory_page_budget_is_partial_and_never_silently_truncated() {
    let f = Fixture::new(false).await;
    let mut mocks = Vec::new();
    for index in 0..20 {
        let mut node = f.node_json();
        node["id"] = json!(Uuid::from_u128(index + 1));
        let cursor = (index > 0).then(|| format!("page-{index}"));
        mocks.push(
            f.inventory_page(
                false,
                cursor.as_deref(),
                page(json!([node]), false, &format!("page-{}", index + 1)),
                200,
            )
            .await,
        );
    }
    f.activity(None, 0).await;
    f.busy(None).await;
    let client = f.client();
    let snapshot = client
        .compute_pool(&ComputePoolQuery {
            mode: JobMode::Dedicated,
            capabilities: Vec::new(),
            match_all_capabilities: false,
        })
        .await
        .unwrap();
    assert!(!snapshot.complete);
    assert_eq!(snapshot.machines.len(), 20);
    assert_eq!(
        source(&snapshot, FleetSource::Nodes).state,
        FleetSourceState::Partial
    );
    assert!(
        source(&snapshot, FleetSource::Nodes)
            .codes
            .iter()
            .any(|code| code == "inventory_page_limit")
    );
    for mock in mocks {
        mock.assert_calls_async(1).await;
    }
    client.close().await;
}

#[tokio::test]
async fn invalid_or_mixed_provider_inventory_pages_fail_closed() {
    for case in [
        "duplicate",
        "unordered",
        "repeated_cursor",
        "missing_ack",
        "wrong_limit",
        "wrong_version",
        "empty_continuation",
    ] {
        let f = Fixture::new(false).await;
        let mut first_node = f.node_json();
        first_node["id"] = json!(Uuid::from_u128(2));
        let mut second_node = f.node_json();
        second_node["id"] = json!(Uuid::from_u128(3));
        match case {
            "duplicate" => second_node["id"] = first_node["id"].clone(),
            "unordered" => second_node["id"] = json!(Uuid::from_u128(1)),
            _ => {}
        }
        f.inventory_page(false, None, page(json!([first_node]), false, "next"), 200)
            .await;
        let mut response = page(json!([second_node]), false, "");
        match case {
            "repeated_cursor" => response["pagination"]["next_cursor"] = json!("next"),
            "missing_ack" => {
                response.as_object_mut().unwrap().remove("pagination");
            }
            "wrong_limit" => response["pagination"]["limit"] = json!(99),
            "wrong_version" => response["pagination"]["version"] = json!(2),
            "empty_continuation" => {
                response["nodes"] = json!([]);
                response["pagination"]["next_cursor"] = json!("again");
            }
            _ => {}
        }
        f.inventory_page(false, Some("next"), response, 200).await;
        f.activity(None, 0).await;
        f.busy(None).await;
        let client = f.client();
        let error = client
            .compute_pool(&ComputePoolQuery {
                mode: JobMode::Dedicated,
                capabilities: Vec::new(),
                match_all_capabilities: false,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "invalid_response", "{case}");
        client.close().await;
    }
}

#[tokio::test]
async fn inventory_page_methods_validate_inputs_and_accept_empty_final_page() {
    let f = Fixture::new(false).await;
    let cancel = CancellationToken::new();
    for limit in [0, 101] {
        assert!(
            f.session
                .inventory_nodes_page(limit, None, &cancel)
                .await
                .is_err()
        );
    }
    for cursor in ["".to_string(), "a".repeat(2049)] {
        assert!(
            f.session
                .inventory_nodes_page(100, Some(&cursor), &cancel)
                .await
                .is_err()
        );
    }
    let last = f
        .inventory_page(false, Some("last"), page(json!([]), false, ""), 200)
        .await;
    let response = f
        .session
        .inventory_nodes_page(100, Some("last"), &cancel)
        .await
        .unwrap()
        .unwrap();
    assert!(response.items.is_empty());
    assert!(response.paginated);
    assert!(response.next_cursor.is_none());
    last.assert_calls_async(1).await;
}

#[tokio::test]
async fn closing_fleet_cancels_and_drains_a_later_inventory_page() {
    let f = Fixture::new(false).await;
    f.inventory_page(
        false,
        None,
        page(json!([f.node_json()]), false, "slow"),
        200,
    )
    .await;
    let slow = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/api/v1/nodes")
                .query_param("cursor", "slow");
            then.delay(Duration::from_secs(10))
                .header("content-type", "application/json")
                .json_body(page(json!([]), false, ""));
        })
        .await;
    f.activity(None, 0).await;
    f.busy(None).await;
    let client = f.client();
    let running = client.clone();
    let task = tokio::spawn(async move {
        running
            .compute_pool(&ComputePoolQuery {
                mode: JobMode::Dedicated,
                capabilities: Vec::new(),
                match_all_capabilities: false,
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while slow.calls_async().await == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), client.close())
        .await
        .unwrap();
    assert!(matches!(task.await.unwrap(), Err(FleetError::Closed)));
}
