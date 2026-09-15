//! Imported-session HTTP operations share the existing owned refresh/save slot.
use super::*;
use crate::{DomainAccessProvider, DomainListQuery};
use auki_sdk::{AukiDomainData, DataListQuery, DataWrite};

// The facade's dependency uses the non-test auth crate; import its matching
// session types for end-to-end HTTP client tests.
fn import_sdk(server: &MockServer) -> auki_sdk::AuthSession {
    struct Store;
    #[async_trait::async_trait]
    impl auki_sdk::ZitadelSessionStore for Store {
        async fn save(
            &self,
            _: &auki_sdk::ZitadelSessionCredentials,
        ) -> Result<(), auki_sdk::AuthError> {
            panic!("unexpired fixture must not rotate credentials");
        }
    }
    let creds = auki_sdk::ZitadelSessionCredentials::new(
        "opaque.access+/=",
        "refresh",
        "client",
        server.base_url.parse().unwrap(),
        None,
    )
    .unwrap();
    auki_sdk::AuthClient::new(
        auki_sdk::AuthEnvironment::new(&server.base_url, &server.base_url).unwrap(),
    )
    .unwrap()
    .import_zitadel_session(creds, Arc::new(Store))
    .unwrap()
}

fn grant(base: &str, domain: Uuid) -> MockResponse {
    grant_with_claims(
        base,
        domain,
        json!({
            "iss": "dds", "domain_id": domain, "aud": [base],
            "exp": Utc::now().timestamp() + 3600,
        }),
    )
}

fn grant_with_claims(base: &str, domain: Uuid, claims: Value) -> MockResponse {
    MockResponse::json(json!({
        "id": domain, "domain_server": {"url": base},
        "access_token": format!("e30.{}.fixture", URL_SAFE_NO_PAD.encode(claims.to_string())),
    }))
}

fn listing_service_response(domains: &[Uuid]) -> MockResponse {
    listing_service_response_with_claims(json!({
        "type": "user-p2p-access",
        "iss": "api",
        "aud": ["domain-service"],
        "sub": "opaque-zitadel-user",
        "org": Uuid::from_u128(0xaaaa),
        "domains": domains,
        "iat": Utc::now().timestamp(),
        "exp": Utc::now().timestamp() + 3600,
    }))
}

fn listing_service_response_with_claims(claims: Value) -> MockResponse {
    service_response(&format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(claims.to_string())
    ))
}

#[tokio::test]
async fn imported_listing_rejects_broad_filters_and_portal_lookup_without_io() {
    let server = MockServer::start(vec![]).await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), false);
    let query = DomainListQuery {
        organization: "all".into(),
        domain_server_id: Some(Uuid::from_u128(1)),
        ..Default::default()
    };
    assert!(matches!(
        session.list_domains(&query).await,
        Err(Error::InvalidConfiguration(_))
    ));
    let query = DomainListQuery {
        organization: Uuid::from_u128(2).to_string(),
        ..Default::default()
    };
    assert!(matches!(
        session.list_domains(&query).await,
        Err(Error::InvalidConfiguration(_))
    ));
    assert!(matches!(
        session
            .domains_for_portal(
                &crate::PortalId::parse("ABC12345678").unwrap(),
                "own",
                &CancellationToken::new(),
            )
            .await,
        Err(Error::InvalidConfiguration(_))
    ));
    assert!(server.requests.lock().await.is_empty());
    assert!(store.attempts.lock().unwrap().is_empty());
    session.close().await;
    assert!(matches!(
        session.list_domains(&DomainListQuery::default()).await,
        Err(Error::SessionClosed)
    ));
    server.finish().await;
}

#[tokio::test]
async fn imported_listing_uses_peer_profile_and_preserves_server_pages() {
    let server = MockServer::start(vec![
        listing_service_response(&[DOMAIN]),
        domains_page_response(&[DOMAIN], Uuid::from_u128(0xaaaa), 1, 1, 0),
        listing_service_response(&[DOMAIN]),
        domains_response(DOMAIN, Uuid::from_u128(0xaaaa)),
    ])
    .await;
    let session = import(&server, Store::new(false, 0), false);
    let query = DomainListQuery {
        limit: 1,
        ..Default::default()
    };
    let page = session.list_domains(&query).await.unwrap();
    assert_eq!(page.domains.len(), 1);
    assert_eq!(page.domains[0].id, DOMAIN);
    assert_eq!((page.total, page.limit, page.offset), (1, 1, 0));
    assert_eq!(session.accessible_domains().await.unwrap().len(), 1);
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(
        requests[0].target,
        "/service/domains-access-token?purpose=p2p"
    );
    assert_eq!(
        requests[1].target,
        "/api/v1/accessible-domains?limit=1&offset=0"
    );
    assert!(requests[1].headers["authorization"].ends_with(".fixture"));
}

