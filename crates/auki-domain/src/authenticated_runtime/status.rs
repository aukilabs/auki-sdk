use std::sync::Arc;

use auki_p2p::{DDS_VERIFICATION_KEYS_MAX_STALENESS, NodeFailure};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Local state of the private authenticated Domain engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainStatus {
    Ready,
    CredentialUnavailable,
    Failed(DomainFailure),
    Stopped,
}

/// Bounded terminal failures retained by the Domain status snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainFailure {
    Node(NodeFailure),
    NodeStoppedUnexpectedly,
    ProtocolHostStopped,
    SupervisorStopped,
    CleanupFailed,
    CleanupTimeout,
}

#[derive(Clone)]
pub(super) struct DomainStatusController {
    inner: Arc<StatusInner>,
}

struct StatusInner {
    transition_order: Mutex<()>,
    state: Mutex<StatusState>,
    sender: watch::Sender<DomainStatus>,
    authority_revision: watch::Sender<u64>,
}

struct StatusState {
    status: DomainStatus,
    credential_deadline: Option<DateTime<Utc>>,
    verification_keys_deadline: Option<tokio::time::Instant>,
}

#[derive(Clone, Copy)]
enum AuthorityDeadline {
    Credential(DateTime<Utc>),
    VerificationKeys(tokio::time::Instant),
}

impl DomainStatusController {
    pub(super) fn credential_unavailable() -> Self {
        let status = DomainStatus::CredentialUnavailable;
        let (sender, _) = watch::channel(status);
        let (authority_revision, _) = watch::channel(0);
        Self {
            inner: Arc::new(StatusInner {
                transition_order: Mutex::new(()),
                state: Mutex::new(StatusState {
                    status,
                    credential_deadline: None,
                    verification_keys_deadline: None,
                }),
                sender,
                authority_revision,
            }),
        }
    }

