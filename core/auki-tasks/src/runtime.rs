use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use auki_dms::{
    client::{ClaimOutcome, DmsClient},
    types::{CompleteTaskRequest, FailTaskRequest, HeartbeatRequest, TaskSpec},
};
use auki_domain_client::{AukiDomainData, DomainDataClient};
use chrono::Utc;
use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::{
    MachineCredential, Result, TaskCredential, TaskError, TaskPeerFactory, TaskPeerGrant,
    TaskPeerSession,
};

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

pub type TaskResult = CompleteTaskRequest;

pub(crate) fn validate_capabilities(capabilities: &[String]) -> Result<()> {
    if capabilities.is_empty()
        || capabilities.len() > 100
        || capabilities
            .iter()
            .any(|c| c.is_empty() || c.len() > 256 || c.chars().any(char::is_control))
    {
        return Err(TaskError::Configuration("provide 1-100 valid capabilities"));
    }
    let mut sorted = capabilities.to_vec();
    sorted.sort();
    if sorted.windows(2).any(|p| p[0] == p[1]) {
        return Err(TaskError::Configuration("duplicate capability"));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct TasksConfig {
    pub poll_interval: Duration,
    pub heartbeat_interval: Duration,
    pub request_timeout: Duration,
}
impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            heartbeat_interval: Duration::from_secs(30),
            request_timeout: Duration::from_secs(30),
        }
    }
}

/// Application-owned work. Observe cancellation and finish cleanup before returning.
#[async_trait]
pub trait TaskHandler: Send + Sync {
    async fn run(&self, task: TaskContext) -> Result<TaskResult>;
}

/// Handler-visible state contains task metadata and scoped access, never raw leases.
#[derive(Clone)]
pub struct TaskContext {
    pub task: TaskSpec,
    pub credential: TaskCredential,
    data: DomainDataClient,
    progress: Arc<Mutex<Value>>,
    peer: Arc<Mutex<Option<Arc<dyn TaskPeerSession>>>>,
    owns_peer: bool,
}
impl TaskContext {
    pub fn data(&self) -> DomainDataClient {
        self.data.clone()
    }
    /// Transport adapter for the SDK's typed task peer view.
    #[doc(hidden)]
    pub fn peer_session(&self) -> Option<Arc<dyn TaskPeerSession>> {
        self.peer.lock().clone()
    }

    fn revoke(&self) {
        self.credential.revoke();
        if self.owns_peer
            && let Some(peer) = self.peer_session()
        {
            peer.fence();
        }
    }
    pub fn cancellation(&self) -> CancellationToken {
        self.credential.closed.clone()
    }
    pub fn is_cancelled(&self) -> bool {
        self.credential.closed.is_cancelled()
    }
    pub async fn cancelled(&self) {
        self.credential.closed.cancelled().await;
    }
    /// Replace progress. The sole heartbeat owner sends the latest value.
    pub fn progress(&self, value: Value) -> Result<()> {
        if self.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        if serde_json::to_vec(&value)
            .map_err(|_| TaskError::Configuration("invalid progress"))?
            .len()
            > 64 * 1024
        {
            return Err(TaskError::Configuration("progress exceeds 64 KiB"));
        }
        *self.progress.lock() = value;
        self.credential.changed.notify_one();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskOutcome {
    NoWork,
    Busy,
    Completed,
}

struct Owner {
    client: DmsClient,
    client_id: String,
    machine: Option<MachineCredential>,
    peer: Option<Arc<dyn TaskPeerFactory>>,
    robot_peer: Option<crate::robot_peer::RobotPeer>,
    capabilities: Vec<String>,
    config: TasksConfig,
    closed: CancellationToken,
    active: Arc<Semaphore>,
    running: AtomicBool,
    cleanup_failed: AtomicBool,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.closed.cancel();
    }
}

/// One native lease owner, usable by Rust or Python handlers.
#[derive(Clone)]
pub struct AukiDmsTasks(Arc<Owner>);

impl AukiDmsTasks {
    pub fn new(
        machine: impl Into<MachineCredential>,
        capabilities: Vec<String>,
        config: TasksConfig,
    ) -> Result<Self> {
        Self::new_with_optional_peer(machine.into(), capabilities, config, None)
    }

    pub fn new_with_peer(
        machine: impl Into<MachineCredential>,
        capabilities: Vec<String>,
        config: TasksConfig,
        peer: Arc<dyn TaskPeerFactory>,
    ) -> Result<Self> {
        Self::new_with_optional_peer(machine.into(), capabilities, config, Some(peer))
    }

