use std::{
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use auki_datatypes::map::{
    ColorEvidenceDelta, MapUpdate, SemanticDelta, VoxelChunkUpdate, VoxelDelta,
};
use auki_network::{
    message_codec::MAX_MESSAGE_FRAME_BYTES,
    protocol_ids::{MESSAGE_V0_1_0, STREAM_V0_2_0},
    resources_v3_protocol::{
        MessageChannelResource, ResourceEntry, ResourceVariant, ResourcesRequest,
    },
    stream_protocol::{
        CameraFrame, MAX_FRAME_BYTES, ReadFrom, StreamManifest, StreamRequest, end_reason,
    },
    stream_runtime::{StreamDispatch, StreamError, StreamItem, StreamProvider},
};
use auki_p2p::{
    ApplicationProtocol, DdsTokenVerifier, DdsVerificationKeys, Identity, Multiaddr, Node,
    P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE, P2PAccessClaims, PeerId,
    SessionRequirements, SignedApplicationMetadata, SignedP2pCredential,
};
use auki_registry::RegistryRef;
use futures::{AsyncWriteExt, StreamExt, stream};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use parking_lot::Mutex;
use uuid::Uuid;

use super::{
    AuthenticatedDomain, AuthenticatedDomainConfig, AuthenticatedDomainServicesConfig,
    DomainStatus,
    messages::{OpenMessageChannelError, SendMessageError},
    peers::{KnownPeer, KnownPeerEvent, KnownPeerSubscription},
    streams::StreamsError,
};

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
const OPERATION_BOUND: Duration = Duration::from_secs(5);
const MESSAGE_RESOURCE_ID: &str = "commands/operator";
const CAMERA_RESOURCE_ID: &str = "camera/front";
const MAP_RESOURCE_ID: &str = "map/local";

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
        scopes: vec!["message-and-stream-test".into()],
        application: Some(SignedApplicationMetadata {
            name: "p11-test-application".into(),
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

async fn join_domain(
    config: AuthenticatedDomainConfig,
    issued_at: u64,
    services: AuthenticatedDomainServicesConfig,
) -> AuthenticatedDomain {
    let credential = credential(config.peer_id(), config.domain_id(), issued_at);
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

fn message_resource(owner_peer_id: PeerId) -> MessageChannelResource {
    MessageChannelResource {
        owner_peer_id,
        resource_id: MESSAGE_RESOURCE_ID.into(),
        clock: RegistryRef {
            peer_id: owner_peer_id.to_string(),
            id: "session/monotonic".into(),
            hash: "p11-clock-hash".into(),
        },
    }
}

fn camera_manifest(producer: PeerId) -> StreamManifest {
    StreamManifest {
        resource_id: CAMERA_RESOURCE_ID.into(),
        sensor_id: "camera/front".into(),
        sensor_hash: "camera-sensor-hash".into(),
        clock_peer_id: producer.to_string(),
        clock_id: "session/monotonic".into(),
        clock_hash: "p11-clock-hash".into(),
        frame_id: "camera/front/optical".into(),
        frame_hash: "camera-frame-hash".into(),
        payload: "auki.camera.CameraFrame".into(),
        expected_rate_hz: 30,
        ..Default::default()
    }
}

fn map_manifest(producer: PeerId) -> StreamManifest {
    StreamManifest {
        resource_id: MAP_RESOURCE_ID.into(),
        sensor_id: "map/local".into(),
        sensor_hash: "map-sensor-hash".into(),
        clock_peer_id: producer.to_string(),
        clock_id: "session/monotonic".into(),
        clock_hash: "p11-clock-hash".into(),
        frame_id: "map".into(),
        frame_hash: "map-frame-hash".into(),
        payload: "auki.map.MapUpdate".into(),
        map_peer_id: producer.to_string(),
        map_id: "warehouse".into(),
        map_hash: "map-registry-hash".into(),
        expected_rate_hz: 5,
        ..Default::default()
    }
}

fn camera_payloads() -> Vec<CameraFrame> {
    vec![
        CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![0x01, 0x02, 0x03],
        },
        CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![0x04, 0x05, 0x06, 0x07],
        },
    ]
}

fn map_payload() -> MapUpdate {
    MapUpdate {
        voxel_chunks: vec![VoxelChunkUpdate {
            chunk_x: -2,
            chunk_y: 3,
            chunk_z: 1,
            voxels: vec![VoxelDelta {
                x: 4,
                y: 5,
                z: 6,
                occupancy_delta: 0.75,
                semantics: vec![SemanticDelta {
                    class_id: 17,
                    evidence_delta: 0.5,
                }],
                color: Some(ColorEvidenceDelta {
                    red_sum_delta: 0.1,
                    green_sum_delta: 0.2,
                    blue_sum_delta: 0.3,
                    weight_delta: 0.4,
                }),
            }],
        }],
        checkpoint: None,
    }
}

fn stream_request(source: PeerId, resource_id: &str) -> StreamRequest {
    StreamRequest {
        source_peer_id: source.to_string(),
        resource_id: resource_id.into(),
        from: ReadFrom::FromStart,
    }
}

fn finite_stream_provider(
    producer: PeerId,
    seen: Arc<Mutex<Vec<(PeerId, StreamRequest)>>>,
) -> StreamProvider {
    Arc::new(move |requester, request| {
        seen.lock().push((requester, request.clone()));
        match request.resource_id.as_str() {
            CAMERA_RESOURCE_ID => StreamDispatch::AcceptCamera {
                manifest: camera_manifest(producer),
                source: Box::pin(stream::iter(
                    camera_payloads().into_iter().zip([1_000_i64, 2_000]).map(
                        |(payload, timestamp_ns)| {
                            Ok(StreamItem {
                                timestamp_ns,
                                payload,
                            })
                        },
                    ),
                )),
            },
            MAP_RESOURCE_ID => StreamDispatch::AcceptMap {
                manifest: map_manifest(producer),
                source: Box::pin(stream::iter([Ok(StreamItem {
                    timestamp_ns: 3_000,
                    payload: map_payload(),
                })])),
            },
            _ => StreamDispatch::Decline {
                reason: auki_network::stream_protocol::DeclineReason::sensor_not_found(),
            },
        }
    })
}

async fn appeared(subscription: &mut KnownPeerSubscription, expected_peer: PeerId) -> KnownPeer {
    tokio::time::timeout(OPERATION_BOUND, async {
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

fn assert_source_ended(error: StreamError) {
    assert!(matches!(
        error,
        StreamError::EndOfStream { reason }
            if matches!(reason.kind, Some(end_reason::Kind::SourceEnded(_)))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn p11_message_and_typed_streams_share_one_authenticated_domain_node() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();

        let a_identity = identity(121);
        let a_peer = a_identity.peer_id();
        let a = join_domain(
            AuthenticatedDomainConfig::new(domain_id, a_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap(),
            issued_at,
            AuthenticatedDomainServicesConfig::default(),
        )
        .await;
        let a_address = a.listen_addresses()[0].clone();
        let a_port = tcp_port(&a_address);

        let b_identity = identity(122);
        let b_peer = b_identity.peer_id();
        let resource = message_resource(b_peer);
        let seen_requests = Arc::new(Mutex::new(Vec::new()));
        let services = AuthenticatedDomainServicesConfig::default()
            .with_message_channel(resource.clone(), 4)
            .with_stream_provider(finite_stream_provider(b_peer, Arc::clone(&seen_requests)));
        let mut b = join_domain(
            AuthenticatedDomainConfig::new(domain_id, b_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap(),
            issued_at,
            services,
        )
        .await;
        let b_address = b.listen_addresses()[0].clone();
        let b_port = tcp_port(&b_address);
        let mut message_receiver = b
            .take_message_channel(MESSAGE_RESOURCE_ID)
            .expect("DomainBuilder declaration must own the receiver");

        a.routes().replace(b_peer, [b_address.clone()]).unwrap();
        b.routes().replace(a_peer, [a_address.clone()]).unwrap();
        let mut a_events = a.peers().subscribe();
        let mut b_events = b.peers().subscribe();

        let catalog = a
            .resources_v3()
            .fetch(
                b_peer,
                ResourcesRequest {
                    variants: vec![ResourceVariant::MessageChannel],
                },
            )
            .await
            .unwrap();
        assert_eq!(
            catalog.resources,
            vec![ResourceEntry::MessageChannel(resource.clone())]
        );

        let sender = a.messages().open(b_peer, &resource).await.unwrap();
        sender
            .send("operator.command.v1", 41_000, b"turn-left".to_vec())
            .await
            .expect("send resolves only after the exact sequence ACK");
        let inbound = tokio::time::timeout(OPERATION_BOUND, message_receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inbound.sender, a_peer);
        assert_eq!(inbound.message.r#type, "operator.command.v1");
        assert_eq!(inbound.message.timestamp_ns, 41_000);
        assert_eq!(inbound.message.payload, b"turn-left");

        let camera_manifest = camera_manifest(b_peer);
        let expected_camera_payloads = camera_payloads();
        let mut camera = a
            .streams()
            .open::<CameraFrame>(b_peer, stream_request(b_peer, CAMERA_RESOURCE_ID))
            .await
            .unwrap();
        assert_eq!(camera.manifest, camera_manifest);
        for (sequence, (timestamp_ns, payload)) in [1_000_i64, 2_000]
            .into_iter()
            .zip(expected_camera_payloads)
            .enumerate()
        {
            let entry = camera.entries.next().await.unwrap().unwrap();
            assert_eq!(entry.seq, sequence as u64);
            assert_eq!(entry.timestamp_ns, timestamp_ns);
            assert_eq!(entry.payload, payload);
        }
        assert_source_ended(camera.entries.next().await.unwrap().unwrap_err());
        assert!(camera.entries.next().await.is_none());

        let expected_map = map_payload();
        let mut map = a
            .streams()
            .open::<MapUpdate>(b_peer, stream_request(b_peer, MAP_RESOURCE_ID))
            .await
            .unwrap();
        assert_eq!(map.manifest, map_manifest(b_peer));
        let entry = map.entries.next().await.unwrap().unwrap();
        assert_eq!(entry.seq, 0);
        assert_eq!(entry.timestamp_ns, 3_000);
        assert_eq!(entry.payload, expected_map);
        assert_source_ended(map.entries.next().await.unwrap().unwrap_err());
        assert!(map.entries.next().await.is_none());

        assert_eq!(
            *seen_requests.lock(),
            vec![
                (a_peer, stream_request(b_peer, CAMERA_RESOURCE_ID)),
                (a_peer, stream_request(b_peer, MAP_RESOURCE_ID)),
            ]
        );
        let observed_b = appeared(&mut a_events, b_peer).await;
        let observed_a = appeared(&mut b_events, a_peer).await;
        assert_eq!(
            observed_b.application().unwrap().name,
            "p11-test-application"
        );
        assert_eq!(
            observed_a.application().unwrap().name,
            "p11-test-application"
        );

        drop((sender, camera, map));
        b.leave().await.unwrap();
        assert!(message_receiver.recv().await.is_none());
        a.leave().await.unwrap();
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, a_port)).unwrap();
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, b_port)).unwrap();
    })
    .await
    .expect("the authenticated P11 end-to-end proof must remain bounded");
}

async fn assert_application_untouched(
    receiver: &mut super::messages::MessageChannelRegistration,
    provider_calls: &AtomicUsize,
) {
    assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .is_err(),
        "an unauthorized attempt reached the message receiver"
    );
}

async fn assert_both_rejected(
    client: &AuthenticatedDomain,
    expected_peer: PeerId,
    resource: &MessageChannelResource,
    stream_source: PeerId,
) {
    assert!(
        tokio::time::timeout(
            OPERATION_BOUND,
            client.messages().open(expected_peer, resource),
        )
        .await
        .expect("message rejection must remain bounded")
        .is_err()
    );
    assert!(
        tokio::time::timeout(
            OPERATION_BOUND,
            client.streams().open::<CameraFrame>(
                expected_peer,
                stream_request(stream_source, CAMERA_RESOURCE_ID),
            ),
        )
        .await
        .expect("stream rejection must remain bounded")
        .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn p11_d09_authorization_matrix_reaches_no_message_receiver_or_stream_provider_then_recovers()
{
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let server_identity = identity(131);
        let server_peer = server_identity.peer_id();
        let resource = message_resource(server_peer);
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&provider_calls);
        let provider: StreamProvider = Arc::new(move |_requester, _request| {
            calls.fetch_add(1, Ordering::SeqCst);
            StreamDispatch::AcceptCamera {
                manifest: camera_manifest(server_peer),
                source: Box::pin(stream::iter([Ok(StreamItem {
                    timestamp_ns: 9_000,
                    payload: camera_payloads().remove(0),
                })])),
            }
        });
        let services = AuthenticatedDomainServicesConfig::default()
            .with_message_channel(resource.clone(), 2)
            .with_stream_provider(provider);
        let mut server = join_domain(
            AuthenticatedDomainConfig::new(domain_id, server_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap(),
            issued_at,
            services,
        )
        .await;
        let server_address = server.listen_addresses()[0].clone();
        let mut receiver = server.take_message_channel(MESSAGE_RESOURCE_ID).unwrap();

        let wrong_peer_client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(132)),
            issued_at,
            AuthenticatedDomainServicesConfig::default(),
        )
        .await;
        let wrong_peer = identity(133).peer_id();
        wrong_peer_client
            .routes()
            .replace(wrong_peer, [server_address.clone()])
            .unwrap();
        let wrong_peer_resource = message_resource(wrong_peer);
        assert_both_rejected(
            &wrong_peer_client,
            wrong_peer,
            &wrong_peer_resource,
            server_peer,
        )
        .await;
        assert!(matches!(
            wrong_peer_client
                .messages()
                .open(wrong_peer, &resource)
                .await,
            Err(OpenMessageChannelError::OwnerMismatch { .. })
        ));
        assert_application_untouched(&mut receiver, &provider_calls).await;

        let wrong_domain_client = join_domain(
            AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(134))
                .with_peer_routes(server_peer, [server_address.clone()])
                .unwrap(),
            issued_at,
            AuthenticatedDomainServicesConfig::default(),
        )
        .await;
        assert_both_rejected(&wrong_domain_client, server_peer, &resource, server_peer).await;
        assert_application_untouched(&mut receiver, &provider_calls).await;

        let expired_client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(135))
                .with_peer_routes(server_peer, [server_address.clone()])
                .unwrap(),
            issued_at - P2P_TOKEN_TTL.as_secs() + 2,
            AuthenticatedDomainServicesConfig::default(),
        )
        .await;
        let mut expired_status = expired_client.subscribe_status();
        while *expired_status.borrow_and_update() != DomainStatus::CredentialUnavailable {
            expired_status.changed().await.unwrap();
        }
        assert_both_rejected(&expired_client, server_peer, &resource, server_peer).await;
        assert_application_untouched(&mut receiver, &provider_calls).await;

        let anonymous = Node::start(
            identity(136),
            DdsTokenVerifier::from_keys(keys()).unwrap(),
            [],
        )
        .unwrap();
        for protocol_id in [MESSAGE_V0_1_0, STREAM_V0_2_0] {
            let opened = tokio::time::timeout(
                OPERATION_BOUND,
                anonymous.open(
                    server_peer,
                    vec![server_address.clone()],
                    ApplicationProtocol::new(protocol_id).unwrap(),
                    SessionRequirements::new(domain_id.to_string())
                        .unwrap()
                        .with_expected_remote_peer_id(server_peer),
                ),
            )
            .await
            .expect("anonymous rejection must remain bounded");
            assert!(opened.is_err(), "anonymous {protocol_id} stream must fail");
        }
        assert_application_untouched(&mut receiver, &provider_calls).await;
        assert_eq!(server.peers().peer_count(), 0);

        let valid_identity = identity(137);
        let valid_peer = valid_identity.peer_id();
        let valid = join_domain(
            AuthenticatedDomainConfig::new(domain_id, valid_identity)
                .with_peer_routes(server_peer, [server_address])
                .unwrap(),
            issued_at,
            AuthenticatedDomainServicesConfig::default(),
        )
        .await;
        let sender = valid.messages().open(server_peer, &resource).await.unwrap();
        sender
            .send("recovery.v1", 99_000, b"authorized".to_vec())
            .await
            .unwrap();
        let delivered = receiver.recv().await.unwrap();
        assert_eq!(delivered.sender, valid_peer);
        assert_eq!(delivered.message.r#type, "recovery.v1");
        assert_eq!(delivered.message.timestamp_ns, 99_000);
        assert_eq!(delivered.message.payload, b"authorized");

        let mut recovered = valid
            .streams()
            .open::<CameraFrame>(server_peer, stream_request(server_peer, CAMERA_RESOURCE_ID))
            .await
            .unwrap();
        assert_eq!(recovered.manifest, camera_manifest(server_peer));
        let entry = recovered.entries.next().await.unwrap().unwrap();
        assert_eq!(entry.seq, 0);
        assert_eq!(entry.timestamp_ns, 9_000);
        assert_eq!(entry.payload, camera_payloads().remove(0));
        assert_source_ended(recovered.entries.next().await.unwrap().unwrap_err());
        assert!(recovered.entries.next().await.is_none());
        assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
        assert_eq!(server.peers().peer_count(), 1);

        drop((sender, recovered));
        valid.leave().await.unwrap();
        anonymous.shutdown().await.unwrap();
        expired_client.leave().await.unwrap();
        wrong_domain_client.leave().await.unwrap();
        wrong_peer_client.leave().await.unwrap();
        server.leave().await.unwrap();
    })
    .await
    .expect("the P11 D09 authorization matrix must remain bounded");
}

