//! `ParticipantInfo` — the SDK-provided wire shape every Auki peer
//! exchanges to introduce itself, served over the libp2p
//! `/auki/info/0.0.1` protocol (cluster-membership-gated; see
//! [`crate::info_protocol`]). The only peer-facing identity surface —
//! HTTP `/api/info` is gone from the cross-app contract (#293).
//!
//! One schema, one source of truth: the SDK constructs this; peers
//! (BoosterApp, Park, Sentinel) don't define their own identity
//! shapes — they instantiate this type with their current state and
//! the runtime serves it. That uniformity is what lets cross-peer
//! operator tooling (status dashboards, ops scripts, debugging) work
//! against one shape rather than per-daemon variants. Apps that also
//! show identity on a local operator UI (Park's browser-facing
//! `/api/info`) serialize the same type verbatim.
//!
//! ## Field semantics
//!
//! - `app` — application identifier (`"boosterapp"`, `"sentinel"`,
//!   `"park"`, …). Same value as the `app_id` carried in every
//!   manifest the daemon writes.
//! - `name` — operator-friendly label (`"k1-walker"`, …),
//!   configurable per device.
//! - `session_id` — UUIDv4 minted at session boot. One daemon run =
//!   one session.
//! - `session_clock_id` / `session_clock_hash` — identifier and
//!   content-addressed hash pinning the session's monotonic clock in
//!   the clock registry.
//! - `session_now_ns` — the session clock's value at the moment this
//!   struct was filled. Pair with the requester's local clock to
//!   derive a `convert_time` transform between the two clocks.
//! - `cluster_joined_at_ns` — session-clock value at the first peer
//!   connection. `None` while the daemon hasn't joined a cluster yet;
//!   set once and sticky thereafter.
//! - `peer_id` — libp2p PeerId derived from
//!   `Wallet::derive_child("peer/v1")`. See [`crate::PeerIdentity`].
//! - `app_instance` — first non-loopback IEEE-administered MAC,
//!   lowercased hex without separators (e.g. `"aabbccddeeff"`).
//!   Stable across daemon restarts on the same hardware.
//! - `is_manager` — `true` if THIS daemon is currently the cluster's
//!   Manager. Populated from the cluster runtime; daemons should not
//!   set it directly.
//! - `manager_peer_id` — canonical libp2p peer-id string (the same
//!   `12D3KooW…` form as `peer_id`) of the current Manager. When
//!   `is_manager` is true, this equals `peer_id`. Populated from the
//!   cluster runtime.
//!
//! ## JSON shape
//!
//! Snake-case field names; `cluster_joined_at_ns` serializes as
//! `null` when `None`; `peer_id` is the canonical
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
//!   "app_instance": "aabbccddeeff",
//!   "is_manager": false,
//!   "manager_peer_id": "12D3KooWAbc..."
//! }
//! ```

use libp2p_identity::PeerId;
use serde::{Deserialize, Serialize};

