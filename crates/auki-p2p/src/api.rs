//! SDK-facing high-level node API.

use crate::{
    AukiP2pNode, AukiP2pNodeConfig, AukiP2pNodeError, ConfiguredPeer, LocalPeerIdentity,
    PeerRelationship, RelationshipFailureRecord, RelationshipFailureScope,
    RelationshipStatusBuildError, RelationshipStatusOptions, build_relationship_status_snapshot,
};
use auki_protocol::v1::{error, status::StatusSnapshot};
use libp2p::{Multiaddr, PeerId};
use std::{collections::BTreeMap, fmt};

/// High-level RFC-first runtime handle for SDK and app code.
pub struct AukiNode {
    node: AukiP2pNode,
    relationships: BTreeMap<PeerId, PeerRelationship>,
}

/// High-level node events that do not expose libp2p stream or frame internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AukiNodeEvent {
    /// A local listener was bound.
    Listening {
        /// Bound listen address.
        address: Multiaddr,
    },
    /// A peer has at least one retained transport connection.
    PeerConnected {
        /// Remote peer id.
        peer_id: PeerId,
    },
    /// A peer connection closed.
    PeerConnectionClosed {
        /// Remote peer id.
        peer_id: PeerId,
        /// Retained connections still active after the close.
        active_connections: usize,
    },
    /// A duplicate connection was closed by local connection policy.
    PeerDuplicateConnectionClosed {
        /// Remote peer id.
        peer_id: PeerId,
    },
    /// An outbound dial failed.
    PeerDialFailed {
        /// Remote peer id, when libp2p associated the failure with one.
        peer_id: Option<PeerId>,
        /// Local diagnostic message.
        error: String,
    },
    /// An inbound connection failed before becoming a peer relationship.
    IncomingConnectionFailed {
        /// Local diagnostic message.
        error: String,
    },
}

/// High-level node API errors.
#[derive(Debug)]
pub enum AukiNodeError {
    /// Low-level node construction or command failed.
    Node(AukiP2pNodeError),
    /// No configured peer exists for the requested peer id.
    UnknownConfiguredPeer {
        /// Requested peer id.
        peer_id: PeerId,
    },
    /// The configured peer exists but has no dial addresses.
    ConfiguredPeerMissingDialAddresses {
        /// Requested peer id.
        peer_id: PeerId,
    },
    /// Status projection failed.
    Status(RelationshipStatusBuildError),
}

impl AukiNode {
    /// Build a high-level node from a local identity and runtime config.
    pub fn new(
        identity: LocalPeerIdentity,
        config: AukiP2pNodeConfig,
    ) -> Result<Self, AukiNodeError> {
        let node = AukiP2pNode::new(identity, config).map_err(AukiNodeError::Node)?;
        let mut this = Self {
            node,
            relationships: BTreeMap::new(),
        };
        for peer in this.node.configured_peers().to_vec() {
            this.relationship_mut(peer.peer_id).configured();
        }
        Ok(this)
    }

    /// Return the local libp2p peer id.
    pub fn peer_id(&self) -> PeerId {
        self.node.peer_id()
    }

    /// Borrow the local identity and current peer binding.
    pub fn identity(&self) -> &LocalPeerIdentity {
        self.node.identity()
    }

    /// Borrow explicitly configured peers.
    pub fn configured_peers(&self) -> &[ConfiguredPeer] {
        self.node.configured_peers()
    }

    /// Borrow one tracked peer relationship.
    pub fn relationship(&self, peer_id: PeerId) -> Option<&PeerRelationship> {
        self.relationships.get(&peer_id)
    }

    /// Return relationship snapshots in deterministic peer-id order.
    pub fn relationships(&self) -> Vec<PeerRelationship> {
        self.relationships.values().cloned().collect()
    }

    /// Configured local bind addresses.
    pub fn configured_listen_addresses(&self) -> &[Multiaddr] {
        self.node.configured_listen_addresses()
    }

    /// Listen addresses observed from libp2p after binding.
    pub fn observed_listen_addresses(&self) -> &[Multiaddr] {
        self.node.observed_listen_addresses()
    }

    /// Operator-supplied advertised addresses.
    pub fn advertised_addresses(&self) -> &[Multiaddr] {
        self.node.advertised_addresses()
    }

