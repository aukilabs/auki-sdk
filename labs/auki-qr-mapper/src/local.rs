use crate::{PortalSizeError, PortalSizeResolver, ResolvedPortal};
use auki_components::InMemoryTransport;
use auki_domain_client::PortalId;
use auki_scenegraph::{
    QrAnchor, RigidTransform,
    component::{LookupQr, MapComponent, SnapshotReference, UpsertQr},
    resolution::{PortalMaps, QrResolver, ResolvedQr},
};
use std::{future::Future, sync::Arc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Injectable metadata source; the production implementation uses authenticated DDS.
pub trait PortalMetadataSource {
    fn resolve(
        &self,
        domain: Uuid,
        portal: &PortalId,
        cancellation: &CancellationToken,
    ) -> impl Future<Output = Result<ResolvedPortal, PortalSizeError>>;
}
impl PortalMetadataSource for PortalSizeResolver {
    async fn resolve(
        &self,
        domain: Uuid,
        portal: &PortalId,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedPortal, PortalSizeError> {
        PortalSizeResolver::resolve(self, domain, portal, cancellation).await
    }
}

/// A host-approved placement, either establishing the first anchor's frame or
/// derived from a camera pose in this map. Never inferred from database size.
pub struct AnchorPlacement {
    pub map: Arc<MapComponent>,
    pub expected_snapshot: SnapshotReference,
    pub pose_in_map: RigidTransform,
}
#[derive(Debug)]
pub enum EnsureQrResult {
    Known(Vec<ResolvedQr>),
    Added(Box<ResolvedQr>),
    NeedsPlacement(ResolvedPortal),
}
#[derive(Debug, thiserror::Error)]
#[error("QR mapping failed: {0}")]
pub struct MappingError(pub String);

pub struct PortalMapper<S> {
    maps: PortalMaps,
    metadata: S,
}
impl<S: PortalMetadataSource> PortalMapper<S> {
    pub fn new(maps: PortalMaps, metadata: S) -> Self {
        Self { maps, metadata }
    }
    /// Query every registered local map before any database lookup. A missing QR
    /// is admitted only after metadata resolution and an authorized map commit.
    pub async fn lookup_helper(
        &self,
        payload: &str,
        domain: Uuid,
        placement: Option<AnchorPlacement>,
        cancellation: &CancellationToken,
    ) -> Result<EnsureQrResult, MappingError> {
        let known = self
            .maps
            .resolve(payload)
            .map_err(|e| MappingError(e.to_string()))?;
        if !known.is_empty() {
            return Ok(EnsureQrResult::Known(known));
        }
        if cancellation.is_cancelled() {
            return Err(MappingError("cancelled".into()));
        }
        let id = portal_id(payload)?;
        let metadata = self
            .metadata
            .resolve(domain, &id, cancellation)
            .await
            .map_err(|e| MappingError(e.to_string()))?;
        if !id.matches(metadata.metadata().id, &metadata.metadata().short_id) {
            return Err(MappingError(
                "metadata does not match detected Portal".into(),
            ));
        }
        if cancellation.is_cancelled() {
            return Err(MappingError("cancelled".into()));
        }
        // Another mapper may have admitted it while the database request was in flight.
        let known = self
            .maps
            .resolve(payload)
            .map_err(|e| MappingError(e.to_string()))?;
        if !known.is_empty() {
            return Ok(EnsureQrResult::Known(known));
        }
        let Some(placement) = placement else {
            return Ok(EnsureQrResult::NeedsPlacement(metadata));
        };
        if !self.maps.contains(&placement.map) {
            return Err(MappingError("target map is not registered locally".into()));
        }
        let anchor = QrAnchor {
            anchor_id: metadata.metadata().id.to_string(),
            payload: payload.into(),
            side_length_m: metadata.side_length_m(),
            pose_in_map: placement.pose_in_map,
        };
        let existing = InMemoryTransport
            .invoke(
                placement.map.lookup_qr(),
                self.maps.context().clone(),
                LookupQr {
                    anchor_id: anchor.anchor_id.clone(),
                },
            )
            .map_err(|e| MappingError(e.to_string()))?
            .result;
        // Do not overwrite a known physical marker under a different decoded payload.
        if existing.anchor.is_some() {
            return Err(MappingError(
                "Portal identity already exists with a different payload".into(),
            ));
        }
        let snapshot = InMemoryTransport
            .invoke(
                placement.map.upsert_qr(),
                self.maps.context().clone(),
                UpsertQr {
                    expected_snapshot: placement.expected_snapshot,
                    anchor: anchor.clone(),
                },
            )
            .map_err(|e| MappingError(e.to_string()))?
            .result;
        Ok(EnsureQrResult::Added(Box::new(ResolvedQr {
            snapshot,
            map: existing.map,
            anchor,
        })))
    }
}

/// Accept a Portal ID or the Console's R8.HR URL convention. Never fetch the URL.
pub fn portal_id(payload: &str) -> Result<PortalId, MappingError> {
    if let Ok(id) = PortalId::parse(payload) {
        return Ok(id);
    }
    if let Some((scheme, rest)) = payload.split_once("://")
        && scheme.eq_ignore_ascii_case("https")
        && let Some((host, id)) = rest.split_once('/')
        && host.eq_ignore_ascii_case("r8.hr")
    {
        return PortalId::parse(id).map_err(|e| MappingError(e.to_string()));
    }
    Err(MappingError(
        "unknown QR is not an accepted Auki Portal payload".into(),
    ))
}
