#![cfg(not(target_arch = "wasm32"))]

use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use auki_component_protocol::{
    CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint, ComponentProtocolError,
    RemoteMirrorStart,
};
use auki_components::{
    BufferLimits, ComponentRuntime, ComponentSpec, ConfiguredObservableSpec, CursorStart, Exposure,
    GaugePayloadContract, InputPort, ObservableContract, Observation, ObservationAccess,
    OperableContract, PayloadContract, ProductForm, ProductInputContract,
};
use auki_p2p::{
    P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_SCOPE, P2P_TOKEN_TYPE, P2PAccessClaims,
};
use auki_sdk::{
    AukiPeer, AukiPeerConfig, DdsVerificationKeys, ExternalAuthorityUpdate, Identity, Multiaddr,
    SignedP2pCredential,
};
use chrono::{TimeZone, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use uuid::Uuid;

const TEST_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

const TEST_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

fn authority(identity: &Identity, domain_id: Uuid) -> ExternalAuthorityUpdate {
    let issued_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let expires_at = issued_at + 30 * 60;
    let claims = P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.to_owned(),
        iss: P2P_TOKEN_ISSUER.to_owned(),
        aud: vec![P2P_TOKEN_AUDIENCE.to_owned()],
        sub: Uuid::new_v4().to_string(),
        organization_id: None,
        peer_type: Some("test".to_owned()),
        peer_id: identity.peer_id().to_string(),
        domain_ids: vec![domain_id.to_string()],
        scopes: vec![P2P_TOKEN_SCOPE.to_owned()],
        application: None,
        iat: issued_at,
        nbf: None,
        exp: expires_at,
    };
    let compact = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY).unwrap(),
    )
    .unwrap();
    ExternalAuthorityUpdate::new(
        domain_id,
        identity.peer_id(),
        DdsVerificationKeys::new(0, TEST_PUBLIC_KEY.to_vec(), None),
        SignedP2pCredential::new(compact).unwrap(),
        Utc.timestamp_opt(expires_at as i64, 0).unwrap(),
    )
}

fn direct_config() -> AukiPeerConfig {
    AukiPeerConfig::new("http://127.0.0.1:9")
        .unwrap()
        .direct_only()
        .with_listen_addresses([Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()])
        .unwrap()
}

fn gauge_contract(name: &str) -> ObservableContract {
    ObservableContract {
        name: name.to_owned(),
        datatype: "float64".to_owned(),
        schema: "test.level/v1".to_owned(),
        access: vec![ObservationAccess::FollowNew],
        exposure: Exposure::Cluster,
    }
}