    /// Add or replace one explicitly configured peer.
    pub fn upsert_configured_peer(&mut self, peer: ConfiguredPeer) -> Result<(), AukiNodeError> {
        let peer_id = peer.peer_id;
        self.node
            .upsert_configured_peer(peer)
            .map_err(AukiNodeError::Node)?;
        self.relationship_mut(peer_id).configured();
        Ok(())
    }

    /// Dial a configured peer through its configured addresses.
    pub fn dial_configured_peer(&mut self, peer_id: PeerId) -> Result<(), AukiNodeError> {
        let peer = self
            .node
            .configured_peer(peer_id)
            .cloned()
            .ok_or(AukiNodeError::UnknownConfiguredPeer { peer_id })?;
        if peer.dial_addresses.is_empty() {
            return Err(AukiNodeError::ConfiguredPeerMissingDialAddresses { peer_id });
        }

        self.node
            .dial_peer(peer_id, peer.dial_addresses)
            .map_err(AukiNodeError::Node)?;
        self.relationship_mut(peer_id).dialing();
        Ok(())
    }

    /// Wait for the next high-level node event and update relationship state.
    ///
    /// `observed_at` is supplied by the caller so this runtime does not create
    /// or interpret a canonical clock.
    pub async fn next_event(&mut self, observed_at: &str) -> Option<AukiNodeEvent> {
        let event = self.node.next_event().await?;
        let failure_cap = self.node.config().p2p.limits.retained_status_failures;
        Some(match event {
            crate::AukiP2pEvent::Listening { address } => AukiNodeEvent::Listening { address },
            crate::AukiP2pEvent::ConnectionEstablished { peer_id } => {
                self.relationship_mut(peer_id).connected();
                AukiNodeEvent::PeerConnected { peer_id }
            }
            crate::AukiP2pEvent::DuplicateConnectionClosed { peer_id } => {
                AukiNodeEvent::PeerDuplicateConnectionClosed { peer_id }
            }
            crate::AukiP2pEvent::ConnectionClosed { peer_id } => {
                let active_connections = self.node.active_connection_count(peer_id);
                if active_connections == 0 {
                    self.relationship_mut(peer_id)
                        .lost(observed_at.to_owned(), failure_cap);
                }
                AukiNodeEvent::PeerConnectionClosed {
                    peer_id,
                    active_connections,
                }
            }
            crate::AukiP2pEvent::OutgoingConnectionError { peer_id, error } => {
                if let Some(peer_id) = peer_id {
                    let mut failure = RelationshipFailureRecord::new(
                        error::TRANSPORT_FAILED,
                        observed_at.to_owned(),
                        RelationshipFailureScope::Transport,
                    );
                    failure.message = Some(error.clone());
                    self.relationship_mut(peer_id)
                        .degraded(failure, failure_cap);
                }
                AukiNodeEvent::PeerDialFailed { peer_id, error }
            }
            crate::AukiP2pEvent::IncomingConnectionError { error } => {
                AukiNodeEvent::IncomingConnectionFailed { error }
            }
        })
    }

    /// Build an in-process diagnostic status snapshot.
    pub fn status_snapshot(&self, generated_at: &str) -> Result<StatusSnapshot, AukiNodeError> {
        let local_peer = self.node.local_peer_status().map_err(AukiNodeError::Node)?;
        let relationships = self.relationships();
        let options = RelationshipStatusOptions::from_config(&self.node.config().p2p);
        build_relationship_status_snapshot(generated_at, local_peer, &relationships, options)
            .map_err(AukiNodeError::Status)
    }

    fn relationship_mut(&mut self, peer_id: PeerId) -> &mut PeerRelationship {
        self.relationships
            .entry(peer_id)
            .or_insert_with(|| PeerRelationship::new(peer_id))
    }
}

