use std::{
    collections::{HashMap, HashSet, VecDeque},
    error::Error as StdError,
    future::Future,
    panic::AssertUnwindSafe,
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use futures::{FutureExt, StreamExt};
#[cfg(target_os = "ios")]
use libp2p::core::{upgrade::Version, Transport as _};
use libp2p::{
    core::transport::{ListenerId, TransportError},
    multiaddr::Protocol,
    noise, relay,
    swarm::{
        dial_opts::{DialOpts, PeerCondition},
        ConnectionId, DialError, NetworkBehaviour, SwarmEvent,
    },
    tcp, yamux, Multiaddr, PeerId, Stream, Swarm, SwarmBuilder,
};
use libp2p_stream::{Behaviour as StreamBehaviour, IncomingStreams};
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex as AsyncMutex};
use uuid::Uuid;

#[cfg(target_os = "ios")]
use crate::system_dns::SystemDnsTransport;
use crate::{
    authentication::{authenticate_duplex, SessionRequirements},
    authority::DomainAuthority,
    observation::{NodeFailure, NodeObservations},
    relay::{
        canonicalize_provider_base, ObservedRelayLimits, RelayCancellation,
        RelayConfirmationRejection, RelayProvider, RelayReservationEvent, RelayReservationHandle,
        RelayReservationNode, RelayReservationSnapshot,
    },
    relay_client,
    relay_dial_gate::{RelayCircuitDialGate, RelayCircuitDialPermit},
    source_admission,
    targeted_stream::{TargetedStreamBehaviour, TargetedStreamControl},
    token::{
        ensure_token_peer, unix_time_now, DdsTokenVerifier, DdsVerificationKeys, P2PAccessClaims,
        SignedP2pCredential, TokenStore,
    },
    ApplicationProtocol, AuthenticatedStream, Error, Identity, Result as P2PResult,
};

#[derive(NetworkBehaviour)]
struct Behaviour {
    relay: relay_client::Behaviour,
    streams: StreamBehaviour,
    targeted_streams: TargetedStreamBehaviour,
}

/// A circuit connection selected by one explicit relay route.
///
/// Its private process nonce and public libp2p connection ID prevent a later
/// application open from silently falling back to a direct or sibling-relay
/// connection to the same target.
#[derive(Clone, Debug)]
pub struct RelayRouteHandle {
    node_instance_id: Uuid,
    relay_peer_id: PeerId,
    target_peer_id: PeerId,
    connection_id: ConnectionId,
    admission_expires_at: chrono::DateTime<Utc>,
    route: Multiaddr,
}

impl RelayRouteHandle {
    pub fn relay_peer_id(&self) -> PeerId {
        self.relay_peer_id
    }

    pub fn target_peer_id(&self) -> PeerId {
        self.target_peer_id
    }

    pub fn route(&self) -> &Multiaddr {
        &self.route
    }

    pub fn admission_expires_at(&self) -> chrono::DateTime<Utc> {
        self.admission_expires_at
    }
}

/// Bounded host-facing reservation lifecycle events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayTransportEvent {
    Publishable(RelayReservationSnapshot),
    Renewed(RelayReservationSnapshot),
    Unpublished {
        handle: RelayReservationHandle,
    },
    ConfirmationRejected {
        handle: RelayReservationHandle,
        reason: RelayConfirmationRejection,
    },
    Canceled {
        handle: RelayReservationHandle,
    },
}

pub struct IncomingAuthenticatedStreams {
    inner: IncomingStreams,
    local_peer_id: PeerId,
    tokens: TokenStore,
    verifier: DdsTokenVerifier,
    requirements: SessionRequirements,
    observations: NodeObservations,
}

impl IncomingAuthenticatedStreams {
    pub async fn accept(&mut self) -> Option<P2PResult<AuthenticatedStream>> {
        let (remote_peer_id, stream) = self.inner.next().await?;
        let result = authenticate(
            stream,
            self.local_peer_id,
            remote_peer_id,
            &self.tokens,
            &self.verifier,
            &self.requirements,
        )
        .await;
        if let Ok(stream) = &result {
            self.observations
                .authenticated(self.requirements.domain_id(), stream.remote_peer().clone());
        }
        Some(result)
    }
}

pub(crate) enum CurrentCredentialStatus {
    Missing,
    Expired,
    Verified(Box<P2PAccessClaims>),
}

#[derive(Clone, Debug)]
struct DirectListenState {
    expected: usize,
    bound_listener_ids: HashSet<ListenerId>,
    addresses: Vec<Multiaddr>,
    failure: Option<DirectListenFailure>,
}

#[derive(Clone, Debug)]
struct DirectListenFailure {
    address: String,
    reason: String,
}

#[derive(Clone)]
pub struct Node {
    node_instance_id: Uuid,
    identity: Identity,
    control: libp2p_stream::Control,
    targeted_control: TargetedStreamControl,
    tokens: TokenStore,
    verifier: DdsTokenVerifier,
    authority_updates: Arc<AsyncMutex<()>>,
    command_sender: mpsc::Sender<Command>,
    direct_listeners: watch::Receiver<DirectListenState>,
    relay_events: broadcast::Sender<RelayTransportEvent>,
    observations: NodeObservations,
    source_admissions: Arc<source_admission::AdmissionCache>,
    relay_circuit_dials: RelayCircuitDialGate,
    task: Arc<NodeTaskOwnership>,
}

impl Node {
    pub fn start(
        identity: Identity,
        verifier: DdsTokenVerifier,
        listen_addresses: impl IntoIterator<Item = Multiaddr>,
    ) -> P2PResult<Self> {
        let stream_behaviour = StreamBehaviour::new();
        let control = stream_behaviour.new_control();
        let targeted_stream_behaviour = TargetedStreamBehaviour::new();
        let targeted_control = targeted_stream_behaviour.new_control();
        let swarm = build_swarm(
            identity.keypair(),
            stream_behaviour,
            targeted_stream_behaviour,
        )?;
        Self::start_swarm(
            identity,
            verifier,
            listen_addresses,
            control,
            targeted_control,
            swarm,
        )
    }

    /// Construct with an explicitly supplied DNS resolver configuration.
    ///
    /// Production callers normally use [`Node::start`]. This seam exists so
    /// deterministic tests can exercise production-shaped public `dns4`
    /// addresses without mutating process-global resolver state.
    pub fn start_with_dns_config(
        identity: Identity,
        verifier: DdsTokenVerifier,
        listen_addresses: impl IntoIterator<Item = Multiaddr>,
        resolver_config: libp2p::dns::ResolverConfig,
        resolver_options: libp2p::dns::ResolverOpts,
    ) -> P2PResult<Self> {
        let stream_behaviour = StreamBehaviour::new();
        let control = stream_behaviour.new_control();
        let targeted_stream_behaviour = TargetedStreamBehaviour::new();
        let targeted_control = targeted_stream_behaviour.new_control();
        let swarm = build_swarm_with_dns_config(
            identity.keypair(),
            stream_behaviour,
            targeted_stream_behaviour,
            resolver_config,
            resolver_options,
        )?;
        Self::start_swarm(
            identity,
            verifier,
            listen_addresses,
            control,
            targeted_control,
            swarm,
        )
    }

    fn start_swarm(
        identity: Identity,
        verifier: DdsTokenVerifier,
        listen_addresses: impl IntoIterator<Item = Multiaddr>,
        control: libp2p_stream::Control,
        targeted_control: TargetedStreamControl,
        mut swarm: Swarm<Behaviour>,
    ) -> P2PResult<Self> {
        let mut direct_listener_addresses = HashMap::new();
        for address in listen_addresses {
            let listener_id = swarm
                .listen_on(address.clone())
                .map_err(|error| Error::Listen {
                    address: address.to_string(),
                    reason: error.to_string(),
                })?;
            direct_listener_addresses.insert(listener_id, address);
        }

        let (command_sender, command_receiver) = mpsc::channel(16);
        let (listen_sender, listen_receiver) = watch::channel(DirectListenState {
            expected: direct_listener_addresses.len(),
            bound_listener_ids: HashSet::new(),
            addresses: Vec::new(),
            failure: None,
        });
        let (relay_events, _) = broadcast::channel(128);
        let observations = NodeObservations::new();
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| Error::RuntimeUnavailable)?;
        let run_observations = observations.clone();
        let expiry_observations = observations.clone();
        let run_relay_events = relay_events.clone();
        let node_task = async move {
            tokio::select! {
                exit = run_swarm(
                    swarm,
                    command_receiver,
                    listen_sender,
                    direct_listener_addresses,
                    run_relay_events,
                    run_observations,
                ) => exit,
                _ = expiry_observations.drive_expiry() => SwarmExit::ExpiryDriverEnded,
            }
        };
        let task_guard = NodeTaskGuard::new(observations.clone());
        let (task_completed, _) = watch::channel(false);
        let completion_guard = NodeTaskCompletionGuard(task_completed.clone());
        let task_observations = observations.clone();
        let task = runtime.spawn(async move {
            let _completion_guard = completion_guard;
            supervise_node_task(node_task, task_observations, task_guard).await;
        });
        let task = Arc::new(NodeTaskOwnership {
            task: AsyncMutex::new(Some(task)),
            completed: task_completed,
        });

