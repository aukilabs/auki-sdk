use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use auki_auth::{AuthorityRenewal, PreparedPeer, RenewedAuthority};
use auki_p2p::{
    DdsTokenVerifier, DdsVerificationKeys, DomainAuthority as P2pDomainAuthority, P2PAccessClaims,
    P2pCredentialError, PeerId, SignedP2pCredential,
};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use reqwest::header::HeaderValue;
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use crate::{
    authorization::{
        AukiPeerAuthorizationError, AukiPeerAuthorizationSnapshot, AuthorizationSnapshotSource,
    },
    relay::{RelayAuthorizationError, RelayAuthorizationProvider, RelayAuthorizationSnapshot},
    runtime_policy::{
        RejectedAuthorityRevision, next_authority_revision, rejected_authority_revision,
    },
};

#[derive(Clone, Debug)]
pub(crate) struct AuthoritySupervisorConfig {
    pub(crate) renewal_attempt_timeout: Duration,
    pub(crate) early_refresh_timeout: Duration,
    pub(crate) retry_initial: Duration,
    pub(crate) retry_max: Duration,
}

impl Default for AuthoritySupervisorConfig {
    fn default() -> Self {
        Self {
            renewal_attempt_timeout: Duration::from_secs(10),
            early_refresh_timeout: Duration::from_secs(10),
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(30),
        }
    }
}

