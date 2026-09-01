//! Shared detached cleanup primitive for same-module Python protocol adapters.

use std::{fmt::Display, future::Future, sync::Arc};

use parking_lot::Mutex;
use tokio::sync::watch;

/// Cloneable result retained after one detached native cleanup finishes.
pub type CleanupResult = Result<(), Arc<str>>;

/// Starts consuming cleanup once and lets any number of Python awaitables
/// observe the retained result without owning or cancelling that cleanup.
#[derive(Default)]
pub struct DetachedCleanup {
    completion: Mutex<Option<watch::Sender<Option<CleanupResult>>>>,
}

impl DetachedCleanup {
    /// Return a new observer for the one cleanup, starting it if necessary.
    pub fn get_or_start<F, E>(
        &self,
        start: impl FnOnce() -> F,
    ) -> watch::Receiver<Option<CleanupResult>>
    where
        F: Future<Output = Result<(), E>> + Send + 'static,
        E: Display,
    {
        let mut completion = self.completion.lock();
        if let Some(sender) = completion.as_ref() {
            return sender.subscribe();
        }

        let (sender, receiver) = watch::channel(None);
        *completion = Some(sender.clone());
        let cleanup = start();
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let result = cleanup
                .await
                .map_err(|error| Arc::<str>::from(error.to_string()));
            sender.send_replace(Some(result));
        });
        receiver
    }
}

/// Wait for a detached cleanup result retained in a watch channel.
pub async fn wait_cleanup(mut receiver: watch::Receiver<Option<CleanupResult>>) -> CleanupResult {
    loop {
        if let Some(result) = receiver.borrow_and_update().clone() {
            return result;
        }
        if receiver.changed().await.is_err() {
            return Err(Arc::from("native cleanup ended without a result"));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::sync::oneshot;

    use super::*;

    #[test]
    fn observers_cannot_cancel_or_restart_native_cleanup() {
        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let barrier = DetachedCleanup::default();
            let starts = Arc::new(AtomicUsize::new(0));
            let (release, released) = oneshot::channel();
            let first = barrier.get_or_start({
                let starts = Arc::clone(&starts);
                move || async move {
                    starts.fetch_add(1, Ordering::SeqCst);
                    released.await.map_err(|error| error.to_string())?;
                    Ok::<(), String>(())
                }
            });
            let second = barrier.get_or_start(|| async {
                panic!("cleanup must not restart");
                #[allow(unreachable_code)]
                Ok::<(), String>(())
            });

            drop(first);
            release.send(()).unwrap();
            assert!(wait_cleanup(second).await.is_ok());
            assert_eq!(starts.load(Ordering::SeqCst), 1);

            let replay = barrier.get_or_start(|| async {
                panic!("completed cleanup must not restart");
                #[allow(unreachable_code)]
                Ok::<(), String>(())
            });
            assert!(wait_cleanup(replay).await.is_ok());
        });
    }
}
