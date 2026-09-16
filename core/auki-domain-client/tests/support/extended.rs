use super::*;
use auki_domain_client::{PortalId, TransferOptions};
use std::sync::{Arc, Mutex};

async fn multipart_fixture(f: &Fixture, part_size: i64) -> Uuid {
    let upload = Uuid::new_v4();
    f.server.mock_async(|when, then| {
        when.method(GET).path("/api/v1/info").header_missing("authorization");
        then.header("content-type", "application/json").json_body(json!({"upload":{"request_max_bytes":4096,"domain_data_max_bytes":10000,"multipart":{"enabled":true,"part_size_bytes":part_size}}}));
    }).await;
    f.server.mock_async(|when, then| {
        when.method(POST).path(format!("{}/multipart",f.path())).query_param("uploads", "").json_body_includes(r#"{"size":7}"#);
        then.header("content-type", "application/json").json_body(json!({"upload_id":upload,"data_id":f.id,"part_size":part_size,"expires_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339()}));
    }).await;
    upload
}
async fn abort_mock(f: &Fixture, upload: Uuid) -> Mock<'_> {
    f.server
        .mock_async(|when, then| {
            when.method(DELETE)
                .path(format!("{}/multipart", f.path()))
                .query_param("uploadId", upload.to_string());
            then.status(200);
        })
        .await
}
fn options() -> TransferOptions {
    TransferOptions {
        max_bytes: 10000,
        max_chunk_bytes: 16,
    }
}
fn source(
    bytes: &[u8],
) -> impl FnMut(usize) -> std::future::Ready<Result<Vec<u8>, DataError>> + '_ {
    let mut data = bytes.iter().copied();
    move |max| std::future::ready(Ok(data.by_ref().take(max.min(2)).collect()))
}

#[tokio::test]
async fn multipart_upload_uses_bounded_parts_exact_size_and_completion_receipts() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    let upload = multipart_fixture(&f, 4).await;
    let abort = abort_mock(&f, upload).await;
    let first = f
        .server
        .mock_async(|when, then| {
            when.method(PUT)
                .path(format!("{}/multipart", f.path()))
                .query_param("uploadId", upload.to_string())
                .query_param("partNumber", "1")
                .body("abcd");
            then.header("content-type", "application/json")
                .json_body(json!({"etag":"one"}));
        })
        .await;
    let last = f
        .server
        .mock_async(|when, then| {
            when.method(PUT)
                .path(format!("{}/multipart", f.path()))
                .query_param("partNumber", "2")
                .body("efg");
            then.header("content-type", "application/json")
                .json_body(json!({"etag":"two"}));
        })
        .await;
    let complete = f.server.mock_async(|when,then| {
        when.method(POST).path(format!("{}/multipart",f.path())).query_param("uploadId",upload.to_string()).json_body(json!({"parts":[{"part_number":1,"etag":"one"},{"part_number":2,"etag":"two"}]}));
        then.header("content-type","application/json").json_body(f.metadata());
    }).await;
    let metadata = f
        .client()
        .write_stream(
            DataWrite::Named {
                name: "report",
                data_type: "my-app.report.v1",
            },
            7,
            options(),
            &CancellationToken::new(),
            source(b"abcdefg"),
        )
        .await
        .unwrap();
    assert_eq!(metadata.id, f.id);
    first.assert_calls_async(1).await;
    last.assert_calls_async(1).await;
    complete.assert_calls_async(1).await;
    abort.assert_calls_async(0).await;
}

#[tokio::test]
async fn failed_or_oversized_sources_abort_known_uploads_without_completion() {
    for kind in ["short", "large", "callback", "extra"] {
        let f = Fixture::new(false).await;
        f.valid_auth().await;
        let upload = multipart_fixture(&f, 4).await;
        let abort = abort_mock(&f, upload).await;
        f.server
            .mock_async(|when, then| {
                when.method(PUT).path(format!("{}/multipart", f.path()));
                then.header("content-type", "application/json")
                    .json_body(json!({"etag":"ok"}));
            })
            .await;
        let mut remaining = 8usize;
        let result = f
            .client()
            .write_stream(
                DataWrite::ById(f.id),
                7,
                options(),
                &CancellationToken::new(),
                |maximum| {
                    std::future::ready(match kind {
                        "short" => Ok(vec![]),
                        "large" => Ok(vec![0; maximum + 1]),
                        "callback" => Err(DataError::Callback),
                        _ => {
                            let take = maximum.min(remaining);
                            remaining -= take;
                            Ok(vec![0; take])
                        }
                    })
                },
            )
            .await;
        assert!(result.is_err());
        abort.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn cancellation_and_close_await_multipart_abort_while_source_is_pending() {
    for close in [false, true] {
        let f = Fixture::new(false).await;
        f.valid_auth().await;
        let upload = multipart_fixture(&f, 4).await;
        let abort = abort_mock(&f, upload).await;
        let client = f.client();
        let cancel = CancellationToken::new();
        let entered = Arc::new(tokio::sync::Notify::new());
        let work = tokio::spawn({
            let client = client.clone();
            let cancel = cancel.clone();
            let entered = entered.clone();
            async move {
                client
                    .write_stream(DataWrite::ById(f.id), 7, options(), &cancel, move |_| {
                        entered.notify_one();
                        std::future::pending::<Result<Vec<u8>, DataError>>()
                    })
                    .await
            }
        });
        entered.notified().await;
        if close {
            tokio::time::timeout(Duration::from_secs(2), client.close())
                .await
                .expect("close must drain without deadlock");
        } else {
            cancel.cancel();
        }
        assert!(matches!(
            work.await.unwrap(),
            Err(DataError::Cancelled | DataError::Closed)
        ));
        abort.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn upload_http_errors_are_not_replayed_and_cleanup_failure_is_visible() {
    for status in [403, 413, 500] {
        let f = Fixture::new(false).await;
        f.valid_auth().await;
        let upload = multipart_fixture(&f, 4).await;
        let part = f
            .server
            .mock_async(|when, then| {
                when.method(PUT).path(format!("{}/multipart", f.path()));
                then.status(status);
            })
            .await;
        let abort = f
            .server
            .mock_async(|when, then| {
                when.method(DELETE)
                    .path(format!("{}/multipart", f.path()))
                    .query_param("uploadId", upload.to_string());
                then.status(503);
            })
            .await;
        let error = f
            .client()
            .write_stream(
                DataWrite::ById(f.id),
                7,
                options(),
                &CancellationToken::new(),
                source(b"abcdefg"),
            )
            .await
            .unwrap_err();
        assert_eq!(error.status(), Some(status));
        assert!(matches!(error, DataError::Cleanup { .. }));
        part.assert_calls_async(1).await;
        abort.assert_calls_async(1).await;
    }
}

#[tokio::test]
async fn selected_server_part_limit_is_checked_before_starting_an_upload() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    multipart_fixture(&f, 32).await;
    let result = f
        .client()
        .write_stream(
            DataWrite::ById(f.id),
            7,
            options(),
            &CancellationToken::new(),
            |_| std::future::ready(Err(DataError::Callback)),
        )
        .await;
    assert!(matches!(result, Err(DataError::TooLarge { maximum: 16 })));
}

#[tokio::test]
async fn streaming_download_delivers_chunks_with_backpressure_and_enforces_limit() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    let raw = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path(format!("{}/{}", f.path(), f.id))
                .query_param("raw", "true");
            then.body("abcdefg");
        })
        .await;
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = received.clone();
    assert_eq!(
        f.client()
            .read_to(
                f.id,
                TransferOptions {
                    max_bytes: 7,
                    max_chunk_bytes: 2
                },
                &CancellationToken::new(),
                move |bytes| {
                    assert!(bytes.len() <= 2);
                    sink.lock().unwrap().extend(bytes);
                    async { Ok(()) }
                }
            )
            .await
            .unwrap(),
        7
    );
    assert_eq!(*received.lock().unwrap(), b"abcdefg");
    assert!(matches!(
        f.client()
            .read_to(
                f.id,
                TransferOptions {
                    max_bytes: 6,
                    max_chunk_bytes: 2
                },
                &CancellationToken::new(),
                |_| async { Ok(()) }
            )
            .await,
        Err(DataError::TooLarge { maximum: 6 })
    ));
    raw.assert_calls_async(2).await;
}

fn portal(f: &Fixture) -> Value {
    json!({"id":f.id,"short_id":"ABC12345678","name":"Portal","size":10.0,"organization_id":null,"default_domain_id":f.domain,"redirect_url":null,"created_at":"2026-09-01T00:00:00Z","updated_at":"2026-09-01T00:00:00Z"})
}
fn pose(f: &Fixture) -> Value {
    json!({"id":f.id,"short_id":"ABC12345678","domain_id":f.domain,"reported_size":10.0,"px":1.0,"py":2.0,"pz":3.0,"rx":0.0,"ry":0.0,"rz":0.0,"rw":1.0,"scanner_device_id":"fixture","scanner_device_name":"fixture","scanner_device_model":"fixture","placed_at":"2026-09-01T00:00:00Z"})
}
#[tokio::test]
async fn portal_lookup_uses_user_or_app_service_authority_without_minting_grants() {
    for app in [false, true] {
        let f = Fixture::new(app).await;
        let auth = f.valid_auth().await;
        let lookup=f.server.mock_async(|when,then| {
            when.method(GET).path("/api/v1/lighthouses/ABC12345678/domains").query_param("org","all").query_param("issue_token","false").header("authorization","Bearer dds-service").header_exists("posemesh-sdk-version");
            then.header("content-type","application/json").json_body(json!({"domains":[{"id":f.domain,"name":"Domain","organization_id":null,"is_default":true,"added_to_domain_at":"2026-09-01T00:00:00Z"}]}));
        }).await;
        let result = AukiDomains::new(f.session.clone())
            .for_portal(
                &PortalId::parse("abc12345678").unwrap(),
                "all",
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(result[0].is_default);
        lookup.assert_calls_async(1).await;
        auth.assert_calls_async(0).await;
        assert!(PortalId::parse("../../other").is_err());
    }
}
#[tokio::test]
async fn portal_and_pose_reads_use_separate_hosts_and_check_selected_identity() {
    let f = Fixture::new(false).await;
    let token=format!("e30.{}.sig",URL_SAFE_NO_PAD.encode(json!({"iss":"dds","domain_id":f.domain,"aud":["dds",f.server.base_url()],"exp":chrono::Utc::now().timestamp()+3600}).to_string()));
    f.auth(f.grant(&token)).await;
    // Both mocked hosts share a listener, so match their differing DDS vs DS envelope
    // through sequential installation while validating Domain/portal paths.
    let path = format!("/api/v1/domains/{}/lighthouses", f.domain);
    let list = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path(&path)
                .header("authorization", format!("Bearer {token}"));
            then.header("content-type", "application/json")
                .json_body(json!({"lighthouses":[portal(&f)]}));
        })
        .await;
    let domains = AukiDomains::new(f.session.clone());
    assert_eq!(
        domains
            .portals(f.domain, &CancellationToken::new())
            .await
            .unwrap()[0]
            .id,
        f.id
    );
    list.delete_async().await;
    let get = f
        .server
        .mock_async(|when, then| {
            when.method(GET).path(format!("{path}/{}", f.id));
            let mut record = portal(&f);
            record["domain_id"] = json!(f.domain);
            then.header("content-type", "application/json")
                .json_body(record);
        })
        .await;
    let portal_id = PortalId::parse(&f.id.to_string()).unwrap();
    assert_eq!(
        domains
            .portal(f.domain, &portal_id, &CancellationToken::new())
            .await
            .unwrap()
            .id,
        f.id
    );
    get.delete_async().await;
    f.server
        .mock_async(|when, then| {
            when.method(GET).path(&path);
            then.header("content-type", "application/json")
                .json_body(json!({"poses":[pose(&f)]}));
        })
        .await;
    assert_eq!(
        f.client().poses(&CancellationToken::new()).await.unwrap()[0].px,
        1.0
    );
    f.server
        .mock_async(|when, then| {
            when.method(GET).path(format!("{path}/{}", f.id));
            let mut record = pose(&f);
            record["domain_id"] = json!(Uuid::new_v4());
            then.header("content-type", "application/json")
                .json_body(record);
        })
        .await;
    assert!(matches!(
        f.client().pose(&portal_id, &CancellationToken::new()).await,
        Err(DataError::InvalidResponse(_))
    ));
}
#[tokio::test]
async fn portal_request_requires_dds_audience_and_surfaces_pose_permission_denial() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    assert!(matches!(
        AukiDomains::new(f.session.clone())
            .portals(f.domain, &CancellationToken::new())
            .await,
        Err(DataError::Auth(_))
    ));
    let denied = f
        .server
        .mock_async(|when, then| {
            when.method(GET)
                .path(format!("/api/v1/domains/{}/lighthouses", f.domain));
            then.status(403);
        })
        .await;
    assert_eq!(
        f.client()
            .poses(&CancellationToken::new())
            .await
            .unwrap_err()
            .status(),
        Some(403)
    );
    denied.assert_calls_async(1).await;
}

