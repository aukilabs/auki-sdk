use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use auki_p2p::{
    AuthenticatedPeer, AuthenticatedPeerObservation, ConnectionId, NodeObservationEvent,
    NodeObservations, PEER_OBSERVATION_CHANNEL_CAPACITY, PeerDisappearanceReason, PeerId,
    SignedApplicationMetadata,
};
use auki_protocols::info::v1::AuthenticatedParticipantInfo;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const PARTICIPANT_INFO_CAPACITY: usize = 1_024;

/// One peer that is both transport-connected and mutually authenticated for
/// this exact Domain. It is observation only, never an authorization cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownPeer {
    peer_id: PeerId,
    authenticated_until: DateTime<Utc>,
    application: Option<SignedApplicationMetadata>,
    participant_info: Option<AuthenticatedParticipantInfo>,
}

impl KnownPeer {
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn authenticated_until(&self) -> DateTime<Utc> {
        self.authenticated_until
    }

    pub fn application(&self) -> Option<&SignedApplicationMetadata> {
        self.application.as_ref()
    }

    pub fn participant_info(&self) -> Option<&AuthenticatedParticipantInfo> {
        self.participant_info.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KnownPeerEvent {
    Appeared(KnownPeer),
    Updated(KnownPeer),
    Disappeared {
        peer_id: PeerId,
        reason: PeerDisappearanceReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownPeerSnapshot {
    peers: Vec<KnownPeer>,
}

impl KnownPeerSnapshot {
    pub fn peers(&self) -> &[KnownPeer] {
        &self.peers
    }
}

#[derive(Clone)]
pub struct DomainPeers {
    domain_id: Uuid,
    observations: NodeObservations,
    lifecycle: CancellationToken,
    participant_info: Arc<Mutex<ParticipantInfoCache>>,
    participant_updates: broadcast::Sender<PeerId>,
}

#[derive(Default)]
struct ParticipantInfoCache {
    entries: BTreeMap<PeerId, ObservedParticipantInfo>,
    oldest_first: VecDeque<PeerId>,
}

#[derive(Clone)]
struct ObservedParticipantInfo {
    info: AuthenticatedParticipantInfo,
    fence: ObservationFence,
}

#[derive(Clone, PartialEq, Eq)]
struct ObservationFence {
    authenticated_until: DateTime<Utc>,
    connection_ids: Vec<ConnectionId>,
}

impl DomainPeers {
    pub(super) fn new(
        domain_id: Uuid,
        observations: NodeObservations,
        lifecycle: CancellationToken,
    ) -> Self {
        let (participant_updates, _) = broadcast::channel(PEER_OBSERVATION_CHANNEL_CAPACITY);
        Self {
            domain_id,
            observations,
            lifecycle,
            participant_info: Arc::new(Mutex::new(ParticipantInfoCache::default())),
            participant_updates,
        }
    }

    pub fn snapshot(&self) -> KnownPeerSnapshot {
        if self.lifecycle.is_cancelled() {
            self.participant_info.lock().clear();
            return KnownPeerSnapshot { peers: Vec::new() };
        }
        let snapshot = self.observations.snapshot();
        let observations = snapshot
            .peers()
            .iter()
            .filter(|observation| observation.domain_id() == self.domain_id)
            .collect::<Vec<_>>();
        let participant_info = self.participant_info.lock();
        let mut peers = observations
            .into_iter()
            .map(|observation| project_peer(observation, &participant_info))
            .collect::<Vec<_>>();
        peers.sort_unstable_by_key(|peer| peer.peer_id.to_string());
        KnownPeerSnapshot { peers }
    }

    pub fn peer_count(&self) -> usize {
        self.snapshot().peers.len()
    }

    pub fn subscribe(&self) -> KnownPeerSubscription {
        KnownPeerSubscription {
            domain_id: self.domain_id,
            receiver: self.observations.subscribe(),
            lifecycle: self.lifecycle.clone(),
            observations: self.observations.clone(),
            participant_info: Arc::clone(&self.participant_info),
            participant_updates: self.participant_updates.subscribe(),
        }
    }

    pub(super) fn refresh_participant_info(
        &self,
        expected_peer: PeerId,
        authenticated_peer: &AuthenticatedPeer,
        info: AuthenticatedParticipantInfo,
    ) -> Result<(), DomainPeerInfoError> {
        if self.lifecycle.is_cancelled() {
            return Err(DomainPeerInfoError::Stopped);
        }
        if info.peer_id != expected_peer {
            return Err(DomainPeerInfoError::PeerMismatch {
                expected: Box::new(expected_peer),
                actual: Box::new(info.peer_id),
            });
        }
        let snapshot = self.observations.snapshot();
        let Some(observation) = snapshot.peers().iter().find(|observation| {
            observation.domain_id() == self.domain_id && observation.peer() == authenticated_peer
        }) else {
            // The authenticated response remains a successful info fetch even
            // if its last transport closes before this diagnostic-only cache
            // update. There is no current KnownPeer to enrich in that race.
            return if self.lifecycle.is_cancelled() {
                Err(DomainPeerInfoError::Stopped)
            } else {
                Ok(())
            };
        };
        let fence = observation_fence(observation);

        let mut participant_info = self.participant_info.lock();
        if self.lifecycle.is_cancelled() {
            return Err(DomainPeerInfoError::Stopped);
        }
        let evicted =
            participant_info.insert(expected_peer, ObservedParticipantInfo { info, fence });
        drop(participant_info);
        if let Some(evicted) = evicted {
            let _ = self.participant_updates.send(evicted);
        }
        let _ = self.participant_updates.send(expected_peer);
        Ok(())
    }

    pub(super) fn clear_participant_info(&self) {
        self.participant_info.lock().clear();
    }
}

pub struct KnownPeerSubscription {
    domain_id: Uuid,
    receiver: broadcast::Receiver<NodeObservationEvent>,
    lifecycle: CancellationToken,
    observations: NodeObservations,
    participant_info: Arc<Mutex<ParticipantInfoCache>>,
    participant_updates: broadcast::Receiver<PeerId>,
}

impl KnownPeerSubscription {
    pub async fn recv(&mut self) -> Result<KnownPeerEvent, KnownPeerRecvError> {
        loop {
            let event = tokio::select! {
                biased;
                _ = self.lifecycle.cancelled() => return Err(KnownPeerRecvError::Closed),
                event = self.receiver.recv() => SubscriptionEvent::Observation(event),
                event = self.participant_updates.recv() => SubscriptionEvent::Participant(event),
            };
            match event {
                SubscriptionEvent::Observation(Ok(event)) => {
                    if let Some(event) =
                        project_event(event, self.domain_id, &self.participant_info)
                    {
                        return Ok(event);
                    }
                }
                SubscriptionEvent::Participant(Ok(peer_id)) => {
                    let snapshot = self.observations.snapshot();
                    if let Some(observation) = snapshot.peers().iter().find(|observation| {
                        observation.domain_id() == self.domain_id
                            && observation.peer().peer_id == peer_id
                    }) {
                        return Ok(KnownPeerEvent::Updated(project_peer(
                            observation,
                            &self.participant_info.lock(),
                        )));
                    }
                }
                SubscriptionEvent::Observation(Err(broadcast::error::RecvError::Lagged(
                    skipped,
                )))
                | SubscriptionEvent::Participant(Err(broadcast::error::RecvError::Lagged(
                    skipped,
                ))) => {
                    return Err(KnownPeerRecvError::Lagged(skipped));
                }
                SubscriptionEvent::Observation(Err(broadcast::error::RecvError::Closed))
                | SubscriptionEvent::Participant(Err(broadcast::error::RecvError::Closed)) => {
                    return Err(KnownPeerRecvError::Closed);
                }
            }
        }
    }
}

enum SubscriptionEvent {
    Observation(Result<NodeObservationEvent, broadcast::error::RecvError>),
    Participant(Result<PeerId, broadcast::error::RecvError>),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DomainPeerInfoError {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("participant info Peer ID {actual} does not match authenticated peer {expected}")]
    PeerMismatch {
        expected: Box<PeerId>,
        actual: Box<PeerId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KnownPeerRecvError {
    #[error("known-peer subscriber lagged by {0} events; read a fresh snapshot")]
    Lagged(u64),
    #[error("known-peer event stream is closed")]
    Closed,
}

fn project_event(
    event: NodeObservationEvent,
    domain_id: Uuid,
    participant_info: &Arc<Mutex<ParticipantInfoCache>>,
) -> Option<KnownPeerEvent> {
    match event {
        NodeObservationEvent::Appeared(observation) if observation.domain_id() == domain_id => {
            Some(KnownPeerEvent::Appeared(project_peer(
                &observation,
                &participant_info.lock(),
            )))
        }
        NodeObservationEvent::Updated(observation) if observation.domain_id() == domain_id => Some(
            KnownPeerEvent::Updated(project_peer(&observation, &participant_info.lock())),
        ),
        NodeObservationEvent::Disappeared {
            peer_id,
            domain_id: observed_domain,
            reason,
        } if observed_domain == domain_id => Some(KnownPeerEvent::Disappeared { peer_id, reason }),
        _ => None,
    }
}

fn project_peer(
    observation: &AuthenticatedPeerObservation,
    participant_info: &ParticipantInfoCache,
) -> KnownPeer {
    KnownPeer {
        peer_id: observation.peer().peer_id,
        authenticated_until: observation.peer().verified_until,
        application: observation.peer().application.clone(),
        participant_info: participant_metadata(observation, participant_info),
    }
}

fn participant_metadata(
    observation: &AuthenticatedPeerObservation,
    participant_info: &ParticipantInfoCache,
) -> Option<AuthenticatedParticipantInfo> {
    participant_info
        .entries
        .get(&observation.peer().peer_id)
        .filter(|observed| observed.fence.matches(observation))
        .map(|observed| observed.info.clone())
}

impl ParticipantInfoCache {
    fn insert(&mut self, peer_id: PeerId, info: ObservedParticipantInfo) -> Option<PeerId> {
        self.oldest_first.retain(|current| *current != peer_id);
        let evicted = if self.entries.len() >= PARTICIPANT_INFO_CAPACITY
            && !self.entries.contains_key(&peer_id)
        {
            self.oldest_first.pop_front().inspect(|oldest| {
                self.entries.remove(oldest);
            })
        } else {
            None
        };
        self.entries.insert(peer_id, info);
        self.oldest_first.push_back(peer_id);
        evicted
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.oldest_first.clear();
    }
}

fn observation_fence(observation: &AuthenticatedPeerObservation) -> ObservationFence {
    ObservationFence {
        authenticated_until: observation.peer().verified_until,
        connection_ids: observation.connection_ids().to_vec(),
    }
}

impl ObservationFence {
    fn matches(&self, observation: &AuthenticatedPeerObservation) -> bool {
        self.authenticated_until == observation.peer().verified_until
            && self
                .connection_ids
                .iter()
                .any(|connection_id| observation.connection_ids().contains(connection_id))
    }
}

#[cfg(test)]
mod tests {
    use auki_p2p::Identity;

    use super::*;

    fn cached(seed: usize) -> (PeerId, ObservedParticipantInfo) {
        let mut identity_seed = [0_u8; 32];
        identity_seed[..size_of::<usize>()].copy_from_slice(&seed.to_le_bytes());
        let peer_id = Identity::from_ed25519_seed(&identity_seed).peer_id();
        (
            peer_id,
            ObservedParticipantInfo {
                info: AuthenticatedParticipantInfo {
                    app: "test".into(),
                    app_version: "1".into(),
                    name: seed.to_string(),
                    session_id: "session".into(),
                    session_clock_id: "clock".into(),
                    session_clock_hash: "hash".into(),
                    session_now_ns: seed as u64,
                    peer_id,
                    app_instance: "instance".into(),
                },
                fence: ObservationFence {
                    authenticated_until: Utc::now() + chrono::Duration::minutes(1),
                    connection_ids: vec![ConnectionId::new_unchecked(seed)],
                },
            },
        )
    }

    #[test]
    fn participant_cache_is_bounded_and_refreshes_fifo_order_without_stale_rejection() {
        let mut cache = ParticipantInfoCache::default();
        let mut peers = Vec::with_capacity(PARTICIPANT_INFO_CAPACITY);
        for seed in 0..PARTICIPANT_INFO_CAPACITY {
            let (peer_id, info) = cached(seed);
            assert_eq!(cache.insert(peer_id, info), None);
            peers.push(peer_id);
        }

        let (refreshed_peer, refreshed) = cached(0);
        assert_eq!(cache.insert(refreshed_peer, refreshed), None);
        let (new_peer, new_info) = cached(PARTICIPANT_INFO_CAPACITY);
        assert_eq!(cache.insert(new_peer, new_info), Some(peers[1]));
        assert_eq!(cache.entries.len(), PARTICIPANT_INFO_CAPACITY);
        assert!(cache.entries.contains_key(&peers[0]));
        assert!(!cache.entries.contains_key(&peers[1]));
        assert!(cache.entries.contains_key(&new_peer));

        let (another_peer, another_info) = cached(PARTICIPANT_INFO_CAPACITY + 1);
        assert_eq!(cache.insert(another_peer, another_info), Some(peers[2]));
        assert_eq!(cache.entries.len(), PARTICIPANT_INFO_CAPACITY);
    }
}
