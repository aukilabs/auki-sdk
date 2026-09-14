#![cfg(not(target_arch = "wasm32"))]

use auki_auth::{AuthClient, AuthEnvironment, AuthSession, Credentials, DomainAccessProvider};
use auki_domain_client::{
    AukiDomainData, AukiDomains, DataError, DataLimits, DataListQuery, DataWrite, DomainListQuery,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use httpmock::{
    Method::{DELETE, GET, POST, PUT},
    Mock, MockServer,
};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Fixture {
    server: MockServer,
    session: AuthSession,
    domain: Uuid,
    id: Uuid,
}

impl Fixture {
    async fn new(app: bool) -> Self {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/user/login");
                then.header("content-type", "application/json")
                    .json_body(json!({"access_token":"user-api", "refresh_token":"user-refresh"}));
            })
            .await;
        let exchange = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/service/domains-access-token")
                    .header(
                        "authorization",
                        if app {
                            "Basic a2V5OnNlY3JldA=="
                        } else {
                            "Bearer user-api"
                        },
                    );
                then.header("content-type", "application/json")
                    .json_body(json!({"access_token":"dds-service"}));
            })
            .await;
        let env = AuthEnvironment::new(server.base_url(), server.base_url())
            .unwrap()
            .with_client_id("stable-installation")
            .unwrap();
        let credentials = if app {
            Credentials::app("key", "secret")
        } else {
            Credentials::user_password("test@example.com", "password")
        };
        let session = AuthClient::new(env)
            .unwrap()
            .authenticate(credentials)
            .await
            .unwrap();
        exchange.assert_calls_async(1).await;
        Self {
            server,
            session,
            domain: Uuid::new_v4(),
            id: Uuid::new_v4(),
        }
    }

    fn client(&self) -> auki_domain_client::DomainDataClient {
        AukiDomainData::new(self.session.clone())
            .unwrap()
            .in_domain(self.domain)
    }

    fn token(&self, domain: Uuid, aud: &str, seconds: i64) -> String {
        format!(
            "e30.{}.fixture-signature",
            URL_SAFE_NO_PAD.encode(
                json!({
                    "iss":"dds","domain_id":domain, "aud":[aud], "exp":chrono::Utc::now().timestamp()+seconds,
                })
                .to_string()
            )
        )
    }

    fn grant(&self, token: &str) -> Value {
        json!({"id":self.domain,"domain_server":{"url":self.server.base_url()},"access_token":token})
    }

    async fn auth(&self, value: Value) -> Mock<'_> {
        self.server
            .mock_async(|when, then| {
                when.method(POST)
                    .path(format!("/api/v1/domains/{}/auth", self.domain))
                    .header("authorization", "Bearer dds-service")
                    .header("posemesh-client-id", "stable-installation")
                    .header_exists("posemesh-sdk-version");
                then.header("content-type", "application/json")
                    .json_body(value);
            })
            .await
    }

    async fn valid_auth(&self) -> Mock<'_> {
        self.auth(self.grant(&self.token(self.domain, &self.server.base_url(), 3600)))
            .await
    }

    fn path(&self) -> String {
        format!("/api/v1/domains/{}/data", self.domain)
    }
    fn metadata(&self) -> Value {
        json!({"id":self.id,"domain_id":self.domain,"name":"report","data_type":"my-app.report.v1","size":7,
            "created_at":"2026-09-01T00:00:00Z","updated_at":"2026-09-01T00:00:00Z"})
    }

    async fn info(&self, request_limit: u64) -> Mock<'_> {
        self.server.mock_async(|when, then| {
            when.method(GET).path("/api/v1/info");
            then.header("content-type","application/json").json_body(json!({"upload":{"domain_data_max_bytes":1048576,"request_max_bytes":request_limit}}));
        }).await
    }
}

