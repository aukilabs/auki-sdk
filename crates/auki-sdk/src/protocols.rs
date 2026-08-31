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
    ApplicationProtocol, ApplicationProtocolServer, ApplicationProtocolSpec,
    AuthenticatedApplicationStream, AuthenticatedRouteStream, ExactRoute, Multiaddr, Node, PeerId,
    SessionRequirements, TargetedStreamError, canonicalize_circuit_route, validate_direct_route,
};
use parking_lot::Mutex;
use tokio::time::{Instant, timeout_at};

use crate::context::ContextLifecycle;

const PROTOCOL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// One exact application protocol and its inbound resource bounds.
#[derive(Clone, Debug)]
pub struct AukiProtocolSpec {
    protocol_id: String,
    inner: ApplicationProtocolSpec,
}

impl AukiProtocolSpec {
    /// Validate one explicitly versioned protocol identifier and its handler bounds.
    pub fn new(
        protocol_id: impl Into<String>,
        max_concurrency: usize,
        max_frame_bytes: u32,
    ) -> Result<Self, AukiProtocolError> {
        let protocol_id = protocol_id.into();
        let protocol =
            ApplicationProtocol::new(protocol_id.clone()).map_err(AukiProtocolError::P2p)?;
        let inner = ApplicationProtocolSpec::new(protocol, max_concurrency, max_frame_bytes)
            .map_err(AukiProtocolError::P2p)?;
        Ok(Self { protocol_id, inner })
    }

    /// Exact libp2p application protocol identifier.
    pub fn protocol_id(&self) -> &str {
        &self.protocol_id
    }

    /// Maximum number of concurrently handled inbound streams.
    pub fn max_concurrency(&self) -> usize {
        self.inner.max_concurrency()
    }

    /// Maximum frame size the mounted codec is required to enforce.
    pub fn max_frame_bytes(&self) -> u32 {
        self.inner.max_frame_bytes()
    }
}

/// Mutually authenticated inbound application stream with its declared frame bound.
pub type AukiProtocolStream = AuthenticatedApplicationStream;

/// One failed route attempt made while opening an authenticated protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AukiProtocolRouteAttempt {
    /// Canonical route that was attempted.
    pub route: Multiaddr,
    /// Bounded transport diagnostic.
    pub error: String,
    /// Whether the peer was reached and rejected only this protocol identifier.
    pub unsupported_protocol: bool,
}

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
        self.inner.registrations.lock().clear();
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
}

impl Drop for AukiProtocolRegistration {
    fn drop(&mut self) {
        if !self.closed {
            drop(self.take_server());
        }
    }
}

/// Failure from the SDK-owned authenticated application-protocol surface.
#[derive(Debug, thiserror::Error)]
pub enum AukiProtocolError {
    /// The owning peer has begun or completed shutdown.
    #[error("the Auki peer protocol surface is stopped")]
    Stopped,
    /// The same exact protocol identifier is already mounted.
    #[error("authenticated protocol {0} is already registered")]
    DuplicateProtocol(String),
    /// The monotonic registration generation cannot advance further.
    #[error("protocol registration generation is exhausted")]
    GenerationExhausted,
    /// No static route is configured for the expected remote peer.
    #[error("no route is configured for expected peer {0}")]
    NoRoutes(PeerId),
    /// Every configured route failed.
    #[error(
        "all routes to expected peer {peer_id} failed for authenticated protocol {protocol_id}"
    )]
    AllRoutesFailed {
        /// Expected mutually authenticated remote peer.
        peer_id: Box<PeerId>,
        /// Exact requested protocol identifier.
        protocol_id: String,
        /// Stable direct-first attempt order.
        attempts: Vec<AukiProtocolRouteAttempt>,
    },
    /// One exact route hint is neither a canonical direct nor circuit route.
    #[error("route for expected peer {peer_id} is invalid: {reason}")]
    InvalidRoute {
        /// Expected mutually authenticated remote peer.
        peer_id: PeerId,
        /// Validation diagnostic.
        reason: String,
    },
    /// The underlying authenticated P2P runtime rejected the operation.
    #[error("authenticated protocol operation failed: {0}")]
    P2p(#[source] auki_p2p::Error),
    /// Ordered handler cleanup exceeded its fixed deadline.
    #[error("authenticated protocol cleanup timed out")]
    CleanupTimeout,
}

impl AukiProtocolError {
    /// Whether every route reached the peer and rejected only this protocol ID.
    pub fn all_routes_unsupported_protocol(&self) -> bool {
        matches!(
            self,
            Self::AllRoutesFailed { attempts, .. }
                if !attempts.is_empty()
                    && attempts.iter().all(|attempt| attempt.unsupported_protocol)
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_uses_the_shared_p2p_bounds() {
        assert!(AukiProtocolSpec::new("/example/echo/1.0.0", 0, 1).is_err());
        assert!(AukiProtocolSpec::new("/example/echo/1.0.0", 1, 0).is_err());
        let spec = AukiProtocolSpec::new("/example/echo/1.0.0", 4, 4096).unwrap();
        assert_eq!(spec.protocol_id(), "/example/echo/1.0.0");
        assert_eq!(spec.max_concurrency(), 4);
        assert_eq!(spec.max_frame_bytes(), 4096);
    }

    #[test]
    fn unsupported_fallback_requires_every_attempt() {
        let peer_id = auki_p2p::Identity::generate().peer_id();
        let attempt = AukiProtocolRouteAttempt {
            route: "/ip4/127.0.0.1/tcp/4001".parse().unwrap(),
            error: "unsupported".into(),
            unsupported_protocol: true,
        };
        let error = AukiProtocolError::AllRoutesFailed {
            peer_id: Box::new(peer_id),
            protocol_id: "/example/echo/1.0.0".into(),
            attempts: vec![attempt],
        };
        assert!(error.all_routes_unsupported_protocol());
    }
}
