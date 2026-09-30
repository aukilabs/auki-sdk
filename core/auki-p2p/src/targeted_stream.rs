//! Outbound application streams pinned to one established libp2p connection.
//!
//! The stock stream control selects a connection by peer. Relay routes need a
//! stronger invariant: after dialing a particular circuit, the application
//! stream must use the resulting [`ConnectionId`] even when another connection
//! to the same peer is healthy. This behaviour never dials and never falls back
//! to a different connection. SDK transports opt into retiring a direct relay
//! connection after sustained source-admission negotiation timeouts; reservation
//! lifecycle events then drive recovery.

use std::{
    collections::{HashMap, VecDeque},
    pin::Pin as StdPin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};

use async_channel::{Receiver, Sender};
use futures::{channel::oneshot, Stream as FuturesStream};
use libp2p::{
    core::{
        transport::PortUse,
        upgrade::{DeniedUpgrade, ReadyUpgrade},
        Endpoint,
    },
    swarm::{
        handler::{ConnectionEvent, DialUpgradeError, FullyNegotiatedOutbound},
        CloseConnection, ConnectionClosed, ConnectionDenied, ConnectionHandler,
        ConnectionHandlerEvent, ConnectionId, FromSwarm, NetworkBehaviour, NotifyHandler,
        StreamUpgradeError, SubstreamProtocol, THandler, THandlerInEvent, THandlerOutEvent,
        ToSwarm,
    },
    Multiaddr, PeerId, Stream, StreamProtocol,
};

use web_time::Instant;

const COMMAND_CAPACITY: usize = 64;
const RELAY_TIMEOUT_THRESHOLD: u8 = 3;
const RELAY_TIMEOUT_WINDOW: Duration = Duration::from_secs(20);
const RELAY_TIMEOUT_IDLE_RESET: Duration = Duration::from_secs(60);

#[derive(Default)]
struct RelayProgress {
    last_success: Option<Instant>,
    first_timeout: Option<Instant>,
    last_timeout: Option<Instant>,
    timeouts: u8,
    closing: bool,
}

impl RelayProgress {
    fn succeeded(&mut self, now: Instant) {
        self.last_success = Some(now);
        self.first_timeout = None;
        self.last_timeout = None;
        self.timeouts = 0;
    }

    fn timed_out(&mut self, started: Instant, now: Instant) -> bool {
        // Older pending requests cannot contradict more recent progress.
        if self.closing || self.last_success.is_some_and(|success| started <= success) {
            return false;
        }
        if self
            .last_timeout
            .is_some_and(|last| now.duration_since(last) > RELAY_TIMEOUT_IDLE_RESET)
        {
            self.first_timeout = None;
            self.timeouts = 0;
        }
        self.last_timeout = Some(now);
        self.timeouts = self.timeouts.saturating_add(1);
        let first = *self.first_timeout.get_or_insert(now);
        // Concurrent callers must not turn one stall into an immediate teardown.
        if self.timeouts >= RELAY_TIMEOUT_THRESHOLD
            && now.duration_since(first) >= RELAY_TIMEOUT_WINDOW
        {
            self.closing = true;
            return true;
        }
        false
    }
}

