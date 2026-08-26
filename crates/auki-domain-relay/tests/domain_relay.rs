use auki_domain_relay::{DomainRelay, DomainRelayConfig, DomainRelayEvent};
use auki_p2p::Identity;
use multiaddr::Multiaddr;
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn relay_emits_dialable_relay_multiaddr() {
    let identity = Identity::from_ed25519_seed(&[42u8; 32]);
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
