//! Domain HTTP access, independent of peer startup and DMS configuration.
//!
//! The wire operations and metadata follow Posemesh's `core/domain-http` client.
//! Authentication is supplied by `auki-auth`; no second login/refresh loop runs.

mod data;
mod error;
mod http;
mod types;

pub use auki_auth::{DomainListQuery, DomainPage, DomainSummary};
pub use data::{AukiDomainData, DomainDataClient};
pub use error::DataError;
pub use types::{DataLimits, DataListQuery, DataMetadata, DataWrite};

use auki_auth::AuthSession;
use tokio_util::sync::CancellationToken;

/// Ordinary DDS Domain discovery. Listing does not grant data read/write access.
#[derive(Clone, Debug)]
pub struct AukiDomains(AuthSession);

impl AukiDomains {
    pub fn new(credential: AuthSession) -> Self {
        Self(credential)
    }

    pub async fn list(&self, query: &DomainListQuery) -> Result<DomainPage, DataError> {
        Ok(self.0.list_domains(query).await?)
    }

    pub async fn list_with_cancellation(
        &self,
        query: &DomainListQuery,
        cancellation: &CancellationToken,
    ) -> Result<DomainPage, DataError> {
        Ok(self
            .0
            .list_domains_with_cancellation(query, cancellation)
            .await?)
    }
}
