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
    /// A matching keyed submission is still being processed. The caller may
    /// retry the same key and specification after this delay, with a bound.
    #[error(
        "job submission is in progress; retry the same key and request after {retry_after_seconds} seconds"
    )]
    SubmissionInProgress { retry_after_seconds: u32 },
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
    /// DMS may have accepted the submission. On a verified capable deployment,
    /// a keyed request can be retried with the same key and specification.
    /// Unkeyed submissions require reconciliation to avoid duplicate work/charges.
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
            Self::SubmissionInProgress { .. } => "submission_in_progress",
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
            Self::SubmissionInProgress { .. } => Some(409),
            Self::SubmissionUncertain { source } => source.http_status(),
            _ => None,
        }
    }

    pub fn retry_after_seconds(&self) -> Option<u32> {
        match self {
            Self::SubmissionInProgress {
                retry_after_seconds,
            } => Some(*retry_after_seconds),
            _ => None,
        }
    }

    pub(super) fn ambiguous_after_send(&self) -> bool {
        // All ordinary 4xx responses explicitly reject creation. An upstream
        // timeout, redirect, broken success response, or 5xx cannot prove that.
        match self {
            Self::SubmissionInProgress { .. } => false,
            Self::HttpStatus { status } if (400..500).contains(status) && *status != 408 => false,
            _ => true,
        }
    }
}
