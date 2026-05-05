//! Networking substrate for the Auki SDK.
//!
//! M0: the data types — [`PeerIdentity`] derived from a [`Wallet`] via
//! `derive_child("peer/v1")`, [`ReachabilityRecord`] describing how to
//! dial a peer, [`Capability`] tagging what a peer offers, and
//! [`ParticipantInfo`] (the wire shape every Auki participant exchanges to
//! introduce itself; one schema, two transports — `GET /api/info` and the
//! `/auki/cluster/1.0.0` libp2p protocol).
//!
//! M1 (behind the `swarm` feature): a libp2p `Swarm` with TCP + QUIC +
//! Noise + Yamux. Behaviour: `identify` + `ping` always; `mdns` (on by
//! default for daemons) for `_p2p._udp.local.` LAN discovery;
//! `relay::client` always so any peer can be a relay-client; `relay`
//! (the server side) optional, off by default for consumer daemons.
//! Plus a [`swarm::dial_peer`] helper for Park-from-home circuit-relay
//! dialing.
//!
//! ansuz milestone deliverable #1 — see [`cluster_doc`] for the
//! `cluster.json` discovery-doc spec and loader. Always available
//! (no feature gate); native-only because the loader uses `std::fs`.

use auki_identity::Wallet;
use libp2p_identity::{Keypair, PeerId, PublicKey, ed25519};
use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};

pub mod cluster_doc;
pub mod participant;
pub use participant::ParticipantInfo;

#[cfg(feature = "swarm")]
pub mod swarm;

#[cfg(feature = "swarm")]
pub mod cluster_protocol;

#[cfg(feature = "swarm")]
pub mod cluster_runtime;

#[cfg(feature = "app_instance")]
pub mod app_instance;

/// Label used when deriving a wallet's peer key. Stable; do not change.
///
/// `Wallet::derive_child(PEER_DERIVATION_LABEL)` yields the ed25519 keypair
/// the SDK uses for libp2p connections. Keeping this label fixed means the
/// peer identity is regenerable from a wallet seed backup alone.
pub const PEER_DERIVATION_LABEL: &str = "peer/v1";

// ─── PeerIdentity ────────────────────────────────────────────────────────────

/// libp2p peer identity for a node. Holds the ed25519 keypair libp2p uses
/// for connection-level authentication; treat instances as sensitive.
///
/// Construct via [`PeerIdentity::from_wallet`] — this is the canonical
/// path. [`PeerIdentity::from_seed`] is provided for tooling that already
/// has the derived peer seed in hand (e.g. a key store that cached it
/// instead of re-deriving from the wallet each session).
#[derive(Clone)]
pub struct PeerIdentity {
    keypair: Keypair,
}

impl PeerIdentity {
    /// Derive the peer identity from `wallet`. Equivalent to
    /// `PeerIdentity::from_seed(&wallet.derive_child("peer/v1").seed())`.
    /// A backup of the wallet seed is sufficient to regenerate this.
    pub fn from_wallet(wallet: &Wallet) -> Self {
        let peer_wallet = wallet.derive_child(PEER_DERIVATION_LABEL);
        Self::from_seed(&peer_wallet.seed())
    }

    /// Construct directly from a 32-byte ed25519 seed. Same seed → same
    /// keypair → same `PeerId`. The seed is consumed (zeroized) by
    /// `libp2p-identity`'s ed25519 constructor; we copy first so the
    /// caller's buffer stays intact.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let mut seed_copy = *seed;
        let secret = ed25519::SecretKey::try_from_bytes(&mut seed_copy)
            .expect("ed25519::SecretKey accepts any 32 bytes");
        let kp = ed25519::Keypair::from(secret);
        Self {
            keypair: Keypair::from(kp),
        }
    }

    /// libp2p `Keypair` for swarm construction in M1. Holds the secret;
    /// don't hand this out beyond the swarm.
    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    /// libp2p public key — safe to publish. Round-trips through the
    /// underlying ed25519 bytes.
    pub fn public_key(&self) -> PublicKey {
        self.keypair.public()
    }

    /// libp2p `PeerId` — multihash of the protobuf-encoded public key.
    /// This is the dialable identity: stable for a given wallet, safe to
    /// publish, what shows up in multiaddrs as `/p2p/<peer-id>`.
    pub fn peer_id(&self) -> PeerId {
        self.keypair.public().to_peer_id()
    }
}