#[tokio::test]
async fn imported_listing_accepts_an_empty_page_beyond_the_total() {
    let server = MockServer::start(vec![
        listing_service_response(&[DOMAIN]),
        domains_page_response(&[], Uuid::from_u128(0xaaaa), 1, 10, 50),
    ])
    .await;
    let session = import(&server, Store::new(false, 0), false);
    let page = session
        .list_domains(&DomainListQuery {
            offset: 50,
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(page.domains.is_empty());
    assert_eq!((page.total, page.limit, page.offset), (1, 10, 50));
    session.close().await;
    assert_eq!(server.finish().await.len(), 2);
}

#[tokio::test]
async fn imported_accessible_domains_accumulates_server_pagination() {
    let domains: Vec<_> = (1..=101).map(Uuid::from_u128).collect();
    let server = MockServer::start(vec![
        listing_service_response(&domains),
        domains_page_response(&domains[..100], Uuid::from_u128(0xaaaa), 101, 100, 0),
        domains_page_response(&domains[100..], Uuid::from_u128(0xaaaa), 101, 100, 100),
    ])
    .await;
    let session = import(&server, Store::new(false, 0), false);
    let choices = session.accessible_domains().await.unwrap();
    assert_eq!(choices.len(), 101);
    assert_eq!(choices[100].domain.id, domains[100]);
    session.close().await;
    let requests = server.finish().await;
    assert!(requests[2].target.ends_with("limit=100&offset=100"));
}

#[tokio::test]
async fn imported_listing_rejects_legacy_and_invalid_peer_profiles_before_dds() {
    let base = json!({
        "type": "user-p2p-access", "iss": "api", "aud": ["domain-service"],
        "sub": "opaque-user", "org": Uuid::from_u128(0xaaaa), "domains": [DOMAIN],
        "iat": Utc::now().timestamp(), "exp": Utc::now().timestamp() + 3600,
    });
    for (field, value, configuration) in [
        ("type", json!("user-access"), true),
        ("iss", json!("other"), false),
        ("aud", json!(["other"]), false),
        ("sub", json!(""), false),
        ("org", json!("not-a-uuid"), false),
        ("iat", json!(Utc::now().timestamp() + 7200), false),
        ("exp", json!(1), false),
        ("domains", json!([]), false),
        ("domains", json!([DOMAIN, DOMAIN]), false),
    ] {
        let mut claims = base.clone();
        claims[field] = value;
        let server = MockServer::start(vec![listing_service_response_with_claims(claims)]).await;
        let session = import(&server, Store::new(false, 0), false);
        let error = session
            .list_domains(&DomainListQuery::default())
            .await
            .unwrap_err();
        assert_eq!(
            matches!(error, Error::InvalidConfiguration(_)),
            configuration
        );
        assert!(configuration || matches!(error, Error::InvalidResponse { .. }));
        session.close().await;
        assert_eq!(server.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn imported_listing_rejects_dds_domains_outside_service_allowlist() {
    let other = Uuid::from_u128(2);
    let server = MockServer::start(vec![
        listing_service_response(&[DOMAIN]),
        domains_response(other, Uuid::from_u128(0xaaaa)),
    ])
    .await;
    let session = import(&server, Store::new(false, 0), false);
    assert!(matches!(
        session.accessible_domains().await,
        Err(Error::InvalidResponse { .. })
    ));
    session.close().await;
    assert_eq!(server.finish().await.len(), 2);
}

#[tokio::test]
async fn imported_listing_preserves_api_forbidden_and_reexchanges_after_dds_unauthorized() {
    let denied = MockServer::start(vec![MockResponse::status(403)]).await;
    let session = import(&denied, Store::new(false, 0), false);
    assert!(matches!(
        session.accessible_domains().await,
        Err(Error::HttpStatus { status: 403, .. })
    ));
    session.close().await;
    assert_eq!(denied.finish().await.len(), 1);

    let denied = MockServer::start(vec![
        listing_service_response(&[DOMAIN]),
        MockResponse::status(403),
    ])
    .await;
    let session = import(&denied, Store::new(false, 0), false);
    assert!(matches!(
        session.accessible_domains().await,
        Err(Error::HttpStatus { status: 403, .. })
    ));
    session.close().await;
    assert_eq!(denied.finish().await.len(), 2);

    let server = MockServer::start(vec![
        listing_service_response(&[DOMAIN]),
        MockResponse::status(401),
        listing_service_response(&[DOMAIN]),
        domains_response(DOMAIN, Uuid::from_u128(0xaaaa)),
    ])
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), false);
    assert_eq!(session.accessible_domains().await.unwrap().len(), 1);
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(
        requests[0].headers["authorization"],
        requests[2].headers["authorization"]
    );
    assert!(store.attempts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn imported_listing_retries_a_failed_save_before_exchange() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            listing_service_response(&[DOMAIN]),
            domains_response(DOMAIN, Uuid::from_u128(0xaaaa)),
        ]
    })
    .await;
    let store = Store::new(false, 1);
    let session = import(&server, store.clone(), true);
    assert!(matches!(
        session.accessible_domains().await,
        Err(Error::Persistence)
    ));
    assert_eq!(server.requests.lock().await.len(), 2);
    assert_eq!(session.accessible_domains().await.unwrap().len(), 1);
    let attempts = store.attempts.lock().unwrap().clone();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], attempts[1]);
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(
        requests[2].headers["authorization"],
        "Bearer replacement-opaque"
    );
}

