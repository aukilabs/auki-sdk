use auki_domain::{ClusterManager, DaemonInfo};
use auki_domain_relay::{DomainRelay, DomainRelayConfig, DomainRelayEvent};
use auki_network::PeerIdentity;
use auki_network::discovery_client::DiscoveryClient;
use auki_network::stream_runtime::decline_all_streams;
use auki_network::swarm::{SwarmConfig, build_swarm};
use libp2p::Swarm;
use libp2p::swarm::SwarmEvent;
use multiaddr::Multiaddr;
use std::time::{Duration as StdDuration, SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn relay_emits_discovery_ready_relay_multiaddr() {
    let identity = PeerIdentity::from_seed(&[42u8; 32]);
    let mut relay = DomainRelay::new(
        &identity,
        DomainRelayConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0/ws".parse::<Multiaddr>().unwrap()],
            agent_version: "auki-domain-relay/test".to_string(),
        },
    )
    .await
    .expect("relay starts");

    let event = timeout(Duration::from_secs(5), relay.next_event())
        .await
        .expect("relay emits a listen event")
        .expect("relay event stream stays open");

    let DomainRelayEvent::Listening { relay_multiaddr } = event;
    let rendered = relay_multiaddr.to_string();
    assert!(rendered.starts_with("/ip4/127.0.0.1/tcp/"), "{rendered}");
    assert!(rendered.contains("/ws"), "{rendered}");
    assert!(
        rendered.ends_with(&format!("/p2p/{}", identity.peer_id())),
        "{rendered}"
    );
}

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

fn sample_daemon_info(name: &str) -> DaemonInfo {
    DaemonInfo {
        app: "domain-relay-smoke".into(),
        name: name.into(),
        session_id: "relay-smoke-session".into(),
        session_clock_id: format!("{name}/clock"),
        session_clock_hash: "relay-smoke-clock".into(),
        app_instance: "relay-smoke-instance".into(),
    }
}

async fn wait_for_manager_listen_addr(
    swarm: &mut Swarm<auki_network::swarm::Behaviour>,
) -> Multiaddr {
    use futures::StreamExt;

    timeout(StdDuration::from_secs(5), async {
        loop {
            if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                return address;
            }
        }
    })
    .await
    .expect("manager listen addr did not appear within timeout")
}

async fn wait_for_relay_multiaddr(relay: &mut DomainRelay) -> Multiaddr {
    let event = timeout(Duration::from_secs(5), relay.next_event())
        .await
        .expect("relay emits a listen event")
        .expect("relay event stream stays open");

    let DomainRelayEvent::Listening { relay_multiaddr } = event;
    relay_multiaddr
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn relay_multiaddr_can_be_published_through_cluster_manager_to_discovery() {
    let discovery = DiscoveryClient::new(discovery_url());
    let cluster_name = unique_cluster_name("sdk-domain-relay-smoke");

    let relay_identity = PeerIdentity::from_seed(&[90u8; 32]);
    let mut relay = DomainRelay::new(
        &relay_identity,
        DomainRelayConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0/ws".parse().unwrap()],
            agent_version: "auki-domain-relay/smoke".to_string(),
        },
    )
    .await
    .expect("relay starts");
    let relay_multiaddr = wait_for_relay_multiaddr(&mut relay).await;

    let manager_identity = PeerIdentity::from_seed(&[91u8; 32]);
    let mut manager_swarm = build_swarm(
        &manager_identity,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: "auki-domain-relay-smoke-manager/0".into(),
            enable_relay_server: false,
        },
    )
    .expect("manager swarm builds");
    let manager_multiaddr = wait_for_manager_listen_addr(&mut manager_swarm).await;

    let manager = ClusterManager::create_cluster_with_relay_multiaddrs(
        cluster_name.clone(),
        manager_identity.clone(),
        vec![manager_multiaddr.clone()],
        vec![relay_multiaddr.clone()],
        discovery_url(),
        manager_swarm,
        decline_all_streams(),
        sample_daemon_info("manager"),
    )
    .await
    .expect("cluster publishes relay multiaddrs");
    assert_eq!(manager.relay_multiaddrs(), &[relay_multiaddr.clone()]);

    let snapshot = discovery.list_clusters().await.expect("list clusters");
    let entry = snapshot
        .iter()
        .find(|entry| entry.name == cluster_name)
        .expect("created cluster appears in Discovery");

    assert_eq!(entry.manager_peer_id, manager_identity.peer_id());
    assert_eq!(entry.manager_multiaddrs, vec![manager_multiaddr]);
    assert_eq!(
        entry.relay_multiaddrs,
        vec![relay_multiaddr],
        "Discovery did not preserve the relay_multiaddrs published by ClusterManager; \
         upgrade the Discovery deployment/server contract before browser peers can discover relays"
    );

    manager.shutdown().await.expect("manager shuts down");
}