#[tokio::test]
async fn user_and_app_list_real_pages_without_minting_domain_tokens() {
    for app in [false, true] {
        let f = Fixture::new(app).await;
        let auth = f.valid_auth().await;
        let page = f.server.mock_async(|when, then| {
            when.method(GET).path("/api/v1/domains").query_param("org","own")
                .query_param("limit","2").query_param("offset","4").query_param("issue_token","false")
                .header("authorization","Bearer dds-service").header("posemesh-client-id","stable-installation")
                .header_exists("posemesh-sdk-version");
            then.header("content-type","application/json").json_body(json!({"domains":[{"id":f.domain,"name":"Test","organization_id":null}],"total":5,"limit":2,"offset":4}));
        }).await;
        let result = AukiDomains::new(f.session.clone())
            .list(&DomainListQuery {
                limit: 2,
                offset: 4,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.domains[0].id, f.domain);
        assert_eq!((result.total, result.limit, result.offset), (5, 2, 4));
        page.assert_calls_async(1).await;
        auth.assert_calls_async(0).await;
    }
}

#[tokio::test]
async fn metadata_filters_get_and_raw_read_share_one_domain_grant() {
    let f = Fixture::new(false).await;
    let auth = f.valid_auth().await;
    let token = f
        .session
        .domain_access(f.domain, None, &CancellationToken::new())
        .await
        .unwrap();
    let list = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path(f.path())
                .query_param("name", "report & next")
                .query_param("data_type", "my-app.report.v1")
                .query_param("ids", f.id.to_string())
                .header("accept", "application/json")
                .header(
                    "authorization",
                    format!("Bearer {}", token.bearer().expose_secret()),
                );
            then.header("content-type", "application/json")
                .json_body(json!({"data":[f.metadata()]}));
        })
        .await;
    let get = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path(format!("{}/{}", f.path(), f.id))
                .query_param_missing("raw")
                .header("accept", "application/json");
            then.header("content-type", "application/json")
                .json_body(f.metadata());
        })
        .await;
    let read = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path(format!("{}/{}", f.path(), f.id))
                .query_param("raw", "true");
            then.body("payload");
        })
        .await;
    let client = f.client();
    assert_eq!(
        client
            .list(&DataListQuery {
                ids: vec![f.id],
                name: Some("report & next".into()),
                data_type: Some("my-app.report.v1".into())
            })
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(client.get(f.id).await.unwrap().id, f.id);
    assert_eq!(client.read(f.id).await.unwrap(), b"payload");
    for mock in [auth, list, get, read] {
        mock.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn named_write_id_replacement_and_delete_preserve_wire_contracts() {
    let f = Fixture::new(false).await;
    let auth = f.valid_auth().await;
    f.info(1048576).await;
    let named = f
        .server
        .mock_async(|when, then| {
            when.method(POST)
                .path(f.path())
                .body_includes("name=\"report\"; data-type=\"my-app.report.v1\"")
                .body_includes("\r\n\r\npayload\r\n--auki-");
            then.header("content-type", "application/json")
                .json_body(json!({"data":[f.metadata()]}));
        })
        .await;
    let update = f
        .server
        .mock_async(|when, then| {
            when.method(PUT)
                .path(f.path())
                .body_includes(format!("id=\"{}\"", f.id))
                .body_includes("replacement");
            then.header("content-type", "application/json")
                .json_body(json!({"data":[f.metadata()]}));
        })
        .await;
    let delete = f
        .server
        .mock_async(|when, then| {
            when.method(DELETE).path(format!("{}/{}", f.path(), f.id));
            then.status(200);
        })
        .await;
    let client = f.client();
    assert_eq!(
        client
            .write(
                DataWrite::Named {
                    name: "report",
                    data_type: "my-app.report.v1"
                },
                b"payload"
            )
            .await
            .unwrap()
            .id,
        f.id
    );
    assert_eq!(
        client
            .write(DataWrite::ById(f.id), b"replacement")
            .await
            .unwrap()
            .id,
        f.id
    );
    client.delete(f.id).await.unwrap();
    for mock in [auth, named, update, delete] {
        mock.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn denied_app_writes_and_deletes_do_not_refresh_or_retry() {
    let f = Fixture::new(true).await;
    let auth = f.valid_auth().await;
    f.info(1048576).await;
    let denied = f
        .server
        .mock_async(|when, then| {
            when.method(POST).path(f.path());
            then.status(403).body("secret server details");
        })
        .await;
    let deleted = f
        .server
        .mock_async(|when, then| {
            when.method(DELETE).path(format!("{}/{}", f.path(), f.id));
            then.status(403);
        })
        .await;
    let client = f.client();
    let error = client
        .write(
            DataWrite::Named {
                name: "report",
                data_type: "custom",
            },
            b"payload",
        )
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(403));
    assert!(!format!("{error:?}").contains("secret server details"));
    assert_eq!(client.delete(f.id).await.unwrap_err().status(), Some(403));
    for mock in [auth, denied, deleted] {
        mock.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn wrong_domain_wrong_audience_expired_and_unsafe_server_grants_are_rejected() {
    let f = Fixture::new(false).await;
    let values = [
        f.grant(&format!("e30.{}.fixture-signature", URL_SAFE_NO_PAD.encode(json!({"iss":"api","domain_id":f.domain,"aud":[f.server.base_url()],"exp":chrono::Utc::now().timestamp()+3600}).to_string()))),
        f.grant(&f.token(Uuid::new_v4(), &f.server.base_url(), 3600)),
        f.grant(&f.token(f.domain, "https://other.example", 3600)),
        f.grant(&f.token(f.domain, &f.server.base_url(), -10)),
        json!({"id":Uuid::new_v4(),"domain_server":{"url":f.server.base_url()},"access_token":f.token(f.domain,&f.server.base_url(),3600)}),
        json!({"id":f.domain,"domain_server":{"url":"http://untrusted.example"},"access_token":"redacted"}),
    ];
    let data = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(f.path());
            then.header("content-type", "application/json")
                .json_body(json!({"data":[]}));
        })
        .await;
    for value in values {
        let auth = f.auth(value).await;
        assert!(f.client().list(&DataListQuery::default()).await.is_err());
        auth.assert_calls_async(1).await;
        auth.delete_async().await;
    }
    data.assert_calls_async(0).await;
}

#[tokio::test]
async fn rejection_renews_once_and_concurrent_clients_share_the_replacement() {
    let f = Fixture::new(false).await;
    let first = f.valid_auth().await;
    let cancel = CancellationToken::new();
    let old = f
        .session
        .domain_access(f.domain, None, &cancel)
        .await
        .unwrap();
    first.delete_async().await;
    let next_token = f.token(f.domain, &f.server.base_url(), 7200);
    let next = f.auth(f.grant(&next_token)).await;
    let (a, b) = tokio::join!(
        f.session.domain_access(f.domain, Some(&old), &cancel),
        f.session.domain_access(f.domain, Some(&old), &cancel)
    );
    assert!(std::sync::Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
    next.assert_calls_async(1).await;

    let rejected = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(f.path());
            then.status(401);
        })
        .await;
    assert_eq!(
        f.client()
            .list(&DataListQuery::default())
            .await
            .unwrap_err()
            .status(),
        Some(401)
    );
    rejected.assert_calls_async(2).await;
    next.assert_calls_async(2).await;
}

#[tokio::test]
async fn expiring_data_access_is_refetched_without_another_user_login() {
    let f = Fixture::new(false).await;
    let expiring = f
        .auth(f.grant(&f.token(f.domain, &f.server.base_url(), 10)))
        .await;
    f.session
        .domain_access(f.domain, None, &CancellationToken::new())
        .await
        .unwrap();
    expiring.delete_async().await;
    let renewed = f.valid_auth().await;
    let access = f
        .session
        .domain_access(f.domain, None, &CancellationToken::new())
        .await
        .unwrap();
    assert!(access.expires_at() > chrono::Utc::now() + chrono::Duration::minutes(30));
    renewed.assert_calls_async(1).await;
}

#[tokio::test]
async fn closing_one_data_client_keeps_other_clients_and_session_usable() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    let data = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(f.path());
            then.header("content-type", "application/json")
                .json_body(json!({"data":[]}));
        })
        .await;
    let first = f.client();
    let second = f.client();
    first.close().await;
    assert!(matches!(
        first.list(&DataListQuery::default()).await,
        Err(DataError::Closed)
    ));
    second.list(&DataListQuery::default()).await.unwrap();
    f.session.close().await;
    assert!(matches!(
        second.list(&DataListQuery::default()).await,
        Err(DataError::Auth(auki_auth::Error::SessionClosed))
    ));
    data.assert_calls_async(1).await;
}

