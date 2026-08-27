use std::{
    future::pending,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use auki_p2p::{
    ApplicationProtocol, DdsTokenVerifier, DdsVerificationKeys, Identity, Multiaddr, Node,
    P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE, P2PAccessClaims, PeerId,
    SessionRequirements, SignedApplicationMetadata, SignedP2pCredential,
};
use auki_protocols::{
    blob::v1::{ID as BLOBS_V0_1_0, MAX_BLOB_CHUNK_BYTES},
    registry::{
        v2::{
            ID as REGISTRIES_V0_2_0, RegistryRequest as RegistryRequestV2,
            RegistryResponse as RegistryResponseV2,
            read_registry_request as read_registry_request_v2,
            write_registry_response as write_registry_response_v2,
        },
        v3::{
            ID as REGISTRIES_V0_3_0, MAX_REGISTRIES_FRAME_BYTES, RegistriesProtocolError,
            RegistryEntryEnvelope, RegistryKind, RegistryListEntry, RegistryRequest,
            RegistryResponse, read_registry_request,
        },
    },
};
use auki_registry::{
    DeviceModelBody, DeviceModelFormat, DeviceModelRegistryEntry, FrameRegistryEntry,
};
use futures::AsyncWriteExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use tokio::sync::oneshot;
use uuid::Uuid;

use super::{
    AuthenticatedDomain, AuthenticatedDomainConfig, AuthenticatedDomainServicesConfig,
    DomainStatus,
    peers::{KnownPeer, KnownPeerEvent, KnownPeerSubscription},
    registries::RegistriesError,
};
use crate::served_protocols::ServedProtocols;

const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

const TEST_BOUND: Duration = Duration::from_secs(45);

fn identity(seed: u8) -> Identity {
    Identity::from_ed25519_seed(&[seed; 32])
}

fn keys() -> DdsVerificationKeys {
    DdsVerificationKeys::new(0, TEST_DDS_PUBLIC_KEY.to_vec(), None)
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time must follow the Unix epoch")
        .as_secs()
}

fn credential(peer_id: PeerId, domain_id: Uuid, issued_at: u64) -> SignedP2pCredential {
    let claims = P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.into(),
        iss: P2P_TOKEN_ISSUER.into(),
        aud: vec![P2P_TOKEN_AUDIENCE.into()],
        sub: Uuid::new_v4().to_string(),
        peer_type: None,
        peer_id: peer_id.to_string(),
        domain_ids: vec![domain_id.to_string()],
        scopes: vec!["registry-and-blob-test".into()],
        application: Some(SignedApplicationMetadata {
            name: "p10-test-application".into(),
            version: "1.0.0".into(),
        }),
        iat: issued_at,
        nbf: None,
        exp: issued_at + P2P_TOKEN_TTL.as_secs(),
    };
    let token = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(TEST_DDS_PRIVATE_KEY).unwrap(),
    )
    .unwrap();
    SignedP2pCredential::new(token).unwrap()
}

async fn join_domain(config: AuthenticatedDomainConfig, issued_at: u64) -> AuthenticatedDomain {
    let credential = credential(config.peer_id(), config.domain_id(), issued_at);
    let services = AuthenticatedDomainServicesConfig::default().with_served_protocols(
        ServedProtocols::none()
            .with_registries_v2()
            .with_registries_v3()
            .with_blobs_v1(),
    );
    AuthenticatedDomain::join_with_services(config, keys(), credential, services)
        .await
        .unwrap()
}

fn tcp_listener() -> Multiaddr {
    Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()
}

fn tcp_port(address: &Multiaddr) -> u16 {
    address
        .iter()
        .find_map(|protocol| match protocol {
            auki_p2p::Protocol::Tcp(port) => Some(port),
            _ => None,
        })
        .expect("test listener must contain a TCP port")
}

struct StorageFixture {
    frame: FrameRegistryEntry,
    frame_hash: String,
    device_model: DeviceModelRegistryEntry,
    device_model_hash: String,
    blob: Vec<u8>,
    blob_sha256: String,
}