/// Identity card a peer serves over libp2p `/auki/info/0.0.1`.
/// Constructed by the SDK (with cluster-aware fields populated from
/// the runtime) and served verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantInfo {
    /// Application identifier (`"boosterapp"`, `"sentinel"`, …). Same
    /// value as the `app_id` carried in every manifest the daemon
    /// writes.
    pub app: String,

    /// Operator-friendly label (`"k1-walker"`, …). Configurable per
    /// device.
    pub name: String,

    /// UUIDv4 minted at session boot. One daemon run = one session.
    pub session_id: String,

    /// Identifier of the session's monotonic clock in the clock
    /// registry.
    pub session_clock_id: String,

    /// Content-addressed hash pinning the exact clock-registry entry.
    pub session_clock_hash: String,

    /// The session clock's current value at the moment this struct
    /// was filled.
    pub session_now_ns: u64,

    /// Session-clock value at first peer connection. `None` until
    /// the daemon has connected to at least one cluster peer. Set
    /// once and sticky thereafter. Serializes as `null` when `None`.
    pub cluster_joined_at_ns: Option<u64>,

    /// libp2p PeerId derived from `Wallet::derive_child("peer/v1")`.
    /// JSON form is the canonical multibase-base58 string
    /// (`12D3KooW…`), via `libp2p-identity`'s `serde` feature.
    pub peer_id: PeerId,

    /// First non-loopback IEEE-administered MAC, lowercased hex
    /// without separators (e.g. `"aabbccddeeff"`). Stable across
    /// daemon restarts on the same hardware.
    pub app_instance: String,

    /// `true` if this daemon is currently the cluster's Manager.
    /// Populated from the cluster runtime; daemons should not set
    /// this directly.
    pub is_manager: bool,

    /// Canonical libp2p peer-id string of the current Manager. When
    /// [`is_manager`](Self::is_manager) is `true`, this equals
    /// [`peer_id`](Self::peer_id)'s canonical string form. Populated
    /// from the cluster runtime.
    pub manager_peer_id: String,
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ParticipantInfo {
        use libp2p_identity::{Keypair, ed25519};
        let mut seed = [7u8; 32];
        let secret = ed25519::SecretKey::try_from_bytes(&mut seed)
            .expect("32 bytes is a valid ed25519 secret");
        let kp = Keypair::from(ed25519::Keypair::from(secret));
        let peer_id = kp.public().to_peer_id();
        let peer_id_str = peer_id.to_string();
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
            is_manager: false,
            manager_peer_id: peer_id_str,
        }
    }

    #[test]
    fn round_trips_with_cluster_joined_some() {
        let info = fixture();
        let json = serde_json::to_string(&info).expect("serialize");
        let back: ParticipantInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(info, back);
    }

    #[test]
    fn round_trips_with_cluster_joined_none() {
        let mut info = fixture();
        info.cluster_joined_at_ns = None;
        let json = serde_json::to_string(&info).expect("serialize");
        assert!(
            json.contains("\"cluster_joined_at_ns\":null"),
            "expected cluster_joined_at_ns:null, got {json}"
        );
        let back: ParticipantInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(info, back);
        assert!(back.cluster_joined_at_ns.is_none());
    }

    #[test]
    fn is_manager_true_when_self_is_manager() {
        let mut info = fixture();
        info.is_manager = true;
        info.manager_peer_id = info.peer_id.to_string();
        let json = serde_json::to_string(&info).expect("serialize");
        let back: ParticipantInfo = serde_json::from_str(&json).expect("deserialize");
        assert!(back.is_manager);
        assert_eq!(back.manager_peer_id, back.peer_id.to_string());
    }

    /// Pins the wire shape against rename. Cross-peer tooling
    /// (operator dashboards, ops scripts) reads `ParticipantInfo` by
    /// these exact JSON keys; a rename on the SDK side breaks every
    /// consumer.
    #[test]
    fn json_keys_are_snake_case_and_locked() {
        let info = fixture();
        let value: serde_json::Value = serde_json::to_value(&info).expect("to_value");
        let obj = value.as_object().expect("object");
        let mut keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
        keys.sort();
        let mut expected = vec![
            "app",
            "app_instance",
            "cluster_joined_at_ns",
            "is_manager",
            "manager_peer_id",
            "name",
            "peer_id",
            "session_clock_hash",
            "session_clock_id",
            "session_id",
            "session_now_ns",
        ];
        expected.sort();
        assert_eq!(keys, expected, "wire-shape keys drifted");
    }

    /// Rejects on extra fields — daemons that try to inject custom
    /// identity fields go through SDK upgrades, not local additions.
    #[test]
    fn deserialize_rejects_missing_required_field() {
        let json = r#"{
            "app": "boosterapp",
            "name": "k1-walker",
            "session_id": "x",
            "session_clock_id": "x",
            "session_clock_hash": "x",
            "session_now_ns": 0,
            "cluster_joined_at_ns": null,
            "peer_id": "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw",
            "app_instance": "aabbccddeeff",
            "is_manager": false
        }"#;
        // Missing `manager_peer_id` — deserialization fails.
        let result: Result<ParticipantInfo, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "expected error for missing manager_peer_id"
        );
    }
}
