#![cfg(feature = "browser_runtime")]

use auki_domain::browser_session::BrowserDomainSession;
use auki_identity::Wallet;
use auki_network::PeerIdentity;

#[test]
fn browser_session_peer_id_uses_shared_identity() {
    let wallet = Wallet::from_seed(&[3u8; 32]);
    let session = BrowserDomainSession::new(PeerIdentity::from_wallet(&wallet));

    assert_eq!(
        session.peer_id(),
        "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
    );
}