// ─── ReachabilityRecord ──────────────────────────────────────────────────────

/// What a peer advertises about how to reach it: the peer id, one or more
/// dialable multiaddrs, the named capabilities it offers, and a last-seen
/// timestamp for staleness pruning.
///
/// This is the on-the-wire shape for peer discovery — published to
/// whatever directory is in scope (LAN mDNS for M1's local case;
/// Discovery Service for the cross-network case). The fields are
/// intentionally minimal; richer metadata (load, geographic hints,
/// operator-defined tags) can ride alongside as a future struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityRecord {
    pub peer_id: PeerId,
    #[serde(with = "multiaddr_vec_serde")]
    pub addresses: Vec<Multiaddr>,
    pub capabilities: Vec<Capability>,
    pub last_seen_ns: i64,
}

/// `multiaddr` 0.18 dropped its serde feature; we serialize each
/// `Multiaddr` as its canonical text form (`/ip4/.../tcp/...`) and parse
/// back via `FromStr`.
mod multiaddr_vec_serde {
    use multiaddr::Multiaddr;
    use serde::{Deserialize, Deserializer, Serializer, ser::SerializeSeq};
    use std::str::FromStr;

    pub fn serialize<S: Serializer>(addrs: &Vec<Multiaddr>, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(addrs.len()))?;
        for a in addrs {
            seq.serialize_element(&a.to_string())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Multiaddr>, D::Error> {
        let strs: Vec<String> = Vec::deserialize(d)?;
        strs.into_iter()
            .map(|s| Multiaddr::from_str(&s).map_err(serde::de::Error::custom))
            .collect()
    }
}

// ─── Capability ──────────────────────────────────────────────────────────────

/// A namespaced string identifying something a peer offers. Format is
/// `"<namespace>:<name>"` — e.g. `"networking:message-forwarding"`,
/// `"discovery:domain-membership"`, `"compute:cuda-12"`. Forward-extensible
/// without crate changes; new capabilities are just new strings.
///
/// The four canonical networking capabilities are surfaced as `&str`
/// constants on this type; build a `Capability` from them via
/// `Capability::new(Capability::MESSAGE_FORWARDING)` or `"...".into()`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability(pub String);

impl Capability {
    /// Hagall-`rosrelay` parity — small frequent control-plane messages.
    pub const MESSAGE_FORWARDING: &str = "networking:message-forwarding";
    /// Large non-real-time binary transfer.
    pub const BULK_DATA_CHANNEL: &str = "networking:bulk-data-channel";
    /// Real-time media P2P fallback (TURN-style).
    pub const TURN: &str = "networking:turn";
    /// Real-time media one-to-many fan-out (SFU-style).
    pub const SFU: &str = "networking:sfu";

