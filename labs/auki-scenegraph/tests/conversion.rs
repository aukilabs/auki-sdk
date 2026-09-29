use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
use auki_geometry::spatial_transform_to_matrix4;
use auki_scenegraph::*;

fn frame(id: &str, up_axis: UpAxis, meters_per_unit: f64) -> MapFrame {
    MapFrame {
        convention: None,
        id: id.into(),
        handedness: Handedness::Right,
        up_axis,
        meters_per_unit,
        origin_description: "same physical origin; axis rotation supplied explicitly".into(),
    }
}
fn rotation(from: &str, to: &str, q: [f64; 4]) -> RigidTransform {
    RigidTransform {
        from_frame_id: from.into(),
        to_frame_id: to.into(),
        translation: [0.; 3],
        rotation_wxyz: q,
    }
}
fn scene() -> Scenegraph {
    let mut scene = Scenegraph::new(MapDefinition {
        map_id: "map-z".into(),
        name: Some("fixture".into()),
        domain_reference: Some("store".into()),
        frame: frame("z-m", UpAxis::Z, 1.),
    })
    .unwrap();
    scene.anchors.insert(
        "A".into(),
        QrAnchor {
            anchor_id: "A".into(),
            payload: "portal-A".into(),
            side_length_m: 0.4,
            pose_in_map: RigidTransform {
                from_frame_id: "portal-A".into(),
                to_frame_id: "z-m".into(),
                translation: [0.25, -0.5, 1.5],
                rotation_wxyz: [
                    std::f64::consts::FRAC_1_SQRT_2,
                    0.,
                    0.,
                    std::f64::consts::FRAC_1_SQRT_2,
                ],
            },
        },
    );
    scene
}
fn conversion(source: &Scenegraph) -> MapConventionConversion {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    MapConventionConversion::new(
        source.map.frame.clone(),
        frame("y-cm", UpAxis::Y, 0.01),
        rotation("z-m", "y-cm", [h, -h, 0., 0.]),
    )
    .unwrap()
}
fn close(actual: [f64; 3], expected: [f64; 3]) {
    for i in 0..3 {
        assert!(
            (actual[i] - expected[i]).abs() < 1e-9,
            "{actual:?} != {expected:?}"
        );
    }
}
fn point(matrix: [[f64; 4]; 4], p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| matrix[i][3] + (0..3).map(|j| matrix[i][j] * p[j]).sum::<f64>())
}
fn pose_matrix(pose: &RigidTransform) -> [[f64; 4]; 4] {
    let [x, y, z] = pose.translation;
    let [w, qx, qy, qz] = pose.rotation_wxyz;
    spatial_transform_to_matrix4(&SpatialTransform {
        translation: Some(Vec3 { x, y, z }),
        orientation: Some(Quat {
            w,
            x: qx,
            y: qy,
            z: qz,
        }),
    })
    .unwrap()
}

#[test]
fn changes_positions_orientation_and_mesh_units_without_changing_physical_portal() {
    let original = scene();
    let before = original.clone();
    let c = conversion(&original);
    let converted = c.convert_scenegraph(&original, "map-y").unwrap();
    let a = &converted.anchors["A"];
    assert_eq!(original, before);
    assert_eq!(converted.map.name, original.map.name);
    assert_eq!(
        converted.map.domain_reference,
        original.map.domain_reference
    );
    assert_eq!(a.anchor_id, "A");
    assert_eq!(a.payload, "portal-A");
    assert_eq!(a.side_length_m, 0.4);
    assert_eq!(a.pose_in_map.from_frame_id, "portal-A");
    assert_eq!(a.pose_in_map.to_frame_id, "y-cm");
    close(a.pose_in_map.translation, [25., 150., 50.]);
    let m = pose_matrix(&a.pose_in_map);
    // Independent geometry: original Rz(+90) maps local (u,v,0) to (-v,u,0).
    // Explicit Rx(-90) then maps world (x,y,z) to (x,z,-y), in centimeters.
    for [u, v] in [[-0.2, -0.2], [0.2, -0.2], [0.2, 0.2], [-0.2, 0.2]] {
        close(
            point(m, [u * 100., v * 100., 0.]),
            [(0.25 - v) * 100., 150., (0.5 - u) * 100.],
        );
    }
    let t = c.coordinate_transform();
    assert_eq!(t.from_frame_id, "z-m");
    assert_eq!(t.to_frame_id, "y-cm");
    close(point(t.matrix, [0.25, -0.5, 1.5]), [25., 150., 50.]);
    let usd = converted.to_usda_with_portal_geometry().unwrap();
    assert!(usd.contains("upAxis = \"Y\""));
    assert!(usd.contains("metersPerUnit = 0.01"));
    assert!(usd.contains("(-20, -20, 0)")); // 40cm square = 40 scene units.
}

