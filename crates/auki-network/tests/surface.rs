use auki_identity::Wallet;
use auki_network::{Capability, PEER_DERIVATION_LABEL, PeerIdentity, ReachabilityRecord};
use libp2p_identity::Keypair;

#[test]
fn rust_root_api_remains_source_compatible() {
    let wallet = Wallet::from_seed(&[3u8; 32]);
    let peer = PeerIdentity::from_wallet(&wallet);

    assert_eq!(
        PEER_DERIVATION_LABEL, "peer/v1",
        "peer derivation label is part of the cross-language identity contract"
    );
    assert_eq!(
        peer.peer_id().to_string(),
        "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
    );
    assert_eq!(
        Capability::new(Capability::MESSAGE_FORWARDING)
            .namespace()
            .unwrap(),
        "networking"
    );

    let record = ReachabilityRecord {
        peer_id: peer.peer_id(),
        addresses: vec!["/ip4/127.0.0.1/tcp/4001".parse().unwrap()],
        capabilities: vec![Capability::new(Capability::TURN)],
        last_seen_ns: 42,
    };
    let json = serde_json::to_string(&record).unwrap();
    let decoded: ReachabilityRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, record);
}

#[test]
fn peer_identity_exports_libp2p_private_key_protobuf_for_jslibp2p() {
    let peer = PeerIdentity::from_seed(&[9u8; 32]);

    let encoded = peer.private_key_protobuf();
    let decoded = Keypair::from_protobuf_encoding(&encoded).unwrap();

    assert_eq!(decoded.public().to_peer_id(), peer.peer_id());
}
