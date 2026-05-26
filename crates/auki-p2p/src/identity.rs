//! Local peer identity helpers for the libp2p runtime.

use auki_identity::{PublicKey as WalletPublicKey, Wallet};
use auki_protocol::v1::identity::{PeerBinding, PeerBindingError};
use libp2p_identity::{Keypair, PeerId, PublicKey, ed25519};
use std::{fmt, sync::Arc};

/// Stable wallet-child derivation label used for libp2p peer keys.
pub const PEER_DERIVATION_LABEL: &str = "peer/v1";

/// Local wallet authority plus derived libp2p peer identity.
#[derive(Clone)]
pub struct LocalPeerIdentity {
    wallet: Arc<Wallet>,
    keypair: Keypair,
    peer_id: PeerId,
    peer_binding: PeerBinding,
}

/// Errors produced while constructing or refreshing local peer identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalPeerIdentityError {
    /// Derived wallet seed was not 32 bytes.
    InvalidDerivedSeedLength {
        /// Actual seed length.
        len: usize,
    },
    /// Peer binding creation failed.
    PeerBinding(PeerBindingError),
}

impl fmt::Display for LocalPeerIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDerivedSeedLength { len } => {
                write!(f, "derived peer seed length is {len}, expected 32")
            }
            Self::PeerBinding(error) => write!(f, "failed to create peer binding: {error}"),
        }
    }
}

impl std::error::Error for LocalPeerIdentityError {}

impl LocalPeerIdentity {
    /// Derive the local libp2p peer identity from the wallet and create a peer binding.
    pub fn from_wallet(
        wallet: Arc<Wallet>,
        issued_at: &str,
        label: Option<&str>,
    ) -> Result<Self, LocalPeerIdentityError> {
        let keypair = keypair_from_wallet(&wallet)?;
        let peer_id = keypair.public().to_peer_id();
        let peer_binding = PeerBinding::create(&wallet, &peer_id, issued_at, label)
            .map_err(LocalPeerIdentityError::PeerBinding)?;

        Ok(Self {
            wallet,
            keypair,
            peer_id,
            peer_binding,
        })
    }

    /// Borrow the wallet authority key.
    pub fn wallet(&self) -> &Arc<Wallet> {
        &self.wallet
    }

    /// Return the wallet public key used to sign peer bindings.
    pub fn wallet_public_key(&self) -> WalletPublicKey {
        self.wallet.public_key()
    }

    /// Borrow the local libp2p keypair.
    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    /// Return the local libp2p public key.
    pub fn public_key(&self) -> PublicKey {
        self.keypair.public()
    }

    /// Return the local libp2p peer id.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Borrow the current peer binding.
    pub fn peer_binding(&self) -> &PeerBinding {
        &self.peer_binding
    }

    /// Refresh the wallet-signed peer binding for the current peer id.
    pub fn refresh_peer_binding(
        &mut self,
        issued_at: &str,
        label: Option<&str>,
    ) -> Result<(), LocalPeerIdentityError> {
        self.peer_binding = PeerBinding::create(&self.wallet, &self.peer_id, issued_at, label)
            .map_err(LocalPeerIdentityError::PeerBinding)?;
        Ok(())
    }
}

fn keypair_from_wallet(wallet: &Wallet) -> Result<Keypair, LocalPeerIdentityError> {
    let peer_wallet = wallet.derive_child(PEER_DERIVATION_LABEL);
    let seed = peer_wallet.seed();
    let mut seed: [u8; 32] = seed
        .as_slice()
        .try_into()
        .map_err(|_| LocalPeerIdentityError::InvalidDerivedSeedLength { len: seed.len() })?;
    let secret = ed25519::SecretKey::try_from_bytes(&mut seed)
        .expect("ed25519::SecretKey accepts any 32 bytes");
    Ok(Keypair::from(ed25519::Keypair::from(secret)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";

    #[test]
    fn local_peer_identity_uses_locked_peer_derivation() {
        let wallet = Wallet::from_seed(vec![3u8; 32]).expect("32-byte seed");
        let identity = LocalPeerIdentity::from_wallet(wallet, ISSUED_AT, Some("local")).unwrap();

        assert_eq!(
            identity.peer_id().to_string(),
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
        );
        assert_eq!(
            identity
                .peer_binding()
                .verify_for_peer_id(&identity.peer_id())
                .unwrap()
                .peer_id,
            identity.peer_id()
        );
    }

    #[test]
    fn refreshing_peer_binding_keeps_peer_id_and_updates_timestamp() {
        let wallet = Wallet::from_seed(vec![4u8; 32]).expect("32-byte seed");
        let mut identity = LocalPeerIdentity::from_wallet(wallet, ISSUED_AT, None).unwrap();
        let peer_id = identity.peer_id();

        identity
            .refresh_peer_binding("2026-05-26T12:30:00Z", Some("refreshed"))
            .unwrap();
        let verified = identity
            .peer_binding()
            .verify_for_peer_id(&identity.peer_id())
            .unwrap();

        assert_eq!(identity.peer_id(), peer_id);
        assert_eq!(verified.issued_at, "2026-05-26T12:30:00Z");
        assert_eq!(verified.label.as_deref(), Some("refreshed"));
    }
}
