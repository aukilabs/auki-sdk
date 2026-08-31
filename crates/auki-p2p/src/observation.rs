use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use libp2p::{swarm::ConnectionId, PeerId};
use parking_lot::Mutex;
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use crate::AuthenticatedPeer;

/// Maximum number of peer-observation events retained for each subscriber.
///
/// A lagged subscriber must discard event-derived state and read a fresh
/// [`NodeObservations::snapshot`]. Events are deliberately not authority.
pub const PEER_OBSERVATION_CHANNEL_CAPACITY: usize = 256;

/// Current lifecycle state of the node that produced an observation snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeObservationStatus {
    Running,
    Stopped,
    Failed(NodeFailure),
}

/// Bounded reasons for an unexpected node-runtime termination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeFailure {
    SwarmEnded,
    ExpiryDriverEnded,
    ListenerClosed,
    Panicked,
    TaskCancelled,
}

/// Why one previously observable authenticated peer disappeared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerDisappearanceReason {
    CredentialExpired,
    FinalConnectionClosed,
    NodeStopped,
    NodeFailed,
}

/// One exact-Domain authenticated peer while at least one transport
/// connection to its Noise Peer ID remains live.
///
/// `connection_ids` is the authoritative set of currently live libp2p
/// connections for this Peer ID at the time of the snapshot. The upstream
/// generic stream API identifies inbound streams by Peer ID rather than one
/// connection, so this record intentionally makes no false claim that a
/// particular application stream used one selected connection. D07 only
/// requires a live connection to the same authenticated Noise peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedPeerObservation {
    domain_id: Uuid,
    peer: AuthenticatedPeer,
    connection_ids: Vec<ConnectionId>,
}

impl AuthenticatedPeerObservation {
    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    pub fn peer(&self) -> &AuthenticatedPeer {
        &self.peer
    }

    pub fn connection_ids(&self) -> &[ConnectionId] {
        &self.connection_ids
    }
}

/// A bounded notification about the authoritative local observation state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeObservationEvent {
    Appeared(AuthenticatedPeerObservation),
    Updated(AuthenticatedPeerObservation),
    Disappeared {
        peer_id: PeerId,
        domain_id: Uuid,
        reason: PeerDisappearanceReason,
    },
    StatusChanged(NodeObservationStatus),
}

/// Authoritative recovery point after startup or subscriber lag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeObservationSnapshot {
    status: NodeObservationStatus,
    peers: Vec<AuthenticatedPeerObservation>,
}

impl NodeObservationSnapshot {
    pub fn status(&self) -> NodeObservationStatus {
        self.status
    }

    pub fn peers(&self) -> &[AuthenticatedPeerObservation] {
        &self.peers
    }
}

/// Cloneable read handle for local authenticated-peer observations.
///
/// This is neither a membership list nor an authorization cache. Every new
/// application stream still performs mutual authentication independently.
#[derive(Clone)]
pub struct NodeObservations {
    inner: Arc<ObservationInner>,
}

struct ObservationInner {
    transition_order: Mutex<()>,
    state: Mutex<ObservationState>,
    events: broadcast::Sender<NodeObservationEvent>,
    deadline_revision: watch::Sender<u64>,
}

struct ObservationState {
    status: NodeObservationStatus,
    connections: HashMap<PeerId, BTreeSet<ConnectionId>>,
    authentications: HashMap<(PeerId, Uuid), AuthenticatedPeer>,
}

impl NodeObservations {
    pub(crate) fn new() -> Self {
        Self::with_capacity(PEER_OBSERVATION_CHANNEL_CAPACITY)
    }

