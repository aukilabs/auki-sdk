use auki_sdk::{
    AukiPeerProtocols, AuthenticatedPeer, AuthenticatedRouteStream, DomainProtocolError,
    DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream, Identity, Peer,
    PreparedPeer, Session,
};

fn register_custom_protocol(protocols: &AukiPeerProtocols) {
    let spec = DomainProtocolSpec::new("/auki/example/1.0.0", 4, 4096).unwrap();
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

    let _register = register_custom_protocol as fn(&AukiPeerProtocols);
    let _prepared: Option<PreparedPeer> = None;
    let _registration: Option<DomainProtocolRegistration> = None;
    let _error: Option<DomainProtocolError> = None;
    let _stream: Option<AuthenticatedRouteStream> = None;
}
