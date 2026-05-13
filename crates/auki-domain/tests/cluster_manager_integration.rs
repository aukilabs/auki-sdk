//! Integration test for [`ClusterManager`] against a real running
//! Discovery service. `#[ignore]` by default — CI doesn't depend on
//! the live deployment.
//!
//! ## Running
//!
//! ```bash
//! DISCOVERY_URL=http://192.168.9.130:8080 \
//!   cargo test -p auki-domain --test cluster_manager_integration \
//!     -- --ignored --nocapture
//! ```
//!
//! What this test covers, end-to-end:
//!
//! 1. `ClusterManager::create_cluster` creates the cluster on
//!    Discovery, becomes the initial Manager, spawns the runtime.
//! 2. `is_manager()`, `manager_peer_id()`, `peer_count()` reflect
//!    the post-create state.
//! 3. `participant_info(...)` produces the daemon's `/api/info`
//!    JSON shape with cluster-aware fields populated.
//! 4. `admit_peer(...)` mutates the membership, updates the runtime
//!    allow-list, increments peer_count.
//! 5. The Manager heartbeat tick keeps Discovery's entry alive past
//!    the 10s sweep window.
//! 6. `shutdown()` deregisters from Discovery; a follow-up
//!    `list_clusters` confirms it's gone.

use auki_domain::{ClusterManager, DaemonInfo};
use auki_network::PeerIdentity;
use auki_network::discovery_client::DiscoveryClient;
use auki_network::stream_runtime::decline_all_streams;
use auki_network::swarm::{SwarmConfig, build_swarm};
use libp2p::Swarm;
use libp2p::swarm::SwarmEvent;
use multiaddr::Multiaddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn cluster_manager_full_lifecycle_against_live_discovery() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-mgr-it");

    let identity = PeerIdentity::from_seed(&[42u8; 32]);
    let local_peer_id = identity.peer_id();
    let mut swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-mgr-it/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let local_addr = wait_for_listen_addr(&mut swarm).await;
    let local_multiaddrs: Vec<Multiaddr> = vec![local_addr];

    // 1. Create cluster.
    let manager = ClusterManager::create_cluster(
        cluster_name.clone(),
        identity.clone(),
        local_multiaddrs.clone(),
        discovery.clone(),
        swarm,
        decline_all_streams(),
    )
    .await
    .expect("create_cluster succeeds");

    // 2. Post-create accessors.
    assert_eq!(manager.cluster_name(), cluster_name);
    assert_eq!(manager.local_peer_id(), local_peer_id);
    assert!(manager.is_manager(), "creator is the initial Manager");
    assert_eq!(manager.manager_peer_id(), local_peer_id);
    assert_eq!(manager.peer_count(), 1, "Manager is the sole member");

    // 3. participant_info shape.
    let info = manager.participant_info(DaemonInfo {
        app: "test-daemon".into(),
        name: "test".into(),
        session_id: "abc".into(),
        session_clock_id: "test/clock".into(),
        session_clock_hash: "h".into(),
        session_now_ns: 1,
        cluster_joined_at_ns: Some(1),
        app_instance: "deadbeef".into(),
    });
    assert!(info.is_manager);
    assert_eq!(info.manager_peer_id, local_peer_id.to_string());
    assert_eq!(info.peer_id, local_peer_id);

    // 4. admit_peer.
    let other_identity = PeerIdentity::from_seed(&[43u8; 32]);
    let other_peer_id = other_identity.peer_id();
    let other_multiaddrs: Vec<Multiaddr> = vec!["/ip4/127.0.0.1/tcp/40099".parse().unwrap()];
    let member = manager
        .admit_peer(other_peer_id, other_multiaddrs.clone())
        .await
        .expect("admit_peer succeeds");
    assert_eq!(member.peer_id, other_peer_id);
    assert_eq!(member.multiaddrs, other_multiaddrs);
    assert_eq!(manager.peer_count(), 2);

    // Duplicate admit rejected.
    let dup = manager.admit_peer(other_peer_id, other_multiaddrs).await;
    assert!(
        matches!(dup, Err(auki_domain::AdmitError::AlreadyMember(_))),
        "duplicate admit returns AlreadyMember; got {dup:?}"
    );

    // 5. Heartbeat keeps Discovery entry alive past the 10s sweep.
    //    Wait 12s, then list and confirm we're still there.
    eprintln!("waiting 12s to verify heartbeat keeps the cluster alive past Discovery's sweep…");
    tokio::time::sleep(Duration::from_secs(12)).await;
    let snapshot = discovery.list_clusters().await.expect("list_clusters");
    let entry = snapshot
        .iter()
        .find(|c| c.name == cluster_name)
        .expect("cluster still in directory after sweep window");
    assert_eq!(entry.peer_count, 2, "Discovery sees the updated peer_count");

    // 6. Shutdown. NOTE: we admitted a fake peer above (no real
    //    libp2p connection), so the Manager's liveness handler
    //    never receives a Lost for it — membership.peers.len()
    //    stays at 2, and `shutdown` correctly does NOT deregister
    //    (the design assumes a surviving real peer would take
    //    over). Explicit `deregister` to clean up Discovery.
    manager.shutdown().await.expect("shutdown succeeds");
    discovery
        .deregister(&cluster_name)
        .await
        .expect("explicit cleanup deregister");
    let after = discovery.list_clusters().await.expect("list_clusters after");
    assert!(
        !after.iter().any(|c| c.name == cluster_name),
        "cluster {cluster_name} still in directory after explicit deregister"
    );

    eprintln!("ClusterManager full lifecycle OK against {}", discovery_url());
}

