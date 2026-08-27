use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Fail-closed authentication and authority-preparation failures.
///
/// Response bodies, credentials, and bearer tokens are deliberately absent
/// from every variant.
#[derive(Debug, Error)]
pub enum Error {
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
