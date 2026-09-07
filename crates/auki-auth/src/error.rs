use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Recovery action shared by sessions, peer supervisors, and host adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthFailureKind {
    AuthenticationRequired,
    Configuration,
    AuthorizationDenied,
    Persistence,
    Transient,
    Cancelled,
    Closed,
}

impl AuthFailureKind {
    /// These failures require host action, not another automatic renewal.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::AuthenticationRequired
                | Self::Configuration
                | Self::AuthorizationDenied
                | Self::Closed
        )
    }
}

/// Fail-closed authentication and authority-preparation failures.
///
/// Response bodies, credentials, and bearer tokens are deliberately absent
/// from every variant.
#[derive(Debug, Error)]
pub enum Error {
    #[error("authentication is required; sign in again")]
    AuthenticationRequired,

    #[error("the auth session is closed")]
    SessionClosed,

    #[error("the session operation is still running; retry waiting without another refresh")]
    SessionOperationPending,

    #[error("the principal has no readable Domains")]
    AuthorizationDenied,

    #[error("ZITADEL rejected refresh: {0:?}")]
    ZitadelOAuth(crate::ZitadelOAuthError),

    #[error("ZITADEL refresh outcome is unknown; sign in again")]
    RefreshOutcomeUnknown,

    #[error("could not persist the replacement ZITADEL session")]
    Persistence,

    #[error("invalid auth configuration: {0}")]
    InvalidConfiguration(&'static str),

    #[error("invalid {field}: {reason}")]
    InvalidInput {
        field: &'static str,
        reason: &'static str,
    },

    #[error("{endpoint} was cancelled")]
    Cancelled { endpoint: &'static str },

    #[error("{endpoint} timed out")]
    RequestTimedOut { endpoint: &'static str },

    #[error("{endpoint} request failed")]
    Transport { endpoint: &'static str },

    #[error("{endpoint} returned HTTP {status}")]
    HttpStatus { endpoint: &'static str, status: u16 },

    #[error("{endpoint} response exceeded {maximum} bytes")]
    ResponseTooLarge {
        endpoint: &'static str,
        maximum: usize,
    },

    #[error("{endpoint} returned an invalid response: {reason}")]
    InvalidResponse {
        endpoint: &'static str,
        reason: &'static str,
    },

    #[error("selected Domain is not accessible")]
    DomainNotAccessible,

    #[error("accessible Domain result is truncated ({returned} of advisory total {total})")]
    AccessibleDomainsTruncated { total: u64, returned: usize },

    #[error("DDS returned stale authority material")]
    StaleAuthority,

    #[error("DDS changed verification keys without advancing their generation")]
    VerificationKeyGenerationConflict,

    #[error("P2P authority validation failed")]
    InvalidP2pAuthority(#[source] auki_p2p::Error),
}

impl Error {
    pub fn kind(&self) -> AuthFailureKind {
        use crate::ZitadelOAuthError as OAuth;
        match self {
            Self::AuthenticationRequired
            | Self::RefreshOutcomeUnknown
            | Self::ZitadelOAuth(OAuth::InvalidGrant | OAuth::Other)
            | Self::HttpStatus { status: 401, .. } => AuthFailureKind::AuthenticationRequired,
            Self::InvalidConfiguration(_)
            | Self::InvalidInput { .. }
            | Self::ZitadelOAuth(
                OAuth::InvalidClient
                | OAuth::UnauthorizedClient
                | OAuth::InvalidRequest
                | OAuth::InvalidScope,
            ) => AuthFailureKind::Configuration,
            Self::AuthorizationDenied
            | Self::DomainNotAccessible
            | Self::HttpStatus { status: 403, .. } => AuthFailureKind::AuthorizationDenied,
            Self::Persistence => AuthFailureKind::Persistence,
            Self::Cancelled { .. } => AuthFailureKind::Cancelled,
            Self::SessionClosed => AuthFailureKind::Closed,
            _ => AuthFailureKind::Transient,
        }
    }

    pub(crate) fn invalid_response(endpoint: &'static str, reason: &'static str) -> Self {
        Self::InvalidResponse { endpoint, reason }
    }

    pub(crate) fn is_unauthorized(&self) -> bool {
        matches!(self, Self::HttpStatus { status: 401, .. })
    }
}

impl From<auki_p2p::Error> for Error {
    fn from(error: auki_p2p::Error) -> Self {
        Self::InvalidP2pAuthority(error)
    }
}
