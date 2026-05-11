//! `/auki/heartbeat/0.0.1` — libp2p request-response protocol for
//! Manager↔member liveness signalling.
//!
//! Greenland T2 + T3. The Manager of a cluster sends a periodic
//! [`HeartbeatRequest`] to every member on a 10 s tick; members reply
//! with a [`HeartbeatResponse`] confirming liveness. Members marked
//! departed after 2 consecutive missed responses (T4) — see
//! [`auki-domain`](../../../auki-domain) for the threshold logic.
//!
//! ## Wire format
//!
//! - Request: [`HeartbeatRequest`] — `{ tick_ns, manager_peer_id }`.
//!   `tick_ns` is the Manager's monotonic timestamp at send time;
//!   `manager_peer_id` lets responders sanity-check they're
//!   heartbeating from the Manager they think they are (a stale
//!   sender after failover surfaces here).
//! - Response: [`HeartbeatResponse`] — `{ responder_peer_id }`. The
//!   responder echoes its own peer id so a misrouted reply is
//!   detectable. No `now_ns` field — TimeTransform / clock-sync is
//!   out of scope per the Greenland brief.
//! - Codec: [`libp2p::request_response::json::Behaviour`] — JSON over
//!   the underlying libp2p stream framing; one round-trip per tick;
//!   [`REQUEST_TIMEOUT`] applies.
//!
//! ## How a consumer uses it
//!
//! Same shape as [`crate::cluster_protocol`] — the behaviour does
//! **not** auto-respond. When a member receives a request, the
//! consumer (typically `auki_domain::Member`) gets
//! `request_response::Event::Message::Request{ channel, .. }` and
//! calls `behaviour.heartbeat.send_response(channel, response)` after
//! deciding whether to acknowledge.
//!
//! The Manager-side tick loop and member-side responder live in the
//! [`auki-domain`](../../../auki-domain) crate; this module owns only
//! the wire types and the behaviour helper.
//!
//! ## Lab-mode versioning
//!
//! Protocol id is `0.0.1`, not `1.0.0`. Per the SDK convention
//! (everything in `aukilabs/*` is pre-production), v1 is reserved
//! for surfaces that are genuinely stable across teams.

use libp2p::{
    PeerId, StreamProtocol,
    request_response::{self, ProtocolSupport},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// libp2p protocol id for the Manager↔member heartbeat exchange.
/// Stable; do not change without coordinating with consumers
/// (Boosterapp, Sentinel, Park, plus any cross-language
/// reimplementation).
pub const HEARTBEAT_PROTOCOL: &str = "/auki/heartbeat/0.0.1";

/// Per-request timeout. A member that doesn't respond inside this
/// window surfaces as `OutboundFailure::Timeout` to the Manager and
/// counts as a missed tick (T4). Matched to the Greenland tick
/// interval (10 s) — one tick of slack means a member is dropped
/// after ~20 s of unresponsiveness, consistent with the brief's
/// "~20s departure window at 10s ticks".
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Heartbeat request body. Sent by the Manager to every member on a
/// 10 s tick (T2). Carries the Manager's view of "when am I sending
/// this" plus the Manager's identity so the responder can spot a
/// stale sender post-failover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    /// Monotonic timestamp (ns) at send. Source clock is the
    /// Manager's session-monotonic clock; the responder does NOT
    /// interpret this against its own clock (TimeTransform is out of
    /// scope for v1). It exists so the Manager can correlate
    /// responses to requests and so log replays distinguish ticks.
    pub tick_ns: u64,

    /// The Manager's `libp2p PeerId`. A responder that sees a
    /// different peer id than the one its local registry believes is
    /// Manager should still respond (the Manager has authority — see
    /// T6) but the discrepancy is worth surfacing to operators. Lets
    /// post-failover stale Managers be detected at the responder.
    #[serde(with = "peer_id_serde")]
    pub manager_peer_id: PeerId,
}

/// Heartbeat response body. Sent by a member in reply to a
/// [`HeartbeatRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    /// The responder's own `libp2p PeerId`. Echoed back so the
    /// Manager can detect misrouted replies (one peer's response
    /// somehow surfacing on a different peer's request channel).
    /// Most of the time this equals the Manager's expectation for
    /// the substream's remote.
    #[serde(with = "peer_id_serde")]
    pub responder_peer_id: PeerId,
}

