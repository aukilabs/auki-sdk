use std::{future::Future, sync::Arc};

use auki_domain::{
    DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
    DomainProtocols,
};
use auki_p2p::{
    AuthenticatedRouteStream, Multiaddr, PeerId, RouteCatalog, RouteCatalogError,
    RouteCatalogStatus, RouteFence, RouteSnapshot,
};
use parking_lot::{Mutex, MutexGuard};
use tokio::sync::watch;
use uuid::Uuid;

use crate::authorization::{AukiPeerAuthorization, AuthorizationSnapshotSource};

#[derive(Clone)]
pub(crate) struct ContextLifecycle {
    running: Arc<Mutex<bool>>,
}

impl ContextLifecycle {
    pub(crate) fn new() -> Self {
        Self {
            running: Arc::new(Mutex::new(true)),
        }
    }

    pub(crate) fn enter(&self) -> Option<MutexGuard<'_, bool>> {
        let guard = self.running.lock();
        (*guard).then_some(guard)
    }

    pub(crate) fn fence(&self) {
        *self.running.lock() = false;
    }
}

/// Read-only view of the local routes published by one facade runtime.
///
/// The facade runtime alone can replace direct routes or publish and tombstone
/// relay routes. Protocol adapters can take snapshots and atomically bind
/// application state to an exact route revision without receiving the mutable
/// [`RouteCatalog`] capability.
#[derive(Clone)]
pub struct AukiPeerRoutes {
    catalog: RouteCatalog,
    lifecycle: ContextLifecycle,
}

impl AukiPeerRoutes {
    pub(crate) fn new(catalog: RouteCatalog, lifecycle: ContextLifecycle) -> Self {
        Self { catalog, lifecycle }
    }

    /// Return the current immutable direct and confirmed-relay route set.
    pub fn snapshot(&self) -> Result<RouteSnapshot, AukiPeerRoutesError> {
        let _running = self.lifecycle.enter().ok_or(AukiPeerRoutesError::Stopped)?;
        self.catalog.snapshot().map_err(Into::into)
    }

    /// Return the current route counts and monotonic revision.
    pub fn status(&self) -> Result<RouteCatalogStatus, AukiPeerRoutesError> {
        let _running = self.lifecycle.enter().ok_or(AukiPeerRoutesError::Stopped)?;
        self.catalog.status().map_err(Into::into)
    }

    /// Subscribe to route revision and count changes, including shutdown clearing.
    ///
    /// A subscription is observational and does not prove that the facade is
    /// still running. Runtime shutdown fences active operations first, then
    /// clears the catalog so existing receivers can observe the empty route set.
    pub fn subscribe(&self) -> watch::Receiver<RouteCatalogStatus> {
        self.catalog.subscribe()
    }

    /// Commit application state only while an observed route snapshot remains exact.
    ///
    /// The callback runs synchronously while the route revision is fenced. It
    /// must not block, await, or re-enter any handle cloned from the same
    /// protocol context. `Ok(None)` means the revision or at least one
    /// requested relay fence changed before commit.
    pub fn commit_if_current<T>(
        &self,
        expected_revision: u64,
        expected_fences: &[RouteFence],
        commit: impl FnOnce(&RouteSnapshot) -> T,
    ) -> Result<Option<T>, AukiPeerRoutesError> {
        let _running = self.lifecycle.enter().ok_or(AukiPeerRoutesError::Stopped)?;
        self.catalog
            .commit_if_current(expected_revision, expected_fences, commit)
            .map_err(Into::into)
    }
}

/// Rejected access through a read-only local route view.
#[derive(Debug, thiserror::Error)]
pub enum AukiPeerRoutesError {
    /// The facade has entered ordered shutdown.
    #[error("the Auki peer route view is stopped")]
    Stopped,
    /// The underlying bounded route catalog rejected the read or commit.
    #[error("the Auki peer route catalog rejected the operation")]
    Catalog(#[from] RouteCatalogError),
}

/// Lifecycle-fenced authenticated custom-protocol surface.
///
/// A call that passes the synchronous lifecycle check before shutdown is an
/// admitted in-flight operation and is subsequently bounded by Domain
/// teardown. Calls that begin after the facade fence are rejected without
/// touching the Domain runtime.
#[derive(Clone)]
pub struct AukiPeerProtocols {
    protocols: DomainProtocols,
    lifecycle: ContextLifecycle,
}

impl AukiPeerProtocols {
    fn new(protocols: DomainProtocols, lifecycle: ContextLifecycle) -> Self {
        Self {
            protocols,
            lifecycle,
        }
    }

    /// Register one exact inbound application protocol and bounded handler.
    pub fn register<H, F>(
        &self,
        spec: DomainProtocolSpec,
        handler: H,
    ) -> Result<DomainProtocolRegistration, AukiPeerProtocolsError>
    where
        H: Fn(DomainProtocolStream) -> F + Send + Sync + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        let _running = self
            .lifecycle
            .enter()
            .ok_or(AukiPeerProtocolsError::Stopped)?;
        self.protocols.register(spec, handler).map_err(Into::into)
    }

    /// Open the selected protocol using the configured routes for one peer.
    pub async fn open(
        &self,
        expected_peer: PeerId,
        protocol_id: impl Into<String>,
    ) -> Result<AuthenticatedRouteStream, AukiPeerProtocolsError> {
        {
            let _running = self
                .lifecycle
                .enter()
                .ok_or(AukiPeerProtocolsError::Stopped)?;
        }
        self.protocols
            .open(expected_peer, protocol_id)
            .await
            .map_err(Into::into)
    }

