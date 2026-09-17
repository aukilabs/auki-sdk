use auki_auth::AuthFailureKind;
use auki_dms::jobs::JobsError;

#[derive(Debug, thiserror::Error)]
pub enum FleetError {
    #[error(transparent)]
    Auth(#[from] auki_auth::Error),
    #[error(transparent)]
    Jobs(#[from] JobsError),
    #[error("invalid fleet input: {0}")]
    InvalidInput(&'static str),
    #[error("invalid fleet response: {0}")]
    InvalidResponse(&'static str),
    #[error("fleet request cancelled")]
    Cancelled,
    #[error("fleet client is closed")]
    Closed,
    #[error("fleet snapshot timed out")]
    TimedOut,
}

impl FleetError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Auth(_) | Self::Jobs(JobsError::Auth(_)) => "auth",
            Self::InvalidInput(_) | Self::Jobs(JobsError::InvalidInput(_)) => "input",
            Self::InvalidResponse(_) | Self::Jobs(JobsError::InvalidResponse(_)) => "response",
            Self::Cancelled | Self::Jobs(JobsError::Cancelled) => "cancelled",
            Self::Closed | Self::Jobs(JobsError::Closed) => "closed",
            Self::TimedOut | Self::Jobs(JobsError::TimedOut) => "timeout",
            Self::Jobs(JobsError::HttpStatus { .. }) => "http",
            Self::Jobs(JobsError::TooLarge { .. }) => "limit",
            Self::Jobs(_) => "transport",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Auth(error) => match error {
                auki_auth::Error::ResponseTooLarge { .. } => "too_large",
                auki_auth::Error::InvalidResponse { .. } => "invalid_response",
                _ => match error.kind() {
                    AuthFailureKind::AuthenticationRequired => "authentication_required",
                    AuthFailureKind::Configuration => "configuration",
                    AuthFailureKind::AuthorizationDenied => "authorization_denied",
                    AuthFailureKind::Persistence => "persistence",
                    AuthFailureKind::Transient => "transient",
                    AuthFailureKind::Cancelled => "cancelled",
                    AuthFailureKind::Closed => "closed",
                },
            },
            Self::Jobs(error) => error.code(),
            Self::InvalidInput(_) => "invalid_input",
            Self::InvalidResponse(_) => "invalid_response",
            Self::Cancelled => "cancelled",
            Self::Closed => "closed",
            Self::TimedOut => "timed_out",
        }
    }

    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Auth(auki_auth::Error::HttpStatus { status, .. }) => Some(*status),
            Self::Jobs(error) => error.http_status(),
            _ => None,
        }
    }

    pub(crate) fn fatal(&self) -> bool {
        matches!(
            self.code(),
            "authentication_required"
                | "configuration"
                | "persistence"
                | "closed"
                | "cancelled"
                | "invalid_input"
                | "invalid_response"
        )
    }
}
