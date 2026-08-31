use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    future::Future,
    rc::{Rc, Weak},
};

use auki_p2p::{
    ApplicationProtocol, ApplicationProtocolServer, ApplicationProtocolSpec,
    AuthenticatedApplicationStream, BrowserAuthenticatedRouteStream, BrowserNode, Multiaddr,
    PeerId, SessionRequirements, TargetedStreamError,
};
use futures::{FutureExt, pin_mut};
use tokio_util::sync::CancellationToken;

use crate::browser_peer_runtime::AukiPeerReachability;

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
    inner: Rc<ProtocolRuntime>,
}

impl AukiPeerProtocols {
    pub(crate) fn new(
        node: Rc<BrowserNode>,
        domain_id: uuid::Uuid,
        reachability: AukiPeerReachability,
    ) -> Self {
        Self {
            inner: Rc::new(ProtocolRuntime {
                node,
                domain_id,
                reachability,
                lifecycle: ProtocolLifecycle::new(),
                registrations: RefCell::new(HashMap::new()),
                next_generation: Cell::new(1),
            }),
        }
    }

    /// Register one exact inbound application protocol and bounded local handler.
    pub fn register<H, F>(
        &self,
        spec: AukiProtocolSpec,
        handler: H,
    ) -> Result<AukiProtocolRegistration, AukiProtocolError>
    where
        H: Fn(AukiProtocolStream) -> F + 'static,
        F: Future<Output = ()> + 'static,
    {
        if !self.inner.lifecycle.is_running() {
            return Err(AukiProtocolError::Stopped);
        }
        let mut registrations = self.inner.registrations.borrow_mut();
        if registrations.contains_key(&spec.protocol_id) {
            return Err(AukiProtocolError::DuplicateProtocol(spec.protocol_id));
        }
        let generation = self.inner.next_generation.get();
        let next_generation = generation
            .checked_add(1)
            .ok_or(AukiProtocolError::GenerationExhausted)?;
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
        self.inner.next_generation.set(next_generation);
        let entry = Rc::new(ProtocolEntry {
            protocol_id: spec.protocol_id.clone(),
            generation,
            server: RefCell::new(Some(server)),
        });
        registrations.insert(spec.protocol_id, Rc::clone(&entry));
        Ok(AukiProtocolRegistration {
            runtime: Rc::downgrade(&self.inner),
            entry,
            closed: false,
        })
    }

    /// Open the selected protocol through this browser's confirmed relay.
    pub async fn open(
        &self,
        expected_peer: PeerId,
        protocol_id: impl Into<String>,
    ) -> Result<BrowserAuthenticatedRouteStream, AukiProtocolError> {
        let protocol_id = protocol_id.into();
        let route = self.inner.reachability.wss_route_to(expected_peer);
        match self
            .open_exact_protocol(expected_peer, route.clone(), protocol_id.clone())
            .await
        {
            Ok(stream) => Ok(stream),
            Err(AukiProtocolError::P2p(error)) => {
                let unsupported_protocol = matches!(
                    &error,
                    auki_p2p::Error::TargetedStream(
                        TargetedStreamError::UnsupportedProtocol { .. }
                    )
                );
                Err(AukiProtocolError::AllRoutesFailed {
                    peer_id: Box::new(expected_peer),
                    protocol_id,
                    attempts: vec![AukiProtocolRouteAttempt {
                        route,
                        error: error.to_string(),
                        unsupported_protocol,
                    }],
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Open the selected protocol through one exact advertised WSS circuit.
    pub async fn open_exact(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        protocol_id: impl Into<String>,
    ) -> Result<BrowserAuthenticatedRouteStream, AukiProtocolError> {
        self.open_exact_protocol(expected_peer, route, protocol_id.into())
            .await
    }

    async fn open_exact_protocol(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        protocol_id: String,
    ) -> Result<BrowserAuthenticatedRouteStream, AukiProtocolError> {
        if !self.inner.lifecycle.is_running() {
            return Err(AukiProtocolError::Stopped);
        }
        let protocol = ApplicationProtocol::new(protocol_id).map_err(AukiProtocolError::P2p)?;
        let opening = self
            .inner
            .node
            .open_exact_route(expected_peer, route, protocol)
            .fuse();
        let cancelled = self.inner.lifecycle.cancelled().fuse();
        pin_mut!(opening, cancelled);
        futures::select_biased! {
            () = cancelled => Err(AukiProtocolError::Stopped),
            result = opening => result.map_err(AukiProtocolError::P2p),
        }
    }

    pub(crate) async fn shutdown_all(&self) -> Result<(), AukiProtocolError> {
        self.inner.lifecycle.stop();
        let servers = self
            .inner
            .registrations
            .borrow_mut()
            .drain()
            .filter_map(|(_, entry)| entry.server.borrow_mut().take())
            .collect::<Vec<_>>();
        let mut first_error = None;
        for server in servers {
            if let Err(error) = server.shutdown().await {
                first_error.get_or_insert(AukiProtocolError::P2p(error));
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(crate) fn abort_all(&self) {
        self.inner.lifecycle.stop();
        self.inner.registrations.borrow_mut().clear();
    }
}

struct ProtocolLifecycle {
    running: Cell<bool>,
    cancellation: CancellationToken,
}

impl ProtocolLifecycle {
    fn new() -> Self {
        Self {
            running: Cell::new(true),
            cancellation: CancellationToken::new(),
        }
    }

    fn is_running(&self) -> bool {
        self.running.get()
    }

    fn token(&self) -> &CancellationToken {
        &self.cancellation
    }

    async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }

    fn stop(&self) {
        self.running.set(false);
        self.cancellation.cancel();
    }
}

struct ProtocolRuntime {
    node: Rc<BrowserNode>,
    domain_id: uuid::Uuid,
    reachability: AukiPeerReachability,
    lifecycle: ProtocolLifecycle,
    registrations: RefCell<HashMap<String, Rc<ProtocolEntry>>>,
    next_generation: Cell<u64>,
}

struct ProtocolEntry {
    protocol_id: String,
    generation: u64,
    server: RefCell<Option<ApplicationProtocolServer>>,
}

/// RAII ownership of one mounted inbound protocol generation.
pub struct AukiProtocolRegistration {
    runtime: Weak<ProtocolRuntime>,
    entry: Rc<ProtocolEntry>,
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
            let mut registrations = runtime.registrations.borrow_mut();
            if registrations
                .get(&self.entry.protocol_id)
                .is_some_and(|entry| entry.generation == self.entry.generation)
            {
                registrations.remove(&self.entry.protocol_id);
            }
        }
        self.entry.server.borrow_mut().take()
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
    /// No route is configured for the expected remote peer.
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
    /// One exact route hint is invalid for the browser transport.
    #[error("route for expected peer {peer_id} is invalid: {reason}")]
    InvalidRoute {
        /// Expected remote peer.
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
}