fn install_storage(root: &std::path::Path, owner: PeerId, label: &str) -> StorageFixture {
    let frame_id = format!("{label}-frame");
    let frame = FrameRegistryEntry::ros_body(owner.to_string(), frame_id);
    let frame_hash = auki_registry::write_frame(root, &frame)
        .unwrap()
        .hash()
        .to_owned();

    let blob_len = MAX_BLOB_CHUNK_BYTES as usize + 32_777;
    let blob = (0..blob_len)
        .map(|index| (index.wrapping_add(label.len()) % 251) as u8)
        .collect::<Vec<_>>();
    let blob_sha256 = auki_registry::put_blob(root, &blob).unwrap();
    let device_model = DeviceModelRegistryEntry {
        peer_id: owner.to_string(),
        device_model_id: format!("{label}-model"),
        body: DeviceModelBody {
            model_id: format!("{label}-robot"),
            format: DeviceModelFormat::Urdf {
                urdf_sha256: blob_sha256.clone(),
                meshes: vec![],
            },
            root_convention: Some("ros".into()),
        },
    };
    let device_model_hash = auki_registry::write_device_model(root, &device_model)
        .unwrap()
        .hash()
        .to_owned();

    StorageFixture {
        frame,
        frame_hash,
        device_model,
        device_model_hash,
        blob,
        blob_sha256,
    }
}

async fn appeared(subscription: &mut KnownPeerSubscription, expected_peer: PeerId) -> KnownPeer {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let KnownPeerEvent::Appeared(peer) = subscription.recv().await.unwrap()
                && peer.peer_id() == expected_peer
            {
                return peer;
            }
        }
    })
    .await
    .expect("authenticated peer must appear")
}

#[tokio::test(flavor = "multi_thread")]
async fn p10_registry_and_multiround_blob_round_trip_bidirectionally() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();

        let a_identity = identity(91);
        let a_peer = a_identity.peer_id();
        let a = join_domain(
            AuthenticatedDomainConfig::new(domain_id, a_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap(),
            issued_at,
        )
        .await;
        let a_address = a.listen_addresses()[0].clone();
        let a_port = tcp_port(&a_address);

        let b_identity = identity(92);
        let b_peer = b_identity.peer_id();
        let b = join_domain(
            AuthenticatedDomainConfig::new(domain_id, b_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap(),
            issued_at,
        )
        .await;
        let b_address = b.listen_addresses()[0].clone();
        let b_port = tcp_port(&b_address);

        let a_root = tempfile::tempdir().unwrap();
        let b_root = tempfile::tempdir().unwrap();
        let a_storage = install_storage(a_root.path(), a_peer, "alpha");
        let b_storage = install_storage(b_root.path(), b_peer, "bravo");
        a.set_registry_app_root(a_root.path()).unwrap();
        b.set_registry_app_root(b_root.path()).unwrap();
        a.routes().replace(b_peer, [b_address]).unwrap();
        b.routes().replace(a_peer, [a_address]).unwrap();

        let mut a_events = a.peers().subscribe();
        let mut b_events = b.peers().subscribe();

        let frame_response = a
            .registries()
            .request_v2(
                b_peer,
                RegistryRequestV2 {
                    kind: RegistryKind::Frame,
                    id: b_storage.frame.frame_id.clone(),
                    hash: b_storage.frame_hash.clone(),
                },
            )
            .await
            .unwrap();
        let frame_envelope = frame_response.entry.expect("v0.2 Frame must exist");
        assert_eq!(frame_envelope.kind, RegistryKind::Frame);
        assert_eq!(frame_envelope.id, b_storage.frame.frame_id);
        assert_eq!(frame_envelope.hash, b_storage.frame_hash);
        assert_eq!(
            serde_json::from_str::<FrameRegistryEntry>(&frame_envelope.canonical_json).unwrap(),
            b_storage.frame
        );

        let observed_b = appeared(&mut a_events, b_peer).await;
        let observed_a = appeared(&mut b_events, a_peer).await;
        assert_eq!(
            observed_b.application().unwrap().name,
            "p10-test-application"
        );
        assert_eq!(
            observed_a.application().unwrap().name,
            "p10-test-application"
        );
        assert!(observed_b.authenticated_until().timestamp() > unix_time() as i64);
        assert!(observed_a.authenticated_until().timestamp() > unix_time() as i64);

        assert_eq!(
            a.registries()
                .list(b_peer, RegistryKind::DeviceModel)
                .await
                .unwrap(),
            vec![RegistryListEntry {
                id: b_storage.device_model.device_model_id.clone(),
                hash: b_storage.device_model_hash.clone(),
            }]
        );
        assert_eq!(
            a.registries()
                .fetch_device_model(
                    b_peer,
                    b_storage.device_model.device_model_id.clone(),
                    b_storage.device_model_hash.clone(),
                )
                .await
                .unwrap(),
            b_storage.device_model
        );

        assert_eq!(
            b.registries()
                .fetch_frame(
                    a_peer,
                    a_storage.frame.frame_id.clone(),
                    a_storage.frame_hash.clone(),
                )
                .await
                .unwrap(),
            a_storage.frame
        );
        assert_eq!(
            a.blobs()
                .fetch(b_peer, b_storage.blob_sha256.clone())
                .await
                .unwrap(),
            b_storage.blob
        );
        assert_eq!(
            b.blobs()
                .fetch(a_peer, a_storage.blob_sha256.clone())
                .await
                .unwrap(),
            a_storage.blob
        );
        assert!(a.storage.source_reads() >= 2);
        assert!(b.storage.source_reads() >= 4);

        let a_registries = a.registries();
        let b_blobs = b.blobs();
        b.leave().await.unwrap();
        a.leave().await.unwrap();
        assert!(
            a_registries
                .request_v3(b_peer, RegistryRequest::list(RegistryKind::DeviceModel))
                .await
                .is_err()
        );
        assert!(b_blobs.fetch(a_peer, a_storage.blob_sha256).await.is_err());
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, a_port)).unwrap();
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, b_port)).unwrap();
    })
    .await
    .expect("the authenticated Registry/blob family must remain bounded");
}