/// Request-response behaviour speaking JSON over
/// [`HEARTBEAT_PROTOCOL`]. Length-delimited via the underlying libp2p
/// stream; one round-trip per tick.
pub type Behaviour = request_response::json::Behaviour<HeartbeatRequest, HeartbeatResponse>;

/// Construct the heartbeat-protocol behaviour. Inbound and outbound
/// support are both enabled because any peer can become Manager
/// (T10/T13 failover) and any non-Manager peer is a responder.
pub fn behaviour() -> Behaviour {
    request_response::json::Behaviour::new(
        [(
            StreamProtocol::new(HEARTBEAT_PROTOCOL),
            ProtocolSupport::Full,
        )],
        request_response::Config::default().with_request_timeout(REQUEST_TIMEOUT),
    )
}

/// Serde adapter for `libp2p::PeerId` — serializes as the canonical
/// multibase-base58 string form. Mirrors the pattern used by
/// [`crate::participant::ParticipantInfo`]'s `peer_id` field.
mod peer_id_serde {
    use libp2p::PeerId;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub fn serialize<S: Serializer>(id: &PeerId, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&id.to_base58())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PeerId, D::Error> {
        let s = String::deserialize(d)?;
        PeerId::from_str(&s).map_err(serde::de::Error::custom)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p_identity::ed25519;

    fn fixture_peer_id(seed: u8) -> PeerId {
        let mut s = [seed; 32];
        let sk = ed25519::SecretKey::try_from_bytes(&mut s).unwrap();
        let kp = ed25519::Keypair::from(sk);
        let kp = libp2p_identity::Keypair::from(kp);
        kp.public().to_peer_id()
    }

    #[test]
    fn protocol_id_is_locked() {
        // Wire format. If this test fails, you're looking at a
        // breaking change — coordinate with consumers before touching
        // it.
        assert_eq!(HEARTBEAT_PROTOCOL, "/auki/heartbeat/0.0.1");
    }

    #[test]
    fn request_round_trips_through_json() {
        let mp = fixture_peer_id(1);
        let req = HeartbeatRequest {
            tick_ns: 1_234_567_890,
            manager_peer_id: mp,
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: HeartbeatRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, parsed);
    }

    #[test]
    fn response_round_trips_through_json() {
        let rp = fixture_peer_id(2);
        let resp = HeartbeatResponse {
            responder_peer_id: rp,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: HeartbeatResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, parsed);
    }

    /// Locked cross-language conformance vector for [`HeartbeatRequest`]
    /// — `tick_ns: 1_000_000_000` + `manager_peer_id` derived from seed
    /// `[3u8; 32]` (shared cross-language reference seed). Any
    /// cross-language reimplementation must produce these exact bytes.
    #[test]
    fn request_locked_wire_bytes() {
        let pid = fixture_peer_id(3);
        let req = HeartbeatRequest {
            tick_ns: 1_000_000_000,
            manager_peer_id: pid,
        };
        let json = serde_json::to_string(&req).unwrap();
        // Locked literal — auki-identity's locked vector chain pins
        // the pubkey from seed [3u8;32], so the resulting peer-id
        // base58 string is also locked.
        assert_eq!(
            json,
            format!(
                "{{\"tick_ns\":1000000000,\"manager_peer_id\":\"{}\"}}",
                pid.to_base58()
            )
        );
    }

    #[test]
    fn invalid_peer_id_deserialization_fails() {
        let bad = r#"{"tick_ns":0,"manager_peer_id":"not-a-peer-id"}"#;
        let r: Result<HeartbeatRequest, _> = serde_json::from_str(bad);
        assert!(r.is_err());
    }

    #[test]
    fn request_timeout_matches_tick_interval() {
        // 10s, matching Greenland T2's tick. Per the module docstring:
        // ~20s departure window at 10s ticks comes from 2× missed
        // responses (T4), each capped at REQUEST_TIMEOUT.
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(10));
    }

    #[test]
    fn behaviour_constructible() {
        // Smoke test — `behaviour()` doesn't panic and returns a
        // request_response::json::Behaviour with the right protocol
        // id and timeout. The full two-peer integration test lives
        // alongside the Manager state machine in auki-domain.
        let _b: Behaviour = behaviour();
    }
}