/// Failure to open an application stream on the caller-selected connection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetedStreamError {
    /// The selected connection was already closed or was never established.
    #[error("selected connection {connection_id} to peer {peer_id} is not established")]
    SelectedConnectionClosed {
        peer_id: String,
        connection_id: ConnectionId,
    },
    /// The connection exists, but belongs to a different authenticated peer.
    #[error(
        "selected connection {connection_id} belongs to peer {actual_peer_id}, not {expected_peer_id}"
    )]
    SelectedConnectionPeerMismatch {
        expected_peer_id: String,
        actual_peer_id: String,
        connection_id: ConnectionId,
    },
    /// A handler response arrived from somewhere other than the selected route.
    #[error(
        "targeted stream response for peer {expected_peer_id} connection {expected_connection_id} arrived from peer {actual_peer_id} connection {actual_connection_id}"
    )]
    HandlerRouteMismatch {
        expected_peer_id: String,
        expected_connection_id: ConnectionId,
        actual_peer_id: String,
        actual_connection_id: ConnectionId,
    },
    /// Protocol negotiation on the selected connection timed out.
    #[error(
        "opening protocol {protocol} on selected peer {peer_id} connection {connection_id} timed out"
    )]
    NegotiationTimeout {
        peer_id: String,
        connection_id: ConnectionId,
        protocol: String,
    },
    /// The peer on the selected connection does not support the protocol.
    #[error(
        "peer {peer_id} on selected connection {connection_id} does not support protocol {protocol}"
    )]
    UnsupportedProtocol {
        peer_id: String,
        connection_id: ConnectionId,
        protocol: String,
    },
    /// An I/O failure occurred while opening the substream.
    #[error(
        "I/O failure opening protocol {protocol} on selected peer {peer_id} connection {connection_id}: {reason}"
    )]
    Io {
        peer_id: String,
        connection_id: ConnectionId,
        protocol: String,
        reason: String,
    },
    /// The swarm no longer owns the targeted-stream behaviour.
    #[error("targeted-stream behaviour has stopped")]
    BehaviourStopped,
    /// The process exhausted the monotonic request-ID space.
    #[error("targeted-stream request IDs are exhausted")]
    RequestIdsExhausted,
}

/// Control plane for exact-connection stream opens.
///
/// Clones share one monotonic request-ID sequence, so simultaneous callers are
/// correlated independently even if negotiations complete out of order.
#[derive(Clone)]
pub struct TargetedStreamControl {
    commands: Sender<OpenCommand>,
    next_request_id: Arc<AtomicU64>,
}

impl TargetedStreamControl {
    /// Open `protocol` only on `connection_id` to `peer_id`.
    ///
    /// This method never dials and never selects another connection. If the
    /// selected connection closes before negotiation completes, the returned
    /// future fails with [`TargetedStreamError::SelectedConnectionClosed`].
    pub async fn open_stream(
        &self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        protocol: StreamProtocol,
    ) -> Result<Stream, TargetedStreamError> {
        let request_id = self.next_request_id()?;
        let (response, receiver) = oneshot::channel();
        self.commands
            .send(OpenCommand {
                request_id,
                peer_id,
                connection_id,
                protocol,
                response,
            })
            .await
            .map_err(|_| TargetedStreamError::BehaviourStopped)?;
        receiver
            .await
            .map_err(|_| TargetedStreamError::BehaviourStopped)?
    }

    fn next_request_id(&self) -> Result<RequestId, TargetedStreamError> {
        self.next_request_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map(RequestId)
            .map_err(|_| TargetedStreamError::RequestIdsExhausted)
    }
}

/// A behaviour that opens outbound substreams on an exact connection handler.
pub struct TargetedStreamBehaviour {
    command_sender: Sender<OpenCommand>,
    commands: StdPin<Box<Receiver<OpenCommand>>>,
    next_request_id: Arc<AtomicU64>,
    connections: HashMap<ConnectionId, PeerId>,
    pending: HashMap<RequestId, PendingOpen>,
    relay_recovery: bool,
    relay_progress: HashMap<ConnectionId, RelayProgress>,
    pending_closes: VecDeque<(PeerId, ConnectionId)>,
}

impl TargetedStreamBehaviour {
    pub fn new() -> Self {
        let (command_sender, commands) = async_channel::bounded(COMMAND_CAPACITY);
        Self {
            command_sender,
            commands: Box::pin(commands),
            next_request_id: Arc::new(AtomicU64::new(1)),
            connections: HashMap::new(),
            pending: HashMap::new(),
            relay_recovery: false,
            relay_progress: HashMap::new(),
            pending_closes: VecDeque::new(),
        }
    }

    pub(crate) fn with_relay_recovery() -> Self {
        Self {
            relay_recovery: true,
            ..Self::new()
        }
    }

    pub fn new_control(&self) -> TargetedStreamControl {
        TargetedStreamControl {
            commands: self.command_sender.clone(),
            next_request_id: self.next_request_id.clone(),
        }
    }