    /// Open the selected protocol through one exact untrusted route hint.
    pub async fn open_exact(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        protocol_id: impl Into<String>,
    ) -> Result<AuthenticatedRouteStream, AukiPeerProtocolsError> {
        {
            let _running = self
                .lifecycle
                .enter()
                .ok_or(AukiPeerProtocolsError::Stopped)?;
        }
        self.protocols
            .open_exact(expected_peer, route, protocol_id)
            .await
            .map_err(Into::into)
    }
}

/// Rejected custom-protocol access through the facade.
#[derive(Debug, thiserror::Error)]
pub enum AukiPeerProtocolsError {
    /// The facade has entered ordered shutdown.
    #[error("the Auki peer protocol surface is stopped")]
    Stopped,
    /// The authenticated Domain rejected the protocol operation.
    #[error("the authenticated Domain protocol operation failed")]
    Domain(#[from] DomainProtocolError),
}

/// Narrow surface passed to an application protocol adapter.
///
/// It exposes authenticated protocol registration/opening, local published
/// routes, and non-secret local identity metadata. It deliberately omits the
/// raw Domain, transport node, authority installer, and relay reservations.
#[derive(Clone)]
pub struct AukiPeerProtocolContext {
    domain_id: Uuid,
    peer_id: PeerId,
    authorization: AukiPeerAuthorization,
    protocols: AukiPeerProtocols,
    routes: AukiPeerRoutes,
    lifecycle: ContextLifecycle,
}

impl AukiPeerProtocolContext {
    pub(crate) fn new(
        domain_id: Uuid,
        peer_id: PeerId,
        authorization: Arc<dyn AuthorizationSnapshotSource>,
        protocols: DomainProtocols,
        catalog: RouteCatalog,
    ) -> Self {
        let lifecycle = ContextLifecycle::new();
        Self {
            domain_id,
            peer_id,
            authorization: AukiPeerAuthorization::new(authorization, lifecycle.clone()),
            protocols: AukiPeerProtocols::new(protocols, lifecycle.clone()),
            routes: AukiPeerRoutes::new(catalog, lifecycle.clone()),
            lifecycle,
        }
    }

    /// Fence every cloned route, authorization, and protocol view before
    /// runtime cleanup.
    pub(crate) fn fence(&self) {
        self.lifecycle.fence();
    }

    /// Exact authenticated DDS Domain UUID.
    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    /// Stable local libp2p Peer ID.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Readiness-fenced, non-secret local DDS authorization metadata.
    ///
    /// Protocol adapters should obtain a fresh snapshot for every operation
    /// whose policy depends on signed claims such as `peer_type` or scopes.
    pub fn authorization(&self) -> AukiPeerAuthorization {
        self.authorization.clone()
    }

    /// Authenticated application protocol registration and opening surface.
    pub fn protocols(&self) -> AukiPeerProtocols {
        self.protocols.clone()
    }

    /// Read-only local published-route view.
    pub fn routes(&self) -> AukiPeerRoutes {
        self.routes.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use auki_p2p::{Multiaddr, RouteCatalogLimits};

    use super::*;

    fn peer(seed: u64) -> PeerId {
        let mut encoded = [0_u8; 34];
        encoded[1] = 32;
        encoded[2..10].copy_from_slice(&seed.to_be_bytes());
        PeerId::from_bytes(&encoded).expect("test Peer ID must parse")
    }

    #[tokio::test]
    async fn route_view_observes_but_cannot_mutate_the_catalog() {
        let local = peer(31);
        let initial = Multiaddr::from_str("/ip4/127.0.0.1/tcp/4001").unwrap();
        let catalog =
            RouteCatalog::new(local, vec![initial.clone()], RouteCatalogLimits::new(16, 3))
                .unwrap();
        let lifecycle = ContextLifecycle::new();
        let routes = AukiPeerRoutes::new(catalog.clone(), lifecycle.clone());
        let mut changes = routes.subscribe();

        let snapshot = routes.snapshot().unwrap();
        assert_eq!(snapshot.direct_routes, [initial]);
        assert_eq!(routes.status().unwrap().revision, snapshot.revision);
        assert_eq!(
            routes
                .commit_if_current(snapshot.revision, &[], |current| current.revision)
                .unwrap(),
            Some(snapshot.revision)
        );

        catalog
            .replace_direct_routes(vec![
                Multiaddr::from_str("/ip4/127.0.0.1/tcp/4002").unwrap(),
            ])
            .unwrap();
        changes.changed().await.unwrap();
        assert_eq!(changes.borrow().direct_route_count, 1);
        assert_eq!(
            routes
                .commit_if_current(snapshot.revision, &[], |_| ())
                .unwrap(),
            None
        );

        lifecycle.fence();
        assert!(matches!(
            routes.snapshot(),
            Err(AukiPeerRoutesError::Stopped)
        ));
        assert!(matches!(routes.status(), Err(AukiPeerRoutesError::Stopped)));
        assert!(matches!(
            routes.commit_if_current(changes.borrow().revision, &[], |_| ()),
            Err(AukiPeerRoutesError::Stopped)
        ));

        catalog.replace_direct_routes(Vec::new()).unwrap();
        changes.changed().await.unwrap();
        assert_eq!(changes.borrow().direct_route_count, 0);
    }
}
