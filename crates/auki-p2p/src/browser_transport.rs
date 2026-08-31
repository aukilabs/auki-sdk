//! Browser WSS/Circuit Relay runtime driven on the local Wasm executor.
//!
//! The first browser contract intentionally owns one relay reservation for its
//! lifetime. Changing providers or recovering a terminal reservation starts a
//! fresh node; discovery and automatic relay recovery remain facade concerns.

use std::{
    collections::{HashMap, VecDeque},
    error::Error as StdError,
    pin::Pin,
    rc::Rc,
    time::Duration,
};

use async_channel::{Receiver, Sender};
use chrono::{DateTime, Utc};
use futures::{
    channel::oneshot,
    future::{poll_fn, FutureExt, Shared},
    Stream as FuturesStream, StreamExt,
};
use libp2p::{
    core::{upgrade, Transport as _},
    noise, relay,
    swarm::{
        dial_opts::{DialOpts, PeerCondition},
        ConnectionId, NetworkBehaviour, SwarmEvent,
    },
    websocket_websys, yamux, Multiaddr, PeerId, Stream, Swarm, SwarmBuilder,
};
use libp2p_stream::{Behaviour as StreamBehaviour, IncomingStreams};
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;

use crate::{
    authentication::{authenticate_duplex, SessionRequirements},
    browser_authority::BrowserAuthority,
    browser_route::{browser_direct_address, parse_browser_relay_route_for_peer},
    relay::{
        ObservedRelayLimits, RelayCancellation, RelayProvider, RelayReservationEvent,
        RelayReservationHandle, RelayReservationNode, RelayReservationSnapshot,
    },
    relay_client, source_admission,
    targeted_stream::{TargetedStreamBehaviour, TargetedStreamControl},
    ApplicationProtocol, AuthenticatedStream, Error, Identity, PeerAuthorityUpdate, Result,
};

const COMMAND_CAPACITY: usize = 16;
const INCOMING_AUTH_CONCURRENCY: usize = 4;
const INCOMING_AUTH_QUEUE_CAPACITY: usize = 8;

#[derive(NetworkBehaviour)]
struct BrowserBehaviour {
    relay: relay_client::Behaviour,
    streams: StreamBehaviour,
    targeted_streams: TargetedStreamBehaviour,
}

/// Incoming application streams authenticated for the node's fixed Domain.
///
/// A bounded local worker pool keeps accepting and authenticating streams even
/// while the caller handles an earlier result.
pub struct BrowserIncomingAuthenticatedStreams {
    results: Receiver<Result<AuthenticatedStream>>,
}

impl BrowserIncomingAuthenticatedStreams {
    pub async fn accept(&mut self) -> Option<Result<AuthenticatedStream>> {
        self.results.recv().await.ok()
    }
}

/// Why a browser node's local swarm stopped.
///
/// Relay failure is terminal in the first browser contract. Callers restart a
/// fresh node rather than coordinating in-place provider replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserNodeExit {
    Shutdown,
    OwnersDropped,
    SwarmEnded,
    RuntimeDropped,
    RelayReservationAbandoned,
    RelayReservationFailed { reason: String },
}

/// Exact browser circuit selected after relay source admission.
#[derive(Clone, Debug)]
pub struct BrowserRelayRoute {
    node_instance_id: Uuid,
    relay_peer_id: PeerId,
    target_peer_id: PeerId,
    connection_id: ConnectionId,
    admission_expires_at: DateTime<Utc>,
    route: Multiaddr,
}

impl BrowserRelayRoute {
    pub fn relay_peer_id(&self) -> PeerId {
        self.relay_peer_id
    }

    pub fn target_peer_id(&self) -> PeerId {
        self.target_peer_id
    }

    pub fn route(&self) -> &Multiaddr {
        &self.route
    }

    pub fn admission_expires_at(&self) -> DateTime<Utc> {
        self.admission_expires_at
    }
}

/// Authenticated browser P2P node.
///
/// The swarm remains on the browser's local executor. Raw libp2p controls,
/// unauthenticated streams, token bytes, and the swarm itself never escape.
pub struct BrowserNode {
    node_instance_id: Uuid,
    peer_id: PeerId,
    authority: BrowserAuthority,
    streams: libp2p_stream::Control,
    targeted_streams: TargetedStreamControl,
    commands: Sender<Command>,
    stopped: Shared<oneshot::Receiver<BrowserNodeExit>>,
    _local_only: Rc<()>,
}

