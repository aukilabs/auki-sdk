//! `ParticipantInfo` — the wire shape every Auki participant exchanges to
//! introduce itself.
//!
//! One schema, two transports:
//!
//! - **HTTP**: `GET /api/info` on the cross-app Control API (see
//!   [`docs/control-api.md`](../../../docs/control-api.md)) returns this
//!   exact JSON.
//! - **libp2p**: the `/auki/cluster/1.0.0` participant protocol — a
//!   request/response exchange where each side sends its own
//!   `ParticipantInfo` to the other (deliverable #3, separate work).
//!
//! Keeping the shape identical across transports means a peer that
//! reaches a daemon by URL (Park) and a peer that reaches it via libp2p
//! (another participant) parse the same struct out of either wire.
//!
//! ## Field semantics
//!
//! - `app` — application identifier (`"boosterapp"`, `"sentinel"`,
//!   `"park"`, …). Same value as the `app` TXT record when the daemon
//!   advertises via mDNS, and the `app_id` field on every manifest the
//!   daemon writes.
//! - `name` — operator-friendly label (`"k1-walker"`, …), configurable
//!   per device.
//! - `session_id` — UUIDv4 minted at session boot. One daemon run = one
//!   session; matches the directory name and the `session_id` carried in
//!   every manifest written during the run. See
//!   [`auki-session`](../../auki-session/README.md).
//! - `session_clock_id` / `session_clock_hash` — identifier and
//!   content-addressed hash pinning the session's monotonic clock in the
//!   clock registry.
//! - `session_now_ns` — the session clock's value at the moment this
//!   struct was filled. Pair with the requester's local clock to derive
//!   a `convert_time` transform between the two clocks.
//! - `cluster_joined_at_ns` — session-clock value at the first peer
//!   connection. `None` while the participant is alone; set once and
//!   sticky thereafter.
//! - `peer_id` — libp2p PeerId derived from
//!   `Wallet::derive_child("peer/v1")`. See [`crate::PeerIdentity`].
//! - `app_instance` — first non-loopback IEEE-administered MAC,
//!   lowercased hex without separators (e.g. `"aabbccddeeff"`). Stable
//!   across daemon restarts on the same hardware.
//!
//! ## JSON shape
//!
//! Snake-case field names; `cluster_joined_at_ns` serializes as `null`
//! when `None`; `peer_id` serializes as the canonical
//! multibase-base58 string (`12D3KooW…`).
//!
//! ```json
//! {
//!   "app": "boosterapp",
//!   "name": "k1-walker",
//!   "session_id": "abc-123-...",
//!   "session_clock_id": "K1-AABBCCDDEEFF/session-monotonic",
//!   "session_clock_hash": "abc123...",
//!   "session_now_ns": 12345678900,
//!   "cluster_joined_at_ns": 1745000000,
//!   "peer_id": "12D3KooWAbc...",
//!   "app_instance": "aabbccddeeff"
//! }
//! ```

use libp2p_identity::PeerId;
use serde::{Deserialize, Serialize};

