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
//! 3. `participant_info(...)` produces the daemon's `ParticipantInfo`
//!    JSON shape with cluster-aware fields populated.
//! 4. `admit_peer(...)` mutates the membership, updates the runtime
//!    allow-list, increments peer_count.
//! 5. The Manager heartbeat tick keeps Discovery's entry alive past
//!    the 10s sweep window.
//! 6. `shutdown()` deregisters from Discovery; a follow-up
//!    `list_clusters` confirms it's gone.

use auki_domain::ClusterManager;
use auki_network::PeerIdentity;
use auki_network::discovery_client::DiscoveryClient;
use auki_network::stream_runtime::decline_all_streams;
use auki_network::swarm::{SwarmConfig, build_swarm};
use libp2p::Swarm;
use libp2p::swarm::SwarmEvent;
use multiaddr::Multiaddr;
use std::sync::Arc;
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

/// Build a minimal `DaemonInfo` with the static fields populated.
/// `session_now_ns` and `cluster_joined_at_ns` are no longer on
/// `DaemonInfo`; the SDK computes them from `SessionClock` plus
/// observed membership. The clock id/hash fields remain compatibility
/// inputs until callers stop supplying session clock identity.
fn sample_daemon_info(name: &str) -> auki_domain::DaemonInfo {
    auki_domain::DaemonInfo {
        app: "test-daemon".into(),
        name: name.into(),
        session_id: "abc".into(),
        session_clock_id: format!("{name}/clock"),
        session_clock_hash: "h".into(),
        app_instance: "deadbeef".into(),
    }
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

/// #295 recovery-latency budget for a clean-kill Manager failover.
/// The survivor's first heartbeat timeout fires ~1.5–2.25s after the
/// Manager dies; the Discovery gate still sees the dead Manager's row
/// (3s sweep window) and **Defers** — one Defer cycle includes a
/// rejoin attempt toward the dead peer with up to a 5s connect wait —
/// then the re-armed watch times out again, finds the row swept,
/// elects, and promotes only after `register_as_manager` commits.
/// Typical end-to-end: ~8–10s. Budget 25s for LAN + Discovery jitter.
const TAKEOVER_DEADLINE: Duration = Duration::from_secs(25);

/// Discovery's server-side row-liveness sweep window
/// (`LIVENESS_REQUIREMENT_NS`): a row whose Manager stops POSTing
/// `liveness_check` is evicted ~3s after the last refresh. Verified
/// against the live deployment (probe row swept between +3s and +4s).
const DISCOVERY_SWEEP_WINDOW: Duration = Duration::from_secs(3);

/// Fetch the cluster's current Discovery row, if any.
async fn cluster_row(
    discovery: &DiscoveryClient,
    name: &str,
) -> Option<auki_network::discovery_client::ClusterEntry> {
    discovery
        .list_clusters()
        .await
        .expect("list_clusters")
        .into_iter()
        .find(|c| c.name == name)
}

/// Block until the row's `last_liveness_check_ns` advances past its
/// current value — i.e. until the Manager's 1s liveness tick has just
/// refreshed it. Killing the Manager immediately afterwards pins the
/// row sweep to ~`DISCOVERY_SWEEP_WINDOW` after the kill, which makes
/// "the survivor must not promote before the sweep" lower bounds
/// meaningful (#295).
async fn wait_for_fresh_liveness_refresh(discovery: &DiscoveryClient, name: &str) {
    let initial = cluster_row(discovery, name)
        .await
        .expect("cluster row exists while its Manager is alive")
        .last_liveness_check_ns;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "Manager liveness tick did not refresh the Discovery row within 3s"
        );
        if let Some(row) = cluster_row(discovery, name).await {
            if row.last_liveness_check_ns > initial {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
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
        discovery_url(),
        swarm,
        decline_all_streams(),
        sample_daemon_info("test"),
    )
    .await
    .expect("create_cluster succeeds");

    // 2. Post-create accessors.
    assert_eq!(manager.cluster_name(), cluster_name);
    assert_eq!(manager.local_peer_id(), local_peer_id);
    assert!(manager.is_manager(), "creator is the initial Manager");
    assert_eq!(manager.manager_peer_id(), local_peer_id);
    assert_eq!(manager.peer_count(), 1, "Manager is the sole member");

    // 3. participant_info shape — built from stored `DaemonInfo`
    //    (passed to `create_cluster`) + SDK-tracked dynamic fields.
    let info = manager.participant_info();
    assert!(info.is_manager);
    assert_eq!(info.manager_peer_id, local_peer_id.to_string());
    assert_eq!(info.peer_id, local_peer_id);
    assert_eq!(info.app, "test-daemon");
    assert!(
        info.session_now_ns > 0,
        "session_now_ns advances after construction"
    );
    assert!(
        info.cluster_joined_at_ns.is_none(),
        "alone in cluster — cluster_joined_at_ns stays None per ansuz D3"
    );

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
    //    stays at 2, and `shutdown` correctly skips the Discovery
    //    DELETE (the design assumes a surviving real peer would
    //    take over). Explicit `deregister` for test cleanup.
    manager.shutdown().await.expect("shutdown succeeds");
    discovery
        .deregister(cluster_name.clone())
        .await
        .expect("explicit cleanup deregister");
    let after = discovery
        .list_clusters()
        .await
        .expect("list_clusters after");
    assert!(
        !after.iter().any(|c| c.name == cluster_name),
        "cluster {cluster_name} still in directory after explicit deregister"
    );

    eprintln!(
        "ClusterManager full lifecycle OK against {}",
        discovery_url()
    );
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
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test"),
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
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test"),
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
    // checks `peers.len()`. The 500ms breath gives A's liveness
    // handler time to process ConnectionClosed → evict → so that
    // A.shutdown observes peers.len() == 1 and deregisters cleanly.
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
/// Manager, commits the claim at Discovery, and starts the
/// Manager-side Discovery liveness tick.
///
/// To simulate A "dying" without it deregistering the cluster
/// first (which would be the graceful exit path), we abort A's
/// runtime via Rust's `Drop` without going through
/// `ClusterManager::shutdown` — that's the unclean-exit path.
///
/// Re-timed for #295: detection still fires at ~1.5s, but B no
/// longer promotes immediately — while Discovery's row still names
/// dead A, B Defers (rejoin attempt + re-armed watch) and only
/// elects once the row is swept (~3s after A's last liveness tick).
/// See `TAKEOVER_DEADLINE` for the arithmetic; the dedicated
/// sweep-bound regression is `dead_manager_drop_is_replaced_only_
/// after_row_sweep`.
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
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test"),
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
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test"),
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

    // B detects the loss via the domain-owned heartbeat timer
    // (~1500ms). Under #295 the takeover then legitimately waits for
    // Discovery: the row still names dead A, so B Defers (rejoin
    // attempt toward A incl. up to a 5s dead-dial wait, re-armed
    // watch) until the row sweeps ~3s after A's last liveness tick,
    // and only then elects + commits. Typical ~8–10s end to end.
    eprintln!(
        "waiting up to {TAKEOVER_DEADLINE:?} for B to defer until the row sweep, \
         elect, and commit at Discovery…"
    );
    let deadline = std::time::Instant::now() + TAKEOVER_DEADLINE;
    while std::time::Instant::now() < deadline {
        if manager_b.is_manager() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        manager_b.is_manager(),
        "B did not become Manager within {TAKEOVER_DEADLINE:?} after A died \
         (#295 defer-until-sweep failover)"
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

/// Domain-clock continuity across Manager handoff:
///
/// 1. A creates the cluster and advertises A/session-clock as the
///    domain-clock backing source at offset 0.
/// 2. B joins, receives A's heartbeat domain-clock metadata, and
///    accumulates heartbeat NTP samples until `B.domain_clock_estimate()`
///    becomes available.
/// 3. A dies.
/// 4. B promotes to Manager and advertises B/session-clock as the
///    new domain-clock backing source with B's inherited domain offset.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn domain_clock_metadata_survives_manager_handoff() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-domain-clock-handoff-it");

    // --- Peer A: creates the cluster and starts as domain backing source. ---
    let id_a = PeerIdentity::from_seed(&[151u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-domain-clock-handoff-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;
    let daemon_a = sample_daemon_info("domain-A");

    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        daemon_a.clone(),
    )
    .await
    .expect("create_cluster A");

    let initial_domain = manager_a
        .domain_clock_estimate()
        .expect("initial Manager has identity domain clock");
    assert_eq!(initial_domain.backing_peer_id, pid_a.to_string());
    assert_eq!(initial_domain.backing_clock_id, daemon_a.session_clock_id);
    assert_eq!(initial_domain.total_offset_ns, 0);

    // --- Peer B: joins and learns A-backed domain time from heartbeat. ---
    let id_b = PeerIdentity::from_seed(&[152u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-domain-clock-handoff-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;
    let daemon_b = sample_daemon_info("domain-B");

    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        daemon_b.clone(),
    )
    .await
    .expect("join_cluster B");
    assert!(!manager_b.is_manager(), "B starts as non-Manager");
    assert_eq!(manager_b.manager_peer_id(), pid_a);

    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    let b_domain_before = loop {
        if let Ok(estimate) = manager_b.domain_clock_estimate() {
            break estimate;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "B did not acquire A-backed domain time within 6s"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(b_domain_before.backing_peer_id, pid_a.to_string());
    assert_eq!(b_domain_before.backing_clock_id, daemon_a.session_clock_id);
    assert_eq!(b_domain_before.local_clock_id, daemon_b.session_clock_id);

    // --- A dies; B should promote and republish itself as backing source. ---
    eprintln!("A=({pid_a}) dropping after B has domain time…");
    drop(manager_a);

    // Re-timed for #295: B defers while Discovery's row still names
    // dead A and only elects after the ~3s row sweep (see
    // `TAKEOVER_DEADLINE`), so the handoff lands at ~8–10s, not ~2s.
    let deadline = std::time::Instant::now() + TAKEOVER_DEADLINE;
    while std::time::Instant::now() < deadline {
        if manager_b.is_manager() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        manager_b.is_manager(),
        "B did not become Manager within {TAKEOVER_DEADLINE:?} after A died \
         (#295 defer-until-sweep failover)"
    );
    assert_eq!(manager_b.manager_peer_id(), pid_b);

    let b_domain_after = manager_b
        .domain_clock_estimate()
        .expect("promoted B should still have domain time");
    assert_eq!(b_domain_after.backing_peer_id, pid_b.to_string());
    assert_eq!(b_domain_after.backing_clock_id, daemon_b.session_clock_id);
    assert_eq!(
        b_domain_after.total_offset_ns, b_domain_after.backing_to_domain_offset_ns,
        "B-backed estimate should be identity into its new backing source plus inherited source offset"
    );

    manager_b.shutdown().await.expect("B shutdown");
    let after = discovery.list_clusters().await.expect("list after");
    if after.iter().any(|c| c.name == cluster_name) {
        discovery
            .deregister(cluster_name.clone())
            .await
            .expect("explicit cleanup deregister");
    }

    eprintln!(
        "Domain-clock handoff OK against {}: A={pid_a} → B={pid_b}",
        discovery_url()
    );
}

/// Three-peer membership convergence via `/auki/membership/0.0.1`:
/// A creates cluster, B joins, then C joins. After all joins settle,
/// peer B's local membership must contain A + B + C — proving the
/// Manager's post-admit broadcast reached B when C joined.
///
/// Without gossip, B's snapshot is frozen at join time (A + B only)
/// and B's `/auki/stream/0.1.0` allow-list never includes C — i.e.
/// the demo's step 14 would silently drop Charlie-Park's substreams
/// to existing peers. This test pins the convergence behaviour.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn three_peer_membership_converges_via_gossip() {
    let cluster_name = unique_cluster_name("sdk-gossip-it");

    // --- Peer A creates ---
    let id_a = PeerIdentity::from_seed(&[81u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-gossip-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;
    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a.clone()],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test"),
    )
    .await
    .expect("create_cluster A");
    assert!(manager_a.is_manager());

    // --- Peer B joins ---
    let id_b = PeerIdentity::from_seed(&[82u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-gossip-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let _addr_b = wait_for_listen_addr(&mut swarm_b).await;
    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![_addr_b.clone()],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test"),
    )
    .await
    .expect("join_cluster B");
    assert_eq!(manager_b.manager_peer_id(), pid_a);

    // B settles at 2 members (itself + A) via the JoinResponse::Accept
    // snapshot. The gossip path is exercised by the NEXT join.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(manager_b.peer_count(), 2, "B sees A+B after its own join");

    // --- Peer C joins ---
    let id_c = PeerIdentity::from_seed(&[83u8; 32]);
    let pid_c = id_c.peer_id();
    let mut swarm_c = build_swarm(
        &id_c,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-gossip-it-C/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let _addr_c = wait_for_listen_addr(&mut swarm_c).await;
    let manager_c = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_c.clone(),
        vec![_addr_c.clone()],
        discovery_url(),
        swarm_c,
        decline_all_streams(),
        sample_daemon_info("test"),
    )
    .await
    .expect("join_cluster C");
    assert_eq!(manager_c.manager_peer_id(), pid_a);
    assert_eq!(
        manager_c.peer_count(),
        3,
        "C sees A+B+C in its admit snapshot"
    );

    // The gossip-blocking assertion: B should converge to 3 members
    // within a few seconds via the membership broadcast from A
    // (post-admit broadcast in `ClusterManager::admit_peer`).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut b_saw_three = false;
    while std::time::Instant::now() < deadline {
        if manager_b.peer_count() == 3 {
            b_saw_three = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        b_saw_three,
        "peer B did not converge to 3-member membership within 5s — \
         gossip did not propagate (peer_count = {})",
        manager_b.peer_count()
    );

    // Verify B's membership shape — sorted peer-ids match all three.
    let m_b = manager_b.membership();
    let mut peers_b: Vec<libp2p_identity::PeerId> = m_b.peers.iter().map(|p| p.peer_id).collect();
    peers_b.sort();
    let mut expected = vec![pid_a, pid_b, pid_c];
    expected.sort();
    assert_eq!(
        peers_b, expected,
        "B's membership should contain A + B + C after gossip"
    );

    // --- Cleanup ---
    manager_c.shutdown().await.expect("C shutdown");
    manager_b.shutdown().await.expect("B shutdown");
    manager_a.shutdown().await.expect("A shutdown");

    eprintln!(
        "3-peer gossip convergence OK against {}: A={pid_a}, B={pid_b}, C={pid_c}",
        discovery_url()
    );
}

/// Cross-fetch ParticipantInfo over `/auki/info/0.0.1`: A creates,
/// B joins with a different `app` / `name`. After cluster
/// convergence, A calls `manager_a.fetch_participant_info(pid_b)`
/// and gets back B's full `ParticipantInfo` (app, name, peer_id,
/// is_manager, manager_peer_id). And vice versa from B's side.
///
/// Pre-Hagall this used mDNS + HTTP `/api/info`. Post-Hagall it's
/// libp2p-only, cluster-trust-gated. This test pins the new wire.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn cluster_peers_fetch_each_other_participant_info_over_libp2p() {
    let cluster_name = unique_cluster_name("sdk-info-it");

    // A — the "park"-like daemon, creates the cluster.
    let id_a = PeerIdentity::from_seed(&[91u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-info-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;
    let daemon_a = auki_domain::DaemonInfo {
        app: "park".into(),
        name: "nils-park".into(),
        session_id: "session-a".into(),
        session_clock_id: "park-aabbcc/session-monotonic".into(),
        session_clock_hash: "ha".into(),
        app_instance: "aabbcc".into(),
    };
    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a.clone()],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        daemon_a,
    )
    .await
    .expect("create_cluster A");

    // B — the "boosterapp"-like daemon, joins.
    let id_b = PeerIdentity::from_seed(&[92u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-info-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let _addr_b = wait_for_listen_addr(&mut swarm_b).await;
    let daemon_b = auki_domain::DaemonInfo {
        app: "boosterapp".into(),
        name: "walker-1".into(),
        session_id: "session-b".into(),
        session_clock_id: "K1-aabbccddee/session-monotonic".into(),
        session_clock_hash: "hb".into(),
        app_instance: "aabbccddee".into(),
    };
    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![_addr_b],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        daemon_b,
    )
    .await
    .expect("join_cluster B");

    // A fetches B's ParticipantInfo over /auki/info/0.0.1.
    let b_info_from_a = manager_a
        .fetch_participant_info(pid_b)
        .await
        .expect("A fetches B's ParticipantInfo");
    assert_eq!(b_info_from_a.peer_id, pid_b);
    assert_eq!(b_info_from_a.app, "boosterapp");
    assert_eq!(b_info_from_a.name, "walker-1");
    assert_eq!(b_info_from_a.session_id, "session-b");
    assert_eq!(b_info_from_a.app_instance, "aabbccddee");
    assert!(!b_info_from_a.is_manager, "B is not the Manager");
    assert_eq!(b_info_from_a.manager_peer_id, pid_a.to_string());
    assert!(
        b_info_from_a.session_now_ns > 0,
        "B's session clock advances"
    );

    // B fetches A's ParticipantInfo over /auki/info/0.0.1.
    let a_info_from_b = manager_b
        .fetch_participant_info(pid_a)
        .await
        .expect("B fetches A's ParticipantInfo");
    assert_eq!(a_info_from_b.peer_id, pid_a);
    assert_eq!(a_info_from_b.app, "park");
    assert_eq!(a_info_from_b.name, "nils-park");
    assert!(a_info_from_b.is_manager, "A is the Manager");
    assert_eq!(a_info_from_b.manager_peer_id, pid_a.to_string());

    manager_b.shutdown().await.expect("B shutdown");
    manager_a.shutdown().await.expect("A shutdown");

    eprintln!(
        "Cross-fetch ParticipantInfo OK against {}: A={pid_a} (park/nils-park), B={pid_b} (boosterapp/walker-1)",
        discovery_url()
    );
}

/// Cross-fetch a Frame Registry entry over `/auki/registries/0.0.1`:
/// A creates, B joins. B writes an existing app-root
/// `FrameRegistryEntry`, registers that app root with the
/// `ClusterManager`, and A fetches the exact `(frame_id, frame_hash)`.
///
/// This pins the metadata-resolution layer Park needs after it sees a
/// stream manifest or catalog row with `frame_id + frame_hash`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn cluster_peers_fetch_frame_registry_entry_over_libp2p() {
    use auki_domain::FetchRegistryEntryError;
    use auki_registry::{FrameRegistryEntry, write_frame};

    let cluster_name = unique_cluster_name("sdk-registries-it");

    // A — Park-like consumer, creates the cluster.
    let id_a = PeerIdentity::from_seed(&[103u8; 32]);
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-registries-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;
    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("park-a"),
    )
    .await
    .expect("create_cluster A");

    // B — Booster-like producer, joins and writes a frame registry entry.
    let id_b = PeerIdentity::from_seed(&[104u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-registries-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;
    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("booster-b"),
    )
    .await
    .expect("join_cluster B");

    let app_root = tempfile::tempdir().expect("temp app root");
    let frame = FrameRegistryEntry::ros_optical("K1-FAKE", "K1-FAKE/head_cam_points");
    let frame_hash = write_frame(app_root.path(), &frame)
        .expect("write frame entry")
        .hash()
        .to_string();
    manager_b.set_registry_app_root(app_root.path());

    let fetched = manager_a
        .fetch_frame_entry(pid_b, frame.frame_id.clone(), frame_hash.clone())
        .await
        .expect("A fetches B's frame entry");
    assert_eq!(fetched, frame);

    let missing = manager_a
        .fetch_frame_entry(pid_b, frame.frame_id.clone(), "deadbeef".to_string())
        .await
        .expect_err("wrong hash should be a clean not-found");
    assert!(
        matches!(
            missing,
            FetchRegistryEntryError::NotFound {
                kind: auki_network::registries_protocol::RegistryKind::Frame,
                ..
            }
        ),
        "expected NotFound for missing hash, got {missing:?}"
    );

    manager_b.shutdown().await.expect("B shutdown");
    manager_a.shutdown().await.expect("A shutdown");

    eprintln!(
        "Cross-fetch frame registry entry OK against {}: B={pid_b} served {}@{}",
        discovery_url(),
        frame.frame_id,
        frame_hash
    );
}

/// Park-side ergonomics: a daemon's stream consumers may hold
/// `Arc<ClusterManager>` clones (per Park's stream-provider closure
/// shape, see boosterapp-clone-fan-out + Park's tile consumers).
/// Shutdown has to work when ANY of those clones calls it — the
/// daemon shouldn't have to first drain every clone to recover
/// unique ownership before the heartbeat tick + Discovery DELETE
/// can run.
///
/// Pins the SDK-T2/T7 daemon-lifecycle fix that closed the "ghost
/// cluster on Discovery" leak: a `.shutdown()` from any Arc clone
/// (1) aborts every background task (heartbeat + handler tasks),
/// (2) DELETEs the cluster on Discovery, (3) flips `stopped` so
/// concurrent / repeat callers fast-fail rather than re-issuing
/// the Discovery DELETE, (4) leaves other live clones drop-safe.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn shutdown_via_arc_clone_deregisters_and_remains_idempotent() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-arc-shutdown-it");

    let identity = PeerIdentity::from_seed(&[55u8; 32]);
    let mut swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-arc-shutdown-it/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let local_addr = wait_for_listen_addr(&mut swarm).await;

    let manager = ClusterManager::create_cluster(
        cluster_name.clone(),
        identity.clone(),
        vec![local_addr],
        discovery_url(),
        swarm,
        decline_all_streams(),
        sample_daemon_info("arc-shutdown"),
    )
    .await
    .expect("create_cluster succeeds");

    // Stand in for Park's tile-consumer fan-out: hold multiple
    // Arc clones, drop the "original" handle, then call shutdown
    // through a clone.
    let manager = Arc::new(manager);
    let consumer_clone = manager.clone();
    let leftover_clone = manager.clone();
    drop(manager);

    // Sanity: pre-shutdown, Discovery sees the cluster.
    let pre = discovery.list_clusters().await.expect("list_clusters pre");
    assert!(
        pre.iter().any(|c| c.name == cluster_name),
        "pre-shutdown cluster {cluster_name} should be in Discovery"
    );

    // Shutdown through a stream-consumer Arc clone — the Park bug
    // scenario in reverse.
    consumer_clone
        .shutdown()
        .await
        .expect("shutdown via Arc clone succeeds");

    // Discovery DELETE landed: cluster is gone within the same
    // call (shutdown awaits the DELETE before returning).
    let post = discovery.list_clusters().await.expect("list_clusters post");
    assert!(
        !post.iter().any(|c| c.name == cluster_name),
        "cluster {cluster_name} still in Discovery after shutdown via Arc clone"
    );

    // The second clone is still live and drop-safe; calling
    // shutdown on it is idempotent (Discovery isn't DELETEd
    // twice, no panic).
    leftover_clone
        .shutdown()
        .await
        .expect("second shutdown via leftover Arc clone is idempotent");

    // Post-shutdown I/O calls fast-fail with `Stopped` — Park's
    // stream consumers holding a stale clone get a clean signal
    // rather than a cascading runtime-channel-closed error.
    let other_peer_id = PeerIdentity::from_seed(&[56u8; 32]).peer_id();
    let dummy_multiaddrs: Vec<Multiaddr> = vec!["/ip4/127.0.0.1/tcp/40099".parse().unwrap()];
    let admit_err = leftover_clone
        .admit_peer(other_peer_id, dummy_multiaddrs)
        .await
        .expect_err("admit_peer after shutdown must error");
    assert!(
        matches!(admit_err, auki_domain::AdmitError::Stopped),
        "admit_peer after shutdown should return AdmitError::Stopped; got {admit_err:?}"
    );

    let fetch_err = leftover_clone
        .fetch_participant_info(other_peer_id)
        .await
        .expect_err("fetch_participant_info after shutdown must error");
    assert!(
        matches!(fetch_err, auki_domain::FetchParticipantInfoError::Stopped),
        "fetch_participant_info after shutdown should return Stopped; got {fetch_err:?}"
    );

    // Drop the last Arc — runtime + tasks already torn down by
    // shutdown, so Drop is a no-op (no panic, no hang).
    drop(leftover_clone);

    eprintln!(
        "Arc-clone shutdown ergonomics OK against {}",
        discovery_url()
    );
}

/// Regression test for the "Manager leaves → cluster closes → no
/// successor elected" bug. `ClusterManager::shutdown()` used to
/// unconditionally call `discovery.deregister(...)` when the local
/// peer was the Manager, nuking the cluster from Discovery before
/// the surviving peer's election could `rotate_manager`. Per the
/// Hagall design ("graceful and ungraceful Manager exits are the
/// same code path — peers detect the loss + run the election +
/// rotate"), the Manager should NOT deregister on graceful exit
/// when other peers can take over.
///
/// Scenario: A creates, B joins, A calls `shutdown()` GRACEFULLY
/// (not `drop`). Verifies (1) B detects A's libp2p disconnect, (2)
/// B runs election and promotes itself, (3) Discovery's directory
/// still has the cluster (A did NOT deregister) AND its
/// manager_peer_id reflects B.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn manager_graceful_shutdown_passes_cluster_to_surviving_peer() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-graceful-it");

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
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test-A"),
    )
    .await
    .expect("create_cluster A");

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
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test-B"),
    )
    .await
    .expect("join_cluster B");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // --- A graceful-exits via shutdown() (NOT drop) ---
    // `shutdown()` deregisters from Discovery only when the local
    // peer is the LAST member; with B remaining it must leave the
    // row in place (naming A) and stop ticking it. Sync the shutdown
    // to a fresh liveness refresh so the row sweep lands a full
    // `DISCOVERY_SWEEP_WINDOW` later — that pins the lower bound
    // asserted below (#295).
    wait_for_fresh_liveness_refresh(&discovery, &cluster_name).await;
    eprintln!("A=({pid_a}) calling shutdown() gracefully…");
    let shutdown_at = std::time::Instant::now();
    manager_a.shutdown().await.expect("A graceful shutdown");

    // Re-timed for #295: B defers while the row still names A and
    // elects only after the ~3s sweep — graceful and ungraceful
    // Manager exits share the loss path, so the takeover lands at
    // ~8–10s, not ~2s.
    eprintln!("waiting up to {TAKEOVER_DEADLINE:?} for B to take over after A's graceful exit…");
    let deadline = std::time::Instant::now() + TAKEOVER_DEADLINE;
    let mut took_over_after: Option<Duration> = None;
    while std::time::Instant::now() < deadline {
        if manager_b.is_manager() {
            took_over_after = Some(shutdown_at.elapsed());
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let took_over_after = took_over_after.unwrap_or_else(|| {
        panic!(
            "B did not take over within {TAKEOVER_DEADLINE:?} after A's graceful shutdown \
             (#295 defer-until-sweep failover)"
        )
    });
    eprintln!("B took over {took_over_after:?} after A's graceful shutdown");

    // #295 twist: since B's commit path re-creates a swept row
    // (rotate → 404 → create), a wrongly-deregistering A would no
    // longer show up as a *missing* row — it would show up as a
    // takeover faster than the sweep window (the row would already
    // be absent at B's first gate check, ~1.5–2.25s). The lower
    // bound is what now pins "A left the row in place". The 200ms
    // slack absorbs the one-RTT skew between Discovery stamping the
    // refresh and the test observing it.
    assert!(
        took_over_after >= DISCOVERY_SWEEP_WINDOW - Duration::from_millis(200),
        "B took over {took_over_after:?} after A's graceful shutdown — before Discovery's \
         {DISCOVERY_SWEEP_WINDOW:?} sweep window. A's shutdown() must have deregistered the \
         row despite B remaining (the pre-Hagall graceful-exit bug)."
    );

    // Discovery still has the cluster, now naming B. (The row may
    // have been swept and re-created by B's commit in between —
    // presence + manager==B is the post-handoff invariant.)
    let snapshot = discovery.list_clusters().await.expect("list_clusters");
    let entry = snapshot
        .iter()
        .find(|c| c.name == cluster_name)
        .expect("cluster gone from Discovery after B's takeover commit");
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

/// **Regression test for the QUIC-transport handoff bug** Nils reported
/// on K1 + Park on 2026-05-14 against SDK v0.0.37 (commit `b6fab53`):
/// Park as Manager, Booster joins, Park leaves — Booster's view freezes
/// indefinitely because libp2p `ConnectionClosed` doesn't fire for
/// minutes on QUIC (peer-side detection has to wait for QUIC's idle
/// timeout when the Manager exits without a close frame).
///
/// Root cause: the peer-side `/auki/heartbeat/0.0.1` substream — which
/// should detect Manager death in 1500 ms regardless of transport —
/// never opened. When the Manager admits a peer via the join protocol,
/// the Manager-side `ConnectionEstablished` for the joiner has ALREADY
/// fired (the joiner dialed in to send the join handshake); at that
/// moment the joiner wasn't in `known_peers`, so the heartbeat-spawn
/// branch skipped. After the admit, `apply_peer_update` retroactively
/// recognises the connection but — pre-fix — didn't catch the missed
/// heartbeat spawn. The bug was masked over TCP (fast RST on swarm
/// drop) but exposed over QUIC.
///
/// Test shape: bind A + B to QUIC loopback (not TCP — the previous
/// integration tests use TCP and pass even pre-fix because
/// ConnectionClosed fires within ms). With QUIC + the
/// pre-fix-missing-heartbeat substream, drop(A) would leave B waiting
/// for QUIC's multi-second idle timeout. Post-fix, B's loss DETECTION
/// fires in <2 s via the domain-owned heartbeat timer; under #295 the
/// takeover then completes only after Discovery's ~3s row sweep +
/// a Defer cycle (see `TAKEOVER_DEADLINE`) — still far inside the
/// tens-of-seconds QUIC idle window this test guards against.
///
/// Seeds `[81]` (Manager) and `[82]` (Joiner) pin `pid_a > pid_b`.
/// The Manager now opens the heartbeat regardless of peer-id ordering,
/// so this guards against regressing into "the lower peer happens to
/// save us" behaviour.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn manager_failover_over_quic_when_joiner_pid_lower() {
    let cluster_name = unique_cluster_name("sdk-quic-jpidlow-it");

    let id_a = PeerIdentity::from_seed(&[81u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
            agent_version: "sdk-quic-jpidlow-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;

    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test-A"),
    )
    .await
    .expect("create_cluster A");

    let id_b = PeerIdentity::from_seed(&[82u8; 32]);
    let pid_b = id_b.peer_id();
    assert!(pid_a > pid_b, "fixture pins Manager pid higher than Joiner");
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
            agent_version: "sdk-quic-jpidlow-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;

    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test-B"),
    )
    .await
    .expect("join_cluster B");

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!manager_b.is_manager(), "B starts as non-Manager");

    // Drop A (simulates Park's process exit without a clean libp2p
    // close — QUIC peer has to detect via idle timeout OR heartbeat
    // timeout).
    eprintln!("A=({pid_a}) dropping over QUIC…");
    drop(manager_a);

    // Post-fix: heartbeat-timeout fires ~1.5 s. Re-timed for #295:
    // B then defers until Discovery sweeps dead A's row (~3s) and
    // commits the claim — typically ~8–10s. Pre-fix on QUIC the
    // DETECTION alone took tens of seconds (idle timeout), so the
    // 25s budget still separates the two.
    eprintln!("waiting up to {TAKEOVER_DEADLINE:?} for B to take over via heartbeat-timeout…");
    let deadline = std::time::Instant::now() + TAKEOVER_DEADLINE;
    while std::time::Instant::now() < deadline {
        if manager_b.is_manager() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        manager_b.is_manager(),
        "B did not take over within {TAKEOVER_DEADLINE:?} after A dropped over QUIC — \
         heartbeat substream probably didn't open (the BUG this test pins)"
    );

    manager_b.shutdown().await.expect("B shutdown");
    eprintln!(
        "QUIC failover OK against {}: A={pid_a} → B={pid_b}",
        discovery_url()
    );
}

/// Sibling of `manager_failover_over_quic_when_joiner_pid_lower` with
/// the opposite peer-id ordering: Manager (lower-pid) admits a Joiner
/// with higher pid. Seeds `[91]` / `[92]` pin `pid_a < pid_b`.
///
/// This is the exact peer-id ordering Nils hit on K1: Park (lower) +
/// Booster (higher), Park leaves, Booster should take over via the
/// domain-owned heartbeat timer regardless of QUIC's slow connection-
/// close detection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn manager_failover_over_quic_when_manager_pid_lower() {
    let cluster_name = unique_cluster_name("sdk-quic-mpidlow-it");

    let id_a = PeerIdentity::from_seed(&[91u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
            agent_version: "sdk-quic-mpidlow-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;

    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test-A"),
    )
    .await
    .expect("create_cluster A");

    let id_b = PeerIdentity::from_seed(&[92u8; 32]);
    let pid_b = id_b.peer_id();
    assert!(pid_a < pid_b, "fixture pins Manager pid lower than Joiner");
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
            agent_version: "sdk-quic-mpidlow-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;

    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test-B"),
    )
    .await
    .expect("join_cluster B");

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!manager_b.is_manager(), "B starts as non-Manager");

    // Drop A (simulates Park's process exit without a clean libp2p
    // close).
    eprintln!("A=({pid_a}) dropping over QUIC (Manager-pid-lower)…");
    drop(manager_a);

    // Re-timed for #295: defer-until-sweep failover, see
    // `TAKEOVER_DEADLINE`.
    eprintln!("waiting up to {TAKEOVER_DEADLINE:?} for B to take over via heartbeat-timeout…");
    let deadline = std::time::Instant::now() + TAKEOVER_DEADLINE;
    while std::time::Instant::now() < deadline {
        if manager_b.is_manager() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        manager_b.is_manager(),
        "B did not take over within {TAKEOVER_DEADLINE:?} after A dropped over QUIC \
         (Manager-pid-lower path — the exact scenario Nils hit on K1)"
    );

    manager_b.shutdown().await.expect("B shutdown");
    eprintln!(
        "QUIC failover OK against {}: A={pid_a} → B={pid_b}",
        discovery_url()
    );
}

/// Manager dies immediately after the join response, before the
/// joiner has had any grace window to observe a heartbeat frame.
///
/// This pins the "no first heartbeat ever arrives" hole: the joiner
/// must arm Manager-death detection once it has a membership snapshot,
/// not only after `run_heartbeat_pair` receives its first frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn manager_failover_when_manager_dies_before_first_heartbeat() {
    let cluster_name = unique_cluster_name("sdk-no-first-hb-it");

    let id_a = PeerIdentity::from_seed(&[91u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
            agent_version: "sdk-no-first-hb-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;

    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test-A"),
    )
    .await
    .expect("create_cluster A");

    let id_b = PeerIdentity::from_seed(&[92u8; 32]);
    let pid_b = id_b.peer_id();
    assert!(pid_a < pid_b, "fixture pins Manager pid lower than Joiner");
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()],
            agent_version: "sdk-no-first-hb-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;

    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test-B"),
    )
    .await
    .expect("join_cluster B");
    assert!(!manager_b.is_manager(), "B starts as non-Manager");
    assert_eq!(manager_b.manager_peer_id(), pid_a);

    eprintln!("A=({pid_a}) dropping immediately after admitting B…");
    drop(manager_a);

    // Re-timed for #295: defer-until-sweep failover, see
    // `TAKEOVER_DEADLINE`.
    let deadline = std::time::Instant::now() + TAKEOVER_DEADLINE;
    while std::time::Instant::now() < deadline {
        if manager_b.is_manager() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        manager_b.is_manager(),
        "B did not take over within {TAKEOVER_DEADLINE:?} after A died before the first \
         heartbeat window"
    );
    assert_eq!(manager_b.manager_peer_id(), pid_b);

    manager_b.shutdown().await.expect("B shutdown");
    eprintln!(
        "No-first-heartbeat failover OK against {}: A={pid_a} → B={pid_b}",
        discovery_url()
    );
}

