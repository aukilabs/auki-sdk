#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use auki_sdk::RelayCircuitRoutes;
use auki_sdk::{
    AukiPeer, AukiPeerBootstrap, AukiPeerBootstrapError, AukiPeerConfig, AukiPeerError,
    AukiPeerExit, AukiPeerFailure, AukiPeerLifecycle, AukiPeerProtocols, AukiPeerReachability,
    AukiPeerShutdownError, AukiPeerStartError, AukiProtocolRegistration, AukiRelayConfig,
    AukiRelayMode, AuthClient, AuthEnvironment, Credentials, DomainChoice, DomainSelection,
};

#[test]
fn browser_facade_exposes_only_browser_runtime_configuration_and_lifecycle() {
    let _start = AukiPeer::start;
    let _bootstrap_authenticate = AukiPeerBootstrap::authenticate;
    let _bootstrap_dev = AukiPeerBootstrap::dev;
    let _bootstrap_domains = AukiPeerBootstrap::accessible_domains;
    let _bootstrap_start = AukiPeerBootstrap::start_peer;
    let _bootstrap_ephemeral = AukiPeerBootstrap::start_ephemeral_peer;
    let _auth_client: Option<AuthClient> = None;
    let _auth_environment = AuthEnvironment::dev();
    let _credentials: Option<Credentials> = None;
    let _domain_choice: Option<DomainChoice> = None;
    let _domain_selection: Option<DomainSelection> = None;
    let _bootstrap_error: Option<AukiPeerBootstrapError> = None;
    let _shutdown = AukiPeer::shutdown;
    let _wait_stopped = AukiPeer::wait_stopped;
    let _protocols: fn(&AukiPeer) -> AukiPeerProtocols = AukiPeer::protocols;
    let _reachability: fn(&AukiPeer) -> &AukiPeerReachability = AukiPeer::reachability;
    let _tcp: fn(&RelayCircuitRoutes) -> &auki_sdk::Multiaddr = RelayCircuitRoutes::tcp;
    let _wss: fn(&RelayCircuitRoutes) -> &auki_sdk::Multiaddr = RelayCircuitRoutes::wss;
    let _lifecycle: fn(&AukiPeer) -> AukiPeerLifecycle = AukiPeer::lifecycle;
    let _registration: Option<AukiProtocolRegistration> = None;
    let _exit: Option<AukiPeerExit> = None;
    let _failure: Option<AukiPeerFailure> = None;
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
