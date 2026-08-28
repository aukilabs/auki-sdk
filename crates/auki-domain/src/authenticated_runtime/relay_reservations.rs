use std::sync::Arc;

use auki_p2p::{
    Node, RelayProvider, RelayReservationHandle, RelayReservationSnapshot, RelayTransportEvent,
};
use tokio::sync::broadcast;

use super::RuntimeAccess;

/// Restricted access to the owning Domain's Circuit Relay v2 reservations.
///
/// This handle deliberately exposes no raw [`Node`]. A higher-level runtime
/// may reconcile a trusted relay assignment with the Domain-owned transport,
/// while DMS HTTP and allocation policy remain outside `auki-domain`.
#[derive(Clone)]
pub struct DomainRelayReservations {
    access: Arc<RuntimeAccess>,
}

impl DomainRelayReservations {
    pub(super) fn new(access: Arc<RuntimeAccess>) -> Self {
        Self { access }
    }

    /// Subscribe to reservation acceptance, loss, and cancellation events.
    pub fn subscribe(&self) -> Result<broadcast::Receiver<RelayTransportEvent>, DomainRelayError> {
        Ok(self.node()?.subscribe_relay_events())
    }

    /// Begin one reservation generation for a trusted relay assignment.
    pub async fn start(
        &self,
        provider: RelayProvider,
    ) -> Result<RelayReservationHandle, DomainRelayError> {
        let node = self.node()?;
        tokio::select! {
            biased;
            _ = self.access.lifecycle.cancelled() => Err(DomainRelayError::Stopped),
            result = node.start_relay_reservation(provider) => result.map_err(DomainRelayError::P2p),
        }
    }

    /// Wait until one exact generation is confirmed and publishable.
    pub async fn wait(
        &self,
        handle: RelayReservationHandle,
    ) -> Result<RelayReservationSnapshot, DomainRelayError> {
        let node = self.node()?;
        tokio::select! {
            biased;
            _ = self.access.lifecycle.cancelled() => Err(DomainRelayError::Stopped),
            result = node.wait_relay_reservation(handle) => result.map_err(DomainRelayError::P2p),
        }
    }

    /// Read the current state of one exact reservation generation.
    pub async fn snapshot(
        &self,
        handle: RelayReservationHandle,
    ) -> Result<RelayReservationSnapshot, DomainRelayError> {
        let node = self.node()?;
        tokio::select! {
            biased;
            _ = self.access.lifecycle.cancelled() => Err(DomainRelayError::Stopped),
            result = node.relay_reservation(handle) => result.map_err(DomainRelayError::P2p),
        }
    }

    /// Cancel and unpublish one exact reservation generation.
    pub async fn cancel(&self, handle: RelayReservationHandle) -> Result<(), DomainRelayError> {
        let node = self.node()?;
        tokio::select! {
            biased;
            _ = self.access.lifecycle.cancelled() => Err(DomainRelayError::Stopped),
            result = node.cancel_relay_reservation(handle) => result.map_err(DomainRelayError::P2p),
        }
    }

    fn node(&self) -> Result<Node, DomainRelayError> {
        self.access.node().map_err(|_| DomainRelayError::Stopped)
    }
}

/// Failure from the restricted Domain relay-reservation capability.
#[derive(Debug, thiserror::Error)]
pub enum DomainRelayError {
    /// The owning Domain has begun or completed shutdown.
    #[error("the Domain relay-reservation capability is stopped")]
    Stopped,
    /// The authenticated transport rejected or lost the reservation.
    #[error(transparent)]
    P2p(#[from] auki_p2p::Error),
}