async fn assert_storage_handlers_reject(
    domain: &AuthenticatedDomain,
    expected_peer: PeerId,
    frame: &FrameRegistryEntry,
    frame_hash: &str,
    blob_sha256: &str,
) {
    assert!(
        domain
            .registries()
            .request_v2(
                expected_peer,
                RegistryRequestV2 {
                    kind: RegistryKind::Frame,
                    id: frame.frame_id.clone(),
                    hash: frame_hash.into(),
                },
            )
            .await
            .is_err()
    );
    assert!(
        domain
            .registries()
            .fetch_frame(expected_peer, frame.frame_id.clone(), frame_hash)
            .await
            .is_err()
    );
    assert!(
        domain
            .registries()
            .request_v3(
                expected_peer,
                RegistryRequest::list(RegistryKind::DeviceModel),
            )
            .await
            .is_err()
    );
    assert!(
        domain
            .blobs()
            .fetch(expected_peer, blob_sha256)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn p10_d09_authorization_matrix_reads_no_storage_then_recovers() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let server_identity = identity(101);
        let server_peer = server_identity.peer_id();
        let server = join_domain(
            AuthenticatedDomainConfig::new(domain_id, server_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap(),
            issued_at,
        )
        .await;
        let server_address = server.listen_addresses()[0].clone();
        let server_root = tempfile::tempdir().unwrap();
        let fixture = install_storage(server_root.path(), server_peer, "server");
        server.set_registry_app_root(server_root.path()).unwrap();
        assert_eq!(server.storage.source_reads(), 0);

        let wrong_peer_client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(102)),
            issued_at,
        )
        .await;
        let wrong_peer = identity(103).peer_id();
        wrong_peer_client
            .routes()
            .replace(wrong_peer, [server_address.clone()])
            .unwrap();
        assert_storage_handlers_reject(
            &wrong_peer_client,
            wrong_peer,
            &fixture.frame,
            &fixture.frame_hash,
            &fixture.blob_sha256,
        )
        .await;
        assert_eq!(server.storage.source_reads(), 0);

        let wrong_domain_client = join_domain(
            AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(104))
                .with_peer_routes(server_peer, [server_address.clone()])
                .unwrap(),
            issued_at,
        )
        .await;
        assert_storage_handlers_reject(
            &wrong_domain_client,
            server_peer,
            &fixture.frame,
            &fixture.frame_hash,
            &fixture.blob_sha256,
        )
        .await;
        assert_eq!(server.storage.source_reads(), 0);

        let expired_client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(105))
                .with_peer_routes(server_peer, [server_address.clone()])
                .unwrap(),
            issued_at - P2P_TOKEN_TTL.as_secs() + 2,
        )
        .await;
        let mut expired_status = expired_client.subscribe_status();
        while *expired_status.borrow_and_update() != DomainStatus::CredentialUnavailable {
            expired_status.changed().await.unwrap();
        }
        assert_storage_handlers_reject(
            &expired_client,
            server_peer,
            &fixture.frame,
            &fixture.frame_hash,
            &fixture.blob_sha256,
        )
        .await;
        assert_eq!(server.storage.source_reads(), 0);

        let anonymous = Node::start(
            identity(106),
            DdsTokenVerifier::from_keys(keys()).unwrap(),
            [],
        )
        .unwrap();
        for protocol_id in [REGISTRIES_V0_2_0, REGISTRIES_V0_3_0, BLOBS_V0_1_0] {
            let opened = anonymous
                .open(
                    server_peer,
                    vec![server_address.clone()],
                    ApplicationProtocol::new(protocol_id).unwrap(),
                    SessionRequirements::new(domain_id.to_string())
                        .unwrap()
                        .with_expected_remote_peer_id(server_peer),
                )
                .await;
            assert!(opened.is_err(), "anonymous {protocol_id} stream must fail");
        }
        assert_eq!(server.storage.source_reads(), 0);
        assert_eq!(server.peers().peer_count(), 0);

        let valid = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(107))
                .with_peer_routes(server_peer, [server_address])
                .unwrap(),
            issued_at,
        )
        .await;
        let v2 = valid
            .registries()
            .request_v2(
                server_peer,
                RegistryRequestV2 {
                    kind: RegistryKind::Frame,
                    id: fixture.frame.frame_id.clone(),
                    hash: fixture.frame_hash.clone(),
                },
            )
            .await
            .unwrap();
        assert!(v2.entry.is_some());
        assert!(matches!(
            valid
                .registries()
                .request_v3(
                    server_peer,
                    RegistryRequest::list(RegistryKind::DeviceModel),
                )
                .await
                .unwrap(),
            RegistryResponse::List { ref entries }
                if entries == &[RegistryListEntry {
                    id: fixture.device_model.device_model_id.clone(),
                    hash: fixture.device_model_hash.clone(),
                }]
        ));
        assert_eq!(
            valid
                .blobs()
                .fetch(server_peer, fixture.blob_sha256.clone())
                .await
                .unwrap(),
            fixture.blob
        );
        assert!(server.storage.source_reads() >= 4);
        assert_eq!(server.peers().peer_count(), 1);

        valid.leave().await.unwrap();
        anonymous.shutdown().await.unwrap();
        expired_client.leave().await.unwrap();
        wrong_domain_client.leave().await.unwrap();
        wrong_peer_client.leave().await.unwrap();
        server.leave().await.unwrap();
    })
    .await
    .expect("the P10 D09 authorization matrix must remain bounded");
}

