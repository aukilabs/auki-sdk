//! UniFFI Swift bindings — thin scaffolding host.
//!
//! Per the SDK Swift binding expansion design spec (revision 2), the
//! actual UniFFI proc-macros live on the upstream types under the
//! `swift-bindings` cargo feature. This crate's only job is to call
//! `uniffi::setup_scaffolding!()`, which aggregates the metadata emitted
//! by upstream crates (`auki-identity::Wallet`,
//! `auki-network::PeerIdentity`) into a single `cdylib`/`staticlib` that
//! Swift consumes. The `pub use` re-exports below make the upstream
//! types visible to UniFFI's metadata scanner.

pub use auki_identity_rs::Wallet;
// D14 deliberately keeps this deprecated canonical-Identity adapter for the
// pinned Swift line for one release. Stage 2 replaces the Swift runtime rather
// than teaching this compatibility scaffold the new Domain lifecycle.
#[allow(deprecated)]
pub use auki_network_rs::PeerIdentity;

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the upstream types are constructable through the
    /// binding crate's re-exports, and the FFI-friendly methods produce
    /// the expected deterministic outputs. This is the proof that the
    /// scaffolding + feature-flagged annotations land coherently.
    #[test]
    #[allow(deprecated)]
    fn wallet_and_peer_identity_round_trip_through_re_exports() {
        let wallet = Wallet::from_seed(vec![42u8; 32]).expect("32-byte seed");
        let wallet_again = Wallet::from_seed(vec![42u8; 32]).expect("32-byte seed");
        assert_eq!(wallet.wallet_id_str(), wallet_again.wallet_id_str());

        let peer = PeerIdentity::from_wallet(wallet.clone());
        let peer_again = PeerIdentity::from_wallet(wallet_again.clone());
        assert_eq!(peer.peer_id_string(), peer_again.peer_id_string());
        assert!(peer.peer_id_string().starts_with("12D3KooW"));
    }
}
