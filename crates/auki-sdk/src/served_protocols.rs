use tokio::sync::watch;

/// Immutable process-local view of the inbound protocols mounted on one Peer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ServedProtocolSnapshot {
    pub(crate) revision: u64,
    pub(crate) protocol_ids: Vec<String>,
}

/// Retained latest-value publication used by the future discovery supervisor.
#[derive(Clone)]
pub(crate) struct ServedProtocolSnapshots {
    updates: watch::Sender<ServedProtocolSnapshot>,
}

impl ServedProtocolSnapshots {
    pub(crate) fn new() -> Self {
        let (updates, _) = watch::channel(ServedProtocolSnapshot::default());
        Self { updates }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<ServedProtocolSnapshot> {
        self.updates.subscribe()
    }

    /// Replace the served set after its owning registration mutation succeeds.
    ///
    /// Callers hold their registration lock/borrow while invoking this method,
    /// so the snapshot revision and mounted handler set change as one operation.
    pub(crate) fn replace(&self, protocol_ids: impl IntoIterator<Item = String>) {
        let mut protocol_ids = protocol_ids.into_iter().collect::<Vec<_>>();
        protocol_ids.sort_unstable();
        protocol_ids.dedup();
        self.updates.send_if_modified(move |current| {
            if current.protocol_ids == protocol_ids {
                return false;
            }
            // Keep the observational cursor nondecreasing even after its
            // theoretical process-lifetime limit. The complete retained value
            // remains authoritative for every notification.
            current.revision = current.revision.saturating_add(1);
            current.protocol_ids = protocol_ids;
            true
        });
    }
}
