#![cfg(not(target_arch = "wasm32"))]

use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use auki_component_protocol::{
    CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint, ComponentProtocolError,
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
            server_route,
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

    mirror.close();
    endpoint.close().await.unwrap();
    client_peer.shutdown().await.unwrap();
    server_peer.shutdown().await.unwrap();
}
