#![cfg(not(target_arch = "wasm32"))]

use async_trait::async_trait;
use auki_auth::{
    DomainAccessProvider,
    machine::token_manager::{TokenProvider, TokenProviderResult},
};
use auki_dms::{client::DmsClient, types::CompleteTaskRequest};
use auki_tasks::{AukiDmsTasks, TaskContext, TaskError, TaskHandler, TaskResult, TasksConfig};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration as ChronoDuration, Utc};
use httpmock::prelude::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Auth;
#[async_trait]
impl TokenProvider for Auth {
    async fn bearer(&self) -> TokenProviderResult<String> {
        Ok("machine-fixture".into())
    }
    async fn on_unauthorized(&self) {}
}

fn grant(server: &MockServer, task: Uuid, domain: Uuid) -> Value {
    let expires = Utc::now() + ChronoDuration::minutes(1);
    let claims = json!({"iss":"dds", "domain_id":domain, "aud":[server.base_url()], "exp":expires.timestamp()});
    let token = format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    json!({"task":{"id":task,"capability":"/example/v1"},"domain_id":domain,"domain_server_url":server.base_url(),
        "access_token":token,"access_token_expires_at":expires,"lease_expires_at":expires})
}

fn runtime(server: &MockServer) -> AukiDmsTasks {
    let client = DmsClient::new(
        server.base_url().parse().unwrap(),
        Duration::from_secs(2),
        Arc::new(Auth),
    )
    .unwrap();
    AukiDmsTasks::from_client(
        client,
        "native-fixture".into(),
        vec!["/example/v1".into()],
        TasksConfig::default(),
    )
    .unwrap()
}

#[tokio::test]
async fn custom_loop_keeps_one_lease_and_revokes_retained_data_access() {
    let server = MockServer::start();
    let task_id = Uuid::new_v4();
    let domain = Uuid::new_v4();
    let response = grant(&server, task_id, domain);
    let claim = server.mock(|when, then| {
        when.method(GET)
            .path("/tasks")
            .query_param("capability", "/example/v1");
        then.json_body(response.clone());
    });
    let heartbeat = server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/heartbeat"));
        then.json_body(response.clone());
    });
    let complete = server.mock(|when, then| {
        when.method(POST).path(format!("/tasks/{task_id}/complete"));
        then.status(200);
    });
    let runtime = runtime(&server);
    let cancel = CancellationToken::new();
    let mut lease = runtime
        .claim("/example/v1", &cancel)
        .await
        .unwrap()
        .unwrap();
    let context = lease.context();
    let token = context.access_token.clone();
    assert!(token.get().is_ok());
    assert!(matches!(
        runtime.clone().claim("/example/v1", &cancel).await,
        Err(TaskError::Busy)
    ));
    assert!(
        context
            .credential
            .domain_access(Uuid::new_v4(), None, &cancel)
            .await
            .is_err()
    );
    lease.heartbeat().await.unwrap();
    lease
        .complete(CompleteTaskRequest::default())
        .await
        .unwrap();
    assert!(context.is_cancelled());
    assert!(token.get().is_err());
    assert!(
        context
            .credential
            .domain_access(domain, None, &cancel)
            .await
            .is_err()
    );
    runtime.close().await.unwrap();
    assert!(matches!(
        runtime.claim("/example/v1", &cancel).await,
        Err(TaskError::Closed)
    ));
    claim.assert_calls(1);
    heartbeat.assert_calls(1);
    complete.assert_calls(1);
}

#[tokio::test]
async fn dropped_custom_lease_does_not_leave_a_heartbeat_owner() {
    let server = MockServer::start();
    let task_id = Uuid::new_v4();
    let domain = Uuid::new_v4();
    server.mock(|when, then| {
        when.method(GET).path("/tasks");
        then.json_body(grant(&server, task_id, domain));
    });
    let heartbeat = server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/heartbeat"));
        then.status(200);
    });
    let runtime = runtime(&server);
    let lease = runtime
        .claim("/example/v1", &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    let context = lease.context();
    drop(lease);
    assert!(context.is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), runtime.close())
        .await
        .unwrap()
        .unwrap();
    heartbeat.assert_calls(0);
}

#[tokio::test]
async fn heartbeat_cannot_replace_domain_or_task_identity() {
    for wrong_domain in [true, false] {
        let server = MockServer::start();
        let task_id = Uuid::new_v4();
        let domain = Uuid::new_v4();
        server.mock(|when, then| {
            when.method(GET).path("/tasks");
            then.json_body(grant(&server, task_id, domain));
        });
        server.mock(|when, then| {
            when.method(POST)
                .path(format!("/tasks/{task_id}/heartbeat"));
            then.json_body(if wrong_domain {
                json!({"domain_id":Uuid::new_v4()})
            } else {
                json!({"task_id":Uuid::new_v4()})
            });
        });
        let complete = server.mock(|when, then| {
            when.method(POST).path(format!("/tasks/{task_id}/complete"));
            then.status(200);
        });
        let runtime = runtime(&server);
        let mut lease = runtime
            .claim("/example/v1", &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            lease.heartbeat().await,
            Err(TaskError::Authority(_))
        ));
        assert!(lease.context().is_cancelled());
        assert!(
            lease
                .complete(CompleteTaskRequest::default())
                .await
                .is_err()
        );
        runtime.close().await.unwrap();
        complete.assert_calls(0);
    }
}

