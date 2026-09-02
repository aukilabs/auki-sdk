use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    future::Future,
    rc::{Rc, Weak},
    time::Duration,
};

use auki_p2p::{
    ApplicationProtocol, ApplicationProtocolServer, BrowserAuthenticatedRouteStream, BrowserNode,
    CandidateRouteKind, Multiaddr, PeerId, SessionRequirements, canonicalize_candidate_route,
};
use futures::{FutureExt, pin_mut};
use futures_timer::Delay;
use tokio_util::sync::CancellationToken;

use crate::{
    protocol_contract::{AukiProtocolError, AukiProtocolSpec, AukiProtocolStream},
    served_protocols::{ServedProtocolSnapshot, ServedProtocolSnapshots},
};

const PROTOCOL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct AukiPeerProtocols {
    inner: Rc<ProtocolRuntime>,
}

impl AukiPeerProtocols {
    pub(crate) fn new(node: Rc<BrowserNode>, domain_id: uuid::Uuid) -> Self {
        Self {
            inner: Rc::new(ProtocolRuntime {
                node,
                domain_id,
                lifecycle: ProtocolLifecycle::new(),
                registrations: RefCell::new(HashMap::new()),
                next_generation: Cell::new(1),
                served_protocols: ServedProtocolSnapshots::new(),
            }),
        }
    }

    /// Stable local libp2p Peer ID shared by every mounted protocol.
    pub fn peer_id(&self) -> PeerId {
        self.inner.node.peer_id()
    }

    /// Exact authenticated DDS Domain UUID shared by every mounted protocol.
    pub fn domain_id(&self) -> uuid::Uuid {
        self.inner.domain_id
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
        self.inner
            .served_protocols
            .replace(registrations.keys().cloned());
        Ok(AukiProtocolRegistration {
            runtime: Rc::downgrade(&self.inner),
            entry,
            closed: false,
        })
    }

    #[allow(
        dead_code,
        reason = "consumed by the DDS publisher in discovery integration"
    )]
    pub(crate) fn subscribe_served_protocols(
        &self,
    ) -> tokio::sync::watch::Receiver<ServedProtocolSnapshot> {
        self.inner.served_protocols.subscribe()
    }

    /// Open the selected protocol through one exact advertised WSS circuit.
    pub async fn open_exact(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        protocol_id: impl Into<String>,
    ) -> Result<BrowserAuthenticatedRouteStream, AukiProtocolError> {
        if !self.inner.lifecycle.is_running() {
            return Err(AukiProtocolError::Stopped);
        }
        let route = canonicalize_candidate_route(&route, expected_peer).map_err(|error| {
            AukiProtocolError::InvalidRoute {
                peer_id: expected_peer,
                reason: error.to_string(),
            }
        })?;
        if route.kind() != CandidateRouteKind::RelayWss {
            return Err(AukiProtocolError::InvalidRoute {
                peer_id: expected_peer,
                reason: "browser peers require an exact WSS relay route".into(),
            });
        }
        let protocol =
            ApplicationProtocol::new(protocol_id.into()).map_err(AukiProtocolError::P2p)?;
        let opening = self
            .inner
            .node
            .open_exact_route(expected_peer, route.into_route(), protocol)
            .fuse();
        let cancelled = self.inner.lifecycle.cancelled().fuse();
        pin_mut!(opening, cancelled);
        futures::select_biased! {
            () = cancelled => Err(AukiProtocolError::Stopped),
            result = opening => result.map_err(AukiProtocolError::P2p),
        }
    }

    pub(crate) async fn shutdown_all(&self) -> Result<(), AukiProtocolError> {
        self.begin_shutdown();
        let servers = {
            let mut registrations = self.inner.registrations.borrow_mut();
            let servers = registrations
                .drain()
                .filter_map(|(_, entry)| entry.server.borrow_mut().take())
                .collect::<Vec<_>>();
            self.inner.served_protocols.replace(std::iter::empty());
            servers
        };
        shutdown_servers(servers).await
    }

    pub(crate) fn begin_shutdown(&self) {
        self.inner.lifecycle.stop();
    }

    pub(crate) fn abort_all(&self) {
        self.begin_shutdown();
        let servers = {
            let mut registrations = self.inner.registrations.borrow_mut();
            let servers = registrations
                .drain()
                .filter_map(|(_, entry)| entry.server.borrow_mut().take())
                .collect::<Vec<_>>();
            self.inner.served_protocols.replace(std::iter::empty());
            servers
        };
        drop(servers);
    }
}

