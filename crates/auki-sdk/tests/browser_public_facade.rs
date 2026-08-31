#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use auki_sdk::{
    AukiPeer, AukiPeerConfig, AukiPeerError, AukiPeerExit, AukiPeerLifecycle, AukiPeerProtocols,
    AukiPeerReachability, AukiPeerShutdownError, AukiPeerStartError, AukiProtocolRegistration,
    AukiRelayConfig, AukiRelayMode,
};

#[test]
fn browser_facade_exposes_only_browser_runtime_configuration_and_lifecycle() {
    let _start = AukiPeer::start;
    let _shutdown = AukiPeer::shutdown;
    let _protocols: fn(&AukiPeer) -> AukiPeerProtocols = AukiPeer::protocols;
    let _reachability: fn(&AukiPeer) -> &AukiPeerReachability = AukiPeer::reachability;
    let _lifecycle: fn(&AukiPeer) -> AukiPeerLifecycle = AukiPeer::lifecycle;
    let _registration: Option<AukiProtocolRegistration> = None;
    let _exit: Option<AukiPeerExit> = None;
    let _start_error: Option<AukiPeerStartError> = None;
    let _shutdown_error: Option<AukiPeerShutdownError> = None;
    let _error: Option<AukiPeerError> = None;

    let relay = AukiRelayConfig::new(
        AukiRelayMode::Public,
        1,
        Duration::from_secs(300),
        Duration::from_secs(5),
    )
    .unwrap();
    let config = AukiPeerConfig::dev().with_relay(relay).unwrap();
    assert!(config.relay_required());
    assert_eq!(config.relay(), Some(relay));
}