        Ok(Self {
            node_instance_id: Uuid::new_v4(),
            identity,
            control,
            targeted_control,
            tokens: TokenStore::default(),
            verifier,
            authority_updates: Arc::new(AsyncMutex::new(())),
            command_sender,
            direct_listeners: listen_receiver,
            relay_events,
            observations,
            source_admissions: Arc::new(source_admission::AdmissionCache::default()),
            relay_circuit_dials: RelayCircuitDialGate::new(),
            task,
        })
    }

    pub fn peer_id(&self) -> PeerId {
        self.identity.peer_id()
    }

    pub fn authority(&self) -> DomainAuthority {
        DomainAuthority::new(self.clone())
    }

    pub fn observations(&self) -> NodeObservations {
        self.observations.clone()
    }

    pub(crate) async fn install_verification_keys(
        &self,
        keys: DdsVerificationKeys,
    ) -> P2PResult<()> {
        let _authority_update = self.authority_updates.lock().await;
        self.verifier.replace_keys(keys)
    }

    pub(crate) fn peer_public_key_protobuf(&self) -> Vec<u8> {
        self.identity.public_key_protobuf()
    }

    pub(crate) fn sign_peer_challenge(&self, challenge: &[u8]) -> P2PResult<Vec<u8>> {
        self.identity.sign_challenge(challenge)
    }

    pub(crate) async fn install_credential(
        &self,
        credential: SignedP2pCredential,
    ) -> P2PResult<P2PAccessClaims> {
        let _authority_update = self.authority_updates.lock().await;
        self.tokens
            .install(credential, &self.verifier, self.peer_id(), None, None)
            .await
    }

    pub(crate) async fn install_credential_for_domain(
        &self,
        credential: SignedP2pCredential,
        domain_id: Uuid,
    ) -> P2PResult<P2PAccessClaims> {
        let _authority_update = self.authority_updates.lock().await;
        self.tokens
            .install(
                credential,
                &self.verifier,
                self.peer_id(),
                None,
                Some(domain_id),
            )
            .await
    }

    pub(crate) async fn install_credential_checked(
        &self,
        credential: SignedP2pCredential,
        expected_expiration: u64,
    ) -> P2PResult<P2PAccessClaims> {
        let _authority_update = self.authority_updates.lock().await;
        self.tokens
            .install(
                credential,
                &self.verifier,
                self.peer_id(),
                Some(expected_expiration),
                None,
            )
            .await
    }

    pub(crate) async fn clear_credential(&self) {
        let _authority_update = self.authority_updates.lock().await;
        self.tokens.clear().await;
    }

    pub(crate) async fn current_credential_status(&self) -> P2PResult<CurrentCredentialStatus> {
        let _authority_update = self.authority_updates.lock().await;
        let Some((credential, installed_claims)) = self.tokens.snapshot_with_claims().await else {
            return Ok(CurrentCredentialStatus::Missing);
        };
        let verified = self.verifier.verify(credential.as_str());
        if installed_claims.exp <= unix_time_now() {
            return Ok(CurrentCredentialStatus::Expired);
        }
        let claims = verified?;
        ensure_token_peer(&claims, self.peer_id())?;
        if claims != installed_claims {
            return Err(Error::InvalidToken(
                "installed credential claims changed during verification".into(),
            ));
        }
        Ok(CurrentCredentialStatus::Verified(Box::new(claims)))
    }

    pub async fn first_listen_address(&self) -> P2PResult<Multiaddr> {
        let mut receiver = self.direct_listeners.clone();
        loop {
            let state = receiver.borrow().clone();
            if let Some(failure) = state.failure {
                return Err(Error::Listen {
                    address: failure.address,
                    reason: failure.reason,
                });
            }
            if let Some(address) = state.addresses.first().cloned() {
                return Ok(address);
            }
            receiver.changed().await.map_err(|_| Error::SwarmStopped)?;
        }
    }

    /// Wait until every configured direct listener has emitted at least one
    /// bound address. A zero-listener node is ready immediately. Any listener
    /// that closes before or after binding fails the node rather than leaving a
    /// partially reachable runtime reported as ready.
    pub async fn wait_for_listeners(&self) -> P2PResult<Vec<Multiaddr>> {
        let mut receiver = self.direct_listeners.clone();
        loop {
            let state = receiver.borrow().clone();
            if let Some(failure) = state.failure {
                return Err(Error::Listen {
                    address: failure.address,
                    reason: failure.reason,
                });
            }
            if state.bound_listener_ids.len() == state.expected {
                return Ok(state.addresses);
            }
            receiver.changed().await.map_err(|_| Error::SwarmStopped)?;
        }
    }

    pub fn subscribe_relay_events(&self) -> broadcast::Receiver<RelayTransportEvent> {
        self.relay_events.subscribe()
    }

    pub fn accept(
        &self,
        protocol: ApplicationProtocol,
        requirements: SessionRequirements,
    ) -> P2PResult<IncomingAuthenticatedStreams> {
        let mut control = self.control.clone();
        let inner = control
            .accept(protocol.stream_protocol())
            .map_err(|_| Error::ProtocolAlreadyRegistered)?;
        Ok(IncomingAuthenticatedStreams {
            inner,
            local_peer_id: self.peer_id(),
            tokens: self.tokens.clone(),
            verifier: self.verifier.clone(),
            requirements,
            observations: self.observations.clone(),
        })
    }

    pub async fn open(
        &self,
        remote_peer_id: PeerId,
        remote_addresses: Vec<Multiaddr>,
        protocol: ApplicationProtocol,
        requirements: SessionRequirements,
    ) -> P2PResult<AuthenticatedStream> {
        if remote_addresses.is_empty() {
            return Err(Error::MissingRemoteAddress);
        }
        if let Some(expected) = requirements.expected_remote_peer_id() {
            if expected != remote_peer_id {
                return Err(Error::UnexpectedRemotePeer {
                    expected: expected.to_string(),
                    actual: remote_peer_id.to_string(),
                });
            }
        }

        let addresses = remote_addresses
            .into_iter()
            .map(|address| normalize_remote_address(address, remote_peer_id, false))
            .collect::<P2PResult<Vec<_>>>()?;
        let connection_id = self.connect_exact(remote_peer_id, addresses, None).await?;
        let stream = self
            .targeted_control
            .open_stream(remote_peer_id, connection_id, protocol.stream_protocol())
            .await
            .map_err(Error::from)?;
        let stream = authenticate(
            stream,
            self.peer_id(),
            remote_peer_id,
            &self.tokens,
            &self.verifier,
            &requirements,
        )
        .await?;
        self.observations
            .authenticated(requirements.domain_id(), stream.remote_peer().clone());
        Ok(stream)
    }

    /// Start one relay reservation generation after DMS has supplied a ready
    /// provider snapshot. This operation sends no Domain or P2P token.
    pub async fn start_relay_reservation(
        &self,
        provider: RelayProvider,
    ) -> P2PResult<RelayReservationHandle> {
        let relay_peer_id = provider.relay_peer_id();
        let direct_address =
            normalize_remote_address(provider.selected_base().clone(), relay_peer_id, false)?;
        let direct_connection = self
            .select_relay_connection(relay_peer_id, direct_address)
            .await?;
        let (response, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::BeginReservation {
                provider,
                direct_connection,
                response,
            })
            .await
            .map_err(|_| Error::SwarmStopped)?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    /// Wait until acceptance and listener evidence make the selected trusted
    /// DMS base publishable. A caller that abandons this wait still owns the
    /// handle and must cancel it explicitly.
    pub async fn wait_relay_reservation(
        &self,
        handle: RelayReservationHandle,
    ) -> P2PResult<RelayReservationSnapshot> {
        let (response, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::WaitReservation { handle, response })
            .await
            .map_err(|_| Error::SwarmStopped)?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    pub async fn relay_reservation(
        &self,
        handle: RelayReservationHandle,
    ) -> P2PResult<RelayReservationSnapshot> {
        let (response, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::ReservationSnapshot { handle, response })
            .await
            .map_err(|_| Error::SwarmStopped)?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    /// Tombstone one exact generation, unpublish it, remove its listener, and
    /// wait for its direct relay connection and listener to close.
    pub async fn cancel_relay_reservation(&self, handle: RelayReservationHandle) -> P2PResult<()> {
        let (response, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::CancelReservation { handle, response })
            .await
            .map_err(|_| Error::SwarmStopped)?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    /// Directly authenticate to the requested relay, perform the typed source
    /// CONNECT preflight there, and dial the exact full circuit route.
    pub async fn connect_relayed(
        &self,
        route: Multiaddr,
        requirements: &SessionRequirements,
    ) -> P2PResult<RelayRouteHandle> {
        self.connect_relayed_inner(route, requirements, false).await
    }

    pub(crate) async fn connect_relayed_reusing_admission(
        &self,
        route: Multiaddr,
        requirements: &SessionRequirements,
    ) -> P2PResult<RelayRouteHandle> {
        self.connect_relayed_inner(route, requirements, true).await
    }

    async fn connect_relayed_inner(
        &self,
        route: Multiaddr,
        requirements: &SessionRequirements,
        reuse_admission: bool,
    ) -> P2PResult<RelayRouteHandle> {
        let parsed = parse_relay_route(&route)?;
        let expected = requirements
            .expected_remote_peer_id()
            .ok_or(Error::MissingExpectedRelayTarget)?;
        if expected != parsed.target_peer_id {
            return Err(Error::UnexpectedRemotePeer {
                expected: expected.to_string(),
                actual: parsed.target_peer_id.to_string(),
            });
        }

        let relay_connection = self
            .select_relay_connection(parsed.relay_peer_id, parsed.direct_relay_address)
            .await?;
        let domain_id = requirements.domain_id();
        let admission = if reuse_admission {
            self.authorize_relay_source_reusing(
                parsed.relay_peer_id,
                relay_connection,
                parsed.target_peer_id,
                domain_id,
            )
            .await?
        } else {
            source_admission::CachedAdmission::uncached(
                self.authorize_relay_source(
                    parsed.relay_peer_id,
                    relay_connection,
                    parsed.target_peer_id,
                    domain_id,
                )
                .await?,
            )
        };

        let circuit_address = normalize_remote_address(route.clone(), parsed.target_peer_id, true)?;
        let first_connection = self
            .connect_exact(
                parsed.target_peer_id,
                vec![circuit_address.clone()],
                Some(parsed.relay_peer_id),
            )
            .await;
        let connection_id = match first_connection {
            Ok(connection_id) => connection_id,
            Err(error) => {
                if reuse_admission {
                    self.source_admissions
                        .invalidate(
                            parsed.relay_peer_id,
                            relay_connection,
                            parsed.target_peer_id,
                            domain_id,
                            &admission,
                        )
                        .await;
                }
                return Err(error);
            }
        };
        let admission_expires_at = admission.expires_at;
        Ok(RelayRouteHandle {
            node_instance_id: self.node_instance_id,
            relay_peer_id: parsed.relay_peer_id,
            target_peer_id: parsed.target_peer_id,
            connection_id,
            admission_expires_at,
            route,
        })
    }

    /// Open the application protocol only on the circuit connection captured
    /// by `route`, then apply the existing end-to-end mutual DDS auth.
    pub async fn open_relayed(
        &self,
        route: &RelayRouteHandle,
        protocol: ApplicationProtocol,
        requirements: SessionRequirements,
    ) -> P2PResult<AuthenticatedStream> {
        if route.node_instance_id != self.node_instance_id {
            return Err(Error::ForeignRelayRoute);
        }
        let expected = requirements
            .expected_remote_peer_id()
            .ok_or(Error::MissingExpectedRelayTarget)?;
        if expected != route.target_peer_id {
            return Err(Error::UnexpectedRemotePeer {
                expected: expected.to_string(),
                actual: route.target_peer_id.to_string(),
            });
        }
        let stream = self
            .targeted_control
            .open_stream(
                route.target_peer_id,
                route.connection_id,
                protocol.stream_protocol(),
            )
            .await?;
        let stream = authenticate(
            stream,
            self.peer_id(),
            route.target_peer_id,
            &self.tokens,
            &self.verifier,
            &requirements,
        )
        .await?;
        self.observations
            .authenticated(requirements.domain_id(), stream.remote_peer().clone());
        Ok(stream)
    }

    /// Close only the circuit represented by this handle. The possibly shared
    /// direct relay connection and unrelated target paths remain established.
    pub async fn close_relay_route(&self, route: &RelayRouteHandle) -> P2PResult<()> {
        if route.node_instance_id != self.node_instance_id {
            return Err(Error::ForeignRelayRoute);
        }
        self.close_connection(route.connection_id).await
    }

    pub async fn disconnect(&self, peer_id: PeerId) -> P2PResult<()> {
        self.send_unit_command(|response| Command::Disconnect { peer_id, response })
            .await
    }

    pub async fn shutdown(&self) -> P2PResult<()> {
        let result = self
            .send_unit_command(|response| Command::Shutdown { response })
            .await;
        self.finish_node_task(false).await;
        result
    }

    /// Abort the node task and wait until its swarm and listeners have been
    /// dropped.
    ///
    /// Normal owners should call [`Node::shutdown`]. This forced barrier exists
    /// for a bounded outer lifecycle that has already exhausted its graceful
    /// cleanup deadline. Unlike dropping the command sender, it does not leave
    /// the node task detached.
    pub async fn shutdown_now(&self) {
        self.finish_node_task(true).await;
    }

    async fn finish_node_task(&self, abort: bool) {
        let mut owned = self.task.task.lock().await;
        if let Some(task) = owned.as_mut() {
            if abort {
                task.abort();
            }
            let _ = task.await;
            owned.take();
            return;
        }
        drop(owned);

        let mut completed = self.task.completed.subscribe();
        while !*completed.borrow_and_update() && completed.changed().await.is_ok() {}
    }

    async fn connect_exact(
        &self,
        peer_id: PeerId,
        addresses: Vec<Multiaddr>,
        circuit_relay_peer_id: Option<PeerId>,
    ) -> P2PResult<ConnectionId> {
        let circuit_permit = match circuit_relay_peer_id {
            Some(_) => Some(self.relay_circuit_dials.acquire().await),
            None => None,
        };
        let (response, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::Connect {
                peer_id,
                addresses,
                circuit_relay_peer_id,
                circuit_permit,
                response,
            })
            .await
            .map_err(|_| Error::SwarmStopped)?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    async fn select_relay_connection(
        &self,
        peer_id: PeerId,
        address: Multiaddr,
    ) -> P2PResult<ConnectionId> {
        let (response, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SelectRelayConnection {
                peer_id,
                address,
                response,
            })
            .await
            .map_err(|_| Error::SwarmStopped)?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    async fn close_connection(&self, connection_id: ConnectionId) -> P2PResult<()> {
        let (response, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::CloseConnection {
                connection_id,
                response,
            })
            .await
            .map_err(|_| Error::SwarmStopped)?;
        receiver.await.map_err(|_| Error::SwarmStopped)?
    }

    async fn authorize_relay_source(
        &self,
        relay_peer_id: PeerId,
        relay_connection: ConnectionId,
        target_peer_id: PeerId,
        domain_id: Uuid,
    ) -> P2PResult<chrono::DateTime<Utc>> {
        let authorization = source_admission::prepare_authorization(
            self.peer_id(),
            target_peer_id,
            domain_id,
            &self.tokens,
            &self.verifier,
        )
        .await?;
        self.authorize_relay_source_prepared(relay_peer_id, relay_connection, authorization)
            .await
    }

    async fn authorize_relay_source_reusing(
        &self,
        relay_peer_id: PeerId,
        relay_connection: ConnectionId,
        target_peer_id: PeerId,
        domain_id: Uuid,
    ) -> P2PResult<source_admission::CachedAdmission> {
        // Validate the currently installed credential on every connection
        // attempt. Only the relay's already-proven, shorter authority window
        // is reused.
        let authorization = source_admission::prepare_authorization(
            self.peer_id(),
            target_peer_id,
            domain_id,
            &self.tokens,
            &self.verifier,
        )
        .await?;
        let issued_at = authorization.issued_at();
        self.source_admissions
            .authorize(
                relay_peer_id,
                relay_connection,
                target_peer_id,
                domain_id,
                issued_at,
                || {
                    self.authorize_relay_source_prepared(
                        relay_peer_id,
                        relay_connection,
                        authorization,
                    )
                },
            )
            .await
    }

    async fn authorize_relay_source_prepared(
        &self,
        relay_peer_id: PeerId,
        relay_connection: ConnectionId,
        authorization: source_admission::PreparedAuthorization,
    ) -> P2PResult<chrono::DateTime<Utc>> {
        let mut stream = self
            .targeted_control
            .open_stream(relay_peer_id, relay_connection, source_admission::PROTOCOL)
            .await?;
        source_admission::authorize_prepared(&mut stream, authorization, Utc::now).await
    }

    async fn send_unit_command(
        &self,
        build: impl FnOnce(oneshot::Sender<P2PResult<()>>) -> Command,
    ) -> P2PResult<()> {
        let (response_sender, response_receiver) = oneshot::channel();
        self.command_sender
            .send(build(response_sender))
            .await
            .map_err(|_| Error::SwarmStopped)?;
        response_receiver.await.map_err(|_| Error::SwarmStopped)?
    }
}

enum Command {
    Connect {
        peer_id: PeerId,
        addresses: Vec<Multiaddr>,
        circuit_relay_peer_id: Option<PeerId>,
        circuit_permit: Option<RelayCircuitDialPermit>,
        response: oneshot::Sender<P2PResult<ConnectionId>>,
    },
    SelectRelayConnection {
        peer_id: PeerId,
        address: Multiaddr,
        response: oneshot::Sender<P2PResult<ConnectionId>>,
    },
    CloseConnection {
        connection_id: ConnectionId,
        response: oneshot::Sender<P2PResult<()>>,
    },
    BeginReservation {
        provider: RelayProvider,
        direct_connection: ConnectionId,
        response: oneshot::Sender<P2PResult<RelayReservationHandle>>,
    },
    WaitReservation {
        handle: RelayReservationHandle,
        response: oneshot::Sender<P2PResult<RelayReservationSnapshot>>,
    },
    ReservationSnapshot {
        handle: RelayReservationHandle,
        response: oneshot::Sender<P2PResult<RelayReservationSnapshot>>,
    },
    CancelReservation {
        handle: RelayReservationHandle,
        response: oneshot::Sender<P2PResult<()>>,
    },
    Disconnect {
        peer_id: PeerId,
        response: oneshot::Sender<P2PResult<()>>,
    },
    Shutdown {
        response: oneshot::Sender<P2PResult<()>>,
    },
}

#[cfg(not(target_os = "ios"))]
fn build_swarm(
    identity: libp2p::identity::Keypair,
    streams: StreamBehaviour,
    targeted_streams: TargetedStreamBehaviour,
) -> P2PResult<Swarm<Behaviour>> {
    SwarmBuilder::with_existing_identity(identity)
        .with_tokio()
        .with_tcp(
            tcp::Config::default().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|error| Error::TransportBuild(error.to_string()))?
        .with_dns()
        .map_err(|error| Error::TransportBuild(error.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|error| Error::TransportBuild(error.to_string()))?
        .with_behaviour(|_, relay| Behaviour {
            relay: relay_client::Behaviour::new(relay),
            streams,
            targeted_streams,
        })
        .map_err(|error| Error::TransportBuild(error.to_string()))
        .map(|builder| {
            builder
                .with_swarm_config(|config| {
                    config.with_idle_connection_timeout(Duration::from_secs(60))
                })
                .build()
        })
}

#[cfg(target_os = "ios")]
fn build_swarm(
    identity: libp2p::identity::Keypair,
    streams: StreamBehaviour,
    targeted_streams: TargetedStreamBehaviour,
) -> P2PResult<Swarm<Behaviour>> {
    SwarmBuilder::with_existing_identity(identity)
        .with_tokio()
        .with_other_transport(|keypair| -> Result<_, Box<dyn StdError + Send + Sync>> {
            let security = noise::Config::new(keypair)?;
            let transport = tcp::tokio::Transport::new(tcp::Config::default().nodelay(true))
                .upgrade(Version::V1Lazy)
                .authenticate(security)
                .multiplex(yamux::Config::default());
            Ok(SystemDnsTransport::new(transport))
        })
        .map_err(|error| Error::TransportBuild(error.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|error| Error::TransportBuild(error.to_string()))?
        .with_behaviour(|_, relay| Behaviour {
            relay: relay_client::Behaviour::new(relay),
            streams,
            targeted_streams,
        })
        .map_err(|error| Error::TransportBuild(error.to_string()))
        .map(|builder| {
            builder
                .with_swarm_config(|config| {
                    config.with_idle_connection_timeout(Duration::from_secs(60))
                })
                .build()
        })
}

fn build_swarm_with_dns_config(
    identity: libp2p::identity::Keypair,
    streams: StreamBehaviour,
    targeted_streams: TargetedStreamBehaviour,
    resolver_config: libp2p::dns::ResolverConfig,
    resolver_options: libp2p::dns::ResolverOpts,
) -> P2PResult<Swarm<Behaviour>> {
    SwarmBuilder::with_existing_identity(identity)
        .with_tokio()
        .with_tcp(
            tcp::Config::default().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|error| Error::TransportBuild(error.to_string()))?
        .with_dns_config(resolver_config, resolver_options)
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|error| Error::TransportBuild(error.to_string()))?
        .with_behaviour(|_, relay| Behaviour {
            relay: relay_client::Behaviour::new(relay),
            streams,
            targeted_streams,
        })
        .map_err(|error| Error::TransportBuild(error.to_string()))
        .map(|builder| {
            builder
                .with_swarm_config(|config| {
                    config.with_idle_connection_timeout(Duration::from_secs(60))
                })
                .build()
        })
}

struct ReservationRuntime {
    state: RelayReservationNode,
    events: broadcast::Sender<RelayTransportEvent>,
    direct_connections: HashMap<PeerId, Vec<DirectConnection>>,
    pending_direct_dials: HashMap<PeerId, HashSet<ConnectionId>>,
    cancellation_barriers: HashMap<PeerId, CancellationBarrier>,
    terminal_rejections: HashMap<RelayReservationHandle, RelayConfirmationRejection>,
    terminal_closures: HashMap<RelayReservationHandle, String>,
    pending_starts:
        HashMap<RelayReservationHandle, oneshot::Sender<P2PResult<RelayReservationHandle>>>,
    confirmation_waiters:
        HashMap<RelayReservationHandle, Vec<oneshot::Sender<P2PResult<RelayReservationSnapshot>>>>,
    cancellation_waiters: HashMap<RelayReservationHandle, Vec<oneshot::Sender<P2PResult<()>>>>,
}

struct CancellationBarrier {
    handle: RelayReservationHandle,
    pending_connections: HashSet<ConnectionId>,
    state_canceled: bool,
    dispatch_terminal: bool,
}

#[derive(Clone)]
struct DirectConnection {
    connection_id: ConnectionId,
    requested_address: Option<Multiaddr>,
}

impl ReservationRuntime {
    fn new(local_peer_id: PeerId, events: broadcast::Sender<RelayTransportEvent>) -> Self {
        Self {
            state: RelayReservationNode::new(local_peer_id),
            events,
            direct_connections: HashMap::new(),
            pending_direct_dials: HashMap::new(),
            cancellation_barriers: HashMap::new(),
            terminal_rejections: HashMap::new(),
            terminal_closures: HashMap::new(),
            pending_starts: HashMap::new(),
            confirmation_waiters: HashMap::new(),
            cancellation_waiters: HashMap::new(),
        }
    }

    fn record_direct_connection(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        requested_address: Option<Multiaddr>,
    ) {
        self.record_pending_dial_finished(peer_id, connection_id);
        let connections = self.direct_connections.entry(peer_id).or_default();
        if !connections
            .iter()
            .any(|connection| connection.connection_id == connection_id)
        {
            connections.push(DirectConnection {
                connection_id,
                requested_address,
            });
        }
    }

    fn record_pending_dial_finished(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        if let Some(pending) = self.pending_direct_dials.get_mut(&peer_id) {
            pending.remove(&connection_id);
            if pending.is_empty() {
                self.pending_direct_dials.remove(&peer_id);
            }
        }
    }

    fn record_pending_direct_dial(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        self.pending_direct_dials
            .entry(peer_id)
            .or_default()
            .insert(connection_id);
    }

    fn record_direct_dial_failed(&mut self, peer_id: PeerId, connection_id: ConnectionId) {
        if let Some(pending) = self.pending_direct_dials.get_mut(&peer_id) {
            pending.remove(&connection_id);
            if pending.is_empty() {
                self.pending_direct_dials.remove(&peer_id);
            }
        }
        if let Some(barrier) = self.cancellation_barriers.get_mut(&peer_id) {
            barrier.pending_connections.remove(&connection_id);
        }
        self.maybe_finish_cancellation(peer_id);
    }

    fn selected_direct_connection(
        &self,
        peer_id: PeerId,
        expected_address: &Multiaddr,
    ) -> P2PResult<Option<ConnectionId>> {
        let Some(connection) = self
            .direct_connections
            .get(&peer_id)
            .and_then(|connections| connections.first())
        else {
            return Ok(None);
        };
        if connection.requested_address.as_ref() == Some(expected_address) {
            return Ok(Some(connection.connection_id));
        }
        Err(Error::RelayDirectConnectionMismatch {
            relay_peer_id: peer_id.to_string(),
            expected: expected_address.to_string(),
            actual: connection
                .requested_address
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "an inbound or untracked direct connection".to_string()),
        })
    }

    fn record_connection_closed(
        &mut self,
        swarm: &mut Swarm<Behaviour>,
        peer_id: PeerId,
        connection_id: ConnectionId,
    ) {
        if let Some(connections) = self.direct_connections.get_mut(&peer_id) {
            if let Some(position) = connections
                .iter()
                .position(|candidate| candidate.connection_id == connection_id)
            {
                connections.remove(position);
            }
            if connections.is_empty() {
                self.direct_connections.remove(&peer_id);
            }
        }
        if let Some(barrier) = self.cancellation_barriers.get_mut(&peer_id) {
            barrier.pending_connections.remove(&connection_id);
        }
        if let Some(handle) = self.state.handle_for_relay(peer_id) {
            let selected = self
                .state
                .snapshot(handle)
                .ok()
                .and_then(|snapshot| snapshot.direct_connection());
            if selected == Some(connection_id) {
                self.close_before_confirmation(
                    handle,
                    "selected direct relay connection closed".to_string(),
                );
                if let Ok(event) = self
                    .state
                    .observe_direct_connection_closed(handle, connection_id)
                {
                    self.apply_event(swarm, event);
                }
            }
        }
        self.maybe_finish_cancellation(peer_id);
    }

    fn apply_event(&mut self, swarm: &mut Swarm<Behaviour>, event: RelayReservationEvent) {
        let mut events = VecDeque::from([event]);
        while let Some(event) = events.pop_front() {
            match event {
                RelayReservationEvent::EvidenceRecorded { .. }
                | RelayReservationEvent::Fenced { .. } => {}
                RelayReservationEvent::Renewed { handle, .. } => {
                    if let Ok(snapshot) = self.state.snapshot(handle) {
                        let _ = self.events.send(RelayTransportEvent::Renewed(snapshot));
                    }
                }
                RelayReservationEvent::Publishable { handle, .. } => {
                    let result = self.state.snapshot(handle).map_err(Error::from);
                    if let Some(waiters) = self.confirmation_waiters.remove(&handle) {
                        match result {
                            Ok(snapshot) => {
                                let _ = self
                                    .events
                                    .send(RelayTransportEvent::Publishable(snapshot.clone()));
                                for waiter in waiters {
                                    let _ = waiter.send(Ok(snapshot.clone()));
                                }
                            }
                            Err(error) => {
                                let reason = error.to_string();
                                for waiter in waiters {
                                    let _ = waiter
                                        .send(Err(Error::RelayReservationClosed(reason.clone())));
                                }
                            }
                        }
                    } else if let Ok(snapshot) = self.state.snapshot(handle) {
                        let _ = self.events.send(RelayTransportEvent::Publishable(snapshot));
                    }
                }
                RelayReservationEvent::ConfirmationRejected {
                    handle,
                    reason,
                    cancellation,
                } => {
                    let _ = self
                        .events
                        .send(RelayTransportEvent::ConfirmationRejected { handle, reason });
                    self.reject_confirmation(handle, reason);
                    events.extend(self.start_teardown(swarm, cancellation));
                }
                RelayReservationEvent::CancellationStarted { cancellation } => {
                    let _ = self.events.send(RelayTransportEvent::Unpublished {
                        handle: cancellation.handle(),
                    });
                    events.extend(self.start_teardown(swarm, cancellation));
                }
                RelayReservationEvent::CancellationPending { cancellation } => {
                    events.extend(self.start_teardown(swarm, cancellation));
                }
                RelayReservationEvent::CloseLateConnection {
                    handle,
                    connection_id,
                } => {
                    if let Some(barrier) =
                        self.cancellation_barriers.get_mut(&handle.relay_peer_id())
                    {
                        barrier.pending_connections.insert(connection_id);
                    }
                    if !swarm.close_connection(connection_id) {
                        if let Ok(event) = self
                            .state
                            .observe_direct_connection_closed(handle, connection_id)
                        {
                            events.push_back(event);
                        }
                    }
                }
                RelayReservationEvent::Canceled { handle } => {
                    self.cancellation_barriers
                        .entry(handle.relay_peer_id())
                        .or_insert_with(|| CancellationBarrier {
                            handle,
                            pending_connections: HashSet::new(),
                            state_canceled: false,
                            dispatch_terminal: true,
                        })
                        .state_canceled = true;
                    self.maybe_finish_cancellation(handle.relay_peer_id());
                }
            }
        }
    }

    fn reject_confirmation(
        &mut self,
        handle: RelayReservationHandle,
        reason: RelayConfirmationRejection,
    ) {
        self.terminal_rejections.insert(handle, reason);
        if let Some(waiters) = self.confirmation_waiters.remove(&handle) {
            for waiter in waiters {
                let _ = waiter.send(Err(Error::RelayConfirmationRejected(reason)));
            }
        }
    }

    fn close_before_confirmation(&mut self, handle: RelayReservationHandle, reason: String) {
        self.terminal_closures
            .entry(handle)
            .or_insert_with(|| reason.clone());
        if let Some(waiters) = self.confirmation_waiters.remove(&handle) {
            for waiter in waiters {
                let _ = waiter.send(Err(Error::RelayReservationClosed(reason.clone())));
            }
        }
    }

    fn fail_pending_start(&mut self, handle: RelayReservationHandle, reason: &str) {
        if let Some(response) = self.pending_starts.remove(&handle) {
            let _ = response.send(Err(Error::RelayReservationClosed(reason.to_string())));
        }
    }

    fn start_teardown(
        &mut self,
        swarm: &mut Swarm<Behaviour>,
        cancellation: RelayCancellation,
    ) -> Vec<RelayReservationEvent> {
        self.fail_pending_start(
            cancellation.handle(),
            "reservation closed before dispatch to the selected relay connection",
        );
        let dispatch_pending = swarm
            .behaviour_mut()
            .relay
            .fence_dispatch(cancellation.handle());
        let mut events = Vec::new();
        let connections = self
            .direct_connections
            .get(&cancellation.relay_peer_id())
            .map(|connections| {
                connections
                    .iter()
                    .map(|connection| connection.connection_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let pending_dials = self
            .pending_direct_dials
            .get(&cancellation.relay_peer_id())
            .cloned()
            .unwrap_or_default();
        let barrier = self
            .cancellation_barriers
            .entry(cancellation.relay_peer_id())
            .or_insert_with(|| CancellationBarrier {
                handle: cancellation.handle(),
                pending_connections: HashSet::new(),
                state_canceled: false,
                dispatch_terminal: !dispatch_pending,
            });
        barrier.dispatch_terminal |= !dispatch_pending;
        barrier
            .pending_connections
            .extend(connections.iter().copied());
        barrier.pending_connections.extend(pending_dials);

        if !swarm.remove_listener(cancellation.listener_id()) {
            if let Ok(event) = self
                .state
                .observe_listener_closed(cancellation.handle(), cancellation.listener_id())
            {
                events.push(event);
            }
        }

        for connection_id in connections.iter().copied() {
            let was_open = swarm.close_connection(connection_id);
            if !was_open && Some(connection_id) == cancellation.direct_connection() {
                if let Ok(event) = self
                    .state
                    .observe_direct_connection_closed(cancellation.handle(), connection_id)
                {
                    events.push(event);
                }
            }
        }
        match cancellation.direct_connection() {
            None => {
                if let Ok(event) = self
                    .state
                    .observe_no_direct_connection(cancellation.handle())
                {
                    events.push(event);
                }
            }
            Some(connection_id) if !connections.contains(&connection_id) => {
                if let Ok(event) = self
                    .state
                    .observe_direct_connection_closed(cancellation.handle(), connection_id)
                {
                    events.push(event);
                }
            }
            Some(_) => {}
        }
        events
    }

    fn maybe_finish_cancellation(&mut self, relay_peer_id: PeerId) {
        let complete = self
            .cancellation_barriers
            .get(&relay_peer_id)
            .is_some_and(|barrier| {
                barrier.state_canceled
                    && barrier.pending_connections.is_empty()
                    && barrier.dispatch_terminal
            });
        if !complete {
            return;
        }
        let Some(barrier) = self.cancellation_barriers.remove(&relay_peer_id) else {
            return;
        };
        let handle = barrier.handle;
        let _ = self.events.send(RelayTransportEvent::Canceled { handle });
        if let Some(waiters) = self.cancellation_waiters.remove(&handle) {
            for waiter in waiters {
                let _ = waiter.send(Ok(()));
            }
        }
        if let Some(waiters) = self.confirmation_waiters.remove(&handle) {
            for waiter in waiters {
                let _ = waiter.send(Err(Error::RelayReservationClosed(
                    "reservation was canceled before confirmation".into(),
                )));
            }
        }
    }
}

enum DialCompletion {
    Exact,
    FirstRelayConnection { expected_address: Multiaddr },
}

struct PendingDial {
    peer_id: PeerId,
    response: oneshot::Sender<P2PResult<ConnectionId>>,
    completion: DialCompletion,
    requested_direct_address: Option<Multiaddr>,
    circuit_relay_peer_id: Option<PeerId>,
    _circuit_permit: Option<RelayCircuitDialPermit>,
}

fn classify_dial_error(error: DialError) -> Error {
    let dns_failure = match &error {
        DialError::Transport(errors) => errors.iter().any(|(_, error)| match error {
            TransportError::MultiaddrNotSupported(_) => false,
            TransportError::Other(error) => error_chain_contains_dns_failure(error),
        }),
        _ => false,
    };
    let reason = error.to_string();
    if dns_failure {
        Error::Dns(reason)
    } else {
        Error::Dial(reason)
    }
}

fn error_chain_contains_dns_failure(error: &(dyn StdError + 'static)) -> bool {
    if error.is::<hickory_resolver::ResolveError>() {
        return true;
    }
    #[cfg(any(test, target_os = "ios"))]
    if error.is::<crate::system_dns::SystemDnsError>() {
        return true;
    }
    if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
        if io_error
            .get_ref()
            .is_some_and(|source| error_chain_contains_dns_failure(source))
        {
            return true;
        }
    }
    error.source().is_some_and(error_chain_contains_dns_failure)
}

enum SwarmExit {
    Shutdown(oneshot::Sender<P2PResult<()>>),
    OwnersDropped,
    SwarmEnded,
    ExpiryDriverEnded,
    ListenerClosed,
}

struct NodeTaskOwnership {
    task: AsyncMutex<Option<tokio::task::JoinHandle<()>>>,
    completed: watch::Sender<bool>,
}

struct NodeTaskCompletionGuard(watch::Sender<bool>);

impl Drop for NodeTaskCompletionGuard {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

struct NodeTaskGuard {
    observations: NodeObservations,
    armed: bool,
}

impl NodeTaskGuard {
    fn new(observations: NodeObservations) -> Self {
        Self {
            observations,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for NodeTaskGuard {
    fn drop(&mut self) {
        if self.armed {
            self.observations.failed(NodeFailure::TaskCancelled);
        }
    }
}

async fn supervise_node_task<F>(task: F, observations: NodeObservations, mut guard: NodeTaskGuard)
where
    F: Future<Output = SwarmExit>,
{
    let outcome = AssertUnwindSafe(task).catch_unwind().await;
    let shutdown_response = match outcome {
        Ok(SwarmExit::Shutdown(response)) => {
            observations.stopped();
            Some(response)
        }
        Ok(SwarmExit::OwnersDropped) => {
            observations.stopped();
            None
        }
        Ok(SwarmExit::SwarmEnded) => {
            observations.failed(NodeFailure::SwarmEnded);
            None
        }
        Ok(SwarmExit::ExpiryDriverEnded) => {
            observations.failed(NodeFailure::ExpiryDriverEnded);
            None
        }
        Ok(SwarmExit::ListenerClosed) => {
            observations.failed(NodeFailure::ListenerClosed);
            None
        }
        Err(_) => {
            observations.failed(NodeFailure::Panicked);
            None
        }
    };
    guard.disarm();
    if let Some(response) = shutdown_response {
        let _ = response.send(Ok(()));
    }
}

async fn run_swarm(
    mut swarm: Swarm<Behaviour>,
    mut commands: mpsc::Receiver<Command>,
    direct_listeners: watch::Sender<DirectListenState>,
    direct_listener_addresses: HashMap<ListenerId, Multiaddr>,
    relay_events: broadcast::Sender<RelayTransportEvent>,
    observations: NodeObservations,
) -> SwarmExit {
    let local_peer_id = *swarm.local_peer_id();
    let mut reservations = ReservationRuntime::new(local_peer_id, relay_events);
    let mut pending_dials: HashMap<ConnectionId, PendingDial> = HashMap::new();
    let mut pending_disconnects: HashMap<PeerId, Vec<oneshot::Sender<P2PResult<()>>>> =
        HashMap::new();
    loop {
        tokio::select! {
            event = swarm.next() => {
                let Some(event) = event else { return SwarmExit::SwarmEnded };
                match event {
                    SwarmEvent::NewListenAddr { listener_id, address } => {
                        if direct_listener_addresses.contains_key(&listener_id) {
                            direct_listeners.send_modify(|state| {
                                state.bound_listener_ids.insert(listener_id);
                                if !state.addresses.contains(&address) {
                                    state.addresses.push(address.clone());
                                }
                            });
                        }
                        if let Some(handle) = reservations.state.handle_for_listener(listener_id) {
                            if let Ok(event) = reservations.state.observe_listener_address(
                                handle,
                                listener_id,
                                &address,
                            ) {
                                reservations.apply_event(&mut swarm, event);
                            }
                        }
                    }
                    SwarmEvent::ConnectionEstablished {
                        peer_id,
                        connection_id,
                        endpoint,
                        ..
                    } => {
                        observations.connection_established(peer_id, connection_id);
                        let requested_direct_address = pending_dials
                            .get(&connection_id)
                            .and_then(|pending| pending.requested_direct_address.clone());
                        reservations.record_pending_dial_finished(peer_id, connection_id);
                        if !endpoint.is_relayed() {
                            reservations.record_direct_connection(
                                peer_id,
                                connection_id,
                                requested_direct_address,
                            );
                            let barrier_active = if let Some(barrier) =
                                reservations.cancellation_barriers.get_mut(&peer_id)
                            {
                                barrier.pending_connections.insert(connection_id);
                                true
                            } else {
                                false
                            };
                            if barrier_active {
                                swarm.close_connection(connection_id);
                            } else if let Some(handle) =
                                reservations.state.handle_for_relay(peer_id)
                            {
                                let canceling = reservations
                                    .state
                                    .snapshot(handle)
                                    .is_ok_and(|snapshot| {
                                        snapshot.state()
                                            == crate::relay::RelayReservationState::Canceling
                                    });
                                if canceling {
                                    if let Ok(event) = reservations
                                        .state
                                        .observe_direct_connection(handle, connection_id)
                                    {
                                        reservations.apply_event(&mut swarm, event);
                                    }
                                }
                            }
                        }
                        if let Some(pending) = pending_dials.remove(&connection_id) {
                            if pending.peer_id == peer_id {
                                let selected = match pending.completion {
                                    DialCompletion::Exact => connection_id,
                                    DialCompletion::FirstRelayConnection {
                                        expected_address,
                                    } => match reservations.selected_direct_connection(
                                        peer_id,
                                        &expected_address,
                                    ) {
                                        Ok(Some(selected)) => selected,
                                        Ok(None) => connection_id,
                                        Err(error) => {
                                            swarm.close_connection(connection_id);
                                            let _ = pending.response.send(Err(error));
                                            continue;
                                        }
                                    },
                                };
                                if selected != connection_id {
                                    swarm.close_connection(connection_id);
                                }
                                if pending.response.send(Ok(selected)).is_err()
                                    && selected == connection_id
                                {
                                    swarm.close_connection(selected);
                                }
                            } else {
                                swarm.close_connection(connection_id);
                                let _ = pending.response.send(Err(Error::UnexpectedRemotePeer {
                                    expected: pending.peer_id.to_string(),
                                    actual: peer_id.to_string(),
                                }));
                            }
                        }
                    }
                    SwarmEvent::OutgoingConnectionError { connection_id, error, .. } => {
                        if let Some(pending) = pending_dials.remove(&connection_id) {
                            reservations
                                .record_direct_dial_failed(pending.peer_id, connection_id);
                            let _ = pending
                                .response
                                .send(Err(classify_dial_error(error)));
                        }
                    }
                    SwarmEvent::ConnectionClosed {
                        peer_id,
                        connection_id,
                        endpoint,
                        num_established,
                        ..
                    } => {
                        observations.connection_closed(peer_id, connection_id);
                        if !endpoint.is_relayed() {
                            reservations.record_connection_closed(
                                &mut swarm,
                                peer_id,
                                connection_id,
                            );
                        }
                        if num_established != 0 {
                            continue;
                        }
                        if let Some(responses) = pending_disconnects.remove(&peer_id) {
                            for response in responses {
                                let _ = response.send(Ok(()));
                            }
                        }
                    }
                    SwarmEvent::ListenerClosed { listener_id, reason, .. } => {
                        if let Some(requested_address) = direct_listener_addresses.get(&listener_id) {
                            let reason = match reason {
                                Ok(()) => "listener closed unexpectedly".to_string(),
                                Err(error) => error.to_string(),
                            };
                            direct_listeners.send_modify(|state| {
                                state.failure = Some(DirectListenFailure {
                                    address: requested_address.to_string(),
                                    reason: reason.clone(),
                                });
                            });
                            return SwarmExit::ListenerClosed;
                        }
                        if let Some(handle) = reservations.state.handle_for_listener(listener_id) {
                            match reason {
                                Ok(()) => {
                                    if let Ok(event) = reservations
                                        .state
                                        .observe_listener_closed(handle, listener_id)
                                    {
                                        reservations.apply_event(&mut swarm, event);
                                    }
                                }
                                Err(error) => {
                                    reservations
                                        .close_before_confirmation(handle, error.to_string());
                                    if let Ok(event) = reservations
                                        .state
                                        .observe_listener_closed(handle, listener_id)
                                    {
                                        reservations.apply_event(&mut swarm, event);
                                    }
                                }
                            }
                        }
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Relay(
                        relay_client::Event::ReservationDispatched { handle },
                    )) => {
                        let Some(response) = reservations.pending_starts.remove(&handle) else {
                            continue;
                        };
                        if response.send(Ok(handle)).is_err() {
                            if let Ok(event) = reservations.state.cancel(handle) {
                                reservations.apply_event(&mut swarm, event);
                            }
                        }
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Relay(
                        relay_client::Event::ReservationDispatchFailed { handle, reason },
                    )) => {
                        if let Some(barrier) = reservations
                            .cancellation_barriers
                            .get_mut(&handle.relay_peer_id())
                            .filter(|barrier| barrier.handle == handle)
                        {
                            barrier.dispatch_terminal = true;
                        }
                        reservations.fail_pending_start(handle, reason);
                        if let Ok(event) = reservations.state.cancel(handle) {
                            reservations.apply_event(&mut swarm, event);
                        }
                        reservations.maybe_finish_cancellation(handle.relay_peer_id());
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Relay(
                        relay_client::Event::Upstream {
                            event: relay::client::Event::ReservationReqAccepted {
                                relay_peer_id,
                                renewal,
                                limit,
                            },
                            handle: Some(handle),
                        },
                    )) => {
                        if handle.relay_peer_id() != relay_peer_id {
                            continue;
                        }
                        let observed = limit.map(|limit| {
                            ObservedRelayLimits::new(
                                limit.duration(),
                                limit.data_in_bytes(),
                            )
                        });
                        if let Ok(event) = reservations
                            .state
                            .observe_acceptance(handle, renewal, observed)
                        {
                            reservations.apply_event(&mut swarm, event);
                        }
                    }
                    _ => {}
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { return SwarmExit::OwnersDropped };
                match command {
                    Command::Connect {
                        peer_id,
                        addresses,
                        circuit_relay_peer_id,
                        circuit_permit,
                        response,
                    } => {
                        let direct = addresses.iter().all(|address| {
                            !address
                                .iter()
                                .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
                        });
                        if direct != circuit_relay_peer_id.is_none() {
                            let _ = response.send(Err(Error::InvalidRelayRoute {
                                address: addresses
                                    .first()
                                    .map(ToString::to_string)
                                    .unwrap_or_default(),
                                reason: "circuit dial metadata does not match the route".into(),
                            }));
                            continue;
                        }
                        if direct && reservations.cancellation_barriers.contains_key(&peer_id) {
                            let _ = response.send(Err(Error::RelayReservationClosed(
                                "relay cancellation is still closing direct connections".into(),
                            )));
                            continue;
                        }
                        if circuit_relay_peer_id.is_some_and(|relay_peer_id| {
                            swarm
                                .behaviour()
                                .relay
                                .has_pending_dispatch(relay_peer_id)
                                || reservations
                                    .pending_starts
                                    .keys()
                                    .any(|handle| handle.relay_peer_id() == relay_peer_id)
                        }) {
                            // The pinned relay behaviour exposes private
                            // Reserve and EstablishCircuit handler inputs as
                            // the same opaque action type. Keep its transport
                            // queue serialized until the wrapper has stamped
                            // the reservation action with this generation.
                            let _ = response.send(Err(Error::RelayReservationClosed(
                                "relay reservation dispatch is still pending".into(),
                            )));
                            continue;
                        }
                        let requested_direct_address = if direct && addresses.len() == 1 {
                            addresses.first().cloned()
                        } else {
                            None
                        };
                        let dial = DialOpts::peer_id(peer_id)
                                .condition(PeerCondition::Always)
                                .allocate_new_port()
                                .addresses(addresses)
                                .build();
                        let connection_id = dial.connection_id();
                        match swarm.dial(dial) {
                            Ok(()) => {
                                if direct {
                                    reservations
                                        .record_pending_direct_dial(peer_id, connection_id);
                                }
                                pending_dials.insert(
                                    connection_id,
                                    PendingDial {
                                        peer_id,
                                        response,
                                        completion: DialCompletion::Exact,
                                        requested_direct_address,
                                        circuit_relay_peer_id,
                                        _circuit_permit: circuit_permit,
                                    },
                                );
                            }
                            Err(error) => {
                                let _ = response.send(Err(classify_dial_error(error)));
                            }
                        }
                    }
                    Command::SelectRelayConnection {
                        peer_id,
                        address,
                        response,
                    } => {
                        if reservations.cancellation_barriers.contains_key(&peer_id) {
                            let _ = response.send(Err(Error::RelayReservationClosed(
                                "relay cancellation is still closing direct connections".into(),
                            )));
                            continue;
                        }
                        match reservations.selected_direct_connection(peer_id, &address) {
                            Ok(Some(connection_id)) => {
                                let _ = response.send(Ok(connection_id));
                                continue;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                let _ = response.send(Err(error));
                                continue;
                            }
                        }
                        let dial = DialOpts::peer_id(peer_id)
                            .condition(PeerCondition::Always)
                            .allocate_new_port()
                            .addresses(vec![address.clone()])
                            .build();
                        let connection_id = dial.connection_id();
                        match swarm.dial(dial) {
                            Ok(()) => {
                                reservations.record_pending_direct_dial(peer_id, connection_id);
                                pending_dials.insert(
                                    connection_id,
                                    PendingDial {
                                        peer_id,
                                        response,
                                        completion: DialCompletion::FirstRelayConnection {
                                            expected_address: address.clone(),
                                        },
                                        requested_direct_address: Some(address),
                                        circuit_relay_peer_id: None,
                                        _circuit_permit: None,
                                    },
                                );
                            }
                            Err(error) => {
                                let _ = response.send(Err(classify_dial_error(error)));
                            }
                        }
                    }
                    Command::CloseConnection {
                        connection_id,
                        response,
                    } => {
                        swarm.close_connection(connection_id);
                        let _ = response.send(Ok(()));
                    }
                    Command::BeginReservation {
                        provider,
                        direct_connection,
                        response,
                    } => {
                        let relay_peer_id = provider.relay_peer_id();
                        if !reservations
                            .direct_connections
                            .get(&relay_peer_id)
                            .is_some_and(|connections| {
                                connections.iter().any(|connection| {
                                    connection.connection_id == direct_connection
                                })
                            })
                        {
                            let _ = response.send(Err(Error::RelayReservationClosed(
                                "selected direct relay connection closed before reservation start"
                                    .into(),
                            )));
                            continue;
                        }
                        if pending_dials.values().any(|pending| {
                            pending.circuit_relay_peer_id == Some(relay_peer_id)
                        }) {
                            // A previously queued circuit request could be the
                            // next opaque relay-client action. Starting a
                            // reservation now would make generation stamping
                            // ambiguous, so fail closed before listen_on.
                            let _ = response.send(Err(Error::RelayReservationClosed(
                                "an outbound circuit request is still pending on the selected relay"
                                    .into(),
                            )));
                            continue;
                        }
                        if reservations
                            .state
                            .handle_for_relay(relay_peer_id)
                            .is_some()
                            || reservations
                                .cancellation_barriers
                                .contains_key(&relay_peer_id)
                        {
                            let _ = response.send(Err(Error::RelayReservation(
                                crate::relay::RelayReservationError::ReservationAlreadyExists(
                                    relay_peer_id,
                                ),
                            )));
                            continue;
                        }
                        let listen_address = provider.reservation_listen_address();
                        let listener_id = match swarm.listen_on(listen_address.clone()) {
                            Ok(listener_id) => listener_id,
                            Err(error) => {
                                let _ = response.send(Err(Error::Listen {
                                    address: listen_address.to_string(),
                                    reason: error.to_string(),
                                }));
                                continue;
                            }
                        };
                        let handle = match reservations.state.begin(provider, listener_id) {
                            Ok(handle) => handle,
                            Err(error) => {
                                swarm.remove_listener(listener_id);
                                let _ = response.send(Err(error.into()));
                                continue;
                            }
                        };
                        reservations
                            .terminal_rejections
                            .retain(|old_handle, _| {
                                old_handle.relay_peer_id() != relay_peer_id
                            });
                        reservations
                            .terminal_closures
                            .retain(|old_handle, _| old_handle.relay_peer_id() != relay_peer_id);
                        match reservations
                            .state
                            .observe_direct_connection(handle, direct_connection)
                        {
                            Ok(event) => reservations.apply_event(&mut swarm, event),
                            Err(error) => {
                                swarm.remove_listener(listener_id);
                                let _ = response.send(Err(error.into()));
                                continue;
                            }
                        }
                        if swarm
                            .behaviour_mut()
                            .relay
                            .register_dispatch(handle, direct_connection)
                            .is_err()
                        {
                            swarm.remove_listener(listener_id);
                            if let Ok(event) = reservations.state.cancel(handle) {
                                reservations.apply_event(&mut swarm, event);
                            }
                            let _ = response.send(Err(Error::RelayReservationClosed(
                                "another reservation dispatch is already pending".into(),
                            )));
                            continue;
                        }
                        if reservations.pending_starts.insert(handle, response).is_some() {
                            if let Ok(event) = reservations.state.cancel(handle) {
                                reservations.apply_event(&mut swarm, event);
                            }
                        }
                    }
                    Command::WaitReservation { handle, response } => {
                        if let Some(reason) =
                            reservations.terminal_rejections.get(&handle).copied()
                        {
                            let _ = response
                                .send(Err(Error::RelayConfirmationRejected(reason)));
                            continue;
                        }
                        if let Some(reason) = reservations.terminal_closures.get(&handle) {
                            let _ = response
                                .send(Err(Error::RelayReservationClosed(reason.clone())));
                            continue;
                        }
                        match reservations.state.snapshot(handle) {
                            Ok(snapshot)
                                if snapshot.state()
                                    == crate::relay::RelayReservationState::Publishable =>
                            {
                                let _ = response.send(Ok(snapshot));
                            }
                            Ok(snapshot)
                                if snapshot.state()
                                    == crate::relay::RelayReservationState::AwaitingConfirmation =>
                            {
                                reservations
                                    .confirmation_waiters
                                    .entry(handle)
                                    .or_default()
                                    .push(response);
                            }
                            Ok(_) => {
                                let _ = response.send(Err(Error::RelayReservationClosed(
                                    "reservation is canceling".into(),
                                )));
                            }
                            Err(error) => {
                                let _ = response.send(Err(error.into()));
                            }
                        }
                    }
                    Command::ReservationSnapshot { handle, response } => {
                        if let Some(reason) =
                            reservations.terminal_rejections.get(&handle).copied()
                        {
                            let _ = response
                                .send(Err(Error::RelayConfirmationRejected(reason)));
                        } else if let Some(reason) = reservations.terminal_closures.get(&handle) {
                            let _ = response
                                .send(Err(Error::RelayReservationClosed(reason.clone())));
                        } else {
                            let _ = response.send(
                                reservations.state.snapshot(handle).map_err(Error::from),
                            );
                        }
                    }
                    Command::CancelReservation { handle, response } => {
                        if reservations
                            .cancellation_barriers
                            .get(&handle.relay_peer_id())
                            .is_some_and(|barrier| barrier.handle == handle)
                        {
                            reservations
                                .cancellation_waiters
                                .entry(handle)
                                .or_default()
                                .push(response);
                            reservations.maybe_finish_cancellation(
                                handle.relay_peer_id(),
                            );
                            continue;
                        }
                        match reservations.state.cancel(handle) {
                            Ok(event) => {
                                reservations
                                    .cancellation_waiters
                                    .entry(handle)
                                    .or_default()
                                    .push(response);
                                reservations.apply_event(&mut swarm, event);
                            }
                            Err(error) => {
                                let _ = response.send(Err(error.into()));
                            }
                        }
                    }
                    Command::Disconnect { peer_id, response } => {
                        if let Some(responses) = pending_disconnects.get_mut(&peer_id) {
                            responses.push(response);
                            continue;
                        }
                        match swarm.disconnect_peer_id(peer_id) {
                            Ok(()) => {
                                pending_disconnects.insert(peer_id, vec![response]);
                            }
                            Err(()) => {
                                let _ = response.send(Err(Error::Disconnect(peer_id.to_string())));
                            }
                        }
                    }
                    Command::Shutdown { response } => {
                        return SwarmExit::Shutdown(response);
                    }
                }
            }
        }
    }
}

fn normalize_remote_address(
    mut address: Multiaddr,
    peer_id: PeerId,
    allow_circuit: bool,
) -> P2PResult<Multiaddr> {
    if let Some(Protocol::P2p(address_peer_id)) = address.iter().last() {
        if address_peer_id != peer_id {
            return Err(Error::InvalidRemoteAddress {
                address: address.to_string(),
                reason: format!("contains different Peer ID {address_peer_id}"),
            });
        }
        address.pop();
    }
    let has_tcp = address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Tcp(_)));
    let has_circuit = address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::P2pCircuit));
    if !has_tcp || has_circuit != allow_circuit {
        return Err(Error::InvalidRemoteAddress {
            address: address.to_string(),
            reason: if allow_circuit {
                "expected one explicit TCP Circuit Relay route".into()
            } else {
                "only explicit direct TCP multiaddrs are supported".into()
            },
        });
    }
    Ok(address)
}

struct ParsedRelayRoute {
    relay_peer_id: PeerId,
    target_peer_id: PeerId,
    direct_relay_address: Multiaddr,
}

fn parse_relay_route(route: &Multiaddr) -> P2PResult<ParsedRelayRoute> {
    let mut protocols = route.iter();
    let (host, port, relay_peer_id, target_peer_id) = match (
        protocols.next(),
        protocols.next(),
        protocols.next(),
        protocols.next(),
        protocols.next(),
        protocols.next(),
    ) {
        (
            Some(Protocol::Dns4(host)),
            Some(Protocol::Tcp(port)),
            Some(Protocol::P2p(relay_peer_id)),
            Some(Protocol::P2pCircuit),
            Some(Protocol::P2p(target_peer_id)),
            None,
        ) => (host, port, relay_peer_id, target_peer_id),
        _ => {
            return Err(Error::InvalidRelayRoute {
                address: route.to_string(),
                reason: "expected exact dns4/tcp/p2p/p2p-circuit/p2p grammar".into(),
            });
        }
    };

    let raw_base = format!("/dns4/{host}/tcp/{port}/p2p/{relay_peer_id}");
    let canonical = canonicalize_provider_base(&raw_base, relay_peer_id).map_err(|error| {
        Error::InvalidRelayRoute {
            address: route.to_string(),
            reason: error.to_string(),
        }
    })?;
    let canonical_route = canonical
        .clone()
        .with(Protocol::P2pCircuit)
        .with(Protocol::P2p(target_peer_id));
    if canonical_route != *route {
        return Err(Error::InvalidRelayRoute {
            address: route.to_string(),
            reason: "route is not in canonical form".into(),
        });
    }
    let mut direct_relay_address = canonical;
    direct_relay_address.pop();
    Ok(ParsedRelayRoute {
        relay_peer_id,
        target_peer_id,
        direct_relay_address,
    })
}

async fn authenticate(
    stream: Stream,
    local_peer_id: PeerId,
    remote_peer_id: PeerId,
    tokens: &TokenStore,
    verifier: &DdsTokenVerifier,
    requirements: &SessionRequirements,
) -> P2PResult<AuthenticatedStream> {
    let (inner, remote) = authenticate_duplex(
        stream,
        local_peer_id,
        remote_peer_id,
        tokens,
        verifier,
        requirements,
    )
    .await?;
    Ok(AuthenticatedStream::new(inner, remote))
}

#[cfg(test)]
mod dial_error_tests {
    use std::{future::pending, io};

    use hickory_resolver::{ResolveError, ResolveErrorKind};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    use crate::{P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE};

    use super::*;

    #[tokio::test]
    async fn node_supervisor_reports_exit_panic_and_pre_poll_cancellation() {
        let ended = NodeObservations::new();
        supervise_node_task(
            async { SwarmExit::SwarmEnded },
            ended.clone(),
            NodeTaskGuard::new(ended.clone()),
        )
        .await;
        assert_eq!(
            ended.snapshot().status(),
            crate::NodeObservationStatus::Failed(NodeFailure::SwarmEnded)
        );

        let panicked = NodeObservations::new();
        supervise_node_task(
            async {
                panic!("synthetic node task panic");
                #[allow(unreachable_code)]
                SwarmExit::SwarmEnded
            },
            panicked.clone(),
            NodeTaskGuard::new(panicked.clone()),
        )
        .await;
        assert_eq!(
            panicked.snapshot().status(),
            crate::NodeObservationStatus::Failed(NodeFailure::Panicked)
        );

        let cancelled = NodeObservations::new();
        let never_polled = supervise_node_task(
            pending::<SwarmExit>(),
            cancelled.clone(),
            NodeTaskGuard::new(cancelled.clone()),
        );
        drop(never_polled);
        assert_eq!(
            cancelled.snapshot().status(),
            crate::NodeObservationStatus::Failed(NodeFailure::TaskCancelled)
        );
    }

    #[tokio::test]
    async fn canceled_task_join_retains_ownership_for_forced_shutdown() {
        const PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;
        let node = Node::start(
            Identity::generate(),
            DdsTokenVerifier::from_es256_pem(PUBLIC_KEY).unwrap(),
            std::iter::empty::<Multiaddr>(),
        )
        .unwrap();

        assert!(
            tokio::time::timeout(Duration::ZERO, node.finish_node_task(false))
                .await
                .is_err(),
            "canceling an in-progress join must leave the task owned"
        );
        tokio::time::timeout(Duration::from_secs(1), node.shutdown_now())
            .await
            .expect("forced shutdown must abort and join the retained task");
        assert_eq!(
            node.observations().snapshot().status(),
            crate::NodeObservationStatus::Failed(NodeFailure::TaskCancelled)
        );
    }

    #[tokio::test]
    async fn domain_authority_require_reverifies_against_the_live_key_ring() {
        const PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;
        const PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;
        let identity = Identity::generate();
        let peer_id = identity.peer_id();
        let verifier = DdsTokenVerifier::from_es256_pem(PUBLIC_KEY).unwrap();
        let node =
            Node::start(identity, verifier.clone(), std::iter::empty::<Multiaddr>()).unwrap();
        let authority = node.authority();
        let domain_id = Uuid::new_v4();
        let now = unix_time_now();
        let claims = P2PAccessClaims {
            token_type: P2P_TOKEN_TYPE.into(),
            iss: P2P_TOKEN_ISSUER.into(),
            aud: vec![P2P_TOKEN_AUDIENCE.into()],
            sub: Uuid::new_v4().to_string(),
            organization_id: None,
            peer_type: None,
            peer_id: peer_id.to_string(),
            domain_ids: vec![domain_id.to_string()],
            scopes: Vec::new(),
            application: None,
            iat: now,
            nbf: None,
            exp: now + P2P_TOKEN_TTL.as_secs(),
        };
        let signed = SignedP2pCredential::new(
            encode(
                &Header::new(Algorithm::ES256),
                &claims,
                &EncodingKey::from_ec_pem(PRIVATE_KEY).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        authority.install_credential(signed).await.unwrap();
        assert_eq!(authority.require(domain_id).await.unwrap(), claims);

        verifier.make_stale_for_test();
        assert!(matches!(
            authority.require(domain_id).await,
            Err(crate::P2pCredentialError::InvalidAccessToken(
                Error::VerificationKeysStale
            ))
        ));
        assert!(authority.current_claims().await.is_none());
        node.shutdown().await.unwrap();
    }

    #[test]
    fn dns_resolution_and_transport_dial_failures_remain_typed() {
        let address: Multiaddr = "/dns4/relay.testnet.aukiverse.com/tcp/443".parse().unwrap();
        let resolve_error = ResolveError::from(ResolveErrorKind::Message("no DNS record"));
        let dns = DialError::Transport(vec![(
            address.clone(),
            TransportError::Other(io::Error::other(resolve_error)),
        )]);
        assert!(matches!(classify_dial_error(dns), Error::Dns(_)));

        let system_dns = DialError::Transport(vec![(
            address.clone(),
            TransportError::Other(io::Error::other(crate::system_dns::SystemDnsError::new(
                io::Error::from(io::ErrorKind::NotFound),
            ))),
        )]);
        assert!(matches!(classify_dial_error(system_dns), Error::Dns(_)));

        let refused = DialError::Transport(vec![(
            address,
            TransportError::Other(io::Error::from(io::ErrorKind::ConnectionRefused)),
        )]);
        assert!(matches!(classify_dial_error(refused), Error::Dial(_)));
    }
}
