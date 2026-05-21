//! `ClusterMembership` — per-cluster membership document.
//!
//! The authoritative record of who is in a cluster. Held in RAM by the
//! current Manager, gossiped to peers on every change, served to
//! newcomers on join admission. There is no on-disk persistence: when
//! the Manager dies and a successor is elected (the cluster-internal
//! earliest-joined rule), the successor inherits the document via the
//! last gossip it received before the Manager went silent.
//!
//! ## Per-cluster filename
//!
//! On the wire and (if a peer chose to dump it) on disk, the document
//! is named after its cluster: a cluster named `foo` is `foo.json`, a
//! cluster named `kitchen-2026` is `kitchen-2026.json`. The filename
//! convention is purely lexical; the type itself is cluster-neutral.
//! See [`ClusterMembership::filename`].

use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};

/// The authoritative record of a cluster's membership.
///
/// Held by the current Manager in RAM. Gossiped to peers on every
/// mutation. There is no canonical on-disk form; a peer choosing to
/// snapshot it writes [`Self::filename`] (`{cluster_name}.json`) but
/// the SDK never relies on that file existing.
///
/// ## Election input
///
/// `peers` is an ordered list — insertion order is admission order, and
/// the cluster-internal election rule sorts by `(join_ts_ns, peer_id)`
/// to elect the next Manager when the current one dies. Insertion order
/// is preserved across serde round-trips so the order observed by every
/// peer agrees.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterMembership {
    /// The cluster's name. The wire/disk filename is
    /// `{cluster_name}.json` — see [`Self::filename`].
    pub cluster_name: String,
    /// Members of the cluster, in admission order.
    pub peers: Vec<ClusterMember>,
}

/// One peer entry in a [`ClusterMembership`] document.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterMember {
    /// libp2p peer-id. Stable across daemon restarts when the peer
    /// re-derives its keypair from the same wallet seed.
    pub peer_id: PeerId,
    /// Dialable libp2p multiaddrs. Empty list is allowed; consumers
    /// that need a dialable peer filter such entries out.
    #[serde(with = "multiaddr_vec_serde")]
    pub multiaddrs: Vec<Multiaddr>,
    /// Unix nanoseconds at which the Manager admitted this peer (the
    /// `admit_peer` call timestamp). The election rule sorts on this —
    /// earliest `join_ts_ns` wins, with `peer_id` as the tie-breaker.
    pub join_ts_ns: i64,
    /// Successor token. Issued by the Manager at admit time, signed by
    /// the Manager's libp2p key; the peer presents it (along with a
    /// signature on a Discovery-issued challenge) when claiming the
    /// Manager role at Discovery's rotation endpoint.
    ///
    /// Format is opaque (`Vec<u8>`) for v1: the on-wire shape is still
    /// unresolved (prost / JWT / bare signed JSON). The v1 Discovery
    /// contract skips signature verification entirely, so the bytes
    /// here may be empty or arbitrary for the demo; v2 swaps in a
    /// typed shape once the format is pinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successor_token: Option<Vec<u8>>,
}

impl ClusterMembership {
    /// Construct an empty `ClusterMembership` for a freshly-created
    /// cluster. The creator (initial Manager) appends itself via
    /// [`Self::admit`] before serving the document to its first
    /// joiner.
    pub fn new(cluster_name: impl Into<String>) -> Self {
        Self {
            cluster_name: cluster_name.into(),
            peers: Vec::new(),
        }
    }

    /// The per-cluster filename: `{cluster_name}.json`. This is the
    /// wire identifier newcomers ask for and the disk name a peer
    /// would use if it chose to snapshot the document.
    ///
    /// # Examples
    ///
    /// ```
    /// use auki_domain::ClusterMembership;
    ///
    /// let m = ClusterMembership::new("foo");
    /// assert_eq!(m.filename(), "foo.json");
    ///
    /// let m = ClusterMembership::new("kitchen-2026");
    /// assert_eq!(m.filename(), "kitchen-2026.json");
    /// ```
    pub fn filename(&self) -> String {
        format!("{}.json", self.cluster_name)
    }

    /// Append a peer to the membership. The caller is the current
    /// Manager; it has already issued (and signed, in v2) the
    /// successor token carried in `member.successor_token`. Returns
    /// the index of the new entry — the same index a peer would
    /// observe in the gossiped document.
    pub fn admit(&mut self, member: ClusterMember) -> usize {
        let idx = self.peers.len();
        self.peers.push(member);
        idx
    }
}

