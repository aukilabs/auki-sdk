use std::{
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use auki_manifests::{PoseSource, PoseWriterMode};
use auki_network::{
    info_protocol::AuthenticatedParticipantInfo,
    protocol_ids::{INFO_V1_0_0, RESOURCES_V0_2_0, RESOURCES_V0_3_0, RESOURCES_V0_4_0},
    resources_protocol::{
        Available, Head, PoseBlock, PoseManifestPointer, ResourceEntry as ResourceEntryV2,
        ResourcesRequest as ResourcesRequestV2, SensorBlock, SensorKind, SensorManifestPointer,
        Variant as VariantV2, VariantContent,
    },
    resources_v3_protocol::{
        MessageChannelResource, ResourceEntry as ResourceEntryV3, ResourceVariant,
        ResourcesRequest as ResourcesRequestV3,
    },
    resources_v4_protocol::{
        MapCatalogProvider, MapLogResource, ResourcesResponse as ResourcesResponseV4,
    },
};
use auki_p2p::{
    ApplicationProtocol, DdsTokenVerifier, DdsVerificationKeys, Identity, Multiaddr, Node,
    P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE, P2PAccessClaims, PeerId,
    SessionRequirements, SignedApplicationMetadata, SignedP2pCredential,
};
use auki_registry::RegistryRef;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use uuid::Uuid;

use super::{
    AuthenticatedDomain, AuthenticatedDomainConfig, DomainStatus,
    info_v1::ParticipantInfoProvider,
    peers::{KnownPeer, KnownPeerEvent, KnownPeerSubscription},
    resources_v3::MessageChannelCatalogProvider,
    routes::DomainRoutesError,
};
use crate::resource_catalog::ResourceCatalogProvider;

const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

const TEST_BOUND: Duration = Duration::from_secs(30);

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
        scopes: vec!["diagnostic-only".into()],
        application: Some(SignedApplicationMetadata {
            name: "signed-application".into(),
            version: "9.9.9".into(),
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
    AuthenticatedDomain::join(config, keys(), credential)
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

fn registry(peer: PeerId, id: &str) -> RegistryRef {
    RegistryRef {
        peer_id: peer.to_string(),
        id: id.into(),
        hash: format!("{id}-hash"),
    }
}

fn sensor(peer: PeerId, resource_id: &str) -> ResourceEntryV2 {
    ResourceEntryV2 {
        source_peer_id: peer.to_string(),
        writer_peer_id: peer.to_string(),
        resource_id: resource_id.into(),
        state: "live".into(),
        head: Some(Head::Rolling {
            retention_ns: 5_000_000_000,
        }),
        extent: None,
        available: Available {
            bytes: 4_096,
            entries: 23,
            duration_ns: 1_500_000,
        },
        sensor: Some(SensorBlock {
            kind: SensorKind::Camera,
            r#type: "rgb".into(),
            sensor_id: "front-camera".into(),
            sensor_hash: "sensor-hash".into(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: registry(peer, "clock"),
                frame: Some(registry(peer, "camera-frame")),
            },
        },
    }
}

fn pose(peer: PeerId, resource_id: &str) -> ResourceEntryV2 {
    ResourceEntryV2 {
        source_peer_id: peer.to_string(),
        writer_peer_id: peer.to_string(),
        resource_id: resource_id.into(),
        state: "live".into(),
        head: Some(Head::Rolling {
            retention_ns: 5_000_000_000,
        }),
        extent: None,
        available: Available {
            bytes: 512,
            entries: 8,
            duration_ns: 1_000_000,
        },
        sensor: None,
        pose: Some(PoseBlock {
            writer_mode: PoseWriterMode::Movable,
        }),
        variant_content: VariantContent::PoseLog {
            manifest: PoseManifestPointer {
                from_frame: registry(peer, "world"),
                to_frame: registry(peer, "robot"),
                clock: registry(peer, "clock"),
                source: PoseSource::Manual,
                expected_rate_hz: 30,
            },
        },
    }
}

fn channel(peer: PeerId, resource_id: &str) -> MessageChannelResource {
    MessageChannelResource {
        owner_peer_id: peer,
        resource_id: resource_id.into(),
        clock: registry(peer, "clock"),
    }
}

fn map_log(peer: PeerId, resource_id: &str) -> MapLogResource {
    MapLogResource {
        source_peer_id: peer.to_string(),
        writer_peer_id: peer.to_string(),
        resource_id: resource_id.into(),
        map: registry(peer, "map"),
        clock: registry(peer, "clock"),
    }
}

fn participant(peer: PeerId, label: &str) -> AuthenticatedParticipantInfo {
    AuthenticatedParticipantInfo {
        app: "diagnostic-application".into(),
        app_version: "1.2.3".into(),
        name: label.into(),
        session_id: "session-1".into(),
        session_clock_id: "clock".into(),
        session_clock_hash: "clock-hash".into(),
        session_now_ns: 42,
        peer_id: peer,
        app_instance: "device-1".into(),
    }
}

struct CountingInfoProvider {
    value: AuthenticatedParticipantInfo,
    calls: Arc<AtomicUsize>,
}

impl ParticipantInfoProvider for CountingInfoProvider {
    fn participant_info(&self) -> AuthenticatedParticipantInfo {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.value.clone()
    }
}

struct CountingResourceProvider {
    values: Vec<ResourceEntryV2>,
    calls: Arc<AtomicUsize>,
}

impl ResourceCatalogProvider for CountingResourceProvider {
    fn snapshot(&self) -> Vec<ResourceEntryV2> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values.clone()
    }
}

struct CountingChannelProvider {
    values: Vec<MessageChannelResource>,
    calls: Arc<AtomicUsize>,
}

impl MessageChannelCatalogProvider for CountingChannelProvider {
    fn message_channel_catalog(&self) -> Vec<MessageChannelResource> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values.clone()
    }
}

struct CountingMapProvider {
    value: ResourcesResponseV4,
    calls: Arc<AtomicUsize>,
}

impl MapCatalogProvider for CountingMapProvider {
    fn map_catalog(&self) -> ResourcesResponseV4 {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.value.clone()
    }
}

#[derive(Clone)]
struct SourceCounters {
    info: Arc<AtomicUsize>,
    v2: Arc<AtomicUsize>,
    v3: Arc<AtomicUsize>,
    v4: Arc<AtomicUsize>,
}

impl SourceCounters {
    fn new() -> Self {
        Self {
            info: Arc::new(AtomicUsize::new(0)),
            v2: Arc::new(AtomicUsize::new(0)),
            v3: Arc::new(AtomicUsize::new(0)),
            v4: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn values(&self) -> [usize; 4] {
        [
            self.info.load(Ordering::SeqCst),
            self.v2.load(Ordering::SeqCst),
            self.v3.load(Ordering::SeqCst),
            self.v4.load(Ordering::SeqCst),
        ]
    }
}

fn install_sources(
    domain: &AuthenticatedDomain,
    info: AuthenticatedParticipantInfo,
    v2: Vec<ResourceEntryV2>,
    v3: Vec<MessageChannelResource>,
    v4: ResourcesResponseV4,
    counters: &SourceCounters,
) {
    domain
        .info_v1()
        .set_provider(Arc::new(CountingInfoProvider {
            value: info,
            calls: Arc::clone(&counters.info),
        }))
        .unwrap();
    domain
        .resources_v2()
        .set_provider(Arc::new(CountingResourceProvider {
            values: v2,
            calls: Arc::clone(&counters.v2),
        }))
        .unwrap();
    domain
        .resources_v3()
        .set_message_channel_provider(Arc::new(CountingChannelProvider {
            values: v3,
            calls: Arc::clone(&counters.v3),
        }))
        .unwrap();
    domain
        .resources_v4()
        .set_provider(Arc::new(CountingMapProvider {
            value: v4,
            calls: Arc::clone(&counters.v4),
        }))
        .unwrap();
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

async fn metadata_update(
    subscription: &mut KnownPeerSubscription,
    expected_peer: PeerId,
    expected_info: &AuthenticatedParticipantInfo,
) -> KnownPeer {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let KnownPeerEvent::Updated(peer) = subscription.recv().await.unwrap()
                && peer.peer_id() == expected_peer
                && peer.participant_info() == Some(expected_info)
            {
                return peer;
            }
        }
    })
    .await
    .expect("authenticated info must refresh diagnostic peer metadata")
}

#[tokio::test(flavor = "multi_thread")]
async fn p09_family_preserves_payloads_sampling_filters_and_diagnostic_metadata() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();

        let a_identity = identity(71);
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

        let b_identity = identity(72);
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
        a.routes().replace(b_peer, [b_address]).unwrap();
        b.routes().replace(a_peer, [a_address]).unwrap();

        let info = participant(b_peer, "server-b");
        let sensor_row = sensor(b_peer, "front-camera");
        let pose_row = pose(b_peer, "world-to-robot");
        let message_row = channel(b_peer, "events");
        let map_row = map_log(b_peer, "occupancy");
        let counters = SourceCounters::new();
        install_sources(
            &b,
            info.clone(),
            vec![sensor_row.clone(), pose_row.clone()],
            vec![message_row.clone()],
            ResourcesResponseV4 {
                resources: vec![map_row.clone()],
            },
            &counters,
        );

        let routes_before_info = a.routes().snapshot().unwrap();
        let mut a_events = a.peers().subscribe();
        let mut b_events = b.peers().subscribe();

        let v2 = a
            .resources_v2()
            .fetch(
                b_peer,
                ResourcesRequestV2 {
                    variants: vec![VariantV2::SensorLog],
                },
            )
            .await
            .unwrap();
        assert_eq!(v2.resources, vec![sensor_row.clone()]);
        let observed_b = appeared(&mut a_events, b_peer).await;
        let observed_a = appeared(&mut b_events, a_peer).await;
        let authenticated_until = observed_b.authenticated_until();
        assert_eq!(observed_b.application().unwrap().name, "signed-application");
        assert_eq!(observed_a.application().unwrap().name, "signed-application");
        assert!(observed_b.participant_info().is_none());

        let merged = a
            .resources_v3()
            .fetch(b_peer, ResourcesRequestV3::all())
            .await
            .unwrap();
        assert_eq!(
            merged.resources,
            vec![
                ResourceEntryV3::V2(Box::new(sensor_row.clone())),
                ResourceEntryV3::V2(Box::new(pose_row.clone())),
                ResourceEntryV3::MessageChannel(message_row.clone()),
            ]
        );

        let messages = a
            .resources_v3()
            .fetch(
                b_peer,
                ResourcesRequestV3 {
                    variants: vec![ResourceVariant::MessageChannel],
                },
            )
            .await
            .unwrap();
        assert_eq!(
            messages.resources,
            vec![ResourceEntryV3::MessageChannel(message_row.clone())]
        );

        let pose_and_messages = a
            .resources_v3()
            .fetch(
                b_peer,
                ResourcesRequestV3 {
                    variants: vec![ResourceVariant::PoseLog, ResourceVariant::MessageChannel],
                },
            )
            .await
            .unwrap();
        assert_eq!(
            pose_and_messages.resources,
            vec![
                ResourceEntryV3::V2(Box::new(pose_row)),
                ResourceEntryV3::MessageChannel(message_row),
            ]
        );

        assert_eq!(
            a.resources_v4().fetch(b_peer).await.unwrap(),
            ResourcesResponseV4 {
                resources: vec![map_row],
            }
        );
        assert_eq!(a.info_v1().fetch(b_peer).await.unwrap(), info);

        let observed_b = metadata_update(&mut a_events, b_peer, &info).await;
        assert_eq!(observed_b.participant_info(), Some(&info));
        assert_eq!(observed_b.application().unwrap().name, "signed-application");
        assert_ne!(
            observed_b.application().unwrap().name,
            observed_b.participant_info().unwrap().app,
            "diagnostic info must not replace signed transport metadata"
        );
        assert_eq!(observed_b.authenticated_until(), authenticated_until);
        assert_eq!(a.routes().snapshot().unwrap(), routes_before_info);

        let refreshed_info = participant(b_peer, "server-b-refreshed");
        b.info_v1()
            .set_provider(Arc::new(CountingInfoProvider {
                value: refreshed_info.clone(),
                calls: Arc::clone(&counters.info),
            }))
            .unwrap();
        assert_eq!(a.info_v1().fetch(b_peer).await.unwrap(), refreshed_info);
        let observed_b = metadata_update(&mut a_events, b_peer, &refreshed_info).await;
        assert_eq!(observed_b.authenticated_until(), authenticated_until);
        assert_eq!(a.routes().snapshot().unwrap(), routes_before_info);

        b.info_v1()
            .set_provider(Arc::new(CountingInfoProvider {
                value: participant(a_peer, "wrong-peer"),
                calls: Arc::clone(&counters.info),
            }))
            .unwrap();
        assert!(a.info_v1().fetch(b_peer).await.is_err());
        let observed_b = a
            .peers()
            .snapshot()
            .peers()
            .iter()
            .find(|peer| peer.peer_id() == b_peer)
            .cloned()
            .unwrap();
        assert_eq!(observed_b.participant_info(), Some(&refreshed_info));
        assert_eq!(observed_b.authenticated_until(), authenticated_until);
        assert_eq!(a.routes().snapshot().unwrap(), routes_before_info);
        assert_eq!(a.peers().peer_count(), 1);
        assert_eq!(b.peers().peer_count(), 1);
        assert_eq!(counters.values(), [3, 3, 3, 1]);

        let info_handle = b.info_v1();
        let v2_handle = b.resources_v2();
        let v3_handle = b.resources_v3();
        let v4_handle = b.resources_v4();
        let a_routes = a.routes();
        let b_routes = b.routes();
        b.leave().await.unwrap();
        a.leave().await.unwrap();

        assert!(info_handle.local().is_err());
        assert!(v2_handle.local(&ResourcesRequestV2::all()).is_err());
        assert!(v3_handle.local(&ResourcesRequestV3::all()).is_err());
        assert!(v4_handle.local().is_err());
        assert!(matches!(
            a_routes.snapshot(),
            Err(DomainRoutesError::Stopped)
        ));
        assert!(matches!(
            b_routes.snapshot(),
            Err(DomainRoutesError::Stopped)
        ));
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, a_port)).unwrap();
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, b_port)).unwrap();
    })
    .await
    .expect("the complete authenticated info/resource family must remain bounded");
}

