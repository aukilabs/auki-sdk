use super::{MockResponse, MockServer};
use crate::{AuthLimits, Error, ZitadelOAuthError, ZitadelSessionCredentials, ZitadelTokenClient};
use chrono::{Duration as ChronoDuration, Utc};
use reqwest::Url;
use serde_json::{Value, json};
use std::time::Duration;

fn credentials(issuer: &str) -> ZitadelSessionCredentials {
    ZitadelSessionCredentials::new(
        "opaque.access+/=",
        "rotate+me&next=1",
        "public-client@project",
        issuer.parse().unwrap(),
        None,
    )
    .unwrap()
}

fn discovery(base: &str) -> Value {
    json!({"issuer":base, "token_endpoint":format!("{base}/oauth/v2/token"),
        "token_endpoint_auth_methods_supported":["none"], "grant_types_supported":["refresh_token"]})
}

fn token() -> Value {
    json!({"access_token":"replacement-opaque", "refresh_token":"replacement-refresh", "expires_in":3600, "token_type":"Bearer"})
}

#[test]
fn credential_contract_validates_and_redacts_without_decoding_tokens() {
    let creds = credentials("https://issuer.example");
    assert_eq!(creds.access_token().expose_secret(), "opaque.access+/=");
    assert_eq!(creds.refresh_token().expose_secret(), "rotate+me&next=1");
    assert_eq!(creds.client_id(), "public-client@project");
    assert_eq!(creds.issuer().as_str(), "https://issuer.example/");
    assert_eq!(creds.access_token_expires_at(), None);
    let diagnostic = format!(
        "{creds:?} {} {:?}",
        creds.access_token(),
        creds.refresh_token()
    );
    for secret in ["opaque.access", "rotate+me", "public-client"] {
        assert!(!diagnostic.contains(secret));
    }
    let expired = Utc::now() - ChronoDuration::hours(1);
    let creds = ZitadelSessionCredentials::new(
        "opaque",
        "refresh",
        "client",
        "https://issuer.example".parse().unwrap(),
        Some(expired),
    )
    .unwrap();
    assert_eq!(creds.access_token_expires_at(), Some(expired));
    for url in [
        "http://remote.example",
        "https://user:password@issuer.example",
        "https://issuer.example/?secret=x",
        "https://issuer.example/#secret",
        "file:///tmp/tokens",
    ] {
        let err = ZitadelSessionCredentials::new(
            "opaque",
            "refresh",
            "client",
            url.parse().unwrap(),
            None,
        )
        .unwrap_err();
        assert!(!format!("{err:?}").contains("password@"));
    }
    for url in [
        "http://127.0.0.1:8888",
        "http://[::1]:8888",
        "http://localhost:8888",
    ] {
        assert!(
            ZitadelSessionCredentials::new(
                "opaque",
                "refresh",
                "client",
                url.parse().unwrap(),
                None
            )
            .is_ok()
        );
    }
    for (access, refresh, client) in [
        ("", "rt", "client"),
        ("at", "", "client"),
        ("at", "rt", ""),
        ("at ", "rt", "client"),
        ("at", " rt", "client"),
        ("at", "rt", " client"),
    ] {
        assert!(
            ZitadelSessionCredentials::new(
                access,
                refresh,
                client,
                "https://issuer.example".parse().unwrap(),
                None
            )
            .is_err()
        );
    }
    assert!(
        ZitadelSessionCredentials::new(
            "x".repeat(65537),
            "rt",
            "client",
            "https://issuer.example".parse().unwrap(),
            None
        )
        .is_err()
    );
    assert!(
        ZitadelSessionCredentials::new(
            "at",
            "rt",
            "界".repeat(86),
            "https://issuer.example".parse().unwrap(),
            None
        )
        .is_err()
    );
}