/// `multiaddr` 0.18 dropped its serde feature; we serialize each
/// `Multiaddr` as its canonical text form (`/ip4/.../tcp/...`) and
/// parse back via `FromStr`.
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

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use auki_network::PeerIdentity;

    fn peer(seed: u8) -> PeerId {
        PeerIdentity::from_seed(&[seed; 32]).peer_id()
    }

    fn sample_member(seed: u8, join_ts_ns: i64) -> ClusterMember {
        ClusterMember {
            peer_id: peer(seed),
            multiaddrs: vec![
                format!("/ip4/192.168.1.{}/tcp/4001", seed).parse().unwrap(),
                format!("/ip4/192.168.1.{}/udp/4001/quic-v1", seed)
                    .parse()
                    .unwrap(),
            ],
            join_ts_ns,
            successor_token: Some(vec![0xde, 0xad, 0xbe, 0xef]),
        }
    }

    #[test]
    fn new_creates_empty_membership() {
        let m = ClusterMembership::new("foo");
        assert_eq!(m.cluster_name, "foo");
        assert!(m.peers.is_empty());
    }

    #[test]
    fn filename_appends_dot_json() {
        assert_eq!(ClusterMembership::new("foo").filename(), "foo.json");
        assert_eq!(
            ClusterMembership::new("kitchen-2026").filename(),
            "kitchen-2026.json"
        );
        assert_eq!(ClusterMembership::new("").filename(), ".json");
    }

    #[test]
    fn admit_appends_in_order_and_returns_index() {
        let mut m = ClusterMembership::new("foo");
        let i0 = m.admit(sample_member(1, 1_715_000_000_000_000_000));
        let i1 = m.admit(sample_member(2, 1_715_000_000_000_000_001));
        let i2 = m.admit(sample_member(3, 1_715_000_000_000_000_002));
        assert_eq!((i0, i1, i2), (0, 1, 2));
        assert_eq!(m.peers.len(), 3);
        assert_eq!(m.peers[0].peer_id, peer(1));
        assert_eq!(m.peers[1].peer_id, peer(2));
        assert_eq!(m.peers[2].peer_id, peer(3));
    }

    #[test]
    fn round_trips_through_json_with_peers() {
        let mut m = ClusterMembership::new("foo");
        m.admit(sample_member(1, 1_715_423_400_000_000_000));
        m.admit(sample_member(2, 1_715_423_500_000_000_000));

        let json = serde_json::to_string(&m).unwrap();
        let back: ClusterMembership = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn round_trips_empty_cluster() {
        let m = ClusterMembership::new("foo");
        let json = serde_json::to_string(&m).unwrap();
        let back: ClusterMembership = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn round_trips_member_without_successor_token() {
        let mut m = ClusterMembership::new("foo");
        m.admit(ClusterMember {
            peer_id: peer(7),
            multiaddrs: vec!["/ip4/10.0.0.1/tcp/4001".parse().unwrap()],
            join_ts_ns: 1,
            successor_token: None,
        });
        let json = serde_json::to_string(&m).unwrap();
        let back: ClusterMembership = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
        // `None` token is omitted from the JSON so an outsider deserializing
        // pre-Q3 documents doesn't see a `"successor_token": null` field that
        // implies the token is present-but-empty.
        assert!(
            !json.contains("successor_token"),
            "successor_token: None should be skipped in serialization; got {json}"
        );
    }

    #[test]
    fn round_trips_member_with_empty_multiaddrs() {
        let mut m = ClusterMembership::new("foo");
        m.admit(ClusterMember {
            peer_id: peer(5),
            multiaddrs: vec![],
            join_ts_ns: 0,
            successor_token: Some(vec![]),
        });
        let json = serde_json::to_string(&m).unwrap();
        let back: ClusterMembership = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn peer_order_is_preserved_through_round_trip() {
        let mut m = ClusterMembership::new("foo");
        for seed in [9u8, 3, 7, 1, 5] {
            m.admit(sample_member(seed, seed as i64));
        }
        let json = serde_json::to_string(&m).unwrap();
        let back: ClusterMembership = serde_json::from_str(&json).unwrap();
        let seeds_back: Vec<PeerId> = back.peers.iter().map(|p| p.peer_id).collect();
        let seeds_orig: Vec<PeerId> = m.peers.iter().map(|p| p.peer_id).collect();
        assert_eq!(seeds_orig, seeds_back);
    }

    /// The on-wire JSON keys are stable — consumers in other languages
    /// (Python via `auki-domain-py`, future TS/Swift Park clients)
    /// can deserialize against these names. This test locks the key
    /// set; a field rename triggers it.
    #[test]
    fn wire_shape_locked_field_names() {
        let mut m = ClusterMembership::new("foo");
        m.admit(sample_member(1, 42));
        let json = serde_json::to_string(&m).unwrap();
        for key in [
            "\"cluster_name\":",
            "\"peers\":",
            "\"peer_id\":",
            "\"multiaddrs\":",
            "\"join_ts_ns\":",
            "\"successor_token\":",
        ] {
            assert!(json.contains(key), "missing wire key {key:?} in {json}");
        }
    }
}
