use auki_sdk::{
    AukiKnownPeerEvent, AukiKnownPeerRecvError, AukiKnownPeerSnapshot, AukiKnownPeers, AukiPeer,
    AukiPeerAuthorityError, AukiPeerProtocolContext, AukiPeerProtocols, AukiPeerRelayError,
    AukiPeerShutdownError, AukiPeerStartError, AukiPeerStatus, AukiProtocolError,
    AukiProtocolRegistration, AukiProtocolRouteAttempt, AukiProtocolSpec, AukiProtocolStream,
    AuthenticatedPeer, AuthenticatedRouteStream, DdsVerificationKeys, ExternalAuthorityControl,
    ExternalAuthorityRefreshRequest, ExternalAuthorityReplaceOutcome, ExternalAuthorityUpdate,
    Identity, PreparedPeer, SignedP2pCredential,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

fn assert_send_sync<T: Send + Sync>() {}

const CUSTOM_PROTOCOL: &str = "/example/echo/1.0.0";

fn register_custom_protocol(protocols: &AukiPeerProtocols) {
    let spec = AukiProtocolSpec::new(CUSTOM_PROTOCOL, 4, 4096).unwrap();
    let _registration = protocols.register(spec, |stream: AukiProtocolStream| async move {
        let _remote: &AuthenticatedPeer = stream.remote_peer();
    });
}

#[test]
fn facade_reexports_the_complete_safe_custom_protocol_surface() {
    let _identity = Identity::generate();
    AukiProtocolSpec::new(CUSTOM_PROTOCOL, 4, 4096)
        .expect("documented custom protocol namespace must remain valid");

    let _register = register_custom_protocol as fn(&AukiPeerProtocols);
    let _prepared: Option<PreparedPeer> = None;
    let _registration: Option<AukiProtocolRegistration> = None;
    let _route_attempt: Option<AukiProtocolRouteAttempt> = None;
    let _error: Option<AukiProtocolError> = None;
    let _stream: Option<AuthenticatedRouteStream> = None;

    let _start = AukiPeer::start;
    let _start_external = AukiPeer::start_external;
    let _shutdown = AukiPeer::shutdown;
    let _status: fn(&AukiPeer) -> AukiPeerStatus = AukiPeer::status;
    let _context: fn(&AukiPeer) -> AukiPeerProtocolContext = AukiPeer::protocol_context;
    let _protocols: fn(&AukiPeer) -> AukiPeerProtocols = AukiPeer::protocols;
    let _protocol_peer_id: fn(&AukiPeerProtocols) -> auki_sdk::PeerId = AukiPeerProtocols::peer_id;
    let _protocol_domain_id: fn(&AukiPeerProtocols) -> Uuid = AukiPeerProtocols::domain_id;
    let _known_peers: fn(&AukiPeer) -> AukiKnownPeers = AukiPeer::known_peers;
    let _known_snapshot: Option<AukiKnownPeerSnapshot> = None;
    let _known_event: Option<AukiKnownPeerEvent> = None;
    let _known_error: Option<AukiKnownPeerRecvError> = None;
    let _start_error: Option<AukiPeerStartError> = None;
    let _shutdown_error: Option<AukiPeerShutdownError> = None;
    let _authority_error: Option<AukiPeerAuthorityError> = None;
    let _relay_error: Option<AukiPeerRelayError> = None;

    let _external_update_new: fn(
        Uuid,
        auki_sdk::PeerId,
        DdsVerificationKeys,
        SignedP2pCredential,
        DateTime<Utc>,
    ) -> ExternalAuthorityUpdate = ExternalAuthorityUpdate::new;
    let _external_domain: fn(&ExternalAuthorityUpdate) -> Uuid = ExternalAuthorityUpdate::domain_id;
    let _external_peer: fn(&ExternalAuthorityUpdate) -> auki_sdk::PeerId =
        ExternalAuthorityUpdate::peer_id;
    let _external_key_generation: fn(&ExternalAuthorityUpdate) -> u64 =
        ExternalAuthorityUpdate::verification_key_generation;
    let _external_expiration: fn(&ExternalAuthorityUpdate) -> DateTime<Utc> =
        ExternalAuthorityUpdate::credential_expires_at;
    let _replace = ExternalAuthorityControl::replace;
    let _next_refresh_request = ExternalAuthorityControl::next_refresh_request;
    let _request_id: fn(&ExternalAuthorityRefreshRequest) -> u64 =
        ExternalAuthorityRefreshRequest::request_id;
    let _rejected_revision: fn(&ExternalAuthorityRefreshRequest) -> u64 =
        ExternalAuthorityRefreshRequest::rejected_credential_revision;
    let outcome = ExternalAuthorityReplaceOutcome::Replaced {
        credential_revision: 7,
    };
    assert_eq!(outcome.credential_revision(), 7);
    assert_send_sync::<ExternalAuthorityControl>();
}
