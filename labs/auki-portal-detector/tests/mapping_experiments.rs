//! Rendered QR images -> PortalDetector -> host PnP placement -> localizer ->
//! authenticated map transfer -> local import/localization -> aligned merge.
#[path = "../../auki-scenegraph/tests/support/mod.rs"]
mod support;
use auki_component_protocol::{
    CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint, ObservationStart,
    RemoteObservationEvent,
};
use auki_components::*;
use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
use auki_domain_client::{Portal, PortalId};
use auki_geometry::{
    compose_spatial_transforms, inverse_spatial_transform,
    pnp::{Camera, Vector2, estimate_square_pose_from_pixels, pose_tools},
};
use auki_portal_detector::*;
use auki_qr_localizer::{component::*, *};
use auki_qr_mapper::{
    PortalSizeError, ResolvedPortal,
    local::{AnchorPlacement, EnsureQrResult, PortalMapper, PortalMetadataSource},
};
use auki_scenegraph::{alignment::*, catalog::MapCatalogData, component::*, resolution::*, *};
use auki_sdk::{AukiPeer, Identity};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Metadata;
impl PortalMetadataSource for Metadata {
    async fn resolve(
        &self,
        _: Uuid,
        id: &PortalId,
        _: &CancellationToken,
    ) -> Result<ResolvedPortal, PortalSizeError> {
        ResolvedPortal::try_from(
            serde_json::from_value::<Portal>(serde_json::json!({
                "id": id.as_str(), "short_id":"ABC12345678", "name":"fixture", "size":40.,
                "organization_id":null,"default_domain_id":null,"redirect_url":null,
                "created_at":"2026-09-01T00:00:00Z","updated_at":"2026-09-02T00:00:00Z"
            }))
            .unwrap(),
        )
    }
}
fn id(n: u128) -> String {
    Uuid::from_u128(n).to_string()
}
fn ctx(r: &ComponentRuntime) -> InvocationContext {
    InvocationContext {
        invocation_id: Uuid::new_v4().to_string(),
        caller_peer_id: r.peer_id().into(),
        caller_component_id: "host".into(),
    }
}
fn numeric(t: &RigidTransform) -> SpatialTransform {
    let [x, y, z] = t.translation;
    let [w, qx, qy, qz] = t.rotation_wxyz;
    SpatialTransform {
        translation: Some(Vec3 { x, y, z }),
        orientation: Some(Quat {
            x: qx,
            y: qy,
            z: qz,
            w,
        }),
    }
}
fn labelled(t: SpatialTransform, from: &str, to: &str) -> RigidTransform {
    let p = t.translation.unwrap();
    let q = t.orientation.unwrap();
    RigidTransform {
        from_frame_id: from.into(),
        to_frame_id: to.into(),
        translation: [p.x, p.y, p.z],
        rotation_wxyz: [q.w, q.x, q.y, q.z],
    }
}
fn compose(a: &RigidTransform, b: &RigidTransform) -> RigidTransform {
    assert_eq!(a.to_frame_id, b.from_frame_id);
    labelled(
        compose_spatial_transforms(&numeric(a), &numeric(b)).unwrap(),
        &a.from_frame_id,
        &b.to_frame_id,
    )
}
fn inverse(t: &RigidTransform) -> RigidTransform {
    labelled(
        inverse_spatial_transform(&numeric(t)).unwrap(),
        &t.to_frame_id,
        &t.from_frame_id,
    )
}
fn check_pose(actual: &RigidTransform, expected: &RigidTransform) {
    assert_eq!(actual.from_frame_id, expected.from_frame_id);
    assert_eq!(actual.to_frame_id, expected.to_frame_id);
    for (a, b) in actual.translation.iter().zip(expected.translation) {
        assert!((a - b).abs() < 0.02, "position {a} != {b}");
    }
    let dot: f64 = actual
        .rotation_wxyz
        .iter()
        .zip(expected.rotation_wxyz)
        .map(|(a, b)| a * b)
        .sum();
    assert!(
        (1. - dot.abs()).abs() < 0.0002,
        "orientation mismatch {dot}"
    );
}
fn make_map(r: &ComponentRuntime, name: &str, frame: &str, domain: Uuid) -> Arc<MapComponent> {
    let ticks = AtomicU64::new(1);
    let owner = r.peer_id().to_string();
    Arc::new(
        MapComponent::new(
            r,
            MapComponentConfig {
                component_id: name.into(),
                publication_id: name.into(),
                clock: fixture_clock("map-clock"),
                map: MapDefinition {
                    map_id: name.into(),
                    name: Some(name.into()),
                    domain_reference: Some(domain.to_string()),
                    frame: MapFrame::z_up_meters(
                        frame,
                        "Host-declared Z-up, meter frame; no gravity inferred from QR",
                    ),
                },
            },
            move || ticks.fetch_add(1, Ordering::SeqCst),
            |_| true,
            move |c| c.caller_peer_id == owner,
        )
        .unwrap(),
    )
}
fn put(r: &ComponentRuntime, map: &MapComponent, anchor: QrAnchor) {
    InMemoryTransport
        .invoke(
            map.upsert_qr(),
            ctx(r),
            UpsertQr {
                expected_snapshot: map.snapshot_reference(),
                anchor,
            },
        )
        .unwrap();
}
fn snapshot(map: &MapComponent) -> MapSnapshot {
    (*map.product().latest_existing().unwrap().unwrap().payload).clone()
}
fn export(map: &MapComponent, path: &Path) {
    let scene = snapshot(map);
    scene.validate().unwrap();
    std::fs::write(
        path,
        scene.scenegraph.to_usda_with_portal_geometry().unwrap(),
    )
    .unwrap();
    std::fs::write(
        path.with_extension("json"),
        serde_json::to_vec_pretty(&scene.scenegraph).unwrap(),
    )
    .unwrap();
}
struct CameraRig {
    _camera: Component,
    _capture: BufferProductCapture<VideoFrame>,
    frames: ConfiguredObservable<VideoFrame>,
    detector: PortalDetector,
    results: BufferProductCapture<PortalDetection>,
    localizer: QrLocalizerComponent,
    maps: PortalMaps,
    clock: AtomicU64,
    frame: String,
}
impl CameraRig {
    fn new(r: &ComponentRuntime, domain: Uuid) -> Self {
        let frame = format!("{}-optical", r.peer_id());
        let camera = r
            .component(ComponentSpec::new("camera").observable(ObservableContract {
                name: "frames".into(),
                datatype: VideoFrame::DATATYPE.into(),
                schema: "auki.video-frame/v1".into(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            }))
            .unwrap();
        let frames = camera
            .configured_observable(
                ConfiguredObservableSpec::new(
                    "frames",
                    "camera-run",
                    fixture_clock("capture-clock"),
                    PayloadContract::Camera(CameraPayloadContract {
                        datatype: VideoFrame::DATATYPE.into(),
                        schema: "auki.video-frame/v1".into(),
                        encoding: "rgb8".into(),
                        width: 800,
                        height: 600,
                        nominal_frame_rate_hz: Some(10),
                        observes: "fixture portals".into(),
                    }),
                )
                .in_registered_frame(
                    auki_components::FrameRegistryEntry::ros_optical("camera-peer", &frame),
                ),
            )
            .unwrap();
        camera.expose().unwrap();
        let capture = r
            .capture_buffer(
                "camera-frames",
                &frames,
                BufferLimits::entries(16),
                |v: &VideoFrame| v.bytes.len(),
            )
            .unwrap();
        let maps = PortalMaps::new(ctx(r));
        let ticks = AtomicU64::new(1);
        let detector = PortalDetector::bind(
            r,
            "detector",
            "detector-run",
            &capture.product(),
            maps.clone(),
            Metadata,
            domain,
            fixture_clock("processing-clock"),
            move || ticks.fetch_add(1, Ordering::SeqCst),
        )
        .unwrap();
        let results = r
            .capture_buffer(
                "portal-results",
                detector.portals(),
                BufferLimits::entries(64),
                |_| 16384,
            )
            .unwrap();
        let calibration = Calibration::for_product(
            "calibration",
            &capture.product(),
            Intrinsics {
                fx: 500.,
                fy: 500.,
                cx: 400.,
                cy: 300.,
                distortion: Distortion::None,
            },
        )
        .unwrap();
        let localizer = QrLocalizerComponent::bind(
            r,
            "localizer",
            "pose-results",
            &detector.detection_product(),
            Arc::new(maps.clone()),
            calibration,
            QualityGate::default(),
        )
        .unwrap();
        Self {
            _camera: camera,
            _capture: capture,
            frames,
            detector,
            results,
            localizer,
            maps,
            clock: AtomicU64::new(100),
            frame,
        }
    }
    async fn observe(&self, n: u128) -> (PortalDetection, f64) {
        let qr = qrcode::QrCode::new(id(n)).unwrap();
        let modules = qr.width();
        let scale = 6;
        let width = modules * scale;
        let mut bytes = vec![255; 800 * 600 * 3];
        let left = 400 - width / 2;
        let top = 300 - width / 2;
        for y in 0..modules {
            for x in 0..modules {
                if qr[(x, y)] == qrcode::Color::Dark {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let at = ((top + y * scale + dy) * 800 + left + x * scale + dx) * 3;
                            bytes[at..at + 3].fill(0);
                        }
                    }
                }
            }
        }
        let time = self.clock.fetch_add(100, Ordering::SeqCst);
        self.frames
            .publish(
                time,
                Arc::new(VideoFrame {
                    width: 800,
                    height: 600,
                    encoding: "rgb8".into(),
                    bytes: bytes.into(),
                }),
            )
            .unwrap();
        let found = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                for entry in self.results.product().buffer().snapshot(0, u64::MAX) {
                    let record = &entry.payload.payload;
                    if record.source_frame.timestamp_ns == time
                        && record.detection.payload == id(n)
                        && !matches!(record.size, PortalSize::Pending)
                    {
                        return (**record).clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            found.size,
            PortalSize::Database { .. } | PortalSize::LocalMaps { .. }
        ));
        (found, 500. * 0.4 / width as f64)
    }
    fn localize(
        &self,
        r: &ComponentRuntime,
        record: &PortalDetection,
        map: &MapComponent,
    ) -> CameraPoseEstimate {
        let result = InMemoryTransport
            .invoke(
                self.localizer.localize_once(),
                ctx(r),
                LocalizeOnce {
                    detection_product: record.detection_product.clone(),
                    detection_sequence: record.detection_sequence,
                    detection_index: record.detection_index,
                    target_map: map.snapshot_reference(),
                },
            )
            .unwrap()
            .result;
        assert_eq!(result.estimates.len(), 1);
        result.estimates[0].pose.clone()
    }
    fn close(&mut self) {
        self.detector.close();
        self.localizer.close(u64::MAX).unwrap();
    }
}
fn camera_pose(frame: &str, x: f64, range: f64) -> RigidTransform {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    RigidTransform {
        from_frame_id: frame.into(),
        to_frame_id: "store-frame".into(),
        translation: [x, -range, 1.5],
        rotation_wxyz: [h, -h, 0., 0.],
    }
}
fn expected_portal(n: u128, x: f64) -> RigidTransform {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    RigidTransform {
        from_frame_id: format!("portal-{}", id(n)),
        to_frame_id: "store-frame".into(),
        translation: [x, 0., 1.5],
        rotation_wxyz: [h, h, 0., 0.],
    }
}
// Explicit host placement: calibrated PnP yields portal->camera, which is composed
// with a capture-time camera->map pose. PortalMapper itself still does no PnP.
async fn map_detection(
    rig: &CameraRig,
    domain: Uuid,
    map: Arc<MapComponent>,
    record: &PortalDetection,
    camera_to_map: &RigidTransform,
) {
    assert_eq!(camera_to_map.from_frame_id, rig.frame);
    let size = match &record.size {
        PortalSize::Database { side_length_m, .. }
        | PortalSize::LocalMaps { side_length_m, .. } => *side_length_m,
        _ => panic!("unresolved size"),
    };
    let corners = record
        .detection
        .refined_corners_px
        .as_ref()
        .unwrap_or(&record.detection.corners_px)
        .map(|p| Vector2::new(p.x, p.y));
    let solved = estimate_square_pose_from_pixels(
        corners,
        size,
        &Camera::pinhole(500., 500., 400., 300.).unwrap(),
    )
    .unwrap();
    let optical = pose_tools::from_opengl_to_opencv(&solved.pose);
    let portal_to_camera = RigidTransform {
        from_frame_id: format!("portal-{}", record.detection.payload),
        to_frame_id: rig.frame.clone(),
        translation: [optical.position.x, optical.position.y, optical.position.z],
        rotation_wxyz: [
            optical.rotation.w,
            optical.rotation.x,
            optical.rotation.y,
            optical.rotation.z,
        ],
    };
    let placement = compose(&portal_to_camera, camera_to_map);
    let mapper = PortalMapper::new(rig.maps.clone(), Metadata);
    let result = mapper
        .lookup_helper(
            &record.detection.payload,
            domain,
            Some(AnchorPlacement {
                expected_snapshot: map.snapshot_reference(),
                map,
                pose_in_map: placement,
            }),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(result, EnsureQrResult::Added(_)));
}
async fn receive(reader: &AukiPeer, owner: &AukiPeer, map: &MapComponent) -> MapSnapshot {
    let client = ComponentProtocolClient::new(reader.protocols());
    let route = owner.listen_addresses()[0].clone();
    let CatalogResponse::Snapshot { snapshot: catalog } = client
        .catalog_exact(owner.peer_id(), route.clone(), None)
        .await
        .unwrap()
    else {
        panic!("catalog")
    };
    let entry = catalog
        .products
        .iter()
        .find(|p| p.manifest.reference() == map.product().reference())
        .unwrap();
    let meta = entry.metadata.as_ref().unwrap();
    let data: MapCatalogData = serde_json::from_value(meta.value.clone()).unwrap();
    assert_eq!(data.portals.len(), 2);
    let mut stream = client
        .subscribe_product_exact::<MapSnapshot>(
            owner.peer_id(),
            route,
            map.product().reference(),
            ObservationStart::LatestExisting,
            BufferLimits::entries(2),
            MapSnapshot::encoded_size,
        )
        .await
        .unwrap();
    let RemoteObservationEvent::Observation(o) = stream.next().await.unwrap().unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(o.sequence, meta.source_sequence);
    o.payload.validate().unwrap();
    let value = (*o.payload).clone();
    stream.close().await.unwrap();
    value
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct DepthHits(Vec<[f64; 3]>);
impl ContractType for DepthHits {
    const DATATYPE: &'static str = "fixture.depth-hits/v1";
}

async fn voxel_experiment(
    r: &ComponentRuntime,
    rig: &CameraRig,
    portals: &Arc<MapComponent>,
    out: &Path,
) {
    use auki_registry::{AxisConvention, AxisDirection, FrameRegistryEntry};
    use auki_voxel_map::*;
    rig.maps.register(portals.clone()).unwrap();
    let component = r
        .component(ComponentSpec::new("depth").observable(ObservableContract {
            name: "hits".into(),
            datatype: DepthHits::DATATYPE.into(),
            schema: DepthHits::DATATYPE.into(),
            access: vec![ObservationAccess::FollowNew],
            exposure: Exposure::Cluster,
        }))
        .unwrap();
    let depth = component
        .configured_observable(
            ConfiguredObservableSpec::new(
                "hits",
                "depth-run",
                fixture_clock("capture-clock"),
                PayloadContract::Structured(StructuredPayloadContract {
                    modality: "depth".into(),
                    datatype: DepthHits::DATATYPE.into(),
                    schema: DepthHits::DATATYPE.into(),
                    observes: "synthetic surfaces below portals".into(),
                    unit: Some("meters".into()),
                }),
            )
            .in_registered_frame(auki_components::FrameRegistryEntry::ros_optical(
                "camera-peer",
                &rig.frame,
            )),
        )
        .unwrap();
    component.expose().unwrap();
    let capture = r
        .capture_buffer("depth-product", &depth, BufferLimits::entries(3), |_| 1024)
        .unwrap();
    // The fixture explicitly co-locates RGB and depth optical frames.
    let mut frame = FrameRegistryEntry::ros_body(r.peer_id(), "store-frame");
    frame.axes = AxisConvention {
        x: AxisDirection::Right,
        y: AxisDirection::Forward,
        z: AxisDirection::Up,
    };
    let definition = VoxelMapDefinition::from_portal_map(
        r.peer_id(),
        "store-voxels",
        portals.snapshot_reference(),
        &snapshot(portals),
        frame,
        capture.product().reference(),
        FrameRegistryEntry::ros_optical(r.peer_id(), &rig.frame),
        fixture_clock("capture-clock"),
        0.1,
    )
    .unwrap();
    let ticks = AtomicU64::new(1);
    let mut voxels = VoxelMapComponent::new(
        r,
        "voxels",
        "voxels-run",
        fixture_clock("voxel-publication-clock"),
        definition,
        move || ticks.fetch_add(1, Ordering::SeqCst),
    )
    .unwrap();
    let mut expected = vec![];
    for (n, x) in [(1, 0.), (2, 2.), (3, 4.)] {
        let (record, range) = rig.observe(n).await;
        let pose = rig.localize(r, &record, portals).camera_pose_in_map;
        check_pose(&pose, &camera_pose(&rig.frame, x, range));
        let mut hits = vec![];
        for horizontal in [-0.25, 0.05, 0.35] {
            for down in [0.65, 0.95, 1.25] {
                hits.push([horizontal, down, range - 0.45]);
                expected.push([x + horizontal, -0.45, 1.5 - down]);
            }
        }
        let time = record.source_frame.timestamp_ns;
        depth.publish(time, Arc::new(DepthHits(hits))).unwrap();
        let observed = capture.product().latest_existing().unwrap().unwrap();
        let observation = DepthObservation {
            source: capture.product().reference(),
            sequence: observed.sequence,
            timestamp_ns: time,
            clock: fixture_clock("capture-clock"),
            sensor_frame_id: rig.frame.clone(),
            hit_points_m: observed.payload.0.clone(),
            pose: TimedSensorPose {
                timestamp_ns: time,
                clock: fixture_clock("capture-clock"),
                sensor_to_map: pose,
                portal_snapshot: portals.snapshot_reference(),
            },
        };
        assert_eq!(
            voxels.integrate(observation.clone()).unwrap(),
            IntegrationResult::Applied
        );
        assert_eq!(
            voxels.integrate(observation).unwrap(),
            IntegrationResult::Duplicate
        );
    }
    let result = voxels
        .product()
        .latest_existing()
        .unwrap()
        .unwrap()
        .payload
        .clone();
    let occupied = result.accumulator().unwrap().viewer_snapshot(0.).unwrap();
    let cells: Vec<_> = occupied
        .chunks
        .iter()
        .flat_map(|chunk| &chunk.voxels)
        .collect();
    assert_eq!(cells.len(), 27);
    for position in &expected {
        assert!(
            cells.iter().any(|cell| cell
                .center_m
                .iter()
                .zip(position)
                .all(|(a, b)| (a - b).abs() < 1e-6)),
            "missing expected occupied voxel {position:?}"
        );
    }
    let portal_snapshot = snapshot(portals);
    let usda = result
        .to_usda_with_portals(&portal_snapshot, &portals.snapshot_reference())
        .unwrap();
    assert_eq!(usda.matches("def Cube").count(), 27);
    assert_eq!(usda.matches("def Mesh").count(), 3);
    let mut wrong = portals.snapshot_reference();
    wrong.sequence += 1;
    assert!(
        result
            .to_usda_with_portals(&portal_snapshot, &wrong)
            .is_err()
    );
    let mut wrong_frame = portal_snapshot.scenegraph.clone();
    wrong_frame.map.map_id = "unrelated".into();
    assert!(
        result
            .to_usda_with_portals(
                &MapSnapshot::new(wrong_frame).unwrap(),
                &portals.snapshot_reference()
            )
            .is_err()
    );
    // Expected centers come from independent fixture geometry, not reverse-transforming the PnP result.
    std::fs::write(out.join("06-portals-and-voxels.usda"), usda).unwrap();
    std::fs::write(
        out.join("06-portals-and-voxels.json"),
        serde_json::to_vec_pretty(&*result).unwrap(),
    )
    .unwrap();
    std::fs::write(
        out.join("06-expected-voxel-centers.json"),
        serde_json::to_vec_pretty(&expected).unwrap(),
    )
    .unwrap();
    voxels.close();
    println!(
        "PASS 5: three portal localizations -> depth observations -> 27 occupied 10cm voxels in the same USDA as three 40cm portals"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn portal_mapping_localization_transfer_and_merge_export_usda() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let out = std::env::var_os("AUKI_PORTAL_EXPERIMENT_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/portal-experiments")
            });
        std::fs::create_dir_all(&out).unwrap();
        let domain = Uuid::new_v4();
        let i1 = Identity::generate();
        let i2 = Identity::generate();
        let (p1, _a1) = AukiPeer::start_external(
            i1.clone(),
            support::authority(&i1, domain),
            support::direct_config(),
        )
        .await
        .unwrap();
        let (p2, _a2) = AukiPeer::start_external(
            i2.clone(),
            support::authority(&i2, domain),
            support::direct_config(),
        )
        .await
        .unwrap();
        let r1 = ComponentRuntime::new(p1.peer_id().to_string());
        let r2 = ComponentRuntime::new(p2.peer_id().to_string());
        let mut rig1 = CameraRig::new(&r1, domain);
        let mut rig2 = CameraRig::new(&r2, domain);
        let ab = make_map(&r1, "peer1-AB", "store-frame", domain);
        rig1.maps.register(ab.clone()).unwrap();
        let (a, range) = rig1.observe(1).await;
        let camera_a = camera_pose(&rig1.frame, 0., range);
        map_detection(&rig1, domain, ab.clone(), &a, &camera_a).await;
        check_pose(
            &snapshot(&ab).scenegraph.anchors[&id(1)].pose_in_map,
            &expected_portal(1, 0.),
        );
        // New observation after placement, rather than only inverting the mapping observation.
        let (a_again, range) = rig1.observe(1).await;
        check_pose(
            &rig1.localize(&r1, &a_again, &ab).camera_pose_in_map,
            &camera_pose(&rig1.frame, 0., range),
        );
        export(&ab, &out.join("01-portal-A.usda"));
        println!("PASS 1: rendered portal A -> size -> PnP placement -> fresh-frame localization");
        let (b, range) = rig1.observe(2).await;
        map_detection(
            &rig1,
            domain,
            ab.clone(),
            &b,
            &camera_pose(&rig1.frame, 2., range),
        )
        .await;
        assert_eq!(snapshot(&ab).scenegraph.anchors.len(), 2);
        check_pose(
            &snapshot(&ab).scenegraph.anchors[&id(2)].pose_in_map,
            &expected_portal(2, 2.),
        );
        export(&ab, &out.join("02-peer1-AB.usda"));
        println!("PASS 2: map A,B has two correctly placed portals");
        let imported = make_map(&r2, "peer2-imported-AB", "store-frame", domain);
        let bc = make_map(&r2, "peer2-BC", "peer2-frame", domain);
        let merged = make_map(&r1, "peer1-ABC", "store-frame", domain);
        let e1 = ComponentProtocolEndpoint::mount(p1.protocols(), r1.clone()).unwrap();
        e1.export_product(&ab.product()).unwrap();
        let e2 = ComponentProtocolEndpoint::mount(p2.protocols(), r2.clone()).unwrap();
        let received = receive(&p2, &p1, &ab).await;
        for anchor in received.scenegraph.anchors.values() {
            put(&r2, &imported, anchor.clone());
        }
        rig2.maps.register(imported.clone()).unwrap();
        let (b_remote, range) = rig2.observe(2).await;
        check_pose(
            &rig2.localize(&r2, &b_remote, &imported).camera_pose_in_map,
            &camera_pose(&rig2.frame, 2., range),
        );
        export(&imported, &out.join("03-peer2-received-AB.usda"));
        println!(
            "PASS 3: authenticated transfer -> local import on Peer2 -> fresh-frame localization"
        );
        rig2.maps.unregister(&imported);
        rig2.maps.register(bc.clone()).unwrap();
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let frame2_to_store = RigidTransform {
            from_frame_id: "peer2-frame".into(),
            to_frame_id: "store-frame".into(),
            translation: [2., 0., 0.],
            rotation_wxyz: [h, 0., 0., h],
        };
        for (n, x) in [(2, 2.), (3, 4.)] {
            let (record, range) = rig2.observe(n).await;
            let camera_in_frame2 = compose(
                &camera_pose(&rig2.frame, x, range),
                &inverse(&frame2_to_store),
            );
            map_detection(&rig2, domain, bc.clone(), &record, &camera_in_frame2).await;
        }
        export(&bc, &out.join("04-peer2-BC.usda"));
        e2.export_product(&bc.product()).unwrap();
        let remote_bc = receive(&p1, &p2, &bc).await;
        let mut checker = MapAlignmentChecker::new(AlignmentOptions::default()).unwrap();
        checker
            .receive_snapshot("AB", ab.snapshot_reference(), snapshot(&ab))
            .unwrap();
        checker
            .receive_snapshot("BC", bc.snapshot_reference(), remote_bc.clone())
            .unwrap();
        let AlignmentResult::Available { transform, .. } = checker.check("BC", "AB") else {
            panic!("alignment through B unavailable")
        };
        check_pose(&transform, &frame2_to_store);
        for anchor in snapshot(&ab).scenegraph.anchors.values() {
            put(&r1, &merged, anchor.clone());
        }
        for remote in remote_bc.scenegraph.anchors.values() {
            let mut anchor = remote.clone();
            anchor.pose_in_map = compose(&remote.pose_in_map, &transform);
            if let Some(existing) = snapshot(&merged).scenegraph.anchors.get(&anchor.anchor_id) {
                check_pose(&anchor.pose_in_map, &existing.pose_in_map);
            } else {
                put(&r1, &merged, anchor);
            }
        }
        let final_map = snapshot(&merged);
        assert_eq!(final_map.scenegraph.anchors.len(), 3);
        for (n, x) in [(1, 0.), (2, 2.), (3, 4.)] {
            check_pose(
                &final_map.scenegraph.anchors[&id(n)].pose_in_map,
                &expected_portal(n, x),
            );
        }
        assert_eq!(snapshot(&ab).scenegraph.anchors.len(), 2);
        assert_eq!(snapshot(&bc).scenegraph.anchors.len(), 2);
        export(&merged, &out.join("05-peer1-ABC.usda"));
        e1.export_product(&merged.product()).unwrap();
        println!(
            "PASS 4: A,B + rotated/translated B,C -> A,B,C; B deduplicated; originals unchanged"
        );
        voxel_experiment(&r1, &rig1, &merged, &out).await;
        println!("USDA artifacts: {}", out.display());
        rig1.close();
        rig2.close();
        e1.close().await.unwrap();
        e2.close().await.unwrap();
        ab.close();
        bc.close();
        merged.close();
        imported.close();
        p1.shutdown().await.unwrap();
        p2.shutdown().await.unwrap();
    })
    .await
    .expect("experiment timeout");
}

#[path = "../../auki-components/tests/support/clock.rs"]
mod clock_fixture;
use clock_fixture::fixture_clock;
