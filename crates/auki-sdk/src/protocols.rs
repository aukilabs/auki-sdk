use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use auki_p2p::{
    ApplicationProtocol, ApplicationProtocolServer, AuthenticatedRouteStream, ExactRoute,
    Multiaddr, Node, PeerId, SessionRequirements, TargetedStreamError, canonicalize_circuit_route,
    validate_direct_route,
};
use parking_lot::Mutex;
use tokio::time::{Instant, timeout_at};

use crate::{
    context::ContextLifecycle,
    protocol_contract::{
        AukiProtocolError, AukiProtocolRouteAttempt, AukiProtocolSpec, AukiProtocolStream,
    },
};

const PROTOCOL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct AukiPeerProtocols {
    inner: Arc<ProtocolRuntime>,
}

impl AukiPeerProtocols {
    pub(crate) fn new(
        node: Node,
        domain_id: uuid::Uuid,
        routes: impl IntoIterator<Item = (PeerId, Vec<Multiaddr>)>,
        lifecycle: ContextLifecycle,
    ) -> Self {
        Self {
            inner: Arc::new(ProtocolRuntime {
                node,
                domain_id,
                routes: routes.into_iter().collect(),
                lifecycle,
                registrations: Mutex::new(HashMap::new()),
                next_generation: AtomicU64::new(1),
            }),
        }
    }

