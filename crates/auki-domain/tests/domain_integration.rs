//! Public D16 vertical slice: one authenticated Domain owner, explicit routes,
//! retained Resource Catalog v0.2, fail-closed authority, and ordered teardown.

use std::{
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use auki_domain::{
    Domain, DomainBuilder, DomainConfig, DomainRoutesError, DomainStatus, Identity, KnownPeer,
    KnownPeerEvent, KnownPeerSubscription, MapCatalogProvider, MapLogResource,
    MessageChannelResource, Multiaddr, PeerId, ReadFrom, ResourceCatalogProvider, ResourcesRequest,
    ResourcesRequestV3, SignedP2pCredential, StreamRequest,
};
use auki_network::{
    protocol_ids::RESOURCES_V0_2_0,
    resources_protocol::{
        Available, Head, ResourceEntry, SensorBlock, SensorKind, SensorManifestPointer,
        VariantContent,
    },
    stream_protocol::CameraFrame,
};
use auki_p2p::{
    ApplicationProtocol, DdsTokenVerifier, DdsVerificationKeys, Node, P2P_TOKEN_AUDIENCE,
    P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE, P2PAccessClaims, SessionRequirements,
    SignedApplicationMetadata,
};
use auki_registry::RegistryRef;
use auki_session::Peer;
use futures::StreamExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use tempfile::TempDir;
use uuid::Uuid;

const TEST_BOUND: Duration = Duration::from_secs(60);
const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;
const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

#[derive(Clone)]
struct CountingCatalog {
    rows: Vec<ResourceEntry>,
    calls: Arc<AtomicUsize>,
}

impl ResourceCatalogProvider for CountingCatalog {
    fn snapshot(&self) -> Vec<ResourceEntry> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.rows.clone()
    }
}

