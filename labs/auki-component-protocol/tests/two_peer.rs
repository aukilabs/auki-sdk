#![cfg(not(target_arch = "wasm32"))]

use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use auki_component_protocol::{
    CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint, ComponentProtocolError,
    ObservationStart, RemoteMirrorStart, RemoteObservationEvent, RemoteProductSubscription,
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
    AukiPeer, AukiPeerConfig, DdsVerificationKeys, ExternalAuthorityControl,
    ExternalAuthorityReplaceOutcome, ExternalAuthorityUpdate, Identity, Multiaddr,
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
    authority_at(identity, domain_id, Uuid::new_v4(), unix_now() - 60)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn authority_at(
    identity: &Identity,
    domain_id: Uuid,
    subject: Uuid,
    issued_at: u64,
) -> ExternalAuthorityUpdate {
    let expires_at = issued_at + 30 * 60;
    let claims = P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.to_owned(),
        iss: P2P_TOKEN_ISSUER.to_owned(),
        aud: vec![P2P_TOKEN_AUDIENCE.to_owned()],
        sub: subject.to_string(),
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
    domain: Uuid,
    server_identity: Identity,
    client_identity: Identity,
    server_subject: Uuid,
    client_subject: Uuid,
    server_authority: ExternalAuthorityControl,
    client_authority: ExternalAuthorityControl,
    runtime: ComponentRuntime,
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
        let server_subject = Uuid::new_v4();
        let client_subject = Uuid::new_v4();
        // Leave room for a genuinely newer credential without sleeping or
        // issuing future-dated claims. Each peer keeps its authenticated subject.
        let issued_at = unix_now() - 60;
        let (server, server_authority) = AukiPeer::start_external(
            server_id.clone(),
            authority_at(&server_id, domain, server_subject, issued_at),
            direct_config(),
        )
        .await
        .unwrap();
        let (client_peer, client_authority) = AukiPeer::start_external(
            client_id.clone(),
            authority_at(&client_id, domain, client_subject, issued_at),
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
        let endpoint =
            ComponentProtocolEndpoint::mount(server.protocols(), runtime.clone()).unwrap();
        endpoint.export_product(&capture.product()).unwrap();
        let client = ComponentProtocolClient::new(client_peer.protocols());
        Self {
            server,
            client_peer,
            domain,
            server_identity: server_id,
            client_identity: client_id,
            server_subject,
            client_subject,
            server_authority,
            client_authority,
            runtime,
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

    async fn subscribe(&self, start: ObservationStart) -> RemoteProductSubscription<f64> {
        self.client
            .subscribe_product_exact(
                self.server.peer_id(),
                self.route.clone(),
                self.capture.product().reference(),
                start,
                BufferLimits::entries(16),
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

async fn subscription_event(
    subscription: &mut RemoteProductSubscription<f64>,
) -> RemoteObservationEvent<f64> {
    tokio::time::timeout(Duration::from_secs(3), subscription.next())
        .await
        .expect("subscription event timed out")
        .unwrap()
        .expect("subscription already ended")
}

async fn wait_for_subscriptions(endpoint: &ComponentProtocolEndpoint, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while endpoint.active_subscriptions() != count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("subscription handler cleanup timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn credential_replacement_preserves_active_subscription_and_accepts_new_subscribers() {
    let fixture = TwoPeerFixture::new().await;
    let mut subscription = fixture.subscribe(ObservationStart::LatestExisting).await;
    let reference = subscription.product().reference();
    assert!(matches!(
        subscription_event(&mut subscription).await,
        RemoteObservationEvent::Observation(value) if value.sequence == 0
    ));

    // Replace both authorities while the relationship is idle, not a Component
    // contract. This must not end or rebind the Product subscription.
    let renewed_at = unix_now();
    for (control, identity, subject) in [
        (
            &fixture.server_authority,
            &fixture.server_identity,
            fixture.server_subject,
        ),
        (
            &fixture.client_authority,
            &fixture.client_identity,
            fixture.client_subject,
        ),
    ] {
        let outcome = tokio::time::timeout(
            Duration::from_secs(3),
            control.replace(authority_at(identity, fixture.domain, subject, renewed_at)),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(
            outcome,
            ExternalAuthorityReplaceOutcome::Replaced { .. }
        ));
    }
    fixture.output.publish(20, Arc::new(18.0)).unwrap();
    assert!(matches!(
        subscription_event(&mut subscription).await,
        RemoteObservationEvent::Observation(value) if value.sequence == 1
    ));
    assert_eq!(subscription.product().reference(), reference);
    assert!(subscription.end_notice().is_none());
    assert!(!subscription.is_closed());

    // A new stream must authenticate using the replacement credentials too.
    let mut second = fixture.subscribe(ObservationStart::NewOnly).await;
    wait_for_subscriptions(&fixture.endpoint, 2).await;
    let renewed_expiry = Utc.timestamp_opt((renewed_at + 30 * 60) as i64, 0).unwrap();
    for (observer, remote_id) in [
        (&fixture.server, fixture.client_peer.peer_id()),
        (&fixture.client_peer, fixture.server.peer_id()),
    ] {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !observer
                .known_peers()
                .snapshot()
                .peers()
                .iter()
                .any(|peer| {
                    peer.peer_id() == remote_id && peer.authenticated_until() == renewed_expiry
                })
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("new stream did not authenticate the replacement credential");
    }
    fixture.output.publish(30, Arc::new(24.0)).unwrap();
    for reader in [&mut subscription, &mut second] {
        assert!(matches!(
            subscription_event(reader).await,
            RemoteObservationEvent::Observation(value) if value.sequence == 2
        ));
    }
    subscription.close().await.unwrap();
    wait_for_subscriptions(&fixture.endpoint, 1).await;
    fixture.output.publish(40, Arc::new(30.0)).unwrap();
    assert!(matches!(
        subscription_event(&mut second).await,
        RemoteObservationEvent::Observation(value) if value.sequence == 3
    ));
    second.close().await.unwrap();
    wait_for_subscriptions(&fixture.endpoint, 0).await;
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_authority_replacement_does_not_rebind_or_disrupt_the_subscription() {
    let fixture = TwoPeerFixture::new().await;
    let mut subscription = fixture.subscribe(ObservationStart::NewOnly).await;
    let reference = subscription.product().reference();
    // Validly signed test credentials, but for a different Domain or Peer.
    let wrong_domain = authority(&fixture.server_identity, Uuid::new_v4());
    let wrong_peer = authority(&Identity::generate(), fixture.domain);
    for invalid in [wrong_domain, wrong_peer] {
        assert!(
            tokio::time::timeout(
                Duration::from_secs(3),
                fixture.server_authority.replace(invalid),
            )
            .await
            .unwrap()
            .is_err()
        );
    }
    fixture.output.publish(20, Arc::new(18.0)).unwrap();
    assert!(matches!(
        subscription_event(&mut subscription).await,
        RemoteObservationEvent::Observation(value) if value.sequence == 1
    ));
    assert_eq!(subscription.product().reference(), reference);
    assert!(subscription.end_notice().is_none());
    let mut second = fixture.subscribe(ObservationStart::LatestExisting).await;
    assert!(matches!(
        subscription_event(&mut second).await,
        RemoteObservationEvent::Observation(value) if value.sequence == 1
    ));
    subscription.close().await.unwrap();
    second.close().await.unwrap();
    fixture.shutdown().await;
}

/// Transparent loopback TCP forwarding: the SDK still performs its real
/// encrypted transport and signed peer handshake. Dropping the sockets injects
/// a network loss without unexporting a Product or ending its producer.
struct CuttableConnection {
    route: Multiaddr,
    task: tokio::task::JoinHandle<()>,
}

impl CuttableConnection {
    async fn start(target: &Multiaddr) -> Self {
        let target_port: u16 = target
            .to_string()
            .strip_prefix("/ip4/127.0.0.1/tcp/")
            .expect("fixture must use loopback TCP")
            .parse()
            .unwrap();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let route = format!(
            "/ip4/127.0.0.1/tcp/{}",
            listener.local_addr().unwrap().port()
        )
        .parse()
        .unwrap();
        let task = tokio::spawn(async move {
            let (mut incoming, _) = listener.accept().await.unwrap();
            drop(listener); // One connection only; no hidden reconnection path.
            let mut outgoing =
                tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, target_port))
                    .await
                    .unwrap();
            let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
        });
        Self { route, task }
    }

    async fn cut(&mut self) {
        self.task.abort();
        let result = tokio::time::timeout(Duration::from_secs(3), &mut self.task)
            .await
            .expect("TCP forwarding did not stop");
        assert!(result.is_err_and(|error| error.is_cancelled()));
    }
}

impl Drop for CuttableConnection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_loss_requires_explicit_resubscription_and_reports_evicted_history() {
    let fixture = TwoPeerFixture::new().await;
    let mut connection = CuttableConnection::start(&fixture.route).await;
    let mut subscription = fixture
        .client
        .subscribe_product_exact::<f64>(
            fixture.server.peer_id(),
            connection.route.clone(),
            fixture.capture.product().reference(),
            ObservationStart::LatestExisting,
            BufferLimits::entries(16),
            |_| 8,
        )
        .await
        .unwrap();
    assert!(matches!(
        subscription_event(&mut subscription).await,
        RemoteObservationEvent::Observation(value) if value.sequence == 0
    ));
    let old_product = subscription.product().clone();
    let mut old_reader = old_product.buffer().subscribe(CursorStart::FromSequence(0));

    connection.cut().await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), subscription.next())
            .await
            .unwrap(),
        Err(ComponentProtocolError::Wire(_))
    ));
    assert!(subscription.is_closed());
    assert!(
        subscription.end_notice().is_none(),
        "network loss is not a producer end"
    );
    assert_eq!(subscription.next_sequence(), 1);
    wait_for_subscriptions(&fixture.endpoint, 0).await;
    assert!(matches!(
        old_reader.next_timeout(Duration::ZERO),
        auki_components::CursorRead::Item(_)
    ));
    assert!(matches!(
        old_reader.next_timeout(Duration::ZERO),
        auki_components::CursorRead::Closed
    ));

    // Capture continues while disconnected and evicts the missing prefix.
    for sequence in 1..=12 {
        fixture
            .output
            .publish(10 + sequence, Arc::new(sequence as f64))
            .unwrap();
    }
    assert!(fixture.capture.end_notice().is_none());
    assert!(subscription.next().await.unwrap().is_none());
    assert_eq!(old_product.buffer().range().entries, 1);

    // The host deliberately selects a working route and resumes from its last
    // accepted source sequence. This creates a new local Buffer, not a silent
    // reconnection or splice into the closed imported Product.
    let mut resumed = fixture
        .subscribe(ObservationStart::FromSequence {
            sequence: subscription.next_sequence(),
        })
        .await;
    assert!(matches!(
        subscription_event(&mut resumed).await,
        RemoteObservationEvent::Gap(auki_component_protocol::SourceGap {
            requested_sequence: 1,
            available_from: 5,
        })
    ));
    for expected in 5..=12 {
        assert!(matches!(
            subscription_event(&mut resumed).await,
            RemoteObservationEvent::Observation(value) if value.sequence == expected
        ));
    }
    assert_eq!(resumed.product().reference(), old_product.reference());
    assert_eq!(resumed.product().buffer().range().entries, 8);
    assert_eq!(old_product.buffer().range().entries, 1);
    assert_eq!(resumed.next_sequence(), 13);
    resumed.close().await.unwrap();
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn either_peer_shutdown_ends_idle_subscription_without_a_fictitious_source_notice() {
    for shutdown_server in [true, false] {
        let fixture = TwoPeerFixture::new().await;
        let mut subscription = fixture.subscribe(ObservationStart::NewOnly).await;
        let mut reader = subscription
            .product()
            .buffer()
            .subscribe(CursorStart::Latest);
        let (stopped, remaining) = if shutdown_server {
            (fixture.server, fixture.client_peer)
        } else {
            (fixture.client_peer, fixture.server)
        };
        // Deliberately test peer-owned cleanup without closing the endpoint
        // first (normal host shutdown still closes endpoints before the peer).
        tokio::time::timeout(Duration::from_secs(3), stopped.shutdown())
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(3), subscription.next())
                .await
                .unwrap()
                .is_err()
        );
        assert!(subscription.is_closed());
        assert!(subscription.end_notice().is_none());
        assert!(matches!(
            reader.next_timeout(Duration::ZERO),
            auki_components::CursorRead::Closed
        ));
        assert!(subscription.next().await.unwrap().is_none());
        wait_for_subscriptions(&fixture.endpoint, 0).await;
        fixture.output.publish(20, Arc::new(18.0)).unwrap();
        assert!(fixture.capture.end_notice().is_none());
        fixture.endpoint.close().await.unwrap();
        remaining.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscription_pushes_new_data_after_cancelled_wait_and_ends_without_migration() {
    let fixture = TwoPeerFixture::new().await;
    let mut subscription = fixture.subscribe(ObservationStart::NewOnly).await;
    let product_reference = subscription.product().reference();
    let runtime = ComponentRuntime::new(fixture.client_peer.peer_id().to_string());
    let consumer = runtime
        .component(
            ComponentSpec::new("derived").product_input(ProductInputContract {
                name: "level".into(),
                form: ProductForm::Buffer,
                datatype: "float64".into(),
                schema: "test.level/v1".into(),
                exposure: Exposure::Cluster,
            }),
        )
        .unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let delivered = Arc::clone(&received);
    let input = InputPort::<Observation<f64>>::new("derived.level", move |envelope| {
        delivered.lock().unwrap().push(envelope.payload.sequence);
    });
    let _binding = consumer
        .configured_buffer_input(
            "level",
            subscription.product(),
            CursorStart::FromSequence(subscription.next_sequence()),
            &input,
        )
        .unwrap();
    let mut cursor = subscription
        .product()
        .buffer()
        .subscribe(CursorStart::FromSequence(subscription.next_sequence()));
    // Cancelling next() must not discard partially read framing or cancel the relationship.
    assert!(
        tokio::time::timeout(Duration::from_millis(20), subscription.next())
            .await
            .is_err()
    );
    assert!(!subscription.is_closed());
    fixture.output.publish(20, Arc::new(18.0)).unwrap();
    let RemoteObservationEvent::Observation(observation) =
        subscription_event(&mut subscription).await
    else {
        panic!("missing observation");
    };
    assert_eq!(observation.sequence, 1);
    assert_eq!(observation.timestamp_ns, 20);
    assert_eq!(*observation.payload, 18.0);
    assert_eq!(observation.output, *fixture.output.reference());
    tokio::time::timeout(Duration::from_secs(3), async {
        while received.lock().unwrap().as_slice() != [1] {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("ordinary Component input did not receive pushed data");
    assert!(matches!(
        cursor.next_timeout(Duration::ZERO),
        auki_components::CursorRead::Item(_)
    ));

    let transition = fixture
        .sensor
        .replace_configured_observable(
            &fixture.output,
            ConfiguredObservableSpec::new("level", "level-2", "fixture.clock", gauge_payload()),
            30,
        )
        .unwrap();
    let replacement_capture = fixture
        .runtime
        .capture_buffer(
            "replacement-history",
            &transition.replacement,
            BufferLimits::entries(8),
            |_| 8,
        )
        .unwrap();
    fixture
        .endpoint
        .export_product(&replacement_capture.product())
        .unwrap();
    transition.replacement.publish(40, Arc::new(99.0)).unwrap();
    let RemoteObservationEvent::Closed(Some(end)) = subscription_event(&mut subscription).await
    else {
        panic!("missing source end");
    };
    assert_eq!(end, transition.previous_end);
    assert_eq!(subscription.end_notice(), Some(end));
    assert_eq!(subscription.product().reference(), product_reference);
    assert_eq!(subscription.product().buffer().range().entries, 1);
    assert!(matches!(
        cursor.next_timeout(Duration::ZERO),
        auki_components::CursorRead::Closed
    ));
    assert!(subscription.next().await.unwrap().is_none());
    wait_for_subscriptions(&fixture.endpoint, 0).await;

    // Reconfiguration requires inspecting the Catalog and deliberately selecting
    // the replacement Product. The old handle and local input stay ended.
    let CatalogResponse::Snapshot { snapshot } = fixture
        .client
        .catalog_exact(fixture.server.peer_id(), fixture.route.clone(), None)
        .await
        .unwrap()
    else {
        panic!("missing Catalog snapshot");
    };
    let replacement_reference = replacement_capture.product().reference();
    let advertised = snapshot
        .products
        .iter()
        .find(|entry| entry.manifest.product_id == replacement_reference.product_id)
        .expect("replacement Product must be advertised");
    assert_eq!(
        advertised.manifest_hash,
        replacement_reference.manifest_hash
    );
    assert_ne!(replacement_reference, product_reference);
    let mut replacement_subscription = fixture
        .client
        .subscribe_product_exact::<f64>(
            fixture.server.peer_id(),
            fixture.route.clone(),
            replacement_reference,
            ObservationStart::FromSequence { sequence: 0 },
            BufferLimits::entries(8),
            |_| 8,
        )
        .await
        .unwrap();
    assert!(matches!(
        subscription_event(&mut replacement_subscription).await,
        RemoteObservationEvent::Observation(value)
            if value.output == *transition.replacement.reference() && *value.payload == 99.0
    ));
    assert!(subscription.next().await.unwrap().is_none());
    assert_eq!(subscription.product().buffer().range().entries, 1);
    replacement_subscription.close().await.unwrap();
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscription_replays_retained_tail_with_gap_and_closes_after_the_final_observation() {
    let fixture = TwoPeerFixture::new().await;
    for sequence in 1..=12 {
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
                reason: "stopped".into(),
            },
        )
        .unwrap();
    let mut subscription = fixture
        .subscribe(ObservationStart::FromSequence { sequence: 0 })
        .await;
    assert!(matches!(
        subscription_event(&mut subscription).await,
        RemoteObservationEvent::Gap(auki_component_protocol::SourceGap {
            requested_sequence: 0,
            available_from: 5
        })
    ));
    for sequence in 5..=12 {
        let RemoteObservationEvent::Observation(observation) =
            subscription_event(&mut subscription).await
        else {
            panic!("missing retained observation");
        };
        assert_eq!(observation.sequence, sequence);
    }
    assert!(
        matches!(subscription_event(&mut subscription).await, RemoteObservationEvent::Closed(Some(received)) if received == end)
    );
    assert_eq!(subscription.product().buffer().range().entries, 8);
    assert_eq!(subscription.next_sequence(), 13);
    let mut latest = fixture.subscribe(ObservationStart::LatestExisting).await;
    assert!(
        matches!(subscription_event(&mut latest).await, RemoteObservationEvent::Observation(observation) if observation.sequence == 12)
    );
    assert!(
        matches!(subscription_event(&mut latest).await, RemoteObservationEvent::Closed(Some(received)) if received == end)
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscription_retention_failure_retries_the_same_payload_before_reading_its_end() {
    let fixture = TwoPeerFixture::new().await;
    let mut subscription = fixture.subscribe(ObservationStart::NewOnly).await;
    subscription
        .product()
        .buffer()
        .set_limits(BufferLimits {
            max_entries: Some(8),
            max_bytes: Some(4),
            target_duration: None,
        })
        .unwrap();
    fixture.output.publish(20, Arc::new(18.0)).unwrap();
    let end = fixture
        .output
        .end(
            30,
            auki_components::ObservationEndReason::Failed {
                reason: "stopped".into(),
            },
        )
        .unwrap();
    for _ in 0..2 {
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(3), subscription.next())
                .await
                .unwrap(),
            Err(ComponentProtocolError::Import(_))
        ));
        assert_eq!(subscription.next_sequence(), 1);
        assert_eq!(subscription.end_notice(), None);
        assert!(!subscription.is_closed());
    }
    subscription
        .product()
        .buffer()
        .set_limits(BufferLimits::entries(8))
        .unwrap();
    assert!(
        matches!(subscription_event(&mut subscription).await, RemoteObservationEvent::Observation(observation) if observation.sequence == 1)
    );
    assert!(
        matches!(subscription_event(&mut subscription).await, RemoteObservationEvent::Closed(Some(received)) if received == end)
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_or_closing_idle_subscription_releases_only_its_handler_and_readers() {
    let fixture = TwoPeerFixture::new().await;
    let first = fixture.subscribe(ObservationStart::NewOnly).await;
    let first_product = first.product().clone();
    let mut second = fixture.subscribe(ObservationStart::NewOnly).await;
    wait_for_subscriptions(&fixture.endpoint, 2).await;
    drop(first);
    wait_for_subscriptions(&fixture.endpoint, 1).await;
    assert!(matches!(
        first_product
            .buffer()
            .subscribe(CursorStart::Latest)
            .next_timeout(Duration::ZERO),
        auki_components::CursorRead::Closed
    ));
    assert_eq!(
        first_product.end_notice(),
        None,
        "cancellation is not a producer failure"
    );
    fixture.output.publish(20, Arc::new(18.0)).unwrap();
    assert!(matches!(
        subscription_event(&mut second).await,
        RemoteObservationEvent::Observation(_)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), second.next())
            .await
            .is_err()
    );
    second.close().await.unwrap();
    second.close().await.unwrap();
    wait_for_subscriptions(&fixture.endpoint, 0).await;
    assert!(second.next().await.unwrap().is_none());
    assert!(fixture.capture.end_notice().is_none());
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unexport_interrupts_idle_subscription_without_ending_the_source_or_auto_reconnecting() {
    let fixture = TwoPeerFixture::new().await;
    let mut subscription = fixture.subscribe(ObservationStart::NewOnly).await;
    assert!(
        fixture
            .endpoint
            .unexport_product(&fixture.capture.product().manifest.product_id)
    );
    let result = tokio::time::timeout(Duration::from_secs(3), subscription.next())
        .await
        .unwrap();
    assert!(result.is_err());
    assert!(subscription.is_closed());
    assert!(subscription.end_notice().is_none());
    wait_for_subscriptions(&fixture.endpoint, 0).await;
    fixture
        .endpoint
        .export_product(&fixture.capture.product())
        .unwrap();
    fixture.output.publish(20, Arc::new(18.0)).unwrap();
    assert!(subscription.next().await.unwrap().is_none());
    assert_eq!(subscription.product().buffer().range().entries, 0);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn endpoint_shutdown_interrupts_idle_subscription_and_closes_local_readers() {
    let fixture = TwoPeerFixture::new().await;
    let mut subscription = fixture.subscribe(ObservationStart::NewOnly).await;
    let mut cursor = subscription
        .product()
        .buffer()
        .subscribe(CursorStart::Latest);
    tokio::time::timeout(Duration::from_secs(3), fixture.endpoint.close())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), subscription.next())
            .await
            .unwrap()
            .is_err()
    );
    assert!(matches!(
        cursor.next_timeout(Duration::ZERO),
        auki_components::CursorRead::Closed
    ));
    fixture.client_peer.shutdown().await.unwrap();
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn empty_capture_closure_is_distinct_from_source_failure() {
    let fixture = TwoPeerFixture::with_initial_observation(false).await;
    let mut subscription = fixture.subscribe(ObservationStart::NewOnly).await;
    fixture.capture.cancel();
    assert!(matches!(
        subscription_event(&mut subscription).await,
        RemoteObservationEvent::Closed(None)
    ));
    assert!(subscription.is_closed());
    assert!(subscription.end_notice().is_none());
    assert_eq!(subscription.product().buffer().range().entries, 0);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscription_rejects_unexported_stale_or_wrong_peer_products() {
    let fixture = TwoPeerFixture::new().await;
    let product = fixture.capture.product().reference();
    let mut stale = product.clone();
    stale.manifest_hash = "not-the-product-hash".into();
    let result = fixture
        .client
        .subscribe_product_exact::<f64>(
            fixture.server.peer_id(),
            fixture.route.clone(),
            stale,
            ObservationStart::NewOnly,
            BufferLimits::entries(8),
            |_| 8,
        )
        .await;
    assert!(
        matches!(result, Err(ComponentProtocolError::RemoteRejected { code, .. }) if code == "product_not_current")
    );
    let wrong_peer = Identity::generate().peer_id();
    let mut claimed = product.clone();
    claimed.peer_id = wrong_peer.to_string();
    let result = fixture
        .client
        .subscribe_product_exact::<f64>(
            wrong_peer,
            fixture.route.clone(),
            claimed,
            ObservationStart::NewOnly,
            BufferLimits::entries(8),
            |_| 8,
        )
        .await;
    assert!(matches!(result, Err(ComponentProtocolError::Sdk(_))));
    fixture.endpoint.unexport_product(&product.product_id);
    let result = fixture
        .client
        .subscribe_product_exact::<f64>(
            fixture.server.peer_id(),
            fixture.route.clone(),
            product,
            ObservationStart::NewOnly,
            BufferLimits::entries(8),
            |_| 8,
        )
        .await;
    assert!(
        matches!(result, Err(ComponentProtocolError::RemoteRejected { code, .. }) if code == "unknown_product")
    );
    wait_for_subscriptions(&fixture.endpoint, 0).await;
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_subscription_wait_mid_frame_preserves_framing_for_the_next_wait() {
    use futures::{AsyncReadExt, AsyncWriteExt};
    let fixture = TwoPeerFixture::new().await;
    let product = fixture.capture.product();
    fixture.endpoint.close().await.unwrap();
    let (partial_tx, partial_rx) = async_channel::bounded::<()>(1);
    let (resume_tx, resume_rx) = async_channel::bounded::<()>(1);
    let header = serde_json::json!({
        "status": "accepted", "product": product.manifest,
        "product_manifest_hash": product.manifest_hash, "producer": product.producer,
        "next_sequence": 1,
    });
    let event = serde_json::json!({
        "event": "observation", "record": {
            "output": product.manifest.producer, "sequence": 1, "timestamp_ns": 20,
            "payload_encoding": "application/json", "payload_bytes": 4,
        },
    });
    let registration = fixture
        .server
        .protocols()
        .register(
            auki_sdk::AukiProtocolSpec::new(
                auki_component_protocol::OBSERVATION_STREAM_PROTOCOL_ID,
                1,
                1024,
            )
            .unwrap(),
            move |mut stream| {
                let header = header.clone();
                let event = event.clone();
                let partial_tx = partial_tx.clone();
                let resume_rx = resume_rx.clone();
                async move {
                    let mut len = [0; 4];
                    stream.read_exact(&mut len).await.unwrap();
                    let mut request = vec![0; u32::from_be_bytes(len) as usize];
                    stream.read_exact(&mut request).await.unwrap();
                    let header = serde_json::to_vec(&header).unwrap();
                    stream
                        .write_all(&(header.len() as u32).to_be_bytes())
                        .await
                        .unwrap();
                    stream.write_all(&header).await.unwrap();
                    let event = serde_json::to_vec(&event).unwrap();
                    let len = (event.len() as u32).to_be_bytes();
                    stream.write_all(&len[..2]).await.unwrap();
                    stream.flush().await.unwrap();
                    partial_tx.send(()).await.unwrap();
                    resume_rx.recv().await.unwrap();
                    stream.write_all(&len[2..]).await.unwrap();
                    stream.write_all(&event).await.unwrap();
                    stream.write_all(&4_u32.to_be_bytes()).await.unwrap();
                    stream.write_all(b"18.0").await.unwrap();
                    let end = br#"{"event":"closed","end":null}"#;
                    stream
                        .write_all(&(end.len() as u32).to_be_bytes())
                        .await
                        .unwrap();
                    stream.write_all(end).await.unwrap();
                    stream.close().await.unwrap();
                }
            },
        )
        .unwrap();
    let mut subscription = fixture
        .client
        .subscribe_product_exact::<f64>(
            fixture.server.peer_id(),
            fixture.route,
            product.reference(),
            ObservationStart::NewOnly,
            BufferLimits::entries(8),
            |_| 8,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), partial_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), subscription.next())
            .await
            .is_err()
    );
    resume_tx.send(()).await.unwrap();
    assert!(
        matches!(subscription_event(&mut subscription).await, RemoteObservationEvent::Observation(observation) if observation.sequence == 1 && *observation.payload == 18.0)
    );
    assert!(matches!(
        subscription_event(&mut subscription).await,
        RemoteObservationEvent::Closed(None)
    ));
    registration.close().await.unwrap();
    fixture.client_peer.shutdown().await.unwrap();
    fixture.server.shutdown().await.unwrap();
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
