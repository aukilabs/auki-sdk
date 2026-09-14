#![cfg(not(target_arch = "wasm32"))]

use async_trait::async_trait;
use auki_auth::{
    DomainAccessProvider,
    machine::token_manager::{TokenProvider, TokenProviderResult},
};
use auki_dms::{client::DmsClient, types::CompleteTaskRequest};
use auki_tasks::{AukiDmsTasks, TaskError, TasksConfig};
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
