use super::{MockResponse, MockServer};
use crate::machine::{
    robot::RobotAuthenticator,
    token_manager::{
        AccessAuthenticator, SystemClock, TokenManager, TokenManagerConfig, TokenProvider,
    },
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

fn response(token: &str) -> MockResponse {
    MockResponse::json(json!({
        "robot_id": Uuid::new_v4(),
        "access_token": token,
        "access_expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
    }))
}

fn authenticator(server: &MockServer) -> RobotAuthenticator {
    RobotAuthenticator::new(
        server.base_url.parse().unwrap(),
        "test-robot-credential".into(),
        "1.2.3".into(),
        vec!["/example/robot/v1".into()],
        Duration::from_secs(5),
    )
    .unwrap()
}

#[tokio::test]
async fn robot_registration_and_concurrent_refresh_preserve_wire_contract() {
    let server = MockServer::start(vec![response("token-a"), response("token-b")]).await;
    let manager = Arc::new(TokenManager::new(
        Arc::new(authenticator(&server)),
        Arc::new(SystemClock),
        TokenManagerConfig {
            safety_ratio: 0.75,
            max_retries: 0,
            jitter: Duration::ZERO,
        },
    ));
    assert_eq!(manager.bearer().await.unwrap(), "token-a");
    manager.start_bg().await;
    manager.on_unauthorized().await;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let manager = manager.clone();
        tasks.push(tokio::spawn(async move { manager.bearer().await.unwrap() }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), "token-b");
    }
    manager.stop_bg().await;
    assert!(manager.bearer().await.is_err());
    let requests = server.finish().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].target, "/internal/v1/robots/register");
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&requests[0].body).unwrap(),
        json!({
            "registration_credentials": "test-robot-credential",
            "version": "1.2.3",
            "capabilities": ["/example/robot/v1"],
        })
    );
    assert_eq!(requests[1].target, "/internal/v1/auth/robot/verify");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&requests[1].body).unwrap(),
        json!({
            "registration_credentials": "test-robot-credential",
        })
    );
}

#[tokio::test]
async fn robot_rejection_does_not_fall_back_and_new_authenticator_registers_again() {
    let server = MockServer::start(vec![
        response("token-a"),
        MockResponse::status(401),
        response("token-c"),
    ])
    .await;
    let first = authenticator(&server);
    assert_eq!(first.login().await.unwrap().token(), "token-a");
    assert_eq!(
        first.login().await.unwrap_err().status_code(),
        Some(reqwest::StatusCode::UNAUTHORIZED)
    );
    assert_eq!(
        authenticator(&server).login().await.unwrap().token(),
        "token-c"
    );
    let requests = server.finish().await;
    assert_eq!(
        requests
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/internal/v1/robots/register",
            "/internal/v1/auth/robot/verify",
            "/internal/v1/robots/register",
        ]
    );
}