    pub(super) fn status(&self) -> DomainStatus {
        self.expire_if_due(Utc::now(), tokio::time::Instant::now());
        self.inner.state.lock().status
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<DomainStatus> {
        self.expire_if_due(Utc::now(), tokio::time::Instant::now());
        self.inner.sender.subscribe()
    }

    pub(super) fn is_terminal(&self) -> bool {
        matches!(
            self.status(),
            DomainStatus::Failed(_) | DomainStatus::Stopped
        )
    }

    pub(super) fn set_credential_deadline(&self, deadline: Option<DateTime<Utc>>) {
        let now = Utc::now();
        let deadline = deadline.filter(|deadline| *deadline > now);
        self.transition(|state| {
            if matches!(
                state.status,
                DomainStatus::Failed(_) | DomainStatus::Stopped
            ) {
                return None;
            }
            let changed = state.credential_deadline != deadline;
            state.credential_deadline = deadline;
            let next = authority_status(state, now, tokio::time::Instant::now());
            let changed = changed || state.status != next;
            state.status = next;
            changed.then_some(next)
        });
    }

    pub(super) fn refresh_verification_keys(&self, refresh_started_at: tokio::time::Instant) {
        // The verifier records its own refresh no earlier than the host begins
        // installation. Using this conservative instant prevents Domain status
        // from remaining Ready for even a small interval after verification has
        // already become stale.
        let deadline = refresh_started_at + DDS_VERIFICATION_KEYS_MAX_STALENESS;
        self.transition(|state| {
            if matches!(
                state.status,
                DomainStatus::Failed(_) | DomainStatus::Stopped
            ) {
                return None;
            }
            state.verification_keys_deadline = Some(deadline);
            let next = authority_status(state, Utc::now(), tokio::time::Instant::now());
            state.status = next;
            Some(next)
        });
    }

    pub(super) fn fail(&self, failure: DomainFailure) {
        let next = DomainStatus::Failed(failure);
        self.transition(|state| {
            if matches!(
                state.status,
                DomainStatus::Failed(_) | DomainStatus::Stopped
            ) {
                None
            } else {
                state.status = next;
                state.credential_deadline = None;
                state.verification_keys_deadline = None;
                Some(next)
            }
        });
    }

    pub(super) fn stop(&self) {
        self.transition(|state| {
            if matches!(
                state.status,
                DomainStatus::Failed(_) | DomainStatus::Stopped
            ) {
                None
            } else {
                state.status = DomainStatus::Stopped;
                state.credential_deadline = None;
                state.verification_keys_deadline = None;
                Some(DomainStatus::Stopped)
            }
        });
    }

    pub(super) async fn drive_authority_expiry(&self, shutdown: CancellationToken) {
        let mut revision = self.inner.authority_revision.subscribe();
        loop {
            if shutdown.is_cancelled() || self.is_terminal() {
                return;
            }
            let deadline = next_authority_deadline(&self.inner.state.lock());
            match deadline {
                Some(deadline) => {
                    let delay = deadline.delay();
                    tokio::select! {
                        biased;
                        _ = shutdown.cancelled() => return,
                        changed = revision.changed() => {
                            if changed.is_err() {
                                return;
                            }
                        }
                        _ = tokio::time::sleep(delay) => {
                            // Fence the exact deadline that armed this timer. A
                            // refresh racing the old timer must not regress the
                            // new authority to unavailable.
                            self.expire_deadline_if_current(deadline);
                        }
                    }
                }
                None => {
                    tokio::select! {
                        biased;
                        _ = shutdown.cancelled() => return,
                        changed = revision.changed() => {
                            if changed.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    fn expire_if_due(&self, now: DateTime<Utc>, monotonic_now: tokio::time::Instant) {
        self.transition(|state| {
            if matches!(
                state.status,
                DomainStatus::Failed(_) | DomainStatus::Stopped
            ) {
                return None;
            }
            if state
                .credential_deadline
                .is_some_and(|deadline| deadline <= now)
            {
                state.credential_deadline = None;
            }
            if state
                .verification_keys_deadline
                .is_some_and(|deadline| deadline <= monotonic_now)
            {
                state.verification_keys_deadline = None;
            }
            let next = authority_status(state, now, monotonic_now);
            let changed = state.status != next;
            state.status = next;
            changed.then_some(next)
        });
    }

    fn expire_deadline_if_current(&self, deadline: AuthorityDeadline) {
        self.transition(|state| {
            if matches!(
                state.status,
                DomainStatus::Failed(_) | DomainStatus::Stopped
            ) {
                return None;
            }
            match deadline {
                AuthorityDeadline::Credential(deadline)
                    if state.credential_deadline == Some(deadline) =>
                {
                    state.credential_deadline = None;
                }
                AuthorityDeadline::VerificationKeys(deadline)
                    if state.verification_keys_deadline == Some(deadline) =>
                {
                    state.verification_keys_deadline = None;
                }
                _ => return None,
            }
            let next = authority_status(state, Utc::now(), tokio::time::Instant::now());
            let changed = state.status != next;
            state.status = next;
            changed.then_some(next)
        });
    }

    fn transition(&self, update: impl FnOnce(&mut StatusState) -> Option<DomainStatus>) {
        self.transition_with_before_publish(update, || {});
    }

    fn transition_with_before_publish(
        &self,
        update: impl FnOnce(&mut StatusState) -> Option<DomainStatus>,
        before_publish: impl FnOnce(),
    ) {
        // State and watch publication form one ordered transition. Without
        // this independent ordering lock, a concurrent terminal transition
        // can publish before an earlier Ready update and then be overwritten
        // in the watch channel even though the state snapshot is terminal.
        let _transition = self.inner.transition_order.lock();
        let next = {
            let mut state = self.inner.state.lock();
            update(&mut state)
        };
        if let Some(next) = next {
            before_publish();
            self.inner.sender.send_replace(next);
            self.bump_authority_revision();
        }
    }

    fn bump_authority_revision(&self) {
        self.inner
            .authority_revision
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
}

impl AuthorityDeadline {
    fn delay(self) -> std::time::Duration {
        match self {
            Self::Credential(deadline) => deadline
                .signed_duration_since(Utc::now())
                .to_std()
                .unwrap_or_default(),
            Self::VerificationKeys(deadline) => {
                deadline.saturating_duration_since(tokio::time::Instant::now())
            }
        }
    }
}

fn next_authority_deadline(state: &StatusState) -> Option<AuthorityDeadline> {
    let credential = state.credential_deadline.map(AuthorityDeadline::Credential);
    let keys = state
        .verification_keys_deadline
        .map(AuthorityDeadline::VerificationKeys);
    match (credential, keys) {
        (Some(credential), Some(keys)) => {
            if credential.delay() <= keys.delay() {
                Some(credential)
            } else {
                Some(keys)
            }
        }
        (deadline, None) | (None, deadline) => deadline,
    }
}

fn authority_status(
    state: &StatusState,
    now: DateTime<Utc>,
    monotonic_now: tokio::time::Instant,
) -> DomainStatus {
    if state
        .credential_deadline
        .is_some_and(|deadline| deadline > now)
        && state
            .verification_keys_deadline
            .is_some_and(|deadline| deadline > monotonic_now)
    {
        DomainStatus::Ready
    } else {
        DomainStatus::CredentialUnavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn refreshed_deadline_fences_an_old_expiry_timer() {
        let status = DomainStatusController::credential_unavailable();
        let shutdown = CancellationToken::new();
        let driver_shutdown = shutdown.clone();
        let driver_status = status.clone();
        let driver = tokio::spawn(async move {
            driver_status.drive_authority_expiry(driver_shutdown).await;
        });
        status.refresh_verification_keys(tokio::time::Instant::now());
        status.set_credential_deadline(Some(Utc::now() + chrono::Duration::seconds(5)));
        assert_eq!(status.status(), DomainStatus::Ready);

        tokio::time::advance(std::time::Duration::from_secs(4)).await;
        status.set_credential_deadline(Some(Utc::now() + chrono::Duration::seconds(30)));
        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        assert_eq!(status.status(), DomainStatus::Ready);

        tokio::time::advance(std::time::Duration::from_secs(30)).await;
        tokio::task::yield_now().await;
        assert_eq!(status.status(), DomainStatus::CredentialUnavailable);
        shutdown.cancel();
        driver.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn stale_verification_keys_make_current_credentials_unavailable() {
        let status = DomainStatusController::credential_unavailable();
        let shutdown = CancellationToken::new();
        let driver_status = status.clone();
        let driver_shutdown = shutdown.clone();
        let driver = tokio::spawn(async move {
            driver_status.drive_authority_expiry(driver_shutdown).await;
        });
        status.set_credential_deadline(Some(Utc::now() + chrono::Duration::hours(2)));
        status.refresh_verification_keys(tokio::time::Instant::now());
        assert_eq!(status.status(), DomainStatus::Ready);

        tokio::time::advance(DDS_VERIFICATION_KEYS_MAX_STALENESS).await;
        tokio::task::yield_now().await;
        assert_eq!(status.status(), DomainStatus::CredentialUnavailable);

        status.refresh_verification_keys(tokio::time::Instant::now());
        assert_eq!(status.status(), DomainStatus::Ready);
        shutdown.cancel();
        driver.await.unwrap();
    }

    #[test]
    fn failure_is_terminal_and_dominates_stop_or_authority_updates() {
        let status = DomainStatusController::credential_unavailable();
        status.fail(DomainFailure::ProtocolHostStopped);
        status.refresh_verification_keys(tokio::time::Instant::now());
        status.set_credential_deadline(Some(Utc::now() + chrono::Duration::hours(1)));
        status.stop();
        assert_eq!(
            status.status(),
            DomainStatus::Failed(DomainFailure::ProtocolHostStopped)
        );
    }

    #[test]
    fn terminal_transition_cannot_be_published_before_an_earlier_ready_transition() {
        let status = DomainStatusController::credential_unavailable();
        let ready_at_publish = Arc::new(std::sync::Barrier::new(2));
        let release_ready = Arc::new(std::sync::Barrier::new(2));

        let ready_status = status.clone();
        let ready_at_publish_task = Arc::clone(&ready_at_publish);
        let release_ready_task = Arc::clone(&release_ready);
        let ready = std::thread::spawn(move || {
            ready_status.transition_with_before_publish(
                |state| {
                    state.status = DomainStatus::Ready;
                    Some(DomainStatus::Ready)
                },
                || {
                    ready_at_publish_task.wait();
                    release_ready_task.wait();
                },
            );
        });
        ready_at_publish.wait();

        let failed_status = status.clone();
        let (failed_started_tx, failed_started_rx) = std::sync::mpsc::channel();
        let (failed_finished_tx, failed_finished_rx) = std::sync::mpsc::channel();
        let failed = std::thread::spawn(move || {
            failed_started_tx.send(()).unwrap();
            failed_status.fail(DomainFailure::ProtocolHostStopped);
            failed_finished_tx.send(()).unwrap();
        });
        failed_started_rx.recv().unwrap();
        assert!(
            failed_finished_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "the terminal transition must wait until Ready is published"
        );

        release_ready.wait();
        ready.join().unwrap();
        failed.join().unwrap();
        assert_eq!(
            status.status(),
            DomainStatus::Failed(DomainFailure::ProtocolHostStopped)
        );
        assert_eq!(
            *status.subscribe().borrow(),
            DomainStatus::Failed(DomainFailure::ProtocolHostStopped),
            "the watch snapshot must agree with the terminal state snapshot"
        );
    }
}