    fn new_with_optional_peer(
        machine: MachineCredential,
        capabilities: Vec<String>,
        config: TasksConfig,
        peer: Option<Arc<dyn TaskPeerFactory>>,
    ) -> Result<Self> {
        if machine.peer_id() != peer.as_ref().map(|p| p.peer_id())
            || peer.as_ref().is_some_and(|p| {
                p.dds_url() != machine.dds_url()
                    || p.dms_url().trim_end_matches('/')
                        != machine.dms_url().as_str().trim_end_matches('/')
            })
        {
            return Err(TaskError::Configuration(
                "task peer must match the machine identity and DDS/DMS endpoints",
            ));
        }
        let client = DmsClient::new(
            machine.dms_url().clone(),
            config.request_timeout,
            Arc::new(machine.clone()),
        )
        .map_err(|_| TaskError::Configuration("cannot construct DMS client"))?;
        Self::build(
            client,
            machine.client_id().into(),
            capabilities,
            config,
            Some(machine),
            peer,
        )
    }

    /// Adapter for an existing host which already owns machine authentication.
    /// The caller must stop this runtime when that external authority closes.
    pub fn from_client(
        client: DmsClient,
        client_id: String,
        capabilities: Vec<String>,
        config: TasksConfig,
    ) -> Result<Self> {
        Self::build(client, client_id, capabilities, config, None, None)
    }

    fn build(
        client: DmsClient,
        client_id: String,
        mut capabilities: Vec<String>,
        config: TasksConfig,
        machine: Option<MachineCredential>,
        peer: Option<Arc<dyn TaskPeerFactory>>,
    ) -> Result<Self> {
        validate_capabilities(&capabilities)?;
        capabilities.sort();
        if capabilities.windows(2).any(|p| p[0] == p[1]) {
            return Err(TaskError::Configuration("duplicate capability"));
        }
        if client_id.is_empty()
            || client_id.len() > 128
            || !client_id.bytes().all(|b| b.is_ascii_graphic())
            || [
                config.poll_interval,
                config.heartbeat_interval,
                config.request_timeout,
            ]
            .iter()
            .any(|d| *d < Duration::from_millis(10) || *d > Duration::from_secs(300))
        {
            return Err(TaskError::Configuration("invalid client ID or task timing"));
        }
        if let Some(machine) = &machine {
            machine.attach_runtime(&capabilities)?;
        }
        let closed = machine
            .as_ref()
            .map(|m| m.cancellation().child_token())
            .unwrap_or_default();
        let robot_peer = match (&machine, &peer) {
            (Some(MachineCredential::Robot(robot)), Some(factory)) => Some(
                crate::robot_peer::RobotPeer::new(robot.clone(), factory.clone(), closed.clone()),
            ),
            _ => None,
        };
        Ok(Self(Arc::new(Owner {
            client,
            client_id,
            machine,
            peer,
            robot_peer,
            capabilities,
            config,
            closed,
            active: Arc::new(Semaphore::new(1)),
            running: AtomicBool::new(false),
            cleanup_failed: AtomicBool::new(false),
        })))
    }

    pub fn capabilities(&self) -> &[String] {
        &self.0.capabilities
    }

    /// Register/authenticate without claiming work. Robots also start their
    /// assigned-Domain peer, which remains owned until runtime shutdown.
    pub async fn start(&self, cancellation: &CancellationToken) -> Result<()> {
        let operation = async {
            if let Some(machine) = &self.0.machine {
                machine.start(&self.0.capabilities, cancellation).await?;
            }
            if let Some(peer) = &self.0.robot_peer {
                peer.start(cancellation).await?;
            }
            Ok(())
        };
        tokio::select! { biased;
            _ = self.stopped() => Err(self.stopped_error()),
            _ = cancellation.cancelled() => Err(TaskError::Cancelled),
            result = operation => result,
        }
    }

    /// SDK adapter for the persistent robot peer; compute peers belong to tasks.
    #[doc(hidden)]
    pub fn peer_session(&self) -> Option<Arc<dyn TaskPeerSession>> {
        self.0.robot_peer.as_ref().and_then(|peer| peer.session())
    }

    pub fn request_shutdown(&self) {
        self.0.closed.cancel();
    }

    fn stopped_error(&self) -> TaskError {
        if let Some(error) = self.0.robot_peer.as_ref().and_then(|peer| peer.failure()) {
            return error;
        }
        if self
            .0
            .machine
            .as_ref()
            .is_some_and(MachineCredential::failed)
        {
            TaskError::Authentication
        } else {
            TaskError::Closed
        }
    }

