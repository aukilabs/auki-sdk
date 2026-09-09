#![cfg(all(feature = "client", not(target_arch = "wasm32")))]

use async_trait::async_trait;
use auki_auth::machine::token_manager::{TokenProvider, TokenProviderResult};
use auki_dms::{
    client::DmsClient,
    types::{CompleteTaskRequest, FailTaskRequest, HeartbeatRequest},
};
use httpmock::prelude::*;
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

#[derive(Default)]
struct RotatingProvider(AtomicUsize);

#[async_trait]
impl TokenProvider for RotatingProvider {
    async fn bearer(&self) -> TokenProviderResult<String> {
        Ok(format!("machine-{}", self.0.load(Ordering::SeqCst)))
    }

    async fn on_unauthorized(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn all_operations_replay_once_with_the_new_bearer() {
    for operation in ["lease", "heartbeat", "complete", "fail"] {
        for final_status in [200, 401] {
            let server = MockServer::start();
            let task_id = Uuid::new_v4();
            let path = if operation == "lease" {
                "/api/v1/tasks".to_owned()
            } else {
                format!("/api/v1/tasks/{task_id}/{operation}")
            };
            let method = || if operation == "lease" { GET } else { POST };
            let denied = server.mock(|when, then| {
                when.method(method())
                    .path(&path)
                    .header("authorization", "Bearer machine-0");
                then.status(401);
            });
            let replay = server.mock(|when, then| {
                when.method(method())
                    .path(&path)
                    .header("authorization", "Bearer machine-1");
                then.status(final_status).json_body(json!({
                    "task": {"id": task_id, "capability": "/example/task/v1"},
                    "access_token": "task-storage-token",
                }));
            });
            let provider = Arc::new(RotatingProvider::default());
            let client = DmsClient::new(
                format!("{}/api/v1", server.base_url()).parse().unwrap(),
                Duration::from_secs(2),
                provider.clone(),
            )
            .unwrap();
            denied.assert_calls(0);
            replay.assert_calls(0);
            let result = match operation {
                "lease" => client
                    .lease_by_capability("/example/task/v1")
                    .await
                    .map(|_| ()),
                "heartbeat" => client
                    .heartbeat(task_id, &HeartbeatRequest::default())
                    .await
                    .map(|_| ()),
                "complete" => {
                    client
                        .complete(task_id, &CompleteTaskRequest::default())
                        .await
                }
                "fail" => client.fail(task_id, &FailTaskRequest::default()).await,
                _ => unreachable!(),
            };
            assert_eq!(
                result.is_ok(),
                final_status == 200,
                "{operation}: {result:?}"
            );
            assert_eq!(provider.0.load(Ordering::SeqCst), 1);
            denied.assert_calls(1);
            replay.assert_calls(1);
        }
    }
}

#[tokio::test]
async fn empty_and_busy_leases_keep_the_existing_unfiltered_request() {
    for status in [204, 409] {
        let server = MockServer::start();
        let lease = server.mock(|when, then| {
            when.method(GET)
                .path("/tasks")
                .query_param_missing("capability");
            then.status(status);
        });
        let client = DmsClient::new(
            server.base_url().parse().unwrap(),
            Duration::from_secs(2),
            Arc::new(RotatingProvider::default()),
        )
        .unwrap();
        assert!(
            client
                .lease_by_capability("/example/ignored/v1")
                .await
                .unwrap()
                .is_none()
        );
        lease.assert_calls(1);
    }
}
