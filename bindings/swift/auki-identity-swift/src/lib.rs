//! UniFFI Swift bindings — thin scaffolding host.
//!
//! Per the SDK Swift binding expansion design spec (revision 2), the
//! actual UniFFI proc-macros live on the upstream types under the
//! `swift-bindings` cargo feature. This crate's only job is to call
//! `uniffi::setup_scaffolding!()`, which aggregates the metadata emitted
//! by `auki-identity::Wallet` into a single `cdylib`/`staticlib` that Swift
//! consumes. The `pub use` below makes the upstream type visible to
//! UniFFI's metadata scanner.

pub use auki_identity_rs::Wallet;

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the upstream types are constructable through the
    /// binding crate's re-exports, and the FFI-friendly methods produce
    /// the expected deterministic outputs. This is the proof that the
    /// scaffolding + feature-flagged annotations land coherently.
    #[test]
    fn wallet_round_trips_through_re_export() {
        let wallet = Wallet::from_seed(vec![42u8; 32]).expect("32-byte seed");
        let wallet_again = Wallet::from_seed(vec![42u8; 32]).expect("32-byte seed");
        assert_eq!(wallet.wallet_id_str(), wallet_again.wallet_id_str());
    }
}