    async fn stopped(&self) {
        match &self.0.machine {
            Some(machine) => {
                tokio::select! { _ = self.0.closed.cancelled() => {}, _ = machine.wait_closed() => {} }
            }
            None => self.0.closed.cancelled().await,
        }
    }

    /// Claims exactly one capability. The returned lease keeps this runtime busy.
    /// No heartbeat is started; custom loops call heartbeat/complete/fail themselves.
    pub async fn claim(
        &self,
        capability: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<TaskLease>> {
        if !self.0.capabilities.iter().any(|c| c == capability) {
            return Err(TaskError::Configuration("unregistered capability"));
        }
        let permit = self
            .0
            .active
            .clone()
            .try_acquire_owned()
            .map_err(|_| TaskError::Busy)?;
        let operation = async {
            self.start(cancellation).await?;
            let assignment = if let Some(MachineCredential::Robot(robot)) = &self.0.machine {
                let Some(domain) = robot.assigned_domain_id(cancellation).await? else {
                    return Ok(None);
                };
                Some(domain)
            } else {
                None
            };
            let claimed = tokio::time::timeout(
                self.0.config.request_timeout,
                self.0.client.claim(capability),
            )
            .await
            .map_err(|_| TaskError::Service("claim"))?
            .map_err(|error| TaskError::dms("claim", error))?;
            let lease = match claimed {
                ClaimOutcome::NoWork => return Ok(None),
                ClaimOutcome::Busy => return Err(TaskError::Busy),
                ClaimOutcome::Leased(lease) => lease,
            };
            if lease.task.capability != capability {
                return Err(TaskError::Authority("claim capability mismatch"));
            }
            if assignment.is_some_and(|domain| Some(domain) != lease.domain_id) {
                return Err(TaskError::Authority(
                    "robot task is outside its assigned Domain",
                ));
            }
            let mut credential = TaskCredential::new(&lease, &self.0.client_id)?;
            credential.closed = self.0.closed.child_token();
            let data = AukiDomainData::new(credential.clone())
                .map_err(|_| TaskError::Data)?
                .in_domain(credential.domain_id());
            let context = TaskContext {
                task: lease.task.clone(),
                credential,
                data,
                progress: Arc::new(Mutex::new(Value::Object(Default::default()))),
                peer: Arc::new(Mutex::new(self.peer_session())),
                owns_peer: self.0.robot_peer.is_none(),
            };
            let peer_grant = if self.0.peer.is_some() && self.0.robot_peer.is_none() {
                TaskPeerGrant::from_wire(
                    context.credential.domain_id(),
                    lease.p2p_access_token.as_deref(),
                    lease.p2p_access_token_expires_at,
                )?
            } else {
                None
            };
            Ok(Some(TaskLease {
                runtime: self.clone(),
                context,
                _permit: permit,
                peer_grant,
            }))
        };
        tokio::select! {
            biased;
            _ = self.stopped() => Err(self.stopped_error()),
            _ = cancellation.cancelled() => Err(TaskError::Cancelled),
            result = operation => result,
        }
    }

    pub async fn run_once(
        &self,
        capability: &str,
        handler: &dyn TaskHandler,
        cancellation: &CancellationToken,
    ) -> Result<TaskOutcome> {
        let lease = match self.claim(capability, cancellation).await {
            Ok(Some(lease)) => lease,
            Ok(None) => return Ok(TaskOutcome::NoWork),
            Err(TaskError::Busy) => return Ok(TaskOutcome::Busy),
            Err(error) => return Err(error),
        };
        lease.execute(handler, cancellation).await?;
        Ok(TaskOutcome::Completed)
    }

    pub async fn run(
        &self,
        handlers: &BTreeMap<String, Arc<dyn TaskHandler>>,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        if self.0.running.swap(true, Ordering::AcqRel) {
            return Err(TaskError::Busy);
        }
        struct RunGuard<'a>(&'a AtomicBool);
        impl Drop for RunGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _running = RunGuard(&self.0.running);
        if self
            .0
            .capabilities
            .iter()
            .any(|c| !handlers.contains_key(c))
            || handlers.len() != self.0.capabilities.len()
        {
            return Err(TaskError::Configuration(
                "handlers must match registered capabilities",
            ));
        }
        loop {
            for capability in &self.0.capabilities {
                match self
                    .run_once(capability, handlers[capability].as_ref(), cancellation)
                    .await
                {
                    Err(TaskError::Cancelled | TaskError::Closed) => return Ok(()),
                    Err(error) => return Err(error),
                    Ok(_) => {}
                }
            }
            tokio::select! {
                biased;
                _ = self.stopped() => return match self.stopped_error() { TaskError::Closed => Ok(()), error => Err(error) },
                _ = cancellation.cancelled() => return Ok(()),
                _ = tokio::time::sleep(self.0.config.poll_interval) => {}
            }
        }
    }

