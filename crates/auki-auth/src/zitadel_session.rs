//! One session-owned refresh/save slot, independent of any waiting peer.

use std::{sync::Arc, time::Duration};

use chrono::{Duration as ChronoDuration, Utc};
use futures::{FutureExt, channel::oneshot};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result, ZitadelSessionCredentials, ZitadelSessionStore, ZitadelTokenClient};

const EXPIRY_MARGIN: ChronoDuration = ChronoDuration::seconds(30);
const SESSION_OPERATION: &str = "ZITADEL session operation";

#[derive(Clone, Copy)]
pub(crate) enum RefreshMode {
    IfExpiring,
    Force,
}

pub(crate) struct ReadyCredentials {
    pub credentials: Arc<ZitadelSessionCredentials>,
    pub refreshed: bool,
}

#[derive(Clone, Copy)]
enum Terminal {
    Login,
    Configuration,
}

struct State {
    credentials: Option<Arc<ZitadelSessionCredentials>>,
    pending_save: bool,
    terminal: Option<Terminal>,
}

pub(crate) struct ZitadelSession {
    client: ZitadelTokenClient,
    store: Arc<dyn ZitadelSessionStore>,
    state: Mutex<State>,
    // Keep the receiver itself here, not in a caller's stack. Cancelling a wait
    // leaves both the owned worker and its unconsumed completion intact.
    task: Mutex<Option<oneshot::Receiver<Result<bool>>>>,
    closed: CancellationToken,
    wait_timeout: Duration,
}

impl ZitadelSession {
    pub fn new(
        credentials: ZitadelSessionCredentials,
        client: ZitadelTokenClient,
        store: Arc<dyn ZitadelSessionStore>,
        closed: CancellationToken,
        wait_timeout: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            client,
            store,
            state: Mutex::new(State {
                credentials: Some(Arc::new(credentials)),
                pending_save: false,
                terminal: None,
            }),
            task: Mutex::new(None),
            closed,
            wait_timeout,
        })
    }

    pub async fn ready(
        self: &Arc<Self>,
        mode: RefreshMode,
        cancellation: &CancellationToken,
    ) -> Result<ReadyCredentials> {
        let mut task = tokio::select! {
            biased;
            _ = self.closed.cancelled() => return Err(Error::SessionClosed),
            _ = cancellation.cancelled() => return Err(Error::Cancelled { endpoint: SESSION_OPERATION }),
            task = self.task.lock() => task,
        };
        let mut refreshed = false;
        // At most: finish an existing task, save a pending generation, refresh.
        // A successful joined refresh consumes this operation's refresh budget.
        loop {
            if let Some(receiver) = task.as_mut() {
                let deadline = futures_timer::Delay::new(self.wait_timeout).fuse();
                tokio::pin!(deadline);
                let result = tokio::select! {
                    biased;
                    _ = self.closed.cancelled() => return Err(Error::SessionClosed),
                    _ = cancellation.cancelled() => return Err(Error::Cancelled { endpoint: SESSION_OPERATION }),
                    result = receiver => result,
                    _ = &mut deadline => return Err(Error::SessionOperationPending),
                };
                *task = None;
                match result {
                    Ok(result) => refreshed |= result?,
                    Err(_) => {
                        // A panicking/aborted worker has no trustworthy rotation
                        // outcome. Never submit the old token again.
                        self.require_login().await;
                        return Err(Error::AuthenticationRequired);
                    }
                }
            }
            let state = self.state.lock().await;
            self.check_open(&state)?;
            let credentials = state.credentials.as_ref().ok_or(Error::SessionClosed)?;
            let refresh = !refreshed
                && match mode {
                    RefreshMode::IfExpiring => credentials
                        .access_token_expires_at()
                        .is_some_and(|expiry| expiry <= Utc::now() + EXPIRY_MARGIN),
                    RefreshMode::Force => true,
                };
            if !state.pending_save && !refresh {
                return Ok(ReadyCredentials {
                    credentials: credentials.clone(),
                    refreshed,
                });
            }
            // A rejected save is always retried before any new rotation.
            let refresh = refresh && !state.pending_save;
            drop(state);
            let (sender, receiver) = oneshot::channel();
            let owned = self.clone();
            #[cfg(not(target_arch = "wasm32"))]
            let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
                Error::InvalidConfiguration("ZITADEL sessions require an active Tokio runtime")
            })?;
            *task = Some(receiver);
            let worker = async move {
                let result = owned.run(refresh).await;
                let _ = sender.send(result);
            };
            #[cfg(not(target_arch = "wasm32"))]
            runtime.spawn(worker);
            #[cfg(target_arch = "wasm32")]
            wasm_bindgen_futures::spawn_local(worker);
        }
    }

    async fn run(&self, refresh: bool) -> Result<bool> {
        let mut state = self.state.lock().await;
        self.check_open(&state)?;
        if refresh {
            let credentials = state.credentials.as_ref().ok_or(Error::SessionClosed)?;
            match self.client.refresh(credentials).await {
                Ok(replacement) => {
                    // Commit BEFORE awaiting the host. A consumed refresh token
                    // is never restored, even when saving or startup fails.
                    state.credentials = Some(Arc::new(replacement));
                    state.pending_save = true;
                }
                Err(error) => {
                    use crate::AuthFailureKind;
                    state.terminal = match error.kind() {
                        AuthFailureKind::AuthenticationRequired => Some(Terminal::Login),
                        AuthFailureKind::Configuration => Some(Terminal::Configuration),
                        _ => None,
                    };
                    return Err(error);
                }
            }
        }
        self.check_open(&state)?;
        if state.pending_save {
            let credentials = state.credentials.as_ref().ok_or(Error::SessionClosed)?;
            // Do not cancel a host write: dropping its future does not prove
            // its external side effect stopped. Waiters have their own bound;
            // close drains this acknowledgement before storage may be cleared.
            self.store
                .save(credentials)
                .await
                .map_err(|_| Error::Persistence)?;
            state.pending_save = false;
        }
        self.check_open(&state)?;
        Ok(refresh)
    }

    fn check_open(&self, state: &State) -> Result<()> {
        if self.closed.is_cancelled() {
            return Err(Error::SessionClosed);
        }
        match state.terminal {
            Some(Terminal::Login) => Err(Error::AuthenticationRequired),
            Some(Terminal::Configuration) => Err(Error::InvalidConfiguration(
                "ZITADEL session configuration was rejected; import a corrected session",
            )),
            None => Ok(()),
        }
    }

    pub async fn require_login(&self) {
        self.state.lock().await.terminal = Some(Terminal::Login);
    }

    pub async fn close(&self) {
        self.closed.cancel();
        let mut task = self.task.lock().await;
        if let Some(receiver) = task.as_mut() {
            // Deliberately no timeout: only acknowledgement proves an already
            // running host save cannot repopulate storage after clear.
            let _ = receiver.await;
        }
        *task = None;
        self.state.lock().await.credentials = None;
    }
}