#[tokio::test]
async fn redirects_never_forward_credentials_or_follow_discovery() {
    for during_refresh in [false, true] {
        let trap = MockServer::start(vec![MockResponse::json(token())]).await;
        let server = MockServer::start_with(|base| {
            let redirect = MockResponse {
                location: Some(format!("{}/redirect-target", trap.base_url)),
                ..MockResponse::status(307)
            };
            if during_refresh {
                vec![MockResponse::json(discovery(base)), redirect]
            } else {
                vec![redirect]
            }
        })
        .await;
        let creds = credentials(&server.base_url);
        let client = ZitadelTokenClient::new(
            creds.issuer().clone(),
            creds.client_id(),
            AuthLimits::default(),
        )
        .unwrap();
        assert!(client.refresh(&creds).await.is_err());
        assert_eq!(
            server.finish().await.len(),
            if during_refresh { 2 } else { 1 }
        );
        assert!(
            trap.requests.lock().await.is_empty(),
            "redirect target must receive no request"
        );
        trap.task.abort();
        assert!(trap.task.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn streaming_size_bound_and_json_content_type_are_enforced() {
    for during_refresh in [false, true] {
        for oversized in [false, true] {
            let server = MockServer::start_with(|base| {
                let response = if oversized {
                    MockResponse {
                        omit_content_length: true,
                        ..MockResponse::json(json!({"oversized":"x".repeat(4096)}))
                    }
                } else {
                    MockResponse {
                        content_type: "text/html",
                        ..MockResponse::json(token())
                    }
                };
                if during_refresh {
                    vec![MockResponse::json(discovery(base)), response]
                } else {
                    vec![response]
                }
            })
            .await;
            let creds = credentials(&server.base_url);
            let client = ZitadelTokenClient::new(
                creds.issuer().clone(),
                creds.client_id(),
                AuthLimits {
                    max_response_bytes: 1024,
                    ..AuthLimits::default()
                },
            )
            .unwrap();
            let error = client.refresh(&creds).await.unwrap_err();
            if during_refresh {
                assert!(matches!(error, Error::RefreshOutcomeUnknown));
            } else if oversized {
                assert!(matches!(error, Error::ResponseTooLarge { .. }));
            } else {
                assert!(matches!(error, Error::InvalidResponse { .. }));
            }
            assert_eq!(
                server.finish().await.len(),
                if during_refresh { 2 } else { 1 }
            );
        }
    }
}

#[tokio::test]
async fn public_client_refresh_rotates_or_retains_refresh_and_omits_scope() {
    for rotate in [true, false] {
        let server = MockServer::start_with(|base| {
            let mut response = token();
            if !rotate {
                response.as_object_mut().unwrap().remove("refresh_token");
            }
            vec![
                MockResponse::json(discovery(base)),
                MockResponse::json(response),
            ]
        })
        .await;
        let creds = credentials(&server.base_url);
        let client = ZitadelTokenClient::new(
            creds.issuer().clone(),
            creds.client_id(),
            AuthLimits::default(),
        )
        .unwrap();
        let before = Utc::now();
        let replacement = client.refresh(&creds).await.unwrap();
        assert_eq!(
            replacement.access_token().expose_secret(),
            "replacement-opaque"
        );
        assert_eq!(
            replacement.refresh_token().expose_secret(),
            if rotate {
                "replacement-refresh"
            } else {
                "rotate+me&next=1"
            }
        );
        assert!(
            replacement.access_token_expires_at().unwrap() >= before + ChronoDuration::hours(1)
        );
        assert!(
            replacement.access_token_expires_at().unwrap() <= Utc::now() + ChronoDuration::hours(1)
        );
        assert_eq!(replacement.issuer(), creds.issuer());
        assert_eq!(replacement.client_id(), creds.client_id());
        let requests = server.finish().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].target, "/.well-known/openid-configuration");
        assert_eq!(requests[0].method, "GET");
        assert!(requests[0].body.is_empty());
        assert!(!requests[0].headers.contains_key("authorization"));
        assert_eq!(requests[1].method, "POST");
        assert_eq!(requests[1].target, "/oauth/v2/token");
        assert!(
            requests[1].headers["content-type"].starts_with("application/x-www-form-urlencoded")
        );
        assert!(!requests[1].headers.contains_key("authorization"));
        // Use the URL parser to decode the form, including opaque +/&/= bytes.
        let mut form_url = Url::parse("https://unused.example/").unwrap();
        form_url.set_query(Some(std::str::from_utf8(&requests[1].body).unwrap()));
        let fields: std::collections::HashMap<_, _> = form_url.query_pairs().into_owned().collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields["grant_type"], "refresh_token");
        assert_eq!(fields["refresh_token"], "rotate+me&next=1");
        assert_eq!(fields["client_id"], "public-client@project");
    }
}

#[tokio::test]
async fn unsafe_discovery_never_submits_a_refresh() {
    for mutation in 0..7 {
        let server = MockServer::start_with(|base| {
            let mut metadata = discovery(base);
            match mutation {
                0 => metadata["issuer"] = json!("https://other.example"),
                1 => metadata["token_endpoint"] = json!("https://other.example/token"),
                2 => metadata["token_endpoint"] = json!(format!("{base}/token?leak=yes")),
                3 => metadata["token_endpoint"] = json!("http://remote.example/token"),
                4 => {
                    metadata["token_endpoint_auth_methods_supported"] =
                        json!(["client_secret_basic"])
                }
                5 => metadata["grant_types_supported"] = json!(["authorization_code"]),
                _ => {
                    metadata.as_object_mut().unwrap().remove("token_endpoint");
                }
            }
            vec![MockResponse::json(metadata)]
        })
        .await;
        let creds = credentials(&server.base_url);
        let client = ZitadelTokenClient::new(
            creds.issuer().clone(),
            creds.client_id(),
            AuthLimits::default(),
        )
        .unwrap();
        assert!(client.refresh(&creds).await.is_err());
        assert_eq!(server.finish().await.len(), 1);
    }
    let creds = credentials("https://issuer.example");
    let client = ZitadelTokenClient::new(
        creds.issuer().clone(),
        "wrong-client",
        AuthLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        client.refresh(&creds).await,
        Err(Error::InvalidConfiguration(_))
    ));
}

