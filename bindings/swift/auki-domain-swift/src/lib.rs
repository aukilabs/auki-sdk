//! UniFFI Swift bindings for `auki-domain`.
//!
//! ## Scope (v0 — PR C)
//!
//! Full ClusterManager surface for native iOS / Swift consumers, with
//! parity to `bindings/python/auki-domain-py` plus the upstream-only
//! methods (clock sync, diagnostics) explicitly included per the design
//! spec.
//!
//! See `README.md` for the full API surface description and `src/readme.md`
//! for the implementation breakdown.

use libp2p_identity::PeerId;
use multiaddr::Multiaddr;

uniffi::setup_scaffolding!();

// ─── Custom-type registrations ─────────────────────────────────────
//
// PeerId and Multiaddr custom_type! declarations live in auki-network-swift
// too. UniFFI generates per-crate FfiConverter impls anchored on each
// crate's UniFfiTag — since this crate has its own UniFfiTag, we need our
// own custom_type registrations (with `remote` keyword for foreign types).

uniffi::custom_type!(PeerId, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<PeerId>()
            .map_err(|e| anyhow::anyhow!("invalid peer-id {s:?}: {e}"))
    },
    lower: |p: PeerId| p.to_string(),
});

uniffi::custom_type!(Multiaddr, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<Multiaddr>()
            .map_err(|e| anyhow::anyhow!("invalid multiaddr {s:?}: {e}"))
    },
    lower: |m: Multiaddr| m.to_string(),
});

// Subsequent tasks (5+) add upstream type re-exports, orchestrator
// functions (bootstrap_swift / create_cluster_swift / join_cluster_swift),
// and additional Swift-side adapters.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_id_custom_type_round_trips() {
        let pid = libp2p_identity::Keypair::ed25519_from_bytes([5u8; 32])
            .expect("valid ed25519 seed")
            .public()
            .to_peer_id();
        let s = pid.to_string();
        let back: PeerId = s.parse().expect("canonical PeerId string parses");
        assert_eq!(back, pid);
    }

    #[test]
    fn multiaddr_custom_type_round_trips() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        assert_eq!(addr.to_string().parse::<Multiaddr>().unwrap(), addr);
    }
}