fn gauge_payload() -> PayloadContract {
    PayloadContract::Gauge(GaugePayloadContract {
        datatype: "float64".to_owned(),
        schema: "test.level/v1".to_owned(),
        observes: "test_signal".to_owned(),
        unit: "percent".to_owned(),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catalog_products_and_operables_cross_two_authenticated_peers() {
    let domain_id = Uuid::new_v4();
    let server_identity = Identity::generate();
    let client_identity = Identity::generate();
    let (server_peer, _server_authority) = AukiPeer::start_external(
        server_identity.clone(),
        authority(&server_identity, domain_id),
        direct_config(),
    )
    .await
    .unwrap();
    let (client_peer, _client_authority) = AukiPeer::start_external(
        client_identity.clone(),
        authority(&client_identity, domain_id),
        direct_config(),
    )
    .await
    .unwrap();
    let server_route = server_peer.listen_addresses()[0].clone();

    let server_runtime = ComponentRuntime::new(server_peer.peer_id().to_string());
    let sensor = server_runtime
        .component(ComponentSpec::new("sensor").observable(gauge_contract("level")))
        .unwrap();
    let output = sensor
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            format!("{}.clock", server_peer.peer_id()),
            gauge_payload(),
        ))
        .unwrap();
    sensor.expose().unwrap();
    let capture = server_runtime
        .capture_buffer("level-history", &output, BufferLimits::entries(8), |_| 8)
        .unwrap();
    output.publish(10, Arc::new(12.5)).unwrap();

    let private_sensor = server_runtime
        .component(ComponentSpec::new("unexported-sensor").observable(gauge_contract("level")))
        .unwrap();
    let private_output = private_sensor
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "private-level-1",
            format!("{}.clock", server_peer.peer_id()),
            gauge_payload(),
        ))
        .unwrap();
    private_sensor.expose().unwrap();
    let _private_capture = server_runtime
        .capture_buffer(
            "unexported-level-history",
            &private_output,
            BufferLimits::entries(8),
            |_| 8,
        )
        .unwrap();

    let expected_client = client_peer.peer_id().to_string();
    let actuator = server_runtime
        .component(ComponentSpec::new("actuator").operable(OperableContract {
            name: "double".to_owned(),
            instruction: "uint64".to_owned(),
            result: "uint64".to_owned(),
            exposure: Exposure::Cluster,
        }))
        .unwrap();
    let double = actuator
        .operable(
            "double",
            move |context| {
                context.caller_peer_id == expected_client
                    && context.caller_component_id == "operator-console"
            },
            |_, value: u64| Ok(value * 2),
        )
        .unwrap();
    actuator.expose().unwrap();

    let endpoint =
        ComponentProtocolEndpoint::mount(server_peer.protocols(), server_runtime).unwrap();
    endpoint.export_product(&capture.product()).unwrap();
    endpoint.export_operable(&double).unwrap();
    let client = ComponentProtocolClient::new(client_peer.protocols());

    let catalog = client
        .catalog_exact(client_identity.peer_id(), server_route.clone(), None)
        .await;
    assert!(
        catalog.is_err(),
        "the expected peer identity must be enforced"
    );

    let catalog = client
        .catalog_exact(server_peer.peer_id(), server_route.clone(), None)
        .await
        .unwrap();
    let CatalogResponse::Snapshot { snapshot } = catalog else {
        panic!("first Catalog request must return a snapshot")
    };
    assert_eq!(snapshot.components.len(), 2);
    assert_eq!(snapshot.products.len(), 1);
    assert!(
        snapshot
            .components
            .iter()
            .all(|entry| entry.manifest.component_id != "unexported-sensor")
    );
    assert!(
        snapshot
            .products
            .iter()
            .all(|entry| entry.manifest.product_id != "unexported-level-history")
    );
    assert!(matches!(
        client
            .catalog_exact(
                server_peer.peer_id(),
                server_route.clone(),
                Some(snapshot.revision),
            )
            .await
            .unwrap(),
        CatalogResponse::Unchanged { revision } if revision == snapshot.revision
    ));

    let product_reference = capture.product().reference();
    let mut mirror = client
        .mirror_product_exact::<f64>(
            server_peer.peer_id(),
            server_route.clone(),
            product_reference,
            BufferLimits::entries(8),
            |_| 8,
        )
        .await
        .unwrap();
    assert_eq!(mirror.product().buffer().range().first_sequence, Some(0));

    let client_runtime = ComponentRuntime::new(client_peer.peer_id().to_string());
    let detector = client_runtime
        .component(
            ComponentSpec::new("detector").product_input(ProductInputContract {
                name: "levels".to_owned(),
                form: ProductForm::Buffer,
                datatype: "float64".to_owned(),
                schema: "test.level/v1".to_owned(),
                exposure: Exposure::Cluster,
            }),
        )
        .unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let input = InputPort::<Observation<f64>>::new("detector.levels", move |envelope| {
        sink.lock().unwrap().push(envelope.payload.sequence);
    });
    let _binding = detector
        .configured_buffer_input(
            "levels",
            mirror.product(),
            CursorStart::FromSequence(0),
            &input,
        )
        .unwrap();
    detector.expose().unwrap();

    output.publish(20, Arc::new(18.0)).unwrap();
    let sync = mirror.sync_once().await.unwrap();
    assert_eq!(sync.accepted, 1);
    assert_eq!(sync.next_sequence, 2);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while received.lock().unwrap().as_slice() != [0, 1] {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }

    output.publish(30, Arc::new(22.0)).unwrap();
    let mut live_mirror = client
        .mirror_product_exact_with_start::<f64>(
            server_peer.peer_id(),
            server_route.clone(),
            capture.product().reference(),
            RemoteMirrorStart::LatestExisting,
            BufferLimits::entries(8),
            |_| 8,
        )
        .await
        .unwrap();
    assert_eq!(
        live_mirror.product().buffer().range().first_sequence,
        Some(2)
    );
    assert_eq!(live_mirror.next_sequence(), 3);

    output.publish(40, Arc::new(24.0)).unwrap();
    let live_sync = live_mirror.sync_once().await.unwrap();
    assert_eq!(live_sync.accepted, 1);
    assert_eq!(live_sync.next_sequence, 4);
    assert_eq!(
        live_mirror.product().buffer().range().last_sequence,
        Some(3)
    );

    output.publish(50, Arc::new(26.0)).unwrap();
    let second_live_sync = live_mirror.sync_once().await.unwrap();
    assert_eq!(second_live_sync.accepted, 1);
    assert_eq!(second_live_sync.next_sequence, 5);
    assert_eq!(
        live_mirror.product().buffer().range().last_sequence,
        Some(4)
    );

    assert!(
        !live_mirror
            .rebind_exact_route(server_route.clone())
            .unwrap()
    );
    assert!(
        live_mirror
            .rebind_exact_route(Multiaddr::from_str("/ip4/127.0.0.1/tcp/9").unwrap())
            .unwrap()
    );
    assert!(
        live_mirror
            .rebind_exact_route(server_route.clone())
            .unwrap()
    );
    output.publish(60, Arc::new(28.0)).unwrap();
    let rebound_sync = live_mirror.sync_once().await.unwrap();
    assert_eq!(rebound_sync.accepted, 1);
    assert_eq!(rebound_sync.next_sequence, 6);
    assert_eq!(
        live_mirror.product().buffer().range().last_sequence,
        Some(5)
    );

    output.publish(70, Arc::new(30.0)).unwrap();
    output.publish(80, Arc::new(32.0)).unwrap();
    let live_edge_sync = live_mirror.sync_latest_once().await.unwrap();
    assert_eq!(live_edge_sync.accepted, 1);
    assert_eq!(live_edge_sync.next_sequence, 8);
    assert_eq!(
        live_edge_sync.gap,
        Some(auki_component_protocol::SourceGap {
            requested_sequence: 6,
            available_from: 7,
        })
    );
    assert_eq!(live_mirror.last_gap(), live_edge_sync.gap);
    assert_eq!(
        live_mirror.product().buffer().range().last_sequence,
        Some(7)
    );

    let invocation = client
        .invoke_exact::<u64, u64>(
            server_peer.peer_id(),
            server_route.clone(),
            actuator.reference().clone(),
            "double",
            "operator-console",
            "invocation-1",
            Some(Duration::from_secs(1)),
            &21,
        )
        .await
        .unwrap();
    assert_eq!(invocation.result, 42);

    let unauthorized = client
        .invoke_exact::<u64, u64>(
            server_peer.peer_id(),
            server_route.clone(),
            actuator.reference().clone(),
            "double",
            "impostor",
            "invocation-2",
            Some(Duration::from_secs(1)),
            &21,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        unauthorized,
        ComponentProtocolError::RemoteOperation(error) if error.code == "unauthorized"
    ));

    assert!(endpoint.unexport_product("level-history"));
    let revised = client
        .catalog_exact(server_peer.peer_id(), server_route, Some(snapshot.revision))
        .await
        .unwrap();
    let CatalogResponse::Snapshot { snapshot: revised } = revised else {
        panic!("unexporting a Product must revise the network Catalog")
    };
    assert!(revised.revision > snapshot.revision);
    assert!(revised.products.is_empty());
    assert_eq!(revised.components.len(), 1);
    assert_eq!(revised.components[0].manifest.component_id, "actuator");

    mirror.close();
    live_mirror.close();
    endpoint.close().await.unwrap();
    client_peer.shutdown().await.unwrap();
    server_peer.shutdown().await.unwrap();
}