#[tokio::test]
async fn multipart_renews_after_401_without_reading_a_part_twice() {
    let f = Fixture::new(false).await;
    let old_auth = f.valid_auth().await;
    let upload = multipart_fixture(&f, 8).await;
    let abort = abort_mock(&f, upload).await;
    let old_token = f.token(f.domain, &f.server.base_url(), 3600);
    let new_token = f.token(f.domain, &f.server.base_url(), 7200);
    let rejected = f
        .server
        .mock_async(|when, then| {
            when.method(PUT)
                .path(format!("{}/multipart", f.path()))
                .header("authorization", format!("Bearer {old_token}"));
            then.status(401);
        })
        .await;
    let accepted = f
        .server
        .mock_async(|when, then| {
            when.method(PUT)
                .path(format!("{}/multipart", f.path()))
                .header("authorization", format!("Bearer {new_token}"))
                .body("abcdefg");
            then.header("content-type", "application/json")
                .json_body(json!({"etag":"ok"}));
        })
        .await;
    f.server
        .mock_async(|when, then| {
            when.method(POST)
                .path(format!("{}/multipart", f.path()))
                .query_param("uploadId", upload.to_string());
            then.header("content-type", "application/json")
                .json_body(f.metadata());
        })
        .await;
    let reads = std::sync::atomic::AtomicUsize::new(0);
    f.client()
        .write_stream(
            DataWrite::ById(f.id),
            7,
            options(),
            &CancellationToken::new(),
            |_| {
                let first = reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
                let f = &f;
                let old_auth = &old_auth;
                let new_token = &new_token;
                async move {
                    if first {
                        old_auth.delete_async().await;
                        f.auth(f.grant(new_token)).await;
                        Ok(b"abcdefg".to_vec())
                    } else {
                        Ok(vec![])
                    }
                }
            },
        )
        .await
        .unwrap();
    assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 2);
    rejected.assert_calls_async(1).await;
    accepted.assert_calls_async(1).await;
    abort.assert_calls_async(0).await;
}

