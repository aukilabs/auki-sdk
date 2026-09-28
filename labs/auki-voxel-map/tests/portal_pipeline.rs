use auki_components::{ComponentRuntime, ProductReference};
use auki_qr_localizer::{Calibration, Distortion, QualityGate, estimate_camera_pose};
use auki_registry::FrameRegistryEntry;
use auki_scenegraph::{
    MapDefinition, MapFrame, MapSnapshot, QrAnchor, RigidTransform, Scenegraph, UpAxis,
    component::SnapshotReference,
};
use auki_voxel_map::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn fixture() -> (ComponentRuntime, VoxelMapDefinition, DepthObservation) {
    let runtime = ComponentRuntime::new("peer1");
    let source = ProductReference {
        peer_id: "peer1".into(),
        product_id: "depth-camera/run1".into(),
        manifest_hash: "fixture-camera-hash".into(),
    };
    let mut frame = MapFrame::z_up_meters("portal-map-frame", "explicitly established at portal A");
    frame.up_axis = UpAxis::Y;
    let mut scene = Scenegraph::new(MapDefinition {
        map_id: "portal-map".into(),
        name: None,
        domain_reference: Some("store".into()),
        frame,
    })
    .unwrap();
    let anchor = QrAnchor {
        anchor_id: "A".into(),
        payload: "portal:A".into(),
        side_length_m: 0.2,
        pose_in_map: RigidTransform {
            from_frame_id: "A-frame".into(),
            to_frame_id: "portal-map-frame".into(),
            translation: [0.05, 0.05, 0.05],
            rotation_wxyz: [1., 0., 0., 0.],
        },
    };
    scene.anchors.insert("A".into(), anchor.clone());
    let portal = MapSnapshot::new(scene).unwrap();
    let reference = SnapshotReference {
        product: ProductReference {
            peer_id: "peer1".into(),
            product_id: "portal-map/run1".into(),
            manifest_hash: "fixture-map-hash".into(),
        },
        sequence: 1,
    };
    let definition = VoxelMapDefinition::from_portal_map(
        "peer1",
        "voxel-map",
        reference.clone(),
        &portal,
        FrameRegistryEntry::opengl("peer1", "portal-map-frame"),
        source.clone(),
        FrameRegistryEntry::ros_optical("peer1", "camera-optical"),
        "camera-clock".into(),
        0.1,
    )
    .unwrap();
    let cal = Calibration {
        id: "cal".into(),
        camera_product: source.clone(),
        camera_frame_id: "camera-optical".into(),
        clock_id: "camera-clock".into(),
        width: 640,
        height: 480,
        fx: 500.,
        fy: 500.,
        cx: 320.,
        cy: 240.,
        distortion: Distortion::None,
    };
    // A 20cm square at one meter, seen by a calibrated optical camera.
    let pose = estimate_camera_pose(
        &portal.scenegraph.map,
        &anchor,
        [[270., 190.], [370., 190.], [370., 290.], [270., 290.]],
        &cal,
        QualityGate::default(),
    )
    .unwrap();
    let observation = DepthObservation {
        source,
        sequence: 1,
        timestamp_ns: 100,
        clock_id: "camera-clock".into(),
        sensor_frame_id: "camera-optical".into(),
        hit_points_m: vec![[0., 0., 0.5]],
        pose: TimedSensorPose {
            timestamp_ns: 100,
            clock_id: "camera-clock".into(),
            sensor_to_map: pose.camera_pose_in_map,
            portal_snapshot: reference,
        },
    };
    (runtime, definition, observation)
}
fn component(runtime: &ComponentRuntime, definition: VoxelMapDefinition) -> VoxelMapComponent {
    let clock = AtomicU64::new(1);
    VoxelMapComponent::new(
        runtime,
        "voxel-map",
        "voxel-run1",
        "publication-clock",
        definition,
        move || clock.fetch_add(1, Ordering::SeqCst),
    )
    .unwrap()
}
fn latest(map: &VoxelMapComponent) -> VoxelSnapshot {
    (*map.product().latest_existing().unwrap().unwrap().payload).clone()
}
fn evidence(snapshot: &VoxelSnapshot, position: [f64; 3]) -> Option<f64> {
    use prost::Message;
    let update = auki_datatypes::map::MapUpdate::decode(snapshot.checkpoint.as_slice()).unwrap();
    let grid = snapshot.definition.grid();
    for chunk in update.checkpoint.unwrap().voxel_chunks {
        for voxel in chunk.voxels {
            let center = [
                (chunk.chunk_x, voxel.x),
                (chunk.chunk_y, voxel.y),
                (chunk.chunk_z, voxel.z),
            ]
            .map(|(c, v)| {
                (f64::from(c) * f64::from(grid.chunk_dimension) + f64::from(v) + 0.5)
                    * grid.voxel_size_m.0
            });
            if center
                .iter()
                .zip(position)
                .all(|(a, b)| (a - b).abs() < 1e-8)
            {
                return Some(voxel.occupancy_evidence);
            }
        }
    }
    None
}
#[test]
fn portal_pnp_pose_places_depth_and_publishes_explicitly_aligned_voxels() {
    let (runtime, definition, observation) = fixture();
    let mut map = component(&runtime, definition.clone());
    assert_eq!(
        map.product()
            .latest_existing()
            .unwrap()
            .unwrap()
            .payload
            .integrated_observations,
        0
    );
    assert_eq!(
        map.integrate(observation.clone()).unwrap(),
        IntegrationResult::Applied
    );
    let snapshot = latest(&map);
    assert_eq!(snapshot.definition, definition);
    assert_eq!(
        snapshot
            .latest_observation
            .as_ref()
            .unwrap()
            .pose
            .sensor_to_map
            .from_frame_id,
        "camera-optical"
    );
    assert_eq!(
        snapshot
            .latest_observation
            .as_ref()
            .unwrap()
            .pose
            .sensor_to_map
            .to_frame_id,
        "portal-map-frame"
    );
    assert!(evidence(&snapshot, [0.05, 0.05, 0.55]).unwrap() > 0.);
    assert!(evidence(&snapshot, [0.05, 0.05, 0.85]).unwrap() < 0.);
    assert_eq!(evidence(&snapshot, [2.05, 0.05, 0.55]), None);
    let catalog = runtime.catalog().snapshot();
    assert_eq!(catalog.products.len(), 1);
    let metadata = catalog.products[0].metadata.as_ref().unwrap();
    assert_eq!(metadata.schema, "auki.voxel-map.catalog/v2");
    assert_eq!(
        metadata.value["definition"]["portal_map"]["frame"]["id"],
        "portal-map-frame"
    );
    assert_eq!(metadata.value["integrated_observations"], 1);
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    let decoded: VoxelSnapshot = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, snapshot);
    decoded.accumulator().unwrap();
    map.close();
    assert!(map.integrate(observation).is_err());
}
#[test]
fn free_space_can_clear_a_previous_hit_but_replay_cannot_count_twice() {
    let (runtime, definition, mut observation) = fixture();
    let mut map = component(&runtime, definition);
    map.integrate(observation.clone()).unwrap();
    let before = map.product().latest_existing().unwrap().unwrap().sequence;
    assert_eq!(
        map.integrate(observation.clone()).unwrap(),
        IntegrationResult::Duplicate
    );
    assert_eq!(
        map.product().latest_existing().unwrap().unwrap().sequence,
        before
    );
    let old = observation.clone();
    for sequence in [2, 3] {
        observation.sequence = sequence;
        observation.timestamp_ns = 100 * sequence;
        observation.pose.timestamp_ns = observation.timestamp_ns;
        observation.hit_points_m = vec![[0., 0., 1.]];
        map.integrate(observation.clone()).unwrap();
    }
    let snapshot = latest(&map);
    assert!(evidence(&snapshot, [0.05, 0.05, 0.55]).unwrap() < 0.);
    assert!(evidence(&snapshot, [0.05, 0.05, 0.05]).unwrap() > 0.);
    assert_eq!(snapshot.integrated_observations, 3);
    assert!(map.integrate(old).is_err());
    assert_eq!(latest(&map), snapshot);
}
#[test]
fn bad_pose_time_frame_provenance_and_input_leave_the_map_unchanged() {
    let (runtime, definition, observation) = fixture();
    let mut map = component(&runtime, definition);
    let initial = latest(&map);
    for kind in 0..8 {
        let mut bad = observation.clone();
        match kind {
            0 => bad.pose.timestamp_ns += 1,
            1 => bad.pose.clock_id = "other".into(),
            2 => bad.pose.sensor_to_map.from_frame_id = "other-sensor".into(),
            3 => bad.pose.sensor_to_map.to_frame_id = "other-map".into(),
            4 => bad.pose.portal_snapshot.sequence += 1,
            5 => bad.hit_points_m[0][0] = f64::NAN,
            6 => bad.hit_points_m = vec![[0., 0., 1.]; MAX_POINTS + 1],
            _ => bad.source.product_id = "different-camera".into(),
        }
        assert!(map.integrate(bad).is_err());
        assert_eq!(latest(&map), initial);
    }
    map.integrate(observation.clone()).unwrap();
    let accepted = latest(&map);
    let mut changed = observation;
    changed.hit_points_m = vec![[0., 0., 0.9]];
    assert!(map.integrate(changed).is_err());
    assert_eq!(latest(&map), accepted);
}
#[test]
fn invalid_frame_convention_and_portal_alignment_cannot_create_a_map() {
    let (_, definition, _) = fixture();
    let mut bad = definition.clone();
    bad.frame.frame_id = "unrelated".into();
    assert!(bad.validate().is_err());
    let mut bad = definition.clone();
    bad.portal_map.frame.up_axis = UpAxis::Z;
    assert!(bad.validate().is_err());
    let mut bad = definition;
    bad.portal_map.frame.meters_per_unit = 0.01;
    assert!(bad.validate().is_err());
}
#[test]
fn publication_failure_closes_map_without_acknowledging_unretained_state() {
    let (runtime, definition, observation) = fixture();
    let mut map = component(&runtime, definition);
    map.product().buffer().close();
    assert!(map.integrate(observation.clone()).is_err());
    assert!(map.integrate(observation).is_err());
}
#[test]
fn publication_clock_regression_is_atomic() {
    let (runtime, definition, observation) = fixture();
    let clock = Arc::new(AtomicU64::new(10));
    let copy = clock.clone();
    let mut map =
        VoxelMapComponent::new(&runtime, "voxel", "run", "clock", definition, move || {
            copy.load(Ordering::SeqCst)
        })
        .unwrap();
    assert!(map.integrate(observation.clone()).is_err());
    assert_eq!(latest(&map).integrated_observations, 0);
    clock.store(11, Ordering::SeqCst);
    map.integrate(observation).unwrap();
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct MeasuredHits(Vec<[f64; 3]>);
impl auki_components::ContractType for MeasuredHits {
    const DATATYPE: &'static str = "test.measured-hits/v1";
}
fn sensor(
    runtime: &ComponentRuntime,
    frame: &str,
    clock: &str,
) -> (
    auki_components::Component,
    auki_components::ConfiguredObservable<MeasuredHits>,
    auki_components::BufferProductCapture<MeasuredHits>,
) {
    use auki_components::*;
    let component = runtime
        .component(ComponentSpec::new("depth").observable(ObservableContract {
            name: "hits".into(),
            datatype: MeasuredHits::DATATYPE.into(),
            schema: MeasuredHits::DATATYPE.into(),
            access: vec![ObservationAccess::FollowNew],
            exposure: Exposure::Cluster,
        }))
        .unwrap();
    let output = component
        .configured_observable(
            ConfiguredObservableSpec::new(
                "hits",
                "camera-run",
                clock,
                PayloadContract::Structured(StructuredPayloadContract {
                    modality: "depth".into(),
                    datatype: MeasuredHits::DATATYPE.into(),
                    schema: MeasuredHits::DATATYPE.into(),
                    observes: "store".into(),
                    unit: Some("meters".into()),
                }),
            )
            .in_spatial_frame(frame),
        )
        .unwrap();
    component.expose().unwrap();
    let capture = runtime
        .capture_buffer("depth-product", &output, BufferLimits::entries(4), |_| 1024)
        .unwrap();
    (component, output, capture)
}

#[test]
fn one_mapper_consumes_two_peer_products_and_clears_a_moved_object() {
    let (peer1, mut definition, mut first) = fixture();
    let peer2 = ComponentRuntime::new("peer2");
    let (_c1, output1, capture1) = sensor(&peer1, "camera-optical", "camera-clock");
    let (_c2, output2, capture2) = sensor(&peer2, "peer2-optical", "peer2-clock");
    definition.sources[0].sensor_product = capture1.product().reference();
    definition.sources.push(SensorBinding {
        sensor_product: capture2.product().reference(),
        sensor_frame: FrameRegistryEntry::ros_optical("peer2", "peer2-optical"),
        observation_clock_id: "peer2-clock".into(),
    });
    let mut map = component(&peer1, definition);
    output1
        .publish(1000, Arc::new(MeasuredHits(first.hit_points_m.clone())))
        .unwrap();
    let captured = capture1.product().latest_existing().unwrap().unwrap();
    first.source = capture1.product().reference();
    first.sequence = captured.sequence;
    first.timestamp_ns = captured.timestamp_ns;
    first.pose.timestamp_ns = captured.timestamp_ns;
    first.hit_points_m = captured.payload.0.clone();
    map.integrate(first.clone()).unwrap();
    assert_eq!(evidence(&latest(&map), [0.05, 0.05, 0.55]), Some(1.));

    // Peer2 stands 20cm to the side with a different optical orientation.
    // Its ray passes through the old object location and ends at the new one.
    // Its clock is intentionally numerically behind peer1's clock.
    let mut second = first.clone();
    second.source = capture2.product().reference();
    second.clock_id = "peer2-clock".into();
    second.sensor_frame_id = "peer2-optical".into();
    second.pose.clock_id = second.clock_id.clone();
    second.pose.sensor_to_map = RigidTransform {
        from_frame_id: "peer2-optical".into(),
        to_frame_id: "portal-map-frame".into(),
        translation: [0.25, 0.05, 1.05],
        rotation_wxyz: [0., 0., 1., 0.],
    };
    let mut old_second = None;
    for timestamp in [10, 20] {
        output2
            .publish(timestamp, Arc::new(MeasuredHits(vec![[0.4, 0., 1.]])))
            .unwrap();
        let captured = capture2.product().latest_existing().unwrap().unwrap();
        second.sequence = captured.sequence;
        second.timestamp_ns = captured.timestamp_ns;
        second.pose.timestamp_ns = captured.timestamp_ns;
        second.hit_points_m = captured.payload.0.clone();
        // Exercise serialization at the host's ingestion boundary (no network transport).
        let received = serde_json::from_slice(&serde_json::to_vec(&second).unwrap()).unwrap();
        assert_eq!(map.integrate(received).unwrap(), IntegrationResult::Applied);
        if timestamp == 10 {
            old_second = Some(second.clone());
        }
    }
    let snapshot = latest(&map);
    assert_eq!(snapshot.integrated_observations, 3);
    assert_eq!(evidence(&snapshot, [0.05, 0.05, 0.55]), Some(-1.));
    assert_eq!(evidence(&snapshot, [-0.15, 0.05, 0.05]), Some(2.));
    assert_eq!(evidence(&snapshot, [2.05, 0.05, 0.55]), None);
    snapshot.accumulator().unwrap();
    assert_eq!(snapshot.definition.sources.len(), 2);
    assert_eq!(
        snapshot.latest_observation.as_ref().unwrap().source.peer_id,
        "peer2"
    );

    let publication = map.product().latest_existing().unwrap().unwrap().sequence;
    // Each source's latest exact replay is a no-op, even after another peer contributes.
    assert_eq!(
        map.integrate(first.clone()).unwrap(),
        IntegrationResult::Duplicate
    );
    assert_eq!(
        map.integrate(second.clone()).unwrap(),
        IntegrationResult::Duplicate
    );
    assert!(map.integrate(old_second.unwrap()).is_err());
    let mut revised = first;
    revised.hit_points_m[0][2] = 0.8;
    assert!(map.integrate(revised).is_err());
    for kind in 0..4 {
        let mut bad = second.clone();
        bad.sequence += 1;
        bad.timestamp_ns += 1;
        bad.pose.timestamp_ns += 1;
        match kind {
            0 => bad.source.peer_id = "unadmitted-peer".into(),
            1 => bad.pose.sensor_to_map.to_frame_id = "unrelated-map".into(),
            2 => bad.pose.clock_id = "camera-clock".into(),
            _ => bad.pose.portal_snapshot.sequence += 1,
        }
        assert!(map.integrate(bad).is_err());
    }
    assert_eq!(latest(&map), snapshot);
    assert_eq!(
        map.product().latest_existing().unwrap().unwrap().sequence,
        publication
    );
}

#[test]
fn source_admission_is_bounded_unique_and_explicit() {
    let (_, definition, _) = fixture();
    let mut bad = definition.clone();
    bad.sources.clear();
    assert!(bad.validate().is_err());
    let mut bad = definition.clone();
    bad.sources.push(bad.sources[0].clone());
    assert!(bad.validate().is_err());
    let mut bad = definition;
    bad.sources = vec![bad.sources[0].clone(); MAX_SOURCES + 1];
    assert!(bad.validate().is_err());
}
