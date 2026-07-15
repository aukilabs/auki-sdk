use std::{sync::Arc, time::Duration};

use auki_domain::{
    ClusterTarget, DaemonInfo, Domain, DomainBuilder, DomainBuilderError, DomainConfig,
    DomainError, DomainOpenMessageChannelError, MessageChannelReceiver, MessageChannelResource,
    ResourceEntryV3, ResourceVariantV3, ResourcesRequestV3,
};
use auki_network::{
    PeerIdentity,
    stream_runtime::decline_all_streams,
    swarm::{Behaviour, SwarmConfig, build_swarm},
};
use auki_registry::RegistryRef;
use auki_session::Peer;
use libp2p::{Swarm, swarm::SwarmEvent};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    task::JoinHandle,
};

fn swarm_for(identity: &PeerIdentity) -> Swarm<Behaviour> {
    build_swarm(
        identity,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-domain-message-it/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap()
}

async fn wait_for_listen_addr(swarm: &mut Swarm<Behaviour>) -> libp2p::Multiaddr {
    use futures::StreamExt;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                return address;
            }
        }
    })
    .await
    .expect("listen address")
}

fn daemon_info(app: &str) -> DaemonInfo {
    DaemonInfo {
        app: app.into(),
        name: app.into(),
        session_id: String::new(),
        session_clock_id: String::new(),
        session_clock_hash: String::new(),
        app_instance: format!("{app}-instance"),
    }
}

fn config(
    target: ClusterTarget,
    identity: PeerIdentity,
    local_multiaddr: libp2p::Multiaddr,
    discovery_url: &str,
    swarm: Swarm<Behaviour>,
) -> DomainConfig {
    DomainConfig {
        target,
        local_identity: identity,
        local_multiaddrs: vec![local_multiaddr],
        discovery_url: discovery_url.into(),
        swarm,
        stream_provider: decline_all_streams(),
        daemon_info: daemon_info("typed-messaging-test"),
    }
}

fn offline_config(identity: PeerIdentity, swarm: Swarm<Behaviour>) -> DomainConfig {
    DomainConfig {
        target: ClusterTarget::create("must-not-bootstrap"),
        local_identity: identity,
        local_multiaddrs: Vec::new(),
        discovery_url: "http://127.0.0.1:0".into(),
        swarm,
        stream_provider: decline_all_streams(),
        daemon_info: daemon_info("preflight-test"),
    }
}

fn channel(owner: libp2p::PeerId, resource_id: &str, clock: RegistryRef) -> MessageChannelResource {
    MessageChannelResource {
        owner_peer_id: owner,
        resource_id: resource_id.into(),
        clock,
    }
}

#[tokio::test]
async fn builder_rejects_owner_mismatch_and_duplicate_channels_before_join() {
    let identity = PeerIdentity::from_seed(&[120; 32]);
    let first_tmp = tempfile::tempdir().unwrap();
    let peer = Peer::new(identity.peer_id().to_string(), "app")
        .with_storage_root(first_tmp.path().to_path_buf());
    let session = peer.start_session().unwrap();
    let other = PeerIdentity::from_seed(&[121; 32]).peer_id();
    let mut swarm = swarm_for(&identity);
    let addr = wait_for_listen_addr(&mut swarm).await;
    let builder = DomainBuilder::new(
        &peer,
        &session,
        config(
            ClusterTarget::create("never-contact-discovery"),
            identity,
            addr,
            "http://127.0.0.1:0",
            swarm,
        ),
    );

    let mismatch = builder.message_channel(channel(other, "events", session.monotonic_clock()), 4);
    assert!(matches!(
        mismatch,
        Err(DomainBuilderError::ChannelOwnerMismatch { .. })
    ));

    let identity = PeerIdentity::from_seed(&[122; 32]);
    let second_tmp = tempfile::tempdir().unwrap();
    let peer = Peer::new(identity.peer_id().to_string(), "app")
        .with_storage_root(second_tmp.path().to_path_buf());
    let session = peer.start_session().unwrap();
    let mut swarm = swarm_for(&identity);
    let addr = wait_for_listen_addr(&mut swarm).await;
    let row = channel(identity.peer_id(), "events", session.monotonic_clock());
    let duplicate = DomainBuilder::new(
        &peer,
        &session,
        config(
            ClusterTarget::create("never-contact-discovery"),
            identity,
            addr,
            "http://127.0.0.1:0",
            swarm,
        ),
    )
    .message_channel(row.clone(), 4)
    .unwrap()
    .message_channel(row, 8);
    assert!(matches!(
        duplicate,
        Err(DomainBuilderError::DuplicateMessageChannel { .. })
    ));
}