#[tokio::test]
async fn runtime_close_immediately_revokes_a_custom_loops_authority() {
    let server = MockServer::start();
    let domain = Uuid::new_v4();
    server.mock(|when, then| {
        when.method(GET).path("/tasks");
        then.json_body(grant(&server, Uuid::new_v4(), domain));
    });
    let runtime = runtime(&server);
    let lease = runtime
        .claim("/example/v1", &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    let context = lease.context();
    runtime.request_shutdown();
    assert!(context.is_cancelled());
    assert!(context.access_token.get().is_err());
    assert!(
        context
            .credential
            .domain_access(domain, None, &CancellationToken::new())
            .await
            .is_err()
    );
    drop(lease);
    runtime.close().await.unwrap();
}

struct Handler<F>(F);
#[async_trait]
impl<F, Fut> TaskHandler for Handler<F>
where
    F: Fn(TaskContext) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = auki_tasks::Result<TaskResult>> + Send,
{
    async fn run(&self, context: TaskContext) -> auki_tasks::Result<TaskResult> {
        (self.0)(context).await
    }
}

#[tokio::test]
async fn managed_handler_drains_inflight_events_before_final_events_and_failure_receipt() {
    let server = MockServer::start();
    let task_id = Uuid::new_v4();
    let response = grant(&server, task_id, Uuid::new_v4());
    server.mock(|when, then| {
        when.method(GET).path("/tasks");
        then.json_body(response.clone());
    });
    let empty = server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/heartbeat"))
            .json_body(json!({"progress":{},"events":[]}));
        then.json_body(response.clone());
    });
    let first = server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/heartbeat"))
            .json_body(json!({"progress":{},"events":[{"step":1}]}));
        then.json_body(response.clone())
            .delay(Duration::from_millis(300));
    });
    let last = server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/heartbeat"))
            .json_body(json!({"progress":{},"events":[{"step":2},{"step":3}]}));
        then.json_body(response.clone());
    });
    let details = json!({"job":{"task_id":task_id},"artifacts":[{"id":"artifact-fixture","metadata":{"partial":true}}]});
    let failure = server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/fail"))
            .json_body(json!({"reason":"input reconstruction failed","details":details}));
        then.status(200);
    });
    let complete = server.mock(|when, then| {
        when.method(POST).path(format!("/tasks/{task_id}/complete"));
        then.status(200);
    });
    let retained = Arc::new(parking_lot::Mutex::new(None));
    let handler = Handler(|task: TaskContext| {
        let first = &first;
        let details = details.clone();
        let retained = retained.clone();
        async move {
            *retained.lock() = Some(task.clone());
            task.log_event(json!({"step":1}))?;
            // Finish while the first request is held by the local server.
            tokio::time::timeout(Duration::from_secs(2), async {
                while first.calls_async().await == 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            task.log_event(json!({"step":2}))?;
            task.clone().log_event(json!({"step":3}))?;
            task.set_failure("input reconstruction failed", details)?;
            Err(TaskError::Handler)
        }
    });
    let runtime = runtime(&server);
    assert!(matches!(
        runtime
            .run_once("/example/v1", &handler, &CancellationToken::new())
            .await,
        Err(TaskError::Handler)
    ));
    empty.assert_calls(1);
    first.assert_calls(1);
    last.assert_calls(1);
    failure.assert_calls(1);
    complete.assert_calls(0);
    let task = retained.lock().take().unwrap();
    assert!(task.access_token.get().is_err());
    assert!(task.log_event(json!({"late":true})).is_err());
    assert!(task.set_failure("late", Value::Null).is_err());
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn managed_events_are_bounded_and_do_not_replace_progress_or_silently_drop_events() {
    let server = MockServer::start();
    let task_id = Uuid::new_v4();
    let response = grant(&server, task_id, Uuid::new_v4());
    server.mock(|when, then| {
        when.method(GET).path("/tasks");
        then.json_body(response.clone());
    });
    server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/heartbeat"))
            .json_body(json!({"progress":{},"events":[]}));
        then.json_body(response.clone());
    });
    let events: Vec<_> = (0..1024).map(|i| json!({"sequence":i})).collect();
    let final_batch = server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/heartbeat"))
            .json_body(json!({"progress":{"done":true},"events":events}));
        then.json_body(response.clone());
    });
    let complete = server.mock(|when, then| {
        when.method(POST).path(format!("/tasks/{task_id}/complete"));
        then.status(200);
    });
    let failure = server.mock(|when, then| {
        when.method(POST).path(format!("/tasks/{task_id}/fail"));
        then.status(200);
    });
    let handler = Handler(|task: TaskContext| async move {
        assert!(task.log_event(json!("x".repeat(64 * 1024))).is_err());
        // No await: all events are queued in one handler poll before its flush.
        for i in 0..1024 {
            task.log_event(json!({"sequence":i}))?;
        }
        assert!(task.log_event(json!({"overflow":true})).is_err());
        task.progress(json!({"done":true}))?;
        assert!(task.set_failure("", Value::Null).is_err());
        assert!(task.set_failure("x".repeat(4097), Value::Null).is_err());
        assert!(
            task.set_failure("too big", json!("x".repeat(64 * 1024)))
                .is_err()
        );
        // A receipt prepared before recovery must not turn success into failure.
        task.set_failure("recovered", json!({"artifacts":[]}))?;
        Ok(TaskResult::default())
    });
    let runtime = runtime(&server);
    runtime
        .run_once("/example/v1", &handler, &CancellationToken::new())
        .await
        .unwrap();
    final_batch.assert_calls(1);
    complete.assert_calls(1);
    failure.assert_calls(0);
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn managed_token_handle_reads_renewals_and_revokes_after_completion() {
    let server = MockServer::start();
    let task_id = Uuid::new_v4();
    let domain = Uuid::new_v4();
    let response = grant(&server, task_id, domain);
    let mut rotated = response.clone();
    let expires = Utc::now() + ChronoDuration::minutes(2);
    let claims = json!({"iss":"dds","domain_id":domain,"aud":[server.base_url()],"exp":expires.timestamp(),"generation":2});
    let token = format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    rotated["access_token"] = json!(token);
    rotated["access_token_expires_at"] = json!(expires);
    server.mock(|when, then| {
        when.method(GET).path("/tasks");
        then.json_body(response.clone());
    });
    server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/heartbeat"))
            .json_body(json!({"progress":{},"events":[]}));
        then.json_body(response.clone());
    });
    server.mock(|when, then| {
        when.method(POST)
            .path(format!("/tasks/{task_id}/heartbeat"))
            .json_body(json!({"progress":{"renew":true},"events":[]}));
        then.json_body(rotated.clone());
    });
    server.mock(|when, then| {
        when.method(POST).path(format!("/tasks/{task_id}/complete"));
        then.status(200);
    });
    let retained = Arc::new(parking_lot::Mutex::new(None));
    let handler = Handler(|task: TaskContext| {
        let retained = retained.clone();
        let token = token.clone();
        async move {
            let handle = task.access_token.clone();
            let before = handle.get()?;
            assert_eq!(format!("{handle:?}"), "TaskAccessToken([REDACTED])");
            task.progress(json!({"renew":true}))?;
            tokio::time::timeout(Duration::from_secs(2), async {
                while handle.get().unwrap().expose_secret() == before.expose_secret() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(handle.get()?.expose_secret(), token);
            *retained.lock() = Some(handle);
            Ok(TaskResult::default())
        }
    });
    let runtime = runtime(&server);
    runtime
        .run_once("/example/v1", &handler, &CancellationToken::new())
        .await
        .unwrap();
    assert!(retained.lock().as_ref().unwrap().get().is_err());
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn managed_final_flush_observes_cancellation_after_the_handler_has_returned() {
    for fail in [false, true] {
        let server = MockServer::start();
        let task_id = Uuid::new_v4();
        let response = grant(&server, task_id, Uuid::new_v4());
        server.mock(|when, then| {
            when.method(GET).path("/tasks");
            then.json_body(response.clone());
        });
        server.mock(|when, then| {
            when.method(POST)
                .path(format!("/tasks/{task_id}/heartbeat"))
                .json_body(json!({"progress":{},"events":[]}));
            then.json_body(response.clone());
        });
        let flushing = server.mock(|when, then| {
            when.method(POST)
                .path(format!("/tasks/{task_id}/heartbeat"))
                .json_body(json!({"progress":{},"events":[{"final":true}]}));
            then.json_body(response.clone())
                .delay(Duration::from_millis(300));
        });
        let complete = server.mock(|when, then| {
            when.method(POST).path(format!("/tasks/{task_id}/complete"));
            then.status(200);
        });
        let failure = server.mock(|when, then| {
            when.method(POST).path(format!("/tasks/{task_id}/fail"));
            then.status(200);
        });
        let retained = Arc::new(parking_lot::Mutex::new(None));
        let output = retained.clone();
        let handler = Handler(move |task: TaskContext| {
            *output.lock() = Some(task.access_token.clone());
            async move {
                task.log_event(json!({"final":true}))?;
                task.set_failure("must not report", json!({"artifacts":[]}))?;
                if fail {
                    Err(TaskError::Handler)
                } else {
                    Ok(TaskResult::default())
                }
            }
        });
        let runtime = runtime(&server);
        let owner = runtime.clone();
        let cancel = CancellationToken::new();
        let child = cancel.clone();
        let running =
            tokio::spawn(async move { owner.run_once("/example/v1", &handler, &child).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while flushing.calls_async().await == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
        assert!(matches!(running.await.unwrap(), Err(TaskError::Cancelled)));
        assert!(retained.lock().as_ref().unwrap().get().is_err());
        complete.assert_calls(0);
        failure.assert_calls(0);
        runtime.close().await.unwrap();
    }
}
