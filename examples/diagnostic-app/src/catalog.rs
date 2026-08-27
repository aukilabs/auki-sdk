use auki_domain::{PeerId, ResourceCatalogProvider};
use auki_network::resources_protocol::{
    Available, Head, ResourceEntry, SensorBlock, SensorKind, SensorManifestPointer, VariantContent,
};
use auki_registry::RegistryRef;

#[derive(Clone)]
pub(crate) struct StaticCatalog {
    rows: Vec<ResourceEntry>,
}

impl StaticCatalog {
    pub(crate) fn new(owner: PeerId, resource_ids: &[String]) -> Self {
        Self {
            rows: resource_ids
                .iter()
                .map(|resource_id| diagnostic_resource(owner, resource_id))
                .collect(),
        }
    }
}

impl ResourceCatalogProvider for StaticCatalog {
    fn snapshot(&self) -> Vec<ResourceEntry> {
        self.rows.clone()
    }
}

fn diagnostic_resource(owner: PeerId, resource_id: &str) -> ResourceEntry {
    let peer_id = owner.to_string();
    ResourceEntry {
        source_peer_id: peer_id.clone(),
        writer_peer_id: peer_id.clone(),
        resource_id: resource_id.into(),
        state: "live".into(),
        head: Some(Head::Rolling {
            retention_ns: 5_000_000_000,
        }),
        extent: None,
        available: Available {
            bytes: 1_024,
            entries: 1,
            duration_ns: 5_000_000_000,
        },
        sensor: Some(SensorBlock {
            kind: SensorKind::Camera,
            r#type: "diagnostic_rgb".into(),
            sensor_id: resource_id.into(),
            sensor_hash: "diagnostic-only".into(),
        }),
        pose: None,
        variant_content: VariantContent::SensorLog {
            manifest: SensorManifestPointer {
                clock: RegistryRef {
                    peer_id,
                    id: format!("{resource_id}/clock"),
                    hash: "diagnostic-only".into(),
                },
                frame: None,
            },
        },
    }
}