#[tokio::test(flavor = "multi_thread")]
async fn p11_oversized_authenticated_frames_reach_neither_application_surface() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let server_identity = identity(138);
        let server_peer = server_identity.peer_id();
        let resource = message_resource(server_peer);
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&provider_calls);
        let provider: StreamProvider = Arc::new(move |_requester, _request| {
            calls.fetch_add(1, Ordering::SeqCst);
            StreamDispatch::Decline {
                reason: auki_network::stream_protocol::DeclineReason::sensor_not_found(),
            }
        });
        let services = AuthenticatedDomainServicesConfig::default()
            .with_message_channel(resource, 1)
            .with_stream_provider(provider);
        let mut server = join_domain(
            AuthenticatedDomainConfig::new(domain_id, server_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap(),
            issued_at,
            services,
        )
        .await;
        let server_address = server.listen_addresses()[0].clone();
        let mut receiver = server.take_message_channel(MESSAGE_RESOURCE_ID).unwrap();

        let attacker_identity = identity(139);
        let attacker_peer = attacker_identity.peer_id();
        let attacker = Node::start(
            attacker_identity,
            DdsTokenVerifier::from_keys(keys()).unwrap(),
            [],
        )
        .unwrap();
        attacker
            .authority()
            .install_credential(credential(attacker_peer, domain_id, issued_at))
            .await
            .unwrap();
        let requirements = || {
            SessionRequirements::new(domain_id.to_string())
                .unwrap()
                .with_expected_remote_peer_id(server_peer)
        };

        let mut message = attacker
            .open(
                server_peer,
                vec![server_address.clone()],
                ApplicationProtocol::new(MESSAGE_V0_1_0).unwrap(),
                requirements(),
            )
            .await
            .unwrap();
        message
            .write_all(&(MAX_MESSAGE_FRAME_BYTES + 1).to_be_bytes())
            .await
            .unwrap();
        message.flush().await.unwrap();
        drop(message);

        let mut typed = attacker
            .open(
                server_peer,
                vec![server_address],
                ApplicationProtocol::new(STREAM_V0_2_0).unwrap(),
                requirements(),
            )
            .await
            .unwrap();
        typed
            .write_all(&(MAX_FRAME_BYTES + 1).to_be_bytes())
            .await
            .unwrap();
        typed.flush().await.unwrap();
        drop(typed);

        assert_application_untouched(&mut receiver, &provider_calls).await;

        attacker.shutdown().await.unwrap();
        server.leave().await.unwrap();
    })
    .await
    .expect("oversized P11 frames must fail before application dispatch");
}