/// Two-peer end-to-end: Manager `create_cluster`s a fresh cluster;
/// a second peer `join_cluster`s it; both peers see the same
/// `ClusterMembership` (Manager + joiner) at the end.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn two_managers_create_then_join_against_live_discovery() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-join-it");

    // --- Manager side (peer A: creates the cluster) ---
    let id_a = PeerIdentity::from_seed(&[71u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-join-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;

    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a.clone()],
        discovery.clone(),
        swarm_a,
        decline_all_streams(),
    )
    .await
    .expect("create_cluster A");

    assert!(manager_a.is_manager());
    assert_eq!(manager_a.peer_count(), 1);

    // --- Joiner side (peer B: joins A's cluster) ---
    let id_b = PeerIdentity::from_seed(&[72u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-join-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;

    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b.clone()],
        discovery.clone(),
        swarm_b,
        decline_all_streams(),
    )
    .await
    .expect("join_cluster B");

    // B is not the Manager; A is.
    assert!(!manager_b.is_manager(), "joiner is not the Manager");
    assert_eq!(
        manager_b.manager_peer_id(),
        pid_a,
        "joiner sees A as the Manager"
    );

    // Membership convergence: both peers see the same 2-member set.
    // Give A's handler a moment to push the updated allow-list back
    // through the runtime before checking peer_count on A's side
    // (the membership update is synchronous; the runtime
    // set_allowed_peers is fire-and-await but the join handler runs
    // concurrent with the assertion).
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(manager_a.peer_count(), 2, "Manager A sees the joiner");
    assert_eq!(manager_b.peer_count(), 2, "Joiner B sees the membership");

    // Verify membership shape — peer-ids match on both sides.
    let m_a = manager_a.membership();
    let m_b = manager_b.membership();
    let mut peers_a: Vec<libp2p_identity::PeerId> = m_a.peers.iter().map(|p| p.peer_id).collect();
    let mut peers_b: Vec<libp2p_identity::PeerId> = m_b.peers.iter().map(|p| p.peer_id).collect();
    peers_a.sort();
    peers_b.sort();
    assert_eq!(peers_a, peers_b, "both peers see the same peer-id set");
    assert!(peers_a.contains(&pid_a) && peers_a.contains(&pid_b));

    // --- Cleanup ---
    // B exits first; A is Manager and must see B's libp2p
    // disconnect + evict B from membership before A's own shutdown
    // is called. Give A a moment to process the eviction so that
    // A.shutdown finds peers.len() == 1 and deregisters cleanly.
    manager_b.shutdown().await.expect("B shutdown");
    tokio::time::sleep(Duration::from_millis(500)).await;
    manager_a.shutdown().await.expect("A shutdown");

    let after = discovery.list_clusters().await.expect("list after");
    assert!(
        !after.iter().any(|c| c.name == cluster_name),
        "cluster {cluster_name} still in directory after both shutdowns"
    );

    eprintln!(
        "Two-manager create + join OK against {}: A={pid_a} B={pid_b}",
        discovery_url()
    );
}