/// Identity card a participant exchanges peer-to-peer or serves over
/// `GET /api/info`.
///
/// One schema, two transports — see the module-level docs for full
/// field semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantInfo {
    /// Application identifier (`"boosterapp"`, `"sentinel"`, …). Same
    /// value as the `app_id` carried in every manifest this daemon writes.
    pub app: String,

    /// Operator-friendly label (`"k1-walker"`, …). Configurable per device.
    pub name: String,

    /// UUIDv4 minted at session boot. One daemon run = one session.
    pub session_id: String,

    /// Identifier of the session's monotonic clock in the clock registry.
    pub session_clock_id: String,

    /// Content-addressed hash pinning the exact clock-registry entry.
    pub session_clock_hash: String,

    /// The session clock's current value at the moment this struct was
    /// filled.
    pub session_now_ns: u64,

    /// Session-clock value at first peer connection. `None` while the
    /// participant is alone; set once and sticky thereafter. Serializes
    /// as `null` when `None`.
    pub cluster_joined_at_ns: Option<u64>,

    /// libp2p PeerId derived from `Wallet::derive_child("peer/v1")`. See
    /// [`crate::PEER_DERIVATION_LABEL`] and [`crate::PeerIdentity`] for
    /// the derivation recipe. JSON form is the canonical
    /// multibase-base58 string (`12D3KooW…`), via `libp2p-identity`'s
    /// `serde` feature.
    pub peer_id: PeerId,

    /// First non-loopback IEEE-administered MAC, lowercased hex without
    /// separators (e.g. `"aabbccddeeff"`). Stable across daemon restarts
    /// on the same hardware.
    pub app_instance: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical fixture used across the round-trip and golden-bytes
    /// tests. `peer_id` is the libp2p PeerId derived from a deterministic
    /// ed25519 keypair, so the string form is reproducible across runs.
    fn fixture() -> ParticipantInfo {
        // PeerId from a fixed ed25519 secret so the JSON form is stable.
        // Same recipe as `PeerIdentity::from_seed`: 32-byte secret →
        // ed25519 keypair → libp2p PeerId via protobuf+multihash.
        use libp2p_identity::{Keypair, ed25519};
        let mut seed = [7u8; 32];
        let secret = ed25519::SecretKey::try_from_bytes(&mut seed)
            .expect("32 bytes is a valid ed25519 secret");
        let kp = Keypair::from(ed25519::Keypair::from(secret));
        let peer_id = kp.public().to_peer_id();
        ParticipantInfo {
            app: "boosterapp".into(),
            name: "k1-walker".into(),
            session_id: "11111111-2222-4333-8444-555555555555".into(),
            session_clock_id: "K1-AABBCCDDEEFF/session-monotonic".into(),
            session_clock_hash: "abc123".into(),
            session_now_ns: 12_345_678_900,
            cluster_joined_at_ns: Some(1_745_000_000),
            peer_id,
            app_instance: "aabbccddeeff".into(),
        }
    }

    #[test]
    fn round_trip_with_cluster_joined_some() {
        let info = fixture();
        let json = serde_json::to_string(&info).expect("serialize");
        let back: ParticipantInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(info, back);
    }

    #[test]
    fn round_trip_with_cluster_joined_none() {
        let mut info = fixture();
        info.cluster_joined_at_ns = None;
        let json = serde_json::to_string(&info).expect("serialize");
        // None → null per the spec.
        assert!(
            json.contains("\"cluster_joined_at_ns\":null"),
            "expected cluster_joined_at_ns:null, got {json}"
        );
        let back: ParticipantInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(info, back);
        assert!(back.cluster_joined_at_ns.is_none());
    }

    #[test]
    fn json_keys_are_snake_case() {
        let info = fixture();
        let value: serde_json::Value = serde_json::to_value(&info).expect("to_value");
        let obj = value.as_object().expect("object");
        let mut keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
        keys.sort();
        let mut expected = vec![
            "app",
            "name",
            "session_id",
            "session_clock_id",
            "session_clock_hash",
            "session_now_ns",
            "cluster_joined_at_ns",
            "peer_id",
            "app_instance",
        ];
        expected.sort();
        assert_eq!(keys, expected);
    }

    #[test]
    fn golden_bytes_match_fixture() {
        // Locked wire format. If this test fails, you're looking at a
        // breaking change — coordinate with consumers (Park, BoosterApp,
        // Sentinel, the participant protocol) before touching it.
        let info = fixture();
        let json = serde_json::to_string(&info).expect("serialize");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("re-parse our own output");

        // Deterministic peer-id string from the fixed seed.
        let peer_id_str = info.peer_id.to_string();

        let expected = serde_json::json!({
            "app": "boosterapp",
            "name": "k1-walker",
            "session_id": "11111111-2222-4333-8444-555555555555",
            "session_clock_id": "K1-AABBCCDDEEFF/session-monotonic",
            "session_clock_hash": "abc123",
            "session_now_ns": 12_345_678_900u64,
            "cluster_joined_at_ns": 1_745_000_000u64,
            "peer_id": peer_id_str,
            "app_instance": "aabbccddeeff",
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn rejects_missing_field() {
        // Drop `app_instance` — a required field. Deserialization must fail.
        let info = fixture();
        let mut value = serde_json::to_value(&info).expect("to_value");
        value
            .as_object_mut()
            .expect("object")
            .remove("app_instance");
        let result: Result<ParticipantInfo, _> = serde_json::from_value(value);
        assert!(result.is_err(), "missing field should fail to deserialize");
    }

    #[test]
    fn rejects_wrong_type() {
        // session_now_ns must be a u64; a string here is invalid.
        let info = fixture();
        let mut value = serde_json::to_value(&info).expect("to_value");
        value.as_object_mut().expect("object").insert(
            "session_now_ns".into(),
            serde_json::Value::String("not a number".into()),
        );
        let result: Result<ParticipantInfo, _> = serde_json::from_value(value);
        assert!(result.is_err(), "wrong type should fail to deserialize");
    }

    #[test]
    fn rejects_invalid_peer_id() {
        // peer_id must parse as a libp2p PeerId.
        let info = fixture();
        let mut value = serde_json::to_value(&info).expect("to_value");
        value.as_object_mut().expect("object").insert(
            "peer_id".into(),
            serde_json::Value::String("not-a-peer-id".into()),
        );
        let result: Result<ParticipantInfo, _> = serde_json::from_value(value);
        assert!(
            result.is_err(),
            "invalid peer_id string should fail to deserialize"
        );
    }

    #[test]
    fn cluster_joined_field_is_explicit_null_not_omitted() {
        // Wire format: when None, the field must be present with a null
        // value, not omitted. Old `/api/info` shapes that omit unknowns
        // must round-trip differently from None, so this test pins the
        // null vs. missing distinction.
        let mut info = fixture();
        info.cluster_joined_at_ns = None;
        let json = serde_json::to_string(&info).expect("serialize");
        assert!(json.contains("\"cluster_joined_at_ns\":null"));
        assert!(!json.contains("\"cluster_joined_at_ns\":\""));
    }
}