impl fmt::Display for AukiNodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(error) => write!(f, "{error}"),
            Self::UnknownConfiguredPeer { peer_id } => {
                write!(f, "unknown configured peer {peer_id}")
            }
            Self::ConfiguredPeerMissingDialAddresses { peer_id } => {
                write!(f, "configured peer {peer_id} has no dial addresses")
            }
            Self::Status(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AukiNodeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PeerRelationshipState;
    use auki_identity::Wallet;
    use tokio::time::{Duration, timeout};

    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";

    fn identity(seed: u8) -> LocalPeerIdentity {
        let wallet = Wallet::from_seed(vec![seed; 32]).expect("32-byte seed");
        LocalPeerIdentity::from_wallet(wallet, ISSUED_AT, Some("api-test"))
            .expect("local peer identity")
    }

    async fn wait_for_listen_addr(node: &mut AukiNode) -> Multiaddr {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(AukiNodeEvent::Listening { address }) = node.next_event(ISSUED_AT).await
                {
                    return address;
                }
            }
        })
        .await
        .expect("listen address should be emitted")
    }

    #[test]
    fn initializes_configured_peer_relationships() {
        let remote_peer_id = identity(61).peer_id();
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config
            .p2p
            .configured_peers
            .push(ConfiguredPeer::new(remote_peer_id));

        let node = AukiNode::new(identity(60), config).expect("node");
        let relationship = node
            .relationship(remote_peer_id)
            .expect("configured relationship");

        assert_eq!(relationship.state, PeerRelationshipState::Configured);
        assert_eq!(node.configured_peers().len(), 1);

        let snapshot = node.status_snapshot(ISSUED_AT).expect("status snapshot");
        assert_eq!(snapshot.remote_peers.len(), 1);
        assert_eq!(
            snapshot.remote_peers[0].lifecycle_state.as_deref(),
            Some("configured")
        );
    }

    #[test]
    fn upsert_configured_peer_validates_dial_policy() {
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config.p2p.dial_policy = crate::DialPolicy::production_recommended();
        let mut node = AukiNode::new(identity(62), config).expect("node");
        let mut peer = ConfiguredPeer::new(identity(63).peer_id());
        peer.dial_addresses
            .push("/ip4/127.0.0.1/tcp/4001".parse().unwrap());

        let error = node
            .upsert_configured_peer(peer)
            .expect_err("loopback should be rejected by production dial policy");

        assert!(matches!(
            error,
            AukiNodeError::Node(AukiP2pNodeError::DialPolicy(_))
        ));
    }

    #[test]
    fn dial_configured_peer_requires_addresses() {
        let remote_peer_id = identity(65).peer_id();
        let mut config = AukiP2pNodeConfig::dial_only_development();
        config
            .p2p
            .configured_peers
            .push(ConfiguredPeer::new(remote_peer_id));
        let mut node = AukiNode::new(identity(64), config).expect("node");

        let error = node
            .dial_configured_peer(remote_peer_id)
            .expect_err("addresses are required");

        assert!(matches!(
            error,
            AukiNodeError::ConfiguredPeerMissingDialAddresses { peer_id }
                if peer_id == remote_peer_id
        ));
    }

    #[tokio::test]
    async fn configured_peer_dial_updates_relationship_status() {
        let mut dialer =
            AukiNode::new(identity(66), AukiP2pNodeConfig::dial_only_development()).unwrap();
        let mut listener =
            AukiNode::new(identity(67), AukiP2pNodeConfig::loopback_tcp_development()).unwrap();
        let dialer_peer_id = dialer.peer_id();
        let listener_peer_id = listener.peer_id();
        let listener_addr = wait_for_listen_addr(&mut listener).await;
        let mut listener_peer = ConfiguredPeer::new(listener_peer_id);
        listener_peer.dial_addresses.push(listener_addr);

        dialer
            .upsert_configured_peer(listener_peer)
            .expect("configured peer should be accepted");
        dialer
            .dial_configured_peer(listener_peer_id)
            .expect("configured dial should start");
        assert_eq!(
            dialer.relationship(listener_peer_id).unwrap().state,
            PeerRelationshipState::Dialing
        );

        timeout(Duration::from_secs(10), async {
            let mut dialer_observed_listener = false;
            let mut listener_observed_dialer = false;

            loop {
                tokio::select! {
                    event = dialer.next_event(ISSUED_AT) => {
                        if let Some(AukiNodeEvent::PeerConnected { peer_id }) = event {
                            dialer_observed_listener |= peer_id == listener_peer_id;
                        }
                    }
                    event = listener.next_event(ISSUED_AT) => {
                        if let Some(AukiNodeEvent::PeerConnected { peer_id }) = event {
                            listener_observed_dialer |= peer_id == dialer_peer_id;
                        }
                    }
                }

                if dialer_observed_listener && listener_observed_dialer {
                    break;
                }
            }
        })
        .await
        .expect("configured peers should connect");

        assert_eq!(
            dialer.relationship(listener_peer_id).unwrap().state,
            PeerRelationshipState::Connected
        );
        let snapshot = dialer.status_snapshot(ISSUED_AT).expect("status snapshot");
        assert_eq!(
            snapshot.remote_peers[0].lifecycle_state.as_deref(),
            Some("connected")
        );
    }
}