impl BrowserNode {
    pub async fn start(identity: Identity, initial: PeerAuthorityUpdate) -> Result<Self> {
        let peer_id = identity.peer_id();
        let authority = BrowserAuthority::start(peer_id, initial).await?;

        let stream_behaviour = StreamBehaviour::new();
        let streams = stream_behaviour.new_control();
        let targeted_behaviour = TargetedStreamBehaviour::new();
        let targeted_streams = targeted_behaviour.new_control();
        let swarm = build_swarm(identity.keypair(), stream_behaviour, targeted_behaviour)
            .map_err(|error| Error::TransportBuild(error.to_string()))?;
        let (commands, command_receiver) = async_channel::bounded(COMMAND_CAPACITY);
        let (stopped_sender, stopped_receiver) = oneshot::channel();

        spawn_local(async move {
            let status = match run_swarm(swarm, command_receiver).await {
                BrowserSwarmExit::Shutdown(response) => {
                    let _ = response.send(Ok(()));
                    BrowserNodeExit::Shutdown
                }
                BrowserSwarmExit::Terminal(status) => status,
                BrowserSwarmExit::OwnersDropped => BrowserNodeExit::OwnersDropped,
                BrowserSwarmExit::SwarmEnded => BrowserNodeExit::SwarmEnded,
            };
            let _ = stopped_sender.send(status);
        });

        Ok(Self {
            node_instance_id: Uuid::new_v4(),
            peer_id,
            authority,
            streams,
            targeted_streams,
            commands,
            stopped: stopped_receiver.shared(),
            _local_only: Rc::new(()),
        })
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn domain_id(&self) -> Uuid {
        self.authority.domain_id()
    }

    pub fn authority(&self) -> BrowserAuthority {
        self.authority.clone()
    }

    pub fn accept(
        &self,
        protocol: ApplicationProtocol,
    ) -> Result<BrowserIncomingAuthenticatedStreams> {
        let mut streams = self.streams.clone();
        let incoming = streams
            .accept(protocol.stream_protocol())
            .map_err(|_| Error::ProtocolAlreadyRegistered)?;
        let requirements = SessionRequirements::new(self.domain_id().to_string())?;
        let (pending_sender, pending_receiver) =
            async_channel::bounded(INCOMING_AUTH_QUEUE_CAPACITY);
        let (result_sender, results) = async_channel::bounded(INCOMING_AUTH_QUEUE_CAPACITY);

        let pump_results = result_sender.clone();
        spawn_local(async move {
            pump_incoming_streams(incoming, pending_sender, pump_results).await;
        });
        for _ in 0..INCOMING_AUTH_CONCURRENCY {
            let pending = pending_receiver.clone();
            let results = result_sender.clone();
            let authority = self.authority.clone();
            let requirements = requirements.clone();
            let local_peer_id = self.peer_id;
            spawn_local(async move {
                authenticate_incoming_streams(
                    pending,
                    results,
                    local_peer_id,
                    authority,
                    requirements,
                )
                .await;
            });
        }
        drop(pending_receiver);
        drop(result_sender);

        Ok(BrowserIncomingAuthenticatedStreams { results })
    }

    /// Establish the node's one WSS relay reservation and wait until both the
    /// exact listener and finite-limit acceptance evidence make it publishable.
    /// One failed or abandoned attempt is terminal; construct a fresh node.
    pub async fn reserve_relay(&self, provider: RelayProvider) -> Result<RelayReservationSnapshot> {
        browser_direct_address(&provider)?;
        let (response, receiver) = oneshot::channel();
        self.send(Command::ReserveRelay { provider, response })
            .await?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    /// Perform relay source admission and establish one exact WSS circuit.
    pub async fn connect_relayed(
        &self,
        expected_peer_id: PeerId,
        route: Multiaddr,
    ) -> Result<BrowserRelayRoute> {
        // The expected terminal Peer ID is checked before source admission so
        // a mismatched route never receives this peer's DDS credential.
        let parsed = parse_browser_relay_route_for_peer(&route, expected_peer_id)?;
        let tokens = self.authority.tokens();
        let verifier = self.authority.verifier();
        let authorization = source_admission::prepare_authorization(
            self.peer_id,
            parsed.target_peer_id,
            self.domain_id(),
            &tokens,
            &verifier,
        )
        .await?;
        let relay_connection = self
            .select_relay_connection(parsed.relay_peer_id, parsed.direct_relay_address)
            .await?;
        let mut stream = self
            .targeted_streams
            .open_stream(
                parsed.relay_peer_id,
                relay_connection,
                source_admission::PROTOCOL,
            )
            .await?;
        let admission_expires_at =
            source_admission::authorize_prepared(&mut stream, authorization, Utc::now).await?;
        let connection_id = self
            .dial_circuit(
                parsed.target_peer_id,
                parsed.circuit_dial_address,
                parsed.relay_peer_id,
            )
            .await?;
        Ok(BrowserRelayRoute {
            node_instance_id: self.node_instance_id,
            relay_peer_id: parsed.relay_peer_id,
            target_peer_id: parsed.target_peer_id,
            connection_id,
            admission_expires_at,
            route,
        })
    }

    /// Open one application stream on the exact circuit and run mutual DDS
    /// authentication before exposing bytes.
    pub async fn open_relayed(
        &self,
        route: &BrowserRelayRoute,
        protocol: ApplicationProtocol,
    ) -> Result<AuthenticatedStream> {
        if route.node_instance_id != self.node_instance_id {
            return Err(Error::ForeignRelayRoute);
        }
        let requirements = SessionRequirements::new(self.domain_id().to_string())?
            .with_expected_remote_peer_id(route.target_peer_id);
        let stream = self
            .targeted_streams
            .open_stream(
                route.target_peer_id,
                route.connection_id,
                protocol.stream_protocol(),
            )
            .await?;
        authenticate_stream(
            stream,
            self.peer_id,
            route.target_peer_id,
            &self.authority,
            &requirements,
        )
        .await
    }

    pub async fn close_relay_route(&self, route: &BrowserRelayRoute) -> Result<()> {
        if route.node_instance_id != self.node_instance_id {
            return Err(Error::ForeignRelayRoute);
        }
        let (response, receiver) = oneshot::channel();
        self.send(Command::CloseConnection {
            connection_id: route.connection_id,
            response,
        })
        .await?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    /// Drop the browser swarm and its WebSocket/relay resources before this
    /// future resolves.
    pub async fn shutdown(&self) -> Result<()> {
        let (response, receiver) = oneshot::channel();
        self.send(Command::Shutdown { response }).await?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    /// Wait for the shared terminal status while other code continues using
    /// the node. Every waiter observes the same value.
    pub async fn wait_stopped(&self) -> BrowserNodeExit {
        self.stopped
            .clone()
            .await
            .unwrap_or(BrowserNodeExit::RuntimeDropped)
    }

    async fn select_relay_connection(
        &self,
        peer_id: PeerId,
        address: Multiaddr,
    ) -> Result<ConnectionId> {
        let (response, receiver) = oneshot::channel();
        self.send(Command::SelectRelayConnection {
            peer_id,
            address,
            response,
        })
        .await?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    async fn dial_circuit(
        &self,
        peer_id: PeerId,
        address: Multiaddr,
        relay_peer_id: PeerId,
    ) -> Result<ConnectionId> {
        let (response, receiver) = oneshot::channel();
        self.send(Command::DialCircuit {
            peer_id,
            address,
            relay_peer_id,
            response,
        })
        .await?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    async fn send(&self, command: Command) -> Result<()> {
        self.commands
            .send(command)
            .await
            .map_err(|_| Error::SwarmStopped)
    }
}

impl std::fmt::Debug for BrowserNode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserNode")
            .field("peer_id", &self.peer_id)
            .field("domain_id", &self.domain_id())
            .field("authority", &"[redacted]")
            .finish_non_exhaustive()
    }
}

enum Command {
    SelectRelayConnection {
        peer_id: PeerId,
        address: Multiaddr,
        response: oneshot::Sender<Result<ConnectionId>>,
    },
    ReserveRelay {
        provider: RelayProvider,
        response: oneshot::Sender<Result<RelayReservationSnapshot>>,
    },
    DialCircuit {
        peer_id: PeerId,
        address: Multiaddr,
        relay_peer_id: PeerId,
        response: oneshot::Sender<Result<ConnectionId>>,
    },
    CloseConnection {
        connection_id: ConnectionId,
        response: oneshot::Sender<Result<()>>,
    },
    Shutdown {
        response: oneshot::Sender<Result<()>>,
    },
}

struct BrowserRuntime {
    direct_connections: HashMap<PeerId, DirectConnection>,
    pending_dials: HashMap<ConnectionId, PendingDial>,
    reservations: RelayReservationNode,
    reservation_started: bool,
    reservation_waiter: Option<(
        RelayReservationHandle,
        oneshot::Sender<Result<RelayReservationSnapshot>>,
    )>,
    terminal_exit: Option<BrowserNodeExit>,
}

#[derive(Clone)]
struct DirectConnection {
    connection_id: ConnectionId,
    address: Multiaddr,
}

enum PendingDial {
    Direct {
        peer_id: PeerId,
        address: Multiaddr,
        responses: Vec<oneshot::Sender<Result<ConnectionId>>>,
    },
    Reservation {
        peer_id: PeerId,
        address: Multiaddr,
        provider: RelayProvider,
        response: oneshot::Sender<Result<RelayReservationSnapshot>>,
    },
    Circuit {
        peer_id: PeerId,
        relay_peer_id: PeerId,
        response: oneshot::Sender<Result<ConnectionId>>,
    },
}

impl BrowserRuntime {
    fn new(local_peer_id: PeerId) -> Self {
        Self {
            direct_connections: HashMap::new(),
            pending_dials: HashMap::new(),
            reservations: RelayReservationNode::new(local_peer_id),
            reservation_started: false,
            reservation_waiter: None,
            terminal_exit: None,
        }
    }

    fn handle_command(&mut self, swarm: &mut Swarm<BrowserBehaviour>, command: Command) -> bool {
        match command {
            Command::SelectRelayConnection {
                peer_id,
                address,
                response,
            } => self.select_relay_connection(swarm, peer_id, address, response),
            Command::ReserveRelay { provider, response } => {
                self.reserve_relay(swarm, provider, response)
            }
            Command::DialCircuit {
                peer_id,
                address,
                relay_peer_id,
                response,
            } => self.dial_circuit(swarm, peer_id, address, relay_peer_id, response),
            Command::CloseConnection {
                connection_id,
                response,
            } => {
                swarm.close_connection(connection_id);
                let _ = response.send(Ok(()));
            }
            Command::Shutdown { .. } => return true,
        }
        false
    }

    fn select_relay_connection(
        &mut self,
        swarm: &mut Swarm<BrowserBehaviour>,
        peer_id: PeerId,
        address: Multiaddr,
        response: oneshot::Sender<Result<ConnectionId>>,
    ) {
        if response.is_canceled() {
            return;
        }
        if let Some(existing) = self.direct_connections.get(&peer_id) {
            let result = if existing.address == address {
                Ok(existing.connection_id)
            } else {
                Err(Error::RelayDirectConnectionMismatch {
                    relay_peer_id: peer_id.to_string(),
                    expected: address.to_string(),
                    actual: existing.address.to_string(),
                })
            };
            let _ = response.send(result);
            return;
        }
        if self.pending_dials.values().any(|pending| {
            matches!(
                pending,
                PendingDial::Reservation {
                    peer_id: pending_peer,
                    ..
                } if *pending_peer == peer_id
            )
        }) {
            let _ = response.send(Err(Error::RelayReservationClosed(
                "relay reservation connection attempt is still pending".into(),
            )));
            return;
        }
        if let Some(PendingDial::Direct {
            address: pending_address,
            responses,
            ..
        }) = self.pending_dials.values_mut().find(|pending| {
            matches!(pending, PendingDial::Direct { peer_id: pending_peer, .. } if *pending_peer == peer_id)
        }) {
            if *pending_address == address {
                responses.push(response);
            } else {
                let _ = response.send(Err(Error::RelayDirectConnectionMismatch {
                    relay_peer_id: peer_id.to_string(),
                    expected: address.to_string(),
                    actual: pending_address.to_string(),
                }));
            }
            return;
        }

        let dial = DialOpts::peer_id(peer_id)
            .condition(PeerCondition::Always)
            .allocate_new_port()
            .addresses(vec![address.clone()])
            .build();
        let connection_id = dial.connection_id();
        match swarm.dial(dial) {
            Ok(()) => {
                self.pending_dials.insert(
                    connection_id,
                    PendingDial::Direct {
                        peer_id,
                        address,
                        responses: vec![response],
                    },
                );
            }
            Err(error) => {
                let _ = response.send(Err(Error::Dial(error.to_string())));
            }
        }
    }

    fn reserve_relay(
        &mut self,
        swarm: &mut Swarm<BrowserBehaviour>,
        provider: RelayProvider,
        response: oneshot::Sender<Result<RelayReservationSnapshot>>,
    ) {
        if self.reservation_started {
            let _ = response.send(Err(Error::RelayReservationClosed(
                "browser nodes support one relay reservation for their lifetime".into(),
            )));
            return;
        }
        if response.is_canceled() {
            return;
        }

        let relay_peer_id = provider.relay_peer_id();
        let address = match browser_direct_address(&provider) {
            Ok(address) => address,
            Err(error) => {
                let _ = response.send(Err(error));
                return;
            }
        };
        if self.pending_dials.values().any(|pending| {
            matches!(
                pending,
                PendingDial::Direct {
                    peer_id: pending_peer,
                    ..
                } | PendingDial::Reservation {
                    peer_id: pending_peer,
                    ..
                } if *pending_peer == relay_peer_id
            )
        }) {
            let _ = response.send(Err(Error::RelayReservationClosed(
                "a direct connection attempt to the selected relay is already pending".into(),
            )));
            return;
        }
        if self.pending_dials.values().any(|pending| {
            matches!(
                pending,
                PendingDial::Circuit {
                    relay_peer_id: pending_relay,
                    ..
                } if *pending_relay == relay_peer_id
            )
        }) {
            let _ = response.send(Err(Error::RelayReservationClosed(
                "an outbound circuit request is still pending on the selected relay".into(),
            )));
            return;
        }

        self.reservation_started = true;
        if let Some(existing) = self.direct_connections.get(&relay_peer_id).cloned() {
            if existing.address != address {
                let error = Error::RelayDirectConnectionMismatch {
                    relay_peer_id: relay_peer_id.to_string(),
                    expected: address.to_string(),
                    actual: existing.address.to_string(),
                };
                self.mark_reservation_failed(error.to_string());
                let _ = response.send(Err(error));
                return;
            }
            self.begin_reservation(swarm, provider, existing.connection_id, response);
            return;
        }

        let dial = DialOpts::peer_id(relay_peer_id)
            .condition(PeerCondition::Always)
            .allocate_new_port()
            .addresses(vec![address.clone()])
            .build();
        let connection_id = dial.connection_id();
        match swarm.dial(dial) {
            Ok(()) => {
                self.pending_dials.insert(
                    connection_id,
                    PendingDial::Reservation {
                        peer_id: relay_peer_id,
                        address,
                        provider,
                        response,
                    },
                );
            }
            Err(error) => {
                let error = Error::Dial(error.to_string());
                self.mark_reservation_failed(error.to_string());
                let _ = response.send(Err(error));
            }
        }
    }

    fn begin_reservation(
        &mut self,
        swarm: &mut Swarm<BrowserBehaviour>,
        provider: RelayProvider,
        direct_connection: ConnectionId,
        response: oneshot::Sender<Result<RelayReservationSnapshot>>,
    ) {
        if response.is_canceled() {
            self.mark_reservation_abandoned();
            return;
        }
        let relay_peer_id = provider.relay_peer_id();
        if self
            .direct_connections
            .get(&relay_peer_id)
            .is_none_or(|connection| connection.connection_id != direct_connection)
        {
            let error = Error::RelayReservationClosed(
                "selected direct relay connection closed before reservation start".into(),
            );
            self.mark_reservation_failed(error.to_string());
            let _ = response.send(Err(error));
            return;
        }
        if self.pending_dials.values().any(|pending| {
            matches!(
                pending,
                PendingDial::Circuit {
                    relay_peer_id: pending_relay,
                    ..
                } if *pending_relay == relay_peer_id
            )
        }) {
            let error = Error::RelayReservationClosed(
                "an outbound circuit request is still pending on the selected relay".into(),
            );
            self.mark_reservation_failed(error.to_string());
            let _ = response.send(Err(error));
            return;
        }

        let listen_address = provider.reservation_listen_address();
        let listener_id = match swarm.listen_on(listen_address.clone()) {
            Ok(listener_id) => listener_id,
            Err(error) => {
                let error = Error::Listen {
                    address: listen_address.to_string(),
                    reason: error.to_string(),
                };
                self.mark_reservation_failed(error.to_string());
                let _ = response.send(Err(error));
                return;
            }
        };
        let handle = match self.reservations.begin(provider, listener_id) {
            Ok(handle) => handle,
            Err(error) => {
                swarm.remove_listener(listener_id);
                let error = Error::from(error);
                self.mark_reservation_failed(error.to_string());
                let _ = response.send(Err(error));
                return;
            }
        };
        if let Err(error) = self
            .reservations
            .observe_direct_connection(handle, direct_connection)
        {
            if let Ok(event) = self.reservations.cancel(handle) {
                self.apply_reservation_event(swarm, event);
            } else {
                swarm.remove_listener(listener_id);
            }
            let error = Error::from(error);
            self.mark_reservation_failed(error.to_string());
            let _ = response.send(Err(error));
            return;
        }
        if swarm
            .behaviour_mut()
            .relay
            .register_dispatch(handle, direct_connection)
            .is_err()
        {
            let error = Error::RelayReservationClosed(
                "another reservation dispatch is already pending".into(),
            );
            self.mark_reservation_failed(error.to_string());
            let _ = response.send(Err(error));
            if let Ok(event) = self.reservations.cancel(handle) {
                self.apply_reservation_event(swarm, event);
            }
            return;
        }
        self.reservation_waiter = Some((handle, response));
    }

    fn dial_circuit(
        &mut self,
        swarm: &mut Swarm<BrowserBehaviour>,
        peer_id: PeerId,
        address: Multiaddr,
        relay_peer_id: PeerId,
        response: oneshot::Sender<Result<ConnectionId>>,
    ) {
        if response.is_canceled() {
            return;
        }
        if swarm.behaviour().relay.has_pending_dispatch(relay_peer_id) {
            let _ = response.send(Err(Error::RelayReservationClosed(
                "relay reservation dispatch is still pending".into(),
            )));
            return;
        }
        let dial = DialOpts::peer_id(peer_id)
            .condition(PeerCondition::Always)
            .allocate_new_port()
            .addresses(vec![address])
            .build();
        let connection_id = dial.connection_id();
        match swarm.dial(dial) {
            Ok(()) => {
                self.pending_dials.insert(
                    connection_id,
                    PendingDial::Circuit {
                        peer_id,
                        relay_peer_id,
                        response,
                    },
                );
            }
            Err(error) => {
                let _ = response.send(Err(Error::Dial(error.to_string())));
            }
        }
    }

    fn handle_swarm_event(
        &mut self,
        swarm: &mut Swarm<BrowserBehaviour>,
        event: SwarmEvent<BrowserBehaviourEvent>,
    ) {
        match event {
            SwarmEvent::NewListenAddr {
                listener_id,
                address,
            } => {
                if let Some(handle) = self.reservations.handle_for_listener(listener_id) {
                    if let Ok(event) =
                        self.reservations
                            .observe_listener_address(handle, listener_id, &address)
                    {
                        self.apply_reservation_event(swarm, event);
                    }
                }
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            } => self.connection_established(swarm, peer_id, connection_id, endpoint.is_relayed()),
            SwarmEvent::OutgoingConnectionError {
                connection_id,
                error,
                ..
            } => {
                if let Some(pending) = self.pending_dials.remove(&connection_id) {
                    let reason = error.to_string();
                    if pending.is_reservation() {
                        self.mark_reservation_failed(format!(
                            "failed to dial selected relay: {reason}"
                        ));
                    }
                    pending.fail_dial(reason);
                }
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                connection_id,
                endpoint,
                ..
            } => {
                if !endpoint.is_relayed()
                    && self
                        .direct_connections
                        .get(&peer_id)
                        .is_some_and(|connection| connection.connection_id == connection_id)
                {
                    self.direct_connections.remove(&peer_id);
                    if let Some(handle) = self.reservations.handle_for_relay(peer_id) {
                        if self
                            .reservations
                            .snapshot(handle)
                            .ok()
                            .and_then(|snapshot| snapshot.direct_connection())
                            == Some(connection_id)
                        {
                            self.fail_reservation(
                                handle,
                                Error::RelayReservationClosed(
                                    "selected direct relay connection closed".into(),
                                ),
                            );
                            if let Ok(event) = self
                                .reservations
                                .observe_direct_connection_closed(handle, connection_id)
                            {
                                self.apply_reservation_event(swarm, event);
                            }
                        }
                    }
                }
            }
            SwarmEvent::ListenerClosed {
                listener_id,
                reason,
                ..
            } => {
                if let Some(handle) = self.reservations.handle_for_listener(listener_id) {
                    let reason = reason
                        .err()
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "relay reservation listener closed".into());
                    self.fail_reservation(handle, Error::RelayReservationClosed(reason));
                    if let Ok(event) = self
                        .reservations
                        .observe_listener_closed(handle, listener_id)
                    {
                        self.apply_reservation_event(swarm, event);
                    }
                }
            }
            SwarmEvent::Behaviour(BrowserBehaviourEvent::Relay(
                relay_client::Event::ReservationDispatchFailed { handle, reason },
            )) => {
                self.fail_reservation(handle, Error::RelayReservationClosed(reason.to_string()));
                if let Ok(event) = self.reservations.cancel(handle) {
                    self.apply_reservation_event(swarm, event);
                }
            }
            SwarmEvent::Behaviour(BrowserBehaviourEvent::Relay(
                relay_client::Event::Upstream {
                    event:
                        relay::client::Event::ReservationReqAccepted {
                            relay_peer_id,
                            renewal,
                            limit,
                        },
                    handle,
                },
            )) => {
                let Some(handle) = handle else {
                    if let Some(active) = self.reservations.handle_for_relay(relay_peer_id) {
                        self.fail_reservation(
                            active,
                            Error::RelayReservationClosed(
                                "relay acceptance was not correlated to its reservation".into(),
                            ),
                        );
                    }
                    return;
                };
                if handle.relay_peer_id() != relay_peer_id {
                    self.fail_reservation(
                        handle,
                        Error::RelayReservationClosed(
                            "relay acceptance carried a different relay Peer ID".into(),
                        ),
                    );
                    if let Ok(event) = self.reservations.cancel(handle) {
                        self.apply_reservation_event(swarm, event);
                    }
                    return;
                }
                let observed = limit
                    .map(|limit| ObservedRelayLimits::new(limit.duration(), limit.data_in_bytes()));
                if let Ok(event) = self
                    .reservations
                    .observe_acceptance(handle, renewal, observed)
                {
                    self.apply_reservation_event(swarm, event);
                }
            }
            _ => {}
        }
    }

    fn connection_established(
        &mut self,
        swarm: &mut Swarm<BrowserBehaviour>,
        peer_id: PeerId,
        connection_id: ConnectionId,
        relayed: bool,
    ) {
        let Some(pending) = self.pending_dials.remove(&connection_id) else {
            return;
        };
        match pending {
            PendingDial::Direct {
                peer_id: expected,
                address,
                responses,
            } if expected == peer_id && !relayed => {
                self.direct_connections.insert(
                    peer_id,
                    DirectConnection {
                        connection_id,
                        address,
                    },
                );
                let mut accepted = false;
                for response in responses {
                    accepted |= response.send(Ok(connection_id)).is_ok();
                }
                if !accepted {
                    self.direct_connections.remove(&peer_id);
                    swarm.close_connection(connection_id);
                }
            }
            PendingDial::Circuit {
                peer_id: expected,
                response,
                ..
            } if expected == peer_id && relayed => {
                if response.send(Ok(connection_id)).is_err() {
                    swarm.close_connection(connection_id);
                }
            }
            PendingDial::Reservation {
                peer_id: expected,
                address,
                provider,
                response,
            } if expected == peer_id && !relayed => {
                self.direct_connections.insert(
                    peer_id,
                    DirectConnection {
                        connection_id,
                        address,
                    },
                );
                if response.is_canceled() {
                    self.direct_connections.remove(&peer_id);
                    swarm.close_connection(connection_id);
                    self.mark_reservation_abandoned();
                    return;
                }
                self.begin_reservation(swarm, provider, connection_id, response);
            }
            pending => {
                swarm.close_connection(connection_id);
                if pending.is_reservation() {
                    self.mark_reservation_failed(format!(
                        "selected relay dial connected to unexpected Peer {peer_id}"
                    ));
                }
                pending.fail_unexpected_peer(peer_id);
            }
        }
    }

    fn apply_reservation_event(
        &mut self,
        swarm: &mut Swarm<BrowserBehaviour>,
        event: RelayReservationEvent,
    ) {
        let mut events = VecDeque::from([event]);
        while let Some(event) = events.pop_front() {
            match event {
                RelayReservationEvent::EvidenceRecorded { .. }
                | RelayReservationEvent::Renewed { .. }
                | RelayReservationEvent::Fenced { .. } => {}
                RelayReservationEvent::Publishable { handle, .. } => {
                    if let Some((waiting_handle, response)) = self.reservation_waiter.take() {
                        if waiting_handle == handle {
                            match self.reservations.snapshot(handle) {
                                Ok(snapshot) => {
                                    if response.send(Ok(snapshot)).is_err() {
                                        self.mark_reservation_abandoned();
                                        if let Ok(event) = self.reservations.cancel(handle) {
                                            events.push_back(event);
                                        }
                                    }
                                }
                                Err(error) => {
                                    let error = Error::from(error);
                                    self.mark_reservation_failed(error.to_string());
                                    let _ = response.send(Err(error));
                                    if let Ok(event) = self.reservations.cancel(handle) {
                                        events.push_back(event);
                                    }
                                }
                            }
                        } else {
                            self.reservation_waiter = Some((waiting_handle, response));
                        }
                    }
                }
                RelayReservationEvent::ConfirmationRejected {
                    handle,
                    reason,
                    cancellation,
                } => {
                    self.fail_reservation(handle, Error::RelayConfirmationRejected(reason));
                    events.extend(self.start_teardown(swarm, cancellation));
                }
                RelayReservationEvent::CancellationStarted { cancellation }
                | RelayReservationEvent::CancellationPending { cancellation } => {
                    events.extend(self.start_teardown(swarm, cancellation));
                }
                RelayReservationEvent::CloseLateConnection {
                    handle,
                    connection_id,
                } => {
                    if !swarm.close_connection(connection_id) {
                        if let Ok(event) = self
                            .reservations
                            .observe_direct_connection_closed(handle, connection_id)
                        {
                            events.push_back(event);
                        }
                    }
                }
                RelayReservationEvent::Canceled { .. } => {}
            }
        }
    }

    fn start_teardown(
        &mut self,
        swarm: &mut Swarm<BrowserBehaviour>,
        cancellation: RelayCancellation,
    ) -> Vec<RelayReservationEvent> {
        swarm
            .behaviour_mut()
            .relay
            .fence_dispatch(cancellation.handle());
        let mut events = Vec::new();
        if !swarm.remove_listener(cancellation.listener_id()) {
            if let Ok(event) = self
                .reservations
                .observe_listener_closed(cancellation.handle(), cancellation.listener_id())
            {
                events.push(event);
            }
        }
        match cancellation.direct_connection() {
            Some(connection_id) if !swarm.close_connection(connection_id) => {
                if let Ok(event) = self
                    .reservations
                    .observe_direct_connection_closed(cancellation.handle(), connection_id)
                {
                    events.push(event);
                }
            }
            None => {
                if let Ok(event) = self
                    .reservations
                    .observe_no_direct_connection(cancellation.handle())
                {
                    events.push(event);
                }
            }
            Some(_) => {}
        }
        events
    }

    fn fail_reservation(&mut self, handle: RelayReservationHandle, error: Error) {
        let reason = error.to_string();
        if self
            .reservation_waiter
            .as_ref()
            .is_some_and(|(waiting, _)| *waiting == handle)
        {
            if let Some((_, response)) = self.reservation_waiter.take() {
                let _ = response.send(Err(error));
            }
        }
        self.mark_reservation_failed(reason);
    }

    fn mark_reservation_failed(&mut self, reason: String) {
        self.terminal_exit
            .get_or_insert(BrowserNodeExit::RelayReservationFailed { reason });
    }

    fn mark_reservation_abandoned(&mut self) {
        self.terminal_exit
            .get_or_insert(BrowserNodeExit::RelayReservationAbandoned);
    }

    fn poll_reservation_abandoned(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        if self
            .reservation_waiter
            .as_mut()
            .is_some_and(|(_, response)| response.poll_canceled(cx).is_ready())
        {
            return std::task::Poll::Ready(());
        }
        if self
            .pending_dials
            .values_mut()
            .any(|pending| match pending {
                PendingDial::Reservation { response, .. } => response.poll_canceled(cx).is_ready(),
                _ => false,
            })
        {
            return std::task::Poll::Ready(());
        }
        std::task::Poll::Pending
    }
}

impl PendingDial {
    fn peer_id(&self) -> PeerId {
        match self {
            Self::Direct { peer_id, .. }
            | Self::Reservation { peer_id, .. }
            | Self::Circuit { peer_id, .. } => *peer_id,
        }
    }

    fn is_reservation(&self) -> bool {
        matches!(self, Self::Reservation { .. })
    }

    fn fail_dial(self, reason: String) {
        match self {
            Self::Direct { responses, .. } => {
                for response in responses {
                    let _ = response.send(Err(Error::Dial(reason.clone())));
                }
            }
            Self::Circuit { response, .. } => {
                let _ = response.send(Err(Error::Dial(reason)));
            }
            Self::Reservation { response, .. } => {
                let _ = response.send(Err(Error::Dial(reason)));
            }
        }
    }

    fn fail_unexpected_peer(self, actual: PeerId) {
        let expected = self.peer_id().to_string();
        let actual = actual.to_string();
        match self {
            Self::Direct { responses, .. } => {
                for response in responses {
                    let _ = response.send(Err(Error::UnexpectedRemotePeer {
                        expected: expected.clone(),
                        actual: actual.clone(),
                    }));
                }
            }
            Self::Circuit { response, .. } => {
                let _ = response.send(Err(Error::UnexpectedRemotePeer { expected, actual }));
            }
            Self::Reservation { response, .. } => {
                let _ = response.send(Err(Error::UnexpectedRemotePeer { expected, actual }));
            }
        }
    }
}

enum RuntimeInput {
    Swarm(SwarmEvent<BrowserBehaviourEvent>),
    Command(Command),
    CommandsClosed,
    SwarmEnded,
    ReservationAbandoned,
}

enum BrowserSwarmExit {
    Shutdown(oneshot::Sender<Result<()>>),
    Terminal(BrowserNodeExit),
    OwnersDropped,
    SwarmEnded,
}

async fn run_swarm(
    mut swarm: Swarm<BrowserBehaviour>,
    commands: Receiver<Command>,
) -> BrowserSwarmExit {
    let local_peer_id = *swarm.local_peer_id();
    let mut runtime = BrowserRuntime::new(local_peer_id);
    let mut commands = Box::pin(commands);
    let mut commands_first = true;

    loop {
        let input = poll_fn(|cx| {
            if runtime.poll_reservation_abandoned(cx).is_ready() {
                return std::task::Poll::Ready(RuntimeInput::ReservationAbandoned);
            }

            if commands_first {
                if let std::task::Poll::Ready(command) = commands.as_mut().poll_next(cx) {
                    return std::task::Poll::Ready(match command {
                        Some(command) => RuntimeInput::Command(command),
                        None => RuntimeInput::CommandsClosed,
                    });
                }
                if let std::task::Poll::Ready(event) = Pin::new(&mut swarm).poll_next(cx) {
                    return std::task::Poll::Ready(match event {
                        Some(event) => RuntimeInput::Swarm(event),
                        None => RuntimeInput::SwarmEnded,
                    });
                }
            } else {
                if let std::task::Poll::Ready(event) = Pin::new(&mut swarm).poll_next(cx) {
                    return std::task::Poll::Ready(match event {
                        Some(event) => RuntimeInput::Swarm(event),
                        None => RuntimeInput::SwarmEnded,
                    });
                }
                if let std::task::Poll::Ready(command) = commands.as_mut().poll_next(cx) {
                    return std::task::Poll::Ready(match command {
                        Some(command) => RuntimeInput::Command(command),
                        None => RuntimeInput::CommandsClosed,
                    });
                }
            }
            std::task::Poll::Pending
        })
        .await;
        commands_first = !commands_first;

        match input {
            RuntimeInput::Swarm(event) => runtime.handle_swarm_event(&mut swarm, event),
            RuntimeInput::Command(Command::Shutdown { response }) => {
                return BrowserSwarmExit::Shutdown(response);
            }
            RuntimeInput::Command(command) => {
                debug_assert!(!runtime.handle_command(&mut swarm, command));
            }
            RuntimeInput::ReservationAbandoned => {
                runtime.mark_reservation_abandoned();
            }
            RuntimeInput::CommandsClosed => return BrowserSwarmExit::OwnersDropped,
            RuntimeInput::SwarmEnded => return BrowserSwarmExit::SwarmEnded,
        }
        if let Some(exit) = runtime.terminal_exit.take() {
            return BrowserSwarmExit::Terminal(exit);
        }
    }
}

async fn pump_incoming_streams(
    mut incoming: IncomingStreams,
    pending: Sender<(PeerId, Stream)>,
    results: Sender<Result<AuthenticatedStream>>,
) {
    loop {
        let stream = incoming.next().fuse();
        let results_closed = results.closed().fuse();
        futures::pin_mut!(stream, results_closed);
        let next = futures::select! {
            stream = stream => stream,
            () = results_closed => break,
        };
        match next {
            Some(stream) => {
                let send = pending.send(stream).fuse();
                let results_closed = results.closed().fuse();
                futures::pin_mut!(send, results_closed);
                let sent = futures::select! {
                    result = send => result.is_ok(),
                    () = results_closed => false,
                };
                if !sent {
                    break;
                }
            }
            None => break,
        }
    }
}

async fn authenticate_incoming_streams(
    pending: Receiver<(PeerId, Stream)>,
    results: Sender<Result<AuthenticatedStream>>,
    local_peer_id: PeerId,
    authority: BrowserAuthority,
    requirements: SessionRequirements,
) {
    loop {
        let stream = pending.recv().fuse();
        let results_closed = results.closed().fuse();
        futures::pin_mut!(stream, results_closed);
        let next = futures::select! {
            stream = stream => stream,
            () = results_closed => break,
        };
        let Ok((remote_peer_id, stream)) = next else {
            break;
        };
        let result = authenticate_stream(
            stream,
            local_peer_id,
            remote_peer_id,
            &authority,
            &requirements,
        )
        .await;
        if results.send(result).await.is_err() {
            break;
        }
    }
}

async fn authenticate_stream(
    stream: Stream,
    local_peer_id: PeerId,
    remote_peer_id: PeerId,
    authority: &BrowserAuthority,
    requirements: &SessionRequirements,
) -> Result<AuthenticatedStream> {
    let tokens = authority.tokens();
    let verifier = authority.verifier();
    let (stream, remote) = authenticate_duplex(
        stream,
        local_peer_id,
        remote_peer_id,
        &tokens,
        &verifier,
        requirements,
    )
    .await?;
    Ok(AuthenticatedStream::new(stream, remote))
}

fn build_swarm(
    identity: libp2p::identity::Keypair,
    streams: StreamBehaviour,
    targeted_streams: TargetedStreamBehaviour,
) -> std::result::Result<Swarm<BrowserBehaviour>, Box<dyn StdError + Send + Sync>> {
    Ok(SwarmBuilder::with_existing_identity(identity)
        .with_wasm_bindgen()
        .with_other_transport(|identity| {
            websocket_websys::Transport::default()
                .upgrade(upgrade::Version::V1Lazy)
                .authenticate(
                    noise::Config::new(identity).expect("Ed25519 identity supports Noise"),
                )
                .multiplex(yamux::Config::default())
        })?
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|_identity, relay| BrowserBehaviour {
            relay: relay_client::Behaviour::new(relay),
            streams,
            targeted_streams,
        })?
        .with_swarm_config(|config| config.with_idle_connection_timeout(Duration::from_secs(60)))
        .build())
}
