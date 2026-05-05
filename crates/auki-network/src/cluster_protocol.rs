//! `/auki/cluster/1.0.0` — libp2p request-response protocol exchanging
//! [`ParticipantInfo`] between cluster peers.
//!
//! This is the libp2p half of the **one schema, two transports** promise
//! ([`crate::participant`]). The wire content is identical to the JSON
//! returned by `GET /api/info` over HTTP; a peer reaches a daemon by URL
//! (Park) or via libp2p (another participant) and parses the same struct
//! out of either wire.
//!
//! ## Wire format
//!
//! - Request: [`ClusterRequest`], a unit struct serializing as JSON
//!   `null`. Empty by design — the protocol's only purpose is fetching
//!   the responder's [`ParticipantInfo`], so no request payload is needed.
//! - Response: [`ParticipantInfo`], same JSON as `GET /api/info`.
//! - Codec: [`libp2p::request_response::json::Behaviour`] — JSON over the
//!   underlying libp2p stream framing; one round-trip per query;
//!   [`REQUEST_TIMEOUT`] applies.
//!
//! ## How a consumer uses it
//!
//! The behaviour does **not** auto-respond. When a peer sends a request,
//! the receiver gets `request_response::Event::Message::Request{ channel,
//! .. }` and is responsible for filling in its current
//! [`ParticipantInfo`] and calling `behaviour.cluster.send_response(
//! channel, info)`. This is what lets `auki-py`'s `participant_provider`
//! callable plug in: the responder invokes the callable per request so
//! `session_now_ns` is fresh on each reply.
//!
//! Higher-level orchestration — auto-dialing peers from `cluster.json`,
//! tracking `Joined` / `Left` events, holding a peer state map — lives in
//! `cluster_runtime` (deliverable #4). Consumers that want fine control
//! (Sentinel) drive the swarm event loop themselves and call
//! [`send_request`][libp2p::request_response::Behaviour::send_request] /
//! [`send_response`][libp2p::request_response::Behaviour::send_response]
//! directly.

