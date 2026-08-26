use auki_p2p::{
    AuthenticatedPeerObservation, NodeObservationEvent, NodeObservations, PeerDisappearanceReason,
    PeerId, SignedApplicationMetadata,
};
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// One peer that is both transport-connected and mutually authenticated for
/// this exact Domain. It is observation only, never an authorization cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KnownPeer {
    peer_id: PeerId,
    authenticated_until: DateTime<Utc>,
    application: Option<SignedApplicationMetadata>,
}

impl KnownPeer {
    pub(crate) fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub(crate) fn authenticated_until(&self) -> DateTime<Utc> {
        self.authenticated_until
    }

    pub(crate) fn application(&self) -> Option<&SignedApplicationMetadata> {
        self.application.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KnownPeerEvent {
    Appeared(KnownPeer),
    Updated(KnownPeer),
    Disappeared {
        peer_id: PeerId,
        reason: PeerDisappearanceReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KnownPeerSnapshot {
    peers: Vec<KnownPeer>,
}

impl KnownPeerSnapshot {
    pub(crate) fn peers(&self) -> &[KnownPeer] {
        &self.peers
    }
}

#[derive(Clone)]
pub(crate) struct DomainPeers {
    domain_id: Uuid,
    observations: NodeObservations,
    lifecycle: CancellationToken,
}

impl DomainPeers {
    pub(super) fn new(
        domain_id: Uuid,
        observations: NodeObservations,
        lifecycle: CancellationToken,
    ) -> Self {
        Self {
            domain_id,
            observations,
            lifecycle,
        }
    }

    pub(crate) fn snapshot(&self) -> KnownPeerSnapshot {
        if self.lifecycle.is_cancelled() {
            return KnownPeerSnapshot { peers: Vec::new() };
        }
        let mut peers = self
            .observations
            .snapshot()
            .peers()
            .iter()
            .filter(|observation| observation.domain_id() == self.domain_id)
            .map(project_peer)
            .collect::<Vec<_>>();
        peers.sort_unstable_by_key(|peer| peer.peer_id.to_string());
        KnownPeerSnapshot { peers }
    }

    pub(crate) fn peer_count(&self) -> usize {
        self.snapshot().peers.len()
    }

    pub(crate) fn subscribe(&self) -> KnownPeerSubscription {
        KnownPeerSubscription {
            domain_id: self.domain_id,
            receiver: self.observations.subscribe(),
            lifecycle: self.lifecycle.clone(),
        }
    }
}

pub(crate) struct KnownPeerSubscription {
    domain_id: Uuid,
    receiver: broadcast::Receiver<NodeObservationEvent>,
    lifecycle: CancellationToken,
}

impl KnownPeerSubscription {
    pub(crate) async fn recv(&mut self) -> Result<KnownPeerEvent, KnownPeerRecvError> {
        loop {
            let event = tokio::select! {
                biased;
                _ = self.lifecycle.cancelled() => return Err(KnownPeerRecvError::Closed),
                event = self.receiver.recv() => event,
            };
            match event {
                Ok(event) => {
                    if let Some(event) = project_event(event, self.domain_id) {
                        return Ok(event);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    return Err(KnownPeerRecvError::Lagged(skipped));
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(KnownPeerRecvError::Closed);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum KnownPeerRecvError {
    #[error("known-peer subscriber lagged by {0} events; read a fresh snapshot")]
    Lagged(u64),
    #[error("known-peer event stream is closed")]
    Closed,
}

fn project_event(event: NodeObservationEvent, domain_id: Uuid) -> Option<KnownPeerEvent> {
    match event {
        NodeObservationEvent::Appeared(observation) if observation.domain_id() == domain_id => {
            Some(KnownPeerEvent::Appeared(project_peer(&observation)))
        }
        NodeObservationEvent::Updated(observation) if observation.domain_id() == domain_id => {
            Some(KnownPeerEvent::Updated(project_peer(&observation)))
        }
        NodeObservationEvent::Disappeared {
            peer_id,
            domain_id: observed_domain,
            reason,
        } if observed_domain == domain_id => Some(KnownPeerEvent::Disappeared { peer_id, reason }),
        _ => None,
    }
}

fn project_peer(observation: &AuthenticatedPeerObservation) -> KnownPeer {
    KnownPeer {
        peer_id: observation.peer().peer_id,
        authenticated_until: observation.peer().verified_until,
        application: observation.peer().application.clone(),
    }
}
