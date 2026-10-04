use auki_components::*;
use auki_odometry::*;
use auki_registry::{CoordinateConvention, FrameRegistryEntry};
use std::time::Duration;

fn contract() -> PoseContract {
    PoseContract {
        from_frame: FrameRegistryEntry::in_convention(
            "robot",
            "base",
            CoordinateConvention::Ros2Body,
        ),
        to_frame: FrameRegistryEntry::in_convention(
            "robot",
            "odom.boot-1",
            CoordinateConvention::Ros2Body,
        ),
        clock: ClockReference {
            peer_id: "robot".into(),
            id: "capture".into(),
            hash: "a".repeat(32),
        },
        session_id: "boot-1".into(),
    }
}
fn valid(x: f64, q: [f64; 4]) -> Tracking {
    Tracking::Valid {
        pose: Pose {
            translation: [x, 0., 0.],
            rotation_xyzw: q,
        },
    }
}
fn query(t: u64) -> PoseQuery {
    PoseQuery {
        contract: contract(),
        timestamp_ns: t,
        max_gap_ns: 100,
    }
}

#[test]
fn history_is_a_standard_product_with_clocked_interpolation_and_provenance() {
    let rt = ComponentRuntime::new("robot");
    let mut odom = OdometryComponent::new(&rt, "odometry", contract()).unwrap();
    let history = capture_pose_history(&rt, "trajectory", &odom, BufferLimits::entries(3)).unwrap();
    odom.publish(100, valid(0., [0., 0., 0., 1.])).unwrap();
    odom.publish(200, valid(2., [0., 0., 1., 0.])).unwrap();
    let p = history.product();
    let result = pose_at(&p, &query(150)).unwrap();
    assert_eq!(result.pose.translation, [1., 0., 0.]);
    assert!((result.pose.rotation_xyzw[2] - 0.5_f64.sqrt()).abs() < 1e-10);
    assert!((result.pose.rotation_xyzw[3] - 0.5_f64.sqrt()).abs() < 1e-10);
    assert_eq!(
        result.derivation,
        Derivation::Interpolated {
            before_sequence: 0,
            after_sequence: 1,
            fraction: 0.5
        }
    );
    assert_eq!(result.product, p.reference());
    assert_eq!(
        pose_at(&p, &query(100)).unwrap().derivation,
        Derivation::Exact { sequence: 0 }
    );
    assert!(matches!(
        pose_at(&p, &query(99)),
        Err(PoseError::OutsideHistory)
    ));
    assert!(matches!(
        pose_at(&p, &query(201)),
        Err(PoseError::OutsideHistory)
    ));
    let mut q = query(150);
    q.max_gap_ns = 99;
    assert!(matches!(pose_at(&p, &q), Err(PoseError::GapTooLarge)));
    for mutate in [0, 1, 2, 3] {
        let mut q = query(150);
        match mutate {
            0 => q.contract.clock.hash = "b".repeat(32),
            1 => q.contract.from_frame.frame_id = "camera".into(),
            2 => q.contract.to_frame.frame_id = "odom.boot-2".into(),
            _ => q.contract.session_id = "boot-2".into(),
        };
        assert!(matches!(pose_at(&p, &q), Err(PoseError::ContractMismatch)));
    }
    let catalog = rt.catalog().snapshot();
    let metadata = catalog.products[0].metadata.as_ref().unwrap();
    assert_eq!(metadata.schema, METADATA_SCHEMA);
    assert_eq!(metadata.source_sequence, 1);
    assert_eq!(
        serde_json::from_value::<PoseContract>(metadata.value.clone()).unwrap(),
        contract()
    );
    assert!(history.errors().is_empty());
}

#[test]
fn loss_eviction_and_session_end_do_not_create_invented_poses() {
    let rt = ComponentRuntime::new("robot");
    let mut odom = OdometryComponent::new(&rt, "odometry", contract()).unwrap();
    let h = capture_pose_history(
        &rt,
        "trajectory",
        &odom,
        BufferLimits {
            max_entries: Some(3),
            max_bytes: None,
            target_duration: Some(Duration::from_nanos(150)),
        },
    )
    .unwrap();
    odom.publish(100, valid(0., [0., 0., 0., 1.])).unwrap();
    odom.publish(200, Tracking::Lost).unwrap();
    odom.publish(300, valid(3., [0., 0., 0., -1.])).unwrap();
    assert_eq!(h.product().buffer().range().entries, 2);
    assert!(matches!(
        pose_at(&h.product(), &query(100)),
        Err(PoseError::OutsideHistory)
    ));
    assert!(matches!(
        pose_at(&h.product(), &query(250)),
        Err(PoseError::TrackingLost)
    ));
    assert!(matches!(
        pose_at(&h.product(), &query(200)),
        Err(PoseError::TrackingLost)
    ));
    assert!(matches!(
        odom.publish(300, Tracking::Lost),
        Err(PoseError::NonMonotonic)
    ));
    assert!(matches!(
        odom.publish(299, Tracking::Lost),
        Err(PoseError::NonMonotonic)
    ));
    odom.publish(350, valid(4., [0., 0., 0., 1.])).unwrap();
    assert_eq!(
        pose_at(&h.product(), &query(325)).unwrap().pose.translation,
        [3.5, 0., 0.]
    );
    odom.end(
        350,
        ObservationEndReason::Reconfigured { replacement: None },
    )
    .unwrap();
    assert!(h.end_notice().is_some());
    assert!(odom.publish(400, Tracking::Lost).is_err());
    assert!(pose_at(&h.product(), &query(350)).is_ok());
}

#[test]
fn invalid_contracts_and_poses_are_rejected_before_publication() {
    let rt = ComponentRuntime::new("robot");
    let mut c = contract();
    c.clock.hash = "missing".into();
    assert!(OdometryComponent::new(&rt, "bad", c).is_err());
    let mut o = OdometryComponent::new(&rt, "odom", contract()).unwrap();
    let h = capture_pose_history(&rt, "history", &o, BufferLimits::entries(2)).unwrap();
    assert!(o.publish(10, valid(f64::NAN, [0., 0., 0., 1.])).is_err());
    assert!(o.publish(10, valid(0., [0.; 4])).is_err());
    assert_eq!(h.product().buffer().range().entries, 0);
    assert_eq!(o.publish(10, Tracking::Lost).unwrap().sequence, 0);
    let mut c = contract();
    c.to_frame.handedness = auki_registry::Handedness::Left;
    assert!(c.validate().is_err());
}
