use auki_qr_localizer::*;
use auki_scenegraph::*;

fn estimate(
    scene: &Scenegraph,
    id: &str,
    corners: [[f64; 2]; 4],
    cal: &Calibration,
    gate: QualityGate,
) -> Result<CameraPoseEstimate, LocalizationError> {
    let anchor = scene
        .anchors
        .get(id)
        .ok_or(LocalizationError::UnknownAnchor)?;
    estimate_camera_pose(&scene.map, anchor, corners, cal, gate)
}
fn calibration() -> Calibration {
    // Reference is opaque to the geometry API; the live adapter checks it against provenance.
    serde_json::from_value(serde_json::json!({
        "id":"cal-1", "camera_product":{"peer_id":"p", "product_id":"camera", "manifest_hash":"camera-hash"},
        "camera_frame_id":"optical", "clock_id":"clock", "width":640,"height":480,
        "fx":500.,"fy":500.,"cx":320.,"cy":240.,"distortion":{"model":"none"}
    })).unwrap()
}
fn scene(scale: f64) -> Scenegraph {
    let mut frame = MapFrame::z_up_meters("map", "chosen marker origin");
    frame.meters_per_unit = scale;
    let mut scene = Scenegraph::new(MapDefinition {
        map_id: "m".into(),
        name: None,
        domain_reference: None,
        frame,
    })
    .unwrap();
    scene.anchors.insert(
        "qr".into(),
        QrAnchor {
            anchor_id: "qr".into(),
            payload: "qr".into(),
            side_length_m: 0.2,
            pose_in_map: RigidTransform::identity("qr-frame", "map"),
        },
    );
    scene
}
#[test]
fn optical_camera_pose_composes_anchor_rotation_and_map_units() {
    for scale in [1.0, 0.01] {
        let mut map = scene(scale);
        map.map.frame.up_axis = UpAxis::Y;
        map.anchors.get_mut("qr").unwrap().pose_in_map = RigidTransform {
            from_frame_id: "qr-frame".into(),
            to_frame_id: "map".into(),
            translation: [1., 2., 3.],
            rotation_wxyz: [0.5, 0.5, 0.5, 0.5],
        };
        let pose = estimate(
            &map,
            "qr",
            [[270., 190.], [370., 190.], [370., 290.], [270., 290.]],
            &calibration(),
            QualityGate::default(),
        )
        .unwrap();
        // Camera is one meter in front of marker; anchor rotation maps marker Z to map X.
        for (actual, expected) in
            pose.camera_pose_in_map
                .translation
                .into_iter()
                .zip([1. + 1. / scale, 2., 3.])
        {
            assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
        }
        // R(map<-marker) * Rx(pi): wxyz = [-0.5,0.5,0.5,-0.5], up to global sign.
        let dot: f64 = pose
            .camera_pose_in_map
            .rotation_wxyz
            .into_iter()
            .zip([-0.5, 0.5, 0.5, -0.5])
            .map(|(a, b)| a * b)
            .sum();
        assert!((dot.abs() - 1.).abs() < 1e-6);
        assert!(pose.reprojection_rms_px < 1e-6);
    }
}
#[test]
fn oblique_pose_recovers_independent_projected_geometry() {
    let a = 0.3_f64;
    // Marker-to-optical R_y(a) R_x(pi), t=(.04,-.03,1.2).
    let corners = [[-0.1, 0.1], [0.1, 0.1], [0.1, -0.1], [-0.1, -0.1]].map(|[x, y]| {
        let z = -a.sin() * x + 1.2;
        [
            500. * (a.cos() * x + 0.04) / z + 320.,
            500. * (-y - 0.03) / z + 240.,
        ]
    });
    let pose = estimate(
        &scene(1.),
        "qr",
        corners,
        &calibration(),
        QualityGate::default(),
    )
    .unwrap();
    let expected = [
        -a.cos() * 0.04 + a.sin() * 1.2,
        -0.03,
        a.sin() * 0.04 + a.cos() * 1.2,
    ];
    for (actual, expected) in pose
        .camera_pose_in_map
        .translation
        .into_iter()
        .zip(expected)
    {
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
    }
}
#[test]
fn rejects_unusable_geometry_and_calibration() {
    let map = scene(1.);
    let mut cal = calibration();
    let gate = QualityGate::default();
    let corners = [[270., 190.], [370., 190.], [370., 290.], [270., 290.]];
    assert!(matches!(
        estimate(&map, "absent", corners, &cal, gate),
        Err(LocalizationError::UnknownAnchor)
    ));
    for invalid in [
        [[0., 0.]; 4],
        [[270., 190.], [270., 290.], [370., 290.], [370., 190.]],
        [[f64::NAN, 0.]; 4],
        [[-1., 0.], [20., 0.], [20., 20.], [-1., 20.]],
    ] {
        assert!(estimate(&map, "qr", invalid, &cal, gate).is_err());
    }
    cal.fx = 0.;
    assert!(matches!(
        estimate(&map, "qr", corners, &cal, gate),
        Err(LocalizationError::Calibration)
    ));
    cal = calibration();
    cal.distortion = Distortion::BrownConrady(vec![0.; 3]);
    assert!(estimate(&map, "qr", corners, &cal, gate).is_err());
    assert!(
        estimate(
            &map,
            "qr",
            corners,
            &calibration(),
            QualityGate {
                minimum_area_px2: 20000.,
                ..gate
            }
        )
        .is_err()
    );
}

