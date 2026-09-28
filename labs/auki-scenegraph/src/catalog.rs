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