struct TwoPeerFixture {
    server: AukiPeer,
    client_peer: AukiPeer,
    endpoint: ComponentProtocolEndpoint,
    client: ComponentProtocolClient,
    route: Multiaddr,
    sensor: auki_components::Component,
    output: auki_components::ConfiguredObservable<f64>,
    capture: auki_components::BufferProductCapture<f64>,
}

impl TwoPeerFixture {
    async fn new() -> Self {
        Self::with_initial_observation(true).await
    }

    async fn with_initial_observation(publish_initial: bool) -> Self {
        let domain = Uuid::new_v4();
        let server_id = Identity::generate();
        let client_id = Identity::generate();
        let (server, _) = AukiPeer::start_external(
            server_id.clone(),
            authority(&server_id, domain),
            direct_config(),
        )
        .await
        .unwrap();
        let (client_peer, _) = AukiPeer::start_external(
            client_id.clone(),
            authority(&client_id, domain),
            direct_config(),
        )
        .await
        .unwrap();
        let route = server.listen_addresses()[0].clone();
        let runtime = ComponentRuntime::new(server.peer_id().to_string());
        let sensor = runtime
            .component(ComponentSpec::new("fixture-sensor").observable(gauge_contract("level")))
            .unwrap();
        let output = sensor
            .configured_observable::<f64>(ConfiguredObservableSpec::new(
                "level",
                "level-1",
                "fixture.clock",
                gauge_payload(),
            ))
            .unwrap();
        sensor.expose().unwrap();
        let capture = runtime
            .capture_buffer("fixture-history", &output, BufferLimits::entries(8), |_| 8)
            .unwrap();
        if publish_initial {
            output.publish(10, Arc::new(12.5)).unwrap();
        }
        let endpoint = ComponentProtocolEndpoint::mount(server.protocols(), runtime).unwrap();
        endpoint.export_product(&capture.product()).unwrap();
        let client = ComponentProtocolClient::new(client_peer.protocols());
        Self {
            server,
            client_peer,
            endpoint,
            client,
            route,
            sensor,
            output,
            capture,
        }
    }