#[test]
fn round_trip_preserves_portal_frames_sizes_and_poses() {
    let source = scene();
    let original = MapSnapshot::new(source.clone()).unwrap();
    let c = conversion(&source);
    let converted = c.convert_snapshot(&original, "map-y").unwrap();
    converted.validate().unwrap();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let inverse = MapConventionConversion::new(
        c.target_frame().clone(),
        c.source_frame().clone(),
        rotation("y-cm", "z-m", [h, h, 0., 0.]),
    )
    .unwrap();
    let restored = inverse.convert_snapshot(&converted, "map-z").unwrap();
    assert_eq!(restored.scenegraph.map, source.map);
    let a = &restored.scenegraph.anchors["A"];
    assert_eq!(a.side_length_m, 0.4);
    assert_eq!(
        a.pose_in_map.from_frame_id,
        source.anchors["A"].pose_in_map.from_frame_id
    );
    let expected = pose_matrix(&source.anchors["A"].pose_in_map);
    let actual = pose_matrix(&a.pose_in_map);
    for i in 0..4 {
        for j in 0..4 {
            assert!((actual[i][j] - expected[i][j]).abs() < 1e-9);
        }
    }
}

#[test]
fn unit_only_and_empty_map_conversions_work_without_components() {
    let mut source = scene();
    let c = MapConventionConversion::new(
        source.map.frame.clone(),
        frame("z-mm", UpAxis::Z, 0.001),
        rotation("z-m", "z-mm", [1., 0., 0., 0.]),
    )
    .unwrap();
    let converted = c.convert_scenegraph(&source, "map-mm").unwrap();
    close(
        converted.anchors["A"].pose_in_map.translation,
        [250., -500., 1500.],
    );
    assert_eq!(converted.anchors["A"].side_length_m, 0.4);
    source.anchors.clear();
    assert!(
        c.convert_scenegraph(&source, "empty-mm")
            .unwrap()
            .anchors
            .is_empty()
    );
}

#[test]
fn rejects_inference_mislabelled_axes_invalid_scales_and_stale_sources() {
    let source = scene();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    for kind in 0..9 {
        let mut target = frame("y-cm", UpAxis::Y, 0.01);
        let mut r = rotation("z-m", "y-cm", [h, -h, 0., 0.]);
        match kind {
            0 => r.to_frame_id = "wrong".into(),
            1 => r.from_frame_id = "wrong".into(),
            2 => r.translation = [1., 0., 0.],
            3 => r.rotation_wxyz = [1., 0., 0., 0.], // Does not map Z-up to Y-up.
            4 => r.rotation_wxyz = [2., 0., 0., 0.],
            5 => r.rotation_wxyz[0] = f64::NAN,
            6 => target.meters_per_unit = 0.,
            7 => target.meters_per_unit = f64::INFINITY,
            _ => target.id = "z-m".into(),
        }
        assert!(MapConventionConversion::new(source.map.frame.clone(), target, r).is_err());
    }
    let c = conversion(&source);
    assert!(c.convert_scenegraph(&source, "map-z").is_err());
    assert!(c.convert_scenegraph(&source, "").is_err());
    let mut stale = source.clone();
    stale.map.frame.meters_per_unit = 0.1;
    assert!(c.convert_scenegraph(&stale, "converted").is_err());
    let mut snapshot = MapSnapshot::new(source).unwrap();
    snapshot.usda.push_str("# stale");
    assert!(c.convert_snapshot(&snapshot, "converted").is_err());
}

