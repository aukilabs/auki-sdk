use std::{
    collections::HashMap,
    future::Future,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

use auki_p2p::{
    ApplicationProtocol, AuthenticatedRouteStream, AuthenticatedStream, ExactRoute, Multiaddr,
    PeerId, SessionRequirements, TargetedStreamError,
};
use futures::{
    FutureExt,
    io::{AsyncRead, AsyncWrite},
};
use parking_lot::Mutex;
use tokio::{
    sync::{Mutex as AsyncMutex, watch},
    task::{AbortHandle, JoinHandle, JoinSet},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use super::{
    RuntimeAccess, RuntimeFailureSignal,
    routes::{DomainRoutes, DomainRoutesError},
};

pub(crate) const DOMAIN_PROTOCOL_MAX_CONCURRENCY: usize = 1_024;
pub(crate) const DOMAIN_PROTOCOL_MAX_FRAME_BYTES: u32 = 64 * 1024 * 1024;

/// One authenticated application ID and its explicit handler bounds.
#[derive(Clone, Debug)]
pub(crate) struct DomainProtocolSpec {
    protocol_id: String,
    protocol: ApplicationProtocol,
    max_concurrency: usize,
    max_frame_bytes: u32,
}

impl DomainProtocolSpec {
    pub(crate) fn new(
        protocol_id: impl Into<String>,
        max_concurrency: usize,
        max_frame_bytes: u32,
    ) -> Result<Self, DomainProtocolError> {
        if !(1..=DOMAIN_PROTOCOL_MAX_CONCURRENCY).contains(&max_concurrency) {
            return Err(DomainProtocolError::InvalidConcurrency {
                maximum: DOMAIN_PROTOCOL_MAX_CONCURRENCY,
            });
        }
        if !(1..=DOMAIN_PROTOCOL_MAX_FRAME_BYTES).contains(&max_frame_bytes) {
            return Err(DomainProtocolError::InvalidFrameBound {
                maximum: DOMAIN_PROTOCOL_MAX_FRAME_BYTES,
            });
        }
        let protocol_id = protocol_id.into();
        let protocol =
            ApplicationProtocol::new(protocol_id.clone()).map_err(DomainProtocolError::P2p)?;
        Ok(Self {
            protocol_id,
            protocol,
            max_concurrency,
            max_frame_bytes,
        })
    }

    pub(crate) fn protocol_id(&self) -> &str {
        &self.protocol_id
    }

    pub(crate) fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    pub(crate) fn max_frame_bytes(&self) -> u32 {
        self.max_frame_bytes
    }
}

/// Inbound mutually authenticated stream plus the declared codec frame bound.
///
/// The generic runtime cannot infer an application's framing. Retained and
/// third-party handlers must use a bounded codec read no larger than this
/// value; P06's retained codecs enforce their own smaller wire bounds.
pub(crate) struct DomainProtocolStream {
    stream: AuthenticatedStream,
    max_frame_bytes: u32,
}

impl DomainProtocolStream {
    pub(crate) fn remote_peer(&self) -> &auki_p2p::AuthenticatedPeer {
        self.stream.remote_peer()
    }

    pub(crate) fn max_frame_bytes(&self) -> u32 {
        self.max_frame_bytes
    }
}

impl AsyncRead for DomainProtocolStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for DomainProtocolStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_close(context)
    }
}

#[derive(Clone)]
pub(crate) struct DomainProtocols {
    access: Arc<RuntimeAccess>,
    routes: DomainRoutes,
    registry: Arc<ProtocolRegistry>,
}

impl DomainProtocols {
    pub(super) fn new(
        access: Arc<RuntimeAccess>,
        routes: DomainRoutes,
        fatal: tokio::sync::mpsc::UnboundedSender<RuntimeFailureSignal>,
    ) -> Self {
        let registry = Arc::new(ProtocolRegistry {
            entries: Mutex::new(HashMap::new()),
            next_generation: AtomicU64::new(1),
            lifecycle: access.lifecycle.clone(),
            fatal,
        });
        Self {
            access,
            routes,
            registry,
        }
    }

