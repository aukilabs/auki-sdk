use std::sync::Arc;

use async_trait::async_trait;
use auki_auth::{DomainAccess, DomainAccessProvider, Error as AuthError, SecretString};
use auki_dms::types::{HeartbeatResponse, LeaseEnvelope};
use chrono::{DateTime, Utc};
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{Result, TaskError};

#[derive(Clone)]
struct Grant {
    access: Arc<DomainAccess>,
    lease_expires: DateTime<Utc>,
    lease: LeaseEnvelope,
}

/// Data authority supplied by the lease owner. It never logs in or heartbeats.
#[derive(Clone)]
pub struct TaskCredential {
    domain: Uuid,
    client_id: Arc<str>,
    grant: Arc<watch::Sender<Option<Grant>>>,
    pub(crate) changed: Arc<Notify>,
    pub(crate) closed: CancellationToken,
}

/// Read-only view of the current task's Domain HTTP bearer. Clones share renewal
/// and revocation with the lease owner; this handle never refreshes credentials.
#[derive(Clone)]
pub struct TaskAccessToken(pub(crate) TaskCredential);

impl std::fmt::Debug for TaskAccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TaskAccessToken([REDACTED])")
    }
}

impl TaskAccessToken {
    /// Read the latest bearer synchronously. Fails after cancellation, expiry or
    /// task completion. Already returned copies cannot be revoked locally: read
    /// immediately before each request, never cache or log the returned secret.
    pub fn get(&self) -> Result<SecretString> {
        let state = self.0.grant.borrow();
        if self.0.closed.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        let grant = state.as_ref().ok_or(TaskError::LeaseLost)?;
        if grant.lease_expires.min(grant.access.expires_at()) <= Utc::now() {
            return Err(TaskError::LeaseLost);
        }
        Ok(SecretString::new(grant.access.bearer().expose_secret()))
    }
}

impl TaskCredential {
    pub(crate) fn new(lease: &LeaseEnvelope, client_id: &str) -> Result<Self> {
        let domain = lease
            .domain_id
            .ok_or(TaskError::Authority("missing Domain"))?;
        let server = lease
            .domain_server_url
            .as_ref()
            .ok_or(TaskError::Authority("missing server URL"))?;
        let token = lease
            .access_token
            .as_ref()
            .ok_or(TaskError::Authority("missing data token"))?;
        let expires = lease
            .access_token_expires_at
            .ok_or(TaskError::Authority("missing token expiry"))?;
        let lease_expires = lease
            .lease_expires_at
            .or(lease.task.lease_expires_at)
            .ok_or(TaskError::Authority("missing lease expiry"))?;
        if lease.cancel || lease_expires <= Utc::now() {
            return Err(TaskError::LeaseLost);
        }
        let access = DomainAccess::from_issued_grant(
            domain,
            server.as_str(),
            SecretString::new(token),
            expires,
        )
        .map_err(|_| TaskError::Authority("invalid data grant"))?;
        let (grant, _) = watch::channel(Some(Grant {
            access: Arc::new(access),
            lease_expires,
            lease: metadata_only(lease),
        }));
        Ok(Self {
            domain,
            client_id: client_id.into(),
            grant: Arc::new(grant),
            changed: Arc::new(Notify::new()),
            closed: CancellationToken::new(),
        })
    }

    pub fn domain_id(&self) -> Uuid {
        self.domain
    }

    /// Snapshot for native hosts adapting an existing lease-based runner API.
    /// Contains the current Domain bearer; never log or serialize it into reports.
    /// P2P credentials remain private to the transport owner.
    pub fn lease_snapshot(&self) -> Result<LeaseEnvelope> {
        let state = self.grant.borrow();
        if self.closed.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        let grant = state.as_ref().ok_or(TaskError::LeaseLost)?;
        if grant.lease_expires.min(grant.access.expires_at()) <= Utc::now() {
            return Err(TaskError::LeaseLost);
        }
        let mut lease = grant.lease.clone();
        lease.access_token = Some(grant.access.bearer().expose_secret().to_owned());
        lease.access_token_expires_at = Some(grant.access.expires_at());
        lease.lease_expires_at = Some(grant.lease_expires);
        lease.domain_server_url = Some(grant.access.server_url().clone());
        Ok(lease)
    }

    pub(crate) fn task_snapshot(&self) -> Result<auki_dms::types::TaskSpec> {
        self.grant
            .borrow()
            .as_ref()
            .map(|grant| grant.lease.task.clone())
            .ok_or(TaskError::LeaseLost)
    }

    pub(crate) fn deadline(&self) -> Result<DateTime<Utc>> {
        let state = self.grant.borrow();
        let grant = state.as_ref().ok_or(TaskError::LeaseLost)?;
        Ok(grant.lease_expires.min(grant.access.expires_at()))
    }

    pub(crate) fn revoke(&self) {
        self.closed.cancel();
        self.grant.send_replace(None);
    }

