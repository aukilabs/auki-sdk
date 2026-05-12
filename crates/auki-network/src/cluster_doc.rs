//! `ClusterDoc` — Discovery's wire shape for a cluster's membership.
//!
//! A `ClusterDoc` carries the peer-ids and dialable multiaddrs of every
//! daemon in a cluster, plus Discovery-stamped metadata (creation
//! timestamp, current Manager peer-id). It is produced by Discovery in
//! response to `register` / `fetch` / `subscribe` calls, and is the
//! envelope the Manager broadcasts over `/auki/registry/0.0.1`. It is
//! NOT something an integrator constructs by hand — Discovery is the
//! single source of truth for cluster membership.
//!
//! ## Construction
//!
//! Use [`crate::discovery_client::DiscoveryClient`]'s
//! `create_cluster` / `register` / `fetch` methods to obtain a
//! `ClusterDoc` from a running Discovery instance. Cluster runtimes are
//! constructed through [`auki_domain::init_domain`] (and, when it
//! lands, `join_domain`) — both of which call into Discovery and feed
//! the resulting `ClusterDoc` straight into the runtime so consumers
//! never have to handle one directly.
//!
//! ## Greenland-era fields
//!
//! `created_ns` (server-stamped, immutable, sort key for
//! `GET /clusters/latest`) and `current_manager_peer_id` (rotated by
//! the signed `POST /clusters/{name}/manager` handoff after every
//! Manager failover) were added in Greenland T8 / T14. Pre-Greenland
//! consumers of older Discovery instances see `created_ns = 0` and
//! `current_manager_peer_id = None` via `#[serde(default)]`.

use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};

// ─── ClusterDoc ──────────────────────────────────────────────────────────────

/// The cluster-membership snapshot Discovery produces on `register` /
/// `fetch` / `subscribe` and the Manager broadcasts on
/// `/auki/registry/0.0.1`.
///
/// `peers` is an ordered list — Discovery's broadcast preserves the
/// insertion order for stable diffs across SSE events. A peer with
/// zero `addresses` is permitted and round-trips cleanly; consumers
/// that need a dialable peer should filter such entries out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterDoc {
    /// Schema version. Currently always `1`; bumped only on an
    /// incompatible shape change. Additive fields use
    /// `#[serde(default)]` and don't bump the version.
    pub version: u32,
    /// Cluster identifier — Greenland's canonical `{wallet_id}/{name}`
    /// for user-named Domains or the reserved `"Vinland"` singleton.
    /// Used as the path segment in Discovery's `/clusters/{name}`
    /// surface.
    pub cluster_name: String,
    /// Discovery-stamped cluster creation timestamp, in nanoseconds
    /// since the Unix epoch from Discovery's server-side monotonic
    /// clock. Set once when Discovery first accepts the cluster's
    /// `POST /clusters/{name}` and immutable thereafter; the Manager
    /// preserves it across every broadcast and across Manager
    /// failover. Sort key for `GET /clusters/latest` (Greenland T8) —
    /// taking the Manager's announced bootstrap time as the sort key
    /// would fold in real per-peer clock skew, so the server's own
    /// clock owns the field.
    ///
    /// `#[serde(default)]` keeps pre-Greenland Discovery responses
    /// loading: a missing field comes back as `0`. Post-Greenland
    /// Discovery always populates it.
    #[serde(default)]
    pub created_ns: u64,
    /// Live Manager peer-id, written by Discovery and refreshed every
    /// time the cluster's Manager changes. Initially set when
    /// Discovery accepts the cluster's `POST /clusters/{name}`
    /// (Greenland T8) to the creating peer; rotated by the new
    /// Manager's signed `POST /clusters/{name}/manager` handoff after
    /// every failover (Greenland T14). Late joiners read this off
    /// `GET /clusters/latest` to route their `JoinRequest` at the
    /// live Manager rather than a dead one.
    ///
    /// Distinct from the Domain-creator wallet — that wallet's role
    /// is one-shot at Domain creation (Greenland T6 / T1); this is
    /// the current peer-identity authority for registry mutations.
    /// `None` only for pre-Greenland clusters or, transiently, for a
    /// fresh cluster between creation and the first peer
    /// registration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_manager_peer_id: Option<PeerId>,
    /// Ordered list of peers in the cluster.
    pub peers: Vec<ClusterPeer>,
}

