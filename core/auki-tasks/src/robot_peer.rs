//! A robot's assigned-Domain transport outlives individual DMS leases.
use std::{sync::Arc, time::Duration};

use chrono::Utc;
use parking_lot::Mutex;
use tokio::{sync::watch, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{AukiRobotCredential, Result, TaskError, TaskPeerFactory, TaskPeerSession};

#[cfg(test)]
#[path = "robot_peer_tests.rs"]
mod tests;

struct State {
    closed: CancellationToken,
    session: Mutex<Option<Arc<dyn TaskPeerSession>>>,
    ready: watch::Sender<Option<Result<()>>>,
    failure: Mutex<Option<TaskError>>,
    cleanup_failed: Mutex<bool>,
}

pub(crate) struct RobotPeer {
    robot: AukiRobotCredential,
    factory: Arc<dyn TaskPeerFactory>,
    state: Arc<State>,
    driver: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}

impl Drop for RobotPeer {
    fn drop(&mut self) {
        // The owned driver finishes cleanup; explicit close also awaits it.
        self.state.closed.cancel();
    }
}

impl RobotPeer {
    pub fn new(
        robot: AukiRobotCredential,
        factory: Arc<dyn TaskPeerFactory>,
        closed: CancellationToken,
    ) -> Self {
        Self {
            robot,
            factory,
            state: Arc::new(State {
                closed,
                session: Mutex::new(None),
                ready: watch::channel(None).0,
                failure: Mutex::new(None),
                cleanup_failed: Mutex::new(false),
            }),
            driver: tokio::sync::Mutex::new(None),
        }
    }

    pub fn session(&self) -> Option<Arc<dyn TaskPeerSession>> {
        self.state.session.lock().clone()
    }

    pub fn failure(&self) -> Option<TaskError> {
        *self.state.failure.lock()
    }

    /// Start once. Cancelling a waiter does not abandon owned startup/renewal;
    /// the runtime remains its owner and close drains it, including partial startup.
    pub async fn start(&self, cancellation: &CancellationToken) -> Result<()> {
        let mut ready = self.state.ready.subscribe();
        {
            let mut driver = self.driver.lock().await;
            if self.state.closed.is_cancelled() {
                return Err(self.failure().unwrap_or(TaskError::Closed));
            }
            if driver.is_none() {
                let state = self.state.clone();
                let robot = self.robot.clone();
                let factory = self.factory.clone();
                *driver = Some(tokio::spawn(async move {
                    drive(state, robot, factory).await;
                }));
            }
        }
        loop {
            if let Some(result) = *ready.borrow_and_update() {
                return result;
            }
            tokio::select! { biased;
                _ = cancellation.cancelled() => return Err(TaskError::Cancelled),
                _ = self.state.closed.cancelled() => return Err(self.failure().unwrap_or(TaskError::Closed)),
                result = ready.changed() => if result.is_err() { return Err(TaskError::Closed); },
            }
        }
    }

    pub async fn close(&self) -> Result<()> {
        self.state.closed.cancel();
        let mut driver = self.driver.lock().await;
        if let Some(task) = driver.as_mut()
            && task.await.is_err()
        {
            *self.state.cleanup_failed.lock() = true;
        }
        driver.take();
        if *self.state.cleanup_failed.lock() {
            return Err(TaskError::PeerCleanup);
        }
        Ok(())
    }
}

async fn drive(state: Arc<State>, robot: AukiRobotCredential, factory: Arc<dyn TaskPeerFactory>) {
    let result = async {
        let Some(domain) = robot.assigned_domain_id(&state.closed).await? else {
            state.ready.send_replace(Some(Ok(())));
            return Ok(());
        };
        let mut grant = tokio::select! { biased;
            _ = state.closed.cancelled() => return Err(TaskError::Cancelled),
            result = robot.peer_grant(domain) => result?,
        };
        // The SDK adapter owns bounded startup and awaits rollback on cancellation.
        let peer = factory.start(grant.clone(), &state.closed).await?;
        *state.session.lock() = Some(peer.clone());
        if state.closed.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        state.ready.send_replace(Some(Ok(())));
        loop {
            let remaining = grant
                .expires_at
                .signed_duration_since(Utc::now())
                .to_std()
                .map_err(|_| TaskError::Authority("robot peer authority expired"))?;
            if remaining.is_zero() {
                return Err(TaskError::Authority("robot peer authority expired"));
            }
            let delay = remaining
                .div_f64(2.0)
                .max(Duration::from_millis(10))
                .min(remaining)
                .min(robot.config().registration_interval)
                .min(Duration::from_secs(60));
            tokio::select! { biased;
                _ = state.closed.cancelled() => return Ok(()),
                _ = peer.wait_stopped() => return Err(TaskError::Authority("robot peer stopped")),
                _ = peer.refresh_requested() => {},
                _ = tokio::time::sleep(delay) => {},
            }
            // One robot authority driver renews while idle or busy. DMS heartbeats
            // renew task data only; they never drive this exchange.
            let timeout = grant
                .expires_at
                .signed_duration_since(Utc::now())
                .to_std()
                .map_err(|_| TaskError::Authority("robot peer authority expired"))?
                .min(robot.config().request_timeout);
            let refresh = async {
                let updated = robot.peer_grant(domain).await?;
                peer.update(updated.clone()).await?;
                Ok(updated)
            };
            grant = tokio::select! { biased;
                _ = state.closed.cancelled() => return Ok(()),
                result = tokio::time::timeout(timeout, refresh) => result
                    .map_err(|_| TaskError::Authority("robot peer renewal timed out"))??,
            };
        }
    }
    .await;
    if let Err(error) = result {
        if matches!(error, TaskError::PeerCleanup) {
            *state.cleanup_failed.lock() = true;
        }
        if !state.closed.is_cancelled() || matches!(error, TaskError::PeerCleanup) {
            *state.failure.lock() = Some(error);
            state.closed.cancel();
        }
    }
    // Publish startup failure even when cancellation arrived before startup ended.
    if state.ready.borrow().is_none() {
        state.ready.send_replace(Some(result));
    }
    let peer = state.session.lock().take();
    if let Some(peer) = peer {
        peer.fence();
        if peer.shutdown().await.is_err() {
            *state.cleanup_failed.lock() = true;
        }
    }
}