    pub(crate) fn update(&self, task_id: Uuid, update: &HeartbeatResponse) -> Result<()> {
        if update.cancel == Some(true)
            || update
                .status
                .as_deref()
                .is_some_and(|s| matches!(s, "cancelled" | "failed" | "completed"))
        {
            return Err(TaskError::Cancelled);
        }
        if update.domain_id.is_some_and(|id| id != self.domain)
            || update.task_id.is_some_and(|id| id != task_id)
            || update.task.as_ref().is_some_and(|task| task.id != task_id)
        {
            return Err(TaskError::Authority("heartbeat changed task or Domain"));
        }
        let previous = self.grant.borrow().clone().ok_or(TaskError::LeaseLost)?;
        if update
            .task
            .as_ref()
            .is_some_and(|task| task.capability != previous.lease.task.capability)
        {
            return Err(TaskError::Authority("heartbeat changed task capability"));
        }
        let server = update
            .domain_server_url
            .as_ref()
            .unwrap_or(previous.access.server_url());
        let token = update
            .access_token
            .as_deref()
            .unwrap_or(previous.access.bearer().expose_secret());
        let expires = update
            .access_token_expires_at
            .unwrap_or(previous.access.expires_at());
        let lease_expires = update
            .lease_expires_at
            .or_else(|| update.task.as_ref().and_then(|t| t.lease_expires_at))
            .unwrap_or(previous.lease_expires);
        if lease_expires <= Utc::now() || self.closed.is_cancelled() {
            return Err(TaskError::LeaseLost);
        }
        let access = DomainAccess::from_issued_grant(
            self.domain,
            server.as_str(),
            SecretString::new(token),
            expires,
        )
        .map_err(|_| TaskError::Authority("invalid rotated data grant"))?;
        let mut lease = previous.lease;
        if let Some(task) = &update.task {
            lease.task = task.clone();
        }
        if let Some(job) = update.job_id {
            lease.task.job_id = Some(job);
        }
        if let Some(attempts) = update.attempts {
            lease.task.attempts = Some(attempts);
        }
        if let Some(attempts) = update.max_attempts {
            lease.task.max_attempts = Some(attempts);
        }
        if let Some(deps) = update.deps_remaining {
            lease.task.deps_remaining = Some(deps);
        }
        if let Some(status) = &update.status {
            lease.status = Some(status.clone());
        }
        self.grant.send_replace(Some(Grant {
            access: Arc::new(access),
            lease_expires,
            lease,
        }));
        Ok(())
    }
}

fn metadata_only(lease: &LeaseEnvelope) -> LeaseEnvelope {
    let mut lease = lease.without_p2p_credentials();
    // Keep one canonical, zeroizing copy of the current Domain bearer.
    lease.access_token = None;
    lease.access_token_expires_at = None;
    lease
}

#[async_trait]
impl DomainAccessProvider for TaskCredential {
    fn client_id(&self) -> &str {
        &self.client_id
    }

    async fn wait_closed(&self) {
        let mut receiver = self.grant.subscribe();
        loop {
            let deadline = match self.deadline() {
                Ok(value) => value,
                Err(_) => return,
            };
            let remaining = deadline
                .signed_duration_since(Utc::now())
                .to_std()
                .unwrap_or_default();
            tokio::select! {
                biased;
                _ = self.closed.cancelled() => return,
                changed = receiver.changed() => { if changed.is_err() { return; } }
                _ = tokio::time::sleep(remaining) => { self.revoke(); return; }
            }
        }
    }

    async fn domain_access(
        &self,
        domain_id: Uuid,
        rejected: Option<&DomainAccess>,
        cancellation: &CancellationToken,
    ) -> auki_auth::Result<Arc<DomainAccess>> {
        if domain_id != self.domain {
            return Err(AuthError::DomainNotAccessible);
        }
        let mut receiver = self.grant.subscribe();
        let mut waited = false;
        loop {
            if self.closed.is_cancelled() {
                return Err(AuthError::SessionClosed);
            }
            if cancellation.is_cancelled() {
                return Err(AuthError::Cancelled {
                    endpoint: "task data grant",
                });
            }
            let state = receiver
                .borrow_and_update()
                .clone()
                .ok_or(AuthError::SessionClosed)?;
            if state.lease_expires.min(state.access.expires_at()) <= Utc::now() {
                self.revoke();
                return Err(AuthError::StaleAuthority);
            }
            if rejected.is_none_or(|old| old.bearer() != state.access.bearer()) {
                return Ok(state.access);
            }
            if waited {
                return Err(AuthError::AuthenticationRequired);
            }
            // Ask the sole lease owner for an early heartbeat; never renew here.
            self.changed.notify_one();
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(AuthError::Cancelled { endpoint: "task data grant" }),
                _ = self.wait_closed() => return Err(AuthError::SessionClosed),
                value = receiver.changed() => { if value.is_err() { return Err(AuthError::SessionClosed); } }
            }
            waited = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;

    #[test]
    fn synchronous_token_handle_checks_lease_expiry_without_a_background_runtime() {
        let domain = Uuid::new_v4();
        let expires = Utc::now() + chrono::Duration::minutes(1);
        let claims = json!({"iss":"dds", "aud":["http://127.0.0.1:12345"], "domain_id":domain, "exp":expires.timestamp()});
        let bearer = format!(
            "e30.{}.fixture",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        );
        let lease: LeaseEnvelope = serde_json::from_value(json!({
            "task":{"id":Uuid::new_v4(),"capability":"/fixture/v1"},
            "domain_id":domain,"domain_server_url":"http://127.0.0.1:12345",
            "access_token":bearer,"access_token_expires_at":expires,"lease_expires_at":expires
        }))
        .unwrap();
        let credential = TaskCredential::new(&lease, "fixture").unwrap();
        let token = TaskAccessToken(credential.clone());
        assert_eq!(token.get().unwrap().expose_secret(), bearer);
        credential.grant.send_modify(|grant| {
            grant.as_mut().unwrap().lease_expires = Utc::now() - chrono::Duration::seconds(1);
        });
        assert!(matches!(token.get(), Err(TaskError::LeaseLost)));
        credential.revoke();
        assert!(matches!(token.get(), Err(TaskError::Cancelled)));
    }
}
