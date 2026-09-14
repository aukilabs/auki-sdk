use super::*;
use crate::{DomainAccessProvider, DomainListQuery};
use auki_sdk::{AukiDomainData, DataListQuery};

fn page() -> MockResponse {
    MockResponse::json(json!({"domains":[],"total":0,"limit":50,"offset":0}))
}

#[tokio::test]
async fn data_and_peer_clients_share_one_login_and_refresh_owner() {
    let identity = Identity::generate();
    let domain = Uuid::new_v4();
    let server = MockServer::start_with(|base| {
        let token = format!(
            "e30.{}.fixture-signature",
            URL_SAFE_NO_PAD.encode(
                json!({
                    "iss":"dds","domain_id":domain,"aud":[base],"exp":Utc::now().timestamp()+3600
                })
                .to_string()
            )
        );
        vec![
            login_response("access-1", "refresh-1"),
            service_response("dds-1"),
            MockResponse::status(401),
            login_response("access-2", "refresh-2"),
            service_response("dds-2"),
            MockResponse::json(
                json!({"id":domain,"domain_server":{"url":base},"access_token":token}),
            ),
            MockResponse::json(json!({"data":[]})),
            challenge_response("shared-session", [0x71; 32]),
            signed_peer_response(&identity, domain, "user", Utc::now().timestamp() as u64),
            keys_response(),
            MockResponse::json(json!({"data":[]})),
        ]
    })
    .await;
    let session = auki_sdk::AuthClient::new(
        auki_sdk::AuthEnvironment::new(&server.base_url, &server.base_url).unwrap(),
    )
    .unwrap()
    .authenticate(auki_sdk::Credentials::user_password(
        "test@example.com",
        "password",
    ))
    .await
    .unwrap();
    let data = AukiDomainData::new(session.clone())
        .unwrap()
        .in_domain(domain);
    data.list(&DataListQuery::default()).await.unwrap();
    let prepared = session
        .authorize_peer(domain.into(), &identity.proof())
        .await
        .unwrap();
    let peer = AukiPeer::start(identity, prepared, AukiPeerConfig::dev().direct_only())
        .await
        .unwrap();
    data.close().await;
    assert_eq!(peer.domain_id(), domain);
    assert!(peer.status().is_ready());
    peer.shutdown().await.unwrap();
    let another = AukiDomainData::new(session.clone())
        .unwrap()
        .in_domain(domain);
    another.list(&DataListQuery::default()).await.unwrap();
    session.close().await;
    let requests = server.finish().await;
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.target == "/user/login")
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.target == "/user/refresh")
            .count(),
        1
    );
    assert_eq!(requests[7].headers["authorization"], "Bearer dds-2");
    assert_eq!(
        requests[2].headers["posemesh-client-id"],
        requests[5].headers["posemesh-client-id"]
    );
}

#[tokio::test]
async fn domain_listing_refresh_retains_replacement_after_service_exchange_failure() {
    let server = MockServer::start(vec![
        login_response("access-1", "refresh-1"),
        service_response("dds-1"),
        MockResponse::status(401),
        login_response("access-2", "refresh-2"),
        MockResponse::status(503),
        MockResponse::status(401),
        login_response("access-3", "refresh-3"),
        service_response("dds-3"),
        page(),
    ])
    .await;
    let session = client_for(&server)
        .authenticate(Credentials::user_password("test@example.com", "password"))
        .await
        .unwrap();
    assert!(matches!(
        session.list_domains(&DomainListQuery::default()).await,
        Err(Error::HttpStatus { status: 503, .. })
    ));
    session
        .list_domains(&DomainListQuery::default())
        .await
        .unwrap();
    let requests = server.finish().await;
    assert_eq!(requests[6].headers["authorization"], "Bearer refresh-2");
    assert!(requests[2].target.contains("issue_token=false"));
    assert!(requests[2].headers.contains_key("posemesh-sdk-version"));
}

#[tokio::test]
async fn app_domain_listing_refreshes_with_basic_credentials_and_gateway_policy() {
    let server = MockServer::start(vec![
        service_response("dds-1"),
        MockResponse::status(401),
        service_response("dds-2"),
        page(),
    ])
    .await;
    let credentials = AppCredentials::new("key", "secret")
        .with_gateway_mac("aa:bb:cc:dd:ee:ff")
        .unwrap();
    let session = client_for(&server)
        .authenticate(Credentials::AppCredentials(credentials))
        .await
        .unwrap();
    session
        .list_domains(&DomainListQuery::default())
        .await
        .unwrap();
    let requests = server.finish().await;
    assert_eq!(
        requests[0].headers["authorization"],
        requests[2].headers["authorization"]
    );
    assert_eq!(
        requests[3].headers["posemesh-gateway-mac"],
        "AA:BB:CC:DD:EE:FF"
    );
}

#[tokio::test]
async fn imported_sessions_reject_new_data_operations_without_refreshing_or_saving() {
    struct NeverSave;
    #[async_trait::async_trait]
    impl crate::ZitadelSessionStore for NeverSave {
        async fn save(&self, _: &crate::ZitadelSessionCredentials) -> crate::Result<()> {
            panic!("unsupported operation must not refresh");
        }
    }
    let client =
        AuthClient::new(AuthEnvironment::new("http://127.0.0.1:1", "http://127.0.0.1:1").unwrap())
            .unwrap();
    let credentials = crate::ZitadelSessionCredentials::new(
        "access",
        "refresh",
        "client",
        "http://127.0.0.1:1".parse().unwrap(),
        None,
    )
    .unwrap();
    let session = client
        .import_zitadel_session(credentials, std::sync::Arc::new(NeverSave))
        .unwrap();
    assert!(matches!(
        session.list_domains(&DomainListQuery::default()).await,
        Err(Error::InvalidConfiguration(_))
    ));
    assert!(matches!(
        session
            .domain_access(Uuid::new_v4(), None, &CancellationToken::new())
            .await,
        Err(Error::InvalidConfiguration(_))
    ));
    session.close().await;
}
