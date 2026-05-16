//! Live Discovery + **real** infrastructure relay: two [`ClusterManager`] peers
//! (`create_cluster` + `join_cluster`) should land **circuit** multiaddrs
//! on the Manager's [`ClusterMembership`] for both A and B once Discovery
//! lists at least one `relay` node at `GET /nodes?type=relay` (dialable
//! multiaddrs, heartbeating — same contract as production).
//!
//! The test does **not** spawn or register a local relay; run the `relay`
//! daemon (or equivalent) against the same Discovery URL first.
//!
//! `#[ignore]` by default — requires a reachable Discovery (same contract
//! as [`cluster_manager_integration`]). The test sets
//! `AUKI_RELAY_PREPARE_AUTONAT_SECS`, `AUKI_RELAY_PREPARE_RELAY_IDENT_SECS`, and
//! `AUKI_RELAY_PREPARE_RELAY_PHASE_SECS` to generous values so `relay_prepare`
//! can complete against a real relay on the LAN or VPC, and wraps the body
//! in a **120s**
//! wall-clock timeout so `join_cluster` (10s Manager dial budget) + libp2p
//! can finish against a real relay without tripping Discovery's 10s sweep.
//!
//! If `GET /nodes?type=relay` returns **404**, the test **skips** (passes)
//! with a message unless **`STRICT_RELAY_IT=1`**, in which case it panics
//! (useful when you expect a Discovery build that implements infrastructure
//! nodes). An **empty** relay directory also skips the same way.
//!
//! ## How to run (three terminals)
//!
//! 1. **Discovery** (HTTP directory on port 8099 in this example):
//!
//!    ```bash
//!    cd path/to/discovery
//!    cargo run -- --addr 0.0.0.0:8099
//!    ```
//!
//! 2. **Relay** (registers at `POST /nodes`; default **TCP/UDP 4001** must be free):
//!
//!    ```bash
//!    cd path/to/relay
//!    # Use 127.0.0.1 for the HTTP client if Discovery listens on 0.0.0.0:8099
//!    AUKI_DISCOVERY_URL=http://127.0.0.1:8099 cargo run
//!    ```
//!
//!    Optional: `AUKI_RELAY_LISTEN_TCP`, `AUKI_RELAY_LISTEN_QUIC` multiaddrs if you
//!    change ports away from 4001.
//!
//! 3. **Integration test** (from the **auki-sdk** workspace; `#[ignore]` requires `--ignored`):
//!
//!    ```bash
//!    cd path/to/auki-sdk
//!    DISCOVERY_URL=http://127.0.0.1:8099 \
//!      cargo test -p auki-domain --test relay_circuit_live_discovery \
//!        -- --ignored --nocapture
//!    ```
//!
//! Sanity check before step 3: `curl -s 'http://127.0.0.1:8099/nodes?type=relay'`
//! — the JSON body should list at least one relay with non-empty `multiaddrs`.
//!
//! `AUKI_DISCOVERY_URL` is accepted as a fallback when `DISCOVERY_URL` is unset
//! (same env var as the `relay` binary).
//!
//! Prerequisites: Discovery running; a relay already registered via
//! `POST /nodes` with ongoing heartbeats; `GET /nodes?type=relay` returns it.
//! The default relay listens on **TCP/UDP 4001** — if another process holds
//! those ports, `cargo run` for the relay will fail and this test will not
//! get circuit multiaddrs.

use auki_domain::ClusterManager;
use auki_network::PeerIdentity;
use auki_network::discovery_client::{DiscoveryClient, DiscoveryError, NodeEntry};
use auki_network::stream_runtime::decline_all_streams;
use auki_network::swarm::{Behaviour, SwarmConfig, build_swarm};
use futures::StreamExt as _;
use libp2p::Swarm;
use libp2p::swarm::SwarmEvent;
use multiaddr::Multiaddr;
use multiaddr::Protocol;
use std::str::FromStr as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn discovery_url() -> String {
    std::env::var("DISCOVERY_URL")
        .or_else(|_| std::env::var("AUKI_DISCOVERY_URL"))
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
}