/// One peer entry in a [`ClusterDoc`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterPeer {
    /// Required. libp2p Noise rejects connection-time mismatches; this
    /// is what gives identity continuity across daemon restarts. A
    /// daemon that reboots is recognizable as the same daemon because
    /// the same wallet seed produces the same `peer_id`.
    pub peer_id: PeerId,
    /// Dialable multiaddrs for this peer. Direct (`/ip4/.../tcp/...`)
    /// or circuit-relay-mediated (`/p2p/<relay>/p2p-circuit/p2p/<peer>`)
    /// are both accepted; the swarm picks among them at dial time.
    /// Empty list is allowed (Discovery may temporarily lose a peer's
    /// reachability info between heartbeats).
    #[serde(with = "multiaddr_vec_serde")]
    pub addresses: Vec<Multiaddr>,
    /// Optional advisory `app_id` (e.g. `"boosterapp"`, `"sentinel"`).
    /// **Not authoritative** — the wire-borne `app_id` (from the
    /// daemon's `/api/info`) wins. Used for fail-fast operator logging
    /// when Discovery's view and the daemon disagree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_app_id: Option<String>,
    /// Optional human-readable note. Discovery preserves it but
    /// nothing in the SDK reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `multiaddr` 0.18 dropped its serde feature; we serialize each
/// `Multiaddr` as its canonical text form (`/ip4/.../tcp/...`) and parse
/// back via `FromStr`. Mirrors the adapter used by `ReachabilityRecord`.
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
            .map(|s| {
                Multiaddr::from_str(&s)
                    .map_err(|e| serde::de::Error::custom(format!("multiaddr: parse {s:?}: {e}")))
            })
            .collect()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc() -> ClusterDoc {
        let p1 = crate::PeerIdentity::from_seed(&[1u8; 32]).peer_id();
        let p2 = crate::PeerIdentity::from_seed(&[2u8; 32]).peer_id();
        ClusterDoc {
            version: 1,
            cluster_name: "demo-2026-05".to_string(),
            created_ns: 1_715_423_400_000_000_000,
            current_manager_peer_id: Some(p1),
            peers: vec![
                ClusterPeer {
                    peer_id: p1,
                    addresses: vec![
                        "/ip4/192.168.1.10/tcp/4001".parse().unwrap(),
                        "/ip4/192.168.1.10/udp/4001/quic-v1".parse().unwrap(),
                    ],
                    expected_app_id: Some("boosterapp".to_string()),
                    note: Some("robot 1 — K1 NUC".to_string()),
                },
                ClusterPeer {
                    peer_id: p2,
                    addresses: vec!["/ip4/10.0.0.5/tcp/4001".parse().unwrap()],
                    expected_app_id: Some("sentinel".to_string()),
                    note: None,
                },
            ],
        }
    }

    #[test]
    fn round_trips_through_serde_json() {
        let original = sample_doc();
        let json = serde_json::to_string(&original).expect("serialize");
        let loaded: ClusterDoc = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(loaded, original);
    }

    #[test]
    fn missing_greenland_fields_default_to_zero_and_none() {
        // A pre-Greenland Discovery response with no `created_ns` or
        // `current_manager_peer_id` still deserialises cleanly — both
        // fields default-fill.
        let p1 = crate::PeerIdentity::from_seed(&[8u8; 32]).peer_id();
        let json = format!(
            r#"{{
              "version": 1,
              "cluster_name": "pre-greenland",
              "peers": [{{ "peer_id": "{p1}", "addresses": [] }}]
            }}"#
        );
        let doc: ClusterDoc = serde_json::from_str(&json).expect("legacy doc loads");
        assert_eq!(doc.created_ns, 0);
        assert_eq!(doc.current_manager_peer_id, None);
    }

    #[test]
    fn greenland_doc_carries_created_ns_and_manager() {
        let p1 = crate::PeerIdentity::from_seed(&[9u8; 32]).peer_id();
        let p2 = crate::PeerIdentity::from_seed(&[10u8; 32]).peer_id();
        let json = format!(
            r#"{{
              "version": 1,
              "cluster_name": "greenland",
              "created_ns": 1715423400000000000,
              "current_manager_peer_id": "{p1}",
              "peers": [
                {{ "peer_id": "{p1}", "addresses": [] }},
                {{ "peer_id": "{p2}", "addresses": [] }}
              ]
            }}"#
        );
        let doc: ClusterDoc = serde_json::from_str(&json).expect("greenland doc loads");
        assert_eq!(doc.created_ns, 1_715_423_400_000_000_000);
        assert_eq!(doc.current_manager_peer_id, Some(p1));
        assert_eq!(doc.peers.len(), 2);
    }

    #[test]
    fn pretty_serialized_form_omits_none_fields() {
        let p1 = crate::PeerIdentity::from_seed(&[7u8; 32]).peer_id();
        let doc = ClusterDoc {
            version: 1,
            cluster_name: "stability".to_string(),
            created_ns: 0,
            current_manager_peer_id: None,
            peers: vec![ClusterPeer {
                peer_id: p1,
                addresses: vec!["/ip4/127.0.0.1/tcp/4001".parse().unwrap()],
                expected_app_id: None,
                note: None,
            }],
        };
        let json = serde_json::to_string_pretty(&doc).unwrap();
        // None-valued optional fields are skipped on serialise.
        assert!(!json.contains("expected_app_id"));
        assert!(!json.contains("note"));
        assert!(!json.contains("current_manager_peer_id"));
        // `created_ns` is required-shaped on the wire — always serialises.
        assert!(json.contains("created_ns"));
    }
}
