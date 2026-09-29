//! Complete map relevance data carried by the snapshot Product's Catalog entry.
use crate::{MapDefinition, MapSnapshot};
use auki_components::CatalogProductMetadata;
use serde::{Deserialize, Serialize};

pub const MAP_CATALOG_SCHEMA: &str = "auki.scenegraph.qr-catalog/v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPortal {
    /// Map-local anchor ID. Auki's Mapper uses the canonical Portal UUID.
    pub anchor_id: String,
    /// Exact decoded QR content, enabling relevance checks without an HTTP lookup.
    pub payload: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapCatalogData {
    pub map: MapDefinition,
    /// Complete list of QR anchors, sorted by anchor ID. Never a sample or filter.
    /// Generic QR anchors are included too; they are not falsely assigned Portal UUIDs.
    pub portals: Vec<CatalogPortal>,
}
impl MapCatalogData {
    pub fn from_snapshot(snapshot: &MapSnapshot) -> Self {
        Self {
            map: snapshot.scenegraph.map.clone(),
            portals: snapshot
                .scenegraph
                .anchors
                .values()
                .map(|anchor| CatalogPortal {
                    anchor_id: anchor.anchor_id.clone(),
                    payload: anchor.payload.clone(),
                })
                .collect(),
        }
    }
    pub fn contains_payload(&self, payload: &str) -> bool {
        self.portals.iter().any(|portal| portal.payload == payload)
    }
    pub fn metadata(&self, source_sequence: u64) -> Result<CatalogProductMetadata, String> {
        let metadata = CatalogProductMetadata {
            schema: MAP_CATALOG_SCHEMA.into(),
            source_sequence,
            value: serde_json::to_value(self).map_err(|e| e.to_string())?,
        };
        metadata.validate()?;
        Ok(metadata)
    }
}

/// One advertised map revision. Membership is an assertion, not proof of alignment.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvertisedMap {
    pub snapshot: crate::component::SnapshotReference,
    pub data: MapCatalogData,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalMapOverlap {
    pub local: AdvertisedMap,
    pub shared_portal_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlappingMap {
    pub remote: AdvertisedMap,
    /// Each local map is kept separate; disconnected local maps are not flattened.
    pub local_matches: Vec<LocalMapOverlap>,
}

#[derive(Debug, thiserror::Error)]
#[error("map catalog: {0}")]
pub struct MapCatalogError(pub String);

/// List maps advertised using the supported map-catalog metadata schema.
/// Call after fetching an authorized peer catalog (or use a cached catalog).
/// Missing/unknown metadata is not interpreted as an empty map. Such Products
/// are omitted because their map membership cannot be decoded by this helper.
/// Malformed supported metadata rejects the query rather than hiding a map.
pub fn list_maps(
    catalog: &auki_components::CatalogSnapshot,
) -> Result<Vec<AdvertisedMap>, MapCatalogError> {
    let mut maps = Vec::new();
    for entry in &catalog.products {
        let Some(metadata) = &entry.metadata else {
            continue;
        };
        if metadata.schema != MAP_CATALOG_SCHEMA {
            continue;
        }
        metadata.validate().map_err(MapCatalogError)?;
        if entry.manifest_hash != entry.manifest.hash() {
            return Err(MapCatalogError("Product manifest hash mismatch".into()));
        }
        let data: MapCatalogData = serde_json::from_value(metadata.value.clone())
            .map_err(|e| MapCatalogError(e.to_string()))?;
        validate_membership(&data)?;
        maps.push(AdvertisedMap {
            snapshot: crate::component::SnapshotReference {
                product: entry.manifest.reference(),
                sequence: metadata.source_sequence,
            },
            data,
        });
    }
    Ok(maps)
}

/// Find direct Portal overlap without fetching snapshots or comparing geometry.
/// Conventions, units, Domain names and generic QR payloads do not affect matching.
/// Results contain each remote catalog entry once, with all matching local maps.
pub fn find_overlapping_maps(
    local_maps: &[AdvertisedMap],
    remote_catalog: &auki_components::CatalogSnapshot,
) -> Result<Vec<OverlappingMap>, MapCatalogError> {
    for local in local_maps {
        validate_membership(&local.data)?;
    }
    let mut result = Vec::new();
    for remote in list_maps(remote_catalog)? {
        let local_matches: Vec<_> = local_maps
            .iter()
            .filter_map(|local| {
                let shared_portal_ids = shared_portal_ids(&local.data, &remote.data);
                (!shared_portal_ids.is_empty()).then(|| LocalMapOverlap {
                    local: local.clone(),
                    shared_portal_ids,
                })
            })
            .collect();
        if !local_matches.is_empty() {
            result.push(OverlappingMap {
                remote,
                local_matches,
            });
        }
    }
    Ok(result)
}

fn validate_membership(data: &MapCatalogData) -> Result<(), MapCatalogError> {
    crate::Scenegraph::new(data.map.clone()).map_err(|e| MapCatalogError(e.to_string()))?;
    data.metadata(0).map_err(MapCatalogError)?;
    if data.portals.len() > crate::MAX_ANCHORS {
        return Err(MapCatalogError("too many portals".into()));
    }
    let mut ids = std::collections::BTreeSet::new();
    for portal in &data.portals {
        crate::text(&portal.anchor_id, 256).map_err(|e| MapCatalogError(e.to_string()))?;
        crate::text(&portal.payload, 4096).map_err(|e| MapCatalogError(e.to_string()))?;
        if !ids.insert(&portal.anchor_id) {
            return Err(MapCatalogError("duplicate portal identity".into()));
        }
    }
    Ok(())
}

// Keep the query and incremental alignment checker on identical identity rules.
pub(crate) fn shared_portal_ids(a: &MapCatalogData, b: &MapCatalogData) -> Vec<String> {
    let ids: std::collections::BTreeSet<_> =
        b.portals.iter().map(|p| p.anchor_id.as_str()).collect();
    a.portals
        .iter()
        .filter(|p| {
            uuid::Uuid::parse_str(&p.anchor_id).is_ok() && ids.contains(p.anchor_id.as_str())
        })
        .map(|p| p.anchor_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