#[tokio::test]
async fn imported_listing_cancellation_stops_the_exchange_without_dds_io() {
    let server = MockServer::start(vec![
        listing_service_response(&[DOMAIN]).delayed(Duration::from_millis(100)),
    ])
    .await;
    let session = import(&server, Store::new(false, 0), false);
    let cancellation = CancellationToken::new();
    let operation_session = session.clone();
    let operation_cancellation = cancellation.clone();
    let operation = tokio::spawn(async move {
        operation_session
            .accessible_domains_with_cancellation(&operation_cancellation)
            .await
    });
    wait_requests(&server, 1).await;
    cancellation.cancel();
    assert!(matches!(
        operation.await.unwrap(),
        Err(Error::Cancelled { .. })
    ));
    session.close().await;
    assert_eq!(server.finish().await.len(), 1);
}

#[tokio::test]
async fn concurrent_imported_lists_share_one_api_refresh_and_save() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::status(401),
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            listing_service_response(&[DOMAIN]),
            domains_response(DOMAIN, Uuid::from_u128(0xaaaa)),
            listing_service_response(&[DOMAIN]),
            domains_response(DOMAIN, Uuid::from_u128(0xaaaa)),
        ]
    })
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), false);
    let (first, second) = tokio::join!(session.accessible_domains(), session.accessible_domains());
    assert_eq!(first.unwrap().len(), 1);
    assert_eq!(second.unwrap().len(), 1);
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.target == "/oauth/v2/token")
            .count(),
        1
    );
    assert_eq!(store.attempts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn imported_known_domain_data_and_peer_keep_their_credentials_separate() {
    let server = MockServer::start_with(|base| {
        let mut responses = vec![
            service_response("data-service"),
            grant(base, DOMAIN),
            MockResponse::json(json!({"data": []})),
        ];
        responses.extend(admitted());
        responses.push(MockResponse::json(json!({"data": []})));
        responses
    })
    .await;
    let session = import_sdk(&server);
    let data = AukiDomainData::new(session.clone())
        .unwrap()
        .in_domain(DOMAIN);
    data.list(&DataListQuery::default()).await.unwrap();
    session
        .authorize_peer(DOMAIN.into(), &test_identity().proof())
        .await
        .unwrap();
    data.close().await;
    let other = AukiDomainData::new(session.clone())
        .unwrap()
        .in_domain(DOMAIN);
    other.list(&DataListQuery::default()).await.unwrap();
    other.close().await;
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(requests[0].target, "/service/domains-access-token");
    assert_eq!(requests[1].headers["authorization"], "Bearer data-service");
    assert_eq!(
        requests[3].headers["authorization"],
        requests[0].headers["authorization"]
    );
    assert_ne!(
        requests[2].headers["authorization"],
        requests[1].headers["authorization"]
    );
    assert!(requests[2].headers["authorization"].ends_with(".fixture"));
    assert_eq!(
        requests[6].headers["authorization"],
        requests[2].headers["authorization"]
    );
}

#[tokio::test]
async fn imported_data_rejects_wrong_domain_audience_issuer_and_expired_grants() {
    for (field, value) in [
        ("domain_id", json!(Uuid::from_u128(2))),
        ("aud", json!(["https://other.example/"])),
        ("iss", json!("api")),
        ("exp", json!(1)),
    ] {
        let server = MockServer::start_with(|base| {
            let mut claims = json!({
                "iss": "dds", "domain_id": DOMAIN, "aud": [base],
                "exp": Utc::now().timestamp() + 3600,
            });
            claims[field] = value;
            vec![
                service_response("data-service"),
                grant_with_claims(base, DOMAIN, claims),
            ]
        })
        .await;
        let session = import(&server, Store::new(false, 0), false);
        assert!(matches!(
            session
                .domain_access(DOMAIN, None, &CancellationToken::new())
                .await,
            Err(Error::InvalidResponse { .. })
        ));
        session.close().await;
        assert_eq!(server.finish().await.len(), 2);
    }
}

#[tokio::test]
async fn imported_viewer_reads_do_not_authorize_writes_deletes_or_another_domain() {
    let server = MockServer::start_with(|base| {
        vec![
            service_response("viewer-service"),
            grant(base, DOMAIN),
            MockResponse::json(json!({"data": []})),
            MockResponse::status(403),
            MockResponse::status(403),
            service_response("viewer-service"),
            MockResponse::status(403),
        ]
    })
    .await;
    let session = import_sdk(&server);
    let factory = AukiDomainData::new(session.clone()).unwrap();
    let data = factory.in_domain(DOMAIN);
    data.list(&DataListQuery::default()).await.unwrap();
    let error = data
        .write(
            DataWrite::Named {
                name: "report",
                data_type: "test.report.v1",
            },
            b"bytes",
        )
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(403));
    assert_eq!(
        data.delete(Uuid::from_u128(3)).await.unwrap_err().status(),
        Some(403)
    );
    let other = factory.in_domain(Uuid::from_u128(4));
    assert_eq!(
        other
            .list(&DataListQuery::default())
            .await
            .unwrap_err()
            .status(),
        Some(403)
    );
    data.close().await;
    other.close().await;
    session.close().await;
    assert_eq!(server.finish().await.len(), 7);
}

