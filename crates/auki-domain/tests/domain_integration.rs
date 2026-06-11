//! Integration tests for [`auki_domain::Domain`].
//!
//! `domain_join_creates_cluster_and_serves_catalog` is `#[ignore]` — it needs a
//! real running Discovery service:
//!
//! ```bash
//! DISCOVERY_URL=http://192.168.9.130:8080 \
//!   cargo test -p auki-domain --test domain_integration \
//!     -- --ignored --nocapture
//! ```
//!
//! It covers the `Domain::join` bootstrap path: composing a `Peer` + `Session`,
//! creating the cluster, installing the `SessionHandle` catalog bridge, serving
//! the catalog, and unifying the cluster's runtime clock with the session's
//! registered clock. The identity-guard test is hermetic (no Discovery). See #274.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use auki_domain::{ClusterTarget, DaemonInfo, Domain, DomainConfig, DomainError};
use auki_network::PeerIdentity;
use auki_network::stream_runtime::decline_all_streams;
use auki_network::swarm::{SwarmConfig, build_swarm};
use auki_registry::{Camera, SensorBody};
use auki_session::{FrameDef, HeadSpec, Peer, SensorLogSpec};
use libp2p::Swarm;
use libp2p::swarm::SwarmEvent;
use multiaddr::Multiaddr;

fn discovery_url() -> String {
    std::env::var("DISCOVERY_URL").unwrap_or_else(|_| "http://192.168.9.130:8080".to_string())
}

fn unique_cluster_name(prefix: &str) -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{ns}")
}

async fn wait_for_listen_addr(
    swarm: &mut Swarm<auki_network::swarm::Behaviour>,
) -> libp2p::Multiaddr {
    use futures::StreamExt;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                return address;
            }
        }
    })
    .await
    .expect("listen addr did not appear within timeout")
}

fn camera_body(frame: auki_registry::RegistryRef) -> SensorBody {
    SensorBody::Camera(Camera {
        r#type: "rgb".to_string(),
        width: 1920,
        height: 1200,
        frame_rate_hz: 30,
        pixel_format: "rgb8".to_string(),
        color_space: "srgb".to_string(),
        intrinsics_model: "pinhole".to_string(),
        distortion_model: "brown_conrady".to_string(),
        frame,
    })
}

fn swarm_for(identity: &PeerIdentity) -> Swarm<auki_network::swarm::Behaviour> {
    build_swarm(
        identity,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-domain-it/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn domain_join_creates_cluster_and_serves_catalog() {
    let tmp = tempfile::tempdir().unwrap();
    let cluster_name = unique_cluster_name("sdk-domain-it");

    // Identity first — the peer's id IS the local network identity (the SDK
    // convention `Domain::join` enforces).
    let identity = PeerIdentity::from_seed(&[42u8; 32]);
    let local_peer_id = identity.peer_id();

    // ── Peer + Session: register a sensor, start a session, register a log. ──
    let peer = Peer::new(local_peer_id.to_string(), "galbot-ctrl")
        .with_storage_root(tmp.path().to_path_buf());
    let frame = peer
        .register_frame("head_left_camera_optical", FrameDef::ros_optical())
        .unwrap();
    let sensor = peer
        .register_sensor("head_left_rgb", camera_body(frame.clone()))
        .unwrap();

    let session = peer.start_session().unwrap();
    session
        .register_sensor_log(SensorLogSpec {
            sensor,
            clock: session.monotonic_clock(),
            frame: Some(frame),
            head: HeadSpec::Rolling {
                retention_ns: 5_000_000_000,
            },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        })
        .unwrap();

    // ── Swarm + DomainConfig. The session-clock fields in DaemonInfo are
    // placeholders on purpose — Domain::join overwrites them from the session. ──
    let mut swarm = swarm_for(&identity);
    let local_addr = wait_for_listen_addr(&mut swarm).await;
    let daemon_info = DaemonInfo {
        app: "galbot-ctrl".into(),
        name: "galbot".into(),
        session_id: "PLACEHOLDER".into(),
        session_clock_id: "PLACEHOLDER".into(),
        session_clock_hash: "PLACEHOLDER".into(),
        app_instance: "deadbeef".into(),
    };

    let domain = Domain::join(
        &peer,
        &session,
        DomainConfig {
            target: ClusterTarget::Create {
                name: cluster_name.clone(),
            },
            local_identity: identity,
            local_multiaddrs: vec![local_addr],
            discovery_url: discovery_url(),
            swarm,
            stream_provider: decline_all_streams(),
            daemon_info,
        },
    )
    .await
    .expect("Domain::join creates the cluster");

    let manager = domain.cluster_manager();
    assert_eq!(manager.cluster_name(), cluster_name);
    assert_eq!(manager.local_peer_id(), local_peer_id);
    assert!(manager.is_manager(), "creator is the initial Manager");

    // Unification: the cluster advertises exactly the session's registered
    // clock — DaemonInfo placeholders were overwritten from the session, and
    // the runtime SessionClock reconstructs the identical registry entry.
    let info = manager.participant_info();
    let mono = session.monotonic_clock();
    assert_eq!(info.session_id, session.session_id());
    assert_eq!(
        info.session_clock_id, mono.id,
        "advertised clock id == registered"
    );
    assert_eq!(
        info.session_clock_hash, mono.hash,
        "advertised clock hash == registered"
    );

    // Catalog served by the installed bridge reflects the session's log.
    let rows = domain.catalog();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].resource_id, "head_left_rgb");
    assert_eq!(rows[0].sensor.as_ref().unwrap().r#type, "rgb");

    domain
        .leave()
        .await
        .expect("Domain::leave shuts down cleanly");
}

/// Hermetic (no Discovery): `Domain::join` rejects a peer whose id isn't the
/// local network identity, before any network I/O. The swarm is constructed
/// but never polled, so no socket is bound.
#[tokio::test]
async fn join_rejects_peer_id_not_matching_local_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let identity = PeerIdentity::from_seed(&[7u8; 32]);

    // peer id is a friendly string, deliberately NOT the libp2p peer id.
    let peer = Peer::new("not-the-network-id", "app").with_storage_root(tmp.path().to_path_buf());
    let session = peer.start_session().unwrap();

    let result = Domain::join(
        &peer,
        &session,
        DomainConfig {
            target: ClusterTarget::Create { name: "x".into() },
            local_identity: identity.clone(),
            local_multiaddrs: Vec::<Multiaddr>::new(),
            discovery_url: "http://127.0.0.1:0".into(),
            swarm: swarm_for(&identity),
            stream_provider: decline_all_streams(),
            daemon_info: DaemonInfo {
                app: "app".into(),
                name: "n".into(),
                session_id: String::new(),
                session_clock_id: String::new(),
                session_clock_hash: String::new(),
                app_instance: "ai".into(),
            },
        },
    )
    .await;

    match result {
        Err(DomainError::IdentityMismatch { peer, identity: id }) => {
            assert_eq!(peer, "not-the-network-id");
            assert_eq!(id, identity.peer_id().to_string());
        }
        Err(other) => panic!("expected IdentityMismatch, got {other:?}"),
        Ok(_) => panic!("expected IdentityMismatch, got Ok(Domain)"),
    }
}
