/// Errors omit response bodies, URLs, data and bearer material.
#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("Domain authentication: {0}")]
    Auth(#[from] auki_auth::Error),
    #[error("invalid data request: {0}")]
    InvalidInput(&'static str),
    #[error("Domain data client is closed")]
    Closed,
    #[error("Domain data operation was cancelled; a sent write may have completed")]
    Cancelled,
    #[error("Domain data operation timed out; a sent write may have completed")]
    TimedOut,
    #[error("Domain data transport failed; a sent write may have completed")]
    Transport,
    #[error("transfer source or destination failed")]
    Callback,
    #[error("multipart cleanup failed after {operation}; cleanup error: {cleanup}")]
    Cleanup {
        operation: Box<DataError>,
        cleanup: Box<DataError>,
    },
    #[error("Domain Server returned HTTP {status}")]
    HttpStatus { status: u16 },
    #[error("Domain Server returned an invalid response: {0}")]
    InvalidResponse(&'static str),
    #[error("transfer exceeds the {maximum}-byte limit")]
    TooLarge { maximum: usize },
}

impl DataError {
    /// Includes DDS/API errors, retaining permission/credit/missing-item distinctions.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::HttpStatus { status }
            | Self::Auth(auki_auth::Error::HttpStatus { status, .. }) => Some(*status),
            Self::Cleanup { operation, .. } => operation.status(),
            _ => None,
        }
    }
}