    /// Construct from any string-like.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// The full `"<namespace>:<name>"` string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The `<namespace>` prefix (everything before the first `:`), if any.
    /// Useful for grouping (e.g. "show me all `networking:*` capabilities").
    pub fn namespace(&self) -> Option<&str> {
        self.0.split_once(':').map(|(ns, _)| ns)
    }
}

impl From<&str> for Capability {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for Capability {
    fn from(s: String) -> Self {
        Self(s)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_identity_from_wallet_is_deterministic() {
        let w = Wallet::from_seed(&[7u8; 32]);
        let a = PeerIdentity::from_wallet(&w);
        let b = PeerIdentity::from_wallet(&w);
        assert_eq!(a.peer_id(), b.peer_id());
    }

    #[test]
    fn peer_identity_differs_across_wallets() {
        let w1 = Wallet::from_seed(&[1u8; 32]);
        let w2 = Wallet::from_seed(&[2u8; 32]);
        let p1 = PeerIdentity::from_wallet(&w1);
        let p2 = PeerIdentity::from_wallet(&w2);
        assert_ne!(p1.peer_id(), p2.peer_id());
    }

    /// Locked cross-language conformance vector: `Wallet::from_seed([3u8; 32])`
    /// → `PeerIdentity::from_wallet(...).peer_id()` MUST produce the canonical
    /// libp2p PeerId string below. Any reimplementation in another language
    /// (Python, Go, browser JS) is correct only if it produces this exact string
    /// from the same seed bytes. The chain that's locked: ed25519 keypair from
    /// seed → libp2p `PublicKey` (protobuf-encoded) → multihash → multibase-base58
    /// `PeerId` text form. If anything in that chain drifts, every `cluster.json`
    /// in the wild also drifts. Don't update this string without a coordinated
    /// version bump.
    #[test]
    fn locked_seed_to_peer_id_vector() {
        let w = Wallet::from_seed(&[3u8; 32]);
        let peer = PeerIdentity::from_wallet(&w);
        assert_eq!(
            peer.peer_id().to_string(),
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar",
            "PeerId derivation drifted — see crate docs for the locked recipe"
        );
    }

    #[test]
    fn from_wallet_matches_from_seed_of_derived_child() {
        // The contract: `from_wallet(w)` is `from_seed(w.derive_child("peer/v1").seed())`.
        // Cross-language consumers can rely on this exact recipe.
        let w = Wallet::from_seed(&[42u8; 32]);
        let via_wallet = PeerIdentity::from_wallet(&w);
        let derived = w.derive_child(PEER_DERIVATION_LABEL);
        let via_seed = PeerIdentity::from_seed(&derived.seed());
        assert_eq!(via_wallet.peer_id(), via_seed.peer_id());
    }

    #[test]
    fn from_seed_is_deterministic() {
        let seed = [9u8; 32];
        let a = PeerIdentity::from_seed(&seed);
        let b = PeerIdentity::from_seed(&seed);
        assert_eq!(a.peer_id(), b.peer_id());
    }

    #[test]
    fn from_seed_does_not_mutate_caller_buffer() {
        let seed = [11u8; 32];
        let snapshot = seed;
        let _ = PeerIdentity::from_seed(&seed);
        assert_eq!(seed, snapshot);
    }

    #[test]
    fn pubkey_bytes_match_derived_wallet_pubkey() {
        // Sanity: the libp2p public key wraps the same 32 ed25519 bytes
        // that the derived child wallet exposes. If this ever drifts, the
        // wallet ↔ peer relationship is broken.
        let w = Wallet::from_seed(&[13u8; 32]);
        let derived = w.derive_child(PEER_DERIVATION_LABEL);
        let peer = PeerIdentity::from_wallet(&w);
        let ed_pub = peer
            .public_key()
            .try_into_ed25519()
            .expect("derived peer key is ed25519");
        assert_eq!(ed_pub.to_bytes(), derived.public_key().0);
    }

    #[test]
    fn peer_id_matches_public_key_to_peer_id() {
        let p = PeerIdentity::from_seed(&[17u8; 32]);
        assert_eq!(p.peer_id(), p.public_key().to_peer_id());
    }

    #[test]
    fn reachability_record_round_trips_through_json() {
        let p = PeerIdentity::from_seed(&[19u8; 32]);
        let record = ReachabilityRecord {
            peer_id: p.peer_id(),
            addresses: vec![
                "/ip4/192.168.9.130/tcp/4001".parse().unwrap(),
                "/ip4/192.168.9.130/udp/4001/quic-v1".parse().unwrap(),
            ],
            capabilities: vec![
                Capability::new(Capability::MESSAGE_FORWARDING),
                Capability::new(Capability::TURN),
            ],
            last_seen_ns: 1_745_000_000_000_000_000,
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: ReachabilityRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record, back);
    }

    #[test]
    fn capability_constants_match_spec() {
        // These strings are wire-format. If any of them changes,
        // every consumer needs to coordinate. The test exists to make a
        // change deliberate.
        assert_eq!(
            Capability::MESSAGE_FORWARDING,
            "networking:message-forwarding"
        );
        assert_eq!(
            Capability::BULK_DATA_CHANNEL,
            "networking:bulk-data-channel"
        );
        assert_eq!(Capability::TURN, "networking:turn");
        assert_eq!(Capability::SFU, "networking:sfu");
    }

    #[test]
    fn capability_namespace_extraction() {
        let c = Capability::new(Capability::MESSAGE_FORWARDING);
        assert_eq!(c.namespace(), Some("networking"));
        let bare = Capability::new("no-colon");
        assert_eq!(bare.namespace(), None);
    }

    #[test]
    fn capability_round_trips_through_json() {
        let c = Capability::new(Capability::SFU);
        let json = serde_json::to_string(&c).unwrap();
        let back: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
