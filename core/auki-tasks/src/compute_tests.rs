use super::*;
use auki_auth::{SecretString, machine::registration::RegistrationAttemptKind};
use chrono::Utc;
use httpmock::prelude::*;
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const WALLET_KEY: &str = "4c0883a69102937d6231471b5dbb6204fe5129617082798ce3f4fdf2548b6f90";

fn nonce() -> serde_json::Value {
    json!({
        "nonce": "abc12345", "domain": "dds.example.com",
        "uri": "https://dds.example.com", "version": "1", "chainId": 8453,
        "issuedAt": "2026-01-01T00:00:00Z",
    })
}

fn credential(server: &MockServer, later_status: u16) -> (AukiComputeCredential, Arc<AtomicUsize>) {
    server.mock(|when, then| {
        when.method(POST).path("/internal/v1/auth/siwe/request");
        then.status(200).json_body(nonce());
    });
    server.mock(|when, then| {
        when.method(POST).path("/internal/v1/auth/siwe/verify");
        then.status(200).json_body(json!({
            "access_token": "node-token",
            "access_expires_at": (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        }));
    });
    let phase = Arc::new(AtomicUsize::new(0));
    let first = phase.clone();
    server.mock(|when, then| {
        when.method(POST)
            .path("/internal/v1/nodes/register-wallet")
            .is_true(move |_| first.load(AtomicOrdering::Acquire) == 0);
        then.status(200);
    });
    let later = phase.clone();
    server.mock(|when, then| {
        when.method(POST)
            .path("/internal/v1/nodes/register-wallet")
            .is_true(move |_| later.load(AtomicOrdering::Acquire) >= 1);
        then.status(later_status);
    });
    let mut config = ComputeConfig::new(
        &server.base_url(),
        server.base_url().as_str(),
        SecretString::new("fixture-registration"),
        SecretString::new(WALLET_KEY),
        "1.0.0",
        "compute-fixture",
    )
    .unwrap();
    config.registration_interval = Duration::from_millis(100);
    (AukiComputeCredential::new(config).unwrap(), phase)
}

#[test]
fn registration_refresh_retries_transport_failures_and_stops_on_rejection() {
    assert_eq!(
        classify_registration(RegistrationAttemptKind::RetryableFailure),
        RegistrationRefresh::Transient
    );
    assert_eq!(
        classify_registration(RegistrationAttemptKind::Registered),
        RegistrationRefresh::Registered
    );
    assert_eq!(
        classify_registration(RegistrationAttemptKind::SlowRetryFailure),
        RegistrationRefresh::Rejected
    );
    assert_eq!(
        classify_registration(RegistrationAttemptKind::Conflict),
        RegistrationRefresh::Rejected
    );
}

#[tokio::test]
async fn a_transient_registration_failure_keeps_the_compute_credential_open() {
    let server = MockServer::start();
    let (credential, phase) = credential(&server, 503);
    credential
        .start(&["/example/v1".into()], &CancellationToken::new())
        .await
        .unwrap();
    phase.store(1, AtomicOrdering::Release);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!credential.failed());
    assert!(!credential.cancellation().is_cancelled());
    credential.close().await;
}

#[tokio::test]
async fn a_rejected_registration_refresh_still_stops_the_compute_credential() {
    let server = MockServer::start();
    let (credential, phase) = credential(&server, 403);
    credential
        .start(&["/example/v1".into()], &CancellationToken::new())
        .await
        .unwrap();
    phase.store(1, AtomicOrdering::Release);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(credential.failed());
    assert!(credential.cancellation().is_cancelled());
    assert_eq!(
        credential.authentication_error().to_string(),
        "machine authentication or registration failed: registration refresh was rejected"
    );
    credential.close().await;
}