impl AuthoritySupervisorConfig {
    fn validate(&self) -> Result<(), AuthoritySupervisorError> {
        if self.renewal_attempt_timeout.is_zero()
            || self.early_refresh_timeout.is_zero()
            || self.retry_initial.is_zero()
            || self.retry_max.is_zero()
            || self.retry_initial > self.retry_max
        {
            return Err(AuthoritySupervisorError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AuthorityStatus {
    Starting,
    Ready {
        credential_revision: u64,
        expires_at: DateTime<Utc>,
    },
    Expired {
        credential_revision: u64,
        expired_at: DateTime<Utc>,
    },
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthorityInstallOutcome {
    Replaced(u64),
    Unchanged(u64),
}

impl AuthorityInstallOutcome {
    fn advanced(self) -> bool {
        matches!(self, Self::Replaced(_))
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AuthoritySupervisorError {
    #[error("authority supervisor configuration is invalid")]
    InvalidConfiguration,
    #[error("authority supervisor is stopped")]
    Stopped,
    #[error("authority update targets Domain {actual}, expected {expected}")]
    DomainMismatch { expected: Uuid, actual: Uuid },
    #[error("authority update targets Peer {actual}, expected {expected}")]
    PeerMismatch { expected: String, actual: String },
    #[error("signed authority targets a different local Peer")]
    SignedPeerMismatch,
    #[error("signed authority does not authorize the fixed Domain")]
    SignedDomainMismatch,
    #[error("authority expiration is invalid or already elapsed")]
    InvalidExpiration,
    #[error("signed authority expiration differs from its host envelope")]
    SignedExpirationMismatch,
    #[error("authority renewal time must be earlier than literal expiration")]
    InvalidRenewalSchedule,
    #[error("changed authority issued at {proposed} does not advance current issued-at {current}")]
    NonAdvancingCredential { current: u64, proposed: u64 },
    #[error("authority credential revision is exhausted")]
    RevisionExhausted,
    #[error("verification-key update is invalid")]
    VerificationKeys(#[source] auki_p2p::Error),
    #[error("signed authority is invalid")]
    Credential(#[source] auki_p2p::Error),
    #[error("live Domain authority installation failed")]
    Install(#[source] AuthorityInstallerError),
    #[error("authority pull renewal failed")]
    PullRenewal(#[source] auki_auth::Error),
    #[error("authority refresh did not complete before its deadline")]
    RefreshTimedOut,
    #[error("the selected authority source cannot refresh")]
    RefreshUnavailable,
    #[error("the rejected credential revision is not current")]
    RejectedRevisionMismatch,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AuthorityInstallerError {
    #[error("verification keys were rejected by the authenticated peer runtime")]
    VerificationKeys(#[source] auki_p2p::Error),
    #[error("credential was rejected by the authenticated peer runtime")]
    Credential(#[source] P2pCredentialError),
    #[cfg(test)]
    #[error("test installer rejected {0}")]
    Injected(&'static str),
}

#[async_trait]
trait AuthorityInstaller: Send + Sync {
    fn domain_id(&self) -> Uuid;
    fn peer_id(&self) -> PeerId;

    async fn install_verification_keys(
        &self,
        keys: DdsVerificationKeys,
    ) -> Result<(), AuthorityInstallerError>;

    async fn install_credential(
        &self,
        credential: SignedP2pCredential,
    ) -> Result<(), AuthorityInstallerError>;
}

pub(crate) struct FixedDomainAuthority {
    authority: P2pDomainAuthority,
    domain_id: Uuid,
}

impl FixedDomainAuthority {
    pub(crate) fn new(authority: P2pDomainAuthority, domain_id: Uuid) -> Self {
        Self {
            authority,
            domain_id,
        }
    }
}

#[async_trait]
impl AuthorityInstaller for FixedDomainAuthority {
    fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    fn peer_id(&self) -> PeerId {
        self.authority.peer_id()
    }

    async fn install_verification_keys(
        &self,
        keys: DdsVerificationKeys,
    ) -> Result<(), AuthorityInstallerError> {
        self.authority
            .install_verification_keys(keys)
            .await
            .map_err(AuthorityInstallerError::VerificationKeys)
    }

    async fn install_credential(
        &self,
        credential: SignedP2pCredential,
    ) -> Result<(), AuthorityInstallerError> {
        self.authority
            .install_credential_for_domain(credential, self.domain_id)
            .await
            .map(|_| ())
            .map_err(AuthorityInstallerError::Credential)
    }
}

/// Complete externally managed authority material for one fixed Domain and Peer.
///
/// Debug output and getters deliberately omit the signed credential and PEM
/// verification-key material. Constructing an update transfers those secrets
/// into the runtime's authority supervisor.
pub struct ExternalAuthorityUpdate {
    domain_id: Uuid,
    peer_id: PeerId,
    verification_keys: DdsVerificationKeys,
    credential: SignedP2pCredential,
    credential_expires_at: DateTime<Utc>,
}

impl ExternalAuthorityUpdate {
    /// Construct one complete authority replacement envelope.
    pub fn new(
        domain_id: Uuid,
        peer_id: PeerId,
        verification_keys: DdsVerificationKeys,
        credential: SignedP2pCredential,
        credential_expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            domain_id,
            peer_id,
            verification_keys,
            credential,
            credential_expires_at,
        }
    }

    /// Domain pinned by this update.
    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    /// Peer pinned by this update.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Monotonic DDS verification-key generation carried by this update.
    pub fn verification_key_generation(&self) -> u64 {
        self.verification_keys.generation()
    }

    /// Literal signed-credential expiration carried by this update.
    pub fn credential_expires_at(&self) -> DateTime<Utc> {
        self.credential_expires_at
    }

    pub(crate) fn verification_keys(&self) -> &DdsVerificationKeys {
        &self.verification_keys
    }
}

impl fmt::Debug for ExternalAuthorityUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalAuthorityUpdate")
            .field("domain_id", &self.domain_id)
            .field("peer_id", &self.peer_id)
            .field(
                "verification_key_generation",
                &self.verification_keys.generation(),
            )
            .field("credential", &"[redacted]")
            .field("credential_expires_at", &self.credential_expires_at)
            .finish()
    }
}

struct AuthorityUpdate {
    domain_id: Uuid,
    peer_id: PeerId,
    verification_keys: DdsVerificationKeys,
    credential: SignedP2pCredential,
    credential_expires_at: DateTime<Utc>,
    renew_at: Option<DateTime<Utc>>,
}

impl From<ExternalAuthorityUpdate> for AuthorityUpdate {
    fn from(update: ExternalAuthorityUpdate) -> Self {
        Self {
            domain_id: update.domain_id,
            peer_id: update.peer_id,
            verification_keys: update.verification_keys,
            credential: update.credential,
            credential_expires_at: update.credential_expires_at,
            renew_at: None,
        }
    }
}

impl From<RenewedAuthority> for AuthorityUpdate {
    fn from(update: RenewedAuthority) -> Self {
        Self {
            domain_id: update.domain.id,
            peer_id: update.peer_id,
            verification_keys: update.verification_keys,
            credential: update.credential,
            credential_expires_at: update.credential_expires_at,
            renew_at: Some(update.renew_at),
        }
    }
}

struct ValidatedAuthorityUpdate {
    verification_keys: DdsVerificationKeys,
    credential: SignedP2pCredential,
    claims: P2PAccessClaims,
    authorization: HeaderValue,
    credential_expires_at: DateTime<Utc>,
    renew_at: Option<DateTime<Utc>>,
    equivalent_credential: bool,
}

struct CurrentAuthority {
    credential_revision: u64,
    claims: P2PAccessClaims,
    authorization: HeaderValue,
    credential_expires_at: DateTime<Utc>,
    renew_at: Option<DateTime<Utc>>,
    available: bool,
}

struct AuthorityState {
    stopped: bool,
    installed_keys: Option<DdsVerificationKeys>,
    current: Option<CurrentAuthority>,
}

enum AuthorityMode {
    Pull(watch::Sender<Option<u64>>),
    External(watch::Sender<Option<ExternalAuthorityRefreshRequest>>),
}

struct AuthorityInner {
    installer: Arc<dyn AuthorityInstaller>,
    config: AuthoritySupervisorConfig,
    mode: AuthorityMode,
    state: RwLock<AuthorityState>,
    update_lock: Mutex<()>,
    refresh_lock: Mutex<()>,
    status: watch::Sender<AuthorityStatus>,
    shutdown: CancellationToken,
    next_refresh_request: AtomicU64,
}

impl AuthorityInner {
    fn new(
        installer: Arc<dyn AuthorityInstaller>,
        config: AuthoritySupervisorConfig,
        mode: AuthorityMode,
        shutdown: CancellationToken,
    ) -> Arc<Self> {
        let (status, _) = watch::channel(AuthorityStatus::Starting);
        Arc::new(Self {
            installer,
            config,
            mode,
            state: RwLock::new(AuthorityState {
                stopped: false,
                installed_keys: None,
                current: None,
            }),
            update_lock: Mutex::new(()),
            refresh_lock: Mutex::new(()),
            status,
            shutdown,
            next_refresh_request: AtomicU64::new(1),
        })
    }

    async fn apply_update(
        &self,
        update: AuthorityUpdate,
    ) -> Result<AuthorityInstallOutcome, AuthoritySupervisorError> {
        let _update = self.update_lock.lock().await;
        self.require_running()?;
        let validated = self.validate_update(update)?;

        self.installer
            .install_verification_keys(validated.verification_keys.clone())
            .await
            .map_err(AuthoritySupervisorError::Install)?;
        {
            let mut state = self.state.write();
            state.installed_keys = Some(validated.verification_keys);
        }

        self.require_running()?;
        if validated.equivalent_credential {
            let revision = self
                .state
                .read()
                .current
                .as_ref()
                .ok_or(AuthoritySupervisorError::RefreshUnavailable)?
                .credential_revision;
            return Ok(AuthorityInstallOutcome::Unchanged(revision));
        }

        self.installer
            .install_credential(validated.credential)
            .await
            .map_err(AuthoritySupervisorError::Install)?;

        let status = {
            let mut state = self.state.write();
            if state.stopped || self.shutdown.is_cancelled() {
                return Err(AuthoritySupervisorError::Stopped);
            }
            let credential_revision = next_authority_revision(
                state
                    .current
                    .as_ref()
                    .map(|current| current.credential_revision),
            )
            .ok_or(AuthoritySupervisorError::RevisionExhausted)?;
            let expires_at = validated.credential_expires_at;
            state.current = Some(CurrentAuthority {
                credential_revision,
                claims: validated.claims,
                authorization: validated.authorization,
                credential_expires_at: expires_at,
                renew_at: validated.renew_at,
                available: true,
            });
            AuthorityStatus::Ready {
                credential_revision,
                expires_at,
            }
        };
        self.status.send_replace(status.clone());
        if let AuthorityMode::External(requests) = &self.mode {
            requests.send_replace(None);
        }
        match status {
            AuthorityStatus::Ready {
                credential_revision,
                ..
            } => Ok(AuthorityInstallOutcome::Replaced(credential_revision)),
            _ => unreachable!("a successful authority update always publishes Ready"),
        }
    }

    fn validate_update(
        &self,
        update: AuthorityUpdate,
    ) -> Result<ValidatedAuthorityUpdate, AuthoritySupervisorError> {
        let expected_domain = self.installer.domain_id();
        if update.domain_id != expected_domain {
            return Err(AuthoritySupervisorError::DomainMismatch {
                expected: expected_domain,
                actual: update.domain_id,
            });
        }
        let expected_peer = self.installer.peer_id();
        if update.peer_id != expected_peer {
            return Err(AuthoritySupervisorError::PeerMismatch {
                expected: expected_peer.to_string(),
                actual: update.peer_id.to_string(),
            });
        }
        let expected_expiration = exact_expiration(update.credential_expires_at)?;
        if update
            .renew_at
            .is_some_and(|renew_at| renew_at >= update.credential_expires_at)
        {
            return Err(AuthoritySupervisorError::InvalidRenewalSchedule);
        }

        let state = self.state.read();
        if state.stopped {
            return Err(AuthoritySupervisorError::Stopped);
        }
        if let Some(current_keys) = &state.installed_keys {
            current_keys
                .validate_successor(&update.verification_keys)
                .map_err(AuthoritySupervisorError::VerificationKeys)?;
        }
        let verifier = DdsTokenVerifier::from_keys(update.verification_keys.clone())
            .map_err(AuthoritySupervisorError::VerificationKeys)?;
        let claims = verifier
            .verify_credential(&update.credential)
            .map_err(AuthoritySupervisorError::Credential)?;
        if claims.peer_id != expected_peer.to_string() {
            return Err(AuthoritySupervisorError::SignedPeerMismatch);
        }
        if !claims
            .domain_ids
            .iter()
            .any(|domain_id| domain_id == &expected_domain.to_string())
        {
            return Err(AuthoritySupervisorError::SignedDomainMismatch);
        }
        if claims.exp != expected_expiration {
            return Err(AuthoritySupervisorError::SignedExpirationMismatch);
        }
        let authorization = update
            .credential
            .to_sensitive_bearer_header()
            .map_err(AuthoritySupervisorError::Credential)?;
        // ES256 may produce different compact signatures for the same claims.
        // Authority is defined by the fully verified claims, so retain the
        // installed bearer and revision when only the signature differs.
        let equivalent_credential = state
            .current
            .as_ref()
            .is_some_and(|current| current.claims == claims);
        if !equivalent_credential
            && let Some(current) = &state.current
            && claims.iat <= current.claims.iat
        {
            return Err(AuthoritySupervisorError::NonAdvancingCredential {
                current: current.claims.iat,
                proposed: claims.iat,
            });
        }
        drop(state);

        Ok(ValidatedAuthorityUpdate {
            verification_keys: update.verification_keys,
            credential: update.credential,
            claims,
            authorization,
            credential_expires_at: update.credential_expires_at,
            renew_at: update.renew_at,
            equivalent_credential,
        })
    }

    fn require_running(&self) -> Result<(), AuthoritySupervisorError> {
        if self.shutdown.is_cancelled() || self.state.read().stopped {
            Err(AuthoritySupervisorError::Stopped)
        } else {
            Ok(())
        }
    }

    fn expire_if_due(&self) {
        if self.shutdown.is_cancelled() {
            self.stop();
            return;
        }
        let now = Utc::now();
        let expired = {
            let mut state = self.state.write();
            if state.stopped {
                None
            } else {
                state.current.as_mut().and_then(|current| {
                    if current.available && current.credential_expires_at <= now {
                        current.available = false;
                        Some(AuthorityStatus::Expired {
                            credential_revision: current.credential_revision,
                            expired_at: current.credential_expires_at,
                        })
                    } else {
                        None
                    }
                })
            }
        };
        if let Some(status) = expired {
            self.status.send_replace(status);
        }
    }

    fn authorization_snapshot(
        &self,
    ) -> Result<RelayAuthorizationSnapshot, AuthoritySupervisorError> {
        self.expire_if_due();
        let snapshot = {
            let state = self.state.read();
            if state.stopped {
                return Err(AuthoritySupervisorError::Stopped);
            }
            let current = state
                .current
                .as_ref()
                .filter(|current| current.available)
                .ok_or(AuthoritySupervisorError::RefreshUnavailable)?;
            RelayAuthorizationSnapshot::new(
                current.authorization.clone(),
                current.credential_revision,
            )
        };
        self.expire_if_due();
        let still_current = self.state.read().current.as_ref().is_some_and(|current| {
            current.available
                && current.credential_revision == snapshot.revision()
                && current.credential_expires_at > Utc::now()
        });
        if still_current {
            Ok(snapshot)
        } else {
            Err(AuthoritySupervisorError::RefreshUnavailable)
        }
    }

    fn public_authorization_snapshot(
        &self,
    ) -> Result<AukiPeerAuthorizationSnapshot, AukiPeerAuthorizationError> {
        self.expire_if_due();
        let snapshot = {
            let state = self.state.read();
            if state.stopped {
                return Err(AukiPeerAuthorizationError::Stopped);
            }
            let Some(current) = state.current.as_ref() else {
                return Err(AukiPeerAuthorizationError::Unavailable);
            };
            if !current.available {
                return Err(if current.credential_expires_at <= Utc::now() {
                    AukiPeerAuthorizationError::Expired
                } else {
                    AukiPeerAuthorizationError::Unavailable
                });
            }
            AukiPeerAuthorizationSnapshot::new(
                current.credential_revision,
                current.credential_expires_at,
                current.claims.clone(),
            )
        };
        self.expire_if_due();
        let state = self.state.read();
        validate_public_snapshot_fence(&state, snapshot.credential_revision(), Utc::now())?;
        Ok(snapshot)
    }

    fn refresh_deadline(
        &self,
        rejected_revision: u64,
    ) -> Result<Option<Instant>, AuthoritySupervisorError> {
        self.expire_if_due();
        let state = self.state.read();
        if state.stopped {
            return Err(AuthoritySupervisorError::Stopped);
        }
        let current = state
            .current
            .as_ref()
            .filter(|current| current.available)
            .ok_or(AuthoritySupervisorError::RefreshUnavailable)?;
        match rejected_authority_revision(current.credential_revision, rejected_revision) {
            RejectedAuthorityRevision::AlreadyReplaced => return Ok(None),
            RejectedAuthorityRevision::Current => {}
            RejectedAuthorityRevision::Stale => {
                return Err(AuthoritySupervisorError::RejectedRevisionMismatch);
            }
        }
        let remaining = wall_remaining(current.credential_expires_at)
            .ok_or(AuthoritySupervisorError::RefreshTimedOut)?;
        Ok(Some(
            Instant::now() + remaining.min(self.config.early_refresh_timeout),
        ))
    }

    async fn refresh_after_unauthorized(
        &self,
        rejected_revision: u64,
    ) -> Result<(), AuthoritySupervisorError> {
        let Some(original_deadline) = self.refresh_deadline(rejected_revision)? else {
            return Ok(());
        };
        let refresh = tokio::time::timeout_at(original_deadline, self.refresh_lock.lock())
            .await
            .map_err(|_| AuthoritySupervisorError::RefreshTimedOut)?;
        let _refresh = refresh;
        let Some(recomputed_deadline) = self.refresh_deadline(rejected_revision)? else {
            return Ok(());
        };
        let deadline = original_deadline.min(recomputed_deadline);
        let update = tokio::time::timeout_at(deadline, self.update_lock.lock())
            .await
            .map_err(|_| AuthoritySupervisorError::RefreshTimedOut)?;
        let Some(final_deadline) = self.refresh_deadline(rejected_revision)? else {
            return Ok(());
        };
        let deadline = deadline.min(final_deadline);

        match &self.mode {
            AuthorityMode::Pull(trigger) => trigger
                .send(Some(rejected_revision))
                .map_err(|_| AuthoritySupervisorError::RefreshUnavailable)?,
            AuthorityMode::External(requests) => {
                let request_id = self
                    .next_refresh_request
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                        current.checked_add(1)
                    })
                    .map_err(|_| AuthoritySupervisorError::RevisionExhausted)?;
                requests
                    .send(Some(ExternalAuthorityRefreshRequest {
                        request_id,
                        rejected_revision,
                    }))
                    .map_err(|_| AuthoritySupervisorError::RefreshUnavailable)?;
            }
        }
        drop(update);
        self.wait_for_revision(rejected_revision, deadline).await
    }

    async fn wait_for_revision(
        &self,
        rejected_revision: u64,
        deadline: Instant,
    ) -> Result<(), AuthoritySupervisorError> {
        let mut status = self.status.subscribe();
        loop {
            self.expire_if_due();
            match status.borrow_and_update().clone() {
                AuthorityStatus::Ready {
                    credential_revision,
                    ..
                } if credential_revision > rejected_revision => return Ok(()),
                AuthorityStatus::Expired { .. } => {
                    return Err(AuthoritySupervisorError::RefreshTimedOut);
                }
                AuthorityStatus::Stopped => return Err(AuthoritySupervisorError::Stopped),
                AuthorityStatus::Starting | AuthorityStatus::Ready { .. } => {}
            }
            tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => return Err(AuthoritySupervisorError::Stopped),
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(AuthoritySupervisorError::RefreshTimedOut);
                }
                changed = status.changed() => {
                    if changed.is_err() {
                        return Err(AuthoritySupervisorError::Stopped);
                    }
                }
            }
        }
    }

    fn stop(&self) {
        let changed = {
            let mut state = self.state.write();
            if state.stopped {
                false
            } else {
                state.stopped = true;
                if let Some(current) = state.current.as_mut() {
                    current.available = false;
                }
                true
            }
        };
        if changed {
            self.status.send_replace(AuthorityStatus::Stopped);
        }
        self.shutdown.cancel();
    }

    fn pull_schedule(&self) -> Option<PullSchedule> {
        self.expire_if_due();
        let state = self.state.read();
        if state.stopped {
            return None;
        }
        state.current.as_ref().and_then(|current| {
            current.renew_at.map(|renew_at| PullSchedule {
                credential_revision: current.credential_revision,
                renew_at,
                expires_at: current.credential_expires_at,
                available: current.available,
            })
        })
    }

    async fn run_expiry_driver(self: Arc<Self>) {
        let mut status = self.status.subscribe();
        loop {
            if self.shutdown.is_cancelled() {
                self.stop();
                return;
            }
            self.expire_if_due();
            let armed = {
                let state = self.state.read();
                state.current.as_ref().and_then(|current| {
                    current
                        .available
                        .then_some((current.credential_revision, current.credential_expires_at))
                })
            };
            let Some((revision, expires_at)) = armed else {
                tokio::select! {
                    biased;
                    _ = self.shutdown.cancelled() => {
                        self.stop();
                        return;
                    },
                    changed = status.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
                continue;
            };
            let delay = wall_remaining(expires_at).unwrap_or(Duration::ZERO);
            tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => {
                    self.stop();
                    return;
                },
                changed = status.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                _ = tokio::time::sleep(delay) => {
                    let still_armed = self.state.read().current.as_ref().is_some_and(|current| {
                        current.available
                            && current.credential_revision == revision
                            && current.credential_expires_at == expires_at
                    });
                    if still_armed {
                        self.expire_if_due();
                    }
                }
            }
        }
    }

    async fn run_pull_driver(
        self: Arc<Self>,
        renewal: AuthorityRenewal,
        mut triggers: watch::Receiver<Option<u64>>,
    ) {
        let mut status = self.status.subscribe();
        let mut last_revision = 0;
        let mut retry_at = None;
        let mut retry_delay = self.config.retry_initial;

        loop {
            if self.shutdown.is_cancelled() {
                return;
            }
            let Some(schedule) = self.pull_schedule() else {
                return;
            };
            if schedule.credential_revision != last_revision {
                let accepted_replacement = last_revision != 0;
                last_revision = schedule.credential_revision;
                retry_delay = self.config.retry_initial;
                retry_at = if accepted_replacement
                    && schedule.available
                    && wall_remaining(schedule.renew_at).is_none()
                {
                    Some(Instant::now() + self.config.retry_initial)
                } else {
                    None
                };
            }
            let due_at = retry_at.unwrap_or_else(|| {
                if schedule.available {
                    Instant::now() + wall_remaining(schedule.renew_at).unwrap_or(Duration::ZERO)
                } else {
                    Instant::now() + self.config.retry_max
                }
            });
            let requested_revision = tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => return,
                changed = triggers.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    triggers.borrow_and_update().to_owned()
                }
                changed = status.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    continue;
                }
                _ = tokio::time::sleep_until(due_at) => None,
            };
            if requested_revision.is_some_and(|requested| requested < schedule.credential_revision)
            {
                continue;
            }

            let attempt = self
                .renew_once(&renewal, schedule.credential_revision, schedule.expires_at)
                .await;
            match attempt {
                Ok(outcome) if outcome.advanced() => {
                    retry_at = None;
                    retry_delay = self.config.retry_initial;
                }
                Ok(_) => {
                    warn!("authority pull renewal returned an unchanged credential");
                    schedule_retry(
                        schedule.expires_at,
                        &mut retry_at,
                        &mut retry_delay,
                        self.config.retry_max,
                    );
                }
                Err(AuthoritySupervisorError::Stopped) => return,
                Err(error) => {
                    warn!(error = %error, "authority pull renewal failed; retrying");
                    schedule_retry(
                        schedule.expires_at,
                        &mut retry_at,
                        &mut retry_delay,
                        self.config.retry_max,
                    );
                }
            }
        }
    }

    async fn renew_once(
        &self,
        renewal: &AuthorityRenewal,
        observed_revision: u64,
        expires_at: DateTime<Utc>,
    ) -> Result<AuthorityInstallOutcome, AuthoritySupervisorError> {
        if self
            .state
            .read()
            .current
            .as_ref()
            .is_some_and(|current| current.credential_revision > observed_revision)
        {
            return Ok(AuthorityInstallOutcome::Unchanged(observed_revision));
        }
        let attempt_budget = wall_remaining(expires_at)
            .map(|remaining| remaining.min(self.config.renewal_attempt_timeout))
            .unwrap_or(self.config.renewal_attempt_timeout);
        let deadline = Instant::now() + attempt_budget;
        let attempt_cancellation = self.shutdown.child_token();
        let attempt = renewal.renew_with_cancellation(&attempt_cancellation);
        let renewed = tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => {
                attempt_cancellation.cancel();
                return Err(AuthoritySupervisorError::Stopped);
            }
            _ = tokio::time::sleep_until(deadline) => {
                attempt_cancellation.cancel();
                return Err(AuthoritySupervisorError::RefreshTimedOut);
            }
            renewed = attempt => renewed.map_err(AuthoritySupervisorError::PullRenewal)?,
        };
        self.require_running()?;
        self.apply_update(renewed.into()).await
    }
}

fn validate_public_snapshot_fence(
    state: &AuthorityState,
    snapshot_revision: u64,
    now: DateTime<Utc>,
) -> Result<(), AukiPeerAuthorizationError> {
    if state.stopped {
        return Err(AukiPeerAuthorizationError::Stopped);
    }
    let Some(current) = state.current.as_ref() else {
        return Err(AukiPeerAuthorizationError::Unavailable);
    };
    if current.credential_revision != snapshot_revision {
        return Err(AukiPeerAuthorizationError::Unavailable);
    }
    if current.credential_expires_at <= now {
        return Err(AukiPeerAuthorizationError::Expired);
    }
    if !current.available {
        return Err(AukiPeerAuthorizationError::Unavailable);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct PullSchedule {
    credential_revision: u64,
    renew_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    available: bool,
}

fn schedule_retry(
    expires_at: DateTime<Utc>,
    retry_at: &mut Option<Instant>,
    retry_delay: &mut Duration,
    retry_max: Duration,
) {
    let delay = wall_remaining(expires_at)
        .map(|remaining| (*retry_delay).min(remaining))
        .unwrap_or(retry_max);
    *retry_at = Some(Instant::now() + delay);
    *retry_delay = retry_delay.saturating_mul(2).min(retry_max);
}

fn exact_expiration(expires_at: DateTime<Utc>) -> Result<u64, AuthoritySupervisorError> {
    if expires_at <= Utc::now() || expires_at.timestamp_subsec_nanos() != 0 {
        return Err(AuthoritySupervisorError::InvalidExpiration);
    }
    u64::try_from(expires_at.timestamp()).map_err(|_| AuthoritySupervisorError::InvalidExpiration)
}

fn wall_remaining(deadline: DateTime<Utc>) -> Option<Duration> {
    deadline
        .signed_duration_since(Utc::now())
        .to_std()
        .ok()
        .filter(|remaining| !remaining.is_zero())
}

/// Request for the external authority owner to replace a rejected credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalAuthorityRefreshRequest {
    request_id: u64,
    rejected_revision: u64,
}

impl ExternalAuthorityRefreshRequest {
    /// Process-local identifier for this coalesced refresh request.
    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Credential revision rejected by the relay service.
    pub fn rejected_credential_revision(&self) -> u64 {
        self.rejected_revision
    }
}

pub(crate) struct ExternalRefreshRequests {
    requests: watch::Receiver<Option<ExternalAuthorityRefreshRequest>>,
    shutdown: CancellationToken,
}

impl ExternalRefreshRequests {
    pub(crate) async fn recv(&mut self) -> Option<ExternalAuthorityRefreshRequest> {
        loop {
            tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => return None,
                changed = self.requests.changed() => {
                    if changed.is_err() {
                        return None;
                    }
                    if let Some(request) = *self.requests.borrow_and_update() {
                        return Some(request);
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct ExternalAuthorityHandle {
    inner: Weak<AuthorityInner>,
}

impl ExternalAuthorityHandle {
    pub(crate) async fn replace(
        &self,
        update: ExternalAuthorityUpdate,
    ) -> Result<AuthorityInstallOutcome, AuthoritySupervisorError> {
        let inner = self
            .inner
            .upgrade()
            .ok_or(AuthoritySupervisorError::Stopped)?;
        inner.apply_update(update.into()).await
    }
}

struct AuthorityRelayAuthorization {
    inner: Weak<AuthorityInner>,
}

#[async_trait]
impl RelayAuthorizationProvider for AuthorityRelayAuthorization {
    async fn authorization(&self) -> Result<RelayAuthorizationSnapshot, RelayAuthorizationError> {
        self.inner
            .upgrade()
            .ok_or(RelayAuthorizationError)?
            .authorization_snapshot()
            .map_err(|_| RelayAuthorizationError)
    }

    async fn refresh_after_unauthorized(
        &self,
        rejected_revision: u64,
    ) -> Result<(), RelayAuthorizationError> {
        self.inner
            .upgrade()
            .ok_or(RelayAuthorizationError)?
            .refresh_after_unauthorized(rejected_revision)
            .await
            .map_err(|_| RelayAuthorizationError)
    }
}

struct AuthorityPublicAuthorization {
    inner: Weak<AuthorityInner>,
}

impl AuthorizationSnapshotSource for AuthorityPublicAuthorization {
    fn current(&self) -> Result<AukiPeerAuthorizationSnapshot, AukiPeerAuthorizationError> {
        self.inner
            .upgrade()
            .ok_or(AukiPeerAuthorizationError::Stopped)?
            .public_authorization_snapshot()
    }
}

pub(crate) struct AuthoritySupervisor {
    inner: Arc<AuthorityInner>,
    tasks: Vec<JoinHandle<()>>,
}

impl AuthoritySupervisor {
    pub(crate) async fn start_pull(
        authority: FixedDomainAuthority,
        prepared: PreparedPeer,
        config: AuthoritySupervisorConfig,
        shutdown: &CancellationToken,
    ) -> Result<Self, AuthoritySupervisorError> {
        Self::start_pull_with_installer(Arc::new(authority), prepared, config, shutdown).await
    }

    async fn start_pull_with_installer(
        installer: Arc<dyn AuthorityInstaller>,
        prepared: PreparedPeer,
        config: AuthoritySupervisorConfig,
        shutdown: &CancellationToken,
    ) -> Result<Self, AuthoritySupervisorError> {
        config.validate()?;
        let PreparedPeer {
            domain,
            peer_id,
            initial_credential,
            verification_keys,
            credential_expires_at,
            renew_at,
            renewal,
        } = prepared;
        let (trigger, trigger_rx) = watch::channel(None);
        let inner = AuthorityInner::new(
            installer,
            config,
            AuthorityMode::Pull(trigger),
            shutdown.child_token(),
        );
        if let Err(error) = inner
            .apply_update(AuthorityUpdate {
                domain_id: domain.id,
                peer_id,
                verification_keys,
                credential: initial_credential,
                credential_expires_at,
                renew_at: Some(renew_at),
            })
            .await
        {
            inner.stop();
            return Err(error);
        }
        let expiry_inner = Arc::clone(&inner);
        let pull_inner = Arc::clone(&inner);
        let tasks = vec![
            tokio::spawn(async move { expiry_inner.run_expiry_driver().await }),
            tokio::spawn(async move {
                pull_inner.run_pull_driver(renewal, trigger_rx).await;
            }),
        ];
        Ok(Self { inner, tasks })
    }

    pub(crate) async fn start_external(
        authority: FixedDomainAuthority,
        initial: ExternalAuthorityUpdate,
        config: AuthoritySupervisorConfig,
        shutdown: &CancellationToken,
    ) -> Result<(Self, ExternalAuthorityHandle, ExternalRefreshRequests), AuthoritySupervisorError>
    {
        Self::start_external_with_installer(Arc::new(authority), initial, config, shutdown).await
    }

    async fn start_external_with_installer(
        installer: Arc<dyn AuthorityInstaller>,
        initial: ExternalAuthorityUpdate,
        config: AuthoritySupervisorConfig,
        shutdown: &CancellationToken,
    ) -> Result<(Self, ExternalAuthorityHandle, ExternalRefreshRequests), AuthoritySupervisorError>
    {
        config.validate()?;
        let (requests, request_rx) = watch::channel(None);
        let inner = AuthorityInner::new(
            installer,
            config,
            AuthorityMode::External(requests),
            shutdown.child_token(),
        );
        if let Err(error) = inner.apply_update(initial.into()).await {
            inner.stop();
            return Err(error);
        }
        let expiry_inner = Arc::clone(&inner);
        let task = tokio::spawn(async move { expiry_inner.run_expiry_driver().await });
        let handle = ExternalAuthorityHandle {
            inner: Arc::downgrade(&inner),
        };
        let requests = ExternalRefreshRequests {
            requests: request_rx,
            shutdown: inner.shutdown.clone(),
        };
        Ok((
            Self {
                inner,
                tasks: vec![task],
            },
            handle,
            requests,
        ))
    }

    pub(crate) fn relay_authorization(&self) -> Arc<dyn RelayAuthorizationProvider> {
        Arc::new(AuthorityRelayAuthorization {
            inner: Arc::downgrade(&self.inner),
        })
    }

    pub(crate) fn public_authorization(&self) -> Arc<dyn AuthorizationSnapshotSource> {
        Arc::new(AuthorityPublicAuthorization {
            inner: Arc::downgrade(&self.inner),
        })
    }

    pub(crate) fn subscribe_status(&self) -> watch::Receiver<AuthorityStatus> {
        self.inner.expire_if_due();
        self.inner.status.subscribe()
    }

    pub(crate) async fn shutdown(mut self) {
        self.inner.stop();
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
        let updates = self.inner.update_lock.lock().await;
        drop(updates);
    }
}

impl Drop for AuthoritySupervisor {
    fn drop(&mut self) {
        self.inner.stop();
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use auki_auth::{AuthorityRenewalProvider, DomainDescriptor, PreparedPeer, RenewedAuthority};
    use auki_p2p::{
        Identity, Node, P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_SCOPE, P2P_TOKEN_TTL,
        P2P_TOKEN_TYPE, P2PAccessClaims,
    };
    use chrono::TimeZone;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use parking_lot::Mutex as ParkingMutex;
    use tokio::sync::{Notify, Semaphore};

    use super::*;

    const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

    const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

    const ROTATED_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgwRbuxaM6rEI3vYEl
vRmIEsc1QtC3uPMWvXo1xXt+CcOhRANCAAQDFwBFAujMsiq78IWbq5vz0QSWEdc7
7h5NE8sDwgD6Js22t9Ztq84hhkS3Aad4m9FOi8evk5QYW7ef+Bc2oZsr
-----END PRIVATE KEY-----"#;

    const ROTATED_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEAxcARQLozLIqu/CFm6ub89EElhHX
O+4eTRPLA8IA+ibNtrfWbavOIYZEtwGneJvRTovHr5OUGFu3n/gXNqGbKw==
-----END PUBLIC KEY-----"#;

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum InstallEvent {
        VerificationKeys(u64),
        Credential,
    }

    struct FakeInstaller {
        domain_id: Uuid,
        peer_id: PeerId,
        events: ParkingMutex<Vec<InstallEvent>>,
        fail_next_keys: AtomicBool,
        fail_next_credential: AtomicBool,
        block_next_credential: AtomicBool,
        credential_started: Notify,
        credential_release: Semaphore,
        completed_credentials: AtomicUsize,
    }

    impl FakeInstaller {
        fn new(domain_id: Uuid, peer_id: PeerId) -> Self {
            Self {
                domain_id,
                peer_id,
                events: ParkingMutex::new(Vec::new()),
                fail_next_keys: AtomicBool::new(false),
                fail_next_credential: AtomicBool::new(false),
                block_next_credential: AtomicBool::new(false),
                credential_started: Notify::new(),
                credential_release: Semaphore::new(0),
                completed_credentials: AtomicUsize::new(0),
            }
        }

        fn events(&self) -> Vec<InstallEvent> {
            self.events.lock().clone()
        }

        fn clear_events(&self) {
            self.events.lock().clear();
        }

        fn block_next_credential(&self) {
            self.block_next_credential.store(true, Ordering::SeqCst);
        }

        async fn wait_for_blocked_credential(&self) {
            self.credential_started.notified().await;
        }

        fn release_blocked_credential(&self) {
            self.credential_release.add_permits(1);
        }

        fn completed_credentials(&self) -> usize {
            self.completed_credentials.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl AuthorityInstaller for FakeInstaller {
        fn domain_id(&self) -> Uuid {
            self.domain_id
        }

        fn peer_id(&self) -> PeerId {
            self.peer_id
        }

        async fn install_verification_keys(
            &self,
            keys: DdsVerificationKeys,
        ) -> Result<(), AuthorityInstallerError> {
            self.events
                .lock()
                .push(InstallEvent::VerificationKeys(keys.generation()));
            if self.fail_next_keys.swap(false, Ordering::SeqCst) {
                Err(AuthorityInstallerError::Injected("verification keys"))
            } else {
                Ok(())
            }
        }

        async fn install_credential(
            &self,
            _credential: SignedP2pCredential,
        ) -> Result<(), AuthorityInstallerError> {
            self.events.lock().push(InstallEvent::Credential);
            if self.fail_next_credential.swap(false, Ordering::SeqCst) {
                Err(AuthorityInstallerError::Injected("credential"))
            } else {
                if self.block_next_credential.swap(false, Ordering::SeqCst) {
                    self.credential_started.notify_one();
                    self.credential_release
                        .acquire()
                        .await
                        .expect("test credential release semaphore must remain open")
                        .forget();
                }
                self.completed_credentials.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
    }

    enum RenewalStep {
        Return {
            after: Duration,
            update: Box<RenewedAuthority>,
        },
        Fail {
            after: Duration,
        },
    }

    #[derive(Clone)]
    struct ScriptedRenewal {
        steps: Arc<ParkingMutex<VecDeque<RenewalStep>>>,
        calls: Arc<AtomicUsize>,
    }

    impl ScriptedRenewal {
        fn new(steps: impl IntoIterator<Item = RenewalStep>) -> Self {
            Self {
                steps: Arc::new(ParkingMutex::new(steps.into_iter().collect())),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl AuthorityRenewalProvider for ScriptedRenewal {
        async fn renew_authority(
            &self,
            cancellation: &CancellationToken,
        ) -> auki_auth::Result<RenewedAuthority> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let step = self.steps.lock().pop_front().unwrap_or(RenewalStep::Fail {
                after: Duration::ZERO,
            });
            let after = match &step {
                RenewalStep::Return { after, .. } | RenewalStep::Fail { after } => *after,
            };
            if !after.is_zero() {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        return Err(auki_auth::Error::Cancelled { endpoint: "test-renewal" });
                    }
                    _ = tokio::time::sleep(after) => {}
                }
            }
            match step {
                RenewalStep::Return { update, .. } => Ok(*update),
                RenewalStep::Fail { .. } => Err(auki_auth::Error::Transport {
                    endpoint: "test-renewal",
                }),
            }
        }
    }

    fn unix_time() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must follow the Unix epoch")
            .as_secs()
    }

    fn identity() -> Identity {
        Identity::from_ed25519_seed(&[0x51; 32])
    }

    fn keys() -> DdsVerificationKeys {
        DdsVerificationKeys::new(0, TEST_DDS_PUBLIC_KEY.to_vec(), None)
    }

    fn rotated_keys() -> DdsVerificationKeys {
        DdsVerificationKeys::new(
            1,
            ROTATED_DDS_PUBLIC_KEY.to_vec(),
            Some(TEST_DDS_PUBLIC_KEY.to_vec()),
        )
    }

    fn invalid_rotated_keys() -> DdsVerificationKeys {
        DdsVerificationKeys::new(1, ROTATED_DDS_PUBLIC_KEY.to_vec(), None)
    }

    fn signed_credential(
        peer_id: PeerId,
        domain_id: Uuid,
        issued_at: u64,
        subject: Uuid,
        signing_key: &[u8],
    ) -> (SignedP2pCredential, DateTime<Utc>) {
        let expiration = issued_at + P2P_TOKEN_TTL.as_secs();
        let claims = P2PAccessClaims {
            token_type: P2P_TOKEN_TYPE.into(),
            iss: P2P_TOKEN_ISSUER.into(),
            aud: vec![P2P_TOKEN_AUDIENCE.into()],
            sub: subject.to_string(),
            organization_id: None,
            peer_type: Some("compute".into()),
            peer_id: peer_id.to_string(),
            domain_ids: vec![domain_id.to_string()],
            scopes: vec![P2P_TOKEN_SCOPE.into()],
            application: None,
            iat: issued_at,
            nbf: None,
            exp: expiration,
        };
        let compact = encode(
            &Header::new(Algorithm::ES256),
            &claims,
            &EncodingKey::from_ec_pem(signing_key).unwrap(),
        )
        .unwrap();
        (
            SignedP2pCredential::new(compact).unwrap(),
            Utc.timestamp_opt(expiration as i64, 0).single().unwrap(),
        )
    }

    fn credential(
        peer_id: PeerId,
        domain_id: Uuid,
        issued_at: u64,
    ) -> (SignedP2pCredential, DateTime<Utc>) {
        signed_credential(
            peer_id,
            domain_id,
            issued_at,
            Uuid::new_v4(),
            TEST_DDS_PRIVATE_KEY,
        )
    }

    fn external_update(
        domain_id: Uuid,
        peer_id: PeerId,
        verification_keys: DdsVerificationKeys,
        credential: SignedP2pCredential,
        credential_expires_at: DateTime<Utc>,
    ) -> ExternalAuthorityUpdate {
        ExternalAuthorityUpdate::new(
            domain_id,
            peer_id,
            verification_keys,
            credential,
            credential_expires_at,
        )
    }

    fn renewed_update(
        domain_id: Uuid,
        peer_id: PeerId,
        verification_keys: DdsVerificationKeys,
        credential: SignedP2pCredential,
        credential_expires_at: DateTime<Utc>,
    ) -> RenewedAuthority {
        RenewedAuthority {
            domain: DomainDescriptor {
                id: domain_id,
                name: Some("metadata may change".into()),
                description: Some("not an authority binding".into()),
                organization_id: Some(Uuid::new_v4()),
            },
            peer_id,
            verification_keys,
            credential,
            credential_expires_at,
            renew_at: credential_expires_at - chrono::Duration::minutes(1),
        }
    }

    fn config() -> AuthoritySupervisorConfig {
        AuthoritySupervisorConfig {
            renewal_attempt_timeout: Duration::from_millis(500),
            early_refresh_timeout: Duration::from_millis(500),
            retry_initial: Duration::from_millis(20),
            retry_max: Duration::from_millis(50),
        }
    }

    fn current_authority_for_public_fence(
        credential_revision: u64,
        credential_expires_at: DateTime<Utc>,
        available: bool,
    ) -> CurrentAuthority {
        let mut authorization = HeaderValue::from_static("Bearer public-fence-test");
        authorization.set_sensitive(true);
        CurrentAuthority {
            credential_revision,
            claims: P2PAccessClaims {
                token_type: P2P_TOKEN_TYPE.into(),
                iss: P2P_TOKEN_ISSUER.into(),
                aud: vec![P2P_TOKEN_AUDIENCE.into()],
                sub: Uuid::new_v4().to_string(),
                organization_id: None,
                peer_type: Some("robot".into()),
                peer_id: identity().peer_id().to_string(),
                domain_ids: vec![Uuid::new_v4().to_string()],
                scopes: vec![P2P_TOKEN_SCOPE.into()],
                application: None,
                iat: unix_time(),
                nbf: None,
                exp: u64::MAX,
            },
            authorization,
            credential_expires_at,
            renew_at: None,
            available,
        }
    }

    #[test]
    fn public_snapshot_second_fence_classifies_stop_replacement_and_expiry_exactly() {
        let now = Utc::now();
        let future = now + chrono::Duration::minutes(5);
        let stopped = AuthorityState {
            stopped: true,
            installed_keys: None,
            current: Some(current_authority_for_public_fence(1, future, true)),
        };
        assert_eq!(
            validate_public_snapshot_fence(&stopped, 1, now),
            Err(AukiPeerAuthorizationError::Stopped)
        );

        let replaced = AuthorityState {
            stopped: false,
            installed_keys: None,
            current: Some(current_authority_for_public_fence(2, future, true)),
        };
        assert_eq!(
            validate_public_snapshot_fence(&replaced, 1, now),
            Err(AukiPeerAuthorizationError::Unavailable)
        );

        let expired = AuthorityState {
            stopped: false,
            installed_keys: None,
            current: Some(current_authority_for_public_fence(
                1,
                now - chrono::Duration::seconds(1),
                false,
            )),
        };
        assert_eq!(
            validate_public_snapshot_fence(&expired, 1, now),
            Err(AukiPeerAuthorizationError::Expired)
        );
    }

    fn prepared(
        domain_id: Uuid,
        peer_id: PeerId,
        credential: SignedP2pCredential,
        credential_expires_at: DateTime<Utc>,
        renew_at: DateTime<Utc>,
        renewal: ScriptedRenewal,
    ) -> PreparedPeer {
        PreparedPeer {
            domain: DomainDescriptor::assigned(domain_id),
            peer_id,
            initial_credential: credential,
            verification_keys: keys(),
            credential_expires_at,
            renew_at,
            renewal: AuthorityRenewal::new(renewal),
        }
    }

    async fn wait_for_ready_revision(supervisor: &AuthoritySupervisor, expected: u64) {
        let mut status = supervisor.subscribe_status();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if matches!(
                    status.borrow_and_update().clone(),
                    AuthorityStatus::Ready {
                        credential_revision,
                        ..
                    } if credential_revision == expected
                ) {
                    return;
                }
                status.changed().await.unwrap();
            }
        })
        .await
        .expect("authority revision must become ready");
    }

    #[tokio::test]
    async fn external_replacement_is_ordered_and_equivalent_credentials_only_refresh_keys() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let (initial_credential, initial_expiration) = credential(peer_id, domain_id, issued_at);
        let initial = external_update(
            domain_id,
            peer_id,
            keys(),
            initial_credential,
            initial_expiration,
        );
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let (supervisor, handle, _requests) = AuthoritySupervisor::start_external_with_installer(
            installer.clone(),
            initial,
            config(),
            &shutdown,
        )
        .await
        .unwrap();
        assert_eq!(
            installer.events(),
            vec![InstallEvent::VerificationKeys(0), InstallEvent::Credential]
        );

        installer.clear_events();
        let replacement_subject = Uuid::new_v4();
        let (replacement_credential, replacement_expiration) = signed_credential(
            peer_id,
            domain_id,
            issued_at + 1,
            replacement_subject,
            ROTATED_DDS_PRIVATE_KEY,
        );
        let (equivalent_credential, equivalent_expiration) = signed_credential(
            peer_id,
            domain_id,
            issued_at + 1,
            replacement_subject,
            ROTATED_DDS_PRIVATE_KEY,
        );
        assert_ne!(replacement_credential, equivalent_credential);
        assert_eq!(replacement_expiration, equivalent_expiration);
        let expected_header = replacement_credential.to_sensitive_bearer_header().unwrap();
        let replacement = external_update(
            domain_id,
            peer_id,
            rotated_keys(),
            replacement_credential,
            replacement_expiration,
        );
        assert_eq!(
            handle.replace(replacement).await.unwrap(),
            AuthorityInstallOutcome::Replaced(2)
        );
        assert_eq!(
            installer.events(),
            vec![InstallEvent::VerificationKeys(1), InstallEvent::Credential]
        );
        let snapshot = supervisor
            .relay_authorization()
            .authorization()
            .await
            .unwrap();
        assert_eq!(
            snapshot,
            RelayAuthorizationSnapshot::new(expected_header.clone(), 2)
        );
        assert!(snapshot.is_sensitive());

        installer.clear_events();
        assert_eq!(
            handle
                .replace(external_update(
                    domain_id,
                    peer_id,
                    rotated_keys(),
                    equivalent_credential,
                    equivalent_expiration,
                ))
                .await
                .unwrap(),
            AuthorityInstallOutcome::Unchanged(2)
        );
        assert_eq!(installer.events(), vec![InstallEvent::VerificationKeys(1)]);
        let snapshot = supervisor
            .relay_authorization()
            .authorization()
            .await
            .unwrap();
        assert_eq!(
            snapshot,
            RelayAuthorizationSnapshot::new(expected_header, 2)
        );
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn failed_credential_install_never_publishes_the_proposed_header() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let (initial_credential, initial_expiration) = credential(peer_id, domain_id, issued_at);
        let expected_initial_header = initial_credential.to_sensitive_bearer_header().unwrap();
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let (supervisor, handle, _) = AuthoritySupervisor::start_external_with_installer(
            installer.clone(),
            external_update(
                domain_id,
                peer_id,
                keys(),
                initial_credential,
                initial_expiration,
            ),
            config(),
            &shutdown,
        )
        .await
        .unwrap();
        installer.clear_events();
        installer.fail_next_credential.store(true, Ordering::SeqCst);
        let (replacement, expiration) = credential(peer_id, domain_id, issued_at + 1);

        let error = handle
            .replace(external_update(
                domain_id,
                peer_id,
                keys(),
                replacement,
                expiration,
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, AuthoritySupervisorError::Install(_)));
        assert_eq!(
            installer.events(),
            vec![InstallEvent::VerificationKeys(0), InstallEvent::Credential]
        );
        let snapshot = supervisor
            .relay_authorization()
            .authorization()
            .await
            .unwrap();
        assert_eq!(
            snapshot,
            RelayAuthorizationSnapshot::new(expected_initial_header, 1)
        );
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn replacements_are_pinned_to_metadata_signature_expiry_order_and_key_lineage() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let (initial_credential, initial_expiration) = credential(peer_id, domain_id, issued_at);
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let (supervisor, handle, _) = AuthoritySupervisor::start_external_with_installer(
            installer.clone(),
            external_update(
                domain_id,
                peer_id,
                keys(),
                initial_credential,
                initial_expiration,
            ),
            config(),
            &shutdown,
        )
        .await
        .unwrap();
        installer.clear_events();

        let (candidate, expiration) = credential(peer_id, domain_id, issued_at + 1);
        let error = handle
            .replace(external_update(
                Uuid::new_v4(),
                peer_id,
                keys(),
                candidate,
                expiration,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AuthoritySupervisorError::DomainMismatch { .. }
        ));
        assert!(installer.events().is_empty());

        let other_peer = Identity::from_ed25519_seed(&[0x52; 32]).peer_id();
        let (candidate, expiration) = credential(other_peer, domain_id, issued_at + 1);
        let error = handle
            .replace(external_update(
                domain_id,
                peer_id,
                keys(),
                candidate,
                expiration,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AuthoritySupervisorError::SignedPeerMismatch
        ));
        assert!(installer.events().is_empty());

        let (candidate, expiration) = credential(peer_id, domain_id, issued_at + 1);
        let error = handle
            .replace(external_update(
                domain_id,
                peer_id,
                keys(),
                candidate,
                expiration + chrono::Duration::seconds(1),
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AuthoritySupervisorError::SignedExpirationMismatch
        ));
        assert!(installer.events().is_empty());

        let (candidate, expiration) = signed_credential(
            peer_id,
            domain_id,
            issued_at,
            Uuid::new_v4(),
            TEST_DDS_PRIVATE_KEY,
        );
        let error = handle
            .replace(external_update(
                domain_id,
                peer_id,
                keys(),
                candidate,
                expiration,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AuthoritySupervisorError::NonAdvancingCredential { .. }
        ));
        assert!(installer.events().is_empty());

        let (candidate, expiration) = signed_credential(
            peer_id,
            domain_id,
            issued_at + 1,
            Uuid::new_v4(),
            ROTATED_DDS_PRIVATE_KEY,
        );
        let error = handle
            .replace(external_update(
                domain_id,
                peer_id,
                invalid_rotated_keys(),
                candidate,
                expiration,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AuthoritySupervisorError::VerificationKeys(_)
        ));
        assert!(installer.events().is_empty());

        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn pull_start_rejects_a_renewal_time_at_literal_expiry_before_installing() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let (initial_credential, expiration) = credential(peer_id, domain_id, unix_time());
        let renewal = ScriptedRenewal::new([]);
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();

        let error = AuthoritySupervisor::start_pull_with_installer(
            installer.clone(),
            prepared(
                domain_id,
                peer_id,
                initial_credential,
                expiration,
                expiration,
                renewal,
            ),
            config(),
            &shutdown,
        )
        .await
        .err()
        .expect("invalid renewal schedule must fail startup");
        assert!(matches!(
            error,
            AuthoritySupervisorError::InvalidRenewalSchedule
        ));
        assert!(installer.events().is_empty());
    }

    #[tokio::test]
    async fn the_single_pull_loop_retries_and_accepts_changed_descriptor_metadata() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let (initial_credential, initial_expiration) = credential(peer_id, domain_id, issued_at);
        let (replacement, replacement_expiration) = signed_credential(
            peer_id,
            domain_id,
            issued_at + 1,
            Uuid::new_v4(),
            ROTATED_DDS_PRIVATE_KEY,
        );
        let renewal = ScriptedRenewal::new([
            RenewalStep::Fail {
                after: Duration::ZERO,
            },
            RenewalStep::Return {
                after: Duration::ZERO,
                update: Box::new(renewed_update(
                    domain_id,
                    peer_id,
                    rotated_keys(),
                    replacement,
                    replacement_expiration,
                )),
            },
        ]);
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let supervisor = AuthoritySupervisor::start_pull_with_installer(
            installer.clone(),
            prepared(
                domain_id,
                peer_id,
                initial_credential,
                initial_expiration,
                Utc::now() + chrono::Duration::milliseconds(30),
                renewal.clone(),
            ),
            config(),
            &shutdown,
        )
        .await
        .unwrap();
        installer.clear_events();

        wait_for_ready_revision(&supervisor, 2).await;
        assert_eq!(renewal.calls(), 2);
        assert_eq!(
            installer.events(),
            vec![InstallEvent::VerificationKeys(1), InstallEvent::Credential]
        );
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn an_advanced_past_due_pull_replacement_waits_before_renewing_again() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let (initial_credential, initial_expiration) = credential(peer_id, domain_id, issued_at);
        let (replacement, replacement_expiration) = credential(peer_id, domain_id, issued_at + 1);
        let mut past_due = renewed_update(
            domain_id,
            peer_id,
            keys(),
            replacement,
            replacement_expiration,
        );
        past_due.renew_at = Utc::now() - chrono::Duration::seconds(1);
        let renewal = ScriptedRenewal::new([RenewalStep::Return {
            after: Duration::ZERO,
            update: Box::new(past_due),
        }]);
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let mut backoff_config = config();
        backoff_config.retry_initial = Duration::from_millis(300);
        backoff_config.retry_max = Duration::from_millis(300);
        let supervisor = AuthoritySupervisor::start_pull_with_installer(
            installer,
            prepared(
                domain_id,
                peer_id,
                initial_credential,
                initial_expiration,
                Utc::now() - chrono::Duration::seconds(1),
                renewal.clone(),
            ),
            backoff_config,
            &shutdown,
        )
        .await
        .unwrap();

        wait_for_ready_revision(&supervisor, 2).await;
        assert_eq!(renewal.calls(), 1);
        tokio::time::sleep(Duration::from_millis(75)).await;
        assert_eq!(
            renewal.calls(),
            1,
            "a newly accepted past-due revision must not hot-loop"
        );
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn concurrent_pull_unauthorized_refreshes_share_one_renewal() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let (initial_credential, initial_expiration) = credential(peer_id, domain_id, issued_at);
        let (replacement, replacement_expiration) = credential(peer_id, domain_id, issued_at + 1);
        let renewal = ScriptedRenewal::new([RenewalStep::Return {
            after: Duration::from_millis(60),
            update: Box::new(renewed_update(
                domain_id,
                peer_id,
                keys(),
                replacement,
                replacement_expiration,
            )),
        }]);
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let supervisor = AuthoritySupervisor::start_pull_with_installer(
            installer,
            prepared(
                domain_id,
                peer_id,
                initial_credential,
                initial_expiration,
                Utc::now() + chrono::Duration::minutes(10),
                renewal.clone(),
            ),
            config(),
            &shutdown,
        )
        .await
        .unwrap();
        let relay = supervisor.relay_authorization();
        let rejected_revision = relay.authorization().await.unwrap().revision();
        let first = relay.refresh_after_unauthorized(rejected_revision);
        let second = relay.refresh_after_unauthorized(rejected_revision);

        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();
        assert_eq!(renewal.calls(), 1);
        assert_eq!(relay.authorization().await.unwrap().revision(), 2);
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn external_unauthorized_refreshes_coalesce_until_a_real_replacement() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let initial_subject = Uuid::new_v4();
        let (initial_credential, initial_expiration) = signed_credential(
            peer_id,
            domain_id,
            issued_at,
            initial_subject,
            TEST_DDS_PRIVATE_KEY,
        );
        let (equivalent_credential, equivalent_expiration) = signed_credential(
            peer_id,
            domain_id,
            issued_at,
            initial_subject,
            TEST_DDS_PRIVATE_KEY,
        );
        assert_ne!(initial_credential, equivalent_credential);
        assert_eq!(initial_expiration, equivalent_expiration);
        let initial = external_update(
            domain_id,
            peer_id,
            keys(),
            initial_credential,
            initial_expiration,
        );
        let equivalent_initial = external_update(
            domain_id,
            peer_id,
            keys(),
            equivalent_credential,
            equivalent_expiration,
        );
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let (supervisor, handle, mut requests) =
            AuthoritySupervisor::start_external_with_installer(
                installer,
                initial,
                config(),
                &shutdown,
            )
            .await
            .unwrap();
        let relay = supervisor.relay_authorization();
        let first_relay = relay.clone();
        let second_relay = relay.clone();
        let first = tokio::spawn(async move { first_relay.refresh_after_unauthorized(1).await });
        let second = tokio::spawn(async move { second_relay.refresh_after_unauthorized(1).await });

        let request = tokio::time::timeout(Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.rejected_revision, 1);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), requests.recv())
                .await
                .is_err()
        );

        assert_eq!(
            handle.replace(equivalent_initial).await.unwrap(),
            AuthorityInstallOutcome::Unchanged(1)
        );
        assert_eq!(*requests.requests.borrow(), Some(request));

        let (replacement, replacement_expiration) = credential(peer_id, domain_id, issued_at + 1);
        assert_eq!(
            handle
                .replace(external_update(
                    domain_id,
                    peer_id,
                    keys(),
                    replacement,
                    replacement_expiration,
                ))
                .await
                .unwrap(),
            AuthorityInstallOutcome::Replaced(2)
        );
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(*requests.requests.borrow(), None);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), requests.recv())
                .await
                .is_err()
        );
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn waiting_for_the_refresh_lock_does_not_extend_the_original_401_deadline() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let (credential, expiration) = credential(peer_id, domain_id, unix_time());
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let mut deadline_config = config();
        deadline_config.early_refresh_timeout = Duration::from_millis(300);
        let (supervisor, _handle, _requests) = AuthoritySupervisor::start_external_with_installer(
            installer,
            external_update(domain_id, peer_id, keys(), credential, expiration),
            deadline_config,
            &shutdown,
        )
        .await
        .unwrap();
        let refresh_guard = supervisor.inner.refresh_lock.lock().await;
        let inner = Arc::clone(&supervisor.inner);
        let started = Instant::now();
        let refresh = tokio::spawn(async move { inner.refresh_after_unauthorized(1).await });
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(refresh_guard);

        assert!(matches!(
            refresh.await.unwrap(),
            Err(AuthoritySupervisorError::RefreshTimedOut)
        ));
        assert!(
            started.elapsed() < Duration::from_millis(425),
            "waiting for the single-flight lock extended the original deadline"
        );
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn explicit_shutdown_waits_for_an_in_flight_external_install_and_fences_new_work() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let (initial_credential, initial_expiration) = credential(peer_id, domain_id, issued_at);
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let (supervisor, handle, _requests) = AuthoritySupervisor::start_external_with_installer(
            installer.clone(),
            external_update(
                domain_id,
                peer_id,
                keys(),
                initial_credential,
                initial_expiration,
            ),
            config(),
            &shutdown,
        )
        .await
        .unwrap();
        assert_eq!(installer.completed_credentials(), 1);
        installer.block_next_credential();
        let (replacement, replacement_expiration) = credential(peer_id, domain_id, issued_at + 1);
        let active_handle = handle.clone();
        let active_replace = tokio::spawn(async move {
            active_handle
                .replace(external_update(
                    domain_id,
                    peer_id,
                    keys(),
                    replacement,
                    replacement_expiration,
                ))
                .await
        });
        installer.wait_for_blocked_credential().await;
        assert_eq!(installer.completed_credentials(), 1);

        let inner = Arc::clone(&supervisor.inner);
        let shutdown_task = tokio::spawn(async move { supervisor.shutdown().await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !inner.state.read().stopped {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown must synchronously fence authority work");
        assert!(!shutdown_task.is_finished());

        let (late_credential, late_expiration) = credential(peer_id, domain_id, issued_at + 2);
        let late_handle = handle.clone();
        let late_replace = tokio::spawn(async move {
            late_handle
                .replace(external_update(
                    domain_id,
                    peer_id,
                    keys(),
                    late_credential,
                    late_expiration,
                ))
                .await
        });
        tokio::task::yield_now().await;
        assert!(!late_replace.is_finished());

        installer.release_blocked_credential();
        assert!(matches!(
            active_replace.await.unwrap(),
            Err(AuthoritySupervisorError::Stopped)
        ));
        assert!(matches!(
            late_replace.await.unwrap(),
            Err(AuthoritySupervisorError::Stopped)
        ));
        shutdown_task.await.unwrap();
        assert_eq!(installer.completed_credentials(), 2);

        let completed_after_shutdown = installer.completed_credentials();
        let (rejected_credential, rejected_expiration) =
            credential(peer_id, domain_id, issued_at + 3);
        assert!(matches!(
            handle
                .replace(external_update(
                    domain_id,
                    peer_id,
                    keys(),
                    rejected_credential,
                    rejected_expiration,
                ))
                .await,
            Err(AuthoritySupervisorError::Stopped)
        ));
        tokio::task::yield_now().await;
        assert_eq!(installer.completed_credentials(), completed_after_shutdown);
    }

    #[tokio::test]
    async fn shutdown_wakes_external_waiters_and_fences_authorization() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let (credential, expiration) = credential(peer_id, domain_id, unix_time());
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let mut slow_config = config();
        slow_config.early_refresh_timeout = Duration::from_secs(5);
        let (supervisor, _handle, mut requests) =
            AuthoritySupervisor::start_external_with_installer(
                installer,
                external_update(domain_id, peer_id, keys(), credential, expiration),
                slow_config,
                &shutdown,
            )
            .await
            .unwrap();
        let relay = supervisor.relay_authorization();
        let status = supervisor.subscribe_status();
        let waiter_relay = relay.clone();
        let waiter = tokio::spawn(async move { waiter_relay.refresh_after_unauthorized(1).await });
        tokio::time::timeout(Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();

        shutdown.cancel();
        assert!(waiter.await.unwrap().is_err());
        assert_eq!(*status.borrow(), AuthorityStatus::Stopped);
        assert!(relay.authorization().await.is_err());
        assert_eq!(requests.recv().await, None);
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn pull_recovery_continues_after_literal_expiry_while_authorization_stays_fenced() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let initial_expiration_unix = unix_time() + 2;
        let initial_issued_at = initial_expiration_unix - P2P_TOKEN_TTL.as_secs();
        let (initial_credential, initial_expiration) =
            credential(peer_id, domain_id, initial_issued_at);
        let (replacement, replacement_expiration) = credential(peer_id, domain_id, unix_time());
        let renewal = ScriptedRenewal::new([
            RenewalStep::Fail {
                after: Duration::from_secs(5),
            },
            RenewalStep::Return {
                after: Duration::ZERO,
                update: Box::new(renewed_update(
                    domain_id,
                    peer_id,
                    keys(),
                    replacement,
                    replacement_expiration,
                )),
            },
        ]);
        let installer = Arc::new(FakeInstaller::new(domain_id, peer_id));
        let shutdown = CancellationToken::new();
        let mut recovery_config = config();
        recovery_config.renewal_attempt_timeout = Duration::from_secs(5);
        recovery_config.retry_max = Duration::from_millis(200);
        let supervisor = AuthoritySupervisor::start_pull_with_installer(
            installer,
            prepared(
                domain_id,
                peer_id,
                initial_credential,
                initial_expiration,
                Utc::now() + chrono::Duration::milliseconds(30),
                renewal.clone(),
            ),
            recovery_config,
            &shutdown,
        )
        .await
        .unwrap();
        let relay = supervisor.relay_authorization();
        let mut status = supervisor.subscribe_status();

        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if matches!(
                    status.borrow_and_update().clone(),
                    AuthorityStatus::Expired { .. }
                ) {
                    break;
                }
                status.changed().await.unwrap();
            }
        })
        .await
        .expect("literal expiry must be published");
        assert!(relay.authorization().await.is_err());

        wait_for_ready_revision(&supervisor, 2).await;
        assert_eq!(renewal.calls(), 2);
        assert_eq!(relay.authorization().await.unwrap().revision(), 2);
        supervisor.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn real_node_installs_the_same_ordered_external_replacement() {
        let identity = identity();
        let peer_id = identity.peer_id();
        let domain_id = Uuid::new_v4();
        let issued_at = unix_time();
        let (initial_credential, initial_expiration) = credential(peer_id, domain_id, issued_at);
        let verifier = DdsTokenVerifier::from_keys(keys()).unwrap();
        let node = Node::start(identity, verifier, []).unwrap();
        let shutdown = CancellationToken::new();
        let (supervisor, handle, _) = AuthoritySupervisor::start_external(
            FixedDomainAuthority::new(node.authority(), domain_id),
            external_update(
                domain_id,
                peer_id,
                keys(),
                initial_credential,
                initial_expiration,
            ),
            config(),
            &shutdown,
        )
        .await
        .unwrap();
        let (replacement, replacement_expiration) = signed_credential(
            peer_id,
            domain_id,
            issued_at + 1,
            Uuid::new_v4(),
            ROTATED_DDS_PRIVATE_KEY,
        );
        handle
            .replace(external_update(
                domain_id,
                peer_id,
                rotated_keys(),
                replacement,
                replacement_expiration,
            ))
            .await
            .unwrap();
        assert_eq!(
            supervisor
                .relay_authorization()
                .authorization()
                .await
                .unwrap()
                .revision(),
            2
        );
        assert!(node.authority().require(domain_id).await.is_ok());

        supervisor.shutdown().await;
        node.shutdown().await.unwrap();
    }
}
