use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
use auki_geometry::spatial_transform_to_matrix4;
use auki_scenegraph::*;

fn scene() -> Scenegraph {
    let frame = MapFrame::in_convention(
        "gl-frame",
        "fixture origin",
        CoordinateConvention::OpenGl,
        1.,
    )
    .unwrap();
    let mut s = Scenegraph::new(MapDefinition {
        map_id: "gl-map".into(),
        name: Some("fixture".into()),
        domain_reference: Some("store".into()),
        frame,
    })
    .unwrap();
    for (id, t) in [("A", [1., 2., 3.]), ("B", [-4., 5., -6.])] {
        s.anchors.insert(
            id.into(),
            QrAnchor {
                anchor_id: id.into(),
                payload: format!("portal-{id}"),
                side_length_m: 0.4,
                pose_in_map: RigidTransform {
                    from_frame_id: format!("portal-{id}-frame"),
                    to_frame_id: "gl-frame".into(),
                    translation: t,
                    rotation_wxyz: [1., 0., 0., 0.],
                },
            },
        );
    }
    s
}
fn target() -> MapDuplicateTarget {
    MapDuplicateTarget::new("ros-map", "ros-frame", CoordinateConvention::Ros2Body)
}
fn close(a: [f64; 3], b: [f64; 3]) {
    for i in 0..3 {
        assert!((a[i] - b[i]).abs() < 1e-9, "{a:?} != {b:?}");
    }
}
fn matrix(p: &RigidTransform) -> [[f64; 4]; 4] {
    let [x, y, z] = p.translation;
    let [w, qx, qy, qz] = p.rotation_wxyz;
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
fn duplicates_opengl_to_ros_body_without_caller_supplied_rotation() {
    let s = scene();
    let before = s.clone();
    let copy = s.duplicate_in_convention(&target()).unwrap();
    assert_eq!(s, before);
    assert_eq!(
        copy.map.frame.convention,
        Some(CoordinateConvention::Ros2Body)
    );
    assert_eq!(copy.map.frame.up_axis, UpAxis::Z);
    assert_eq!(copy.map.domain_reference, s.map.domain_reference);
    assert_eq!(copy.map.name, s.map.name);
    assert_eq!(copy.anchors.len(), 2);
    // Independent coordinate relation: GL right/up/back -> ROS forward/left/up.
    // (x,y,z) becomes (-z,-x,y).
    close(copy.anchors["A"].pose_in_map.translation, [-3., -1., 2.]);
    close(copy.anchors["B"].pose_in_map.translation, [6., 4., 5.]);
    let a = &copy.anchors["A"];
    assert_eq!(a.pose_in_map.from_frame_id, "portal-A-frame");
    assert_eq!(a.pose_in_map.to_frame_id, "ros-frame");
    assert_eq!(a.side_length_m, 0.4);
    assert_eq!(a.payload, "portal-A");
    let m = matrix(&a.pose_in_map);
    for (column, expected) in [[0., -1., 0.], [0., 0., 1.], [-1., 0., 0.]]
        .into_iter()
        .enumerate()
    {
        close([m[0][column], m[1][column], m[2][column]], expected);
    }
}
#[test]
fn selected_duplicate_keeps_origin_and_snapshot_contains_only_selected_portals() {
    let source = MapSnapshot::new(scene()).unwrap();
    let selected = source
        .duplicate_part_in_convention(&["B"], &target())
        .unwrap();
    selected.validate().unwrap();
    assert_eq!(
        selected
            .scenegraph
            .anchors
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["B"]
    );
    close(
        selected.scenegraph.anchors["B"].pose_in_map.translation,
        [6., 4., 5.],
    ); // Not rebased to zero.
    assert!(
        selected
            .usda
            .contains("auki:coordinateConvention = \"ros2_body\"")
    );
    assert!(selected.usda.contains("auki:anchorId = \"B\""));
    assert!(!selected.usda.contains("auki:anchorId = \"A\""));
    assert_eq!(source.scenegraph.anchors.len(), 2);
    let json = serde_json::to_string(&selected).unwrap();
    let restored: MapSnapshot = serde_json::from_str(&json).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, selected);
    let empty = source.duplicate_part_in_convention(&[], &target()).unwrap();
    assert!(empty.scenegraph.anchors.is_empty());
    empty.validate().unwrap();
}
#[test]
fn named_duplication_scales_units_and_round_trips_orientation() {
    let source = scene();
    let mut cm = target();
    cm.meters_per_unit = 0.01;
    let ros = source.duplicate_in_convention(&cm).unwrap();
    close(
        ros.anchors["A"].pose_in_map.translation,
        [-300., -100., 200.],
    );
    assert_eq!(ros.anchors["A"].side_length_m, 0.4);
    let gl = ros
        .duplicate_in_convention(&MapDuplicateTarget::new(
            "gl-copy",
            "gl-copy-frame",
            CoordinateConvention::OpenGl,
        ))
        .unwrap();
    for id in ["A", "B"] {
        close(
            gl.anchors[id].pose_in_map.translation,
            source.anchors[id].pose_in_map.translation,
        );
        let a = matrix(&gl.anchors[id].pose_in_map);
        let b = matrix(&source.anchors[id].pose_in_map);
        for i in 0..3 {
            for j in 0..3 {
                assert!((a[i][j] - b[i][j]).abs() < 1e-9);
            }
        }
    }
    // Same-convention duplication is also a fresh artifact, with explicit identities.
    let copy = source
        .duplicate_in_convention(&MapDuplicateTarget::new(
            "gl-copy",
            "gl-copy-frame",
            CoordinateConvention::OpenGl,
        ))
        .unwrap();
    close(copy.anchors["A"].pose_in_map.translation, [1., 2., 3.]);
}
#[test]
fn refuses_unknown_source_convention_and_invalid_named_frame_metadata() {
    let mut s = scene();
    s.map.frame.convention = None;
    let legacy = MapSnapshot::new(s.clone()).unwrap();
    assert!(
        !serde_json::to_string(&legacy)
            .unwrap()
            .contains("\"convention\"")
    );
    assert!(legacy.duplicate_in_convention(&target()).is_err());
    s.map.frame.convention = Some(CoordinateConvention::Ros2Body); // Still declares Y-up.
    assert!(s.validate().is_err());
    let mut json = serde_json::to_value(scene()).unwrap();
    json["map"]["frame"]["convention"] = serde_json::json!("guess_ros");
    assert!(serde_json::from_value::<Scenegraph>(json).is_err());
}
#[test]
fn invalid_selections_targets_and_unsupported_conventions_leave_source_unchanged() {
    let s = scene();
    let before = s.clone();
    for ids in [&["unknown"][..], &["A", "A"][..], &["A", "unknown"][..]] {
        assert!(s.duplicate_part_in_convention(ids, &target()).is_err());
    }
    for kind in 0..7 {
        let mut t = target();
        match kind {
            0 => t.map_id = "gl-map".into(),
            1 => t.frame_id = "gl-frame".into(),
            2 => t.frame_id = "portal-A-frame".into(),
            3 => t.meters_per_unit = 0.,
            4 => t.meters_per_unit = f64::NAN,
            5 => t.convention = CoordinateConvention::Unity,
            _ => t.convention = CoordinateConvention::Ros2Optical,
        }
        assert!(s.duplicate_in_convention(&t).is_err());
    }
    assert_eq!(s, before);
    let mut bad = MapSnapshot::new(s).unwrap();
    bad.usda.push_str("# stale");
    assert!(bad.duplicate_part_in_convention(&["A"], &target()).is_err());
}
#[test]
fn explicit_conversion_cannot_contradict_named_heading_even_when_up_axes_match() {
    let source = scene().map.frame;
    let target = MapFrame::in_convention(
        "ros-frame",
        "same origin",
        CoordinateConvention::Ros2Body,
        1.,
    )
    .unwrap();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let wrong = RigidTransform {
        from_frame_id: source.id.clone(),
        to_frame_id: target.id.clone(),
        translation: [0.; 3],
        rotation_wxyz: [h, h, 0., 0.],
    };
    // Rx(+90) maps Y-up to Z-up, but fails the ROS forward/left axis declaration.
    assert!(MapConventionConversion::new(source, target, wrong).is_err());
}