#[tokio::test(flavor = "multi_thread")]
async fn p11_ordered_leave_cancels_a_full_message_queue_and_stalled_typed_reader() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let server_identity = identity(141);
        let server_peer = server_identity.peer_id();
        let resource = message_resource(server_peer);
        let provider: StreamProvider =
            Arc::new(move |_requester, _request| StreamDispatch::AcceptCamera {
                manifest: camera_manifest(server_peer),
                source: Box::pin(stream::pending()),
            });
        let services = AuthenticatedDomainServicesConfig::default()
            .with_message_channel(resource.clone(), 1)
            .with_stream_provider(provider);
        let mut server = join_domain(
            AuthenticatedDomainConfig::new(domain_id, server_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap(),
            issued_at,
            services,
        )
        .await;
        let server_address = server.listen_addresses()[0].clone();
        let server_port = tcp_port(&server_address);
        let mut registration = server.take_message_channel(MESSAGE_RESOURCE_ID).unwrap();

        let client_identity = identity(142);
        let client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, client_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap()
                .with_peer_routes(server_peer, [server_address])
                .unwrap(),
            issued_at,
            AuthenticatedDomainServicesConfig::default(),
        )
        .await;
        let client_address = client.listen_addresses()[0].clone();
        let client_port = tcp_port(&client_address);
        let messages = client.messages();
        let streams = client.streams();

        let sender = messages.open(server_peer, &resource).await.unwrap();
        sender.send("queued.v1", 1, vec![1]).await.unwrap();
        let mut stalled_send = tokio::spawn({
            let sender = sender.clone();
            async move { sender.send("blocked.v1", 2, vec![2]).await }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut stalled_send)
                .await
                .is_err(),
            "the second send must wait for capacity and its ACK"
        );

        let mut subscription = streams
            .open::<CameraFrame>(server_peer, stream_request(server_peer, CAMERA_RESOURCE_ID))
            .await
            .unwrap();
        let mut stalled_read = tokio::spawn(async move { subscription.entries.next().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut stalled_read)
                .await
                .is_err(),
            "the accepted typed stream must remain live while its source is idle"
        );

        client.leave().await.unwrap();
        let send_result = tokio::time::timeout(OPERATION_BOUND, stalled_send)
            .await
            .expect("message send outlived ordered leave")
            .unwrap();
        assert!(matches!(send_result, Err(SendMessageError::Closed)));
        let read_result = tokio::time::timeout(OPERATION_BOUND, stalled_read)
            .await
            .expect("typed read outlived ordered leave")
            .unwrap();
        assert!(matches!(
            read_result,
            Some(Err(StreamError::ConnectionLost))
        ));
        assert!(matches!(
            messages.open(server_peer, &resource).await,
            Err(OpenMessageChannelError::Stopped)
        ));
        assert!(matches!(
            streams
                .open::<CameraFrame>(server_peer, stream_request(server_peer, CAMERA_RESOURCE_ID),)
                .await,
            Err(StreamsError::Stopped)
        ));

        drop(sender);
        server.leave().await.unwrap();
        assert!(registration.recv().await.is_none());
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, client_port)).unwrap();
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, server_port)).unwrap();
    })
    .await
    .expect("P11 ordered cancellation and cleanup must remain bounded");
}
