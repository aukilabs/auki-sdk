use std::{future::Future, panic::AssertUnwindSafe, pin::Pin, sync::Arc};

use futures::FutureExt;
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use super::RuntimeFailureSignal;

type BoxIoTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

const DOMAIN_IO_TASK_CAPACITY: usize = 256;

struct IoTaskRequest {
    task: BoxIoTask,
    cancel: CancellationToken,
    _capacity: OwnedSemaphorePermit,
}

/// Cloneable submission handle for Domain-owned outbound protocol tasks.
///
/// The corresponding host owns every spawned task and drains or aborts them
/// before explicit Domain leave completes. A clone cannot submit new work once
/// the Domain lifecycle is fenced.
#[derive(Clone)]
pub(super) struct DomainIoTasks {
    lifecycle: CancellationToken,
    sender: mpsc::UnboundedSender<IoTaskRequest>,
    capacity: Arc<Semaphore>,
    maximum: usize,
}

pub(super) struct DomainIoTaskHost {
    lifecycle: CancellationToken,
    receiver: mpsc::UnboundedReceiver<IoTaskRequest>,
    fatal: mpsc::UnboundedSender<RuntimeFailureSignal>,
}

/// Per-operation ownership token. Dropping the public operation handle drops
/// this lease and cancels only its associated task.
pub(super) struct DomainIoTaskLease {
    cancel: CancellationToken,
}

impl Drop for DomainIoTaskLease {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl DomainIoTasks {
    pub(super) fn new(
        lifecycle: CancellationToken,
        fatal: mpsc::UnboundedSender<RuntimeFailureSignal>,
    ) -> (Self, DomainIoTaskHost) {
        Self::with_capacity(lifecycle, fatal, DOMAIN_IO_TASK_CAPACITY)
    }

    fn with_capacity(
        lifecycle: CancellationToken,
        fatal: mpsc::UnboundedSender<RuntimeFailureSignal>,
        maximum: usize,
    ) -> (Self, DomainIoTaskHost) {
        debug_assert!(maximum > 0);
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                lifecycle: lifecycle.clone(),
                sender,
                capacity: Arc::new(Semaphore::new(maximum)),
                maximum,
            },
            DomainIoTaskHost {
                lifecycle,
                receiver,
                fatal,
            },
        )
    }

    pub(super) fn spawn<F>(&self, task: F) -> Result<DomainIoTaskLease, DomainIoTaskError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if self.lifecycle.is_cancelled() {
            return Err(DomainIoTaskError::Stopped);
        }
        // The permit is acquired before queueing, so the otherwise unbounded
        // submission channel can never retain more than this fixed number
        // of boxed futures/authenticated streams, including live children.
        let capacity = self.capacity.clone().try_acquire_owned().map_err(|_| {
            if self.lifecycle.is_cancelled() {
                DomainIoTaskError::Stopped
            } else {
                DomainIoTaskError::CapacityExceeded {
                    maximum: self.maximum,
                }
            }
        })?;
        let cancel = CancellationToken::new();
        self.sender
            .send(IoTaskRequest {
                task: Box::pin(task),
                cancel: cancel.clone(),
                _capacity: capacity,
            })
            .map_err(|_| DomainIoTaskError::HostStopped)?;
        if self.lifecycle.is_cancelled() {
            cancel.cancel();
            return Err(DomainIoTaskError::Stopped);
        }
        Ok(DomainIoTaskLease { cancel })
    }
}

impl DomainIoTaskHost {
    pub(super) async fn run(mut self) {
        let outcome = AssertUnwindSafe(self.run_inner()).catch_unwind().await;
        if outcome.is_err() && !self.lifecycle.is_cancelled() {
            let _ = self.fatal.send(RuntimeFailureSignal::ProtocolHostStopped);
        }
    }

    async fn run_inner(&mut self) {
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                _ = self.lifecycle.cancelled() => break,
                completed = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(Err(error)) = completed {
                        tracing::warn!(%error, "Domain-owned protocol I/O task failed");
                    }
                }
                request = self.receiver.recv() => {
                    let Some(request) = request else {
                        if !self.lifecycle.is_cancelled() {
                            let _ = self
                                .fatal
                                .send(RuntimeFailureSignal::ProtocolHostStopped);
                        }
                        break;
                    };
                    tasks.spawn(async move {
                        tokio::select! {
                            biased;
                            _ = request.cancel.cancelled() => {}
                            _ = request.task => {}
                        }
                    });
                }
            }
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum DomainIoTaskError {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("the Domain protocol I/O task host has stopped")]
    HostStopped,
    #[error("the Domain reached its {maximum}-task protocol I/O limit")]
    CapacityExceeded { maximum: usize },
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    #[tokio::test]
    async fn lease_and_domain_cancellation_drop_owned_tasks() {
        let lifecycle = CancellationToken::new();
        let (fatal, mut failures) = mpsc::unbounded_channel();
        let (tasks, host) = DomainIoTasks::new(lifecycle.clone(), fatal);
        let host = tokio::spawn(host.run());

        let first_dropped = Arc::new(AtomicBool::new(false));
        let first = first_dropped.clone();
        let lease = tasks
            .spawn(async move {
                let _guard = DropFlag(first);
                futures::future::pending::<()>().await;
            })
            .unwrap();
        tokio::task::yield_now().await;
        drop(lease);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !first_dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let second_dropped = Arc::new(AtomicBool::new(false));
        let second = second_dropped.clone();
        let _lease = tasks
            .spawn(async move {
                let _guard = DropFlag(second);
                futures::future::pending::<()>().await;
            })
            .unwrap();
        tokio::task::yield_now().await;
        lifecycle.cancel();
        host.await.unwrap();

        assert!(second_dropped.load(Ordering::SeqCst));
        assert!(failures.try_recv().is_err());
        assert!(matches!(
            tasks.spawn(async {}),
            Err(DomainIoTaskError::Stopped)
        ));
    }

    #[tokio::test]
    async fn submission_capacity_bounds_queued_and_live_tasks_and_recovers() {
        let lifecycle = CancellationToken::new();
        let (fatal, mut failures) = mpsc::unbounded_channel();
        let (tasks, host) = DomainIoTasks::with_capacity(lifecycle.clone(), fatal, 1);
        let host = tokio::spawn(host.run());

        let lease = tasks
            .spawn(futures::future::pending::<()>())
            .expect("the first task consumes the sole bounded slot");
        assert!(matches!(
            tasks.spawn(async {}),
            Err(DomainIoTaskError::CapacityExceeded { maximum: 1 })
        ));

        drop(lease);
        let recovered = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                match tasks.spawn(futures::future::pending::<()>()) {
                    Ok(lease) => break lease,
                    Err(DomainIoTaskError::CapacityExceeded { .. }) => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("unexpected task submission error: {error}"),
                }
            }
        })
        .await
        .expect("canceling a child must release its bounded slot");

        drop(recovered);
        lifecycle.cancel();
        host.await.unwrap();
        assert!(failures.try_recv().is_err());
    }

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}
