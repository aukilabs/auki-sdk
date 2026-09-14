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

use crate::{AukiComputeCredential, Result, TaskCredential, TaskError};

pub type TaskResult = CompleteTaskRequest;

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
}
impl TaskContext {
    pub fn data(&self) -> DomainDataClient {
        self.data.clone()
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
    machine: Option<AukiComputeCredential>,
    capabilities: Vec<String>,
    config: TasksConfig,
    closed: CancellationToken,
    active: Arc<Semaphore>,
    running: AtomicBool,
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
        machine: AukiComputeCredential,
        capabilities: Vec<String>,
        config: TasksConfig,
    ) -> Result<Self> {
        let client = DmsClient::new(
            machine.config().dms_url.clone(),
            config.request_timeout,
            Arc::new(machine.clone()),
        )
        .map_err(|_| TaskError::Configuration("cannot construct DMS client"))?;
        Self::build(
            client,
            machine.config().client_id.clone(),
            capabilities,
            config,
            Some(machine),
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
        Self::build(client, client_id, capabilities, config, None)
    }

    fn build(
        client: DmsClient,
        client_id: String,
        mut capabilities: Vec<String>,
        config: TasksConfig,
        machine: Option<AukiComputeCredential>,
    ) -> Result<Self> {
        if capabilities.is_empty()
            || capabilities.len() > 100
            || capabilities
                .iter()
                .any(|c| c.is_empty() || c.len() > 256 || c.chars().any(char::is_control))
        {
            return Err(TaskError::Configuration("provide 1-100 valid capabilities"));
        }
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
            machine.attach_runtime()?;
        }
        Ok(Self(Arc::new(Owner {
            client,
            client_id,
            machine,
            capabilities,
            config,
            closed: CancellationToken::new(),
            active: Arc::new(Semaphore::new(1)),
            running: AtomicBool::new(false),
        })))
    }

    pub fn capabilities(&self) -> &[String] {
        &self.0.capabilities
    }

    pub fn request_shutdown(&self) {
        self.0.closed.cancel();
    }

    fn stopped_error(&self) -> TaskError {
        if self
            .0
            .machine
            .as_ref()
            .is_some_and(AukiComputeCredential::failed)
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
            if let Some(machine) = &self.0.machine {
                machine.start(&self.0.capabilities, cancellation).await?;
            }
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
            };
            Ok(Some(TaskLease {
                runtime: self.clone(),
                context,
                _permit: permit,
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

    /// Stop claims, cancel active work and await handler/data cleanup and registration.
    /// A custom loop must release its TaskLease before this can finish.
    pub async fn close(&self) {
        self.0.closed.cancel();
        let _active = self.0.active.acquire().await;
        if let Some(machine) = &self.0.machine {
            machine.close().await;
        }
    }
}

/// A single claim. Dropping it immediately revokes local data access.
pub struct TaskLease {
    runtime: AukiDmsTasks,
    context: TaskContext,
    _permit: OwnedSemaphorePermit,
}
impl Drop for TaskLease {
    fn drop(&mut self) {
        self.context.credential.revoke();
    }
}

impl TaskLease {
    pub fn context(&self) -> TaskContext {
        self.context.clone()
    }

    fn remaining(&self) -> Result<Duration> {
        let duration = self
            .context
            .credential
            .deadline()?
            .signed_duration_since(Utc::now())
            .to_std()
            .map_err(|_| TaskError::LeaseLost)?;
        if duration.is_zero() {
            return Err(TaskError::LeaseLost);
        }
        Ok(duration.min(self.runtime.0.config.request_timeout))
    }

    pub async fn heartbeat(&mut self) -> Result<()> {
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
                .update(self.context.task.id, &response)
        };
        let result = tokio::select! {
            biased;
            _ = self.runtime.stopped() => Err(self.runtime.stopped_error()),
            _ = self.context.credential.closed.cancelled() => Err(TaskError::Cancelled),
            result = operation => result,
        };
        if result.is_err() {
            self.context.credential.revoke();
        }
        result
    }

    async fn heartbeat_loop(&mut self) -> Result<()> {
        loop {
            let delay = self
                .remaining()?
                .div_f64(2.0)
                .min(self.runtime.0.config.heartbeat_interval);
            tokio::select! {
                biased;
                _ = self.context.credential.closed.cancelled() => return Err(TaskError::Cancelled),
                _ = self.context.credential.changed.notified() => {},
                _ = tokio::time::sleep(delay) => {}
            }
            self.heartbeat().await?;
        }
    }

    pub async fn complete(self, result: TaskResult) -> Result<()> {
        self.context.data.close().await;
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
        self.context.data.close().await;
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
        self.heartbeat().await?;
        let context = self.context.clone();
        let runtime = self.runtime.clone();
        let outcome = {
            let execution = handler.run(context.clone());
            tokio::pin!(execution);
            let heartbeat = self.heartbeat_loop();
            tokio::pin!(heartbeat);
            tokio::select! {
                biased;
                _ = runtime.stopped() => { context.credential.revoke(); let _ = execution.await; Err(runtime.stopped_error()) },
                _ = cancellation.cancelled() => { context.credential.revoke(); let _ = execution.await; Err(TaskError::Cancelled) },
                result = &mut heartbeat => { context.credential.revoke(); let _ = execution.await; result.and(Err(TaskError::LeaseLost)) },
                result = &mut execution => result,
            }
        };
        if context.is_cancelled() {
            context.data.close().await;
            return outcome.map(|_| ()).and(Err(TaskError::Cancelled));
        }
        // Flush the final progress and check cancellation before publishing a result.
        let heartbeat_result = self.heartbeat().await;
        context.data.close().await;
        heartbeat_result?;
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
