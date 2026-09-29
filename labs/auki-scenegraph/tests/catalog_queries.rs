#![cfg(feature = "components")]
use auki_components::*;
use auki_scenegraph::{catalog::*, *};
fn entry(name: &str, ids: &[u128]) -> CatalogProductEntry {
    let data = MapCatalogData {
        map: MapDefinition {
            map_id: name.into(),
            name: None,
            domain_reference: Some("store".into()),
            frame: MapFrame::in_convention(
                format!("{name}-frame"),
                "fixture",
                CoordinateConvention::OpenGl,
                1.,
            )
            .unwrap(),
        },
        portals: ids
            .iter()
            .map(|id| CatalogPortal {
                anchor_id: uuid::Uuid::from_u128(*id).to_string(),
                payload: format!("qr:{id}"),
            })
            .collect(),
    };
    let manifest = ProductManifest {
        schema: "auki.product-manifest/v1".into(),
        peer_id: "peer-2".into(),
        product_id: name.into(),
        form: ProductForm::Artifact,
        producer: OutputReference {
            peer_id: "peer-2".into(),
            component_id: name.into(),
            component_manifest_hash: "component-hash".into(),
            slot: "map".into(),
            output_id: name.into(),
            manifest_hash: "output-hash".into(),
        },
        access: vec![],
    };
    CatalogProductEntry {
        manifest_hash: manifest.hash(),
        manifest,
        metadata: Some(data.metadata(17).unwrap()),
        state: ProductState::Artifact,
    }
}
fn catalog(products: Vec<CatalogProductEntry>) -> CatalogSnapshot {
    CatalogSnapshot {
        revision: 3,
        components: vec![],
        products,
    }
}
#[test]
fn lists_maps_and_groups_direct_overlap_across_conventions() {
    let local_catalog = catalog(vec![entry("abc", &[1, 2, 3]), entry("cx", &[3, 9])]);
    let local = list_maps(&local_catalog).unwrap();
    let mut remote_entry = entry("cde", &[3, 4, 5]);
    let mut data: MapCatalogData =
        serde_json::from_value(remote_entry.metadata.as_ref().unwrap().value.clone()).unwrap();
    data.map.frame = MapFrame::in_convention(
        "remote-frame",
        "fixture",
        CoordinateConvention::Ros2Body,
        0.01,
    )
    .unwrap();
    remote_entry.metadata = Some(data.metadata(23).unwrap());
    let remote = catalog(vec![
        remote_entry,
        entry("fg", &[6, 7]),
        entry("empty", &[]),
    ]);
    assert_eq!(list_maps(&remote).unwrap().len(), 3);
    let found = find_overlapping_maps(&local, &remote).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].remote.data.map.map_id, "cde");
    assert_eq!(found[0].remote.snapshot.sequence, 23);
    assert_eq!(
        found[0].remote.snapshot.product,
        remote.products[0].manifest.reference()
    );
    assert_eq!(found[0].local_matches.len(), 2);
    assert_eq!(found[0].local_matches[0].local, local[0]);
    assert_eq!(
        found[0].local_matches[0].shared_portal_ids,
        vec![uuid::Uuid::from_u128(3).to_string()]
    );
    assert!(find_overlapping_maps(&[], &remote).unwrap().is_empty());
    assert!(
        find_overlapping_maps(&local, &catalog(vec![]))
            .unwrap()
            .is_empty()
    );
}
#[test]
fn unknown_membership_is_skipped_but_malformed_supported_data_is_an_error() {
    let mut missing = entry("missing", &[1]);
    missing.metadata = None;
    let mut unknown = entry("unknown", &[1]);
    unknown.metadata.as_mut().unwrap().schema = "future-schema".into();
    assert!(
        list_maps(&catalog(vec![missing, unknown]))
            .unwrap()
            .is_empty()
    );
    let mut bad = entry("bad", &[1]);
    bad.metadata.as_mut().unwrap().value = serde_json::json!({});
    assert!(list_maps(&catalog(vec![bad])).is_err());
    assert!(list_maps(&catalog(vec![entry("duplicate", &[1, 1])])).is_err());
    let mut bad_hash = entry("hash", &[1]);
    bad_hash.manifest_hash = "wrong".into();
    assert!(list_maps(&catalog(vec![bad_hash])).is_err());
}
#[test]
fn generic_anchor_names_and_payloads_do_not_establish_overlap() {
    let mut generic = entry("generic", &[1]);
    generic.metadata.as_mut().unwrap().value["portals"][0]["anchor_id"] = "C".into();
    let local = list_maps(&catalog(vec![generic.clone()])).unwrap();
    assert!(
        find_overlapping_maps(&local, &catalog(vec![generic]))
            .unwrap()
            .is_empty()
    );
    let local = list_maps(&catalog(vec![entry("local", &[1])])).unwrap();
    let mut remote = entry("remote", &[2]);
    remote.metadata.as_mut().unwrap().value["portals"][0]["payload"] = "qr:1".into();
    assert!(
        find_overlapping_maps(&local, &catalog(vec![remote]))
            .unwrap()
            .is_empty()
    );
}
