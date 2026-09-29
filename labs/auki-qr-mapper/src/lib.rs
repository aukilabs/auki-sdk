//! Local-map-first QR admission. Placement is supplied explicitly by the host.
pub mod local;
use auki_domain_client::{AukiDomains, DataError, Portal, PortalId};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum PortalSizeError {
    #[error(transparent)]
    Lookup(Box<DataError>),
    #[error("Portal size must be positive and finite in both centimeters and meters")]
    InvalidSize,
}
impl From<DataError> for PortalSizeError {
    fn from(error: DataError) -> Self {
        Self::Lookup(Box::new(error))
    }
}

/// Metadata and its validated encoded-square side length. Retains the Portal's
/// canonical ID and updated_at so the mapper can track the source of its size.
#[derive(Clone, Debug)]
pub struct ResolvedPortal {
    portal: Portal,
    side_length_m: f64,
}
impl TryFrom<Portal> for ResolvedPortal {
    type Error = PortalSizeError;
    fn try_from(portal: Portal) -> Result<Self, PortalSizeError> {
        // DDS stores centimeters. Console prints the encoded module matrix at
        // this width; the white quiet zone and AQR decoration are outside it.
        let side_length_m = portal.size / 100.0;
        if !portal.size.is_finite() || !side_length_m.is_finite() || side_length_m <= 0.0 {
            return Err(PortalSizeError::InvalidSize);
        }
        Ok(Self {
            portal,
            side_length_m,
        })
    }
}
impl ResolvedPortal {
    pub fn metadata(&self) -> &Portal {
        &self.portal
    }
    /// Width of the encoded QR square in meters, excluding its quiet zone.
    pub fn side_length_m(&self) -> f64 {
        self.side_length_m
    }
}

/// Uses the existing authenticated DDS client; no separate credentials, endpoint
/// routing, guessed size, or silent offline fallback. The host selects the Domain.
#[derive(Clone, Debug)]
pub struct PortalSizeResolver {
    domains: AukiDomains,
}
impl PortalSizeResolver {
    pub fn new(domains: AukiDomains) -> Self {
        Self { domains }
    }

    /// Resolve before estimating an unknown QR's pose. The underlying client
    /// validates the returned Domain and Portal identity and honors cancellation.
    pub async fn resolve(
        &self,
        domain: Uuid,
        portal: &PortalId,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedPortal, PortalSizeError> {
        let metadata = self.domains.portal(domain, portal, cancellation).await?;
        ResolvedPortal::try_from(metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn portal() -> Portal {
        serde_json::from_value(serde_json::json!({
            "id":"00000000-0000-0000-0000-000000000001",
            "short_id":"ABC12345678","name":"fixture","size":10.0,
            "organization_id":null,"default_domain_id":null,"redirect_url":null,
            "created_at":"2026-09-01T00:00:00Z","updated_at":"2026-09-02T00:00:00Z"
        }))
        .unwrap()
    }
    #[test]
    fn converts_database_centimeters_without_adding_a_quiet_zone() {
        for (cm, meters) in [(5., 0.05), (7.5, 0.075), (10., 0.1), (20., 0.2)] {
            let mut metadata = portal();
            metadata.size = cm;
            let result = ResolvedPortal::try_from(metadata).unwrap();
            assert_eq!(result.side_length_m(), meters);
            assert_eq!(result.metadata().size, cm);
            assert_eq!(result.metadata().short_id, "ABC12345678");
            assert_eq!(result.metadata().updated_at, portal().updated_at);
        }
    }
    #[test]
    fn rejects_invalid_or_underflowing_size_without_a_default() {
        for size in [
            0.,
            -1.,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::from_bits(1),
        ] {
            let mut metadata = portal();
            metadata.size = size;
            assert!(matches!(
                ResolvedPortal::try_from(metadata),
                Err(PortalSizeError::InvalidSize)
            ));
        }
    }
}