    /// Stop claims, cancel work and await handler, data, peer and registration cleanup.
    /// Retains peer cleanup failures so cancellation cannot hide them from the host.
    /// A custom loop must release its TaskLease before this can finish.
    pub async fn close(&self) -> Result<()> {
        self.0.closed.cancel();
        let _active = self.0.active.acquire().await;
        let peer_cleanup = match &self.0.robot_peer {
            Some(peer) => peer.close().await,
            None => Ok(()),
        };
        if let Some(machine) = &self.0.machine {
            machine.close().await;
        }
        if self.0.cleanup_failed.load(Ordering::Acquire) {
            return Err(TaskError::PeerCleanup);
        }
        peer_cleanup
    }
}

/// A single claim. Dropping it immediately revokes local data access.
pub struct TaskLease {
    runtime: AukiDmsTasks,
    context: TaskContext,
    _permit: OwnedSemaphorePermit,
    peer_grant: Option<TaskPeerGrant>,
}
impl Drop for TaskLease {
    fn drop(&mut self) {
        self.context.revoke();
        if let Some(peer) = self.context.peer.lock().take()
            && self.context.owns_peer
        {
            // Drop cannot await. Managed execution and explicit close drain this
            // before releasing the lease; abandoned native leases get best effort.
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = peer.shutdown().await;
                });
            }
        }
    }
}

impl TaskLease {
    pub fn context(&self) -> TaskContext {
        self.context.clone()
    }

    /// Start optional task networking after the initial heartbeat. Custom loops
    /// must await this call and close the lease; managed execution does both.
    pub async fn start_peer(&mut self, cancellation: &CancellationToken) -> Result<()> {
        let Some(factory) = self.runtime.0.peer.clone() else {
            return Ok(());
        };
        if self.context.peer_session().is_some() {
            return Ok(());
        }
        let grant = self.peer_grant.clone().ok_or(TaskError::Authority(
            "task peer access is unavailable; check provider support",
        ))?;
        let token = self.context.cancellation();
        let startup = factory.start(grant, &token);
        tokio::pin!(startup);
        let result = tokio::select! { biased;
            _ = self.runtime.stopped() => { self.context.revoke(); startup.await },
            _ = cancellation.cancelled() => { self.context.revoke(); startup.await },
            result = &mut startup => result,
        };
        let peer = result.inspect_err(|error| {
            if matches!(error, TaskError::PeerCleanup) {
                self.runtime.0.cleanup_failed.store(true, Ordering::Release);
            }
        })?;
        *self.context.peer.lock() = Some(peer);
        if self.context.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        // Startup can consume lease time; never enter a handler on expired authority.
        self.remaining()?;
        Ok(())
    }

    async fn finish_resources(&self) -> Result<()> {
        self.context.data.close().await;
        let peer = self.context.peer.lock().take();
        if let Some(peer) = peer
            && self.context.owns_peer
        {
            peer.fence();
            peer.shutdown().await.map_err(|_| {
                self.runtime.0.cleanup_failed.store(true, Ordering::Release);
                TaskError::PeerCleanup
            })?;
        }
        Ok(())
    }

    /// Revoke local access and await peer/data cleanup without reporting success.
    pub async fn close(self) -> Result<()> {
        self.context.revoke();
        self.finish_resources().await
    }

    fn remaining(&self) -> Result<Duration> {
        let mut deadline = self.context.credential.deadline()?;
        if let Some(grant) = &self.peer_grant {
            deadline = deadline.min(grant.expires_at);
        }
        let duration = deadline
            .signed_duration_since(Utc::now())
            .to_std()
            .map_err(|_| TaskError::LeaseLost)?;
        if duration.is_zero() {
            return Err(TaskError::LeaseLost);
        }
        Ok(duration.min(self.runtime.0.config.request_timeout))
    }

