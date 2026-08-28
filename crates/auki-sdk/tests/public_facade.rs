use auki_sdk::{
    AukiPeer, AukiPeerAuthorityError, AukiPeerProtocolContext, AukiPeerProtocols,
    AukiPeerRelayError, AukiPeerShutdownError, AukiPeerStartError, AukiPeerStatus,
    AuthenticatedPeer, AuthenticatedRouteStream, DdsVerificationKeys, DomainProtocolError,
    DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream, ExternalAuthorityControl,
    ExternalAuthorityRefreshRequest, ExternalAuthorityReplaceOutcome, ExternalAuthorityUpdate,
    Identity, Peer, PreparedPeer, Session, SignedP2pCredential,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

fn assert_send_sync<T: Send + Sync>() {}

const CUSTOM_PROTOCOL: &str = "/example/echo/1.0.0";

fn register_custom_protocol(protocols: &AukiPeerProtocols) {
    let spec = DomainProtocolSpec::new(CUSTOM_PROTOCOL, 4, 4096).unwrap();
    let _registration = protocols.register(spec, |stream: DomainProtocolStream| async move {
        let _remote: &AuthenticatedPeer = stream.remote_peer();
    });
}

#[test]
fn facade_reexports_the_complete_safe_custom_protocol_surface() {
    let storage = tempfile::tempdir().unwrap();
    let identity = Identity::generate();
    let peer = Peer::new(identity.peer_id().to_string(), "public-facade-test")
        .with_storage_root(storage.path().to_path_buf());
    let session: Session = peer.start_session().unwrap();
    assert!(peer.owns_session(&session));
    DomainProtocolSpec::new(CUSTOM_PROTOCOL, 4, 4096)
        .expect("documented custom protocol namespace must remain valid");

    let _register = register_custom_protocol as fn(&AukiPeerProtocols);
    let _prepared: Option<PreparedPeer> = None;
    let _registration: Option<DomainProtocolRegistration> = None;
    let _error: Option<DomainProtocolError> = None;
    let _stream: Option<AuthenticatedRouteStream> = None;

    let _start = AukiPeer::start;
    let _start_external = AukiPeer::start_external;
    let _shutdown = AukiPeer::shutdown;
    let _status: fn(&AukiPeer) -> AukiPeerStatus = AukiPeer::status;
    let _context: fn(&AukiPeer) -> AukiPeerProtocolContext = AukiPeer::protocol_context;
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