    fn handle_open(&mut self, command: OpenCommand) -> Option<OpenRequest> {
        if command.response.is_canceled() {
            return None;
        }

        let selected = self
            .connections
            .get(&command.connection_id)
            .copied()
            .filter(|_| {
                !self
                    .relay_progress
                    .get(&command.connection_id)
                    .is_some_and(|state| state.closing)
            });
        match selected {
            None => {
                let _ = command
                    .response
                    .send(Err(TargetedStreamError::SelectedConnectionClosed {
                        peer_id: command.peer_id.to_string(),
                        connection_id: command.connection_id,
                    }));
                return None;
            }
            Some(actual_peer_id) if actual_peer_id != command.peer_id => {
                let _ = command.response.send(Err(
                    TargetedStreamError::SelectedConnectionPeerMismatch {
                        expected_peer_id: command.peer_id.to_string(),
                        actual_peer_id: actual_peer_id.to_string(),
                        connection_id: command.connection_id,
                    },
                ));
                return None;
            }
            Some(_) => {}
        }

        let request = OpenRequest {
            request_id: command.request_id,
            protocol: command.protocol.clone(),
        };
        let old = self.pending.insert(
            command.request_id,
            PendingOpen {
                peer_id: command.peer_id,
                connection_id: command.connection_id,
                protocol: command.protocol,
                response: command.response,
                started: Instant::now(),
            },
        );
        debug_assert!(old.is_none(), "targeted-stream request ID reused");
        Some(request)
    }

    fn connection_established(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        let old = self.connections.insert(connection_id, peer_id);
        debug_assert!(old.is_none(), "libp2p reused an established connection ID");
    }

    fn connection_closed(&mut self, connection_id: ConnectionId) {
        self.connections.remove(&connection_id);
        self.relay_progress.remove(&connection_id);
        self.pending_closes
            .retain(|(_, queued)| *queued != connection_id);
        let failed = self
            .pending
            .iter()
            .filter_map(|(request_id, pending)| {
                (pending.connection_id == connection_id).then_some(*request_id)
            })
            .collect::<Vec<_>>();
        for request_id in failed {
            let Some(pending) = self.pending.remove(&request_id) else {
                continue;
            };
            let _ = pending
                .response
                .send(Err(TargetedStreamError::SelectedConnectionClosed {
                    peer_id: pending.peer_id.to_string(),
                    connection_id,
                }));
        }
    }
}

impl Default for TargetedStreamBehaviour {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkBehaviour for TargetedStreamBehaviour {
    type ConnectionHandler = TargetedStreamHandler;
    type ToSwarm = ();

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(TargetedStreamHandler::default())
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(TargetedStreamHandler::default())
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match event {
            FromSwarm::ConnectionEstablished(event) => {
                self.connection_established(event.peer_id, event.connection_id)
            }
            FromSwarm::ConnectionClosed(ConnectionClosed { connection_id, .. }) => {
                self.connection_closed(connection_id)
            }
            _ => {}
        }
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        let Some(pending) = self.pending.remove(&event.request_id) else {
            // A late handler event after connection closure is deliberately fenced.
            return;
        };
        if pending.peer_id != peer_id || pending.connection_id != connection_id {
            let _ = pending
                .response
                .send(Err(TargetedStreamError::HandlerRouteMismatch {
                    expected_peer_id: pending.peer_id.to_string(),
                    expected_connection_id: pending.connection_id,
                    actual_peer_id: peer_id.to_string(),
                    actual_connection_id: connection_id,
                }));
            return;
        }

        // Count handler outcomes even if an outer deadline canceled the caller.
        // Opens canceled before dispatch are discarded by handle_open instead.
        if self.relay_recovery {
            let now = Instant::now();
            if event.result.is_ok() {
                self.relay_progress
                    .entry(connection_id)
                    .or_default()
                    .succeeded(now);
            } else if pending.protocol == crate::source_admission::PROTOCOL
                && matches!(&event.result, Err(HandlerFailure::Timeout))
                && self
                    .relay_progress
                    .entry(connection_id)
                    .or_default()
                    .timed_out(pending.started, now)
            {
                tracing::warn!(target: "auki_p2p::relay_recovery",
                    remote_peer_id = %peer_id, connection_id = %connection_id,
                    reason = "relay_negotiation_stalled",
                    "retiring stalled direct relay connection");
                self.pending_closes.push_back((peer_id, connection_id));
            }
        }

