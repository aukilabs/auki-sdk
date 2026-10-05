use std::sync::Arc;

use auki_sdk_rs::{
    AukiPeerBootstrapError, AuthError, AuthFailureKind, ZitadelSessionCredentials,
    ZitadelSessionStore,
};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::{AukiSdkError, operation_error_chain};

/// Host action required by an auth operation or the peer lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiAuthFailureKind {
    AuthenticationRequired,
    Configuration,
    AuthorizationDenied,
    Persistence,
    Transient,
    Cancelled,
    Closed,
}

impl From<AuthFailureKind> for AukiAuthFailureKind {
    fn from(kind: AuthFailureKind) -> Self {
        match kind {
            AuthFailureKind::AuthenticationRequired => Self::AuthenticationRequired,
            AuthFailureKind::Configuration => Self::Configuration,
            AuthFailureKind::AuthorizationDenied => Self::AuthorizationDenied,
            AuthFailureKind::Persistence => Self::Persistence,
            AuthFailureKind::Transient => Self::Transient,
            AuthFailureKind::Cancelled => Self::Cancelled,
            AuthFailureKind::Closed => Self::Closed,
        }
    }
}

pub(crate) fn auth_error(kind: AuthFailureKind) -> AukiSdkError {
    AukiSdkError::Authentication { kind: kind.into() }
}

pub(crate) fn bootstrap_error(
    context: &'static str,
    error: AukiPeerBootstrapError,
) -> AukiSdkError {
    match error {
        AukiPeerBootstrapError::ConfigureAuthentication(e)
        | AukiPeerBootstrapError::Authenticate(e)
        | AukiPeerBootstrapError::ListDomains(e)
        | AukiPeerBootstrapError::AuthorizePeer(e) => auth_error(e.kind()),
        other => operation_error_chain(context, other),
    }
}

/// Opaque credential object: diagnostic reflection never exposes token fields.
/// The five constructor fields are the host's public-client PKCE result.
#[derive(Debug, uniffi::Object)]
pub struct AukiZitadelCredentials {
    credentials: ZitadelSessionCredentials,
}

impl AukiZitadelCredentials {
    pub(crate) fn copy_from(c: &ZitadelSessionCredentials) -> Self {
        Self {
            credentials: ZitadelSessionCredentials::new(
                c.access_token().expose_secret(),
                c.refresh_token().expose_secret(),
                c.client_id(),
                c.issuer().clone(),
                c.access_token_expires_at(),
            )
            .expect("copy of validated credentials"),
        }
    }

    pub(crate) fn copy_credentials(&self) -> ZitadelSessionCredentials {
        Self::copy_from(&self.credentials).credentials
    }
}

#[uniffi::export]
impl AukiZitadelCredentials {
    /// Expiry is optional RFC 3339, never a floating-point Unix timestamp.
    #[uniffi::constructor]
    pub fn new(
        access_token: String,
        refresh_token: String,
        client_id: String,
        issuer: String,
        access_token_expires_at: Option<String>,
    ) -> Result<Arc<Self>, AukiSdkError> {
        let invalid = || auth_error(AuthFailureKind::Configuration);
        let expiry = access_token_expires_at
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|t| t.with_timezone(&Utc))
                    .map_err(|_| invalid())
            })
            .transpose()?;
        let credentials = ZitadelSessionCredentials::new(
            access_token,
            refresh_token,
            client_id,
            issuer.parse().map_err(|_| invalid())?,
            expiry,
        )
        .map_err(|e| auth_error(e.kind()))?;
        Ok(Arc::new(Self { credentials }))
    }

    pub fn expose_access_token(&self) -> String {
        self.credentials.access_token().expose_secret().into()
    }
    pub fn expose_refresh_token(&self) -> String {
        self.credentials.refresh_token().expose_secret().into()
    }
    pub fn client_id(&self) -> String {
        self.credentials.client_id().into()
    }
    pub fn issuer(&self) -> String {
        self.credentials.issuer().to_string()
    }
    /// Canonical UTC/Z representation, preserving fractional second precision.
    pub fn access_token_expires_at(&self) -> Option<String> {
        self.credentials
            .access_token_expires_at()
            .map(|t| t.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }
}

/// Storage failures deliberately carry no platform message or token data.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AukiPersistenceError {
    #[error("could not persist the replacement session")]
    Failed,
}

// Foreign hosts can throw errors outside the declared FFI error type. UniFFI's
// default conversion panics with their text; discard it before diagnostics or
// the session-owned worker can observe that panic.
impl From<uniffi::UnexpectedUniFFICallbackError> for AukiPersistenceError {
    fn from(_: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed
    }
}

/// Atomically persist the whole snapshot before returning. On success OR error,
/// all writes must have settled. No detached writes or reentry into this session.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait AukiZitadelSessionStore: Send + Sync {
    async fn save(
        &self,
        credentials: Arc<AukiZitadelCredentials>,
    ) -> Result<(), AukiPersistenceError>;
}

pub(crate) struct SwiftStore(pub Arc<dyn AukiZitadelSessionStore>);

#[async_trait::async_trait]
impl ZitadelSessionStore for SwiftStore {
    async fn save(&self, credentials: &ZitadelSessionCredentials) -> Result<(), AuthError> {
        self.0
            .save(Arc::new(AukiZitadelCredentials::copy_from(credentials)))
            .await
            .map_err(|_| AuthError::Persistence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_timestamp_and_redacted_credentials() {
        let c = AukiZitadelCredentials::new(
            "ACCESS_SECRET".into(),
            "REFRESH_SECRET".into(),
            "client".into(),
            "https://issuer.example".into(),
            Some("2026-09-07T12:00:00.123456789+07:00".into()),
        )
        .unwrap();
        assert_eq!(
            c.access_token_expires_at().as_deref(),
            Some("2026-09-07T05:00:00.123456789Z")
        );
        let copy = AukiZitadelCredentials::copy_from(&c.copy_credentials());
        assert_eq!(copy.access_token_expires_at(), c.access_token_expires_at());
        assert_eq!(copy.expose_refresh_token(), "REFRESH_SECRET");
        let diagnostic = format!("{c:?}");
        assert!(!diagnostic.contains("ACCESS_SECRET") && !diagnostic.contains("REFRESH_SECRET"));
        assert!(diagnostic.contains("redacted"));
        let unknown = AukiZitadelCredentials::new(
            "a".into(),
            "r".into(),
            "client".into(),
            "https://issuer.example".into(),
            None,
        )
        .unwrap();
        assert_eq!(unknown.access_token_expires_at(), None);
    }

    #[test]
    fn auth_kinds_survive_startup_and_lifecycle_boundaries() {
        let startup = bootstrap_error(
            "test",
            AukiPeerBootstrapError::AuthorizePeer(AuthError::Persistence),
        );
        assert!(matches!(
            startup,
            AukiSdkError::Authentication {
                kind: AukiAuthFailureKind::Persistence
            }
        ));
        let status = crate::AukiPeerStatus::from(auki_sdk_rs::AukiPeerStatus::Failed(
            auki_sdk_rs::AukiPeerFailure::Authentication(AuthFailureKind::AuthenticationRequired),
        ));
        assert_eq!(
            status,
            crate::AukiPeerStatus::FailedAuthentication {
                kind: AukiAuthFailureKind::AuthenticationRequired
            }
        );
        let error = AukiZitadelCredentials::new(
            "a".into(),
            "r".into(),
            "client".into(),
            "DO_NOT_LEAK_SECRET".into(),
            None,
        )
        .unwrap_err();
        assert!(!error.to_string().contains("DO_NOT_LEAK"));
    }
}