#[tokio::test(flavor = "multi_thread")]
async fn p10_registry_v3_falls_back_only_to_authenticated_v2() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let server = Node::start(
            identity(108),
            DdsTokenVerifier::from_keys(keys()).unwrap(),
            [tcp_listener()],
        )
        .unwrap();
        let server_peer = server.peer_id();
        server
            .authority()
            .install_credential(credential(server_peer, domain_id, issued_at))
            .await
            .unwrap();
        let server_address = server.wait_for_listeners().await.unwrap()[0].clone();

        let mut incoming = server
            .accept(
                ApplicationProtocol::new(REGISTRIES_V0_2_0).unwrap(),
                SessionRequirements::new(domain_id.to_string()).unwrap(),
            )
            .unwrap();
        let frame = FrameRegistryEntry::ros_body(server_peer.to_string(), "fallback-frame");
        let canonical_json = String::from_utf8(frame.canonical_bytes()).unwrap();
        let frame_hash = auki_hash::hash_jcs_bytes(canonical_json.as_bytes());
        let expected_request = RegistryRequestV2 {
            kind: RegistryKind::Frame,
            id: frame.frame_id.clone(),
            hash: frame_hash.clone(),
        };
        let response = RegistryResponseV2 {
            entry: Some(RegistryEntryEnvelope {
                kind: RegistryKind::Frame,
                id: frame.frame_id.clone(),
                hash: frame_hash.clone(),
                canonical_json,
            }),
        };
        let server_task = tokio::spawn(async move {
            let mut stream = tokio::time::timeout(Duration::from_secs(5), incoming.accept())
                .await
                .expect("authenticated v0.2 fallback did not arrive")
                .expect("v0.2 listener ended")
                .expect("v0.2 fallback authentication failed");
            assert_eq!(
                read_registry_request_v2(&mut stream).await.unwrap(),
                expected_request
            );
            write_registry_response_v2(&mut stream, &response)
                .await
                .unwrap();
        });

        let client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(109))
                .with_peer_routes(server_peer, [server_address])
                .unwrap(),
            issued_at,
        )
        .await;
        assert_eq!(
            client
                .registries()
                .fetch_frame(server_peer, frame.frame_id.clone(), frame_hash)
                .await
                .unwrap(),
            frame
        );

        server_task.await.unwrap();
        client.leave().await.unwrap();
        server.shutdown().await.unwrap();
    })
    .await
    .expect("authenticated Registry version fallback must remain bounded");
}