    pub(crate) fn register<H, F>(
        &self,
        spec: DomainProtocolSpec,
        handler: H,
    ) -> Result<DomainProtocolRegistration, DomainProtocolError>
    where
        H: Fn(DomainProtocolStream) -> F + Send + Sync + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        if self.access.lifecycle.is_cancelled() {
            return Err(DomainProtocolError::Stopped);
        }
        let node = self
            .access
            .node()
            .map_err(|_| DomainProtocolError::Stopped)?;
        let requirements = SessionRequirements::new(self.access.domain_id.to_string())
            .map_err(DomainProtocolError::P2p)?;
        let incoming =
            node.accept(spec.protocol.clone(), requirements)
                .map_err(|error| match error {
                    auki_p2p::Error::ProtocolAlreadyRegistered => {
                        DomainProtocolError::DuplicateProtocol(spec.protocol_id.clone())
                    }
                    error => DomainProtocolError::P2p(error),
                })?;

        let mut entries = self.registry.entries.lock();
        // Serialize the final lifecycle fence with shutdown_all's registry
        // snapshot. Either registration inserts before leave snapshots it, or
        // observes cancellation and cannot create an unowned host afterward.
        if self.access.lifecycle.is_cancelled() {
            drop(incoming);
            return Err(DomainProtocolError::Stopped);
        }
        if entries.contains_key(&spec.protocol_id) {
            drop(incoming);
            return Err(DomainProtocolError::DuplicateProtocol(spec.protocol_id));
        }
        let generation = self
            .registry
            .next_generation
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |generation| {
                generation.checked_add(1)
            })
            .map_err(|_| DomainProtocolError::GenerationExhausted)?;
        let cancel = self.registry.lifecycle.child_token();
        let (completed, _) = watch::channel(false);
        let entry = Arc::new(RegisteredProtocol {
            protocol_id: spec.protocol_id.clone(),
            generation,
            cancel: cancel.clone(),
            completed,
            task: AsyncMutex::new(None),
            task_abort: Mutex::new(None),
        });
        entries.insert(spec.protocol_id.clone(), Arc::clone(&entry));

        let handler = Arc::new(handler);
        let registry = Arc::downgrade(&self.registry);
        let task_entry = Arc::clone(&entry);
        // Construct the completion guard before spawning. If the task is
        // aborted before its first poll, dropping the captured future still
        // completes every outliving registration handle.
        let completion = ProtocolTaskCompletionGuard {
            entry: Arc::clone(&task_entry),
            registry: registry.clone(),
        };
        let task = tokio::spawn(async move {
            let _completion = completion;
            let _outcome =
                AssertUnwindSafe(run_protocol_host(incoming, spec, cancel.clone(), handler))
                    .catch_unwind()
                    .await;
            let expected_stop = cancel.is_cancelled();
            if !expected_stop && let Some(registry) = registry.upgrade() {
                let _ = registry
                    .fatal
                    .send(RuntimeFailureSignal::ProtocolHostStopped);
            }
        });
        *entry.task_abort.lock() = Some(task.abort_handle());
        *entry
            .task
            .try_lock()
            .expect("new protocol task slot cannot be contended") = Some(task);
        drop(entries);

        Ok(DomainProtocolRegistration {
            registry: Arc::downgrade(&self.registry),
            entry,
            closed: false,
        })
    }

    pub(crate) async fn open(
        &self,
        expected_peer: PeerId,
        protocol_id: impl Into<String>,
    ) -> Result<AuthenticatedRouteStream, DomainProtocolError> {
        let protocol_id = protocol_id.into();
        let protocol =
            ApplicationProtocol::new(protocol_id.clone()).map_err(DomainProtocolError::P2p)?;
        let candidates = self.routes.candidates(expected_peer)?;
        if candidates.is_empty() {
            return Err(DomainProtocolError::NoRoutes(expected_peer));
        }
        let mut attempts = Vec::with_capacity(candidates.len());
        for route in candidates {
            match self
                .open_validated(expected_peer, route.clone(), protocol.clone())
                .await
            {
                Ok(stream) => return Ok(stream),
                Err(DomainProtocolError::P2p(error)) => {
                    let unsupported_protocol = matches!(
                        &error,
                        auki_p2p::Error::TargetedStream(
                            TargetedStreamError::UnsupportedProtocol { .. }
                        )
                    );
                    attempts.push(DomainRouteAttempt {
                        route,
                        error: error.to_string(),
                        unsupported_protocol,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Err(DomainProtocolError::AllRoutesFailed {
            peer_id: Box::new(expected_peer),
            protocol_id,
            attempts,
        })
    }

    pub(crate) async fn open_exact(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        protocol_id: impl Into<String>,
    ) -> Result<AuthenticatedRouteStream, DomainProtocolError> {
        let protocol =
            ApplicationProtocol::new(protocol_id.into()).map_err(DomainProtocolError::P2p)?;
        let route = DomainRoutes::canonicalize_candidate(expected_peer, route)?;
        self.open_validated(expected_peer, route, protocol).await
    }

    async fn open_validated(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        protocol: ApplicationProtocol,
    ) -> Result<AuthenticatedRouteStream, DomainProtocolError> {
        let node = self
            .access
            .node()
            .map_err(|_| DomainProtocolError::Stopped)?;
        let requirements = SessionRequirements::new(self.access.domain_id.to_string())
            .map_err(DomainProtocolError::P2p)?
            .with_expected_remote_peer_id(expected_peer);
        tokio::select! {
            biased;
            _ = self.access.lifecycle.cancelled() => Err(DomainProtocolError::Stopped),
            result = node.open_exact_route(
                expected_peer,
                ExactRoute::from_multiaddr(route),
                protocol,
                requirements,
            ) => result.map_err(DomainProtocolError::P2p),
        }
    }

    pub(super) async fn shutdown_all(&self, deadline: Instant) -> Result<(), DomainProtocolError> {
        let entries = self
            .registry
            .entries
            .lock()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for entry in &entries {
            entry.cancel.cancel();
        }
        let mut first_error = None;
        for entry in entries {
            let mut owned = entry.task.lock().await;
            if let Some(task) = owned.as_mut() {
                match tokio::time::timeout_at(deadline, &mut *task).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) if error.is_cancelled() => {}
                    Ok(Err(error)) => {
                        first_error.get_or_insert(DomainProtocolError::Task(error));
                    }
                    Err(_) => {
                        task.abort();
                        let _ = task.await;
                        first_error.get_or_insert(DomainProtocolError::CleanupTimeout);
                    }
                }
                owned.take();
            }
            // Every explicit-shutdown outcome is a teardown barrier for stale
            // registration handles, including timeout/abort and task failure.
            entry.completed.send_replace(true);
        }
        self.registry.entries.lock().clear();
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(super) fn abort_all(&self) {
        let entries = self
            .registry
            .entries
            .lock()
            .drain()
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>();
        for entry in entries {
            entry.cancel.cancel();
            entry.abort();
        }
    }

    #[cfg(test)]
    pub(super) fn fail_host_for_test(&self) {
        let _ = self
            .registry
            .fatal
            .send(RuntimeFailureSignal::ProtocolHostStopped);
    }

    #[cfg(test)]
    pub(super) fn insert_stubborn_host_for_test(&self) -> DomainProtocolRegistration {
        let protocol_id = "/auki/auth/1/cleanup-test/1.0.0".to_owned();
        let generation = self
            .registry
            .next_generation
            .fetch_add(1, Ordering::Relaxed);
        let (completed, _) = watch::channel(false);
        let entry = Arc::new(RegisteredProtocol {
            protocol_id: protocol_id.clone(),
            generation,
            cancel: self.registry.lifecycle.child_token(),
            completed,
            task: AsyncMutex::new(None),
            task_abort: Mutex::new(None),
        });
        self.registry
            .entries
            .lock()
            .insert(protocol_id, Arc::clone(&entry));
        let task_entry = Arc::clone(&entry);
        let registry = Arc::downgrade(&self.registry);
        let completion = ProtocolTaskCompletionGuard {
            entry: task_entry,
            registry,
        };
        let task = tokio::spawn(async move {
            let _completion = completion;
            std::future::pending::<()>().await;
        });
        *entry.task_abort.lock() = Some(task.abort_handle());
        *entry
            .task
            .try_lock()
            .expect("new test protocol task slot cannot be contended") = Some(task);
        DomainProtocolRegistration {
            registry: Arc::downgrade(&self.registry),
            entry,
            closed: false,
        }
    }
}

struct ProtocolRegistry {
    entries: Mutex<HashMap<String, Arc<RegisteredProtocol>>>,
    next_generation: AtomicU64,
    lifecycle: CancellationToken,
    fatal: tokio::sync::mpsc::UnboundedSender<RuntimeFailureSignal>,
}

impl ProtocolRegistry {
    fn cancel(&self, protocol_id: &str, generation: u64) {
        if let Some(entry) = self
            .entries
            .lock()
            .get(protocol_id)
            .filter(|entry| entry.generation == generation)
            .cloned()
        {
            entry.cancel.cancel();
        }
    }

    fn complete(&self, protocol_id: &str, generation: u64) {
        let mut entries = self.entries.lock();
        if entries
            .get(protocol_id)
            .is_some_and(|entry| entry.generation == generation)
        {
            entries.remove(protocol_id);
        }
    }
}

struct RegisteredProtocol {
    protocol_id: String,
    generation: u64,
    cancel: CancellationToken,
    completed: watch::Sender<bool>,
    task: AsyncMutex<Option<JoinHandle<()>>>,
    task_abort: Mutex<Option<AbortHandle>>,
}

impl RegisteredProtocol {
    fn abort(&self) {
        self.cancel.cancel();
        if let Some(task) = self.task_abort.lock().as_ref() {
            task.abort();
        }
    }
}

struct ProtocolTaskCompletionGuard {
    entry: Arc<RegisteredProtocol>,
    registry: Weak<ProtocolRegistry>,
}

impl Drop for ProtocolTaskCompletionGuard {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.complete(&self.entry.protocol_id, self.entry.generation);
        }
        // Completion is a deregistration barrier: wake registration handles
        // only after the old generation no longer occupies its protocol ID.
        self.entry.completed.send_replace(true);
    }
}

pub(crate) struct DomainProtocolRegistration {
    registry: Weak<ProtocolRegistry>,
    entry: Arc<RegisteredProtocol>,
    closed: bool,
}

impl DomainProtocolRegistration {
    pub(crate) async fn close(mut self) {
        let mut completed = self.entry.completed.subscribe();
        self.cancel();
        while !*completed.borrow_and_update() && completed.changed().await.is_ok() {}
        self.closed = true;
    }

    fn cancel(&self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.cancel(&self.entry.protocol_id, self.entry.generation);
        } else {
            self.entry.cancel.cancel();
        }
    }
}