#[tokio::test]
async fn join_rejects_session_identity_mismatch_before_bootstrap() {
    let identity = PeerIdentity::from_seed(&[125; 32]);
    let peer_tmp = tempfile::tempdir().unwrap();
    let peer = Peer::new(identity.peer_id().to_string(), "app")
        .with_storage_root(peer_tmp.path().to_path_buf());
    let other_tmp = tempfile::tempdir().unwrap();
    let other_peer = Peer::new(
        PeerIdentity::from_seed(&[126; 32]).peer_id().to_string(),
        "app",
    )
    .with_storage_root(other_tmp.path().to_path_buf());
    let wrong_session = other_peer.start_session().unwrap();
    let swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: vec![],
            ..SwarmConfig::default()
        },
    )
    .unwrap();

    assert!(matches!(
        DomainBuilder::new(&peer, &wrong_session, offline_config(identity, swarm))
            .join()
            .await,
        Err(DomainError::SessionIdentityMismatch { .. })
    ));
}

#[tokio::test]
async fn join_rejects_swarm_identity_mismatch_before_bootstrap() {
    let identity = PeerIdentity::from_seed(&[127; 32]);
    let wrong_swarm_identity = PeerIdentity::from_seed(&[128; 32]);
    let tmp = tempfile::tempdir().unwrap();
    let peer = Peer::new(identity.peer_id().to_string(), "app")
        .with_storage_root(tmp.path().to_path_buf());
    let session = peer.start_session().unwrap();
    let swarm = build_swarm(
        &wrong_swarm_identity,
        SwarmConfig {
            listen_addresses: vec![],
            ..SwarmConfig::default()
        },
    )
    .unwrap();

    assert!(matches!(
        DomainBuilder::new(&peer, &session, offline_config(identity, swarm))
            .join()
            .await,
        Err(DomainError::SwarmIdentityMismatch { .. })
    ));
}

#[tokio::test]
async fn builder_rejects_unregistered_and_hash_mismatched_channel_clocks() {
    let identity = PeerIdentity::from_seed(&[129; 32]);
    let tmp = tempfile::tempdir().unwrap();
    let peer = Peer::new(identity.peer_id().to_string(), "app")
        .with_storage_root(tmp.path().to_path_buf());
    let session = peer.start_session().unwrap();

    let mut unknown = session.monotonic_clock();
    unknown.id.push_str("/unknown");
    let swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: vec![],
            ..SwarmConfig::default()
        },
    )
    .unwrap();
    assert!(matches!(
        DomainBuilder::new(&peer, &session, offline_config(identity.clone(), swarm))
            .message_channel(channel(identity.peer_id(), "unknown-clock", unknown), 4),
        Err(DomainBuilderError::UnregisteredChannelClock { .. })
    ));

    let mut wrong_hash = session.monotonic_clock();
    wrong_hash.hash = "wrong-hash".into();
    let swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: vec![],
            ..SwarmConfig::default()
        },
    )
    .unwrap();
    assert!(matches!(
        DomainBuilder::new(&peer, &session, offline_config(identity.clone(), swarm))
            .message_channel(
                channel(identity.peer_id(), "wrong-clock-hash", wrong_hash),
                4
            ),
        Err(DomainBuilderError::UnregisteredChannelClock { .. })
    ));
}