#[tokio::test]
async fn cancellation_and_close_abort_inflight_requests_without_retry() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    let delayed = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(f.path());
            then.delay(Duration::from_secs(3))
                .header("content-type", "application/json")
                .json_body(json!({"data":[]}));
        })
        .await;
    let client = f.client();
    let cancel = CancellationToken::new();
    let pending = tokio::spawn({
        let client = client.clone();
        let cancel = cancel.clone();
        async move {
            client
                .list_with_cancellation(&DataListQuery::default(), &cancel)
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while delayed.calls_async().await == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(matches!(pending.await.unwrap(), Err(DataError::Cancelled)));
    let pending = tokio::spawn({
        let client = client.clone();
        async move { client.list(&DataListQuery::default()).await }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while delayed.calls_async().await < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_millis(500), client.close())
        .await
        .unwrap();
    assert!(matches!(pending.await.unwrap(), Err(DataError::Closed)));
    delayed.assert_calls_async(2).await;
}

#[tokio::test]
async fn oversized_responses_and_server_upload_limits_fail_before_upload() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    f.info(1).await;
    f.server
        .mock_async(|when, then| {
            when.method(GET).path(format!("{}/{}", f.path(), f.id));
            then.body("too many bytes");
        })
        .await;
    let upload = f
        .server
        .mock_async(|when, then| {
            when.method(POST).path(f.path());
            then.status(500);
        })
        .await;
    let client = AukiDomainData::with_limits(
        f.session.clone(),
        DataLimits {
            max_data_bytes: 8,
            ..Default::default()
        },
    )
    .unwrap()
    .in_domain(f.domain);
    assert!(matches!(
        client.read(f.id).await,
        Err(DataError::TooLarge { maximum: 8 })
    ));
    assert!(matches!(
        client.write(DataWrite::ById(f.id), b"123456789").await,
        Err(DataError::TooLarge { maximum: 8 })
    ));
    assert!(matches!(
        client
            .write(
                DataWrite::Named {
                    name: "report",
                    data_type: "custom"
                },
                b"payload"
            )
            .await,
        Err(DataError::TooLarge { maximum: 1 })
    ));
    upload.assert_calls_async(0).await;
}

#[tokio::test]
async fn missing_items_credit_denials_and_server_failures_keep_their_status() {
    let f = Fixture::new(false).await;
    let auth = f.valid_auth().await;
    f.info(1048576).await;
    for status in [402, 403, 404, 413, 429, 500, 503] {
        let failed = f
            .server
            .mock_async(|when, then| {
                when.method(PUT).path(f.path());
                then.status(status);
            })
            .await;
        assert_eq!(
            f.client()
                .write(DataWrite::ById(f.id), b"payload")
                .await
                .unwrap_err()
                .status(),
            Some(status)
        );
        failed.assert_calls_async(1).await;
        failed.delete_async().await;
    }
    auth.assert_calls_async(1).await;
}

#[tokio::test]
async fn metadata_from_another_domain_or_item_is_rejected() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    let mut metadata = f.metadata();
    metadata["domain_id"] = json!(Uuid::new_v4());
    f.server
        .mock_async(|when, then| {
            when.method(GET).path(f.path());
            then.header("content-type", "application/json")
                .json_body(json!({"data":[metadata]}));
        })
        .await;
    assert!(matches!(
        f.client().list(&DataListQuery::default()).await,
        Err(DataError::InvalidResponse(_))
    ));
    let mut metadata = f.metadata();
    metadata["id"] = json!(Uuid::new_v4());
    f.server
        .mock_async(|when, then| {
            when.method(GET).path(format!("{}/{}", f.path(), f.id));
            then.header("content-type", "application/json")
                .json_body(metadata);
        })
        .await;
    assert!(matches!(
        f.client().get(f.id).await,
        Err(DataError::InvalidResponse(_))
    ));
}

#[tokio::test]
async fn redirects_never_replay_domain_tokens_or_uploads() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    f.info(1048576).await;
    let destination = MockServer::start_async().await;
    let leak = destination
        .mock_async(|_, then| {
            then.status(200);
        })
        .await;
    f.server
        .mock_async(|when, then| {
            when.method(POST).path(f.path());
            then.status(307).header("location", destination.base_url());
        })
        .await;
    assert_eq!(
        f.client()
            .write(
                DataWrite::Named {
                    name: "report",
                    data_type: "custom"
                },
                b"payload"
            )
            .await
            .unwrap_err()
            .status(),
        Some(307)
    );
    leak.assert_calls_async(0).await;
}

