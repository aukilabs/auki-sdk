use auki_p2p::{
    NodeObservationEvent, NodeObservationStatus, NodeObservations, PeerDisappearanceReason, PeerId,
    SignedApplicationMetadata,
};
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::context::ContextLifecycle;

/// One currently connected peer mutually authenticated for this exact Domain.
///
/// This is an observation only. It is never a membership list, route source,
/// or authorization cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AukiKnownPeer {
    peer_id: PeerId,
    authenticated_until: DateTime<Utc>,
    application: Option<SignedApplicationMetadata>,
}

impl AukiKnownPeer {
    /// Observed remote Peer ID.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Literal expiration of the mutually authenticated remote credential.
    pub fn authenticated_until(&self) -> DateTime<Utc> {
        self.authenticated_until
    }

    /// Optional application metadata signed into the remote credential.
    pub fn application(&self) -> Option<&SignedApplicationMetadata> {
        self.application.as_ref()
    }
}

/// Immutable point-in-time exact-Domain connection observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AukiKnownPeerSnapshot {
    peers: Vec<AukiKnownPeer>,
}

impl AukiKnownPeerSnapshot {
    /// Peers in stable Peer-ID order.
    pub fn peers(&self) -> &[AukiKnownPeer] {
        &self.peers
    }
}

/// One bounded change to the exact-Domain peer observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AukiKnownPeerEvent {
    /// A peer gained its first authenticated live connection.
    Appeared(AukiKnownPeer),
    /// A peer's authenticated claims or expiration changed.
    Updated(AukiKnownPeer),
    /// A peer lost its final eligible connection or credential.
    Disappeared {
        /// Peer that disappeared.
        peer_id: PeerId,
        /// Transport observation reason.
        reason: PeerDisappearanceReason,
    },
}

/// Read-only exact-Domain view of currently connected authenticated peers.
#[derive(Clone)]
pub struct AukiKnownPeers {
    domain_id: Uuid,
    observations: NodeObservations,
    lifecycle: ContextLifecycle,
}

impl AukiKnownPeers {
    pub(crate) fn new(
        domain_id: Uuid,
        observations: NodeObservations,
        lifecycle: ContextLifecycle,
    ) -> Self {
        Self {
            domain_id,
            observations,
            lifecycle,
        }
    }

    /// Read a fresh stable snapshot.
    pub fn snapshot(&self) -> AukiKnownPeerSnapshot {
        let Some(_running) = self.lifecycle.enter() else {
            return AukiKnownPeerSnapshot { peers: Vec::new() };
        };
        let mut peers = self
            .observations
            .snapshot()
            .peers()
            .iter()
            .filter(|observation| observation.domain_id() == self.domain_id)
            .map(|observation| AukiKnownPeer {
                peer_id: observation.peer().peer_id,
                authenticated_until: observation.peer().verified_until,
                application: observation.peer().application.clone(),
            })
            .collect::<Vec<_>>();
        peers.sort_unstable_by_key(|peer| peer.peer_id.to_string());
        AukiKnownPeerSnapshot { peers }
    }

    /// Number of peers in a fresh snapshot.
    pub fn peer_count(&self) -> usize {
        self.snapshot().peers.len()
    }

    /// Subscribe to bounded changes. Lag requires reading a fresh snapshot.
    pub fn subscribe(&self) -> AukiKnownPeerSubscription {
        AukiKnownPeerSubscription {
            domain_id: self.domain_id,
            receiver: self.observations.subscribe(),
            lifecycle: self.lifecycle.clone(),
        }
    }
}

/// Subscription to exact-Domain authenticated connection observations.
pub struct AukiKnownPeerSubscription {
    domain_id: Uuid,
    receiver: broadcast::Receiver<NodeObservationEvent>,
    lifecycle: ContextLifecycle,
}

impl AukiKnownPeerSubscription {
    /// Receive the next relevant event.
    pub async fn recv(&mut self) -> Result<AukiKnownPeerEvent, AukiKnownPeerRecvError> {
        loop {
            tokio::select! {
                biased;
                _ = self.lifecycle.cancelled() => return Err(AukiKnownPeerRecvError::Closed),
                event = self.receiver.recv() => match event {
                    Ok(NodeObservationEvent::Appeared(observation))
                        if observation.domain_id() == self.domain_id =>
                    {
                        return Ok(AukiKnownPeerEvent::Appeared(project(&observation)));
                    }
                    Ok(NodeObservationEvent::Updated(observation))
                        if observation.domain_id() == self.domain_id =>
                    {
                        return Ok(AukiKnownPeerEvent::Updated(project(&observation)));
                    }
                    Ok(NodeObservationEvent::Disappeared { peer_id, domain_id, reason })
                        if domain_id == self.domain_id =>
                    {
                        return Ok(AukiKnownPeerEvent::Disappeared { peer_id, reason });
                    }
                    Ok(NodeObservationEvent::StatusChanged(
                        NodeObservationStatus::Stopped | NodeObservationStatus::Failed(_),
                    )) => return Err(AukiKnownPeerRecvError::Closed),
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        return Err(AukiKnownPeerRecvError::Lagged(skipped));
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(AukiKnownPeerRecvError::Closed);
                    }
                }
            }
        }
    }
}

/// Failure while consuming the bounded known-peer event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AukiKnownPeerRecvError {
    /// The subscriber fell behind; read a fresh snapshot.
    #[error("known-peer subscriber lagged by {0} events; read a fresh snapshot")]
    Lagged(u64),
    /// The owning peer stopped or the observation stream closed.
    #[error("known-peer event stream is closed")]
    Closed,
}

fn project(observation: &auki_p2p::AuthenticatedPeerObservation) -> AukiKnownPeer {
    AukiKnownPeer {
        peer_id: observation.peer().peer_id,
        authenticated_until: observation.peer().verified_until,
        application: observation.peer().application.clone(),
    }
}