    /// Register one exact inbound application protocol and bounded handler.
    pub fn register<H, F>(
        &self,
        spec: AukiProtocolSpec,
        handler: H,
    ) -> Result<AukiProtocolRegistration, AukiProtocolError>
    where
        H: Fn(AukiProtocolStream) -> F + Send + Sync + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        let _running = self
            .inner
            .lifecycle
            .enter()
            .ok_or(AukiProtocolError::Stopped)?;
        let mut registrations = self.inner.registrations.lock();
        if registrations.contains_key(&spec.protocol_id) {
            return Err(AukiProtocolError::DuplicateProtocol(spec.protocol_id));
        }
        let generation = self
            .inner
            .next_generation
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |generation| {
                generation.checked_add(1)
            })
            .map_err(|_| AukiProtocolError::GenerationExhausted)?;
        let requirements = SessionRequirements::new(self.inner.domain_id.to_string())
            .map_err(AukiProtocolError::P2p)?;
        let server = self
            .inner
            .node
            .serve(
                spec.inner,
                requirements,
                self.inner.lifecycle.token(),
                handler,
            )
            .map_err(|error| match error {
                auki_p2p::Error::ProtocolAlreadyRegistered => {
                    AukiProtocolError::DuplicateProtocol(spec.protocol_id.clone())
                }
                error => AukiProtocolError::P2p(error),
            })?;
        let entry = Arc::new(ProtocolEntry {
            protocol_id: spec.protocol_id.clone(),
            generation,
            server: Mutex::new(Some(server)),
        });
        registrations.insert(spec.protocol_id, Arc::clone(&entry));
        Ok(AukiProtocolRegistration {
            runtime: Arc::downgrade(&self.inner),
            entry,
            closed: false,
        })
    }

    /// Open the selected protocol using the configured routes for one peer.
    pub async fn open(
        &self,
        expected_peer: PeerId,
        protocol_id: impl Into<String>,
    ) -> Result<AuthenticatedRouteStream, AukiProtocolError> {
        let protocol_id = protocol_id.into();
        let protocol =
            ApplicationProtocol::new(protocol_id.clone()).map_err(AukiProtocolError::P2p)?;
        let candidates = {
            let _running = self
                .inner
                .lifecycle
                .enter()
                .ok_or(AukiProtocolError::Stopped)?;
            self.inner
                .routes
                .get(&expected_peer)
                .cloned()
                .unwrap_or_default()
        };
        if candidates.is_empty() {
            return Err(AukiProtocolError::NoRoutes(expected_peer));
        }

        let mut attempts = Vec::with_capacity(candidates.len());
        for route in candidates {
            match self
                .open_validated(expected_peer, route.clone(), protocol.clone())
                .await
            {
                Ok(stream) => return Ok(stream),
                Err(AukiProtocolError::P2p(error)) => {
                    let unsupported_protocol = matches!(
                        &error,
                        auki_p2p::Error::TargetedStream(
                            TargetedStreamError::UnsupportedProtocol { .. }
                        )
                    );
                    attempts.push(AukiProtocolRouteAttempt {
                        route,
                        error: error.to_string(),
                        unsupported_protocol,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Err(AukiProtocolError::AllRoutesFailed {
            peer_id: Box::new(expected_peer),
            protocol_id,
            attempts,
        })
    }

    /// Open the selected protocol through one exact untrusted route hint.
    pub async fn open_exact(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        protocol_id: impl Into<String>,
    ) -> Result<AuthenticatedRouteStream, AukiProtocolError> {
        let protocol =
            ApplicationProtocol::new(protocol_id.into()).map_err(AukiProtocolError::P2p)?;
        let route = canonicalize_candidate(expected_peer, route)?;
        self.open_validated(expected_peer, route, protocol).await
    }

    async fn open_validated(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        protocol: ApplicationProtocol,
    ) -> Result<AuthenticatedRouteStream, AukiProtocolError> {
        {
            let _running = self
                .inner
                .lifecycle
                .enter()
                .ok_or(AukiProtocolError::Stopped)?;
        }
        let requirements = SessionRequirements::new(self.inner.domain_id.to_string())
            .map_err(AukiProtocolError::P2p)?
            .with_expected_remote_peer_id(expected_peer);
        tokio::select! {
            biased;
            _ = self.inner.lifecycle.cancelled() => Err(AukiProtocolError::Stopped),
            result = self.inner.node.open_exact_route(
                expected_peer,
                ExactRoute::from_multiaddr(route),
                protocol,
                requirements,
            ) => result.map_err(AukiProtocolError::P2p),
        }
    }

    pub(crate) async fn shutdown_all(&self) -> Result<(), AukiProtocolError> {
        self.inner.lifecycle.fence();
        let servers = self
            .inner
            .registrations
            .lock()
            .drain()
            .filter_map(|(_, entry)| entry.server.lock().take())
            .collect::<Vec<_>>();
        let deadline = Instant::now() + PROTOCOL_SHUTDOWN_TIMEOUT;
        let mut first_error = None;
        for server in servers {
            match timeout_at(deadline, server.shutdown()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    first_error.get_or_insert(AukiProtocolError::P2p(error));
                }
                Err(_) => {
                    first_error.get_or_insert(AukiProtocolError::CleanupTimeout);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(crate) fn abort_all(&self) {
        self.inner.lifecycle.fence();
        let servers = self
            .inner
            .registrations
            .lock()
            .drain()
            .filter_map(|(_, entry)| entry.server.lock().take())
            .collect::<Vec<_>>();
        drop(servers);
    }
}

struct ProtocolRuntime {
    node: Node,
    domain_id: uuid::Uuid,
    routes: BTreeMap<PeerId, Vec<Multiaddr>>,
    lifecycle: ContextLifecycle,
    registrations: Mutex<HashMap<String, Arc<ProtocolEntry>>>,
    next_generation: AtomicU64,
}

struct ProtocolEntry {
    protocol_id: String,
    generation: u64,
    server: Mutex<Option<ApplicationProtocolServer>>,
}

/// RAII ownership of one mounted inbound protocol generation.
pub struct AukiProtocolRegistration {
    runtime: Weak<ProtocolRuntime>,
    entry: Arc<ProtocolEntry>,
    closed: bool,
}

impl AukiProtocolRegistration {
    /// Stop the handler and wait for all admitted streams to be canceled.
    pub async fn close(mut self) -> Result<(), AukiProtocolError> {
        let server = self.take_server();
        self.closed = true;
        match server {
            Some(server) => server.shutdown().await.map_err(AukiProtocolError::P2p),
            None => Ok(()),
        }
    }

    fn take_server(&self) -> Option<ApplicationProtocolServer> {
        if let Some(runtime) = self.runtime.upgrade() {
            let mut registrations = runtime.registrations.lock();
            if registrations
                .get(&self.entry.protocol_id)
                .is_some_and(|entry| entry.generation == self.entry.generation)
            {
                registrations.remove(&self.entry.protocol_id);
            }
        }
        self.entry.server.lock().take()
    }

    #[cfg(test)]
    pub(crate) fn owns_server_for_test(&self) -> bool {
        self.entry.server.lock().is_some()
    }
}

impl Drop for AukiProtocolRegistration {
    fn drop(&mut self) {
        if !self.closed {
            drop(self.take_server());
        }
    }
}

fn canonicalize_candidate(
    expected_peer: PeerId,
    route: Multiaddr,
) -> Result<Multiaddr, AukiProtocolError> {
    match validate_direct_route(&route, expected_peer) {
        Ok(route) => Ok(route),
        Err(direct) => canonicalize_circuit_route(&route, expected_peer)
            .map(|route| route.route)
            .map_err(|circuit| AukiProtocolError::InvalidRoute {
                peer_id: expected_peer,
                reason: format!("direct: {direct}; circuit: {circuit}"),
            }),
    }
}
