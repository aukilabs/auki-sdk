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
    Multiaddr, Node, PeerId, SessionRequirements, TargetedStreamError,
    canonicalize_candidate_route,
};
use parking_lot::Mutex;
use tokio::time::timeout;

use crate::{
    context::ContextLifecycle,
    protocol_contract::{
        AukiProtocolError, AukiProtocolRouteAttempt, AukiProtocolSpec, AukiProtocolStream,
    },
    served_protocols::{ServedProtocolSnapshot, ServedProtocolSnapshots},
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
        self.inner
            .served_protocols
            .replace(registrations.keys().cloned());
        Ok(AukiProtocolRegistration {
            runtime: Arc::downgrade(&self.inner),
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
        let servers = {
            let mut registrations = self.inner.registrations.lock();
            let servers = registrations
                .drain()
                .filter_map(|(_, entry)| entry.server.lock().take())
                .collect::<Vec<_>>();
            self.inner.served_protocols.replace(std::iter::empty());
            servers
        };
        shutdown_servers(servers).await
    }

    pub(crate) fn abort_all(&self) {
        self.inner.lifecycle.fence();
        let servers = {
            let mut registrations = self.inner.registrations.lock();
            let servers = registrations
                .drain()
                .filter_map(|(_, entry)| entry.server.lock().take())
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
    timeout_duration: Duration,
) -> Option<T> {
    timeout(timeout_duration, cleanup).await.ok()
}

struct ProtocolRuntime {
    node: Node,
    domain_id: uuid::Uuid,
    routes: BTreeMap<PeerId, Vec<Multiaddr>>,
    lifecycle: ContextLifecycle,
    registrations: Mutex<HashMap<String, Arc<ProtocolEntry>>>,
    next_generation: AtomicU64,
    served_protocols: ServedProtocolSnapshots,
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
            Some(server) => shutdown_servers(vec![server]).await,
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
                runtime
                    .served_protocols
                    .replace(registrations.keys().cloned());
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
    canonicalize_candidate_route(&route, expected_peer)
        .map(|candidate| candidate.into_route())
        .map_err(|error| AukiProtocolError::InvalidRoute {
            peer_id: expected_peer,
            reason: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use auki_p2p::DdsTokenVerifier;

    use super::*;

    const TEST_DDS_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----";

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn cleanup_deadline_drops_pending_cleanup() {
        let dropped = Arc::new(AtomicBool::new(false));
        let cleanup = {
            let probe = DropProbe(Arc::clone(&dropped));
            async move {
                let _probe = probe;
                std::future::pending::<()>().await;
            }
        };

        assert!(
            cleanup_before_deadline(cleanup, Duration::ZERO)
                .await
                .is_none()
        );
        assert!(dropped.load(Ordering::SeqCst));
    }

    fn protocol_spec(protocol_id: &str) -> AukiProtocolSpec {
        AukiProtocolSpec::new(protocol_id, 1, 32).unwrap()
    }

    fn test_protocols() -> (AukiPeerProtocols, Node) {
        let identity = auki_p2p::Identity::generate();
        let verifier = DdsTokenVerifier::from_es256_pem(TEST_DDS_PUBLIC_KEY).unwrap();
        let node = Node::start(identity, verifier, std::iter::empty::<Multiaddr>()).unwrap();
        let protocols = AukiPeerProtocols::new(
            node.clone(),
            uuid::Uuid::new_v4(),
            std::iter::empty(),
            ContextLifecycle::new(),
        );
        (protocols, node)
    }

    fn assert_snapshot(
        snapshots: &mut tokio::sync::watch::Receiver<ServedProtocolSnapshot>,
        revision: u64,
        protocol_ids: &[&str],
    ) {
        assert!(snapshots.has_changed().unwrap());
        let current = snapshots.borrow_and_update();
        assert_eq!(current.revision, revision);
        assert_eq!(
            current.protocol_ids,
            protocol_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn served_snapshot_tracks_mount_drop_close_remount_and_shutdown() {
        const ALPHA: &str = "/example/alpha/1.0.0";
        const ZULU: &str = "/example/zulu/1.0.0";

        let (protocols, node) = test_protocols();
        let mut snapshot = protocols.subscribe_served_protocols();
        assert_eq!(*snapshot.borrow(), ServedProtocolSnapshot::default());

        let zulu = protocols
            .register(protocol_spec(ZULU), |_stream| async {})
            .unwrap();
        assert_snapshot(&mut snapshot, 1, &[ZULU]);

        let alpha = protocols
            .register(protocol_spec(ALPHA), |_stream| async {})
            .unwrap();
        assert_snapshot(&mut snapshot, 2, &[ALPHA, ZULU]);

        assert!(matches!(
            protocols.register(protocol_spec(ALPHA), |_stream| async {}),
            Err(AukiProtocolError::DuplicateProtocol(protocol)) if protocol == ALPHA
        ));
        assert!(!snapshot.has_changed().unwrap());

        drop(zulu);
        assert_snapshot(&mut snapshot, 3, &[ALPHA]);

        alpha.close().await.unwrap();
        assert_snapshot(&mut snapshot, 4, &[]);

        let remounted = protocols
            .register(protocol_spec(ALPHA), |_stream| async {})
            .unwrap();
        assert_snapshot(&mut snapshot, 5, &[ALPHA]);

        protocols.shutdown_all().await.unwrap();
        assert_snapshot(&mut snapshot, 6, &[]);

        remounted.close().await.unwrap();
        assert!(!snapshot.has_changed().unwrap());
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn served_snapshot_is_cleared_by_abort() {
        let (protocols, node) = test_protocols();
        let mut snapshot = protocols.subscribe_served_protocols();
        let registration = protocols
            .register(protocol_spec("/example/abort/1.0.0"), |_stream| async {})
            .unwrap();
        assert_snapshot(&mut snapshot, 1, &["/example/abort/1.0.0"]);

        protocols.abort_all();
        assert_snapshot(&mut snapshot, 2, &[]);
        drop(registration);
        assert!(!snapshot.has_changed().unwrap());
        node.shutdown().await.unwrap();
    }
}