fn strict_relay_it() -> bool {
    matches!(
        std::env::var("STRICT_RELAY_IT").as_deref(),
        Ok("1" | "true" | "TRUE")
    )
}

fn skip_or_fail(msg: &str) {
    if strict_relay_it() {
        panic!("{}", msg);
    }
    eprintln!("SKIP relay_circuit_live_discovery: {msg}");
}

fn unique_cluster_name(prefix: &str) -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{ns}")
}

fn sample_daemon_info(name: &str) -> auki_domain::DaemonInfo {
    auki_domain::DaemonInfo {
        app: "relay-it-daemon".into(),
        name: name.into(),
        session_id: "relay-it".into(),
        session_clock_id: format!("{name}/clock"),
        session_clock_hash: "h".into(),
        app_instance: "cafe".into(),
    }
}

async fn wait_for_listen_addr(swarm: &mut Swarm<Behaviour>) -> Multiaddr {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                return address;
            }
        }
    })
    .await
    .expect("listen addr did not appear within timeout")
}

async fn discovery_nodes_api_ready(discovery: &DiscoveryClient) -> Result<(), String> {
    match discovery.list_nodes(Some("relay")).await {
        Ok(_) => Ok(()),
        Err(DiscoveryError::Status { status: 404, .. }) => Err(
            "GET /nodes?type=relay returned 404 — infrastructure node listing is not available on this Discovery build."
                .into(),
        ),
        Err(e) => Err(format!("list_nodes(relay): {e}")),
    }
}

fn membership_has_circuit_for_both_peers(
    m: &auki_domain::ClusterMembership,
    a: libp2p_identity::PeerId,
    b: libp2p_identity::PeerId,
) {
    assert_eq!(m.peers.len(), 2, "expected Manager + joiner");
    for pid in [a, b] {
        let member = m
            .peers
            .iter()
            .find(|p| p.peer_id == pid)
            .unwrap_or_else(|| panic!("membership missing peer {pid}"));
        let has_circuit = member
            .multiaddrs
            .iter()
            .any(|addr| addr.iter().any(|p| matches!(p, Protocol::P2pCircuit)));
        assert!(
            has_circuit,
            "peer {pid} should advertise ≥1 circuit multiaddr; got {:?}",
            member.multiaddrs
        );
    }
}

