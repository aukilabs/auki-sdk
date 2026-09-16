use auki_auth::AuthFailureKind;

/// Errors contain neither backend response bodies nor credentials.
#[derive(Debug, thiserror::Error)]
pub enum JobsError {
    #[error(transparent)]
    Auth(#[from] auki_auth::Error),
    #[error("invalid jobs input: {0}")]
    InvalidInput(&'static str),
    #[error("invalid DMS response: {0}")]
    InvalidResponse(&'static str),
    #[error("DMS returned HTTP {status}")]
    HttpStatus { status: u16 },
    #[error("DMS request failed")]
    Transport,
    #[error("DMS request timed out")]
    TimedOut,
    #[error("DMS request was cancelled")]
    Cancelled,
    #[error("the jobs client is closed")]
    Closed,
    #[error("DMS request or response exceeded {maximum} bytes")]
    TooLarge { maximum: usize },
    /// DMS may have accepted the submission. Inspect existing jobs before
    /// choosing whether to submit again; repeating it may duplicate work/charges.
    #[error("job submission outcome is unknown; do not automatically resubmit: {source}")]
    SubmissionUncertain { source: Box<JobsError> },
}

impl JobsError {
    /// Stable host-facing recovery category, including imported-session failures.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Auth(error) => match error.kind() {
                AuthFailureKind::AuthenticationRequired => "authentication_required",
                AuthFailureKind::Configuration => "configuration",
                AuthFailureKind::AuthorizationDenied => "authorization_denied",
                AuthFailureKind::Persistence => "persistence",
                AuthFailureKind::Transient => "transient",
                AuthFailureKind::Cancelled => "cancelled",
                AuthFailureKind::Closed => "closed",
            },
            Self::InvalidInput(_) => "invalid_input",
            Self::InvalidResponse(_) => "invalid_response",
            Self::HttpStatus { .. } => "http_status",
            Self::Transport => "transport",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Closed => "closed",
            Self::TooLarge { .. } => "too_large",
            Self::SubmissionUncertain { .. } => "submission_uncertain",
        }
    }

    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::HttpStatus { status }
            | Self::Auth(auki_auth::Error::HttpStatus { status, .. }) => Some(*status),
            Self::SubmissionUncertain { source } => source.http_status(),
            _ => None,
        }
    }

    pub(super) fn ambiguous_after_send(&self) -> bool {
        // All ordinary 4xx responses explicitly reject creation. An upstream
        // timeout, redirect, broken success response, or 5xx cannot prove that.
        !matches!(self, Self::HttpStatus { status } if (400..500).contains(status) && *status != 408)
    }
}