#[tokio::test]
async fn imported_data_persistence_failure_retains_rotated_credentials_for_retry() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            service_response("data-service"),
            grant(base, DOMAIN),
        ]
    })
    .await;
    let store = Store::new(false, 1);
    let session = import(&server, store.clone(), true);
    let cancellation = CancellationToken::new();
    assert!(matches!(
        session.domain_access(DOMAIN, None, &cancellation).await,
        Err(Error::Persistence)
    ));
    assert_eq!(server.requests.lock().await.len(), 2);
    session
        .domain_access(DOMAIN, None, &cancellation)
        .await
        .unwrap();
    let attempts = store.attempts.lock().unwrap().clone();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], attempts[1]);
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(
        requests[2].headers["authorization"],
        "Bearer replacement-opaque"
    );
}

#[tokio::test]
async fn imported_dds_unauthorized_reexchanges_without_rotating_the_login() {
    let server = MockServer::start_with(|base| {
        vec![
            service_response("service-1"),
            MockResponse::status(401),
            service_response("service-2"),
            grant(base, DOMAIN),
        ]
    })
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), false);
    session
        .domain_access(DOMAIN, None, &CancellationToken::new())
        .await
        .unwrap();
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(
        requests[0].headers["authorization"],
        requests[2].headers["authorization"]
    );
    assert_eq!(requests[3].headers["authorization"], "Bearer service-2");
    assert!(store.attempts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn imported_data_api_unauthorized_refreshes_once_and_awaits_persistence() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::status(401),
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            service_response("data-service"),
            grant(base, DOMAIN),
        ]
    })
    .await;
    let store = Store::new(true, 0);
    let session = import(&server, store.clone(), false);
    let operation_session = session.clone();
    let operation = tokio::spawn(async move {
        operation_session
            .domain_access(DOMAIN, None, &CancellationToken::new())
            .await
    });
    store.wait_for_save().await;
    assert_eq!(server.requests.lock().await.len(), 3);
    assert!(!operation.is_finished());
    store.allow_save();
    operation.await.unwrap().unwrap();
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(
        requests[3].headers["authorization"],
        "Bearer replacement-opaque"
    );
    assert_eq!(store.attempts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn imported_data_close_drains_a_save_after_the_caller_cancels() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
        ]
    })
    .await;
    let store = Store::new(true, 0);
    let session = import(&server, store.clone(), true);
    let cancellation = CancellationToken::new();
    let operation_session = session.clone();
    let operation_cancel = cancellation.clone();
    let operation = tokio::spawn(async move {
        operation_session
            .domain_access(DOMAIN, None, &operation_cancel)
            .await
    });
    store.wait_for_save().await;
    cancellation.cancel();
    assert!(matches!(
        operation.await.unwrap(),
        Err(Error::Cancelled { .. })
    ));
    let closing_session = session.clone();
    let closing = tokio::spawn(async move { closing_session.close().await });
    tokio::task::yield_now().await;
    assert!(!closing.is_finished());
    store.allow_save();
    closing.await.unwrap();
    assert!(store.durable.lock().unwrap().is_some());
    assert!(matches!(
        session
            .domain_access(DOMAIN, None, &CancellationToken::new())
            .await,
        Err(Error::SessionClosed)
    ));
    assert_eq!(server.finish().await.len(), 2);
}

