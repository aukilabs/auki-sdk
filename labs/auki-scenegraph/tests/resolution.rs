#![cfg(feature = "components")]
use auki_components::*;
use auki_scenegraph::{component::*, resolution::*, *};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
fn context() -> InvocationContext {
    InvocationContext {
        invocation_id: "test".into(),
        caller_peer_id: "local".into(),
        caller_component_id: "mapper".into(),
    }
}
fn map(peer: &str, name: &str, allowed: bool) -> Arc<MapComponent> {
    let runtime = ComponentRuntime::new(peer);
    let clock = AtomicU64::new(1);
    Arc::new(
        MapComponent::new(
            &runtime,
            MapComponentConfig {
                component_id: name.into(),
                publication_id: name.into(),
                clock_id: "clock".into(),
                map: MapDefinition {
                    map_id: name.into(),
                    name: None,
                    domain_reference: Some("same-domain".into()),
                    frame: MapFrame::z_up_meters(format!("{name}-frame"), "chosen origin"),
                },
            },
            move || clock.fetch_add(1, Ordering::SeqCst),
            move |_| allowed,
            |_| true,
        )
        .unwrap(),
    )
}
fn insert(map: &MapComponent, id: &str) {
    InMemoryTransport
        .invoke(
            map.upsert_qr(),
            context(),
            UpsertQr {
                expected_snapshot: map.snapshot_reference(),
                anchor: QrAnchor {
                    anchor_id: id.into(),
                    payload: "qr".into(),
                    side_length_m: 0.1,
                    pose_in_map: RigidTransform::identity(
                        format!("{id}-frame"),
                        map.product()
                            .latest_existing()
                            .unwrap()
                            .unwrap()
                            .payload
                            .scenegraph
                            .map
                            .frame
                            .id
                            .clone(),
                    ),
                },
            },
        )
        .unwrap();
}
#[test]
fn local_lookup_follows_updates_and_keeps_multiple_map_frames_separate() {
    let maps = PortalMaps::new(context());
    let a = map("local", "a", true);
    let b = map("local", "b", true);
    maps.register(a.clone()).unwrap();
    maps.register(b.clone()).unwrap();
    assert!(maps.resolve("qr").unwrap().is_empty());
    insert(&a, "one");
    insert(&b, "two");
    let found = maps.resolve("qr").unwrap();
    assert_eq!(found.len(), 2);
    assert_ne!(found[0].map.frame.id, found[1].map.frame.id);
    assert_eq!(found[0].snapshot, a.snapshot_reference());
    maps.unregister(&b);
    assert_eq!(maps.resolve("qr").unwrap().len(), 1);
    insert(&a, "ambiguous");
    assert!(maps.resolve("qr").is_err());
}
#[test]
fn remote_ownership_denial_and_closed_maps_are_not_mistaken_for_absence() {
    let maps = PortalMaps::new(context());
    assert!(maps.register(map("remote", "r", true)).is_err());
    let denied = map("local", "denied", false);
    maps.register(denied.clone()).unwrap();
    assert!(maps.resolve("qr").is_err());
    maps.unregister(&denied);
    let closed = map("local", "closed", true);
    maps.register(closed.clone()).unwrap();
    closed.close();
    assert!(maps.resolve("qr").is_err());
}

#[test]
fn selected_map_resolution_is_exact_and_ignores_unrelated_denied_maps() {
    let maps = PortalMaps::new(context());
    let selected = map("local", "selected", true);
    let denied = map("local", "denied", false);
    maps.register(denied).unwrap();
    maps.register(selected.clone()).unwrap();
    insert(&selected, "one");
    let revision = selected.snapshot_reference();
    let qr = maps.resolve_in("qr", &revision).unwrap();
    assert_eq!(qr.snapshot, revision);
    assert_eq!(qr.map.frame.id, "selected-frame");
    assert!(maps.resolve_in("absent", &revision).is_err());
    let mut wrong = revision.clone();
    wrong.product.manifest_hash = "wrong".into();
    assert!(maps.resolve_in("qr", &wrong).is_err());
    let mut changed = qr.anchor;
    changed.pose_in_map.translation[0] = 1.;
    InMemoryTransport
        .invoke(
            selected.upsert_qr(),
            context(),
            UpsertQr {
                expected_snapshot: revision.clone(),
                anchor: changed,
            },
        )
        .unwrap();
    assert!(maps.resolve_in("qr", &revision).is_err());
    assert!(
        maps.resolve_in("qr", &selected.snapshot_reference())
            .is_ok()
    );
}