    async fn mirror(&self) -> auki_component_protocol::RemoteProductMirror<f64> {
        self.client
            .mirror_product_exact::<f64>(
                self.server.peer_id(),
                self.route.clone(),
                self.capture.product().reference(),
                BufferLimits::entries(8),
                |_| 8,
            )
            .await
            .unwrap()
    }

    async fn shutdown(self) {
        self.endpoint.close().await.unwrap();
        self.client_peer.shutdown().await.unwrap();
        self.server.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn latest_sync_without_new_data_is_a_noop() {
    let fixture = TwoPeerFixture::new().await;
    let mut mirror = fixture.mirror().await;
    let result = mirror.sync_latest_once().await;
    assert_eq!(mirror.next_sequence(), 1);
    assert_eq!(mirror.product().buffer().range().entries, 1);
    assert_eq!(mirror.sync_latest_once().await.unwrap().accepted, 0);
    mirror.close();
    fixture.shutdown().await;
    assert!(
        result.is_ok(),
        "re-fetching an already mirrored latest value should be a no-op: {result:?}"
    );
    assert_eq!(result.unwrap().accepted, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn obsolete_observation_protocol_cannot_silently_omit_terminal_notices() {
    let fixture = TwoPeerFixture::new().await;
    let result = fixture
        .client_peer
        .protocols()
        .open_exact(
            fixture.server.peer_id(),
            fixture.route.clone(),
            "/aukilabs/components/observations/1.0.0",
        )
        .await;
    assert!(
        result.is_err(),
        "version 1 must not be negotiated by a version 2 endpoint"
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconfiguration_terminates_remote_following() {
    let fixture = TwoPeerFixture::new().await;
    let mut mirror = fixture.mirror().await;
    let mut remote_cursor = mirror.product().buffer().subscribe(CursorStart::Latest);
    let mut local_cursor = fixture
        .capture
        .product()
        .buffer()
        .subscribe(CursorStart::Latest);
    let replacement = fixture
        .sensor
        .replace_configured_observable(
            &fixture.output,
            ConfiguredObservableSpec::new("level", "level-2", "fixture.clock", gauge_payload()),
            20,
        )
        .unwrap();
    assert!(matches!(
        fixture.capture.end_notice().unwrap().reason,
        auki_components::ObservationEndReason::Reconfigured { .. }
    ));
    assert!(matches!(
        local_cursor.next_timeout(Duration::ZERO),
        auki_components::CursorRead::Closed
    ));
    replacement.replacement.publish(30, Arc::new(42.0)).unwrap();
    let sync = mirror.sync_once().await.unwrap();
    assert_eq!(
        sync.accepted, 0,
        "the old Product must not receive the replacement output"
    );
    assert_eq!(sync.end, fixture.capture.end_notice());
    assert_eq!(mirror.end_notice(), sync.end);
    assert_eq!(
        mirror.product().reference(),
        fixture.capture.product().reference()
    );
    assert_eq!(
        *mirror.product().latest_existing().unwrap().unwrap().payload,
        12.5
    );
    let history = fixture
        .client
        .observations_exact::<f64>(
            fixture.server.peer_id(),
            fixture.route.clone(),
            auki_component_protocol::ObservationRequest {
                product: mirror.product().reference(),
                selection: auki_component_protocol::ObservationSelection::FromSequence {
                    sequence: 0,
                    max_observations: 8,
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        history.observations.len(),
        1,
        "ended history is still fetchable"
    );
    assert_eq!(history.end, sync.end);
    let remote_state = remote_cursor.next_timeout(Duration::ZERO);
    mirror.close();
    fixture.shutdown().await;
    assert!(
        matches!(remote_state, auki_components::CursorRead::Closed),
        "local source ended with Reconfigured, but remote reader stayed open: {remote_state:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn source_eviction_reports_gap_and_retained_values() {
    let fixture = TwoPeerFixture::new().await;
    let mut mirror = fixture.mirror().await;
    for sequence in 1..=12 {
        fixture
            .output
            .publish(10 + sequence, Arc::new(sequence as f64))
            .unwrap();
    }
    let sync = mirror.sync_once().await.unwrap();
    let range = mirror.product().buffer().range();
    mirror.close();
    fixture.shutdown().await;
    assert_eq!(
        sync.gap,
        Some(auki_component_protocol::SourceGap {
            requested_sequence: 1,
            available_from: 5,
        })
    );
    assert_eq!(sync.accepted, 8);
    assert_eq!(range.first_sequence, Some(5));
    assert_eq!(range.last_sequence, Some(12));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_retention_does_not_advance_source_cursor() {
    let fixture = TwoPeerFixture::new().await;
    let mut mirror = fixture.mirror().await;
    let pending_sequence = mirror.next_sequence();
    mirror
        .product()
        .buffer()
        .set_limits(BufferLimits {
            max_entries: Some(8),
            max_bytes: Some(4),
            target_duration: None,
        })
        .unwrap();
    fixture.output.publish(20, Arc::new(18.0)).unwrap();
    let failed = mirror.sync_once().await;
    assert!(matches!(failed, Err(ComponentProtocolError::Import(_))));
    let after_failure = mirror.next_sequence();
    mirror
        .product()
        .buffer()
        .set_limits(BufferLimits::entries(8))
        .unwrap();
    let retry = mirror.sync_once().await.unwrap();
    mirror.close();
    fixture.shutdown().await;
    assert_eq!(
        after_failure, pending_sequence,
        "failed append advanced past data never retained; retry accepted {} observations",
        retry.accepted
    );
    assert_eq!(retry.accepted, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_notice_waits_for_the_last_page_and_readers_drain_before_closing() {
    let fixture = TwoPeerFixture::new().await;
    for sequence in 1..=5 {
        fixture
            .output
            .publish(10 + sequence, Arc::new(sequence as f64))
            .unwrap();
    }
    let end = fixture
        .output
        .end(
            30,
            auki_components::ObservationEndReason::Failed {
                reason: "fixture source stopped".to_owned(),
            },
        )
        .unwrap();
    let mut mirror = fixture.mirror().await.with_batch_size(2).unwrap();
    // Bootstrap imports only sequence zero, not the entire ended history.
    assert_eq!(mirror.next_sequence(), 1);
    assert_eq!(mirror.end_notice(), None);
    let mut cursor = mirror
        .product()
        .buffer()
        .subscribe(CursorStart::FromSequence(0));
    for next_sequence in [3, 5] {
        let sync = mirror.sync_once().await.unwrap();
        assert_eq!(sync.accepted, 2);
        assert_eq!(sync.next_sequence, next_sequence);
        assert_eq!(sync.end, None);
    }
    let sync = mirror.sync_once().await.unwrap();
    assert_eq!(sync.accepted, 1);
    assert_eq!(sync.end, Some(end.clone()));
    for sequence in 0..=5 {
        let auki_components::CursorRead::Item(observation) = cursor.next_timeout(Duration::ZERO)
        else {
            panic!("closed before draining sequence {sequence}");
        };
        assert_eq!(observation.sequence, sequence);
    }
    assert!(matches!(
        cursor.next_timeout(Duration::ZERO),
        auki_components::CursorRead::Closed
    ));
    fixture.shutdown().await;
    // Once terminal, either sync API is a local no-op, even with the peer gone.
    assert_eq!(mirror.sync_once().await.unwrap().end, Some(end.clone()));
    assert_eq!(mirror.sync_latest_once().await.unwrap().end, Some(end));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partial_retention_failure_retries_only_the_unaccepted_suffix_before_ending() {
    let fixture = TwoPeerFixture::new().await;
    let mut mirror = fixture
        .client
        .mirror_product_exact::<f64>(
            fixture.server.peer_id(),
            fixture.route.clone(),
            fixture.capture.product().reference(),
            BufferLimits {
                max_entries: Some(8),
                max_bytes: Some(8),
                target_duration: None,
            },
            |value| if *value > 20.0 { 16 } else { 8 },
        )
        .await
        .unwrap();
    fixture.output.publish(20, Arc::new(18.0)).unwrap();
    fixture.output.publish(30, Arc::new(32.0)).unwrap();
    let end = fixture
        .output
        .end(
            40,
            auki_components::ObservationEndReason::Failed {
                reason: "fixture source stopped".to_owned(),
            },
        )
        .unwrap();
    assert!(matches!(
        mirror.sync_once().await,
        Err(ComponentProtocolError::Import(_))
    ));
    assert_eq!(mirror.next_sequence(), 2, "only sequence one was accepted");
    assert_eq!(mirror.product().buffer().range().last_sequence, Some(1));
    assert_eq!(
        mirror.end_notice(),
        None,
        "the rejected tail still needs a retry"
    );
    mirror
        .product()
        .buffer()
        .set_limits(BufferLimits::entries(8))
        .unwrap();
    let retry = mirror.sync_once().await.unwrap();
    assert_eq!(retry.accepted, 1);
    assert_eq!(retry.next_sequence, 3);
    assert_eq!(retry.end, Some(end));
    assert_eq!(mirror.product().buffer().range().entries, 2);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn empty_ended_product_closes_during_bootstrap_without_inventing_data() {
    let fixture = TwoPeerFixture::with_initial_observation(false).await;
    let end = fixture
        .output
        .end(
            20,
            auki_components::ObservationEndReason::Failed {
                reason: "no samples available".to_owned(),
            },
        )
        .unwrap();
    let mirror = fixture.mirror().await;
    assert_eq!(mirror.end_notice(), Some(end));
    assert_eq!(mirror.next_sequence(), 0);
    assert_eq!(mirror.product().buffer().range().entries, 0);
    let mut cursor = mirror.product().buffer().subscribe(CursorStart::Latest);
    assert!(matches!(
        cursor.next_timeout(Duration::ZERO),
        auki_components::CursorRead::Closed
    ));
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn latest_sync_reports_skipped_history_then_accepts_the_terminal_notice_once() {
    let fixture = TwoPeerFixture::new().await;
    let mut mirror = fixture.mirror().await;
    for sequence in 1..=3 {
        fixture
            .output
            .publish(10 + sequence, Arc::new(sequence as f64))
            .unwrap();
    }
    let sync = mirror.sync_latest_once().await.unwrap();
    assert_eq!(sync.accepted, 1);
    assert_eq!(
        sync.gap,
        Some(auki_component_protocol::SourceGap {
            requested_sequence: 1,
            available_from: 3,
        })
    );
    let end = fixture
        .output
        .end(
            20,
            auki_components::ObservationEndReason::Failed {
                reason: "fixture source stopped".to_owned(),
            },
        )
        .unwrap();
    let ended = mirror.sync_latest_once().await.unwrap();
    assert_eq!(
        ended.accepted, 0,
        "the final observation was already retained"
    );
    assert_eq!(ended.gap, None);
    assert_eq!(ended.end, Some(end));
    assert_eq!(mirror.next_sequence(), 4);
    fixture.shutdown().await;
}
