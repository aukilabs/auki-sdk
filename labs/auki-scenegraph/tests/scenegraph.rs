use auki_scenegraph::*;

fn scene() -> Scenegraph {
    Scenegraph::new(MapDefinition {
        map_id: "store/qr-map".into(),
        name: None,
        domain_reference: Some("supermarket".into()),
        frame: MapFrame::z_up_meters("map-frame", "QR origin; heading explicitly chosen"),
    })
    .unwrap()
}

fn anchor() -> QrAnchor {
    QrAnchor {
        anchor_id: "marker/one".into(),
        payload: "https://example.invalid/\"qr\"\\one".into(),
        side_length_m: 0.2,
        pose_in_map: RigidTransform::identity("marker-one-frame", "map-frame"),
    }
}

#[test]
fn usd_export_preserves_ids_convention_pose_and_escaped_content() {
    let mut scene = scene();
    let mut qr = anchor();
    qr.pose_in_map.translation = [1.0, 2.0, 3.0];
    qr.pose_in_map.rotation_wxyz = [0.5, 0.5, 0.5, 0.5];
    scene.anchors.insert(qr.anchor_id.clone(), qr);
    let snapshot = MapSnapshot::new(scene).unwrap();
    snapshot.validate().unwrap();
    assert!(snapshot.usda.contains("upAxis = \"Z\""));
    assert!(snapshot.usda.contains("metersPerUnit = 1"));
    assert!(snapshot.usda.contains("xformOp:translate = (1, 2, 3)"));
    assert!(
        snapshot
            .usda
            .contains("xformOp:orient = (0.5, 0.5, 0.5, 0.5)")
    );
    assert!(snapshot.usda.contains("auki:anchorId = \"marker/one\""));
    assert!(
        snapshot
            .usda
            .contains("def Xform \"QR_6d61726b65722f6f6e65\"")
    );
    let encoded = serde_json::to_vec(&snapshot).unwrap();
    let decoded: MapSnapshot = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, snapshot);
    let mut tampered = decoded;
    tampered.usda.push_str("\n# edited independently");
    assert!(tampered.validate().is_err());
}

#[test]
fn invalid_geometry_is_rejected_and_other_usd_conventions_are_explicit() {
    let mut qr = anchor();
    for size in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        qr.side_length_m = size;
        assert!(qr.validate().is_err());
    }
    qr = anchor();
    qr.pose_in_map.rotation_wxyz = [0.0; 4];
    assert!(qr.validate().is_err());
    qr = anchor();
    qr.pose_in_map.translation[0] = f64::NAN;
    assert!(qr.validate().is_err());
    let mut scene = scene();
    scene.map.frame.up_axis = UpAxis::Y;
    scene.map.frame.meters_per_unit = 0.01;
    assert!(scene.to_usda().unwrap().contains("upAxis = \"Y\""));
    assert!(scene.to_usda().unwrap().contains("metersPerUnit = 0.01"));
    scene.anchors.insert("wrong-key".into(), anchor());
    assert!(scene.validate().is_err());
}