        let result = match event.result {
            Ok(stream) => Ok(stream),
            Err(HandlerFailure::Timeout) => Err(TargetedStreamError::NegotiationTimeout {
                peer_id: peer_id.to_string(),
                connection_id,
                protocol: pending.protocol.as_ref().to_owned(),
            }),
            Err(HandlerFailure::UnsupportedProtocol) => {
                Err(TargetedStreamError::UnsupportedProtocol {
                    peer_id: peer_id.to_string(),
                    connection_id,
                    protocol: pending.protocol.as_ref().to_owned(),
                })
            }
            Err(HandlerFailure::Io(reason)) => Err(TargetedStreamError::Io {
                peer_id: peer_id.to_string(),
                connection_id,
                protocol: pending.protocol.as_ref().to_owned(),
                reason,
            }),
        };
        let _ = pending.response.send(result);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        if let Some((peer_id, connection_id)) = self.pending_closes.pop_front() {
            return Poll::Ready(ToSwarm::CloseConnection {
                peer_id,
                connection: CloseConnection::One(connection_id),
            });
        }
        loop {
            match self.commands.as_mut().poll_next(cx) {
                Poll::Ready(Some(command)) => {
                    let peer_id = command.peer_id;
                    let connection_id = command.connection_id;
                    if let Some(event) = self.handle_open(command) {
                        return Poll::Ready(ToSwarm::NotifyHandler {
                            peer_id,
                            handler: NotifyHandler::One(connection_id),
                            event,
                        });
                    }
                }
                Poll::Ready(None) | Poll::Pending => return Poll::Pending,
            }
        }
    }
}

struct OpenCommand {
    request_id: RequestId,
    peer_id: PeerId,
    connection_id: ConnectionId,
    protocol: StreamProtocol,
    response: oneshot::Sender<Result<Stream, TargetedStreamError>>,
}

struct PendingOpen {
    peer_id: PeerId,
    connection_id: ConnectionId,
    protocol: StreamProtocol,
    response: oneshot::Sender<Result<Stream, TargetedStreamError>>,
    started: Instant,
}

/// Monotonic correlation key carried as `OutboundOpenInfo`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(u64);

/// Behaviour-to-handler request for one negotiated substream.
#[derive(Debug)]
pub struct OpenRequest {
    request_id: RequestId,
    protocol: StreamProtocol,
}

/// Handler-to-behaviour completion for one negotiated substream.
#[derive(Debug)]
pub struct HandlerEvent {
    request_id: RequestId,
    result: Result<Stream, HandlerFailure>,
}

/// Negotiation failure reported by the selected connection handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerFailure {
    Timeout,
    UnsupportedProtocol,
    Io(String),
}

/// Per-connection handler for exact-route stream requests.
#[derive(Default)]
pub struct TargetedStreamHandler {
    queued: VecDeque<ConnectionHandlerEvent<ReadyUpgrade<StreamProtocol>, RequestId, HandlerEvent>>,
}