/// Picks the first relay entry with at least one multiaddr (what `relay_prepare` needs).
fn pick_usable_relay(nodes: &[NodeEntry]) -> Option<&NodeEntry> {
    nodes
        .iter()
        .find(|n| n.node_type == "relay" && !n.multiaddrs.is_empty())
        .or_else(|| nodes.iter().find(|n| !n.multiaddrs.is_empty()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn manager_membership_lists_circuit_addrs_for_a_and_b_with_live_discovery() {
    // SAFETY: integration test runs in isolation; no other threads read these
    // env vars concurrently during this test.
    unsafe {
        // Real relays / cross-host dials need more than CI's 1/2/2s budgets.
        std::env::set_var("AUKI_RELAY_PREPARE_AUTONAT_SECS", "12");
        std::env::set_var("AUKI_RELAY_PREPARE_RELAY_IDENT_SECS", "20");
        std::env::set_var("AUKI_RELAY_PREPARE_RELAY_PHASE_SECS", "25");
    }

    tokio::time::timeout(Duration::from_secs(120), async {
        let discovery = DiscoveryClient::new(discovery_url());
        if let Err(msg) = discovery_nodes_api_ready(&discovery).await {
            skip_or_fail(&msg);
            return;
        }

        let relays = match discovery.list_nodes(Some("relay")).await {
            Ok(r) => r,
            Err(e) => {
                skip_or_fail(&format!("list_nodes(relay) after ready check: {e}"));
                return;
            }
        };
        let Some(relay_entry) = pick_usable_relay(&relays) else {
            skip_or_fail(
                "GET /nodes?type=relay returned no relay with dialable multiaddrs — \
                 start the real `relay` binary (or register one) against this Discovery.",
            );
            return;
        };
        let relay_pid = match libp2p_identity::PeerId::from_str(&relay_entry.peer_id) {
            Ok(p) => p,
            Err(e) => {
                skip_or_fail(&format!(
                    "invalid peer_id on relay node entry {:?}: {e}",
                    relay_entry.peer_id
                ));
                return;
            }
        };

        let cluster_name = unique_cluster_name("sdk-relay-circuit-it");

        // --- Manager A ---
        let id_a = PeerIdentity::from_seed(&[0xE1u8; 32]);
        let pid_a = id_a.peer_id();
        let mut swarm_a = build_swarm(
            &id_a,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                agent_version: "sdk-relay-it-A/0".into(),
                enable_relay_server: false,
                enable_autonat_server: false,
            },
        )
        .expect("build swarm A");
        let addr_a = wait_for_listen_addr(&mut swarm_a).await;

        let manager_a = ClusterManager::create_cluster(
            cluster_name.clone(),
            id_a.clone(),
            vec![addr_a],
            discovery.clone(),
            swarm_a,
            decline_all_streams(),
            sample_daemon_info("A"),
        )
        .await
        .expect("create_cluster A");

        assert!(manager_a.is_manager());
        let m_a_after_create = manager_a.membership();
        assert_eq!(m_a_after_create.peers.len(), 1, "solo Manager after create");
        let a_only = &m_a_after_create.peers[0];
        assert_eq!(a_only.peer_id, pid_a);
        assert!(
            a_only.multiaddrs.iter().any(|addr| {
                addr.iter()
                    .any(|p| matches!(p, Protocol::P2pCircuit))
            }),
            "Manager A should advertise ≥1 circuit addr before B joins: {:?}",
            a_only.multiaddrs
        );

        // --- Joiner B ---
        let id_b = PeerIdentity::from_seed(&[0xE2u8; 32]);
        let pid_b = id_b.peer_id();
        let mut swarm_b = build_swarm(
            &id_b,
            SwarmConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                agent_version: "sdk-relay-it-B/0".into(),
                enable_relay_server: false,
                enable_autonat_server: false,
            },
        )
        .expect("build swarm B");
        let addr_b = wait_for_listen_addr(&mut swarm_b).await;

        let manager_b = ClusterManager::join_cluster(
            cluster_name.clone(),
            id_b.clone(),
            vec![addr_b],
            discovery.clone(),
            swarm_b,
            decline_all_streams(),
            sample_daemon_info("B"),
        )
        .await
        .expect("join_cluster B");

        tokio::time::sleep(Duration::from_millis(300)).await;

        let m_a = manager_a.membership();
        let m_b = manager_b.membership();
        membership_has_circuit_for_both_peers(&m_a, pid_a, pid_b);
        membership_has_circuit_for_both_peers(&m_b, pid_a, pid_b);

        // --- Cleanup ---
        manager_b.shutdown().await.expect("B shutdown");
        tokio::time::sleep(Duration::from_millis(200)).await;
        manager_a.shutdown().await.expect("A shutdown");

        let after = discovery.list_clusters().await.expect("list after");
        assert!(
            !after.iter().any(|c| c.name == cluster_name),
            "cluster {cluster_name} should be gone after shutdowns"
        );

        eprintln!(
            "relay + circuit membership OK against {} (relay={relay_pid} from Discovery; {} multiaddrs)",
            discovery_url(),
            relay_entry.multiaddrs.len()
        );
    })
    .await
    .expect("relay_circuit_live_discovery exceeded 120s wall-clock (check Discovery, relay, and DISCOVERY_URL)");
}