#[cfg(feature = "components")]
mod runtime {
    use super::*;
    use auki_components::{
        ComponentRuntime, InMemoryTransport, InvocationContext, InvocationError,
    };
    use auki_scenegraph::component::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    fn context() -> InvocationContext {
        InvocationContext {
            invocation_id: "invocation".into(),
            caller_peer_id: "editor".into(),
            caller_component_id: "mapper".into(),
        }
    }
    fn map(clock: Arc<AtomicU64>) -> MapComponent {
        let runtime = ComponentRuntime::new("owner");
        MapComponent::new(
            &runtime,
            MapComponentConfig {
                component_id: "map".into(),
                publication_id: "run-1".into(),
                clock_id: "clock".into(),
                map: scene().map,
            },
            move || clock.load(Ordering::SeqCst),
            |_| true,
            |c| c.caller_peer_id == "editor" && c.caller_component_id == "mapper",
        )
        .unwrap()
    }
    #[test]
    fn writes_are_authorized_conditional_and_lookup_matches_retained_snapshot() {
        let clock = Arc::new(AtomicU64::new(1));
        let map = map(clock.clone());
        let initial = map.snapshot_reference();
        let request = UpsertQr {
            expected_snapshot: initial.clone(),
            anchor: anchor(),
        };
        let mut denied = context();
        denied.caller_peer_id = "stranger".into();
        assert!(matches!(
            InMemoryTransport.invoke(map.upsert_qr(), denied, request.clone()),
            Err(InvocationError::Unauthorized)
        ));
        assert_eq!(map.snapshot_reference(), initial);
        clock.store(2, Ordering::SeqCst);
        let applied = InMemoryTransport
            .invoke(map.upsert_qr(), context(), request.clone())
            .unwrap()
            .result;
        assert_eq!(applied.sequence, initial.sequence + 1);
        assert!(
            InMemoryTransport
                .invoke(map.upsert_qr(), context(), request)
                .is_err()
        );
        let answer = InMemoryTransport
            .invoke(
                map.lookup_qr(),
                context(),
                LookupQr {
                    anchor_id: anchor().anchor_id,
                },
            )
            .unwrap()
            .result;
        assert_eq!(answer.snapshot, applied);
        assert_eq!(answer.anchor, Some(anchor()));
        let retained = map.product().latest_existing().unwrap().unwrap();
        assert_eq!(retained.sequence, answer.snapshot.sequence);
        assert_eq!(retained.payload.scenegraph.anchors.len(), 1);
        let absent = InMemoryTransport
            .invoke(
                map.lookup_qr(),
                context(),
                LookupQr {
                    anchor_id: "missing".into(),
                },
            )
            .unwrap()
            .result;
        assert!(absent.anchor.is_none());
        assert_eq!(absent.snapshot, applied);
        map.close();
        assert!(
            InMemoryTransport
                .invoke(
                    map.lookup_qr(),
                    context(),
                    LookupQr {
                        anchor_id: "missing".into()
                    }
                )
                .is_err()
        );
        assert!(matches!(
            map.product()
                .buffer()
                .subscribe(auki_components::CursorStart::Latest)
                .next_timeout(std::time::Duration::ZERO),
            auki_components::CursorRead::Closed
        ));
        assert!(map.product().latest_existing().unwrap().is_some());
    }
    #[test]
    fn invalid_or_regressing_updates_never_change_published_state() {
        let clock = Arc::new(AtomicU64::new(10));
        let map = map(clock.clone());
        let initial = map.snapshot_reference();
        let mut qr = anchor();
        qr.side_length_m = -1.0;
        clock.store(11, Ordering::SeqCst);
        assert!(
            InMemoryTransport
                .invoke(
                    map.upsert_qr(),
                    context(),
                    UpsertQr {
                        expected_snapshot: initial.clone(),
                        anchor: qr
                    }
                )
                .is_err()
        );
        clock.store(9, Ordering::SeqCst);
        assert!(
            InMemoryTransport
                .invoke(
                    map.upsert_qr(),
                    context(),
                    UpsertQr {
                        expected_snapshot: initial.clone(),
                        anchor: anchor()
                    }
                )
                .is_err()
        );
        assert_eq!(map.snapshot_reference(), initial);
        assert!(
            map.product()
                .latest_existing()
                .unwrap()
                .unwrap()
                .payload
                .scenegraph
                .anchors
                .is_empty()
        );
    }
    #[test]
    fn concurrent_writers_cannot_both_commit_against_one_snapshot() {
        let clock = Arc::new(AtomicU64::new(1));
        let map = map(clock.clone());
        let base = map.snapshot_reference();
        clock.store(2, Ordering::SeqCst);
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                InMemoryTransport.invoke(
                    map.upsert_qr(),
                    context(),
                    UpsertQr {
                        expected_snapshot: base.clone(),
                        anchor: anchor(),
                    },
                )
            });
            let second = scope.spawn(|| {
                let mut other = anchor();
                other.anchor_id = "second".into();
                barrier.wait();
                InMemoryTransport.invoke(
                    map.upsert_qr(),
                    context(),
                    UpsertQr {
                        expected_snapshot: base.clone(),
                        anchor: other,
                    },
                )
            });
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(map.snapshot_reference().sequence, base.sequence + 1);
        assert_eq!(
            map.product()
                .latest_existing()
                .unwrap()
                .unwrap()
                .payload
                .scenegraph
                .anchors
                .len(),
            1
        );
    }

    #[test]
    fn failed_retention_cannot_be_acknowledged_as_a_new_snapshot() {
        let clock = Arc::new(AtomicU64::new(1));
        let map = map(clock.clone());
        let base = map.snapshot_reference();
        map.product().buffer().close();
        clock.store(2, Ordering::SeqCst);
        assert!(
            InMemoryTransport
                .invoke(
                    map.upsert_qr(),
                    context(),
                    UpsertQr {
                        expected_snapshot: base.clone(),
                        anchor: anchor()
                    }
                )
                .is_err()
        );
        assert_eq!(map.snapshot_reference(), base);
        assert!(
            map.product()
                .latest_existing()
                .unwrap()
                .unwrap()
                .payload
                .scenegraph
                .anchors
                .is_empty()
        );
        assert!(matches!(
            InMemoryTransport.invoke(
                map.lookup_qr(),
                context(),
                LookupQr {
                    anchor_id: "missing".into()
                }
            ),
            Err(InvocationError::TargetUnavailable)
        ));
    }
}

#[test]
fn poses_require_explicit_frames_and_map_destination_must_match() {
    let unlabeled = serde_json::json!({
        "translation": [0., 0., 0.], "rotation_wxyz": [1., 0., 0., 0.]
    });
    assert!(serde_json::from_value::<RigidTransform>(unlabeled).is_err());
    let mut qr = anchor();
    qr.pose_in_map.from_frame_id.clear();
    assert!(qr.validate().is_err());
    let mut qr = anchor();
    qr.pose_in_map.to_frame_id = "another-map-frame".into();
    let mut scene = scene();
    assert!(qr.validate_in_map(&scene.map).is_err());
    scene.anchors.insert(qr.anchor_id.clone(), qr);
    assert!(MapSnapshot::new(scene).is_err());
}

#[test]
fn inspection_geometry_uses_map_units_without_changing_snapshot_contract() {
    let mut scene = scene();
    scene.map.frame.meters_per_unit = 0.01;
    let qr = anchor();
    scene.anchors.insert(qr.anchor_id.clone(), qr);
    let canonical = scene.to_usda().unwrap();
    let visual = scene.to_usda_with_portal_geometry().unwrap();
    assert!(visual.contains("(-10, -10, 0)")); // 20cm square in centimeter stage units.
    assert!(visual.contains("primvars:displayColor = [(1, 1, 1)]"));
    assert!(visual.contains("doubleSided = true"));
    assert!(!canonical.contains("def Mesh"));
    assert_eq!(scene.to_usda().unwrap(), canonical);
    MapSnapshot::new(scene).unwrap().validate().unwrap();
}