#[tokio::test]
async fn builder_rejects_zero_capacity_and_malformed_rows() {
    let identity = PeerIdentity::from_seed(&[130; 32]);
    let tmp = tempfile::tempdir().unwrap();
    let peer = Peer::new(identity.peer_id().to_string(), "app")
        .with_storage_root(tmp.path().to_path_buf());
    let session = peer.start_session().unwrap();
    let swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: vec![],
            ..SwarmConfig::default()
        },
    )
    .unwrap();
    assert!(matches!(
        DomainBuilder::new(&peer, &session, offline_config(identity.clone(), swarm))
            .message_channel(
                channel(
                    identity.peer_id(),
                    "zero-capacity",
                    session.monotonic_clock()
                ),
                0
            ),
        Err(DomainBuilderError::ZeroReceiverCapacity)
    ));

    let swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: vec![],
            ..SwarmConfig::default()
        },
    )
    .unwrap();
    assert!(matches!(
        DomainBuilder::new(&peer, &session, offline_config(identity.clone(), swarm))
            .message_channel(
                channel(identity.peer_id(), "", session.monotonic_clock()),
                4
            ),
        Err(DomainBuilderError::InvalidMessageChannel(_))
    ));
}

async fn joined_domain_with_receiver(
    seed: u8,
    cluster_name: &str,
) -> (Domain, MessageChannelReceiver, JoinHandle<()>) {
    let (discovery_url, discovery_task) = start_discovery().await;
    let identity = PeerIdentity::from_seed(&[seed; 32]);
    let tmp = tempfile::tempdir().unwrap();
    let peer = Peer::new(identity.peer_id().to_string(), "receiver")
        .with_storage_root(tmp.path().to_path_buf());
    let session = peer.start_session().unwrap();
    let row = channel(
        identity.peer_id(),
        "shutdown-events",
        session.monotonic_clock(),
    );
    let mut swarm = swarm_for(&identity);
    let addr = wait_for_listen_addr(&mut swarm).await;
    let mut domain = DomainBuilder::new(
        &peer,
        &session,
        config(
            ClusterTarget::create(cluster_name),
            identity,
            addr,
            &discovery_url,
            swarm,
        ),
    )
    .message_channel(row, 4)
    .unwrap()
    .join()
    .await
    .unwrap();
    let receiver = domain
        .take_message_channel_receiver("shutdown-events")
        .unwrap();
    (domain, receiver, discovery_task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_receiver_closes_after_clean_domain_leave() {
    let (domain, mut receiver, discovery_task) =
        joined_domain_with_receiver(131, "receiver-clean-leave").await;

    domain.leave().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("receiver closes promptly")
            .is_none()
    );
    discovery_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_receiver_closes_after_domain_drop() {
    let (domain, mut receiver, discovery_task) =
        joined_domain_with_receiver(132, "receiver-domain-drop").await;

    drop(domain);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("receiver closes promptly")
            .is_none()
    );
    discovery_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_domains_discover_and_exchange_live_opaque_typed_messages() {
    let (discovery_url, discovery_task) = start_discovery().await;
    let cluster_name = "typed-messaging-hermetic";

    let receiver_identity = PeerIdentity::from_seed(&[123; 32]);
    let receiver_peer_id = receiver_identity.peer_id();
    let receiver_tmp = tempfile::tempdir().unwrap();
    let receiver_peer = Peer::new(receiver_peer_id.to_string(), "receiver")
        .with_storage_root(receiver_tmp.path().to_path_buf());
    let receiver_session = receiver_peer.start_session().unwrap();
    let clock = receiver_session.monotonic_clock();
    let row = channel(receiver_peer_id, "application-events", clock.clone());
    let mut receiver_swarm = swarm_for(&receiver_identity);
    let receiver_addr = wait_for_listen_addr(&mut receiver_swarm).await;
    let mut receiver_domain = DomainBuilder::new(
        &receiver_peer,
        &receiver_session,
        config(
            ClusterTarget::create(cluster_name),
            receiver_identity,
            receiver_addr,
            &discovery_url,
            receiver_swarm,
        ),
    )
    .message_channel(row.clone(), 4)
    .unwrap()
    .join()
    .await
    .unwrap();
    let mut messages = receiver_domain
        .take_message_channel_receiver("application-events")
        .expect("builder registration yields app receiver");

    let sender_identity = PeerIdentity::from_seed(&[124; 32]);
    let sender_peer_id = sender_identity.peer_id();
    let sender_tmp = tempfile::tempdir().unwrap();
    let sender_peer = Peer::new(sender_peer_id.to_string(), "sender")
        .with_storage_root(sender_tmp.path().to_path_buf());
    let sender_session = sender_peer.start_session().unwrap();
    let mut sender_swarm = swarm_for(&sender_identity);
    let sender_addr = wait_for_listen_addr(&mut sender_swarm).await;
    let sender_domain = DomainBuilder::new(
        &sender_peer,
        &sender_session,
        config(
            ClusterTarget::join(cluster_name),
            sender_identity,
            sender_addr,
            &discovery_url,
            sender_swarm,
        ),
    )
    .join()
    .await
    .unwrap();

    let v2 = sender_domain
        .fetch_resources_catalog(receiver_peer_id)
        .await
        .unwrap();
    assert!(v2.resources.is_empty(), "v0.2 remains unchanged");

    let v3 = sender_domain
        .fetch_resources_catalog_v3_with(
            receiver_peer_id,
            ResourcesRequestV3 {
                variants: vec![ResourceVariantV3::MessageChannel],
            },
        )
        .await
        .unwrap();
    let discovered = match v3.resources.as_slice() {
        [ResourceEntryV3::MessageChannel(discovered)] => discovered.clone(),
        rows => panic!("expected one v0.3 message channel row, got {rows:?}"),
    };
    assert_eq!(discovered.clock, clock);
    assert_eq!(discovered, row);
    assert!(matches!(
        sender_domain
            .open_message_channel(sender_peer_id, &discovered)
            .await,
        Err(DomainOpenMessageChannelError::OwnerMismatch { .. })
    ));

    let sender = sender_domain
        .open_message_channel(receiver_peer_id, &discovered)
        .await
        .unwrap();
    let payload = "opaque application payload: åuki".as_bytes().to_vec();
    sender
        .send("ambientmovement.command.v1", 9_876_543_210, payload.clone())
        .await
        .unwrap();
    sender
        .send("application.other.v7", -55, vec![0x00, 0xff, 0x7f])
        .await
        .unwrap();
    sender_domain
        .send_message(
            &discovered,
            "application.oneshot.v1",
            77,
            b"one-shot".to_vec(),
        )
        .await
        .unwrap();

    let first = messages.recv().await.unwrap();
    assert_eq!(first.channel, discovered);
    assert_eq!(first.sender, sender_peer_id);
    assert_eq!(first.r#type, "ambientmovement.command.v1");
    assert_eq!(first.timestamp_ns, 9_876_543_210);
    assert_eq!(first.payload, payload);

    let second = messages.recv().await.unwrap();
    assert_eq!(second.channel, row);
    assert_eq!(second.sender, sender_peer_id);
    assert_eq!(second.r#type, "application.other.v7");
    assert_eq!(second.timestamp_ns, -55);
    assert_eq!(second.payload, vec![0x00, 0xff, 0x7f]);

    let third = messages.recv().await.unwrap();
    assert_eq!(third.channel, row);
    assert_eq!(third.sender, sender_peer_id);
    assert_eq!(third.r#type, "application.oneshot.v1");
    assert_eq!(third.timestamp_ns, 77);
    assert_eq!(third.payload, b"one-shot");

    drop(messages);
    let catalog_after_receiver_drop = sender_domain
        .fetch_resources_catalog_v3_with(
            receiver_peer_id,
            ResourcesRequestV3 {
                variants: vec![ResourceVariantV3::MessageChannel],
            },
        )
        .await
        .unwrap();
    assert!(
        catalog_after_receiver_drop.resources.is_empty(),
        "dropping the app receiver removes its v0.3 catalog row"
    );
    assert!(
        tokio::time::timeout(
            Duration::from_secs(1),
            sender.send("after-receiver-drop", 1, vec![1]),
        )
        .await
        .expect("send resolves after receiver disconnect")
        .is_err()
    );

    let replacement_row = channel(
        receiver_peer_id,
        "application-events",
        receiver_session.utc_clock(),
    );
    let mut replacement = receiver_domain
        .cluster_manager()
        .register_message_channel(replacement_row.clone(), 4)
        .unwrap();
    let stale_open = sender_domain
        .open_message_channel(receiver_peer_id, &discovered)
        .await;
    let stale_rejected = stale_open.is_err();
    if let Ok(stale_sender) = stale_open {
        let _ = stale_sender.send("stale-row", 88, vec![8, 8]).await;
    }
    assert!(
        stale_rejected,
        "re-registering the same owner/resource with a different clock must reject the old row"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(250), replacement.recv())
            .await
            .is_err(),
        "a sender opened from the stale row must not deliver a payload"
    );
    assert_eq!(replacement.resource(), &replacement_row);

    drop(sender_domain);
    drop(receiver_domain);
    discovery_task.abort();
}

async fn start_discovery() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let entry = Arc::new(Mutex::new(None::<Value>));
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let entry = entry.clone();
            tokio::spawn(async move {
                handle_discovery_request(stream, entry).await;
            });
        }
    });
    (url, task)
}