impl ConnectionHandler for TargetedStreamHandler {
    type FromBehaviour = OpenRequest;
    type ToBehaviour = HandlerEvent;
    type InboundProtocol = DeniedUpgrade;
    type OutboundProtocol = ReadyUpgrade<StreamProtocol>;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = RequestId;

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(DeniedUpgrade, ())
    }

    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        self.queued
            .push_back(ConnectionHandlerEvent::OutboundSubstreamRequest {
                protocol: SubstreamProtocol::new(
                    ReadyUpgrade::new(event.protocol),
                    event.request_id,
                ),
            });
    }

    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        match event {
            ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol,
                info,
            }) => self
                .queued
                .push_back(ConnectionHandlerEvent::NotifyBehaviour(HandlerEvent {
                    request_id: info,
                    result: Ok(protocol),
                })),
            ConnectionEvent::DialUpgradeError(DialUpgradeError { info, error }) => {
                let failure = match error {
                    StreamUpgradeError::Timeout => HandlerFailure::Timeout,
                    StreamUpgradeError::NegotiationFailed => HandlerFailure::UnsupportedProtocol,
                    StreamUpgradeError::Io(error) => HandlerFailure::Io(error.to_string()),
                    StreamUpgradeError::Apply(error) => match error {},
                };
                self.queued
                    .push_back(ConnectionHandlerEvent::NotifyBehaviour(HandlerEvent {
                        request_id: info,
                        result: Err(failure),
                    }));
            }
            _ => {}
        }
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        self.queued.pop_front().map_or(Poll::Pending, Poll::Ready)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::Pin,
        sync::atomic::{AtomicBool, Ordering as AtomicOrdering},
    };

    use futures::task::{noop_waker_ref, waker, ArcWake};
    use libp2p::{
        core::{connection::ConnectedPoint, upgrade::UpgradeInfo},
        swarm::{behaviour::ConnectionEstablished, StreamUpgradeError},
    };

    use super::*;

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn recovery_requires_sustained_timeouts_and_coalesces_concurrent_failures() {
        let now = Instant::now();
        let mut state = RelayProgress::default();
        for _ in 0..100 {
            assert!(!state.timed_out(now, now));
        }
        assert!(!state.timed_out(now, now + Duration::from_secs(19)));
        assert!(state.timed_out(now, now + RELAY_TIMEOUT_WINDOW));
        assert!(!state.timed_out(now, now + Duration::from_secs(60)));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn successful_progress_resets_budget_and_fences_older_pending_failures() {
        let now = Instant::now();
        let mut state = RelayProgress::default();
        assert!(!state.timed_out(now, now));
        assert!(!state.timed_out(now, now + Duration::from_secs(10)));
        let success = now + Duration::from_secs(15);
        state.succeeded(success);
        for _ in 0..10 {
            assert!(!state.timed_out(now, now + Duration::from_secs(30)));
        }
        assert_eq!(state.timeouts, 0);
        assert!(!state.timed_out(
            success + Duration::from_secs(1),
            now + Duration::from_secs(30)
        ));
        assert!(!state.timed_out(
            success + Duration::from_secs(2),
            now + Duration::from_secs(40)
        ));
        assert!(state.timed_out(
            success + Duration::from_secs(3),
            now + Duration::from_secs(50)
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn isolated_timeouts_do_not_accumulate_across_idle_periods() {
        let now = Instant::now();
        let mut state = RelayProgress::default();
        for round in 0..10 {
            let at = now + Duration::from_secs(round * 61);
            assert!(!state.timed_out(at, at));
        }
        assert_eq!(state.timeouts, 1);
    }

    fn inject_timeout(
        behaviour: &mut TargetedStreamBehaviour,
        peer: PeerId,
        connection: ConnectionId,
        protocol: StreamProtocol,
        canceled: bool,
    ) {
        let (response, receiver) = oneshot::channel();
        let id = RequestId(behaviour.next_request_id.fetch_add(1, Ordering::Relaxed));
        let request = behaviour
            .handle_open(OpenCommand {
                request_id: id,
                peer_id: peer,
                connection_id: connection,
                protocol,
                response,
            })
            .expect("request dispatched");
        let receiver = Some(receiver);
        let _retained = if canceled {
            drop(receiver);
            None
        } else {
            receiver
        };
        behaviour.on_connection_handler_event(
            peer,
            connection,
            HandlerEvent {
                request_id: request.request_id,
                result: Err(HandlerFailure::Timeout),
            },
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    async fn canceled_waiters_still_retire_only_the_stalled_connection_once() {
        let peer = PeerId::random();
        let bad = ConnectionId::new_unchecked(1);
        let sibling = ConnectionId::new_unchecked(2);
        let mut behaviour = TargetedStreamBehaviour::with_relay_recovery();
        behaviour.connection_established(peer, bad);
        behaviour.connection_established(peer, sibling);
        for _ in 0..2 {
            inject_timeout(
                &mut behaviour,
                peer,
                bad,
                crate::source_admission::PROTOCOL,
                true,
            );
        }
        // performance.now() begins at page creation: allow a real browser
        // window instead of subtracting an instant older than the page.
        #[cfg(target_arch = "wasm32")]
        futures_timer::Delay::new(RELAY_TIMEOUT_WINDOW).await;
        #[cfg(not(target_arch = "wasm32"))]
        {
            behaviour
                .relay_progress
                .get_mut(&bad)
                .unwrap()
                .first_timeout = Some(Instant::now() - RELAY_TIMEOUT_WINDOW);
        }
        inject_timeout(
            &mut behaviour,
            peer,
            bad,
            crate::source_admission::PROTOCOL,
            true,
        );
        let mut cx = Context::from_waker(noop_waker_ref());
        assert!(
            matches!(behaviour.poll(&mut cx), Poll::Ready(ToSwarm::CloseConnection {
            peer_id, connection: CloseConnection::One(id),
        }) if peer_id == peer && id == bad)
        );
        assert!(behaviour.poll(&mut cx).is_pending());
        assert_eq!(behaviour.connections.get(&sibling), Some(&peer));
        behaviour.connection_closed(bad);
        assert!(!behaviour.relay_progress.contains_key(&bad));
        assert_eq!(behaviour.connections.get(&sibling), Some(&peer));
        assert!(behaviour.pending.is_empty());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn application_protocol_timeouts_do_not_retire_relay_connections() {
        let peer = PeerId::random();
        let connection = ConnectionId::new_unchecked(1);
        let mut behaviour = TargetedStreamBehaviour::with_relay_recovery();
        behaviour.connection_established(peer, connection);
        for _ in 0..10 {
            inject_timeout(
                &mut behaviour,
                peer,
                connection,
                StreamProtocol::new("/app/slow/1"),
                false,
            );
        }
        assert!(behaviour.relay_progress.is_empty());
        assert!(behaviour.pending_closes.is_empty());
    }

    fn context() -> Context<'static> {
        Context::from_waker(noop_waker_ref())
    }

    fn peer() -> PeerId {
        crate::Identity::generate().peer_id()
    }

    fn establish(
        behaviour: &mut TargetedStreamBehaviour,
        peer_id: PeerId,
        connection_id: ConnectionId,
    ) {
        let endpoint = ConnectedPoint::Dialer {
            address: "/memory/1".parse().unwrap(),
            role_override: Endpoint::Dialer,
            port_use: PortUse::New,
        };
        behaviour.on_swarm_event(FromSwarm::ConnectionEstablished(ConnectionEstablished {
            peer_id,
            connection_id,
            endpoint: &endpoint,
            failed_addresses: &[],
            other_established: 0,
        }));
    }

    fn close(
        behaviour: &mut TargetedStreamBehaviour,
        peer_id: PeerId,
        connection_id: ConnectionId,
        remaining_established: usize,
    ) {
        let endpoint = ConnectedPoint::Dialer {
            address: "/memory/1".parse().unwrap(),
            role_override: Endpoint::Dialer,
            port_use: PortUse::New,
        };
        behaviour.on_swarm_event(FromSwarm::ConnectionClosed(ConnectionClosed {
            peer_id,
            connection_id,
            endpoint: &endpoint,
            cause: None,
            remaining_established,
        }));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn command_queue_keeps_its_declared_strict_bound() {
        let behaviour = TargetedStreamBehaviour::new();
        let remote = peer();
        let connection_id = ConnectionId::new_unchecked(10);
        let mut receivers = Vec::with_capacity(COMMAND_CAPACITY);

        for request_id in 0..COMMAND_CAPACITY {
            let (response, receiver) = oneshot::channel();
            let command = OpenCommand {
                request_id: RequestId(request_id as u64),
                peer_id: remote,
                connection_id,
                protocol: StreamProtocol::new("/auki-p2p/capacity/1"),
                response,
            };
            assert!(behaviour.command_sender.try_send(command).is_ok());
            receivers.push(receiver);
        }

        let (response, _receiver) = oneshot::channel();
        let overflow = OpenCommand {
            request_id: RequestId(COMMAND_CAPACITY as u64),
            peer_id: remote,
            connection_id,
            protocol: StreamProtocol::new("/auki-p2p/capacity/1"),
            response,
        };
        assert!(matches!(
            behaviour.command_sender.try_send(overflow),
            Err(async_channel::TrySendError::Full(_))
        ));
        assert_eq!(
            behaviour.commands.as_ref().get_ref().len(),
            COMMAND_CAPACITY
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn idle_behaviour_is_woken_when_a_command_arrives() {
        struct WakeFlag(AtomicBool);

        impl ArcWake for WakeFlag {
            fn wake_by_ref(flag: &Arc<Self>) {
                flag.0.store(true, AtomicOrdering::SeqCst);
            }
        }

        let mut behaviour = TargetedStreamBehaviour::new();
        let remote = peer();
        let connection_id = ConnectionId::new_unchecked(11);
        establish(&mut behaviour, remote, connection_id);

        let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
        let waker = waker(flag.clone());
        let mut cx = Context::from_waker(&waker);
        assert!(behaviour.poll(&mut cx).is_pending());

        let (response, _receiver) = oneshot::channel();
        assert!(behaviour
            .command_sender
            .try_send(OpenCommand {
                request_id: RequestId(1),
                peer_id: remote,
                connection_id,
                protocol: StreamProtocol::new("/auki-p2p/wakeup/1"),
                response,
            })
            .is_ok());
        assert!(flag.0.load(AtomicOrdering::SeqCst));
        assert!(matches!(
            behaviour.poll(&mut cx),
            Poll::Ready(ToSwarm::NotifyHandler {
                peer_id,
                handler: NotifyHandler::One(actual_connection_id),
                ..
            }) if peer_id == remote && actual_connection_id == connection_id
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn open_notifies_only_the_selected_connection_and_never_falls_back() {
        let mut behaviour = TargetedStreamBehaviour::new();
        let control = behaviour.new_control();
        let remote = peer();
        let other_connection = ConnectionId::new_unchecked(11);
        let selected_connection = ConnectionId::new_unchecked(12);
        establish(&mut behaviour, remote, other_connection);
        establish(&mut behaviour, remote, selected_connection);

        let mut open = Box::pin(control.open_stream(
            remote,
            selected_connection,
            StreamProtocol::new("/auki-p2p/test/1"),
        ));
        let mut cx = context();
        assert!(Pin::new(&mut open).poll(&mut cx).is_pending());
        let request_id = match behaviour.poll(&mut cx) {
            Poll::Ready(ToSwarm::NotifyHandler {
                peer_id,
                handler: NotifyHandler::One(connection_id),
                event,
            }) => {
                assert_eq!(peer_id, remote);
                assert_eq!(connection_id, selected_connection);
                event.request_id
            }
            _ => panic!("expected an exact handler notification"),
        };
        assert!(behaviour.pending.contains_key(&request_id));

        close(&mut behaviour, remote, selected_connection, 1);
        assert!(matches!(
            Pin::new(&mut open).poll(&mut cx),
            Poll::Ready(Err(TargetedStreamError::SelectedConnectionClosed {
                peer_id,
                connection_id,
            })) if peer_id == remote.to_string() && connection_id == selected_connection
        ));
        assert_eq!(behaviour.connections.get(&other_connection), Some(&remote));

        let mut closed_open = Box::pin(control.open_stream(
            remote,
            selected_connection,
            StreamProtocol::new("/auki-p2p/test/1"),
        ));
        assert!(Pin::new(&mut closed_open).poll(&mut cx).is_pending());
        assert!(behaviour.poll(&mut cx).is_pending());
        assert!(matches!(
            Pin::new(&mut closed_open).poll(&mut cx),
            Poll::Ready(Err(TargetedStreamError::SelectedConnectionClosed {
                connection_id,
                ..
            })) if connection_id == selected_connection
        ));

        let mut healthy_open = Box::pin(control.open_stream(
            remote,
            other_connection,
            StreamProtocol::new("/auki-p2p/test/1"),
        ));
        assert!(Pin::new(&mut healthy_open).poll(&mut cx).is_pending());
        assert!(matches!(
            behaviour.poll(&mut cx),
            Poll::Ready(ToSwarm::NotifyHandler {
                handler: NotifyHandler::One(connection_id),
                ..
            }) if connection_id == other_connection
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn concurrent_results_are_correlated_by_request_id() {
        let mut behaviour = TargetedStreamBehaviour::new();
        let control = behaviour.new_control();
        let remote = peer();
        let connection_id = ConnectionId::new_unchecked(21);
        establish(&mut behaviour, remote, connection_id);
        let mut cx = context();

        let mut first = Box::pin(control.open_stream(
            remote,
            connection_id,
            StreamProtocol::new("/auki-p2p/first/1"),
        ));
        let mut second = Box::pin(control.open_stream(
            remote,
            connection_id,
            StreamProtocol::new("/auki-p2p/second/1"),
        ));
        assert!(Pin::new(&mut first).poll(&mut cx).is_pending());
        assert!(Pin::new(&mut second).poll(&mut cx).is_pending());
        let first_id = match behaviour.poll(&mut cx) {
            Poll::Ready(ToSwarm::NotifyHandler { event, .. }) => event.request_id,
            _ => panic!("expected first request"),
        };
        let second_id = match behaviour.poll(&mut cx) {
            Poll::Ready(ToSwarm::NotifyHandler { event, .. }) => event.request_id,
            _ => panic!("expected second request"),
        };
        assert_ne!(first_id, second_id);

        behaviour.on_connection_handler_event(
            remote,
            connection_id,
            HandlerEvent {
                request_id: second_id,
                result: Err(HandlerFailure::Timeout),
            },
        );
        assert!(matches!(
            Pin::new(&mut second).poll(&mut cx),
            Poll::Ready(Err(TargetedStreamError::NegotiationTimeout { protocol, .. }))
                if protocol == "/auki-p2p/second/1"
        ));
        assert!(Pin::new(&mut first).poll(&mut cx).is_pending());

        behaviour.on_connection_handler_event(
            remote,
            connection_id,
            HandlerEvent {
                request_id: first_id,
                result: Err(HandlerFailure::UnsupportedProtocol),
            },
        );
        assert!(matches!(
            Pin::new(&mut first).poll(&mut cx),
            Poll::Ready(Err(TargetedStreamError::UnsupportedProtocol { protocol, .. }))
                if protocol == "/auki-p2p/first/1"
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn wrong_peer_for_connection_fails_without_notifying_a_handler() {
        let mut behaviour = TargetedStreamBehaviour::new();
        let control = behaviour.new_control();
        let actual_peer = peer();
        let expected_peer = peer();
        let connection_id = ConnectionId::new_unchecked(31);
        establish(&mut behaviour, actual_peer, connection_id);

        let mut open = Box::pin(control.open_stream(
            expected_peer,
            connection_id,
            StreamProtocol::new("/auki-p2p/test/1"),
        ));
        let mut cx = context();
        assert!(Pin::new(&mut open).poll(&mut cx).is_pending());
        assert!(behaviour.poll(&mut cx).is_pending());
        assert!(matches!(
            Pin::new(&mut open).poll(&mut cx),
            Poll::Ready(Err(TargetedStreamError::SelectedConnectionPeerMismatch {
                expected_peer_id,
                actual_peer_id,
                connection_id: actual_connection_id,
            })) if expected_peer_id == expected_peer.to_string()
                && actual_peer_id == actual_peer.to_string()
                && actual_connection_id == connection_id
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn handler_denies_inbound_and_carries_request_ids_as_open_info() {
        let mut handler = TargetedStreamHandler::default();
        let inbound = handler.listen_protocol().into_upgrade().0;
        assert_eq!(inbound.protocol_info().count(), 0);

        let first_id = RequestId(41);
        let second_id = RequestId(42);
        handler.on_behaviour_event(OpenRequest {
            request_id: first_id,
            protocol: StreamProtocol::new("/auki-p2p/first/1"),
        });
        handler.on_behaviour_event(OpenRequest {
            request_id: second_id,
            protocol: StreamProtocol::new("/auki-p2p/second/1"),
        });
        let mut cx = context();
        for expected in [first_id, second_id] {
            match handler.poll(&mut cx) {
                Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest { protocol }) => {
                    let (_, open_info) = protocol.into_upgrade();
                    assert_eq!(open_info, expected);
                }
                _ => panic!("expected outbound substream request"),
            }
        }

        handler.on_connection_event(ConnectionEvent::DialUpgradeError(DialUpgradeError {
            info: second_id,
            error: StreamUpgradeError::Timeout,
        }));
        handler.on_connection_event(ConnectionEvent::DialUpgradeError(DialUpgradeError {
            info: first_id,
            error: StreamUpgradeError::NegotiationFailed,
        }));
        match handler.poll(&mut cx) {
            Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(HandlerEvent {
                request_id,
                result: Err(HandlerFailure::Timeout),
            })) => assert_eq!(request_id, second_id),
            _ => panic!("expected second request timeout"),
        }
        match handler.poll(&mut cx) {
            Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(HandlerEvent {
                request_id,
                result: Err(HandlerFailure::UnsupportedProtocol),
            })) => assert_eq!(request_id, first_id),
            _ => panic!("expected first request negotiation failure"),
        }
    }
}
