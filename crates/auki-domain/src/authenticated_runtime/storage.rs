use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use auki_p2p::PeerId;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

/// Shared late-bound filesystem source for retained registries and blobs.
///
/// The old runtime configured both protocol families from one application
/// root. Keeping one Domain-owned source preserves that behavior without
/// introducing a provider or Manager dependency.
#[derive(Clone)]
pub(super) struct RegistryBlobStorage {
    local_peer_id: PeerId,
    lifecycle: CancellationToken,
    app_root: Arc<Mutex<Option<PathBuf>>>,
    source_reads: Arc<AtomicUsize>,
}

impl RegistryBlobStorage {
    pub(super) fn new(local_peer_id: PeerId, lifecycle: CancellationToken) -> Self {
        Self {
            local_peer_id,
            lifecycle,
            app_root: Arc::new(Mutex::new(None)),
            source_reads: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    pub(super) fn set_app_root(&self, app_root: impl Into<PathBuf>) -> Result<(), StorageError> {
        self.ensure_running()?;
        let mut current = self.app_root.lock();
        self.ensure_running()?;
        *current = Some(app_root.into());
        Ok(())
    }

    pub(super) fn app_root(&self) -> Result<Option<PathBuf>, StorageError> {
        self.ensure_running()?;
        self.source_reads.fetch_add(1, Ordering::Relaxed);
        let current = self.app_root.lock();
        self.ensure_running()?;
        Ok(current.clone())
    }

    pub(super) fn ensure_running(&self) -> Result<(), StorageError> {
        if self.lifecycle.is_cancelled() {
            Err(StorageError::Stopped)
        } else {
            Ok(())
        }
    }

    pub(super) fn lifecycle(&self) -> &CancellationToken {
        &self.lifecycle
    }

    #[cfg(test)]
    pub(super) fn source_reads(&self) -> usize {
        self.source_reads.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    #[error("the Domain runtime is stopped")]
    Stopped,
}