impl Drop for DomainProtocolRegistration {
    fn drop(&mut self) {
        if !self.closed {
            self.cancel();
        }
    }
}

async fn run_protocol_host<H, F>(
    mut incoming: auki_p2p::IncomingAuthenticatedStreams,
    spec: DomainProtocolSpec,
    cancel: CancellationToken,
    handler: Arc<H>,
) where
    H: Fn(DomainProtocolStream) -> F + Send + Sync + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    let mut handlers = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            completed = handlers.join_next(), if !handlers.is_empty() => {
                if let Some(Err(error)) = completed {
                    tracing::warn!(%error, protocol = %spec.protocol_id, "authenticated Domain protocol handler failed");
                }
            }
            accepted = incoming.accept(), if handlers.len() < spec.max_concurrency => {
                let Some(accepted) = accepted else { break; };
                let stream = match accepted {
                    Ok(stream) => stream,
                    Err(error) => {
                        tracing::warn!(%error, protocol = %spec.protocol_id, "authenticated Domain protocol session was rejected");
                        continue;
                    }
                };
                let handler = Arc::clone(&handler);
                let max_frame_bytes = spec.max_frame_bytes;
                handlers.spawn(async move {
                    handler(DomainProtocolStream { stream, max_frame_bytes }).await;
                });
            }
        }
    }
    handlers.abort_all();
    while handlers.join_next().await.is_some() {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DomainRouteAttempt {
    pub(crate) route: Multiaddr,
    pub(crate) error: String,
    pub(crate) unsupported_protocol: bool,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DomainProtocolError {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("protocol concurrency must be between 1 and {maximum}")]
    InvalidConcurrency { maximum: usize },
    #[error("protocol frame bound must be between 1 and {maximum} bytes")]
    InvalidFrameBound { maximum: u32 },
    #[error("authenticated protocol {0} is already registered")]
    DuplicateProtocol(String),
    #[error("protocol registration generation is exhausted")]
    GenerationExhausted,
    #[error("no route is configured for expected peer {0}")]
    NoRoutes(PeerId),
    #[error(
        "all routes to expected peer {peer_id} failed for authenticated protocol {protocol_id}"
    )]
    AllRoutesFailed {
        peer_id: Box<PeerId>,
        protocol_id: String,
        attempts: Vec<DomainRouteAttempt>,
    },
    #[error(transparent)]
    Routes(#[from] DomainRoutesError),
    #[error("authenticated protocol operation failed: {0}")]
    P2p(#[source] auki_p2p::Error),
    #[error("authenticated protocol task failed: {0}")]
    Task(#[source] tokio::task::JoinError),
    #[error("authenticated protocol cleanup timed out")]
    CleanupTimeout,
}

impl DomainProtocolError {
    /// Whether every configured route reached the expected authenticated peer
    /// and that peer rejected only the requested application protocol.
    ///
    /// Retained version fallback may use this signal. Authentication, routing,
    /// transport, and timeout failures must never be mistaken for version
    /// negotiation.
    pub(crate) fn all_routes_unsupported_protocol(&self) -> bool {
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
    fn protocol_spec_rejects_zero_and_over_limit_bounds() {
        let id = "/auki/auth/1/example/1.0.0";
        assert!(matches!(
            DomainProtocolSpec::new(id, 0, 1),
            Err(DomainProtocolError::InvalidConcurrency { .. })
        ));
        assert!(matches!(
            DomainProtocolSpec::new(id, DOMAIN_PROTOCOL_MAX_CONCURRENCY + 1, 1),
            Err(DomainProtocolError::InvalidConcurrency { .. })
        ));
        assert!(matches!(
            DomainProtocolSpec::new(id, 1, 0),
            Err(DomainProtocolError::InvalidFrameBound { .. })
        ));
        assert!(matches!(
            DomainProtocolSpec::new(id, 1, DOMAIN_PROTOCOL_MAX_FRAME_BYTES + 1),
            Err(DomainProtocolError::InvalidFrameBound { .. })
        ));
        let exact = DomainProtocolSpec::new(
            id,
            DOMAIN_PROTOCOL_MAX_CONCURRENCY,
            DOMAIN_PROTOCOL_MAX_FRAME_BYTES,
        )
        .unwrap();
        assert_eq!(exact.protocol_id(), id);
        assert_eq!(exact.max_concurrency(), DOMAIN_PROTOCOL_MAX_CONCURRENCY);
        assert_eq!(exact.max_frame_bytes(), DOMAIN_PROTOCOL_MAX_FRAME_BYTES);
    }

    #[test]
    fn version_fallback_requires_every_route_to_reject_only_the_protocol() {
        let peer_id = PeerId::random();
        let route: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let error = |attempts| DomainProtocolError::AllRoutesFailed {
            peer_id: Box::new(peer_id),
            protocol_id: "/auki/auth/1/example/1.0.0".into(),
            attempts,
        };
        let attempt = |unsupported_protocol| DomainRouteAttempt {
            route: route.clone(),
            error: "test".into(),
            unsupported_protocol,
        };

        assert!(error(vec![attempt(true), attempt(true)]).all_routes_unsupported_protocol());
        assert!(!error(Vec::new()).all_routes_unsupported_protocol());
        assert!(!error(vec![attempt(true), attempt(false)]).all_routes_unsupported_protocol());
    }
}