async fn handle_discovery_request(mut stream: TcpStream, entry: Arc<Mutex<Option<Value>>>) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 4096];
        let Ok(read) = stream.read(&mut chunk).await else {
            return;
        };
        if read == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let mut chunk = [0; 4096];
        let Ok(read) = stream.read(&mut chunk).await else {
            return;
        };
        if read == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    let request_line = headers.lines().next().unwrap();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap();
    let path = request_parts.next().unwrap();
    let body: Value = if content_length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap()
    };

    let (status, response) = if method == "GET" && path == "/clusters" {
        let resources = entry.lock().await.clone().into_iter().collect::<Vec<_>>();
        ("200 OK", json!({ "clusters": resources }))
    } else if method == "POST"
        && path.starts_with("/clusters/")
        && !path.ends_with("/liveness")
        && !path.ends_with("/manager")
    {
        let name = path.trim_start_matches("/clusters/");
        let created = json!({
            "name": name,
            "manager_peer_id": body["manager_peer_id"],
            "manager_multiaddrs": body["manager_multiaddrs"],
            "relay_multiaddrs": body.get("relay_multiaddrs").cloned().unwrap_or_else(|| json!([])),
            "peer_count": 1,
            "created_ns": 1,
            "last_liveness_check_ns": 1
        });
        *entry.lock().await = Some(created.clone());
        ("201 Created", created)
    } else if method == "POST" && path.ends_with("/liveness") {
        let mut guard = entry.lock().await;
        let current = guard.as_mut().unwrap();
        current["peer_count"] = body["peer_count"].clone();
        current["last_liveness_check_ns"] = json!(2);
        ("200 OK", current.clone())
    } else if method == "DELETE" && path.starts_with("/clusters/") {
        *entry.lock().await = None;
        ("204 No Content", Value::Null)
    } else {
        ("404 Not Found", json!({ "error": "not found" }))
    };
    let body = if response.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(&response).unwrap()
    };
    let header = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}
