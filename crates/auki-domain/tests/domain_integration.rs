//! Integration test for [`auki_domain::Domain`] against a real running
//! Discovery service. `#[ignore]` by default — CI doesn't depend on the
//! live deployment.
//!
//! ## Running
//!
//! ```bash
//! DISCOVERY_URL=http://192.168.9.130:8080 \
//!   cargo test -p auki-domain --test domain_integration \
//!     -- --ignored --nocapture
//! ```
//!
//! What this covers that the pure `catalog_of` unit tests can't: the
//! `Domain::join` bootstrap path itself — it builds a `Domain` by composing a
//! `Peer` + `Session`, creates the cluster on Discovery, installs the
//! `SessionHandle` catalog bridge, and serves the catalog built from
//! `Peer.registries` + `Session.logs`. `leave()` then deregisters. See #274.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use auki_domain::{ClusterTarget, DaemonInfo, Domain, DomainConfig};
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn domain_join_creates_cluster_and_serves_catalog() {
    let tmp = tempfile::tempdir().unwrap();
    let cluster_name = unique_cluster_name("sdk-domain-it");

    // ── Peer + Session: register a sensor, start a session, register a log. ──
    let peer = Peer::new("galbot", "galbot-ctrl").with_storage_root(tmp.path().to_path_buf());
    let frame = peer
        .register_frame("head_left_camera_optical", FrameDef::ros_optical())
        .unwrap();
    let sensor = peer
        .register_sensor("head_left_rgb", camera_body(frame.clone()))
        .unwrap();

    let session = peer.start_session().unwrap();
    let clock = session.monotonic_clock();
    session
        .register_sensor_log(SensorLogSpec {
            sensor,
            clock: clock.clone(),
            frame: Some(frame),
            head: HeadSpec::Rolling {
                retention_ns: 5_000_000_000,
            },
            segment_duration: Duration::from_secs(1),
            retention: Duration::from_secs(5),
        })
        .unwrap();

    // ── Swarm + DomainConfig. ──
    let identity = PeerIdentity::from_seed(&[42u8; 32]);
    let local_peer_id = identity.peer_id();
    let mut swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-domain-it/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let local_addr = wait_for_listen_addr(&mut swarm).await;
    let local_multiaddrs: Vec<Multiaddr> = vec![local_addr];

    // DaemonInfo is stamped from the composed session — Domain joins as this
    // peer's current timeline.
    let daemon_info = DaemonInfo {
        app: "galbot-ctrl".into(),
        name: "galbot".into(),
        session_id: session.session_id(),
        session_clock_id: clock.id.clone(),
        session_clock_hash: clock.hash.clone(),
        app_instance: "deadbeef".into(),
    };

    // ── Domain::join — bootstraps the cluster + installs the catalog bridge. ──
    let domain = Domain::join(
        &peer,
        &session,
        DomainConfig {
            target: ClusterTarget::Create {
                name: cluster_name.clone(),
            },
            local_identity: identity,
            local_multiaddrs,
            discovery_url: discovery_url(),
            swarm,
            stream_provider: decline_all_streams(),
            daemon_info,
        },
    )
    .await
    .expect("Domain::join creates the cluster");

    // Cluster manager reflects the post-create state.
    let manager = domain.cluster_manager();
    assert_eq!(manager.cluster_name(), cluster_name);
    assert_eq!(manager.local_peer_id(), local_peer_id);
    assert!(manager.is_manager(), "creator is the initial Manager");
    assert_eq!(manager.peer_count(), 1, "Manager is the sole member");

    // The catalog served by the installed bridge reflects the session's log,
    // with kind/type resolved from the peer's sensor registry.
    let rows = domain.catalog();
    assert_eq!(rows.len(), 1, "one catalog row for the registered sensor log");
    let row = &rows[0];
    assert_eq!(row.source_peer_id, "galbot");
    assert_eq!(row.resource_id, "head_left_rgb");
    let sensor_block = row.sensor.as_ref().expect("sensor block present");
    assert_eq!(sensor_block.r#type, "rgb");

    // ── leave() deregisters from Discovery. ──
    domain.leave().await.expect("Domain::leave shuts down cleanly");
}