#[tokio::test]
async fn invalid_names_and_types_fail_before_domain_access_or_upload() {
    let f = Fixture::new(false).await;
    let auth = f.valid_auth().await;
    for (name, data_type) in [
        ("report", "my-app/report.v1"),
        ("bad\r\nheader", "report"),
        ("", "report"),
        ("report", "bad\"type"),
    ] {
        assert!(matches!(
            f.client()
                .write(DataWrite::Named { name, data_type }, b"bytes")
                .await,
            Err(DataError::InvalidInput(_))
        ));
    }
    auth.assert_calls_async(0).await;
}

#[tokio::test]
async fn timed_out_write_is_not_replayed_and_does_not_close_the_session() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    f.info(1048576).await;
    let upload = f
        .server
        .mock_async(|when, then| {
            when.method(POST).path(f.path());
            then.delay(Duration::from_secs(2))
                .header("content-type", "application/json")
                .json_body(json!({"data":[f.metadata()]}));
        })
        .await;
    let client = AukiDomainData::with_limits(
        f.session.clone(),
        DataLimits {
            request_timeout: Duration::from_millis(200),
            ..Default::default()
        },
    )
    .unwrap()
    .in_domain(f.domain);
    assert!(matches!(
        client
            .write(
                DataWrite::Named {
                    name: "report",
                    data_type: "custom"
                },
                b"payload"
            )
            .await,
        Err(DataError::TimedOut)
    ));
    upload.assert_calls_async(1).await;
    f.session
        .domain_access(f.domain, None, &CancellationToken::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn upload_rechecks_limits_if_renewal_changes_the_domain_server() {
    let f = Fixture::new(false).await;
    let first = f.valid_auth().await;
    f.session
        .domain_access(f.domain, None, &CancellationToken::new())
        .await
        .unwrap();
    first.delete_async().await;
    f.info(1048576).await;
    let rejected = f
        .server
        .mock_async(|when, then| {
            when.method(POST).path(f.path());
            then.status(401);
        })
        .await;
    let next = MockServer::start_async().await;
    next.mock_async(|when, then| {
        when.method(GET)
            .path("/api/v1/info")
            .header_missing("authorization");
        then.header("content-type", "application/json")
            .json_body(json!({"upload":{"domain_data_max_bytes":1,"request_max_bytes":1048576}}));
    })
    .await;
    let upload = next
        .mock_async(|when, then| {
            when.method(POST).path(f.path());
            then.status(200);
        })
        .await;
    let renewed = f.auth(json!({"id":f.domain,"domain_server":{"url":next.base_url()},"access_token":f.token(f.domain,&next.base_url(),3600)})).await;
    assert!(matches!(
        f.client()
            .write(
                DataWrite::Named {
                    name: "report",
                    data_type: "custom"
                },
                b"payload"
            )
            .await,
        Err(DataError::TooLarge { maximum: 1 })
    ));
    rejected.assert_calls_async(1).await;
    renewed.assert_calls_async(1).await;
    upload.assert_calls_async(0).await;
}