#[test]
fn rejects_numeric_overflow_without_mutating_the_source() {
    let mut source = scene();
    source.anchors.get_mut("A").unwrap().pose_in_map.translation[0] = f64::MAX;
    let before = source.clone();
    assert!(
        conversion(&source)
            .convert_scenegraph(&source, "converted")
            .is_err()
    );
    assert_eq!(source, before);
    for (from, to) in [(1e308, 1e-308), (1e-308, 1e308)] {
        assert!(
            MapConventionConversion::new(
                frame("a", UpAxis::Z, from),
                frame("b", UpAxis::Z, to),
                rotation("a", "b", [1., 0., 0., 0.])
            )
            .is_err()
        );
    }
}

#[cfg(feature = "components")]
#[test]
fn converted_partial_map_aligns_and_merges_through_its_shared_portal() {
    use auki_components::ProductReference;
    use auki_geometry::compose_spatial_transforms;
    use auki_scenegraph::{alignment::*, catalog::MapCatalogData, component::SnapshotReference};
    let id = |n| uuid::Uuid::from_u128(n).to_string();
    let reference = |key: &str| SnapshotReference {
        product: ProductReference {
            peer_id: "fixture-peer".into(),
            product_id: key.into(),
            manifest_hash: format!("fixture-{key}"),
        },
        sequence: 0,
    };
    let snapshot = |name: &str, frame: MapFrame, anchors: Vec<(u128, [f64; 3], [f64; 4])>| {
        let mut s = Scenegraph::new(MapDefinition {
            map_id: name.into(),
            name: None,
            domain_reference: Some("store".into()),
            frame,
        })
        .unwrap();
        for (n, translation, rotation_wxyz) in anchors {
            s.anchors.insert(
                id(n),
                QrAnchor {
                    anchor_id: id(n),
                    payload: id(n),
                    side_length_m: 0.4,
                    pose_in_map: RigidTransform {
                        from_frame_id: format!("portal-{n}"),
                        to_frame_id: s.map.frame.id.clone(),
                        translation,
                        rotation_wxyz,
                    },
                },
            );
        }
        MapSnapshot::new(s).unwrap()
    };
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let ab = snapshot(
        "AB",
        frame("ab-z", UpAxis::Z, 1.),
        vec![
            (1, [0.; 3], [1., 0., 0., 0.]),
            (2, [2., 1., 3.], [1., 0., 0., 0.]),
        ],
    );
    let bc = snapshot(
        "BC",
        frame("bc-y", UpAxis::Y, 0.01),
        vec![
            (2, [100., 300., -200.], [h, -h, 0., 0.]),
            (3, [200., 600., -500.], [0.5, -0.5, 0.5, 0.5]),
        ],
    );
    let original_bc = bc.clone();
    let mut checker = MapAlignmentChecker::new(AlignmentOptions::default()).unwrap();
    checker
        .receive_snapshot("ab", reference("ab"), ab.clone())
        .unwrap();
    checker
        .receive_snapshot("bc", reference("bc"), bc.clone())
        .unwrap();
    assert!(matches!(
        checker.check("bc", "ab"),
        AlignmentResult::PotentialConnection { .. }
    ));

    let conversion = MapConventionConversion::new(
        bc.scenegraph.map.frame.clone(),
        frame("bc-z", UpAxis::Z, 1.),
        rotation("bc-y", "bc-z", [h, h, 0., 0.]),
    )
    .unwrap();
    let normalized = conversion.convert_snapshot(&bc, "BC-z-meters").unwrap();
    checker
        .receive_snapshot("normalized", reference("normalized"), normalized.clone())
        .unwrap();
    let AlignmentResult::Available { transform, .. } = checker.check("normalized", "ab") else {
        panic!("converted map must align")
    };
    assert_eq!(transform.from_frame_id, "bc-z");
    assert_eq!(transform.to_frame_id, "ab-z");
    close(transform.translation, [1., -1., 0.]);
    // The host chooses to merge; conversion itself never performs alignment or writes.
    let mut merged = ab.scenegraph.clone();
    merged.map.map_id = "ABC".into();
    for mut anchor in normalized.scenegraph.anchors.into_values() {
        let numeric = |p: &RigidTransform| {
            let [x, y, z] = p.translation;
            let [w, qx, qy, qz] = p.rotation_wxyz;
            SpatialTransform {
                translation: Some(Vec3 { x, y, z }),
                orientation: Some(Quat {
                    w,
                    x: qx,
                    y: qy,
                    z: qz,
                }),
            }
        };
        let pose = compose_spatial_transforms(&numeric(&anchor.pose_in_map), &numeric(&transform))
            .unwrap();
        let t = pose.translation.unwrap();
        let q = pose.orientation.unwrap();
        anchor.pose_in_map.translation = [t.x, t.y, t.z];
        anchor.pose_in_map.rotation_wxyz = [q.w, q.x, q.y, q.z];
        anchor.pose_in_map.to_frame_id = "ab-z".into();
        if let Some(existing) = merged.anchors.get(&anchor.anchor_id) {
            close(
                anchor.pose_in_map.translation,
                existing.pose_in_map.translation,
            );
            let a = pose_matrix(&anchor.pose_in_map);
            let b = pose_matrix(&existing.pose_in_map);
            for i in 0..3 {
                for j in 0..3 {
                    assert!((a[i][j] - b[i][j]).abs() < 1e-9);
                }
            }
        } else {
            merged.anchors.insert(anchor.anchor_id.clone(), anchor);
        }
    }
    let merged = MapSnapshot::new(merged).unwrap();
    close(
        merged.scenegraph.anchors[&id(3)].pose_in_map.translation,
        [3., 4., 6.],
    );
    assert_eq!(merged.scenegraph.anchors.len(), 3);
    assert_eq!(bc, original_bc);
    let catalog = MapCatalogData::from_snapshot(&merged);
    assert_eq!(
        catalog
            .portals
            .iter()
            .map(|p| p.anchor_id.clone())
            .collect::<Vec<_>>(),
        vec![id(1), id(2), id(3)]
    );
}

