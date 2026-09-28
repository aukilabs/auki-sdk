#![cfg(feature = "components")]
use auki_components::ProductReference;
use auki_scenegraph::{alignment::*, catalog::MapCatalogData, component::SnapshotReference, *};
fn portal(n: u128) -> String {
    uuid::Uuid::from_u128(n).to_string()
}
fn reference(key: &str, sequence: u64) -> SnapshotReference {
    SnapshotReference {
        product: ProductReference {
            peer_id: format!("peer-{key}"),
            product_id: key.into(),
            manifest_hash: format!("hash-{key}"),
        },
        sequence,
    }
}
fn snapshot(key: &str, anchors: &[(u128, [f64; 3])]) -> MapSnapshot {
    let mut scene = Scenegraph::new(MapDefinition {
        map_id: key.into(),
        name: None,
        domain_reference: Some("same-domain".into()),
        frame: MapFrame::z_up_meters(format!("{key}-frame"), "explicit fixture"),
    })
    .unwrap();
    for (id, position) in anchors {
        scene.anchors.insert(
            portal(*id),
            QrAnchor {
                anchor_id: portal(*id),
                payload: format!("portal:{id}"),
                side_length_m: 0.1,
                pose_in_map: RigidTransform {
                    from_frame_id: format!("{key}-portal-{id}"),
                    to_frame_id: format!("{key}-frame"),
                    translation: *position,
                    rotation_wxyz: [1., 0., 0., 0.],
                },
            },
        );
    }
    MapSnapshot::new(scene).unwrap()
}
fn checker(automatic: bool) -> MapAlignmentChecker {
    MapAlignmentChecker::new(AlignmentOptions {
        automatic,
        ..Default::default()
    })
    .unwrap()
}
#[test]
fn catalog_only_is_potential_and_automatic_updates_notify_transitive_alignment() {
    let a = snapshot("a", &[(1, [0.; 3]), (2, [1., 0., 0.]), (3, [2., 0., 0.])]);
    let b = snapshot("b", &[(3, [0.; 3]), (4, [3., 0., 0.])]);
    let c = snapshot("c", &[(4, [0.; 3]), (5, [4., 0., 0.])]);
    let mut index = checker(true);
    index.receive_snapshot("a", reference("a", 1), a).unwrap();
    index.receive_snapshot("c", reference("c", 1), c).unwrap();
    assert_eq!(index.check("a", "c"), AlignmentResult::NoConnection);
    let events = index
        .receive_catalog("b", reference("b", 1), MapCatalogData::from_snapshot(&b))
        .unwrap();
    assert!(events.iter().any(|e| e.source_map == "a"
        && e.target_map == "c"
        && matches!(e.result, AlignmentResult::PotentialConnection { .. })));
    let events = index
        .receive_snapshot("b", reference("b", 1), b.clone())
        .unwrap();
    assert!(events.iter().any(|e| e.source_map == "a"
        && e.target_map == "c"
        && matches!(e.result, AlignmentResult::Available { .. })));
    let AlignmentResult::Available { transform, path } = index.check("c", "a") else {
        panic!("missing transitive alignment")
    };
    assert_eq!(transform.from_frame_id, "c-frame");
    assert_eq!(transform.to_frame_id, "a-frame");
    assert_eq!(transform.translation, [5., 0., 0.]);
    assert_eq!(path.len(), 2);
    assert_eq!(path[0].portal_ids, vec![portal(4)]);
    assert_eq!(path[1].portal_ids, vec![portal(3)]);
    assert_eq!(path[0].from_snapshot, reference("c", 1));
    assert!(
        index
            .receive_snapshot("b", reference("b", 1), b)
            .unwrap()
            .is_empty()
    );
    let events = index.remove("b");
    assert!(events.iter().any(|e| e.source_map == "a"
        && e.target_map == "c"
        && e.result == AlignmentResult::NoConnection));
}
#[test]
fn manual_mode_stale_revisions_and_catalog_updates_cannot_reuse_old_poses() {
    let a = snapshot("a", &[(1, [0.; 3])]);
    let b = snapshot("b", &[(1, [2., 0., 0.])]);
    let mut index = checker(false);
    assert!(
        index
            .receive_snapshot("a", reference("a", 2), a.clone())
            .unwrap()
            .is_empty()
    );
    assert!(
        index
            .receive_snapshot("b", reference("b", 2), b.clone())
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        index.check("a", "b"),
        AlignmentResult::Available { .. }
    ));
    assert_eq!(index.check_all().len(), 1);
    assert!(index.check_all().is_empty());
    assert!(index.receive_snapshot("a", reference("a", 1), a).is_err());
    let changed = snapshot("b", &[(1, [3., 0., 0.])]);
    assert!(
        index
            .receive_snapshot("b", reference("b", 2), changed.clone())
            .is_err()
    );
    index
        .receive_catalog(
            "b",
            reference("b", 3),
            MapCatalogData::from_snapshot(&changed),
        )
        .unwrap();
    assert!(matches!(
        index.check("a", "b"),
        AlignmentResult::PotentialConnection { .. }
    ));
    index
        .receive_snapshot("b", reference("b", 3), changed)
        .unwrap();
    let AlignmentResult::Available { transform, .. } = index.check("a", "b") else {
        panic!()
    };
    assert_eq!(transform.translation, [3., 0., 0.]);
}
#[test]
fn inconsistent_shared_portals_and_cycles_are_conflicts() {
    let mut index = checker(false);
    index
        .receive_snapshot(
            "a",
            reference("a", 1),
            snapshot("a", &[(1, [0.; 3]), (2, [1., 0., 0.])]),
        )
        .unwrap();
    index
        .receive_snapshot(
            "b",
            reference("b", 1),
            snapshot("b", &[(1, [0.; 3]), (2, [9., 0., 0.])]),
        )
        .unwrap();
    assert!(matches!(
        index.check("a", "b"),
        AlignmentResult::Conflict { .. }
    ));
    let mut index = checker(false);
    // Each pair shares one different Portal, but the loop fails to close.
    for (key, anchors) in [
        ("a", vec![(1, [0.; 3]), (3, [0.; 3])]),
        ("b", vec![(1, [0.; 3]), (2, [0.; 3])]),
        ("c", vec![(2, [0.; 3]), (3, [1., 0., 0.])]),
    ] {
        index
            .receive_snapshot(key, reference(key, 1), snapshot(key, &anchors))
            .unwrap();
    }
    assert!(matches!(
        index.check("a", "c"),
        AlignmentResult::Conflict { .. }
    ));
}
#[test]
fn domain_membership_does_not_align_and_conventions_require_explicit_conversion() {
    let mut index = checker(false);
    index
        .receive_snapshot("a", reference("a", 1), snapshot("a", &[(1, [0.; 3])]))
        .unwrap();
    index
        .receive_snapshot("d", reference("d", 1), snapshot("d", &[(4, [0.; 3])]))
        .unwrap();
    assert_eq!(index.check("a", "d"), AlignmentResult::NoConnection);
    let mut b = snapshot("b", &[(1, [0.; 3])]).scenegraph;
    b.map.frame.meters_per_unit = 0.01;
    index
        .receive_snapshot("b", reference("b", 1), MapSnapshot::new(b).unwrap())
        .unwrap();
    assert!(matches!(
        index.check("a", "b"),
        AlignmentResult::PotentialConnection { .. }
    ));
}
#[test]
fn rotation_and_translation_have_the_correct_direction() {
    let a = snapshot("a", &[(2, [2., 3., 1.])]);
    let mut scene = a.scenegraph;
    let h = std::f64::consts::FRAC_1_SQRT_2;
    scene
        .anchors
        .get_mut(&portal(2))
        .unwrap()
        .pose_in_map
        .rotation_wxyz = [h, 0., 0., h];
    let mut index = checker(false);
    index
        .receive_snapshot("a", reference("a", 1), MapSnapshot::new(scene).unwrap())
        .unwrap();
    index
        .receive_snapshot("b", reference("b", 1), snapshot("b", &[(2, [0.; 3])]))
        .unwrap();
    let AlignmentResult::Available { transform, path } = index.check("b", "a") else {
        panic!()
    };
    for (actual, expected) in transform.translation.iter().zip([2., 3., 1.]) {
        assert!((actual - expected).abs() < 1e-10);
    }
    let dot: f64 = transform
        .rotation_wxyz
        .iter()
        .zip([h, 0., 0., h])
        .map(|(a, b)| a * b)
        .sum();
    assert!((dot.abs() - 1.).abs() < 1e-10);
    assert_eq!(path[0].from_snapshot, reference("b", 1));
    assert_eq!(transform.from_frame_id, "b-frame");
    assert_eq!(transform.to_frame_id, "a-frame");
}
#[test]
fn invalid_metadata_and_mismatched_sizes_are_rejected_or_reported() {
    let mut index = checker(false);
    let a = snapshot("a", &[(1, [0.; 3])]);
    let mut data = MapCatalogData::from_snapshot(&a);
    data.portals.push(data.portals[0].clone());
    assert!(index.receive_catalog("a", reference("a", 1), data).is_err());
    index.receive_snapshot("a", reference("a", 1), a).unwrap();
    let mut b = snapshot("b", &[(1, [0.; 3])]).scenegraph;
    b.anchors.get_mut(&portal(1)).unwrap().side_length_m = 0.2;
    index
        .receive_snapshot("b", reference("b", 1), MapSnapshot::new(b).unwrap())
        .unwrap();
    assert!(matches!(
        index.check("a", "b"),
        AlignmentResult::Conflict { .. }
    ));
    assert!(
        MapAlignmentChecker::new(AlignmentOptions {
            translation_tolerance_m: f64::NAN,
            ..Default::default()
        })
        .is_err()
    );
}
