#![cfg(feature = "components")]
use auki_components::*;
use auki_scenegraph::{catalog::*, component::*, *};
use std::sync::atomic::{AtomicU64, Ordering};
fn context() -> InvocationContext {
    InvocationContext {
        invocation_id: "test".into(),
        caller_peer_id: "owner".into(),
        caller_component_id: "mapper".into(),
    }
}
fn fixture() -> (ComponentRuntime, MapComponent) {
    let runtime = ComponentRuntime::new("owner");
    let clock = AtomicU64::new(1);
    let map = MapComponent::new(
        &runtime,
        MapComponentConfig {
            component_id: "map".into(),
            publication_id: "run".into(),
            clock: fixture_clock("clock"),
            map: MapDefinition {
                map_id: "shop-map".into(),
                name: None,
                domain_reference: Some("shop".into()),
                frame: MapFrame::z_up_meters("root", "chosen origin"),
            },
        },
        move || clock.fetch_add(1, Ordering::SeqCst),
        |_| true,
        |c| c.caller_peer_id == "owner",
    )
    .unwrap();
    (runtime, map)
}
fn anchor(id: &str, payload: &str) -> QrAnchor {
    QrAnchor {
        anchor_id: id.into(),
        payload: payload.into(),
        side_length_m: 0.2,
        pose_in_map: RigidTransform::identity(format!("{id}-frame"), "root"),
    }
}
fn entry(runtime: &ComponentRuntime, map: &MapComponent) -> CatalogProductEntry {
    runtime
        .catalog()
        .product(&map.product().manifest.product_id)
        .unwrap()
}
fn data(entry: &CatalogProductEntry) -> MapCatalogData {
    let metadata = entry.metadata.as_ref().unwrap();
    assert_eq!(metadata.schema, MAP_CATALOG_SCHEMA);
    serde_json::from_value(metadata.value.clone()).unwrap()
}
fn write(map: &MapComponent, anchor: QrAnchor) {
    InMemoryTransport
        .invoke(
            map.upsert_qr(),
            context(),
            UpsertQr {
                expected_snapshot: map.snapshot_reference(),
                anchor,
            },
        )
        .unwrap();
}
#[test]
fn complete_list_is_current_sorted_and_does_not_change_product_identity() {
    let (runtime, map) = fixture();
    let original = entry(&runtime, &map);
    assert!(data(&original).portals.is_empty());
    assert_eq!(data(&original).map.map_id, "shop-map");
    assert_eq!(original.metadata.unwrap().source_sequence, 0);
    let identity = map.product().reference();
    let before = runtime.catalog().revision();
    write(&map, anchor("b", "HTTPS://R8.HR/BBBBBBBBBBB"));
    write(&map, anchor("a", "HTTPS://R8.HR/AAAAAAAAAAA"));
    let current = entry(&runtime, &map);
    assert_eq!(current.manifest.reference(), identity);
    assert!(runtime.catalog().revision() > before);
    assert_eq!(
        current.metadata.as_ref().unwrap().source_sequence,
        map.snapshot_reference().sequence
    );
    assert_eq!(
        data(&current)
            .portals
            .iter()
            .map(|p| p.anchor_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert!(data(&current).contains_payload("HTTPS://R8.HR/AAAAAAAAAAA"));
    write(&map, anchor("a", "replacement"));
    let updated = entry(&runtime, &map);
    assert_eq!(data(&updated).portals.len(), 2);
    assert!(!data(&updated).contains_payload("HTTPS://R8.HR/AAAAAAAAAAA"));
    assert!(data(&updated).contains_payload("replacement"));
    let revision = runtime.catalog().revision();
    write(&map, anchor("a", "replacement"));
    assert_eq!(runtime.catalog().revision(), revision);
}
#[test]
fn denied_or_stale_edits_leave_advertised_membership_unchanged() {
    let (runtime, map) = fixture();
    let before = entry(&runtime, &map);
    let mut denied = context();
    denied.caller_peer_id = "stranger".into();
    assert!(
        InMemoryTransport
            .invoke(
                map.upsert_qr(),
                denied,
                UpsertQr {
                    expected_snapshot: map.snapshot_reference(),
                    anchor: anchor("a", "a")
                }
            )
            .is_err()
    );
    assert_eq!(before, entry(&runtime, &map));
    let old = map.snapshot_reference();
    write(&map, anchor("b", "b"));
    let current = entry(&runtime, &map);
    assert!(
        InMemoryTransport
            .invoke(
                map.upsert_qr(),
                context(),
                UpsertQr {
                    expected_snapshot: old,
                    anchor: anchor("a", "a")
                }
            )
            .is_err()
    );
    assert_eq!(current, entry(&runtime, &map));
}
#[test]
fn all_1024_normal_portals_fit_without_truncation() {
    let (_, map) = fixture();
    let mut scene = map
        .product()
        .latest_existing()
        .unwrap()
        .unwrap()
        .payload
        .scenegraph
        .clone();
    for n in 0..MAX_ANCHORS {
        let id = format!("00000000-0000-0000-0000-{n:012}");
        scene
            .anchors
            .insert(id.clone(), anchor(&id, &format!("HTTPS://R8.HR/{n:011}")));
    }
    let snapshot = MapSnapshot::new(scene).unwrap();
    let list = MapCatalogData::from_snapshot(&snapshot);
    assert_eq!(list.portals.len(), MAX_ANCHORS);
    list.metadata(u64::MAX).unwrap();
}
#[test]
fn oversized_complete_list_rejects_edit_before_publication() {
    let (runtime, map) = fixture();
    let payload = "x".repeat(4096);
    for n in 0..MAX_ANCHORS {
        let previous = entry(&runtime, &map);
        let snapshot = map.snapshot_reference();
        let result = InMemoryTransport.invoke(
            map.upsert_qr(),
            context(),
            UpsertQr {
                expected_snapshot: snapshot.clone(),
                anchor: anchor(&format!("qr-{n:04}"), &payload),
            },
        );
        if result.is_err() {
            assert!(n > 0);
            assert_eq!(entry(&runtime, &map), previous);
            assert_eq!(map.snapshot_reference(), snapshot);
            assert_eq!(data(&previous).portals.len(), n);
            assert_eq!(
                map.product().latest_existing().unwrap().unwrap().sequence,
                snapshot.sequence
            );
            return;
        }
    }
    panic!("expected metadata size bound");
}

#[path = "../../auki-components/tests/support/clock.rs"]
mod clock_fixture;
use clock_fixture::fixture_clock;