#[tokio::test(flavor = "multi_thread")]
async fn p10_registry_fetch_is_cancelled_by_ordered_leave() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let server = Node::start(
            identity(110),
            DdsTokenVerifier::from_keys(keys()).unwrap(),
            [tcp_listener()],
        )
        .unwrap();
        let server_peer = server.peer_id();
        server
            .authority()
            .install_credential(credential(server_peer, domain_id, issued_at))
            .await
            .unwrap();
        let server_address = server.wait_for_listeners().await.unwrap()[0].clone();
        let mut incoming = server
            .accept(
                ApplicationProtocol::new(REGISTRIES_V0_3_0).unwrap(),
                SessionRequirements::new(domain_id.to_string()).unwrap(),
            )
            .unwrap();
        let (request_seen_tx, request_seen_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            let mut stream = incoming.accept().await.unwrap().unwrap();
            read_registry_request(&mut stream).await.unwrap();
            request_seen_tx.send(()).unwrap();
            pending::<()>().await;
        });

        let client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(111))
                .with_peer_routes(server_peer, [server_address])
                .unwrap(),
            issued_at,
        )
        .await;
        let registries = client.registries();
        let fetch = tokio::spawn(async move {
            registries
                .fetch_frame(server_peer, "stalled", "a".repeat(32))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), request_seen_rx)
            .await
            .expect("Registry request never reached the authenticated responder")
            .unwrap();

        client.leave().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), fetch)
            .await
            .expect("Registry fetch outlived ordered leave")
            .unwrap();
        assert!(matches!(result, Err(RegistriesError::Stopped)));

        server_task.abort();
        assert!(server_task.await.unwrap_err().is_cancelled());
        server.shutdown().await.unwrap();
    })
    .await
    .expect("Registry cancellation and teardown must remain bounded");
}

#[tokio::test(flavor = "multi_thread")]
async fn p10_oversized_v3_response_never_falls_back_to_v2() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let server = Node::start(
            identity(112),
            DdsTokenVerifier::from_keys(keys()).unwrap(),
            [tcp_listener()],
        )
        .unwrap();
        let server_peer = server.peer_id();
        server
            .authority()
            .install_credential(credential(server_peer, domain_id, issued_at))
            .await
            .unwrap();
        let server_address = server.wait_for_listeners().await.unwrap()[0].clone();
        let requirements = SessionRequirements::new(domain_id.to_string()).unwrap();
        let mut incoming_v3 = server
            .accept(
                ApplicationProtocol::new(REGISTRIES_V0_3_0).unwrap(),
                requirements.clone(),
            )
            .unwrap();
        let mut incoming_v2 = server
            .accept(
                ApplicationProtocol::new(REGISTRIES_V0_2_0).unwrap(),
                requirements,
            )
            .unwrap();
        let server_task = tokio::spawn(async move {
            let mut stream = incoming_v3.accept().await.unwrap().unwrap();
            read_registry_request(&mut stream).await.unwrap();
            stream
                .write_all(&(MAX_REGISTRIES_FRAME_BYTES + 1).to_be_bytes())
                .await
                .unwrap();
            stream.flush().await.unwrap();
        });

        let client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(113))
                .with_peer_routes(server_peer, [server_address])
                .unwrap(),
            issued_at,
        )
        .await;
        let error = client
            .registries()
            .fetch_frame(server_peer, "oversized", "b".repeat(32))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            RegistriesError::Codec(RegistriesProtocolError::FrameTooLarge {
                actual,
                max,
            }) if actual == u64::from(MAX_REGISTRIES_FRAME_BYTES + 1)
                && max == u64::from(MAX_REGISTRIES_FRAME_BYTES)
        ));
        server_task.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), incoming_v2.accept())
                .await
                .is_err(),
            "codec failure incorrectly triggered the authenticated v0.2 fallback"
        );

        client.leave().await.unwrap();
        server.shutdown().await.unwrap();
    })
    .await
    .expect("oversized Registry response handling must remain bounded");
}