/// Failover: A creates cluster, B joins, A "dies" (clean shutdown
/// short of deregister), B detects the loss via the peer-side
/// heartbeat timeout, runs the election, promotes itself to
/// Manager, calls `rotate_manager` on Discovery, and starts the
/// Manager-side Discovery heartbeat tick.
///
/// To simulate A "dying" without it deregistering the cluster
/// first (which would be the graceful exit path), we abort A's
/// runtime via Rust's `Drop` without going through
/// `ClusterManager::shutdown` — that's the unclean-exit path.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn manager_failover_when_a_dies_b_takes_over() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-failover-it");

    // --- Peer A: creates the cluster ---
    let id_a = PeerIdentity::from_seed(&[81u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-failover-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;

    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a],
        discovery.clone(),
        swarm_a,
        decline_all_streams(),
    )
    .await
    .expect("create_cluster A");

    // --- Peer B: joins A's cluster ---
    let id_b = PeerIdentity::from_seed(&[82u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-failover-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;

    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b],
        discovery.clone(),
        swarm_b,
        decline_all_streams(),
    )
    .await
    .expect("join_cluster B");

    // Give heartbeats a moment to start flowing both ways.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!manager_b.is_manager(), "B starts as non-Manager");
    assert_eq!(manager_b.manager_peer_id(), pid_a);

    // --- A "dies" (drop without going through shutdown) ---
    eprintln!("A=({pid_a}) dying (drop without shutdown)…");
    drop(manager_a);

    // B detects the loss via the heartbeat-timeout monitor
    // (~1500ms) plus a margin for the election + rotate_manager
    // round-trip.
    eprintln!("waiting up to 5s for B to detect loss + run election + rotate Discovery…");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if manager_b.is_manager() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        manager_b.is_manager(),
        "B did not become Manager within 5s after A died"
    );
    assert_eq!(manager_b.manager_peer_id(), pid_b);

    // Verify Discovery sees the rotation: the cluster's
    // manager_peer_id should now be B's.
    let snapshot = discovery.list_clusters().await.expect("list_clusters");
    let entry = snapshot
        .iter()
        .find(|c| c.name == cluster_name)
        .expect("cluster still in directory after failover");
    assert_eq!(
        entry.manager_peer_id, pid_b,
        "Discovery's Manager hint rotated to B"
    );

    // --- Cleanup ---
    manager_b.shutdown().await.expect("B shutdown");
    let after = discovery.list_clusters().await.expect("list after");
    assert!(
        !after.iter().any(|c| c.name == cluster_name),
        "cluster {cluster_name} still in directory after B shutdown"
    );

    eprintln!(
        "Manager failover OK against {}: A={pid_a} died, B={pid_b} took over",
        discovery_url()
    );
}

/// Graceful Manager exit (`shutdown()`, not `drop`). The Manager
/// should NOT deregister the cluster from Discovery — instead the
/// surviving peer detects the libp2p disconnection, runs the
/// election, and calls `rotate_manager`. Symmetric with the
/// ungraceful path above; same code path per the Hagall design.
///
/// Regression test for the bug where graceful Manager shutdown
/// deregistered the cluster from Discovery, causing the surviving
/// peer's `rotate_manager` to 404.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn manager_graceful_shutdown_passes_cluster_to_surviving_peer() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-graceful-it");

    // --- Peer A: creates the cluster ---
    let id_a = PeerIdentity::from_seed(&[91u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-graceful-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;
    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a],
        discovery.clone(),
        swarm_a,
        decline_all_streams(),
    )
    .await
    .expect("create_cluster A");

    // --- Peer B: joins ---
    let id_b = PeerIdentity::from_seed(&[92u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-graceful-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;
    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b],
        discovery.clone(),
        swarm_b,
        decline_all_streams(),
    )
    .await
    .expect("join_cluster B");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // --- A graceful-exits via shutdown() (NOT drop) ---
    eprintln!("A=({pid_a}) calling shutdown() gracefully…");
    manager_a.shutdown().await.expect("A graceful shutdown");

    // B should detect, elect, promote, and rotate Discovery within
    // a few seconds.
    eprintln!("waiting up to 5s for B to take over after A's graceful exit…");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if manager_b.is_manager() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        manager_b.is_manager(),
        "B did not take over within 5s after A's graceful shutdown"
    );

    // Discovery should still have the cluster (A did NOT
    // deregister) and reflect B as the Manager.
    let snapshot = discovery.list_clusters().await.expect("list_clusters");
    let entry = snapshot
        .iter()
        .find(|c| c.name == cluster_name)
        .expect("cluster gone from Discovery — A wrongly deregistered on graceful exit?");
    assert_eq!(
        entry.manager_peer_id, pid_b,
        "Discovery's Manager hint should rotate to B after graceful handoff"
    );

    // --- Cleanup ---
    manager_b.shutdown().await.expect("B shutdown");
    let after = discovery.list_clusters().await.expect("list after");
    assert!(
        !after.iter().any(|c| c.name == cluster_name),
        "cluster {cluster_name} still in directory after B (last member) shutdown"
    );

    eprintln!(
        "Graceful Manager handoff OK against {}: A={pid_a} → B={pid_b}",
        discovery_url()
    );
}