    fn with_capacity(capacity: usize) -> Self {
        let (events, _) = broadcast::channel(capacity);
        let (deadline_revision, _) = watch::channel(0);
        Self {
            inner: Arc::new(ObservationInner {
                transition_order: Mutex::new(()),
                state: Mutex::new(ObservationState {
                    status: NodeObservationStatus::Running,
                    connections: HashMap::new(),
                    authentications: HashMap::new(),
                }),
                events,
                deadline_revision,
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NodeObservationEvent> {
        self.inner.events.subscribe()
    }

    pub fn snapshot(&self) -> NodeObservationSnapshot {
        self.expire_at(Utc::now());
        let state = self.inner.state.lock();
        NodeObservationSnapshot {
            status: state.status,
            peers: current_peers(&state),
        }
    }

    /// Run literal credential expiry until the node reaches a terminal state.
    ///
    /// The node supervisor owns and monitors this future; it must not be
    /// detached because an unexpected return would otherwise silently stop
    /// expiry notifications while the swarm remained live.
    pub(crate) async fn drive_expiry(&self) {
        let mut changed = self.inner.deadline_revision.subscribe();
        loop {
            let (status, deadline) = {
                let state = self.inner.state.lock();
                (state.status, next_expiration(&state))
            };
            if status != NodeObservationStatus::Running {
                return;
            }

            match deadline {
                Some(deadline) => {
                    let delay = deadline
                        .signed_duration_since(Utc::now())
                        .to_std()
                        .unwrap_or_default();
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {
                            expire_inner(&self.inner, Utc::now());
                        }
                        result = changed.changed() => {
                            if result.is_err() {
                                return;
                            }
                        }
                    }
                }
                None => {
                    if changed.changed().await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    pub(crate) fn connection_established(&self, peer_id: PeerId, connection_id: ConnectionId) {
        let now = Utc::now();
        let _transition = self.inner.transition_order.lock();
        let events = {
            let mut state = self.inner.state.lock();
            if state.status != NodeObservationStatus::Running {
                return;
            }
            let mut events = prune_expired(&mut state, now);
            let was_connected = state
                .connections
                .get(&peer_id)
                .is_some_and(|connections| !connections.is_empty());
            let inserted = state
                .connections
                .entry(peer_id)
                .or_default()
                .insert(connection_id);
            if inserted {
                events.extend(observations_for_peer(&state, peer_id).into_iter().map(
                    |observation| {
                        if was_connected {
                            NodeObservationEvent::Updated(observation)
                        } else {
                            NodeObservationEvent::Appeared(observation)
                        }
                    },
                ));
            }
            events
        };
        send_events(&self.inner.events, events);
    }

    pub(crate) fn connection_closed(&self, peer_id: PeerId, connection_id: ConnectionId) {
        let now = Utc::now();
        let _transition = self.inner.transition_order.lock();
        let events = {
            let mut state = self.inner.state.lock();
            if state.status != NodeObservationStatus::Running {
                return;
            }
            let mut events = prune_expired(&mut state, now);
            let removed = state
                .connections
                .get_mut(&peer_id)
                .is_some_and(|connections| connections.remove(&connection_id));
            if removed {
                let still_connected = state
                    .connections
                    .get(&peer_id)
                    .is_some_and(|connections| !connections.is_empty());
                if still_connected {
                    events.extend(
                        observations_for_peer(&state, peer_id)
                            .into_iter()
                            .map(NodeObservationEvent::Updated),
                    );
                } else {
                    state.connections.remove(&peer_id);
                    events.extend(remove_authentications_for_peer(
                        &mut state,
                        peer_id,
                        PeerDisappearanceReason::FinalConnectionClosed,
                    ));
                }
            }
            events
        };
        send_events(&self.inner.events, events);
        self.bump_deadline_revision();
    }

    pub(crate) fn authenticated(&self, domain_id: Uuid, peer: AuthenticatedPeer) {
        let now = Utc::now();
        if peer.verified_until <= now {
            return;
        }
        let peer_id = peer.peer_id;
        let _transition = self.inner.transition_order.lock();
        let events = {
            let mut state = self.inner.state.lock();
            if state.status != NodeObservationStatus::Running {
                return;
            }
            let mut events = prune_expired(&mut state, now);
            let connected = state
                .connections
                .get(&peer_id)
                .is_some_and(|connections| !connections.is_empty());
            if !connected {
                // ConnectionEstablished is observed before libp2p can deliver
                // an application stream. No live connection here therefore
                // means this authentication completed concurrently with or
                // after final closure; retaining it could make a later
                // Noise-only reconnect appear authenticated.
                events
            } else {
                let was_visible = state.authentications.contains_key(&(peer_id, domain_id));
                state.authentications.insert((peer_id, domain_id), peer);
                events.extend(observation(&state, peer_id, domain_id).map(|observation| {
                    if was_visible {
                        NodeObservationEvent::Updated(observation)
                    } else {
                        NodeObservationEvent::Appeared(observation)
                    }
                }));
                events
            }
        };
        send_events(&self.inner.events, events);
        self.bump_deadline_revision();
    }

    pub(crate) fn stopped(&self) {
        self.terminate(NodeObservationStatus::Stopped);
    }

    pub(crate) fn failed(&self, reason: NodeFailure) {
        self.terminate(NodeObservationStatus::Failed(reason));
    }

    fn terminate(&self, status: NodeObservationStatus) {
        let _transition = self.inner.transition_order.lock();
        let events = {
            let mut state = self.inner.state.lock();
            if state.status != NodeObservationStatus::Running {
                return;
            }
            let disappearance = match status {
                NodeObservationStatus::Running => return,
                NodeObservationStatus::Stopped => PeerDisappearanceReason::NodeStopped,
                NodeObservationStatus::Failed(_) => PeerDisappearanceReason::NodeFailed,
            };
            let mut events = state
                .authentications
                .keys()
                .filter(|(peer_id, _)| {
                    state
                        .connections
                        .get(peer_id)
                        .is_some_and(|connections| !connections.is_empty())
                })
                .map(|(peer_id, domain_id)| NodeObservationEvent::Disappeared {
                    peer_id: *peer_id,
                    domain_id: *domain_id,
                    reason: disappearance,
                })
                .collect::<Vec<_>>();
            state.connections.clear();
            state.authentications.clear();
            state.status = status;
            events.push(NodeObservationEvent::StatusChanged(status));
            events
        };
        send_events(&self.inner.events, events);
        self.bump_deadline_revision();
    }

    fn expire_at(&self, now: DateTime<Utc>) {
        expire_inner(&self.inner, now);
    }

    fn bump_deadline_revision(&self) {
        self.inner
            .deadline_revision
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
}

fn expire_inner(inner: &ObservationInner, now: DateTime<Utc>) {
    let _transition = inner.transition_order.lock();
    let events = {
        let mut state = inner.state.lock();
        prune_expired(&mut state, now)
    };
    send_events(&inner.events, events);
}

/// Publish while the corresponding transition is still serialized.
///
/// The state mutex is released before `broadcast::Sender::send` wakes any
/// receiver. The separate transition mutex keeps a concurrent close/expiry
/// from publishing `Disappeared` before the earlier `Appeared` batch.
fn send_events(
    sender: &broadcast::Sender<NodeObservationEvent>,
    events: Vec<NodeObservationEvent>,
) {
    for event in events {
        let _ = sender.send(event);
    }
}

fn prune_expired(state: &mut ObservationState, now: DateTime<Utc>) -> Vec<NodeObservationEvent> {
    let expired = state
        .authentications
        .iter()
        .filter_map(|((peer_id, domain_id), peer)| {
            (peer.verified_until <= now).then_some((*peer_id, *domain_id))
        })
        .collect::<Vec<_>>();
    let mut events = Vec::with_capacity(expired.len());
    for (peer_id, domain_id) in expired {
        state.authentications.remove(&(peer_id, domain_id));
        if state
            .connections
            .get(&peer_id)
            .is_some_and(|connections| !connections.is_empty())
        {
            events.push(NodeObservationEvent::Disappeared {
                peer_id,
                domain_id,
                reason: PeerDisappearanceReason::CredentialExpired,
            });
        }
    }
    events
}

fn remove_authentications_for_peer(
    state: &mut ObservationState,
    peer_id: PeerId,
    reason: PeerDisappearanceReason,
) -> Vec<NodeObservationEvent> {
    let domains = state
        .authentications
        .keys()
        .filter_map(|(candidate, domain_id)| (*candidate == peer_id).then_some(*domain_id))
        .collect::<Vec<_>>();
    for domain_id in &domains {
        state.authentications.remove(&(peer_id, *domain_id));
    }
    domains
        .into_iter()
        .map(|domain_id| NodeObservationEvent::Disappeared {
            peer_id,
            domain_id,
            reason,
        })
        .collect()
}

fn observations_for_peer(
    state: &ObservationState,
    peer_id: PeerId,
) -> Vec<AuthenticatedPeerObservation> {
    let mut observations = state
        .authentications
        .keys()
        .filter_map(|(candidate, domain_id)| {
            if *candidate == peer_id {
                observation(state, peer_id, *domain_id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    observations.sort_by_key(AuthenticatedPeerObservation::domain_id);
    observations
}

fn current_peers(state: &ObservationState) -> Vec<AuthenticatedPeerObservation> {
    let mut peers = state
        .authentications
        .keys()
        .filter_map(|(peer_id, domain_id)| observation(state, *peer_id, *domain_id))
        .collect::<Vec<_>>();
    peers.sort_by(|left, right| {
        left.peer
            .peer_id
            .to_bytes()
            .cmp(&right.peer.peer_id.to_bytes())
            .then_with(|| left.domain_id.cmp(&right.domain_id))
    });
    peers
}

fn observation(
    state: &ObservationState,
    peer_id: PeerId,
    domain_id: Uuid,
) -> Option<AuthenticatedPeerObservation> {
    let peer = state.authentications.get(&(peer_id, domain_id))?.clone();
    let connection_ids = state
        .connections
        .get(&peer_id)?
        .iter()
        .copied()
        .collect::<Vec<_>>();
    if connection_ids.is_empty() {
        return None;
    }
    Some(AuthenticatedPeerObservation {
        domain_id,
        peer,
        connection_ids,
    })
}

fn next_expiration(state: &ObservationState) -> Option<DateTime<Utc>> {
    state
        .authentications
        .values()
        .map(|peer| peer.verified_until)
        .min()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::broadcast::error::TryRecvError;

    use super::*;

    fn peer(seed: u8, verified_until: DateTime<Utc>) -> AuthenticatedPeer {
        let identity = crate::Identity::from_ed25519_seed(&[seed; 32]);
        AuthenticatedPeer {
            peer_id: identity.peer_id(),
            subject: Uuid::from_u128(u128::from(seed) + 1),
            peer_type: Some("native".into()),
            domain_ids: vec![Uuid::from_u128(100)],
            scopes: vec!["diagnostic:only".into()],
            application: None,
            verified_until,
        }
    }

    fn connection(id: usize) -> ConnectionId {
        ConnectionId::new_unchecked(id)
    }

    #[test]
    fn authenticated_peer_state_follows_auth_connections_expiry_and_reconnect() {
        let observations = NodeObservations::with_capacity(32);
        let domain_id = Uuid::from_u128(100);
        let now = Utc::now();
        let first_peer = peer(1, now + chrono::Duration::seconds(30));
        let peer_id = first_peer.peer_id;
        let first_connection = connection(1);
        let second_connection = connection(2);

        // An auth completion observed after its connection has already closed
        // is fenced and cannot authorize a later Noise-only reconnect.
        let stale_peer = peer(9, now + chrono::Duration::seconds(30));
        let stale_peer_id = stale_peer.peer_id;
        observations.connection_established(stale_peer_id, connection(89));
        observations.connection_closed(stale_peer_id, connection(89));
        observations.authenticated(domain_id, stale_peer);
        observations.connection_established(stale_peer_id, connection(90));
        assert!(observations.snapshot().peers().is_empty());
        observations.connection_closed(stale_peer_id, connection(90));

        let mut events = observations.subscribe();

        // Noise-only transport is not an authenticated peer.
        observations.connection_established(peer_id, first_connection);
        assert!(observations.snapshot().peers().is_empty());
        assert_eq!(events.try_recv(), Err(TryRecvError::Empty));

        observations.authenticated(domain_id, first_peer.clone());
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Appeared(_))
        ));
        let snapshot = observations.snapshot();
        assert_eq!(snapshot.peers().len(), 1);
        assert_eq!(snapshot.peers()[0].connection_ids(), [first_connection]);

        // A repeated authentication refreshes the exact record.
        let refreshed = peer(1, now + chrono::Duration::seconds(60));
        observations.authenticated(domain_id, refreshed.clone());
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Updated(_))
        ));
        assert_eq!(
            observations.snapshot().peers()[0].peer().verified_until,
            refreshed.verified_until
        );

        observations.connection_established(peer_id, second_connection);
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Updated(_))
        ));
        assert_eq!(
            observations.snapshot().peers()[0].connection_ids(),
            [first_connection, second_connection]
        );
        observations.connection_closed(peer_id, first_connection);
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Updated(_))
        ));
        assert_eq!(
            observations.snapshot().peers()[0].connection_ids(),
            [second_connection]
        );

        // Literal expiry removes authority even while transport remains live.
        observations.expire_at(refreshed.verified_until);
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Disappeared {
                reason: PeerDisappearanceReason::CredentialExpired,
                ..
            })
        ));
        assert!(observations.snapshot().peers().is_empty());

        // A fresh authentication on the still-live connection reappears.
        let reauthenticated = peer(1, now + chrono::Duration::seconds(90));
        observations.authenticated(domain_id, reauthenticated);
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Appeared(_))
        ));
        assert_eq!(observations.snapshot().peers().len(), 1);
        observations.connection_closed(peer_id, second_connection);
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Disappeared {
                reason: PeerDisappearanceReason::FinalConnectionClosed,
                ..
            })
        ));
        assert!(observations.snapshot().peers().is_empty());

        // Reconnecting transport alone cannot reuse the previous authority.
        observations.connection_established(peer_id, connection(3));
        assert!(observations.snapshot().peers().is_empty());
        assert_eq!(events.try_recv(), Err(TryRecvError::Empty));
        observations.authenticated(domain_id, peer(1, now + chrono::Duration::seconds(120)));
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Appeared(_))
        ));
        observations.connection_closed(peer_id, connection(3));
    }

    #[test]
    fn event_sequence_distinguishes_parallel_connections_and_terminal_state() {
        let observations = NodeObservations::with_capacity(32);
        let mut events = observations.subscribe();
        let domain_id = Uuid::from_u128(100);
        let peer = peer(2, Utc::now() + chrono::Duration::minutes(1));
        let peer_id = peer.peer_id;

        observations.connection_established(peer_id, connection(10));
        observations.authenticated(domain_id, peer);
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Appeared(_))
        ));

        observations.connection_established(peer_id, connection(11));
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Updated(_))
        ));
        observations.connection_closed(peer_id, connection(10));
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Updated(_))
        ));
        observations.failed(NodeFailure::SwarmEnded);
        assert!(matches!(
            events.try_recv(),
            Ok(NodeObservationEvent::Disappeared {
                reason: PeerDisappearanceReason::NodeFailed,
                ..
            })
        ));
        assert_eq!(
            events.try_recv(),
            Ok(NodeObservationEvent::StatusChanged(
                NodeObservationStatus::Failed(NodeFailure::SwarmEnded)
            ))
        );
        assert_eq!(
            observations.snapshot().status(),
            NodeObservationStatus::Failed(NodeFailure::SwarmEnded)
        );
    }

    #[test]
    fn lag_is_explicit_and_snapshot_recovers_authoritative_state() {
        let observations = NodeObservations::with_capacity(2);
        let mut events = observations.subscribe();
        let domain_id = Uuid::from_u128(100);
        let peer = peer(3, Utc::now() + chrono::Duration::minutes(1));
        let peer_id = peer.peer_id;

        observations.connection_established(peer_id, connection(20));
        observations.authenticated(domain_id, peer);
        observations.connection_established(peer_id, connection(21));
        observations.connection_closed(peer_id, connection(20));

        assert!(matches!(events.try_recv(), Err(TryRecvError::Lagged(_))));
        let snapshot = observations.snapshot();
        assert_eq!(snapshot.peers().len(), 1);
        assert_eq!(snapshot.peers()[0].connection_ids(), [connection(21)]);
    }

    #[tokio::test]
    async fn expiry_driver_emits_disappearance_without_connection_activity() {
        let observations = NodeObservations::with_capacity(8);
        let expiry_observations = observations.clone();
        let expiry_driver = tokio::spawn(async move { expiry_observations.drive_expiry().await });
        let mut events = observations.subscribe();
        let domain_id = Uuid::from_u128(100);
        let peer = peer(4, Utc::now() + chrono::Duration::milliseconds(20));
        let peer_id = peer.peer_id;
        observations.connection_established(peer_id, connection(30));
        observations.authenticated(domain_id, peer);
        assert!(matches!(
            events.recv().await,
            Ok(NodeObservationEvent::Appeared(_))
        ));

        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("expiry event must be timely")
            .expect("observation channel stays open");
        assert_eq!(
            event,
            NodeObservationEvent::Disappeared {
                peer_id,
                domain_id,
                reason: PeerDisappearanceReason::CredentialExpired,
            }
        );
        observations.stopped();
        expiry_driver.await.unwrap();
    }
}
