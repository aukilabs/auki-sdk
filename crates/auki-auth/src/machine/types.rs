use chrono::{DateTime, Utc};
use hex::FromHexError;
use k256::ecdsa;
use reqwest::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SiweError {
    #[error("invalid private key hex: {0}")]
    InvalidHex(FromHexError),
    #[error("invalid private key length: expected 32 bytes, got {0}")]
    InvalidPrivateKeyLength(usize),
    #[error("failed to initialize signing key: {0}")]
    InvalidSigningKey(ecdsa::Error),
    #[error("failed to sign SIWE message: {0}")]
    Signing(ecdsa::Error),
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error("dds siwe upstream returned status {0}")]
    UpstreamStatus(StatusCode),
    #[error(transparent)]
    InvalidExpiration(#[from] chrono::ParseError),
    #[error("missing field '{0}' in response")]
    MissingField(&'static str),
    #[error("DDS peer binding failed: {0}")]
    PeerBinding(String),
}

pub type Result<T> = std::result::Result<T, SiweError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessBundle {
    token: String,
    expires_at: DateTime<Utc>,
}

impl AccessBundle {
    pub fn new(token: impl Into<String>, expires_at: DateTime<Utc>) -> Self {
        Self {
            token: token.into(),
            expires_at,
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

impl SiweError {
    pub fn status_code(&self) -> Option<StatusCode> {
        match self {
            SiweError::Request(err) => err.status(),
            SiweError::UpstreamStatus(status) => Some(*status),
            _ => None,
        }
    }
}