use crate::ParticipantInfo;
use libp2p::{
    StreamProtocol,
    request_response::{self, ProtocolSupport},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// libp2p protocol id for the cluster participant exchange. Stable; do
/// not change without coordinating with consumers (Boosterapp, Sentinel,
/// Park, plus any cross-language reimplementation).
pub const CLUSTER_PROTOCOL: &str = "/auki/cluster/1.0.0";

/// Per-request timeout. A peer that doesn't respond inside this window
/// surfaces as `OutboundFailure::Timeout` to the requester. Matched to
/// the swarm's 60 s idle-connection timeout — the request budget is
/// shorter than the idle budget so a stuck peer doesn't keep the
/// connection alive past its useful life.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Request body for [`CLUSTER_PROTOCOL`]. A unit struct that serializes
/// as JSON `null`.
///
/// Empty by design: the protocol's only operation is fetching the
/// responder's [`ParticipantInfo`], so no request payload is needed. The
/// type exists (rather than using `()`) so the behaviour's generic
/// parameters carry an unambiguous, named, documented type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterRequest;

/// Response payload for [`CLUSTER_PROTOCOL`]. Type alias for clarity at
/// call sites; the wire shape is [`ParticipantInfo`] (same JSON as
/// `GET /api/info`).
pub type ClusterResponse = ParticipantInfo;

/// Request-response behaviour speaking JSON over [`CLUSTER_PROTOCOL`].
/// Length-delimited via the underlying libp2p stream; one round-trip per
/// cluster query.
pub type Behaviour = request_response::json::Behaviour<ClusterRequest, ClusterResponse>;

/// Construct the cluster-protocol behaviour. Inbound and outbound
/// support are both enabled (every peer can both ask and answer).
pub fn behaviour() -> Behaviour {
    request_response::json::Behaviour::new(
        [(
            StreamProtocol::new(CLUSTER_PROTOCOL),
            ProtocolSupport::Full,
        )],
        request_response::Config::default().with_request_timeout(REQUEST_TIMEOUT),
    )
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PeerIdentity;
    use crate::swarm::{Behaviour as SwarmBehaviour, BehaviourEvent, SwarmConfig, build_swarm};
    use futures::StreamExt;
    use libp2p::Swarm;
    use libp2p::swarm::SwarmEvent;

    #[test]
    fn protocol_id_is_locked() {
        // Wire format. If this test fails, you're looking at a breaking
        // change — coordinate with Boosterapp, Sentinel, Park, and any
        // cross-language reimplementation before touching it.
        assert_eq!(CLUSTER_PROTOCOL, "/auki/cluster/1.0.0");
    }

    #[test]
    fn request_serializes_as_json_null() {
        // `ClusterRequest` is a unit struct; serde renders it as `null`.
        // Locking this means the request shape is unambiguous on the
        // wire — a future field addition would require a version bump.
        let req = ClusterRequest;
        let json = serde_json::to_string(&req).expect("serialize");
        assert_eq!(json, "null");
        let back: ClusterRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(req, back);
    }

    /// Fixture used by the two-peer exchange test. Static values so the
    /// assertion is exact; in real consumers the responder fills these
    /// per request from live session state.
    fn fixture_info(peer_id: libp2p::PeerId) -> ParticipantInfo {
        ParticipantInfo {
            app: "test-b".into(),
            name: "fixture".into(),
            session_id: "00000000-0000-4000-8000-000000000001".into(),
            session_clock_id: "test/session-monotonic".into(),
            session_clock_hash: "deadbeef".into(),
            session_now_ns: 42,
            cluster_joined_at_ns: None,
            peer_id,
            app_instance: "00163eabcdef".into(),
        }
    }

    fn test_tcp_config(agent_version: &str) -> SwarmConfig {
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: agent_version.into(),
            enable_mdns: false,
            enable_relay_server: false,
        }
    }

    async fn wait_for_listen_addr(swarm: &mut Swarm<SwarmBehaviour>) -> libp2p::Multiaddr {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                    return address;
                }
            }
        })
        .await
        .expect("listen addr did not appear within timeout")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_peers_exchange_participant_info_over_tcp() {
        let id_a = PeerIdentity::from_seed(&[20u8; 32]);
        let id_b = PeerIdentity::from_seed(&[21u8; 32]);
        let pid_b = id_b.peer_id();

        let mut a = build_swarm(&id_a, test_tcp_config("test-a/0")).unwrap();
        let mut b = build_swarm(&id_b, test_tcp_config("test-b/0")).unwrap();

        let b_info = fixture_info(pid_b);

        let addr_a = wait_for_listen_addr(&mut a).await;
        b.dial(addr_a).expect("dial");

        // Drive both swarms until A has a response holding B's info.
        // A sends its request as soon as the connection to B is up;
        // B, on receiving the request, sends back the fixture info.
        let received = tokio::time::timeout(Duration::from_secs(10), async {
            let mut request_sent = false;
            loop {
                tokio::select! {
                    Some(event) = a.next() => match event {
                        SwarmEvent::ConnectionEstablished { peer_id, .. }
                            if !request_sent && peer_id == pid_b =>
                        {
                            a.behaviour_mut()
                                .cluster
                                .send_request(&pid_b, ClusterRequest);
                            request_sent = true;
                        }
                        SwarmEvent::Behaviour(BehaviourEvent::Cluster(
                            request_response::Event::Message {
                                message: request_response::Message::Response { response, .. },
                                ..
                            },
                        )) => return response,
                        _ => {}
                    },
                    Some(event) = b.next() => {
                        if let SwarmEvent::Behaviour(BehaviourEvent::Cluster(
                            request_response::Event::Message {
                                message: request_response::Message::Request { channel, .. },
                                ..
                            },
                        )) = event
                        {
                            b.behaviour_mut()
                                .cluster
                                .send_response(channel, b_info.clone())
                                .expect("send response");
                        }
                    }
                }
            }
        })
        .await
        .expect("exchange did not complete in time");

        assert_eq!(received, b_info);
    }
}
