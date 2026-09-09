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

const NODE_KEY: &str = "4c0883a69102937d6231471b5dbb6204fe5129617082798ce3f4fdf2548b6f90";

fn node_nonce() -> MockResponse {
    MockResponse::json(json!({
        "nonce": "abc12345", "domain": "dds.example.com",
        "uri": "https://dds.example.com", "version": "1", "chainId": 8453,
        "issuedAt": "2026-01-01T00:00:00Z",
    }))
}

#[tokio::test]
async fn node_registration_and_siwe_keep_the_existing_signed_contract() {
    use crate::machine::{registration, siwe};
    let server = MockServer::start(vec![
        node_nonce(),
        MockResponse::status(200),
        node_nonce(),
        MockResponse::json(json!({
            "access_token": "node-token",
            "access_expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        })),
    ])
    .await;
    let sk = registration::crypto::load_secp256k1_privhex(NODE_KEY).unwrap();
    let address = siwe::derive_eth_address(NODE_KEY).unwrap();
    assert_eq!(address, "0xfdbb6caf01414300c16ea14859fec7736d95355f");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let result = registration::register_once(
        &server.base_url,
        "1.2.3",
        "node-registration-secret",
        &sk,
        &client,
        &["/example/node/v1".into()],
    )
    .await;
    assert_eq!(
        result.kind(),
        registration::RegistrationAttemptKind::Registered
    );
    let meta = siwe::request_nonce(&server.base_url, &address)
        .await
        .unwrap();
    let message = siwe::compose_message(&meta, &address, None).unwrap();
    let expected = format!(
        "dds.example.com wants you to sign in with your Ethereum account:\n{address}\n\nURI: https://dds.example.com\nVersion: 1\nChain ID: 8453\nNonce: abc12345\nIssued At: 2026-01-01T00:00:00Z"
    );
    assert_eq!(message, expected);
    let signature = siwe::sign_message(NODE_KEY, &message).unwrap();
    assert_eq!(
        siwe::verify(&server.base_url, &address, &message, &signature)
            .await
            .unwrap()
            .token(),
        "node-token"
    );
    let requests = server.finish().await;
    assert_eq!(
        requests
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/internal/v1/auth/siwe/request",
            "/internal/v1/nodes/register-wallet",
            "/internal/v1/auth/siwe/request",
            "/internal/v1/auth/siwe/verify",
        ]
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&requests[1].body).unwrap(),
        json!({
            "message": message, "signature": signature, "registration_credentials": "node-registration-secret",
            "capabilities": ["/example/node/v1"], "version": "1.2.3",
        })
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&requests[3].body).unwrap(),
        json!({
            "address": address, "message": message, "signature": signature,
        })
    );
}

#[tokio::test]
async fn node_registration_preserves_retry_classification() {
    use crate::machine::registration::{self, RegistrationAttemptKind as Kind};
    let sk = registration::crypto::load_secp256k1_privhex(NODE_KEY).unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (status, expected) in [
        (409, Kind::Conflict),
        (503, Kind::RetryableFailure),
        (401, Kind::SlowRetryFailure),
    ] {
        let server = MockServer::start(vec![MockResponse::status(status)]).await;
        let result = registration::register_once(
            &server.base_url,
            "1.2.3",
            "synthetic-secret",
            &sk,
            &client,
            &["/example/node/v1".into()],
        )
        .await;
        assert_eq!(result.kind(), expected);
        assert_eq!(server.finish().await.len(), 1);
    }
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
