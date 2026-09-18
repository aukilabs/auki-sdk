//! Domain HTTP access, independent of peer startup and DMS configuration.
//!
//! The wire operations and metadata follow Posemesh's `core/domain-http` client.
//! Authentication is supplied by `auki-auth`; no second login/refresh loop runs.

mod data;
mod error;
mod http;
mod portals;
mod transfer;
mod types;

pub use auki_auth::{DomainListQuery, DomainPage, DomainSummary, Portal, PortalDomain, PortalId};
pub use data::{AukiDomainData, DomainDataClient};
pub use error::DataError;
pub use types::{DataLimits, DataListQuery, DataMetadata, DataWrite, PortalPose, TransferOptions};

use auki_auth::AuthSession;
use tokio_util::sync::CancellationToken;

/// Ordinary DDS Domain discovery. Listing does not grant data read/write access.
#[derive(Clone, Debug)]
pub struct AukiDomains(AuthSession);

impl AukiDomains {
    pub fn new(credential: AuthSession) -> Self {
        Self(credential)
    }

    pub async fn for_portal(
        &self,
        portal: &PortalId,
        organization: &str,
        cancellation: &CancellationToken,
    ) -> Result<Vec<PortalDomain>, DataError> {
        Ok(self
            .0
            .domains_for_portal(portal, organization, cancellation)
            .await?)
    }

    pub async fn for_portal_page(
        &self,
        portal: &PortalId,
        organization: &str,
        limit: usize,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<auki_auth::InventoryPage<PortalDomain>, DataError> {
        Ok(self
            .0
            .domains_for_portal_page(portal, organization, limit, cursor, cancellation)
            .await?)
    }

    pub async fn portals_page(
        &self,
        domain: uuid::Uuid,
        limit: usize,
        cursor: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<auki_auth::InventoryPage<Portal>, DataError> {
        Ok(self
            .0
            .list_portals_page(domain, limit, cursor, cancellation)
            .await?)
    }

    pub async fn portals(
        &self,
        domain: uuid::Uuid,
        cancellation: &CancellationToken,
    ) -> Result<Vec<Portal>, DataError> {
        Ok(self.0.list_portals(domain, cancellation).await?)
    }

    pub async fn portal(
        &self,
        domain: uuid::Uuid,
        portal: &PortalId,
        cancellation: &CancellationToken,
    ) -> Result<Portal, DataError> {
        Ok(self.0.get_portal(domain, portal, cancellation).await?)
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