async fn assert_four_handlers_reject(domain: &AuthenticatedDomain, expected_peer: PeerId) {
    assert!(domain.info_v1().fetch(expected_peer).await.is_err());
    assert!(
        domain
            .resources_v2()
            .fetch(expected_peer, ResourcesRequestV2::all())
            .await
            .is_err()
    );
    assert!(
        domain
            .resources_v3()
            .fetch(
                expected_peer,
                ResourcesRequestV3 {
                    variants: vec![ResourceVariant::MessageChannel],
                },
            )
            .await
            .is_err()
    );
    assert!(domain.resources_v4().fetch(expected_peer).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn p09_d09_authorization_matrix_invokes_no_sources_then_recovers() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let server_identity = identity(81);
        let server_peer = server_identity.peer_id();
        let server = join_domain(
            AuthenticatedDomainConfig::new(domain_id, server_identity)
                .with_listen_addresses([tcp_listener()])
                .unwrap(),
            issued_at,
        )
        .await;
        let server_address = server.listen_addresses()[0].clone();

        let expected_info = participant(server_peer, "authority-server");
        let expected_v2 = sensor(server_peer, "camera");
        let expected_v3 = channel(server_peer, "events");
        let expected_v4 = map_log(server_peer, "map-log");
        let counters = SourceCounters::new();
        install_sources(
            &server,
            expected_info.clone(),
            vec![expected_v2.clone()],
            vec![expected_v3.clone()],
            ResourcesResponseV4 {
                resources: vec![expected_v4.clone()],
            },
            &counters,
        );

        let wrong_peer_client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(82)),
            issued_at,
        )
        .await;
        let wrong_peer = identity(83).peer_id();
        wrong_peer_client
            .routes()
            .replace(wrong_peer, [server_address.clone()])
            .unwrap();
        assert_four_handlers_reject(&wrong_peer_client, wrong_peer).await;
        assert_eq!(counters.values(), [0, 0, 0, 0]);

        let wrong_domain_client = join_domain(
            AuthenticatedDomainConfig::new(Uuid::new_v4(), identity(84))
                .with_peer_routes(server_peer, [server_address.clone()])
                .unwrap(),
            issued_at,
        )
        .await;
        assert_four_handlers_reject(&wrong_domain_client, server_peer).await;
        assert_eq!(counters.values(), [0, 0, 0, 0]);

        let expired_client = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(85))
                .with_peer_routes(server_peer, [server_address.clone()])
                .unwrap(),
            issued_at - P2P_TOKEN_TTL.as_secs() + 2,
        )
        .await;
        let mut expired_status = expired_client.subscribe_status();
        while *expired_status.borrow_and_update() != DomainStatus::CredentialUnavailable {
            expired_status.changed().await.unwrap();
        }
        assert_four_handlers_reject(&expired_client, server_peer).await;
        assert_eq!(counters.values(), [0, 0, 0, 0]);

        let anonymous = Node::start(
            identity(86),
            DdsTokenVerifier::from_keys(keys()).unwrap(),
            [],
        )
        .unwrap();
        for protocol_id in [
            INFO_V1_0_0,
            RESOURCES_V0_2_0,
            RESOURCES_V0_3_0,
            RESOURCES_V0_4_0,
        ] {
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
        assert_eq!(counters.values(), [0, 0, 0, 0]);
        assert_eq!(server.peers().peer_count(), 0);

        let valid = join_domain(
            AuthenticatedDomainConfig::new(domain_id, identity(87))
                .with_peer_routes(server_peer, [server_address])
                .unwrap(),
            issued_at,
        )
        .await;
        assert_eq!(
            valid.info_v1().fetch(server_peer).await.unwrap(),
            expected_info
        );
        assert_eq!(
            valid
                .resources_v2()
                .fetch(server_peer, ResourcesRequestV2::all())
                .await
                .unwrap()
                .resources,
            vec![expected_v2]
        );
        assert_eq!(
            valid
                .resources_v3()
                .fetch(
                    server_peer,
                    ResourcesRequestV3 {
                        variants: vec![ResourceVariant::MessageChannel],
                    },
                )
                .await
                .unwrap()
                .resources,
            vec![ResourceEntryV3::MessageChannel(expected_v3)]
        );
        assert_eq!(
            valid.resources_v4().fetch(server_peer).await.unwrap(),
            ResourcesResponseV4 {
                resources: vec![expected_v4],
            }
        );
        assert_eq!(counters.values(), [1, 1, 1, 1]);

        valid.leave().await.unwrap();
        anonymous.shutdown().await.unwrap();
        expired_client.leave().await.unwrap();
        wrong_domain_client.leave().await.unwrap();
        wrong_peer_client.leave().await.unwrap();
        server.leave().await.unwrap();
    })
    .await
    .expect("the D09 authorization matrix must remain bounded");
}