/// #295 regression: a dropped (not shut down) Manager is replaced
/// only AFTER Discovery sweeps its stale row — never before.
///
/// Mechanisms pinned:
///
/// - `ClusterManager::drop` aborts every SDK background task,
///   including the Manager-side 1s Discovery liveness tick. A zombie
///   tick would keep the row fresh forever and B would Defer forever
///   (this test would time out at 30s).
/// - B's Manager-loss gate: while the row still names dead A, B
///   **Defers** — rejoin attempt toward A plus a re-armed watch —
///   instead of electing. The pre-#295 code promoted at the first
///   heartbeat timeout (~1.5s), racing live Managers into
///   split-brain. Only once the row is swept does B elect, and
///   `is_manager` flips only after `register_as_manager` commits at
///   Discovery (rotate → 404 → create).
///
/// The drop is synced to a fresh liveness refresh so the sweep lands
/// ~`DISCOVERY_SWEEP_WINDOW` after the drop, making the ≥sweep lower
/// bound meaningful.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn dead_manager_drop_is_replaced_only_after_row_sweep() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-sweepgate-it");

    // --- Peer A: creates the cluster ---
    let id_a = PeerIdentity::from_seed(&[61u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-sweepgate-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;

    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test-A"),
    )
    .await
    .expect("create_cluster A");

    // --- Peer B: joins A's cluster ---
    let id_b = PeerIdentity::from_seed(&[62u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-sweepgate-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;

    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test-B"),
    )
    .await
    .expect("join_cluster B");

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!manager_b.is_manager(), "B starts as non-Manager");
    assert_eq!(manager_b.manager_peer_id(), pid_a);

    // Kill A right after one of its liveness refreshes: the row then
    // sweeps ~DISCOVERY_SWEEP_WINDOW later, so any B promotion before
    // that is a gate violation.
    wait_for_fresh_liveness_refresh(&discovery, &cluster_name).await;
    eprintln!("A=({pid_a}) dropping WITHOUT shutdown right after a row refresh…");
    let dropped_at = std::time::Instant::now();
    drop(manager_a);

    // Record the first instant B claims the role. 100ms sampling so
    // the ≥sweep lower bound below can't be masked by poll
    // granularity.
    eprintln!("polling up to 30s for B's takeover…");
    let poll_deadline = dropped_at + Duration::from_secs(30);
    let mut flipped_after: Option<Duration> = None;
    while std::time::Instant::now() < poll_deadline {
        if manager_b.is_manager() {
            flipped_after = Some(dropped_at.elapsed());
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let flipped_after = flipped_after.expect(
        "B never became Manager within 30s of A's drop — either A's row never swept \
         (zombie liveness tick: Drop failed to abort it) or B's election never committed",
    );
    eprintln!("B promoted {flipped_after:?} after the drop");

    // (i) Within the #295 recovery budget.
    assert!(
        flipped_after <= TAKEOVER_DEADLINE,
        "B's takeover took {flipped_after:?} — beyond the {TAKEOVER_DEADLINE:?} #295 budget"
    );

    // (ii) Not before the sweep. The 200ms slack absorbs the one-RTT
    // skew between Discovery stamping the refresh and the test
    // observing it. The pre-#295 code promoted at the first
    // heartbeat timeout (~1.5s) and would fail here.
    assert!(
        flipped_after >= DISCOVERY_SWEEP_WINDOW - Duration::from_millis(200),
        "B promoted {flipped_after:?} after the drop — before Discovery's \
         {DISCOVERY_SWEEP_WINDOW:?} sweep window. The Defer gate did not hold (#295)."
    );
    assert_eq!(manager_b.manager_peer_id(), pid_b);

    // (iii) The committed row names B.
    let row = cluster_row(&discovery, &cluster_name)
        .await
        .expect("cluster row exists after B's takeover commit");
    assert_eq!(
        row.manager_peer_id, pid_b,
        "Discovery's row should name B after the commit-point promotion"
    );

    // --- Cleanup ---
    manager_b.shutdown().await.expect("B shutdown");
    if cluster_row(&discovery, &cluster_name).await.is_some() {
        discovery
            .deregister(cluster_name.clone())
            .await
            .expect("explicit cleanup deregister");
    }

    eprintln!(
        "Defer-until-sweep failover OK against {}: A={pid_a} dropped, B={pid_b} \
         promoted after {flipped_after:?}",
        discovery_url()
    );
}

/// #295: the Manager-side 1s liveness tick is the Discovery arbiter.
/// When the cluster row is rotated to another member out-of-band, the
/// incumbent must step down within ~3s (next tick → adopt the row's
/// winner → rejoin attempt → STOP ticking) and must never rotate the
/// row back to itself — no flapping.
///
/// Knock-on, also pinned: stepping down clears A's heartbeat targets,
/// so B's own watch on A times out (~1.5s later); B's gate then finds
/// the row naming B itself and B claims the role through the gate's
/// names-local → ElectLocally arm (`register_as_manager` is an
/// idempotent rotate on its own row). Full convergence — B Manager,
/// row naming B, A following B — is therefore asserted at the end;
/// the row must never name A again after the rotation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn foreign_rotation_steps_manager_down() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-stepdown-it");

    // --- Peer A: creates the cluster ---
    let id_a = PeerIdentity::from_seed(&[63u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-stepdown-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;

    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test-A"),
    )
    .await
    .expect("create_cluster A");

    // --- Peer B: joins A's cluster ---
    let id_b = PeerIdentity::from_seed(&[64u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-stepdown-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;

    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b.clone()],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test-B"),
    )
    .await
    .expect("join_cluster B");

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(manager_a.is_manager(), "A starts as Manager");
    assert!(!manager_b.is_manager(), "B starts as non-Manager");

    // --- Out-of-band: rotate the row to B (a real, reachable member). ---
    eprintln!("rotating Discovery row to B=({pid_b}) out-of-band…");
    discovery
        .rotate_manager(cluster_name.clone(), pid_b, vec![addr_b.clone()])
        .await
        .expect("out-of-band rotate_manager to B");
    let rotated_at = std::time::Instant::now();

    // A's next arbiter tick (≤1s) must observe the foreign row and
    // step down: adopt B as Manager and stop being one.
    let stepdown_deadline = rotated_at + Duration::from_secs(6);
    while std::time::Instant::now() < stepdown_deadline {
        if !manager_a.is_manager() && manager_a.manager_peer_id() == pid_b {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let stepped_down_after = rotated_at.elapsed();
    assert!(
        !manager_a.is_manager(),
        "A still claims the Manager role 6s after the row rotated to B \
         (#295 arbiter tick did not step down)"
    );
    assert_eq!(
        manager_a.manager_peer_id(),
        pid_b,
        "A should adopt Discovery's named winner on step-down"
    );
    eprintln!("A stepped down {stepped_down_after:?} after the rotation");

    // --- Hold: A must never reclaim the row. ---
    // Poll for ~5s: the row must never name A again (A's tick must
    // have stopped — a zombie tick would re-register A via the
    // swept-row path or rotate back). The row is allowed to briefly
    // go absent if B's claim races the sweep; what is forbidden is A.
    let hold_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < hold_deadline {
        assert!(
            !manager_a.is_manager(),
            "A re-promoted itself during the hold window — Manager-side flapping (#295)"
        );
        if let Some(row) = cluster_row(&discovery, &cluster_name).await {
            assert_ne!(
                row.manager_peer_id, pid_a,
                "row names A again during the hold window — A rotated the row back (#295)"
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // --- Convergence: B claims the role its row assignment gave it. ---
    // A's step-down starved B's heartbeat watch on A, so B's gate ran
    // with the row naming B itself → ElectLocally → idempotent rotate
    // → promotion. (A real swept-row election is exercised by
    // `dead_manager_drop_is_replaced_only_after_row_sweep`.)
    let converge_deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < converge_deadline {
        if manager_b.is_manager() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        manager_b.is_manager(),
        "B never claimed the Manager role Discovery assigned it — its watch on the \
         silent ex-Manager A never re-entered the gate (#295 unconditional watch)"
    );
    let row = cluster_row(&discovery, &cluster_name)
        .await
        .expect("cluster row exists after B's claim");
    assert_eq!(
        row.manager_peer_id, pid_b,
        "row still names B after convergence"
    );
    assert_eq!(
        manager_a.manager_peer_id(),
        pid_b,
        "A keeps following B after convergence"
    );

    // --- Cleanup ---
    manager_a.shutdown().await.expect("A shutdown");
    // Let B (Manager) evict A so B's shutdown sees itself as last
    // member and deregisters.
    let evict_deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < evict_deadline {
        if manager_b.peer_count() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    manager_b.shutdown().await.expect("B shutdown");
    if cluster_row(&discovery, &cluster_name).await.is_some() {
        discovery
            .deregister(cluster_name.clone())
            .await
            .expect("explicit cleanup deregister");
    }

    eprintln!(
        "Foreign-rotation step-down OK against {}: A={pid_a} stepped down in \
         {stepped_down_after:?}, B={pid_b} claimed the role",
        discovery_url()
    );
}

/// #295 amendment regression (the stranded-ex-Manager fix): a Manager
/// displaced by a row naming an UNREACHABLE foreign peer must
///
/// 1. step down promptly (arbiter tick),
/// 2. keep retrying — Defer cycles toward the row's peer — without
///    ever self-promoting or rotating the row back while the row
///    holds (no flapping), and
/// 3. reclaim the role once the row names it again, through the
///    unconditional-watch retry loop: the watch on the unreachable
///    peer times out → the gate sees the row naming the local peer →
///    ElectLocally → `register_as_manager` (idempotent rotate on its
///    own row) → promote. Pre-amendment, a stepped-down ex-Manager
///    whose membership doc had no entry for the foreign peer had an
///    empty watchlist — no timeout could ever fire and it stayed
///    stranded forever.
///
/// The fake peer never ticks Discovery, so a stranded row would sweep
/// in ~3s and void the hold-phase invariant; the test plays the fake
/// Manager's Discovery presence (out-of-band `liveness_check` every
/// 1s) to hold the row alive. That IS the stranded scenario:
/// Discovery keeps vouching for a Manager that A cannot reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn displaced_manager_keeps_retrying_and_reclaims_when_row_returns() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-displaced-it");

    // --- Peer A: creates the cluster ---
    let id_a = PeerIdentity::from_seed(&[65u8; 32]);
    let pid_a = id_a.peer_id();
    let mut swarm_a = build_swarm(
        &id_a,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-displaced-it-A/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_a = wait_for_listen_addr(&mut swarm_a).await;

    let manager_a = ClusterManager::create_cluster(
        cluster_name.clone(),
        id_a.clone(),
        vec![addr_a.clone()],
        discovery_url(),
        swarm_a,
        decline_all_streams(),
        sample_daemon_info("test-A"),
    )
    .await
    .expect("create_cluster A");

    // --- Peer B: joins A's cluster ---
    let id_b = PeerIdentity::from_seed(&[66u8; 32]);
    let pid_b = id_b.peer_id();
    let mut swarm_b = build_swarm(
        &id_b,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "sdk-displaced-it-B/0".into(),
            enable_relay_server: false,
        },
    )
    .unwrap();
    let addr_b = wait_for_listen_addr(&mut swarm_b).await;

    let manager_b = ClusterManager::join_cluster(
        cluster_name.clone(),
        id_b.clone(),
        vec![addr_b],
        discovery_url(),
        swarm_b,
        decline_all_streams(),
        sample_daemon_info("test-B"),
    )
    .await
    .expect("join_cluster B");

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(manager_a.is_manager(), "A starts as Manager");

    // --- Out-of-band: rotate the row to a fake, unreachable peer. ---
    let fake_pid = PeerIdentity::from_seed(&[99u8; 32]).peer_id();
    let fake_addr: Multiaddr = "/ip4/10.255.255.1/tcp/4".parse().unwrap();
    eprintln!("rotating Discovery row to fake peer {fake_pid} (unroutable)…");
    discovery
        .rotate_manager(cluster_name.clone(), fake_pid, vec![fake_addr])
        .await
        .expect("out-of-band rotate_manager to fake peer");

    // Play the fake Manager's Discovery presence so the row outlives
    // the 3s sweep for the whole hold phase.
    let ticker = {
        let name = cluster_name.clone();
        let d = DiscoveryClient::new(discovery_url());
        tokio::spawn(async move {
            loop {
                let _ = d.liveness_check(name.clone(), 2).await;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
    };

    // A's next arbiter tick must step down toward the fake peer.
    let stepdown_deadline = std::time::Instant::now() + Duration::from_secs(6);
    while std::time::Instant::now() < stepdown_deadline {
        if !manager_a.is_manager() && manager_a.manager_peer_id() == fake_pid {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !manager_a.is_manager(),
        "A still claims the Manager role 6s after the row rotated to the fake peer"
    );
    assert_eq!(
        manager_a.manager_peer_id(),
        fake_pid,
        "A should adopt Discovery's named (unreachable) winner on step-down"
    );
    eprintln!("A stepped down toward the fake peer; holding to prove no flapping…");

    // --- Hold ~5s: no self-promotion, no rotating the row back. ---
    // A is mid Defer cycle (each: heartbeat timeout on the fake peer
    // → gate sees the row still naming it → rejoin attempt with a 5s
    // dead-dial wait → re-arm). B follows the row too and must not
    // promote either.
    let hold_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < hold_deadline {
        assert!(
            !manager_a.is_manager(),
            "displaced A self-promoted while Discovery's row still names the fake peer \
             — Defer gate flapping (#295)"
        );
        assert_eq!(
            manager_a.manager_peer_id(),
            fake_pid,
            "A's Manager view flapped away from the row's named winner during the hold"
        );
        assert!(
            !manager_b.is_manager(),
            "follower B self-promoted while Discovery's row still names the fake peer"
        );
        let row = cluster_row(&discovery, &cluster_name)
            .await
            .expect("row stays alive during the hold (test ticks its liveness)");
        assert_eq!(
            row.manager_peer_id, fake_pid,
            "row no longer names the fake peer during the hold — somebody rotated it"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // --- The row "returns": name A again. ---
    ticker.abort();
    eprintln!("rotating Discovery row back to A=({pid_a})…");
    discovery
        .rotate_manager(cluster_name.clone(), pid_a, vec![addr_a.clone()])
        .await
        .expect("rotate_manager back to A");
    let returned_at = std::time::Instant::now();

    // A's retry loop must reclaim: watch on the fake peer times out →
    // gate sees the row naming A → ElectLocally → A wins → commit
    // (idempotent rotate; or re-create if the unrefreshed row swept
    // first) → promote. Worst case one full Defer cycle ≈ 5s dial
    // wait + re-armed 1.5s timeout + tick granularity.
    let reclaim_deadline = returned_at + Duration::from_secs(10);
    while std::time::Instant::now() < reclaim_deadline {
        if manager_a.is_manager() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let reclaimed_after = returned_at.elapsed();
    assert!(
        manager_a.is_manager(),
        "A did not reclaim the Manager role within 10s of the row naming it again — \
         the displaced-Manager retry loop is dead (#295 amendment)"
    );
    assert_eq!(manager_a.manager_peer_id(), pid_a);
    eprintln!("A reclaimed the Manager role {reclaimed_after:?} after the row returned");

    let row = cluster_row(&discovery, &cluster_name)
        .await
        .expect("cluster row exists after A's reclaim");
    assert_eq!(row.manager_peer_id, pid_a, "row names A after the reclaim");

    // Bonus, not load-bearing: B should converge on A via A's
    // promotion gossip (`broadcast_current_membership`) or its own
    // Follow retry cycle. Its cycle includes 5s dead-dial waits
    // toward the fake peer, so it can lag past this window — log
    // instead of asserting when it does; A's self-view and the row
    // above are the #295 invariants.
    let bonus_deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut b_converged = false;
    while std::time::Instant::now() < bonus_deadline {
        if manager_b.manager_peer_id() == pid_a {
            b_converged = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    eprintln!(
        "B's Manager view after reclaim: {} (converged on A: {b_converged})",
        manager_b.manager_peer_id()
    );

    // --- Cleanup ---
    manager_b.shutdown().await.expect("B shutdown");
    // Let A (Manager again) evict B so A's shutdown sees itself as
    // last member and deregisters.
    let evict_deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < evict_deadline {
        if manager_a.peer_count() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    manager_a.shutdown().await.expect("A shutdown");
    if cluster_row(&discovery, &cluster_name).await.is_some() {
        discovery
            .deregister(cluster_name.clone())
            .await
            .expect("explicit cleanup deregister");
    }

    eprintln!(
        "Displaced-Manager retry + reclaim OK against {}: A={pid_a} reclaimed in \
         {reclaimed_after:?}, B={pid_b} converged={b_converged}",
        discovery_url()
    );
}
