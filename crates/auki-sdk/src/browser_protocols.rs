use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    future::Future,
    rc::{Rc, Weak},
};

use auki_p2p::{
    ApplicationProtocol, ApplicationProtocolServer, BrowserAuthenticatedRouteStream, BrowserNode,
    Multiaddr, PeerId, SessionRequirements, TargetedStreamError,
};
use futures::{FutureExt, pin_mut};
use tokio_util::sync::CancellationToken;

use crate::{
    browser_peer_runtime::AukiPeerReachability,
    protocol_contract::{
        AukiProtocolError, AukiProtocolRouteAttempt, AukiProtocolSpec, AukiProtocolStream,
    },
};

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