#[tokio::test]
async fn imported_data_and_peer_concurrent_start_share_one_refresh_and_save() {
    let server = MockServer::start_with(|base| {
        let mut responses = vec![
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            service_response("data-service"),
            grant(base, DOMAIN),
        ];
        responses.extend(admitted());
        responses
    })
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), true);
    let cancellation = CancellationToken::new();
    let (data, peer) = tokio::join!(
        session.domain_access(DOMAIN, None, &cancellation),
        authorize(&session),
    );
    assert_eq!(data.unwrap().domain_id(), peer.unwrap().domain.id);
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.target == "/oauth/v2/token")
            .count(),
        1
    );
    assert_eq!(store.attempts.lock().unwrap().len(), 1);
    assert_eq!(
        requests[4].headers["authorization"],
        "Bearer replacement-opaque"
    );
}

#[tokio::test]
async fn imported_cached_grant_settles_a_pending_save_from_another_domain() {
    let server = MockServer::start_with(|base| {
        vec![
            service_response("data-service"),
            grant(base, DOMAIN),
            MockResponse::status(401),
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
        ]
    })
    .await;
    let store = Store::new(false, 1);
    let session = import(&server, store.clone(), false);
    let cancellation = CancellationToken::new();
    let original = session
        .domain_access(DOMAIN, None, &cancellation)
        .await
        .unwrap();
    assert!(matches!(
        session
            .domain_access(Uuid::from_u128(42), None, &cancellation)
            .await,
        Err(Error::Persistence)
    ));
    let retained = session
        .domain_access(DOMAIN, None, &cancellation)
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&original, &retained));
    let attempts = store.attempts.lock().unwrap().clone();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], attempts[1]);
    session.close().await;
    assert_eq!(server.finish().await.len(), 5);
}

#[tokio::test]
async fn imported_data_repeated_api_rejection_requires_login_without_retrying_peers() {
    let server = MockServer::start_with(|base| {
        vec![
            MockResponse::status(401),
            MockResponse::json(discovery(base)),
            MockResponse::json(token()),
            MockResponse::status(401),
        ]
    })
    .await;
    let store = Store::new(false, 0);
    let session = import(&server, store.clone(), false);
    assert!(matches!(
        session
            .domain_access(DOMAIN, None, &CancellationToken::new())
            .await,
        Err(Error::AuthenticationRequired)
    ));
    assert!(matches!(
        authorize(&session).await,
        Err(Error::AuthenticationRequired)
    ));
    session.close().await;
    assert_eq!(server.finish().await.len(), 4);
    assert_eq!(store.attempts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn imported_data_cancels_api_and_dds_requests_without_retry() {
    for cancel_at_dds in [false, true] {
        let server = MockServer::start_with(|base| {
            if cancel_at_dds {
                vec![
                    service_response("data-service"),
                    grant(base, DOMAIN).delayed(Duration::from_millis(100)),
                ]
            } else {
                vec![service_response("data-service").delayed(Duration::from_millis(100))]
            }
        })
        .await;
        let session = import(&server, Store::new(false, 0), false);
        let operation_session = session.clone();
        let cancellation = CancellationToken::new();
        let operation_cancel = cancellation.clone();
        let operation = tokio::spawn(async move {
            operation_session
                .domain_access(DOMAIN, None, &operation_cancel)
                .await
        });
        let expected = if cancel_at_dds { 2 } else { 1 };
        wait_requests(&server, expected).await;
        cancellation.cancel();
        assert!(matches!(
            operation.await.unwrap(),
            Err(Error::Cancelled { .. })
        ));
        session.close().await;
        assert_eq!(server.finish().await.len(), expected);
    }
}