#[tokio::test]
async fn oauth_errors_are_typed_and_never_echo_provider_secrets() {
    for (code, expected) in [
        ("invalid_grant", ZitadelOAuthError::InvalidGrant),
        ("invalid_client", ZitadelOAuthError::InvalidClient),
        ("unauthorized_client", ZitadelOAuthError::UnauthorizedClient),
        ("invalid_request", ZitadelOAuthError::InvalidRequest),
        ("invalid_scope", ZitadelOAuthError::InvalidScope),
        ("server_error", ZitadelOAuthError::ServerError),
        (
            "temporarily_unavailable",
            ZitadelOAuthError::TemporarilyUnavailable,
        ),
        ("secret-unrecognized-code", ZitadelOAuthError::Other),
    ] {
        let server = MockServer::start_with(|base| {
            let mut failure = MockResponse::json(
                json!({"error":code, "error_description":"secret-provider-description"}),
            );
            failure.status = 400;
            vec![MockResponse::json(discovery(base)), failure]
        })
        .await;
        let creds = credentials(&server.base_url);
        let client = ZitadelTokenClient::new(
            creds.issuer().clone(),
            creds.client_id(),
            AuthLimits::default(),
        )
        .unwrap();
        let error = client.refresh(&creds).await.unwrap_err();
        assert!(matches!(error, Error::ZitadelOAuth(actual) if actual == expected));
        assert!(!format!("{error:?} {error}").contains("secret"));
        assert_eq!(server.finish().await.len(), 2);
    }
}

#[tokio::test]
async fn malformed_refresh_success_is_an_ambiguous_rotation_not_a_replayable_failure() {
    for mutation in 0..9 {
        let server = MockServer::start_with(|base| {
            let mut response = token();
            match mutation {
                0 => response["access_token"] = json!(""),
                1 => response["refresh_token"] = json!(""),
                2 => response["refresh_token"] = Value::Null,
                3 => response["expires_in"] = json!(0),
                4 => response["expires_in"] = json!(-1),
                5 => response["expires_in"] = json!("3600"),
                6 => response["expires_in"] = json!(u64::MAX),
                7 => response["token_type"] = json!("MAC"),
                _ => {
                    response.as_object_mut().unwrap().remove("expires_in");
                }
            }
            vec![
                MockResponse::json(discovery(base)),
                MockResponse::json(response),
            ]
        })
        .await;
        let creds = credentials(&server.base_url);
        let client = ZitadelTokenClient::new(
            creds.issuer().clone(),
            creds.client_id(),
            AuthLimits::default(),
        )
        .unwrap();
        assert!(matches!(
            client.refresh(&creds).await,
            Err(Error::RefreshOutcomeUnknown)
        ));
        assert_eq!(server.finish().await.len(), 2);
    }
}

#[tokio::test]
async fn response_size_and_deadlines_are_bounded_before_and_after_submission() {
    for during_refresh in [false, true] {
        for oversized in [false, true] {
            let server = MockServer::start_with(|base| {
                let response = if oversized {
                    MockResponse::json(json!({"oversized":"x".repeat(2048)}))
                } else {
                    MockResponse::json(token()).delayed(Duration::from_millis(100))
                };
                if during_refresh {
                    vec![MockResponse::json(discovery(base)), response]
                } else {
                    vec![response]
                }
            })
            .await;
            let creds = credentials(&server.base_url);
            let limits = AuthLimits {
                connect_timeout: Duration::from_millis(20),
                request_timeout: Duration::from_millis(40),
                max_response_bytes: 1024,
            };
            let client =
                ZitadelTokenClient::new(creds.issuer().clone(), creds.client_id(), limits).unwrap();
            let error = client.refresh(&creds).await.unwrap_err();
            if during_refresh {
                assert!(matches!(error, Error::RefreshOutcomeUnknown));
            } else if oversized {
                assert!(matches!(error, Error::ResponseTooLarge { .. }));
            } else {
                assert!(matches!(error, Error::RequestTimedOut { .. }));
            }
            let requests = server.finish().await;
            assert_eq!(requests.len(), if during_refresh { 2 } else { 1 });
        }
    }
}