#[test]
fn distorted_pixels_recover_pose_with_matching_calibration() {
    // Independently apply the five-coefficient Brown-Conrady forward model.
    let mut cal = calibration();
    cal.distortion = Distortion::BrownConrady(vec![0.15, -0.03, 0.002, -0.001, 0.01]);
    let corners = [[-0.1, 0.1], [0.1, 0.1], [0.1, -0.1], [-0.1, -0.1]].map(|[x, y]| {
        let x: f64 = x + 0.25;
        let y: f64 = -y + 0.1;
        let r2 = x * x + y * y;
        let radial = 1. + 0.15 * r2 - 0.03 * r2 * r2 + 0.01 * r2 * r2 * r2;
        [
            500. * (x * radial + 2. * 0.002 * x * y - 0.001 * (r2 + 2. * x * x)) + 320.,
            500. * (y * radial + 0.002 * (r2 + 2. * y * y) - 2. * 0.001 * x * y) + 240.,
        ]
    });
    let result = estimate(&scene(1.), "qr", corners, &cal, QualityGate::default()).unwrap();
    for (a, b) in result
        .camera_pose_in_map
        .translation
        .into_iter()
        .zip([-0.25, 0.1, 1.])
    {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }
}

#[test]
fn inconsistent_quad_fails_strict_reprojection_gate() {
    let result = estimate(
        &scene(1.),
        "qr",
        [[270., 190.], [375., 194.], [370., 290.], [270., 290.]],
        &calibration(),
        QualityGate {
            maximum_reprojection_rms_px: 0.001,
            ..QualityGate::default()
        },
    );
    assert!(result.is_err());
}

#[test]
fn localization_labels_both_frames_and_rejects_wrong_anchor_destination() {
    let mut map = scene(1.0);
    let cal = calibration();
    let corners = [[270., 190.], [370., 190.], [370., 290.], [270., 290.]];
    let pose = estimate(&map, "qr", corners, &cal, QualityGate::default()).unwrap();
    assert_eq!(pose.camera_pose_in_map.from_frame_id, cal.camera_frame_id);
    assert_eq!(pose.camera_pose_in_map.to_frame_id, map.map.frame.id);
    map.anchors.get_mut("qr").unwrap().pose_in_map.to_frame_id = "unrelated".into();
    assert!(estimate(&map, "qr", corners, &cal, QualityGate::default()).is_err());
}