async fn shutdown_servers(
    servers: Vec<ApplicationProtocolServer>,
) -> Result<(), AukiProtocolError> {
    let shutdown = async move {
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
    };
    cleanup_before_deadline(shutdown, PROTOCOL_SHUTDOWN_TIMEOUT)
        .await
        .unwrap_or(Err(AukiProtocolError::CleanupTimeout))
}

async fn cleanup_before_deadline<T>(
    cleanup: impl Future<Output = T>,
    timeout: Duration,
) -> Option<T> {
    let cleanup = cleanup.fuse();
    let timeout = Delay::new(timeout).fuse();
    pin_mut!(cleanup, timeout);
    futures::select_biased! {
        result = cleanup => Some(result),
        () = timeout => None,
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
    lifecycle: ProtocolLifecycle,
    registrations: RefCell<HashMap<String, Rc<ProtocolEntry>>>,
    next_generation: Cell<u64>,
    served_protocols: ServedProtocolSnapshots,
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
            Some(server) => shutdown_servers(vec![server]).await,
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
                runtime
                    .served_protocols
                    .replace(registrations.keys().cloned());
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

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    struct DropProbe(Rc<Cell<bool>>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[wasm_bindgen_test(async)]
    async fn cleanup_deadline_drops_pending_cleanup() {
        let dropped = Rc::new(Cell::new(false));
        let cleanup = {
            let dropped = Rc::clone(&dropped);
            async move {
                let _probe = DropProbe(dropped);
                futures::future::pending::<()>().await;
            }
        };

        assert!(
            cleanup_before_deadline(cleanup, Duration::ZERO)
                .await
                .is_none()
        );
        assert!(dropped.get());
    }

    #[wasm_bindgen_test]
    fn protocol_lifecycle_stop_is_synchronous_and_idempotent() {
        let lifecycle = ProtocolLifecycle::new();
        assert!(lifecycle.is_running());
        assert!(!lifecycle.token().is_cancelled());

        lifecycle.stop();
        lifecycle.stop();

        assert!(!lifecycle.is_running());
        assert!(lifecycle.token().is_cancelled());
    }

    #[wasm_bindgen_test]
    fn served_snapshots_are_sorted_revisioned_and_retained_for_browser_consumers() {
        let snapshots = ServedProtocolSnapshots::new();
        let mut current = snapshots.subscribe();
        assert_eq!(*current.borrow(), ServedProtocolSnapshot::default());

        snapshots.replace([
            "/example/zulu/1.0.0".to_owned(),
            "/example/alpha/1.0.0".to_owned(),
        ]);
        assert!(current.has_changed().unwrap());
        let observed = current.borrow_and_update();
        assert_eq!(observed.revision, 1);
        assert_eq!(
            observed.protocol_ids,
            ["/example/alpha/1.0.0", "/example/zulu/1.0.0"]
        );
        drop(observed);

        snapshots.replace([
            "/example/alpha/1.0.0".to_owned(),
            "/example/alpha/1.0.0".to_owned(),
            "/example/zulu/1.0.0".to_owned(),
        ]);
        assert!(!current.has_changed().unwrap());

        snapshots.replace(["/example/alpha/1.0.0".to_owned()]);
        assert!(current.has_changed().unwrap());
        let observed = current.borrow_and_update();
        assert_eq!(observed.revision, 2);
        assert_eq!(observed.protocol_ids, ["/example/alpha/1.0.0"]);
        drop(observed);

        snapshots.replace([
            "/example/zulu/1.0.0".to_owned(),
            "/example/alpha/1.0.0".to_owned(),
        ]);
        assert!(current.has_changed().unwrap());
        assert_eq!(current.borrow_and_update().revision, 3);

        snapshots.replace(std::iter::empty());
        assert!(current.has_changed().unwrap());
        let observed = current.borrow_and_update();
        assert_eq!(observed.revision, 4);
        assert!(observed.protocol_ids.is_empty());
    }
}