    pub async fn heartbeat(&mut self) -> Result<()> {
        let runtime = self.runtime.clone();
        let context = self.context.clone();
        let progress = self.context.progress.lock().clone();
        let request = HeartbeatRequest {
            progress,
            events: vec![],
        };
        let operation = async {
            let response = tokio::time::timeout(
                self.remaining()?,
                self.runtime
                    .0
                    .client
                    .heartbeat(self.context.task.id, &request),
            )
            .await
            .map_err(|_| TaskError::LeaseLost)?
            .map_err(|error| TaskError::dms("heartbeat", error))?;
            self.context
                .credential
                .update(self.context.task.id, &response)?;
            if self.runtime.0.peer.is_some() && self.context.owns_peer {
                let grant = TaskPeerGrant::from_wire(
                    self.context.credential.domain_id(),
                    response.p2p_access_token.as_deref(),
                    response.p2p_access_token_expires_at,
                )?;
                if let Some(grant) = grant {
                    if let Some(peer) = self.context.peer_session() {
                        peer.update(grant.clone()).await?;
                    }
                    self.peer_grant = Some(grant);
                }
            }
            Ok(())
        };
        let result = tokio::select! {
            biased;
            _ = runtime.stopped() => Err(runtime.stopped_error()),
            _ = context.credential.closed.cancelled() => Err(TaskError::Cancelled),
            result = operation => result,
        };
        if result.is_err() {
            self.context.revoke();
        }
        result
    }

    async fn heartbeat_loop(&mut self) -> Result<()> {
        loop {
            let delay = self
                .remaining()?
                .div_f64(2.0)
                .min(self.runtime.0.config.heartbeat_interval);
            let peer = self.context.peer_session();
            tokio::select! {
                biased;
                _ = self.context.credential.closed.cancelled() => return Err(TaskError::Cancelled),
                _ = self.context.credential.changed.notified() => {},
                _ = async { match &peer { Some(peer) if self.context.owns_peer => peer.refresh_requested().await, _ => std::future::pending().await } } => {},
                _ = async { match &peer { Some(peer) => peer.wait_stopped().await, None => std::future::pending().await } } => return Err(TaskError::Authority("task peer stopped")),
                _ = tokio::time::sleep(delay) => {}
            }
            self.heartbeat().await?;
        }
    }

    pub async fn complete(self, result: TaskResult) -> Result<()> {
        self.finish_resources().await?;
        if self.context.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        tokio::time::timeout(
            self.remaining()?,
            self.runtime
                .0
                .client
                .complete(self.context.task.id, &result),
        )
        .await
        .map_err(|_| TaskError::Service("complete"))?
        .map_err(|error| TaskError::dms("complete", error))
    }

    pub async fn fail(self, failure: FailTaskRequest) -> Result<()> {
        self.finish_resources().await?;
        if self.context.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        tokio::time::timeout(
            self.remaining()?,
            self.runtime.0.client.fail(self.context.task.id, &failure),
        )
        .await
        .map_err(|_| TaskError::Service("fail"))?
        .map_err(|error| TaskError::dms("fail", error))
    }

    async fn execute(
        mut self,
        handler: &dyn TaskHandler,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        // Like the Posemesh host, validate current authority before starting work.
        let startup = async {
            self.heartbeat().await?;
            self.start_peer(cancellation).await
        }
        .await;
        if let Err(error) = startup {
            self.context.revoke();
            self.finish_resources().await?;
            return Err(error);
        }
        let context = self.context.clone();
        let runtime = self.runtime.clone();
        let outcome = {
            let started = AtomicBool::new(false);
            let execution = async {
                started.store(true, Ordering::Release);
                handler.run(context.clone()).await
            };
            tokio::pin!(execution);
            let heartbeat = self.heartbeat_loop();
            tokio::pin!(heartbeat);
            tokio::select! {
                biased;
                _ = runtime.stopped() => { context.revoke(); if started.load(Ordering::Acquire) { let _ = execution.await; } Err(runtime.stopped_error()) },
                _ = cancellation.cancelled() => { context.revoke(); if started.load(Ordering::Acquire) { let _ = execution.await; } Err(TaskError::Cancelled) },
                result = &mut heartbeat => { context.revoke(); if started.load(Ordering::Acquire) { let _ = execution.await; } result.and(Err(TaskError::LeaseLost)) },
                result = &mut execution => result,
            }
        };
        if context.is_cancelled() {
            self.finish_resources().await?;
            return outcome.map(|_| ()).and(Err(TaskError::Cancelled));
        }
        // Flush the final progress and check cancellation before publishing a result.
        let heartbeat_result = self.heartbeat().await;
        if let Err(error) = heartbeat_result {
            self.finish_resources().await?;
            return Err(error);
        }
        match outcome {
            Ok(result) => self.complete(result).await,
            Err(_) => {
                self.fail(FailTaskRequest {
                    reason: "task handler failed".into(),
                    details: Value::Null,
                })
                .await?;
                Err(TaskError::Handler)
            }
        }
    }
}
