//! Application-owned resource catalog snapshots.

use auki_protocols::catalog::{
    v2::{ResourceEntry, ResourcesRequest, Variant, VariantContent},
    v4::ResourcesResponse as ResourcesResponseV4,
};
use std::path::Path;

/// Application-supplied source of truth for resources this Domain can
/// currently provide. Install this provider for sensor streams, pose logs,
/// time-transform logs, detections, and other v0.2-compatible rows.
pub trait ResourceCatalogProvider: Send + Sync + 'static {
    /// Snapshot currently-advertised resources. Called once per inbound
    /// resource-catalog request, so implementations must remain cheap.
    fn snapshot(&self) -> Vec<ResourceEntry>;

    /// Snapshot resources for a concrete request. The default
    /// implementation filters by requested variant and returns the
    /// matching rows.
    fn snapshot_for_request(
        &self,
        request: &ResourcesRequest,
        _registry_app_root: Option<&Path>,
    ) -> Vec<ResourceEntry> {
        let resources = self.snapshot();
        if request.variants.is_empty() {
            return resources;
        }
        resources
            .into_iter()
            .filter(|r| {
                let row_variant = match &r.variant_content {
                    VariantContent::SensorLog { .. } => Variant::SensorLog,
                    VariantContent::PoseLog { .. } => Variant::PoseLog,
                    VariantContent::TimeTransformLog { .. } => Variant::TimeTransformLog,
                    VariantContent::DetectionLog { .. } => Variant::DetectionLog,
                };
                request.variants.contains(&row_variant)
            })
            .collect()
    }
}

/// Application-supplied source of truth for Map Log rows this Domain can
/// currently provide through resource catalog v0.4.0.
pub trait MapCatalogProvider: Send + Sync + 'static {
    /// Return the currently available Map Log catalog.
    fn map_catalog(&self) -> ResourcesResponseV4;
}