#[test]
fn derives_conversion_from_complete_registry_conventions_and_rejects_disagreement() {
    use auki_registry::{AxisConvention, AxisDirection::*, FrameRegistryEntry, LengthUnit};
    let scene = scene();
    let from = FrameRegistryEntry {
        peer_id: "peer".into(),
        frame_id: "z-m".into(),
        handedness: auki_registry::Handedness::Right,
        axes: AxisConvention {
            x: Right,
            y: Forward,
            z: Up,
        },
        units: LengthUnit::Meters,
    };
    let to = FrameRegistryEntry {
        peer_id: "peer".into(),
        frame_id: "y-cm".into(),
        handedness: auki_registry::Handedness::Right,
        axes: AxisConvention {
            x: Right,
            y: Up,
            z: Backward,
        },
        units: LengthUnit::Centimeters,
    };
    let target = frame("y-cm", UpAxis::Y, 0.01);
    let c = MapConventionConversion::from_registry_frames(
        scene.map.frame.clone(),
        target.clone(),
        &from,
        &to,
    )
    .unwrap();
    close(
        c.convert_scenegraph(&scene, "converted").unwrap().anchors["A"]
            .pose_in_map
            .translation,
        [25., 150., 50.],
    );
    for kind in 0..5 {
        let mut bad = to.clone();
        match kind {
            0 => bad.frame_id = "different-frame".into(),
            1 => bad.axes.y = Down,
            2 => bad.units = LengthUnit::Meters,
            3 => bad.handedness = auki_registry::Handedness::Left,
            _ => bad.axes.z = Forward, // Reflection falsely labelled right-handed.
        }
        assert!(
            MapConventionConversion::from_registry_frames(
                scene.map.frame.clone(),
                target.clone(),
                &from,
                &bad
            )
            .is_err()
        );
    }
}
