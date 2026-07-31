use auki_domain::cluster_manager::ManagerRelayReservation;
use auki_domain::{ClusterManager, DaemonInfo};
use auki_domain_relay::{DomainRelay, DomainRelayConfig, DomainRelayEvent};
use auki_network::PeerIdentity;
use auki_network::discovery_client::DiscoveryClient;
use auki_network::stream_runtime::decline_all_streams;
use auki_network::swarm::{
    SwarmConfig, build_swarm, reserve_relay_circuit_addr_with_advertised_addr,
};
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

async fn wait_for_native_and_browser_relay_multiaddrs(
    relay: &mut DomainRelay,
) -> (Multiaddr, Multiaddr) {
    timeout(Duration::from_secs(5), async {
        let mut native = None;
        let mut browser = None;
        while native.is_none() || browser.is_none() {
            let event = relay
                .next_event()
                .await
                .expect("relay event stream stays open");
            let DomainRelayEvent::Listening { relay_multiaddr } = event;
            if relay_multiaddr.to_string().contains("/ws") {
                browser.get_or_insert(relay_multiaddr);
            } else {
                native.get_or_insert(relay_multiaddr);
            }
        }
        (native.unwrap(), browser.unwrap())
    })
    .await
    .expect("relay did not emit both native and browser listen addresses")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manager_reserves_through_native_relay_addr_and_advertises_browser_addr() {
    let relay_identity = PeerIdentity::from_seed(&[44u8; 32]);
    let mut relay = DomainRelay::new(
        &relay_identity,
        DomainRelayConfig {
            listen_addresses: vec![
                "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
                "/ip4/127.0.0.1/tcp/0/ws".parse().unwrap(),
            ],
            agent_version: "auki-domain-relay/test".to_string(),
        },
    )
    .await
    .expect("relay starts");
    let (relay_dial_multiaddr, relay_advertise_multiaddr) =
        wait_for_native_and_browser_relay_multiaddrs(&mut relay).await;

    let manager_identity = PeerIdentity::from_seed(&[45u8; 32]);
    let mut manager_swarm = build_swarm(
        &manager_identity,
        SwarmConfig {
            listen_addresses: vec![],
            agent_version: "auki-domain-relay-test-manager/0".into(),
            enable_relay_server: false,
        },
    )
    .expect("manager swarm builds");
    let expected = relay_advertise_multiaddr
        .clone()
        .with(libp2p::multiaddr::Protocol::P2pCircuit)
        .with(libp2p::multiaddr::Protocol::P2p(manager_identity.peer_id()));

    let circuit_addr = timeout(Duration::from_secs(15), async {
        tokio::select! {
            reservation = reserve_relay_circuit_addr_with_advertised_addr(
                &mut manager_swarm,
                relay_dial_multiaddr,
                relay_advertise_multiaddr,
                Duration::from_secs(10),
            ) => reservation,
            _ = async {
                loop {
                    let _ = relay.next_event().await;
                }
            } => unreachable!("relay event stream ended"),
        }
    })
    .await
    .expect("relay reservation timed out")
    .expect("relay reservation succeeds");

    assert_eq!(circuit_addr, expected);
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
            listen_addresses: vec![
                "/ip4/127.0.0.1/tcp/0".parse().unwrap(),
                "/ip4/127.0.0.1/tcp/0/ws".parse().unwrap(),
            ],
            agent_version: "auki-domain-relay/smoke".to_string(),
        },
    )
    .await
    .expect("relay starts");
    let (relay_dial_multiaddr, relay_advertise_multiaddr) =
        wait_for_native_and_browser_relay_multiaddrs(&mut relay).await;

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
    let expected_circuit = relay_advertise_multiaddr
        .clone()
        .with(libp2p::multiaddr::Protocol::P2pCircuit)
        .with(libp2p::multiaddr::Protocol::P2p(manager_identity.peer_id()));

    let manager = timeout(Duration::from_secs(20), async {
        tokio::select! {
            manager = ClusterManager::create_cluster_with_relay_reservation(
                cluster_name.clone(),
                manager_identity.clone(),
                vec![manager_multiaddr.clone()],
                ManagerRelayReservation {
                    relay_dial_multiaddr,
                    relay_advertise_multiaddr: relay_advertise_multiaddr.clone(),
                    timeout: StdDuration::from_secs(10),
                },
                discovery_url(),
                manager_swarm,
                decline_all_streams(),
                sample_daemon_info("manager"),
            ) => manager,
            _ = async {
                loop {
                    let _ = relay.next_event().await;
                }
            } => unreachable!("relay event stream ended"),
        }
    })
    .await
    .expect("cluster creation timed out")
    .expect("cluster reserves through relay and publishes relay multiaddrs");
    assert_eq!(
        manager.local_multiaddrs(),
        &[manager_multiaddr.clone(), expected_circuit.clone()]
    );
    assert_eq!(
        manager.relay_multiaddrs(),
        &[relay_advertise_multiaddr.clone()]
    );

    let entry = discovery
        .get_peer_manager(cluster_name.clone())
        .await
        .expect("get peer manager");

    assert_eq!(entry.manager_peer_id, manager_identity.peer_id());
    assert_eq!(
        entry.manager_multiaddrs,
        vec![manager_multiaddr, expected_circuit]
    );
    assert_eq!(
        entry.relay_multiaddrs,
        vec![relay_advertise_multiaddr],
        "Discovery did not preserve the browser relay_multiaddrs published by ClusterManager; \
         upgrade the Discovery deployment/server contract before browser peers can discover relays"
    );

    manager.shutdown().await.expect("manager shuts down");
}