// Compile the retained product operations through the public facade. The real
// D16 test below executes lifecycle, routes, observations, and v0.2 catalogs;
// P09-P11 exercise the other adapters in depth.
#[allow(dead_code, clippy::too_many_arguments)]
async fn retained_public_operations_are_reachable(
    domain: &mut Domain,
    peer: PeerId,
    channel: &MessageChannelResource,
    map: &MapLogResource,
    resources: Arc<dyn ResourceCatalogProvider>,
    maps: Arc<dyn MapCatalogProvider>,
) {
    let _ = domain.status();
    let _ = domain.subscribe_status();
    let _ = domain.authority();
    let _ = domain.routes();
    let _ = domain.known_peers();
    let _ = domain.protocols();
    let _ = domain.catalog();
    let _ = domain.set_resource_catalog_provider(resources);
    let _ = domain.set_map_catalog_provider(maps);
    let _ = domain.set_registry_app_root(".");
    let _ = domain.fetch_participant_info(peer).await;
    let _ = domain.fetch_resources_catalog(peer).await;
    let _ = domain
        .fetch_resources_catalog_with(peer, ResourcesRequest::all())
        .await;
    let _ = domain.fetch_resources_catalog_v3(peer).await;
    let _ = domain
        .fetch_resources_catalog_v3_with(peer, ResourcesRequestV3::all())
        .await;
    let _ = domain.fetch_map_catalog(peer).await;
    let _ = domain
        .list_registry_entries(peer, auki_domain::RegistryKind::DeviceModel)
        .await;
    let _ = domain.fetch_sensor_entry(peer, "id", "hash").await;
    let _ = domain.fetch_clock_entry(peer, "id", "hash").await;
    let _ = domain.fetch_frame_entry(peer, "id", "hash").await;
    let _ = domain.fetch_detector_entry(peer, "id", "hash").await;
    let _ = domain.fetch_map_entry(peer, "id", "hash").await;
    let _ = domain.fetch_device_model_entry(peer, "id", "hash").await;
    let _ = domain.fetch_blob(peer, "sha256").await;
    let _ = domain.take_message_channel_receiver("events");
    let _ = domain.open_message_channel(peer, channel).await;
    let _ = domain.send_message(channel, "type", 0, []).await;
    let _ = domain
        .open_stream::<CameraFrame>(peer, StreamRequest::default())
        .await;
    let _ = domain.open_map_stream(peer, map, ReadFrom::Latest).await;
}

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
        scopes: Vec::new(),
        application: Some(SignedApplicationMetadata {
            name: "public-domain-test".into(),
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

fn listener() -> Multiaddr {
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

fn resource(owner: PeerId, id: &str) -> ResourceEntry {
    ResourceEntry {
        source_peer_id: owner.to_string(),
        writer_peer_id: owner.to_string(),
        resource_id: id.into(),
        state: "live".into(),
        head: Some(Head::Rolling {
            retention_ns: 5_000_000_000,
        }),
        extent: None,
        available: Available {
            bytes: 1_024,
            entries: 10,
            duration_ns: 5_000_000_000,
        },
        sensor: Some(SensorBlock {
            kind: SensorKind::Camera,
            r#type: "rgb".into(),
            sensor_id: id.into(),
            sensor_hash: "sensor-hash".into(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: RegistryRef {
                    peer_id: owner.to_string(),
                    id: "clock".into(),
                    hash: "clock-hash".into(),
                },
                frame: None,
            },
        },
    }
}

async fn join_domain(
    domain_id: Uuid,
    identity: Identity,
    issued_at: u64,
    listen: bool,
    initial_route: Option<(PeerId, Multiaddr)>,
    provider: Option<Arc<dyn ResourceCatalogProvider>>,
) -> (Domain, TempDir) {
    let root = tempfile::tempdir().unwrap();
    let peer = Peer::new(identity.peer_id().to_string(), "public-domain-test")
        .with_storage_root(root.path().to_path_buf());
    let session = peer.start_session().unwrap();
    let mut config = DomainConfig::new(domain_id, identity.clone());
    if listen {
        config = config.with_listen_addresses([listener()]).unwrap();
    }
    if let Some((expected_peer, route)) = initial_route {
        config = config.with_peer_routes(expected_peer, [route]).unwrap();
    }
    let mut builder = DomainBuilder::new(&peer, &session, config)
        .authority(keys(), credential(identity.peer_id(), domain_id, issued_at));
    if let Some(provider) = provider {
        builder = builder.resource_catalog_provider(provider);
    }
    (builder.join().await.unwrap(), root)
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
async fn public_domain_d16_vertical_slice_is_authenticated_and_owned() {
    tokio::time::timeout(TEST_BOUND, async {
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();

        let a_identity = identity(201);
        let a_peer = a_identity.peer_id();
        let a_calls = Arc::new(AtomicUsize::new(0));
        let a_row = resource(a_peer, "alpha-camera");
        let (a, _a_root) = join_domain(
            domain_id,
            a_identity,
            issued_at,
            true,
            None,
            Some(Arc::new(CountingCatalog {
                rows: vec![a_row.clone()],
                calls: Arc::clone(&a_calls),
            })),
        )
        .await;
        let a_address = a.listen_addresses()[0].clone();
        let a_port = tcp_port(&a_address);

        let b_identity = identity(202);
        let b_peer = b_identity.peer_id();
        let b_calls = Arc::new(AtomicUsize::new(0));
        let b_row = resource(b_peer, "bravo-camera");
        let (b, _b_root) = join_domain(
            domain_id,
            b_identity,
            issued_at,
            true,
            Some((a_peer, a_address.clone())),
            Some(Arc::new(CountingCatalog {
                rows: vec![b_row.clone()],
                calls: Arc::clone(&b_calls),
            })),
        )
        .await;
        let b_address = b.listen_addresses()[0].clone();
        let b_port = tcp_port(&b_address);
        a.routes().replace(b_peer, [b_address]).unwrap();

        // Every rejected class completes mutual authentication before any
        // Resource Catalog request can reach A's provider.
        let wrong_peer = identity(203).peer_id();
        let (wrong_target, _wrong_target_root) = join_domain(
            domain_id,
            identity(204),
            issued_at,
            false,
            Some((wrong_peer, a_address.clone())),
            None,
        )
        .await;
        assert!(wrong_target.fetch_resources_catalog(wrong_peer).await.is_err());

        let (wrong_domain, _wrong_domain_root) = join_domain(
            Uuid::new_v4(),
            identity(205),
            issued_at,
            false,
            Some((a_peer, a_address.clone())),
            None,
        )
        .await;
        assert!(wrong_domain.fetch_resources_catalog(a_peer).await.is_err());

        let near_expiry = issued_at - P2P_TOKEN_TTL.as_secs() + 8;
        let (expired, _expired_root) = join_domain(
            domain_id,
            identity(206),
            near_expiry,
            false,
            Some((a_peer, a_address.clone())),
            None,
        )
        .await;
        let mut expired_status = expired.subscribe_status();
        tokio::time::timeout(Duration::from_secs(10), async {
            while *expired_status.borrow_and_update() != DomainStatus::CredentialUnavailable {
                expired_status.changed().await.unwrap();
            }
        })
        .await
        .expect("near-expiry authority must become unavailable");
        assert!(expired.fetch_resources_catalog(a_peer).await.is_err());

        let anonymous = Node::start(
            identity(207),
            DdsTokenVerifier::from_keys(keys()).unwrap(),
            [],
        )
        .unwrap();
        assert!(
            anonymous
                .open(
                    a_peer,
                    vec![a_address.clone()],
                    ApplicationProtocol::new(RESOURCES_V0_2_0).unwrap(),
                    SessionRequirements::new(domain_id.to_string())
                        .unwrap()
                        .with_expected_remote_peer_id(a_peer),
                )
                .await
                .is_err()
        );

        use libp2p::{StreamProtocol, SwarmBuilder, noise, swarm::SwarmEvent, tcp, yamux};
        use libp2p_stream::{Behaviour as StreamBehaviour, OpenStreamError};

        let streams = StreamBehaviour::new();
        let mut control = streams.new_control();
        let mut legacy = SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .unwrap()
            .with_behaviour(|_| streams)
            .unwrap()
            .build();
        legacy
            .dial(
                a_address
                    .clone()
                    .with(auki_p2p::Protocol::P2p(a_peer)),
            )
            .unwrap();
        let (connected_sender, connected_receiver) = tokio::sync::oneshot::channel();
        let legacy_driver = tokio::spawn(async move {
            let mut connected_sender = Some(connected_sender);
            while let Some(event) = legacy.next().await {
                if matches!(
                    event,
                    SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == a_peer
                ) && let Some(sender) = connected_sender.take()
                {
                    let _ = sender.send(());
                }
            }
        });
        connected_receiver.await.unwrap();
        let legacy_ids = [
            "/auki/info/0.0.1",
            "/auki/resources/0.2.0",
            "/auki/resources/0.3.0",
            "/auki/resources/0.4.0",
            "/auki/registries/0.2.0",
            "/auki/registries/0.3.0",
            "/auki/blobs/0.1.0",
            "/auki/message/0.1.0",
            "/auki/stream/0.2.0",
        ];
        for legacy_id in legacy_ids {
            let legacy_error = control
                .open_stream(a_peer, StreamProtocol::new(legacy_id))
                .await
                .expect_err("legacy protocol must not negotiate");
            assert!(
                matches!(legacy_error, OpenStreamError::UnsupportedProtocol(ref protocol) if protocol.as_ref() == legacy_id)
            );
        }

        assert_eq!(a_calls.load(Ordering::SeqCst), 0);
        assert_eq!(a.known_peers().peer_count(), 0);

        // The public local snapshot must sample the same live provider that
        // authenticated remote requests reach.
        assert_eq!(a.catalog().unwrap(), vec![a_row.clone()]);

        legacy_driver.abort();
        let _ = legacy_driver.await;
        anonymous.shutdown().await.unwrap();
        expired.leave().await.unwrap();
        wrong_domain.leave().await.unwrap();
        wrong_target.leave().await.unwrap();

        let mut a_events = a.known_peers().subscribe();
        let mut b_events = b.known_peers().subscribe();
        let from_a = b.fetch_resources_catalog(a_peer).await.unwrap();
        assert_eq!(from_a.resources, vec![a_row]);
        let observed_a = appeared(&mut b_events, a_peer).await;
        let observed_b = appeared(&mut a_events, b_peer).await;
        assert_eq!(
            observed_a.application().unwrap().name,
            "public-domain-test"
        );
        assert_eq!(
            observed_b.application().unwrap().name,
            "public-domain-test"
        );

        let from_b = a.fetch_resources_catalog(b_peer).await.unwrap();
        assert_eq!(from_b.resources, vec![b_row]);
        assert_eq!(a_calls.load(Ordering::SeqCst), 2);
        assert_eq!(b_calls.load(Ordering::SeqCst), 1);
        assert_eq!(a.known_peers().peer_count(), 1);
        assert_eq!(b.known_peers().peer_count(), 1);

        let replacement_calls = Arc::new(AtomicUsize::new(0));
        let replacement_row = resource(a_peer, "alpha-camera-replacement");
        a.set_resource_catalog_provider(Arc::new(CountingCatalog {
            rows: vec![replacement_row.clone()],
            calls: Arc::clone(&replacement_calls),
        }))
        .unwrap();
        assert_eq!(a.catalog().unwrap(), vec![replacement_row.clone()]);
        assert_eq!(
            b.fetch_resources_catalog(a_peer).await.unwrap().resources,
            vec![replacement_row]
        );
        assert_eq!(replacement_calls.load(Ordering::SeqCst), 2);

        let a_routes = a.routes();
        let b_routes = b.routes();
        let a_status = a.subscribe_status();
        let b_status = b.subscribe_status();
        b.leave().await.unwrap();
        a.leave().await.unwrap();
        assert_eq!(*a_status.borrow(), DomainStatus::Stopped);
        assert_eq!(*b_status.borrow(), DomainStatus::Stopped);
        assert!(matches!(a_routes.snapshot(), Err(DomainRoutesError::Stopped)));
        assert!(matches!(b_routes.snapshot(), Err(DomainRoutesError::Stopped)));
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, a_port)).unwrap();
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, b_port)).unwrap();
    })
    .await
    .expect("public D16 scenario must remain bounded");
}