#[tokio::test]
async fn interrupted_completion_is_not_replayed_and_known_upload_is_aborted() {
    let f = Fixture::new(false).await;
    f.valid_auth().await;
    let upload = multipart_fixture(&f, 8).await;
    let abort = abort_mock(&f, upload).await;
    f.server
        .mock_async(|when, then| {
            when.method(PUT).path(format!("{}/multipart", f.path()));
            then.header("content-type", "application/json")
                .json_body(json!({"etag":"ok"}));
        })
        .await;
    let complete = f
        .server
        .mock_async(|when, then| {
            when.method(POST)
                .path(format!("{}/multipart", f.path()))
                .query_param("uploadId", upload.to_string());
            then.delay(Duration::from_millis(150))
                .header("content-type", "application/json")
                .json_body(f.metadata());
        })
        .await;
    let client = AukiDomainData::with_limits(
        f.session.clone(),
        DataLimits {
            request_timeout: Duration::from_millis(70),
            ..DataLimits::default()
        },
    )
    .unwrap()
    .in_domain(f.domain);
    assert!(matches!(
        client
            .write_stream(
                DataWrite::ById(f.id),
                7,
                options(),
                &CancellationToken::new(),
                source(b"abcdefg")
            )
            .await,
        Err(DataError::TimedOut)
    ));
    complete.assert_calls_async(1).await;
    abort.assert_calls_async(1).await;
}
